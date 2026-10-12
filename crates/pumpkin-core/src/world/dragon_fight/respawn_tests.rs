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
use std::sync::Mutex;

pub(super) struct Fixture {
    pub(super) world: Arc<World>,
    pub(super) server: Arc<Server>,
    _player: TestPlayer,
    _dir: tempfile::TempDir,
}

impl Fixture {
    pub(super) fn new(stage: DragonRespawnStage, time: i32) -> Self {
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

    pub(super) fn publish_area(&self, x: i32, z: i32) {
        for cx in (x - 12) >> 4..=(x + 12) >> 4 {
            for cz in (z - 12) >> 4..=(z + 12) >> 4 {
                let pos = Vector2::new(cx, cz);
                if !self.world.level.is_chunk_loaded(&pos) {
                    combat_test_support::publish_empty_chunk(&self.world, pos);
                }
            }
        }
    }

    pub(super) fn crystal(&self, pos: Vector3<f64>, respawn: bool) -> Arc<EndCrystalEntity> {
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
            assert!(
                fight.is_ok(),
                "explosion callback would block on dragon_fight"
            );
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
    DragonFight::tick(fixture.world.dragon_fight.as_ref().unwrap(), &fixture.world);
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
    DragonFight::tick(fixture.world.dragon_fight.as_ref().unwrap(), &fixture.world);
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
        {
            // Simulate a notification contending with a fight state update.
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
        };
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

mod inflight_review_tests {
    use super::super::fresh_review_tests::finish_dragon_death;
    use super::{Fixture, *};
    use crate::{
        block::entities::chest::ChestBlockEntity,
        plugin::{
            EventHandler, EventPriority,
            api::events::entity::{
                entity_explode::EntityExplodeEvent, entity_spawn::EntitySpawnEvent,
            },
        },
        server::Server,
    };
    use futures::future::BoxFuture;
    use pumpkin_data::{damage::DamageType, item::Item, item_stack::ItemStack};
    use pumpkin_inventory::Inventory;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn filled_chest(fixture: &Fixture, pos: BlockPos) {
        fixture.world.set_block_state(
            &pos,
            Block::CHEST.default_state.id,
            BlockFlags::NOTIFY_LISTENERS,
        );
        let chest = Arc::new(ChestBlockEntity::new(pos));
        chest.set_stack(0, ItemStack::new(7, &Item::DIAMOND));
        fixture.world.add_block_entity(chest);
    }

    struct InFlightAbort {
        participant: Arc<EndCrystalEntity>,
        spike_center: Vector3<f64>,
        applied: AtomicBool,
        spike_explosions: AtomicUsize,
        spike_crystal_spawns: AtomicUsize,
    }

    impl EventHandler<EntitySpawnEvent> for InFlightAbort {
        fn handle_blocking<'a>(
            &'a self,
            _: &'a Arc<Server>,
            event: &'a mut EntitySpawnEvent,
        ) -> BoxFuture<'a, ()> {
            Box::pin(async move {
                if event.entity_type == EntityType::END_CRYSTAL.id.to_string()
                    && event.position.x == self.spike_center.x
                    && event.position.z == self.spike_center.z
                {
                    self.spike_crystal_spawns.fetch_add(1, Ordering::Relaxed);
                }
                if event.entity_type != EntityType::ITEM.id.to_string()
                    || self.applied.swap(true, Ordering::Relaxed)
                {
                    return;
                }
                let mutex = event.world.dragon_fight.as_ref().unwrap();
                assert!(
                    mutex.try_lock().is_ok(),
                    "spike container callback held fight ownership"
                );
                assert!(self.participant.damage_with_context(
                    self.participant.as_ref(),
                    1.0,
                    DamageType::EXPLOSION,
                    None,
                    None,
                    None,
                ));
                assert!(self.participant.get_entity().is_removed());
                assert!(
                    mutex.lock().unwrap().respawn_stage.is_none(),
                    "actual in-flight participant destruction did not abort respawn"
                );
            })
        }
    }

    impl EventHandler<EntityExplodeEvent> for InFlightAbort {
        fn handle_blocking<'a>(
            &'a self,
            _: &'a Arc<Server>,
            event: &'a mut EntityExplodeEvent,
        ) -> BoxFuture<'a, ()> {
            Box::pin(async move {
                if event.position == self.spike_center {
                    self.spike_explosions.fetch_add(1, Ordering::Relaxed);
                }
            })
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn native_spike_container_abort_stops_remaining_destructive_effects() {
        let fixture = Fixture::new(DragonRespawnStage::SummoningPillars, 39);
        let participants: Vec<_> = [(0, -3), (0, 3), (-3, 0), (3, 0)]
            .into_iter()
            .map(|(x, z)| {
                fixture.crystal(
                    Vector3::new(f64::from(x) + 0.5, 65.0, f64::from(z) + 0.5),
                    true,
                )
            })
            .collect();
        let spike = DragonFight::get_spikes()[0];
        fixture.publish_area(spike.center_x, spike.center_z);
        let chest = BlockPos::new(spike.center_x - 10, spike.height - 10, spike.center_z - 10);
        filled_chest(&fixture, chest);
        let sentinel = BlockPos::new(spike.center_x + 10, spike.height + 10, spike.center_z + 10);
        fixture.world.set_block_state(
            &sentinel,
            Block::STONE.default_state.id,
            BlockFlags::NOTIFY_LISTENERS,
        );
        let observer = Arc::new(InFlightAbort {
            participant: participants[0].clone(),
            spike_center: Vector3::new(
                f64::from(spike.center_x) + 0.5,
                f64::from(spike.height),
                f64::from(spike.center_z) + 0.5,
            ),
            applied: AtomicBool::new(false),
            spike_explosions: AtomicUsize::new(0),
            spike_crystal_spawns: AtomicUsize::new(0),
        });
        fixture
            .server
            .plugin_manager
            .register::<EntitySpawnEvent, _>(observer.clone(), EventPriority::Normal, true);
        fixture
            .server
            .plugin_manager
            .register::<EntityExplodeEvent, _>(observer.clone(), EventPriority::Normal, true);
        fixture.with_locked_fight(|fight| assert_eq!(fight.respawn_crystals.len(), 4));
        DragonFight::tick(fixture.world.dragon_fight.as_ref().unwrap(), &fixture.world);
        assert!(
            observer.applied.load(Ordering::Relaxed),
            "actual spike chest item-spawn callback did not run"
        );
        fixture.with_locked_fight(|fight| {
            assert!(fight.respawn_stage.is_none() && fight.dragon_killed);
            assert_eq!(fight.respawn_time, 0);
        });
        let sentinel_block = fixture.world.get_block(&sentinel).id;
        let explosions = observer.spike_explosions.load(Ordering::Relaxed);
        let spawns = observer.spike_crystal_spawns.load(Ordering::Relaxed);
        let new_crystal = fixture.world.entities.load().iter().any(|entity| {
            let pos = entity.get_entity().pos.load();
            entity.get_entity().entity_type == &EntityType::END_CRYSTAL
                && pos.x == observer.spike_center.x
                && pos.z == observer.spike_center.z
        });
        fixture.world.level.shutdown().await.unwrap();
        tracing::info!(
            "IN_FLIGHT_OUTCOME sentinel={sentinel_block} explosions={explosions} crystal_spawns={spawns} new_crystal={new_crystal}"
        );
        assert_eq!(
            explosions, 0,
            "aborted in-flight spike still executed a subsequent explosion"
        );
        assert_eq!(
            sentinel_block,
            Block::STONE.id,
            "aborted in-flight spike continued clearing after its container callback"
        );
        assert_eq!(
            spawns, 0,
            "aborted in-flight spike still began a crystal admission"
        );
        assert!(
            !new_crystal,
            "aborted in-flight spike published a new crystal"
        );
    }

    struct LoadedDragonDeath {
        dragon: Arc<dyn EntityBase>,
        applied: AtomicBool,
        replacement_spawns: AtomicUsize,
    }

    impl EventHandler<EntitySpawnEvent> for LoadedDragonDeath {
        fn handle_blocking<'a>(
            &'a self,
            server: &'a Arc<Server>,
            event: &'a mut EntitySpawnEvent,
        ) -> BoxFuture<'a, ()> {
            Box::pin(async move {
                if event.entity_type == EntityType::ENDER_DRAGON.id.to_string() {
                    self.replacement_spawns.fetch_add(1, Ordering::Relaxed);
                }
                if event.entity_type != EntityType::END_CRYSTAL.id.to_string()
                    || self.applied.swap(true, Ordering::Relaxed)
                {
                    return;
                }
                assert!(
                    event
                        .world
                        .dragon_fight
                        .as_ref()
                        .unwrap()
                        .try_lock()
                        .is_ok()
                );
                let admitted = event
                    .world
                    .get_entity_by_uuid(self.dragon.get_entity().entity_uuid)
                    .unwrap();
                assert!(Arc::ptr_eq(&admitted, &self.dragon));
                assert!(!self.dragon.get_entity().is_removed());
                finish_dragon_death(&self.dragon, server);
            })
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn initial_crystal_callback_death_preserves_loaded_dragon_rewards() {
        let fixture = Fixture::new(DragonRespawnStage::Start, 0);
        fixture.publish_area(GATEWAY_DISTANCE as i32, 0);
        for spike in DragonFight::get_spikes() {
            fixture.publish_area(spike.center_x, spike.center_z);
        }
        let dragon = crate::entity::r#type::from_type(
            &EntityType::ENDER_DRAGON,
            Vector3::new(8.5, 128.0, 8.5),
            &fixture.world,
            Uuid::new_v4(),
        );
        fixture.world.add_entity_silent(dragon.clone());
        fixture.with_locked_fight(|fight| {
            fight.needs_state_scanning = true;
            fight.respawn_stage = None;
            fight.dragon_killed = false;
            fight.previously_killed = false;
            fight.dragon_uuid = None;
            fight.gateways = vec![0];
        });
        assert!(!DragonFight::has_active_exit_portal(&fixture.world));
        let observer = Arc::new(LoadedDragonDeath {
            dragon: dragon.clone(),
            applied: AtomicBool::new(false),
            replacement_spawns: AtomicUsize::new(0),
        });
        fixture
            .server
            .plugin_manager
            .register::<EntitySpawnEvent, _>(observer.clone(), EventPriority::Normal, true);
        DragonFight::tick(fixture.world.dragon_fight.as_ref().unwrap(), &fixture.world);
        DragonFight::tick(fixture.world.dragon_fight.as_ref().unwrap(), &fixture.world);
        let state = {
            let fight = fixture.world.dragon_fight.as_ref().unwrap().lock().unwrap();
            (
                fight.dragon_killed,
                fight.previously_killed,
                fight.dragon_uuid,
                fight.pending_dragon_spawn,
                fight.gateways.clone(),
            )
        };
        let egg = fixture.world.get_block(&BlockPos::new(0, 68, 0)).id;
        let portal = fixture.world.get_block(&BlockPos::new(1, 64, 0)).id;
        let live_dragons = fixture
            .world
            .entities
            .load()
            .iter()
            .filter(|entity| {
                entity.get_entity().entity_type == &EntityType::ENDER_DRAGON
                    && !entity.get_entity().is_removed()
            })
            .count();
        let replacements = observer.replacement_spawns.load(Ordering::Relaxed);
        fixture.world.level.shutdown().await.unwrap();
        assert!(
            observer.applied.load(Ordering::Relaxed) && dragon.get_entity().is_removed(),
            "actual initial crystal callback did not finish the loaded dragon death"
        );
        tracing::info!(
            "INITIAL_SCAN_OUTCOME killed={} previously_killed={} tracked={:?} pending={:?} gateways={:?} egg={egg} portal={portal} live_dragons={live_dragons} replacements={replacements}",
            state.0,
            state.1,
            state.2,
            state.3,
            state.4
        );
        assert!(
            state.0 && state.1 && state.2.is_none() && state.3.is_none(),
            "initial scan callback lost the loaded dragon's completed death"
        );
        assert!(
            state.4.is_empty(),
            "initial scan callback lost the first-kill gateway"
        );
        assert_eq!(egg, Block::DRAGON_EGG.id);
        assert_eq!(portal, Block::END_PORTAL.id);
        assert_eq!(live_dragons, 0);
        assert_eq!(
            replacements, 0,
            "initial scan replaced the genuinely killed loaded dragon"
        );
    }
}
