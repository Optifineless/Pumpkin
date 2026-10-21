use super::{projectile_tests::paid_arrow, test_support::Scenario};
use crate::{
    entity::{
        Entity, EntityBase,
        living::LivingEntity,
        projectile::{arrow::ArrowEntity, llama_spit::LlamaSpitEntity},
    },
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::entity::projectile_hit::ProjectileHitEvent,
    },
    server::{Server, combat_test_support},
    world::World,
};
use pumpkin_data::{Block, entity::EntityType, game_event::GameEvent};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::world::BlockFlags;
use std::sync::{Arc, atomic::Ordering::Relaxed};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s2_projectile_unknown_foreign_owner_and_deflection() {
    let s = Scenario::new(true);
    s.capture(None);
    let other_dir = tempfile::tempdir().unwrap();
    let other = combat_test_support::world(&s.server, other_dir.path());
    s.server
        .worlds
        .store(Arc::new(vec![s.world.clone(), other.clone()]));
    let foreign: Arc<dyn EntityBase> = Arc::new(Entity::new(
        other.clone(),
        Vector3::default(),
        &EntityType::MARKER,
    ));
    other.add_entity_silent(foreign.clone());
    for mode in 0..3 {
        let arrow = Arc::new(ArrowEntity::new(
            Entity::new(
                s.world.clone(),
                Vector3::new(2.5, 70.0, 2.5),
                &EntityType::ARROW,
            ),
            None,
        ));
        if mode == 1 {
            arrow.projectile.set_owner_uuid(Some(uuid::Uuid::new_v4()));
        }
        if mode == 2 {
            arrow.projectile.set_owner(Some(foreign.get_entity()));
        }
        s.world.add_entity_silent(arrow.clone());
        arrow.tick(arrow.as_ref(), &s.server);
        assert!(!arrow.get_entity().is_removed());
    }
    let sink = s.sink.lock().unwrap();
    assert_eq!(sink.plugins.len(), 3);
    assert_eq!(
        sink.forwarded.len(),
        2,
        "resolved foreign owner rejects; genuinely unknown owners remain actorless"
    );
    assert!(
        sink.forwarded
            .iter()
            .all(|e| e.event == GameEvent::ProjectileShoot && e.source.is_none())
    );
    drop(sink);
    let breeze: Arc<dyn EntityBase> = Arc::new(LivingEntity::new(Entity::new(
        s.world.clone(),
        Vector3::new(12.5, 65.0, 8.5),
        &EntityType::BREEZE,
    )));
    s.world.add_entity_silent(breeze);
    let spit = Arc::new(LlamaSpitEntity::new_shot(
        Entity::new(s.world.clone(), Vector3::default(), &EntityType::LLAMA_SPIT),
        s.owner.player.get_entity(),
    ));
    spit.get_entity().set_pos(Vector3::new(10.5, 65.5, 8.5));
    spit.get_entity()
        .velocity
        .store(Vector3::new(4.0, 0.0, 0.0));
    s.world.add_entity_silent(spit.clone());
    spit.tick(spit.as_ref(), &s.server);
    assert!(!spit.get_entity().is_removed());
    assert!(
        spit.get_entity().velocity_dirty.load(Relaxed),
        "real swept contact runs deflection"
    );
    // LlamaSpit.tick currently reapplies its pre-contact drag/motion after deflection.
    // Keep that existing physics boundary; s2 changes only event context.
    assert_eq!(
        spit.get_entity().velocity.load().x,
        4.0 * f64::from(0.99f32)
    );
    assert!(
        !s.sink
            .lock()
            .unwrap()
            .plugins
            .iter()
            .any(|e| e.key == "projectile_land")
    );
}

struct RoundTrip {
    source: Arc<World>,
    destination: Arc<World>,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s2_late_transfer_retirement_preserves_same_arc_return() {
    for enabled in [true, false] {
        let s = Scenario::new(enabled);
        s.capture(None);
        let other_dir = tempfile::tempdir().unwrap();
        let destination = combat_test_support::world(&s.server, other_dir.path());
        s.server
            .worlds
            .store(Arc::new(vec![s.world.clone(), destination.clone()]));
        let arrow = paid_arrow(&s);
        let entity = arrow.get_entity();
        let late_retire = s.world.game_event_source_retirement(entity);
        arrow.teleport(entity.pos.load(), None, None, destination.clone());
        assert!(Arc::ptr_eq(&entity.world.load_full(), &destination));
        arrow.teleport(entity.pos.load(), None, None, s.world.clone());
        assert!(Arc::ptr_eq(&entity.world.load_full(), &s.world));
        assert!(!entity.is_removed());
        late_retire();
        assert_eq!(
            s.world
                .game_event_source_membership(entity.entity_id)
                .and_then(|membership| membership.entity())
                .map(|current| current.get_entity().entity_uuid),
            Some(entity.entity_uuid),
            "old transfer cleanup must retain new same-Arc admission, even OFF"
        );
        let (plugins, forwarded) = {
            let sink = s.sink.lock().unwrap();
            (sink.plugins.len(), sink.forwarded.len())
        };
        s.world.game_event_item_interact_finish(entity);
        let sink = s.sink.lock().unwrap();
        assert_eq!(sink.plugins.len(), plugins + 1);
        assert_eq!(sink.plugins.last().unwrap().key, "item_interact_finish");
        assert_eq!(sink.forwarded.len(), forwarded + usize::from(enabled));
        if enabled {
            assert_eq!(
                sink.forwarded
                    .last()
                    .unwrap()
                    .source
                    .as_ref()
                    .map(|source| source.uuid),
                Some(entity.entity_uuid)
            );
        }
    }
}
impl EventHandler<ProjectileHitEvent> for RoundTrip {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut ProjectileHitEvent,
    ) -> BoxFuture<'a, ()> {
        let projectile = self.source.get_entity_by_id(event.entity_id).unwrap();
        let entity = projectile.get_entity();
        let position = entity.pos.load();
        projectile.teleport(position, None, None, self.destination.clone());
        projectile.teleport(position, None, None, self.source.clone());
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s2_projectile_hit_callback_roundtrip_rejects_original_action() {
    let s = Scenario::new(true);
    s.capture(None);
    let other_dir = tempfile::tempdir().unwrap();
    let destination = combat_test_support::world(&s.server, other_dir.path());
    s.server
        .worlds
        .store(Arc::new(vec![s.world.clone(), destination.clone()]));
    s.server.plugin_manager.register::<ProjectileHitEvent, _>(
        Arc::new(RoundTrip {
            source: s.world.clone(),
            destination,
        }),
        EventPriority::Normal,
        true,
    );
    let arrow = paid_arrow(&s);
    let hit = BlockPos::new(8, 65, 11);
    s.world
        .set_block_state(&hit, Block::STONE.default_state.id, BlockFlags::FORCE_STATE);
    arrow.get_entity().set_pos(Vector3::new(8.5, 65.5, 9.5));
    arrow
        .get_entity()
        .velocity
        .store(Vector3::new(0.0, 0.0, 2.0));
    arrow.tick(arrow.as_ref(), &s.server);
    assert!(!arrow.get_entity().is_removed());
    assert!(
        arrow
            .cast_any()
            .downcast_ref::<ArrowEntity>()
            .unwrap()
            .in_ground
            .load(Relaxed)
    );
    assert!(Arc::ptr_eq(&arrow.get_entity().world.load_full(), &s.world));
    let sink = s.sink.lock().unwrap();
    assert_eq!(
        sink.plugins
            .iter()
            .filter(|e| e.key == "projectile_land")
            .count(),
        1
    );
    assert_eq!(
        sink.forwarded
            .iter()
            .filter(|e| e.event == GameEvent::ProjectileLand)
            .count(),
        0,
        "pre-hit action membership must survive callbacks, even when same Arc returns"
    );
}
