use super::*;
use crate::{
    plugin::{BoxFuture, EventHandler, EventPriority, block::block_physics::BlockPhysicsEvent},
    server::Server,
};
use pumpkin_data::data_component_impl::{PotDecoration, PotDecorationsImpl};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

struct FlushInitialPot {
    world: Arc<World>,
    position: BlockPos,
    count: Arc<AtomicUsize>,
}
impl EventHandler<BlockPhysicsEvent> for FlushInitialPot {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut BlockPhysicsEvent,
    ) -> BoxFuture<'a, ()> {
        if event.changed_pos == self.position && self.count.fetch_add(1, Relaxed) == 0 {
            // The terrain publication can flush before BlockItem applies the claimed faces.
            self.world.flush_block_updates();
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_bedrock_observer_receives_post_application_pot_faces() -> TestResult {
    let mut fixture = new_fixture().await;
    let world = fixture.world.world();
    let position = BlockPos::new(8, 65, 8);
    world.set_block_state(
        &position.down(),
        Block::STONE.default_state.id,
        pumpkin_world::world::BlockFlags::FORCE_STATE,
    );
    let items = [
        &Item::ANGLER_POTTERY_SHERD,
        &Item::BRICK,
        &Item::ARCHER_POTTERY_SHERD,
        &Item::BRICK,
    ];
    let mut pot = ItemStack::new(1, &Item::DECORATED_POT);
    pot.set_data_component(PotDecorationsImpl {
        decorations: items.map(|item| {
            Some(PotDecoration {
                item,
                count: 1,
                patch: Vec::new(),
            })
        }),
    });
    let player = fixture.owner.player.clone();
    player.get_entity().set_pos(Vector3::new(8.5, 65.0, 10.5));
    player.inventory().set_selected_slot(4);
    player.inventory().set_stack(4, pot.clone());
    let count = Arc::new(AtomicUsize::new(0));
    fixture
        .world
        .server
        .plugin_manager
        .register::<BlockPhysicsEvent, _>(
            Arc::new(FlushInitialPot {
                world: world.clone(),
                position,
                count: count.clone(),
            }),
            EventPriority::Normal,
            true,
        );
    fixture.drain_packets();
    fixture.owner.client().handle_inventory_action(
        &player,
        click_block_packet(
            &player,
            HandSlot::Mainhand,
            position.down(),
            &pot,
            Vec::new(),
        ),
    );
    assert!(
        count.load(Relaxed) > 0,
        "initial state flushed before component application"
    );
    assert_eq!(world.get_block(&position), &Block::DECORATED_POT);
    assert!(player.inventory().held_item().is_empty());
    let packets = fixture.bedrock_observer.take_packets();
    let mut actors = Vec::new();
    for packet in &packets {
        let Some(mut payload) = packet_payload(packet, 56) else {
            continue;
        };
        if BlockPos::read(&mut payload)? != position {
            continue;
        }
        actors.push(Nbt::read(&mut NbtReadHelperBedrock::new(Cursor::new(payload)))?.root_tag);
    }
    assert!(
        actors.len() >= 2,
        "watcher receives initial and post-application actors"
    );
    let expected: Vec<_> = items
        .into_iter()
        .map(|item| NbtTag::String(format!("minecraft:{}", item.registry_key).into()))
        .collect();
    assert_eq!(
        actors.last().unwrap().get_list("sherds"),
        Some(expected.as_slice())
    );
    assert_eq!(
        actors.last().unwrap().get_string("id"),
        Some("DecoratedPot")
    );
    fixture.close().await;
    Ok(())
}
