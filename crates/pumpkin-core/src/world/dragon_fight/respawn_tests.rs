use super::*;
use crate::{
    net::java::combat_test_support::TestPlayer,
    plugin::{
        EventHandler, EventPriority, api::events::entity::entity_explode::EntityExplodeEvent,
    },
    server::{Server, combat_test_support},
};
use futures::future::BoxFuture;
use pumpkin_config::world::LevelConfig;
use pumpkin_data::{damage::DamageType, dimension::Dimension};
use pumpkin_world::level::Level;
use std::{sync::mpsc, thread, time::Duration};

struct Fixture {
    world: Arc<World>,
    server: Arc<Server>,
    _player: TestPlayer,
    _dir: tempfile::TempDir,
}

impl Fixture {
    fn new(stage: DragonRespawnStage, time: i32) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(dir.path());
        let world = Arc::new(World::load(
            Level::from_root_folder(
                &LevelConfig::default(),
                dir.path().to_path_buf(),
                0,
                Dimension::THE_END,
            ),
            server.level_info.clone(),
            Dimension::THE_END,
            server.block_registry.clone(),
            Arc::downgrade(&server),
        ));
        let player = TestPlayer::new(&world);
        player
            .player
            .get_entity()
            .set_pos(Vector3::new(0.0, 128.0, 0.0));
        {
            let mut fight = world.dragon_fight.as_ref().unwrap().lock().unwrap();
            fight.needs_state_scanning = false;
            fight.skip_arena_loaded_check();
            fight.previously_killed = true;
            fight.dragon_killed = true;
            fight.exit_portal_location = Some(BlockPos::new(0, 64, 0));
            fight.respawn_stage = Some(stage);
            fight.respawn_time = time;
        };
        let fixture = Self {
            world,
            server,
            _player: player,
            _dir: dir,
        };
        fixture.publish_area(0, 0);
        fixture
    }

    fn publish_area(&self, x: i32, z: i32) {
        for cx in (x - 12) >> 4..=(x + 12) >> 4 {
            for cz in (z - 12) >> 4..=(z + 12) >> 4 {
                let pos = Vector2::new(cx, cz);
                if !self.world.level.is_chunk_loaded(&pos) {
                    combat_test_support::publish_empty_chunk(&self.world, pos);
                }
            }
        }
    }

    fn crystal(&self, pos: Vector3<f64>, respawn: bool) -> Arc<EndCrystalEntity> {
        let crystal = Arc::new(EndCrystalEntity::new(Entity::new(
            self.world.clone(),
            pos,
            &EntityType::END_CRYSTAL,
        )));
        self.world.add_entity_silent(crystal.clone());
        if respawn {
            self.world
                .dragon_fight
                .as_ref()
                .unwrap()
                .lock()
                .unwrap()
                .respawn_crystals
                .push(crystal.get_entity().entity_uuid);
        }
        crystal
    }
}

fn completes_within_timeout(operation: impl FnOnce() + Send + 'static) {
    let (send, recv) = mpsc::channel();
    let runtime = tokio::runtime::Handle::current();
    let worker = thread::spawn(move || {
        let _runtime = runtime.enter();
        operation();
        send.send(()).unwrap();
    });
    // A stuck worker is detached on timeout; never join a deadlocked thread.
    assert_eq!(
        recv.recv_timeout(Duration::from_secs(5)),
        Ok(()),
        "dragon fight operation timed out"
    );
    worker.join().unwrap();
}

struct Observation {
    source_id: i32,
    unlocked: bool,
    finished: bool,
    spikes_reset: bool,
}

struct ExplosionObserver {
    world: Arc<World>,
    spike_crystal: Arc<EndCrystalEntity>,
    observed: Mutex<Vec<Observation>>,
}

impl EventHandler<EntityExplodeEvent> for ExplosionObserver {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntityExplodeEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let fight = self.world.dragon_fight.as_ref().unwrap().try_lock();
            self.observed.lock().unwrap().push(Observation {
                source_id: event.entity_id,
                unlocked: fight.is_ok(),
                finished: fight.as_ref().is_ok_and(|fight| {
                    fight.respawn_stage.is_none()
                        && !fight.dragon_killed
                        && fight.dragon_uuid.is_some()
                }),
                spikes_reset: !self.spike_crystal.is_invulnerable()
                    && self.spike_crystal.beam_target().is_none(),
            });
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn summoning_pillars_explosion_completes_without_deadlock() {
    let fixture = Fixture::new(DragonRespawnStage::SummoningPillars, 39);
    let respawn = fixture.crystal(Vector3::new(0.5, 65.0, 3.5), true);
    let spike = DragonFight::get_spikes()[0];
    fixture.publish_area(spike.center_x, spike.center_z);
    let pos = Vector3::new(
        f64::from(spike.center_x) + 0.5,
        f64::from(spike.height + 1),
        f64::from(spike.center_z) + 0.5,
    );
    let old_crystal = fixture.crystal(pos, false);
    let observer = Arc::new(ExplosionObserver {
        world: fixture.world.clone(),
        spike_crystal: old_crystal.clone(),
        observed: Mutex::default(),
    });
    fixture
        .server
        .plugin_manager
        .register::<EntityExplodeEvent, _>(observer.clone(), EventPriority::Normal, true);
    let world = fixture.world.clone();
    completes_within_timeout(move || {
        DragonFight::tick(world.dragon_fight.as_ref().unwrap(), &world);
    });
    assert!(observer.observed.lock().unwrap()[0].unlocked);
    assert!(old_crystal.get_entity().is_removed());
    assert!(!respawn.get_entity().is_removed());
    assert!(fixture.world.entities.load().iter().any(|entity| {
        entity.get_entity().entity_type == &EntityType::END_CRYSTAL
            && entity.get_entity().pos.load() == pos
            && entity.get_entity().entity_uuid != old_crystal.get_entity().entity_uuid
    }));
    {
        let fight = fixture.world.dragon_fight.as_ref().unwrap().lock().unwrap();
        assert_eq!(
            fight.respawn_stage,
            Some(DragonRespawnStage::SummoningPillars)
        );
        assert_eq!(fight.respawn_time, 40);
    };
    fixture.world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn summoning_dragon_explodes_and_removes_crystals_without_deadlock() {
    let fixture = Fixture::new(DragonRespawnStage::SummoningDragon, 100);
    let crystals: Vec<_> = [(0.5, -2.5), (0.5, 3.5), (-2.5, 0.5), (3.5, 0.5)]
        .into_iter()
        .map(|(x, z)| fixture.crystal(Vector3::new(x, 65.0, z), true))
        .collect();
    for crystal in &crystals {
        crystal.set_beam_target(Some(BlockPos::new(0, 128, 0)));
    }
    let spike = DragonFight::get_spikes()[0];
    let spike_crystal = fixture.crystal(
        Vector3::new(
            f64::from(spike.center_x) + 0.5,
            f64::from(spike.height + 1),
            f64::from(spike.center_z) + 0.5,
        ),
        false,
    );
    spike_crystal.set_invulnerable(true);
    spike_crystal.set_beam_target(Some(BlockPos::new(0, 128, 0)));
    let observer = Arc::new(ExplosionObserver {
        world: fixture.world.clone(),
        spike_crystal,
        observed: Mutex::default(),
    });
    fixture
        .server
        .plugin_manager
        .register::<EntityExplodeEvent, _>(observer.clone(), EventPriority::Normal, true);
    let world = fixture.world.clone();
    completes_within_timeout(move || {
        DragonFight::tick(world.dragon_fight.as_ref().unwrap(), &world);
    });
    assert!(
        crystals
            .iter()
            .all(|crystal| crystal.get_entity().is_removed())
    );
    assert!(
        crystals
            .iter()
            .all(|crystal| crystal.beam_target().is_none())
    );
    {
        let observed = observer.observed.lock().unwrap();
        assert_eq!(observed.len(), crystals.len());
        for (event, crystal) in observed.iter().zip(&crystals) {
            assert_eq!(event.source_id, crystal.get_entity().entity_id);
            assert!(event.unlocked && event.finished && event.spikes_reset);
        }
    };
    {
        let fight = fixture.world.dragon_fight.as_ref().unwrap().lock().unwrap();
        assert!(fight.respawn_stage.is_none());
        let dragon = fixture
            .world
            .get_entity_by_uuid(fight.dragon_uuid.unwrap())
            .unwrap();
        assert_eq!(dragon.get_entity().entity_type, &EntityType::ENDER_DRAGON);
    };
    fixture.world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn destroying_respawn_crystal_aborts_before_animation() {
    for locked_callback in [false, true] {
        let fixture = Fixture::new(DragonRespawnStage::SummoningPillars, 39);
        let crystal = fixture.crystal(Vector3::new(0.5, 65.0, 3.5), true);
        let world = fixture.world.clone();
        completes_within_timeout(move || {
            // Spawn callbacks run under this guard during DragonFight::scan_state.
            let guard =
                locked_callback.then(|| world.dragon_fight.as_ref().unwrap().lock().unwrap());
            assert!(crystal.damage_with_context(
                crystal.as_ref(),
                1.0,
                DamageType::GENERIC,
                None,
                None,
                None,
            ));
            drop(guard);
            DragonFight::tick(world.dragon_fight.as_ref().unwrap(), &world);
        });
        {
            let fight = fixture.world.dragon_fight.as_ref().unwrap().lock().unwrap();
            assert!(fight.respawn_stage.is_none());
            assert!(fight.dragon_killed && fight.dragon_uuid.is_none());
            assert_eq!(fight.respawn_time, 0);
        };
        assert_eq!(
            fixture.world.get_block(&BlockPos::new(1, 64, 0)),
            &Block::END_PORTAL
        );
        fixture.world.level.shutdown().await.unwrap();
    }
}
