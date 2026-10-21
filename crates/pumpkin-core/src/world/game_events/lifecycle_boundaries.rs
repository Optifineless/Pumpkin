use super::{
    GameEventContext,
    test_support::{DEPARTURE, Scenario},
};
use crate::{
    entity::{Entity, EntityBase},
    plugin::{
        BoxFuture, EventHandler, EventPriority, api::events::entity::entity_spawn::EntitySpawnEvent,
    },
    server::Server,
};
use pumpkin_data::{Block, entity::EntityType};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use std::{cell::Cell, sync::Arc};

struct CancelSpawn(i32);
impl EventHandler<EntitySpawnEvent> for CancelSpawn {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntitySpawnEvent,
    ) -> BoxFuture<'a, ()> {
        if event.entity_id == self.0 {
            event.cancelled = true;
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s2_cancelled_spawn_and_riding_tree_unload_restore() {
    let s = Scenario::new(true);
    s.capture(None);
    let cancelled: Arc<dyn EntityBase> =
        Arc::new(Entity::new(s.world.clone(), DEPARTURE, &EntityType::MARKER));
    s.server.plugin_manager.register::<EntitySpawnEvent, _>(
        Arc::new(CancelSpawn(cancelled.get_entity().entity_id)),
        EventPriority::Normal,
        true,
    );
    assert!(!s.world.spawn_entity(cancelled.clone()));
    assert!(
        s.world
            .game_event_source_membership(cancelled.get_entity().entity_id)
            .is_none()
    );
    let pos = Vector2::new(0, 0);
    let storage = s.world.level.get_entity_chunk(pos).await.unwrap();
    s.world.make_chunk_entities_live(&storage, None);
    let root = crate::entity::r#type::from_type(
        &EntityType::COW,
        Vector3::new(12.5, 65.0, 12.5),
        &s.world,
        uuid::Uuid::new_v4(),
    );
    let rider = crate::entity::r#type::from_type(
        &EntityType::COW,
        Vector3::new(12.5, 66.0, 12.5),
        &s.world,
        uuid::Uuid::new_v4(),
    );
    s.world.add_entity_silent(root.clone());
    s.world.add_entity_silent(rider.clone());
    root.get_entity().add_passenger(root.clone(), rider.clone());
    s.world.game_event_item_interact_finish(rider.get_entity());
    s.world.remove_entities_in_chunks([pos]).await;
    s.world.process_entity_chunk_lifecycle();
    for old in [&root, &rider] {
        assert!(old.get_entity().is_removed());
        assert!(
            s.world
                .game_event_source_membership(old.get_entity().entity_id)
                .is_none()
        );
        s.world.game_event_item_interact_finish(old.get_entity());
    }
    assert_eq!(s.sink.lock().unwrap().forwarded.len(), 1);
    let storage = s.world.level.get_entity_chunk(pos).await.unwrap();
    s.world.make_chunk_entities_live(&storage, None);
    for old in [&root, &rider] {
        let restored = s
            .world
            .get_entity_by_uuid(old.get_entity().entity_uuid)
            .unwrap();
        assert!(!Arc::ptr_eq(&restored, old));
        s.world
            .game_event_item_interact_finish(restored.get_entity());
    }
    assert_eq!(s.sink.lock().unwrap().forwarded.len(), 3);
    s.world.shutdown().await;
    let late: Arc<dyn EntityBase> =
        Arc::new(Entity::new(s.world.clone(), DEPARTURE, &EntityType::MARKER));
    s.world.add_entity_silent(late.clone());
    assert!(
        s.world
            .game_event_source_membership(late.get_entity().entity_id)
            .is_none(),
        "late publisher cannot reopen a shut down world's authority"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s2_off_skips_stale_source_and_lazy_state() {
    let s = Scenario::new(false);
    s.capture(None);
    let missing = Entity::new(s.world.clone(), DEPARTURE, &EntityType::MARKER);
    let resolutions = Cell::new(0);
    for key in ["plugin:custom", "block_change"] {
        s.world
            .emit_game_event_with_lazy_context(key, DEPARTURE, || {
                resolutions.set(resolutions.get() + 1);
                GameEventContext {
                    source: Some(&missing),
                    affected_state: Some(s.world.get_block_state_id(&BlockPos::new(8, 64, 8))),
                }
            });
    }
    assert_eq!(s.world.get_block(&BlockPos::new(8, 64, 8)), &Block::STONE);
    assert_eq!(s.sink.lock().unwrap().plugins.len(), 2);
    assert!(s.sink.lock().unwrap().forwarded.is_empty());
    assert_eq!(
        resolutions.get(),
        0,
        "OFF must not resolve affected-state or stale source"
    );
    assert_eq!(
        s.world
            .game_event_sources
            .probes
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
}
