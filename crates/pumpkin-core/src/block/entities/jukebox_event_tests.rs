use super::*;
use crate::block::{
    BlockBehaviour, BlockHitResult, NormalUseArgs, UseWithItemArgs, blocks::jukebox::JukeboxBlock,
};

use crate::{
    entity::death_test_world::DeathTestWorld,
    plugin::{
        BoxFuture, EventHandler, EventPriority, api::events::world::generic_game::GenericGameEvent,
    },
    server::Server,
    world::spawn_test_support::{proto, publish},
};
use pumpkin_data::{
    Block, biome::Biome, block_properties::JukeboxLikeProperties, game_event::GameEvent, item::Item,
};
use pumpkin_world::world::BlockFlags;
use std::sync::atomic::AtomicUsize;

struct Changes(Arc<AtomicUsize>);
impl EventHandler<GenericGameEvent> for Changes {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut GenericGameEvent,
    ) -> BoxFuture<'a, ()> {
        if event.event_key == GameEvent::BlockChange.name() {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
        Box::pin(async {})
    }
}

fn assert_record(
    world: &World,
    position: &BlockPos,
    changes: &AtomicUsize,
    present: bool,
    count: usize,
) {
    assert_eq!(
        JukeboxLikeProperties::from_state_id(world.get_block_state_id(position)).has_record,
        present
    );
    assert_eq!(changes.load(Ordering::Relaxed), count);
}

async fn create_jukebox() -> (DeathTestWorld, Arc<JukeboxBlockEntity>, Arc<AtomicUsize>) {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let changes = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<GenericGameEvent, _>(
            Arc::new(Changes(changes.clone())),
            EventPriority::Normal,
            true,
        );
    let position = BlockPos::new(8, 64, 8);
    world.set_block_state(
        &position,
        Block::JUKEBOX.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let jukebox = Arc::new(JukeboxBlockEntity::new(position));
    world.add_block_entity(jukebox.clone());
    (fixture, jukebox, changes)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jukebox_record_mutations_emit_one_block_change() {
    let (fixture, jukebox, changes) = create_jukebox().await;
    let world = fixture.world();
    let position = jukebox.get_position();
    let disc = ItemStack::new(1, &Item::MUSIC_DISC_13);
    jukebox.set_record(disc.clone());
    assert_record(&world, &position, &changes, true, 1);
    jukebox.set_record(disc.clone());
    assert_record(&world, &position, &changes, true, 1);
    jukebox.start_playing(1);
    jukebox.tick(&world);
    jukebox.tick(&world);
    assert!(!jukebox.is_playing());
    assert!(!jukebox.get_record().is_empty());
    assert_record(&world, &position, &changes, true, 1);
    assert!(jukebox.clear_record().are_equal(&disc));
    assert_record(&world, &position, &changes, false, 2);
    jukebox.set_stack(0, disc.clone());
    assert_record(&world, &position, &changes, true, 3);
    assert!(jukebox.remove_stack_specific(0, 1).are_equal(&disc));
    assert_record(&world, &position, &changes, false, 4);
    jukebox.set_stack(0, disc.clone());
    assert!(jukebox.remove_stack(0).are_equal(&disc));
    assert_record(&world, &position, &changes, false, 6);
    jukebox.set_stack(0, disc);
    jukebox.clear();
    assert_record(&world, &position, &changes, false, 8);
    jukebox.clear();
    assert_record(&world, &position, &changes, false, 8);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jukebox_player_actions_emit_one_block_change() {
    let (fixture, jukebox, changes) = create_jukebox().await;
    let world = fixture.world();
    let position = jukebox.get_position();
    let player = fixture.player("JukeboxEvents");
    let mut disc = ItemStack::new(2, &Item::MUSIC_DISC_13);
    let cursor = pumpkin_util::math::vector3::Vector3::new(0.5, 1.0, 0.5);
    let hit = BlockHitResult {
        face: &pumpkin_data::BlockDirection::Up,
        cursor_pos: &cursor,
    };
    JukeboxBlock.use_with_item(UseWithItemArgs {
        server: &fixture.server,
        world: &world,
        block: &Block::JUKEBOX,
        position: &position,
        player: &player,
        hit: &hit,
        item_stack: &mut disc,
        equipment_slot: &pumpkin_data::data_component_impl::EquipmentSlot::MAIN_HAND,
    });
    assert_record(&world, &position, &changes, true, 1);
    assert_eq!(disc.item_count, 1);
    JukeboxBlock.normal_use(NormalUseArgs {
        server: &fixture.server,
        world: &world,
        block: &Block::JUKEBOX,
        position: &position,
        player: &player,
        hit: &hit,
    });
    assert_record(&world, &position, &changes, false, 2);
    fixture.server.shutdown().await;
}
