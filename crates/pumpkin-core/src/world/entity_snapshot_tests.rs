use std::sync::{Arc, Mutex};

use pumpkin_data::entity::EntityType;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::{vector2::Vector2, vector3::Vector3};

use super::World;
use crate::{
    command::CommandSender,
    entity::{
        Entity, EntityBase, living::LivingEntity, mob::Mob, projectile::ownership::ProjectileState,
        r#type::from_type,
    },
    server::{
        Server,
        combat_test_support::{publish_empty_chunk, server, world},
    },
};

// Controllable completion at EntityBase::write_nbt, with real entities/commands underneath.
// This needs no pause hook in the production save or tick paths.
type PublishedCompletion = (std::sync::Weak<World>, Box<dyn FnOnce() + Send>);

struct SavingEntity {
    inner: Arc<dyn EntityBase>,
    completion: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    published: Mutex<Option<PublishedCompletion>>,
}

impl EntityBase for SavingEntity {
    fn get_entity(&self) -> &Entity {
        let base = self.inner.get_entity();
        let callback = {
            let mut published = self.published.lock().unwrap();
            let current = base.world.load_full();
            if published.as_ref().is_some_and(|(target, _)| {
                target.upgrade().is_some_and(|target| {
                    Arc::ptr_eq(&target, &current)
                        && current.entities.load().iter().any(|entity| {
                            std::ptr::eq(
                                Arc::as_ptr(entity).cast::<()>(),
                                std::ptr::from_ref(self).cast::<()>(),
                            )
                        })
                })
            }) {
                published.take().map(|(_, callback)| callback)
            } else {
                None
            }
        };
        if let Some(callback) = callback {
            callback();
        }
        base
    }
    fn get_living_entity(&self) -> Option<&LivingEntity> {
        self.inner.get_living_entity()
    }
    fn get_mob(&self) -> Option<&dyn Mob> {
        self.inner.get_mob()
    }
    fn projectile_state(&self) -> Option<&ProjectileState> {
        self.inner.projectile_state()
    }
    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
    fn write_nbt(&self, nbt: &mut NbtCompound) {
        self.inner.write_nbt(nbt);
        let completion = self.completion.lock().unwrap().take();
        if let Some(completion) = completion {
            completion();
        }
    }
}

fn command(server: &Arc<Server>, input: &str) {
    let source = CommandSender::Console.into_source(server);
    assert!(
        server
            .command_dispatcher
            .load()
            .execute_input(input, &source)
            .is_ok(),
        "{input}"
    );
}

fn entity(world: &Arc<World>, ty: &'static EntityType) -> Arc<SavingEntity> {
    let inner = from_type(
        ty,
        Vector3::new(8.5, 64.0, 8.5),
        world,
        uuid::Uuid::new_v4(),
    );
    let wrapped = Arc::new(SavingEntity {
        inner,
        completion: Mutex::default(),
        published: Mutex::default(),
    });
    assert!(world.spawn_entity(wrapped.clone()));
    wrapped
}

async fn activate(world: &Arc<World>) {
    let pos = Vector2::new(0, 0);
    world.level.watch_entity_chunks(&[pos]);
    let storage = world.level.get_entity_chunk(pos).await.unwrap();
    world.make_chunk_entities_live(&storage, None);
}

async fn reload(server: &Arc<Server>, old: &Arc<World>, path: &std::path::Path) -> Arc<World> {
    old.shutdown().await;
    let restored = world(server, path);
    activate(&restored).await;
    restored.tick(server);
    restored
}

fn hold_terrain_ticket(world: &World, player: &crate::entity::player::Player, pos: Vector2<i32>) {
    let level = pumpkin_world::chunk_system::ChunkLoading::FULL_CHUNK_LEVEL;
    *player.held_chunk_tickets.lock().unwrap() = Some((Some(level), None));
    let mut tickets = world.level.chunk_loading.lock().unwrap();
    tickets.add_ticket(pos, level);
    tickets.send_change();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn save_all_cannot_overwrite_a_newer_retirement_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    server.worlds.store(Arc::new(vec![world.clone()]));
    publish_empty_chunk(&world, Vector2::new(0, 0));
    activate(&world).await;
    let mob = entity(&world, &EntityType::PIG);
    let id = mob.get_entity().entity_uuid;
    let (prepared, ready) = tokio::sync::oneshot::channel();
    let (resume, paused) = std::sync::mpsc::sync_channel(0);
    *mob.completion.lock().unwrap() = Some(Box::new(move || {
        prepared.send(()).unwrap();
        paused.recv().unwrap();
    }));
    command(&server, "save-all");
    ready.await.unwrap();
    // The command's old serialized state must not replace the newer retirement snapshot.
    mob.get_living_entity().unwrap().set_health(3.0);
    world.level.unwatch_entity_chunks(&[Vector2::new(0, 0)]);
    world.tick(&server);
    assert!(mob.get_entity().is_removed());
    resume.send(()).unwrap();
    server.finish_test_tasks().await;
    let restored = reload(&server, &world, dir.path()).await;
    let saved = restored.get_entity_by_uuid(id).unwrap();
    assert_eq!(saved.get_living_entity().unwrap().health.load(), 3.0);
    restored.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_chunk_teleport_invalidates_retirement() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    server.worlds.store(Arc::new(vec![world.clone()]));
    publish_empty_chunk(&world, Vector2::new(0, 0));
    activate(&world).await;
    let mob = entity(&world, &EntityType::PIG);
    let id = mob.get_entity().entity_uuid;
    let commands = server.clone();
    *mob.completion.lock().unwrap() = Some(Box::new(move || {
        command(&commands, &format!("tp {id} 12.5 64 8.5"));
    }));
    world.level.unwatch_entity_chunks(&[Vector2::new(0, 0)]);
    world.tick(&server);
    assert!(!mob.get_entity().is_removed());
    world.tick(&server);
    assert!(mob.get_entity().is_removed());
    let restored = reload(&server, &world, dir.path()).await;
    assert_eq!(
        restored
            .get_entity_by_uuid(id)
            .unwrap()
            .get_entity()
            .pos
            .load()
            .x,
        12.5
    );
    restored.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_change_invalidates_retirement() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    publish_empty_chunk(&world, Vector2::new(0, 0));
    activate(&world).await;
    let mob = entity(&world, &EntityType::PIG);
    let id = mob.get_entity().entity_uuid;
    let changing = mob.inner.clone();
    *mob.completion.lock().unwrap() = Some(Box::new(move || {
        // The public living setter is also both Wasm hosts' health mutation entry point.
        changing.get_living_entity().unwrap().set_health(2.0);
    }));
    world.level.unwatch_entity_chunks(&[Vector2::new(0, 0)]);
    world.tick(&server);
    assert!(!mob.get_entity().is_removed());
    world.tick(&server);
    mob.get_living_entity().unwrap().set_health(9.0);
    assert_eq!(mob.get_living_entity().unwrap().health.load(), 2.0);
    mob.get_entity().remove();
    assert!(
        mob.get_entity().removal_reason.load()
            == Some(crate::entity::RemovalReason::UnloadedToChunk)
    );
    let restored = reload(&server, &world, dir.path()).await;
    assert_eq!(
        restored
            .get_entity_by_uuid(id)
            .unwrap()
            .get_living_entity()
            .unwrap()
            .health
            .load(),
        2.0
    );
    restored.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ride_command_invalidates_retirement_and_reloads_one_tree() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let world = world(&server, dir.path());
    server.worlds.store(Arc::new(vec![world.clone()]));
    publish_empty_chunk(&world, Vector2::new(0, 0));
    activate(&world).await;
    let vehicle = entity(&world, &EntityType::PIG);
    let passenger = entity(&world, &EntityType::SHEEP);
    let mount = vehicle.get_entity().entity_uuid;
    let rider = passenger.get_entity().entity_uuid;
    let commands = server.clone();
    *passenger.completion.lock().unwrap() = Some(Box::new(move || {
        command(&commands, &format!("ride {rider} mount {mount}"));
    }));
    world.level.unwatch_entity_chunks(&[Vector2::new(0, 0)]);
    world.tick(&server);
    assert!(!vehicle.get_entity().is_removed());
    assert!(!passenger.get_entity().is_removed());
    world.tick(&server);
    let restored = reload(&server, &world, dir.path()).await;
    let saved = restored.get_entity_by_uuid(rider).unwrap();
    assert_eq!(
        saved
            .get_entity()
            .get_vehicle()
            .unwrap()
            .get_entity()
            .entity_uuid,
        mount
    );
    assert_eq!(restored.entities.load().len(), 2);
    restored.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn projectile_transfer_invalidates_source_retirement() {
    let dir = tempfile::tempdir().unwrap();
    let destination_dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let source = world(&server, dir.path());
    let destination = world(&server, destination_dir.path());
    publish_empty_chunk(&source, Vector2::new(0, 0));
    publish_empty_chunk(&destination, Vector2::new(0, 0));
    activate(&source).await;
    activate(&destination).await;
    let projectile = entity(&source, &EntityType::SNOWBALL);
    let id = projectile.get_entity().entity_uuid;
    source.save().await;
    let storage = source
        .level
        .get_entity_chunk(Vector2::new(0, 0))
        .await
        .unwrap();
    let before = storage.data.lock().unwrap().clone();
    let retiring = source.clone();
    let ticks = server.clone();
    let reached = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reached_callback = reached.clone();
    *projectile.published.lock().unwrap() = Some((
        Arc::downgrade(&destination),
        Box::new(move || {
            let saving = retiring.clone();
            let (saved, completed) = std::sync::mpsc::sync_channel(0);
            tokio::spawn(async move {
                saving.save().await;
                saved.send(()).unwrap();
            });
            completed
                .recv_timeout(std::time::Duration::from_secs(60))
                .unwrap();
            let after = storage.data.lock().unwrap().clone();
            assert_eq!(after.len(), before.len());
            assert_eq!(after[0].get_list("Pos"), before[0].get_list("Pos"));
            // Real source tick while the same instance is already published in the destination,
            // but teleport_projectile has not removed it from the source registry yet.
            retiring.tick(&ticks);
            reached_callback.store(true, std::sync::atomic::Ordering::Release);
        }),
    ));
    let moving = projectile.inner.clone();
    let target = destination.clone();
    *projectile.completion.lock().unwrap() = Some(Box::new(move || {
        moving.teleport(Vector3::new(9.5, 64.0, 8.5), None, None, target);
    }));
    source.level.unwatch_entity_chunks(&[Vector2::new(0, 0)]);
    source.tick(&server);
    assert!(reached.load(std::sync::atomic::Ordering::Acquire));
    assert!(!projectile.get_entity().is_removed());
    assert!(source.get_entity_by_uuid(id).is_none());
    assert!(Arc::ptr_eq(
        &destination.get_entity_by_uuid(id).unwrap(),
        &(projectile.clone() as Arc<dyn EntityBase>)
    ));
    source.tick(&server);
    assert!(!projectile.get_entity().is_removed());
    let restored_source = reload(&server, &source, dir.path()).await;
    assert!(restored_source.get_entity_by_uuid(id).is_none());
    let restored_destination = reload(&server, &destination, destination_dir.path()).await;
    assert!(restored_destination.get_entity_by_uuid(id).is_some());
    restored_source.shutdown().await;
    restored_destination.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chest_without_entity_sidecar_survives_disconnect_and_terrain_unload() {
    use crate::net::java::combat_test_support::TestPlayer;
    use pumpkin_data::item::Item;
    use pumpkin_util::math::position::BlockPos;
    use pumpkin_world::chunk::{
        ChunkData,
        io::{Dirtiable, FileIO},
    };

    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let config = pumpkin_config::world::LevelConfig {
        lighting: pumpkin_config::lighting::LightingEngineConfig::Full,
        ..Default::default()
    };
    let world = crate::server::combat_test_support::world_with_config(&server, dir.path(), &config);
    server.worlds.store(Arc::new(vec![world.clone()]));
    let mut player = TestPlayer::new(&world);
    let chunks: Vec<_> = player
        .player
        .watched_section
        .load()
        .all_chunks_within()
        .collect();
    world.level.watch_entity_chunks(&chunks);
    let pos = Vector2::new(0, 0);
    let terrain = ChunkData::empty_sync(pos.x, pos.y);
    terrain.mark_dirty(true);
    world
        .level
        .chunk_saver
        .save_chunks(&world.level.level_folder, vec![(pos, terrain)])
        .await
        .unwrap();
    let terrain_ready = world.level.chunk_listener.add_single_chunk_listener(pos);
    hold_terrain_ticket(&world, &player.player, pos);
    tokio::time::timeout(std::time::Duration::from_secs(60), terrain_ready)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    // Delay the entity load at its async caller boundary; terrain/inventory operations proceed.
    let (complete_sidecar, sidecar_ready) = tokio::sync::oneshot::channel();
    let loading = world.level.clone();
    let sidecar = tokio::spawn(async move {
        sidecar_ready.await.unwrap();
        loading.get_entity_chunk(pos).await.unwrap()
    });
    command(&server, "setblock 8 64 8 minecraft:chest");
    command(
        &server,
        "item replace block 8 64 8 container.0 with minecraft:diamond 5",
    );
    let position = BlockPos::new(8, 64, 8);
    assert!(world.get_block_entity(&position).is_some());
    assert!(world.level.get_entity_chunk_sync(&pos).is_none());
    let unloaded = world.level.chunk_listener.next_unload(pos);
    let leaving = player.player.clone();
    player.with_outgoing_writer(leaving.remove()).await;
    world.tick(&server);
    // This completes only on the real GenerationSchedule terrain-unload path.
    tokio::time::timeout(std::time::Duration::from_secs(60), unloaded)
        .await
        .unwrap()
        .unwrap();
    assert!(!world.level.is_chunk_loaded(&pos));
    complete_sidecar.send(()).unwrap();
    sidecar.await.unwrap();
    world.shutdown().await;
    let restored =
        crate::server::combat_test_support::world_with_config(&server, dir.path(), &config);
    restored.level.watch_entity_chunks(&[pos]);
    restored
        .level
        .get_or_fetch_chunk(pos, |_| ())
        .await
        .unwrap();
    let chest = restored.get_block_entity(&position).unwrap();
    let inventory = chest.get_inventory().unwrap();
    let stack = inventory.get_stack(0);
    assert_eq!(stack.item.id, Item::DIAMOND.id);
    assert_eq!(stack.item_count, 5);
    restored.shutdown().await;
}
