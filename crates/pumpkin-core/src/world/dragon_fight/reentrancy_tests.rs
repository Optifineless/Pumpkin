use super::{respawn_tests::Fixture, *};
use crate::{
    plugin::{EventHandler, EventPriority, api::events::entity::entity_spawn::EntitySpawnEvent},
    server::Server,
};
use futures::future::BoxFuture;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

impl Fixture {
    pub(super) fn with_locked_fight(&self, operation: impl FnOnce(&mut DragonFight)) {
        let mut fight = self.world.dragon_fight.as_ref().unwrap().lock().unwrap();
        operation(&mut fight);
    }
}

struct SpawnHandler {
    attempts: AtomicUsize,
    reenter: bool,
    kill: bool,
    cancel: AtomicBool,
}

impl EventHandler<EntitySpawnEvent> for SpawnHandler {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntitySpawnEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if event.entity_type != EntityType::ENDER_DRAGON.id.to_string() {
                return;
            }
            let attempt = self.attempts.fetch_add(1, Ordering::Relaxed);
            let world = &event.world;
            let mutex = world.dragon_fight.as_ref().unwrap();
            assert!(
                mutex.try_lock().is_ok(),
                "spawn callback holds the fight lock"
            );
            // Re-enter once so a broken reservation fails by counting, without an infinite queue.
            if self.reenter && attempt == 0 {
                DragonFight::tick(mutex, world);
                DragonFight::with_fight(mutex, world, |fight| {
                    fight.find_or_create_dragon(world);
                    fight.create_new_dragon(world);
                    if fight.respawn_stage.is_some() {
                        fight.set_respawn_stage(world, DragonRespawnStage::End);
                    }
                });
            }
            if self.kill {
                DragonFight::with_fight(mutex, world, |fight| {
                    fight.set_dragon_killed(world, Uuid::new_v4());
                });
            }
            event.cancelled = self.cancel.load(Ordering::Relaxed);
        })
    }
}

fn handler(fixture: &Fixture, reenter: bool, kill: bool, cancel: bool) -> Arc<SpawnHandler> {
    let handler = Arc::new(SpawnHandler {
        attempts: AtomicUsize::new(0),
        reenter,
        kill,
        cancel: AtomicBool::new(cancel),
    });
    fixture
        .server
        .plugin_manager
        .register::<EntitySpawnEvent, _>(handler.clone(), EventPriority::Normal, true);
    handler
}

fn ordinary(fixture: &Fixture) {
    let mut fight = fixture.world.dragon_fight.as_ref().unwrap().lock().unwrap();
    fight.respawn_stage = None;
    fight.dragon_killed = false;
}

fn dragons(fixture: &Fixture) -> Vec<Uuid> {
    fixture
        .world
        .entities
        .load()
        .iter()
        .filter(|entity| entity.get_entity().entity_type == &EntityType::ENDER_DRAGON)
        .map(|entity| entity.get_entity().entity_uuid)
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reentrant_dragon_creation_publishes_one_dragon() {
    for respawning in [false, true] {
        let fixture = Fixture::new(DragonRespawnStage::SummoningDragon, 100);
        fixture.crystal(Vector3::new(0.5, 65.0, 3.5), true);
        if !respawning {
            ordinary(&fixture);
        }
        let handler = handler(&fixture, true, false, false);
        DragonFight::tick(fixture.world.dragon_fight.as_ref().unwrap(), &fixture.world);
        let dragons = dragons(&fixture);
        assert_eq!(dragons.len(), 1, "respawning={respawning}");
        assert_eq!(handler.attempts.load(Ordering::Relaxed), 1);
        fixture.with_locked_fight(|fight| {
            assert_eq!(fight.dragon_uuid, Some(dragons[0]));
            assert!(!fight.dragon_killed && fight.respawn_stage.is_none());
        });
        fixture.world.level.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_respawn_finishes_once_and_consumes_crystals() {
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
    let handler = handler(&fixture, true, false, true);
    DragonFight::tick(fixture.world.dragon_fight.as_ref().unwrap(), &fixture.world);
    assert_eq!(handler.attempts.load(Ordering::Relaxed), 1);
    assert!(dragons(&fixture).is_empty());
    let uuid = {
        let fight = fixture.world.dragon_fight.as_ref().unwrap().lock().unwrap();
        assert!(fight.respawn_stage.is_none() && !fight.dragon_killed);
        assert_eq!(fight.respawn_time, 0);
        fight.dragon_uuid.unwrap()
    };
    assert!(fixture.world.get_entity_by_uuid(uuid).is_none());
    assert!(
        crystals.iter().all(|crystal| {
            crystal.get_entity().is_removed() && crystal.beam_target().is_none()
        })
    );
    assert!(!spike_crystal.is_invulnerable() && spike_crystal.beam_target().is_none());
    for _ in 0..TIME_BETWEEN_CRYSTAL_SCANS {
        DragonFight::tick(fixture.world.dragon_fight.as_ref().unwrap(), &fixture.world);
    }
    assert_eq!(handler.attempts.load(Ordering::Relaxed), 1);
    // Vanilla tracks the failed attempt and uses the ordinary missing-dragon recovery interval.
    handler.cancel.store(false, Ordering::Relaxed);
    DragonFight::with_fight(
        fixture.world.dragon_fight.as_ref().unwrap(),
        &fixture.world,
        |fight| fight.ticks_since_dragon_seen = MAX_TICKS_BEFORE_DRAGON_RESPAWN,
    );
    DragonFight::tick(fixture.world.dragon_fight.as_ref().unwrap(), &fixture.world);
    assert_eq!(handler.attempts.load(Ordering::Relaxed), 2);
    assert_eq!(dragons(&fixture).len(), 1);
    fixture.world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_ordinary_creation_releases_reservation() {
    let fixture = Fixture::new(DragonRespawnStage::Start, 0);
    ordinary(&fixture);
    let handler = handler(&fixture, true, false, true);
    for _ in 0..2 {
        DragonFight::tick(fixture.world.dragon_fight.as_ref().unwrap(), &fixture.world);
    }
    assert_eq!(handler.attempts.load(Ordering::Relaxed), 1);
    for _ in 2..MAX_TICKS_BEFORE_DRAGON_RESPAWN {
        DragonFight::tick(fixture.world.dragon_fight.as_ref().unwrap(), &fixture.world);
    }
    assert_eq!(handler.attempts.load(Ordering::Relaxed), 1);
    assert!(dragons(&fixture).is_empty());
    fixture.with_locked_fight(|fight| {
        assert!(fight.pending_dragon_spawn.is_none());
        assert!(fight.dragon_uuid.is_some() && !fight.dragon_killed);
    });
    handler.cancel.store(false, Ordering::Relaxed);
    // EnderDragonFight.tick retries exactly 1,200 ticks after the failed attempt.
    DragonFight::tick(fixture.world.dragon_fight.as_ref().unwrap(), &fixture.world);
    assert_eq!(handler.attempts.load(Ordering::Relaxed), 2);
    assert_eq!(dragons(&fixture).len(), 1);
    fixture.world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalidated_dragon_creation_is_not_published() {
    let fixture = Fixture::new(DragonRespawnStage::Start, 0);
    ordinary(&fixture);
    let handler = handler(&fixture, false, true, false);
    DragonFight::tick(fixture.world.dragon_fight.as_ref().unwrap(), &fixture.world);
    assert_eq!(handler.attempts.load(Ordering::Relaxed), 1);
    assert!(dragons(&fixture).is_empty());
    fixture.with_locked_fight(|fight| {
        assert!(fight.dragon_killed && fight.dragon_uuid.is_none());
    });
    fixture.world.level.shutdown().await.unwrap();
}
