use std::{num::NonZeroU8, sync::Arc};

use pumpkin_data::{entity::EntityType, item::Item, item_stack::ItemStack};
use pumpkin_util::math::{vector2::Vector2, vector3::Vector3};

use super::{World, chunker};
use crate::{
    entity::{
        Entity, EntityBase, RemovalReason,
        ai::goal::{active_target::ActiveTargetGoal, goal_selector::GoalSelector},
        item::ItemEntity,
        r#type::from_type,
    },
    net::java::combat_test_support::TestPlayer,
    server::{
        Server,
        combat_test_support::{publish_empty_chunk, server, world},
    },
};

fn player(world: &Arc<World>) -> TestPlayer {
    let fixture = TestPlayer::new(world);
    let mut config = (**fixture.player.config.load()).clone();
    config.view_distance = NonZeroU8::new(2).unwrap();
    fixture.player.config.store(Arc::new(config));
    let chunks: Vec<_> = fixture
        .player
        .watched_section
        .load()
        .all_chunks_within()
        .collect();
    world.level.watch_entity_chunks(&chunks);
    fixture
}

fn move_player(fixture: &TestPlayer, chunk_x: i32) {
    fixture
        .player
        .get_entity()
        .set_pos(Vector3::new(f64::from(chunk_x * 16) + 8.5, 64.0, 8.5));
    chunker::update_position(&fixture.player);
}

fn mob(world: &Arc<World>, ty: &'static EntityType, x: f64) -> Arc<dyn EntityBase> {
    let entity = from_type(ty, Vector3::new(x, 64.0, 8.5), world, uuid::Uuid::new_v4());
    assert!(world.spawn_entity(entity.clone()));
    entity
}

async fn activate(world: &Arc<World>, pos: Vector2<i32>) {
    let storage = world.level.get_entity_chunk(pos).await.unwrap();
    world.make_chunk_entities_live(&storage, None);
}

async fn reload_through_watch_and_tick(server: &Arc<Server>, world: &Arc<World>) {
    let (queued, ready) = tokio::sync::oneshot::channel();
    *world.after_entity_activation_queued.lock().unwrap() = Some(queued);
    world.set_chunks_forced(&[Vector2::new(0, 0)], true);
    ready.await.unwrap();
    world.tick(server);
}

async fn reopen(server: &Arc<Server>, old: &Arc<World>, path: &std::path::Path) -> Arc<World> {
    old.shutdown().await;
    world(server, path)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rewatch_before_unload_commit_preserves_mobs_and_items_on_disk() {
    // Also hide the new generation before the old commit: a watcher check alone is insufficient.
    for leave_again in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let server = server(dir.path());
        let world = world(&server, dir.path());
        publish_empty_chunk(&world, Vector2::new(0, 0));
        let fixture = player(&world);
        activate(&world, Vector2::new(0, 0)).await;
        let sheep = mob(&world, &EntityType::SHEEP, 8.5);
        let cow = mob(&world, &EntityType::COW, 9.5);
        let item: Arc<dyn EntityBase> = Arc::new(ItemEntity::new(
            Entity::new(
                world.clone(),
                Vector3::new(10.5, 64.0, 8.5),
                &EntityType::ITEM,
            ),
            ItemStack::new(3, &Item::DIAMOND),
        ));
        assert!(world.spawn_entity(item.clone()));
        let members = [sheep, cow, item];
        let ids: Vec<_> = members
            .iter()
            .map(|entity| entity.get_entity().entity_uuid)
            .collect();
        move_player(&fixture, 6);
        assert!(!world.level.is_chunk_watched(&Vector2::new(0, 0)));
        let returning = fixture.player.clone();
        *world.before_entity_unload_commit.lock().unwrap() = Some(Box::new(move || {
            // Force movement after serialization but before the real tick's commit, without a timer.
            returning.get_entity().set_pos(Vector3::new(8.5, 64.0, 8.5));
            chunker::update_position(&returning);
            if leave_again {
                returning
                    .get_entity()
                    .set_pos(Vector3::new(104.5, 64.0, 8.5));
                chunker::update_position(&returning);
            }
        }));
        world.tick(&server);
        assert_eq!(
            world.level.is_chunk_watched(&Vector2::new(0, 0)),
            !leave_again
        );
        // This snapshot used to overwrite the stale unload's saved records with an empty live set.
        world.save().await;
        let storage = world
            .level
            .get_entity_chunk(Vector2::new(0, 0))
            .await
            .unwrap();
        assert_eq!(storage.data.lock().unwrap().len(), 3);
        for member in &members {
            assert!(!member.get_entity().is_removed());
            assert!(Arc::ptr_eq(
                &world
                    .get_entity_by_uuid(member.get_entity().entity_uuid)
                    .unwrap(),
                member
            ));
        }
        move_player(&fixture, 6);
        world.tick(&server);
        for member in &members {
            assert!(
                member.get_entity().removal_reason.load() == Some(RemovalReason::UnloadedToChunk)
            );
            assert!(
                world
                    .get_entity_by_uuid(member.get_entity().entity_uuid)
                    .is_none()
            );
        }
        let restored = reopen(&server, &world, dir.path()).await;
        reload_through_watch_and_tick(&server, &restored).await;
        for id in &ids {
            assert!(restored.get_entity_by_uuid(*id).is_some());
        }
        assert_eq!(restored.entities.load().len(), 3);
        let stack = restored.get_entity_by_uuid(ids[2]).unwrap();
        assert_eq!(
            stack
                .get_item_entity()
                .unwrap()
                .get_item_stack()
                .lock()
                .unwrap()
                .item_count,
            3
        );
        restored.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wolf_tick_drops_target_retired_by_chunk_unload() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    for pos in [Vector2::new(0, 0), Vector2::new(1, 0)] {
        publish_empty_chunk(&world, pos);
    }
    let fixture = player(&world);
    world.set_chunks_forced(&[Vector2::new(1, 0)], true);
    activate(&world, Vector2::new(0, 0)).await;
    activate(&world, Vector2::new(1, 0)).await;
    let target = mob(&world, &EntityType::SHEEP, 13.5);
    let wolf = mob(&world, &EntityType::WOLF, 18.5);
    let mob = wolf.get_mob().unwrap().get_mob_entity();
    // Remove random search delay and unrelated movement; use the real target selector in mob ticks.
    *mob.goals_selector.lock().unwrap() = GoalSelector::default();
    *mob.target_selector.lock().unwrap() = GoalSelector::default();
    mob.add_target_goal(
        0,
        ActiveTargetGoal::new(
            mob,
            &EntityType::SHEEP,
            0,
            false,
            false,
            None::<fn(&crate::entity::living::LivingEntity, &World) -> bool>,
        ),
    );
    world.tick(&server);
    assert!(Arc::ptr_eq(&mob.get_target().unwrap(), &target));
    move_player(&fixture, 6);
    world.tick(&server);
    for _ in 0..3 {
        world.tick(&server);
    }
    assert!(mob.get_target().is_none());
    assert!(target.get_entity().is_removed());
    assert!(target.get_entity().removal_reason.load() == Some(RemovalReason::UnloadedToChunk));
    assert!(
        world
            .get_entity_by_uuid(target.get_entity().entity_uuid)
            .is_none()
    );
    assert!(
        world
            .get_entity_by_uuid(wolf.get_entity().entity_uuid)
            .is_some()
    );
    let restored = reopen(&server, &world, dir.path()).await;
    reload_through_watch_and_tick(&server, &restored).await;
    let reloaded = restored
        .get_entity_by_uuid(target.get_entity().entity_uuid)
        .unwrap();
    assert!(!reloaded.get_entity().is_removed());
    assert!(!Arc::ptr_eq(&reloaded, &target));
    restored.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn player_cleanup_preserves_dormant_storage_until_spawned_entities_are_saved() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    let mut fixture = player(&world);
    let pos = Vector2::new(0, 0);
    // A command can insert an entity before the chunk's saved copy has been activated.
    let storage = world.level.get_entity_chunk(pos).await.unwrap();
    assert!(!storage.live.load(std::sync::atomic::Ordering::Acquire));
    let pig = mob(&world, &EntityType::PIG, 8.5);
    let player = fixture.player.clone();
    fixture
        .with_outgoing_writer(player.unload_watched_chunks(&world))
        .await;
    assert!(Arc::ptr_eq(
        &world.level.get_entity_chunk_sync(&pos).unwrap(),
        &storage
    ));
    assert!(
        world
            .get_entity_by_uuid(pig.get_entity().entity_uuid)
            .is_some()
    );
    move_player(&fixture, 6);
    world.tick(&server);
    assert!(pig.get_entity().is_removed());
    assert!(
        world
            .get_entity_by_uuid(pig.get_entity().entity_uuid)
            .is_none()
    );
    let restored = reopen(&server, &world, dir.path()).await;
    reload_through_watch_and_tick(&server, &restored).await;
    assert!(
        restored
            .get_entity_by_uuid(pig.get_entity().entity_uuid)
            .is_some()
    );
    assert_eq!(restored.entities.load().len(), 1);
    restored.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn item_entering_chunk_during_preparation_retries_the_complete_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    for pos in [Vector2::new(0, 0), Vector2::new(1, 0)] {
        publish_empty_chunk(&world, pos);
    }
    let fixture = player(&world);
    world.set_chunks_forced(&[Vector2::new(1, 0)], true);
    activate(&world, Vector2::new(0, 0)).await;
    activate(&world, Vector2::new(1, 0)).await;
    let sheep = mob(&world, &EntityType::SHEEP, 8.5);
    let item: Arc<dyn EntityBase> = Arc::new(ItemEntity::new_with_velocity(
        Entity::new(
            world.clone(),
            Vector3::new(16.1, 64.0, 8.5),
            &EntityType::ITEM,
        ),
        ItemStack::new(1, &Item::DIAMOND),
        Vector3::new(-0.5, 0.0, 0.0),
        ItemEntity::DEFAULT_PICKUP_DELAY,
    ));
    assert!(world.spawn_entity(item.clone()));
    move_player(&fixture, 6);
    let entering = item.clone();
    let ticking_server = server.clone();
    *world.before_entity_unload_commit.lock().unwrap() = Some(Box::new(move || {
        // Drive ItemEntity.tick across the boundary after the unload's snapshot was prepared.
        entering.tick(entering.as_ref(), &ticking_server);
        assert_eq!(entering.get_entity().chunk_pos.load(), Vector2::new(0, 0));
    }));
    world.tick(&server);
    assert!(!sheep.get_entity().is_removed());
    assert!(!item.get_entity().is_removed());
    world.tick(&server);
    assert!(sheep.get_entity().is_removed());
    assert!(item.get_entity().is_removed());
    let restored = reopen(&server, &world, dir.path()).await;
    reload_through_watch_and_tick(&server, &restored).await;
    assert!(
        restored
            .get_entity_by_uuid(sheep.get_entity().entity_uuid)
            .is_some()
    );
    assert!(
        restored
            .get_entity_by_uuid(item.get_entity().entity_uuid)
            .is_some()
    );
    assert_eq!(restored.entities.load().len(), 2);
    restored.shutdown().await;
}
