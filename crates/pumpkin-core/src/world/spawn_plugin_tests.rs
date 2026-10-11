use super::spawn_test_support::{proto, publish};
use crate::{
    block::entities::{
        BlockEntity,
        trial_spawner::{SpawnData, TrialSpawnerBlockEntity},
    },
    entity::{death_test_world::DeathTestWorld, spawn_mount, r#type::from_type},
    plugin::{
        EventHandler, EventPriority,
        api::events::entity::{
            entity_mount::EntityMountEvent, entity_spawn::EntitySpawnEvent,
            trial_spawner_spawn::TrialSpawnerSpawnEvent,
        },
    },
    server::Server,
};
use futures::future::BoxFuture;
use pumpkin_data::{Block, biome::Biome, entity::EntityType};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed},
};

#[derive(Default)]
struct SpawnHandler {
    cancel_spawn: AtomicBool,
    cancel_mount: AtomicBool,
    invalidate_trial: AtomicBool,
    births: AtomicUsize,
    mounts: AtomicUsize,
    trials: AtomicUsize,
    trial: Option<Arc<TrialSpawnerBlockEntity>>,
}

impl SpawnHandler {
    fn read_trial(&self) {
        if let Some(block) = &self.trial {
            assert!(
                block.trial_spawner.try_lock().is_ok(),
                "callback retained the trial mutex"
            );
            let mut nbt = NbtCompound::new();
            block.write_nbt(&mut nbt);
            assert!(nbt.get_list("current_mobs").is_some());
        }
    }
}
impl EventHandler<EntitySpawnEvent> for SpawnHandler {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntitySpawnEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.read_trial();
            self.births.fetch_add(1, Relaxed);
            event.cancelled = self.cancel_spawn.load(Relaxed);
        })
    }
}
impl EventHandler<EntityMountEvent> for SpawnHandler {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntityMountEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.read_trial();
            self.mounts.fetch_add(1, Relaxed);
            event.cancelled = self.cancel_mount.load(Relaxed);
        })
    }
}
impl EventHandler<TrialSpawnerSpawnEvent> for SpawnHandler {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut TrialSpawnerSpawnEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.read_trial();
            self.trials.fetch_add(1, Relaxed);
            if self.invalidate_trial.load(Relaxed) {
                self.trial
                    .as_ref()
                    .unwrap()
                    .trial_spawner
                    .lock()
                    .unwrap()
                    .data
                    .total_mobs_spawned = 99;
            }
        })
    }
}
fn register(fixture: &DeathTestWorld, handler: &Arc<SpawnHandler>) {
    let plugins = &fixture.server.plugin_manager;
    plugins.register::<EntitySpawnEvent, _>(handler.clone(), EventPriority::Normal, true);
    plugins.register::<EntityMountEvent, _>(handler.clone(), EventPriority::Normal, true);
    plugins.register::<TrialSpawnerSpawnEvent, _>(handler.clone(), EventPriority::Normal, true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spawn_plugins_cancel_live_chicken_and_fresh_structure_births() {
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    let handler = Arc::new(SpawnHandler::default());
    register(&fixture, &handler);
    let chicken = fixture.mob(&EntityType::CHICKEN);
    let zombie = from_type(
        &EntityType::ZOMBIE,
        chicken.get_entity().pos.load(),
        &world,
        uuid::Uuid::new_v4(),
    );
    for cancel_spawn in [true, false] {
        handler.cancel_spawn.store(cancel_spawn, Relaxed);
        handler.cancel_mount.store(!cancel_spawn, Relaxed);
        spawn_mount::queue_existing_chicken(&zombie, chicken.clone());
        assert!(!world.spawn_entity(zombie.clone()));
        assert!(!chicken.get_entity().has_passengers());
        assert!(zombie.get_entity().get_vehicle().is_none());
        let mut nbt = NbtCompound::new();
        chicken.write_nbt(&mut nbt);
        assert_eq!(nbt.get_bool("IsChickenJockey"), Some(false));
    }
    handler.cancel_spawn.store(true, Relaxed);
    handler.cancel_mount.store(false, Relaxed);
    let structure_mob = from_type(
        &EntityType::COW,
        Vector3::new(8.5, 64.0, 8.5),
        &world,
        uuid::Uuid::new_v4(),
    );
    let mut chunk = proto(&Biome::PLAINS, &Block::STONE);
    let mut nbt = NbtCompound::new();
    structure_mob.write_nbt(&mut nbt);
    world.retain_structure_entities(&mut chunk, vec![nbt]);
    publish(&world, chunk);
    let before = handler.births.load(Relaxed);
    world.publish_generated_entities(Vector2::new(0, 0));
    assert_eq!(handler.births.load(Relaxed), before + 1);
    assert_eq!(world.entities.load().len(), 1);
    // Real disk-style restoration must not emit another birth or mount callback.
    let mut saved = NbtCompound::new();
    structure_mob.write_nbt(&mut saved);
    world.restore_entity_tree(&saved, None);
    assert_eq!(handler.births.load(Relaxed), before + 1);
    assert!(
        world
            .get_entity_by_uuid(structure_mob.get_entity().entity_uuid)
            .is_some()
    );
    for world in fixture.server.worlds.load().iter() {
        world.shutdown().await;
    }
    crate::server::fixture_lifecycle::finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trial_tick_callbacks_read_nbt_and_reject_changed_spawner_state() {
    use pumpkin_data::block_properties::{TrialSpawnerLikeProperties, TrialSpawnerState};
    let fixture = DeathTestWorld::new().await;
    let world = fixture.world();
    publish(&world, proto(&Biome::PLAINS, &Block::STONE));
    let pos = BlockPos::new(8, 64, 8);
    let mut props =
        TrialSpawnerLikeProperties::from_state_id(Block::TRIAL_SPAWNER.default_state.id);
    props.trial_spawner_state = TrialSpawnerState::Active;
    world
        .level
        .set_block_state(&pos, props.to_state_id(&Block::TRIAL_SPAWNER));
    let block = Arc::new(TrialSpawnerBlockEntity::new(pos));
    let mut raw = NbtCompound::new();
    raw.put_string("id", "minecraft:skeleton".to_owned());
    raw.put(
        "Pos",
        NbtTag::List(vec![
            NbtTag::Double(10.5),
            NbtTag::Double(65.0),
            NbtTag::Double(8.5),
        ]),
    );
    let mut passenger = NbtCompound::new();
    passenger.put_string("id", "minecraft:chicken".to_owned());
    raw.put(
        "Passengers",
        NbtTag::List(vec![NbtTag::Compound(passenger)]),
    );
    {
        let mut spawner = block.trial_spawner.lock().unwrap();
        spawner.override_peaceful_and_mob_spawn_rule = true;
        spawner.data.next_spawn_data = Some(SpawnData {
            raw_entity_nbt: Some(raw),
            ..SpawnData::from_entity_type(&EntityType::SKELETON)
        });
    };
    world.add_block_entity(block.clone());
    let handler = Arc::new(SpawnHandler {
        trial: Some(block.clone()),
        ..Default::default()
    });
    register(&fixture, &handler);
    block.tick(&world);
    assert_eq!(handler.trials.load(Relaxed), 1);
    assert_eq!(handler.births.load(Relaxed), 2);
    assert_eq!(handler.mounts.load(Relaxed), 1);
    assert_eq!(
        block.trial_spawner.lock().unwrap().data.total_mobs_spawned,
        1
    );
    let entities = world.entities.load_full();
    for entity in entities.iter() {
        world.remove_entity(entity.as_ref());
    }
    super::entity_persistence::detach_unloaded_trees(&entities);
    {
        let mut spawner = block.trial_spawner.lock().unwrap();
        spawner.data.current_mobs.clear();
        spawner.data.total_mobs_spawned = 0;
        spawner.data.next_mob_spawns_at = 0;
    };
    handler.invalidate_trial.store(true, Relaxed);
    block.tick(&world);
    assert_eq!(handler.trials.load(Relaxed), 2);
    assert!(world.entities.load().is_empty());
    assert_eq!(
        block.trial_spawner.lock().unwrap().data.total_mobs_spawned,
        99
    );
    for world in fixture.server.worlds.load().iter() {
        world.shutdown().await;
    }
    crate::server::fixture_lifecycle::finish().await;
}
