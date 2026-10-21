use super::test_support::Scenario;
use crate::{
    entity::{
        Entity, EntityBase,
        projectile::{
            ProjectileHit,
            arrow::{ArrowEntity, ArrowPickup},
        },
    },
    plugin::{
        BoxFuture, EventHandler, EventPriority, api::events::block::block_break::BlockBreakEvent,
    },
    server::Server,
};
use pumpkin_data::{
    Block, BlockDirection, block_properties::DecoratedPotLikeProperties, entity::EntityType,
    game_event::GameEvent, item::Item, item_stack::ItemStack,
};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::world::BlockFlags;
use std::sync::Arc;

struct CancelBreak;
impl EventHandler<BlockBreakEvent> for CancelBreak {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut BlockBreakEvent,
    ) -> BoxFuture<'a, ()> {
        event.cancelled = true;
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_projectile_destroy_event_retains_source_and_broken_state() {
    for cancelled in [false, true] {
        let scenario = Scenario::new(true);
        let position = BlockPos::new(8, 65, 8);
        scenario.world.set_block_state(
            &position,
            Block::DECORATED_POT.default_state.id,
            BlockFlags::NOTIFY_ALL,
        );
        if cancelled {
            scenario
                .server
                .plugin_manager
                .register::<BlockBreakEvent, _>(Arc::new(CancelBreak), EventPriority::Normal, true);
        }
        scenario.capture(None);
        let arrow = Arc::new(ArrowEntity::new_shot(
            Entity::new(
                scenario.world.clone(),
                position.to_centered_f64(),
                &EntityType::ARROW,
            ),
            scenario.owner.player.get_entity(),
            &ItemStack::new(1, &Item::ARROW),
            ArrowPickup::Disallowed,
        ));
        assert!(scenario.world.spawn_entity(arrow.clone()));
        let source_position = arrow.get_entity().pos.load();
        arrow.on_hit(ProjectileHit::Block {
            pos: position,
            world_border: false,
            face: BlockDirection::West,
            hit_pos: Vector3::new(8.0625, 65.6, 8.5),
            normal: Vector3::new(-1.0, 0.0, 0.0),
        });
        {
            let observations = scenario.sink.lock().unwrap();
            let events: Vec<_> = observations
                .forwarded
                .iter()
                .filter(|event| event.event == GameEvent::BlockDestroy)
                .collect();
            assert_eq!(events.len(), usize::from(!cancelled));
            if !cancelled {
                let event = events[0];
                let source = event
                    .source
                    .as_ref()
                    .expect("pot destruction retains its projectile source");
                assert_eq!(source.id, arrow.get_entity().entity_id);
                assert_eq!(source.uuid, arrow.get_entity().entity_uuid);
                assert_eq!(
                    source.instance,
                    std::ptr::from_ref(arrow.get_entity()) as usize
                );
                assert_eq!(source.world, Arc::as_ptr(&scenario.world) as usize);
                assert_eq!(source.position, source_position);
                assert_eq!(source.life, None);
                assert_eq!(event.position, position.to_centered_f64());
                let mut broken = DecoratedPotLikeProperties::default(&Block::DECORATED_POT);
                broken.cracked = true;
                assert_eq!(
                    event.affected_state,
                    Some(broken.to_state_id(&Block::DECORATED_POT))
                );
            }
        }
        assert_eq!(
            scenario.world.get_block(&position) == &Block::DECORATED_POT,
            cancelled
        );
        tokio::time::timeout(
            std::time::Duration::from_secs(20),
            scenario.server.shutdown(),
        )
        .await
        .expect("projectile game-event fixture shutdown did not finish");
    }
}
