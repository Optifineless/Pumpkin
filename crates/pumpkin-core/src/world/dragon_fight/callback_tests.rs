use super::{respawn_tests::Fixture, *};
use crate::{
    block::entities::chest::ChestBlockEntity,
    plugin::{EventHandler, EventPriority, api::events::entity::entity_spawn::EntitySpawnEvent},
    server::Server,
};
use futures::future::BoxFuture;
use pumpkin_data::{item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

thread_local! {
    static BEFORE_ACTIONS: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
}

pub(super) fn observe_before_actions() {
    if let Some(observer) = BEFORE_ACTIONS.with_borrow_mut(Option::take) {
        observer();
    }
}

struct FightReader {
    spawns: Mutex<Vec<String>>,
}

impl EventHandler<EntitySpawnEvent> for FightReader {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntitySpawnEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            // A plugin may legitimately read the fight during any spawn, including container drops.
            let fight = event.world.dragon_fight.as_ref().unwrap().try_lock();
            assert!(fight.is_ok(), "spawn callback would block on dragon_fight");
            self.spawns.lock().unwrap().push(event.entity_type.clone());
        })
    }
}

fn reader(fixture: &Fixture) -> Arc<FightReader> {
    let reader = Arc::new(FightReader {
        spawns: Mutex::default(),
    });
    fixture
        .server
        .plugin_manager
        .register::<EntitySpawnEvent, _>(reader.clone(), EventPriority::Normal, true);
    reader
}

fn chest(fixture: &Fixture, pos: BlockPos) {
    fixture.world.set_block_state(
        &pos,
        Block::CHEST.default_state.id,
        BlockFlags::NOTIFY_LISTENERS,
    );
    let chest = Arc::new(ChestBlockEntity::new(pos));
    chest.set_stack(0, ItemStack::new(7, &Item::DIAMOND));
    fixture.world.add_block_entity(chest);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dragon_creation_callbacks_can_lock_fight() {
    for completion in [false, true] {
        let fixture = Fixture::new(DragonRespawnStage::SummoningDragon, 100);
        fixture.crystal(Vector3::new(0.5, 65.0, 3.5), true);
        if !completion {
            let mut fight = fixture.world.dragon_fight.as_ref().unwrap().lock().unwrap();
            fight.respawn_stage = None;
            fight.dragon_killed = false;
        }
        let reader = reader(&fixture);
        DragonFight::tick(fixture.world.dragon_fight.as_ref().unwrap(), &fixture.world);
        assert_eq!(
            reader.spawns.lock().unwrap().as_slice(),
            &[EntityType::ENDER_DRAGON.id.to_string()]
        );
        assert!(
            fixture
                .world
                .entities
                .load()
                .iter()
                .any(|e| { e.get_entity().entity_type == &EntityType::ENDER_DRAGON })
        );
        fixture.world.level.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scan_and_spike_spawn_callbacks_can_lock_fight() {
    for scanning in [false, true] {
        let fixture = Fixture::new(DragonRespawnStage::SummoningPillars, 39);
        fixture.crystal(Vector3::new(0.5, 65.0, 3.5), true);
        for spike in DragonFight::get_spikes() {
            fixture.publish_area(spike.center_x, spike.center_z);
        }
        if scanning {
            let mut fight = fixture.world.dragon_fight.as_ref().unwrap().lock().unwrap();
            fight.needs_state_scanning = true;
            fight.respawn_stage = None;
        }
        let reader = reader(&fixture);
        DragonFight::tick(fixture.world.dragon_fight.as_ref().unwrap(), &fixture.world);
        assert!(
            reader
                .spawns
                .lock()
                .unwrap()
                .contains(&EntityType::END_CRYSTAL.id.to_string())
        );
        fixture.world.level.shutdown().await.unwrap();
    }
}

fn portal_fixture(operation: &str) -> Fixture {
    let fixture = Fixture::new(DragonRespawnStage::SummoningDragon, 100);
    chest(&fixture, BlockPos::new(1, 64, 0));
    if operation != "find_portal" {
        for (x, z) in [(0.5, -2.5), (0.5, 3.5), (-2.5, 0.5), (3.5, 0.5)] {
            fixture.crystal(Vector3::new(x, 65.0, z), true);
        }
    }
    {
        let mut fight = fixture.world.dragon_fight.as_ref().unwrap().lock().unwrap();
        if operation == "find_portal" {
            fight.exit_portal_location = None;
        }
        if operation != "abort" {
            fight.respawn_stage = None;
        }
        if operation == "scan" || operation == "create" {
            fight.previously_killed = false;
        }
        if operation == "kill" {
            fight.dragon_killed = false;
            fight.previously_killed = false;
            fight.gateways = vec![0];
        }
    }
    if operation == "kill" {
        fixture.publish_area(GATEWAY_DISTANCE as i32, 0);
        chest(
            &fixture,
            BlockPos::new(GATEWAY_DISTANCE as i32, GATEWAY_Y, 0),
        );
        chest(&fixture, BlockPos::new(0, 68, 0));
    }
    fixture
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn portal_placement_callbacks_can_lock_fight() {
    for operation in [
        "kill",
        "try_respawn",
        "abort",
        "scan",
        "create",
        "find_portal",
    ] {
        let fixture = portal_fixture(operation);
        if operation == "find_portal" {
            fixture
                .server
                .plugin_manager
                .register::<EntitySpawnEvent, _>(
                    Arc::new(PortalCrystals),
                    EventPriority::Normal,
                    true,
                );
        }
        let reader = reader(&fixture);
        let world = fixture.world.clone();
        {
            DragonFight::with_fight(world.dragon_fight.as_ref().unwrap(), &world, |fight| {
                match operation {
                    "kill" => fight.set_dragon_killed(&world, Uuid::new_v4()),
                    "try_respawn" | "find_portal" => fight.try_respawn(&world),
                    "abort" => fight.abort_respawn_sequence(&world),
                    "scan" => fight.scan_state(&world),
                    "create" => {
                        fight.create_new_dragon(&world);
                    }
                    _ => panic!("unknown test operation"),
                }
            });
        };
        assert!(
            reader
                .spawns
                .lock()
                .unwrap()
                .contains(&EntityType::ITEM.id.to_string()),
            "{operation}"
        );
        assert!(
            fixture
                .world
                .entities
                .load()
                .iter()
                .any(|e| { e.get_entity().entity_type == &EntityType::ITEM })
        );
        if operation == "kill" {
            assert_eq!(reader.spawns.lock().unwrap().len(), 3);
        }
        if operation == "find_portal" {
            let fight = fixture.world.dragon_fight.as_ref().unwrap().lock().unwrap();
            assert_eq!(fight.respawn_stage, Some(DragonRespawnStage::Start));
            assert_eq!(fight.respawn_crystals.len(), 4);
        }
        fixture.world.level.shutdown().await.unwrap();
    }
}

struct PortalCrystals;

impl EventHandler<EntitySpawnEvent> for PortalCrystals {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntitySpawnEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if event.entity_type != EntityType::ITEM.id.to_string() {
                return;
            }
            let portal = event
                .world
                .dragon_fight
                .as_ref()
                .unwrap()
                .try_lock()
                .expect("portal callback would block on dragon_fight")
                .exit_portal_location
                .unwrap();
            // EnderDragonFight.tryRespawn looks three blocks out from portal.above(1).
            for (x, z) in [(0.5, -2.5), (0.5, 3.5), (-2.5, 0.5), (3.5, 0.5)] {
                let entity = Entity::new(
                    event.world.clone(),
                    Vector3::new(
                        f64::from(portal.0.x) + x,
                        f64::from(portal.0.y + 1),
                        f64::from(portal.0.z) + z,
                    ),
                    &EntityType::END_CRYSTAL,
                );
                event
                    .world
                    .add_entity_silent(Arc::new(EndCrystalEntity::new(entity)));
            }
        })
    }
}

struct QueuedDestruction {
    crystal: Arc<EndCrystalEntity>,
    queue_only: bool,
    attempts: AtomicUsize,
}

impl EventHandler<EntitySpawnEvent> for QueuedDestruction {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntitySpawnEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if event.entity_type == EntityType::ENDER_DRAGON.id.to_string() {
                self.attempts.fetch_add(1, Ordering::Relaxed);
                // Control the interleaving after the tick's first drain, without removing the entity.
                if self.queue_only {
                    event
                        .world
                        .pending_destroyed_crystals
                        .lock()
                        .unwrap()
                        .push(self.crystal.get_entity().entity_uuid);
                } else {
                    assert!(self.crystal.damage_with_context(
                        self.crystal.as_ref(),
                        1.0,
                        pumpkin_data::damage::DamageType::EXPLOSION,
                        None,
                        None,
                        None,
                    ));
                }
            }
        })
    }
}

fn assert_aborted(fixture: &Fixture) {
    let fight = fixture.world.dragon_fight.as_ref().unwrap().lock().unwrap();
    assert!(fight.respawn_stage.is_none());
    assert!(fight.dragon_killed && fight.dragon_uuid.is_none());
    assert_eq!(fight.respawn_time, 0);
    assert!(
        !fixture
            .world
            .entities
            .load()
            .iter()
            .any(|e| { e.get_entity().entity_type == &EntityType::ENDER_DRAGON })
    );
    assert_eq!(
        fixture.world.get_block(&BlockPos::new(1, 64, 0)),
        &Block::END_PORTAL
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_destruction_during_dragon_spawn_aborts_completion() {
    destruction_during_dragon_spawn_aborts_completion(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn crystal_damage_during_dragon_spawn_aborts_completion() {
    destruction_during_dragon_spawn_aborts_completion(false).await;
}

async fn destruction_during_dragon_spawn_aborts_completion(queue_only: bool) {
    let fixture = Fixture::new(DragonRespawnStage::SummoningDragon, 100);
    let crystal = fixture.crystal(Vector3::new(0.5, 65.0, 3.5), true);
    let handler = Arc::new(QueuedDestruction {
        crystal: crystal.clone(),
        queue_only,
        attempts: AtomicUsize::new(0),
    });
    fixture
        .server
        .plugin_manager
        .register::<EntitySpawnEvent, _>(handler.clone(), EventPriority::Normal, true);
    DragonFight::tick(fixture.world.dragon_fight.as_ref().unwrap(), &fixture.world);
    assert_eq!(handler.attempts.load(Ordering::Relaxed), 1);
    assert_aborted(&fixture);
    assert_eq!(crystal.get_entity().is_removed(), !queue_only);
    assert!(
        fixture
            .world
            .pending_destroyed_crystals
            .lock()
            .unwrap()
            .is_empty()
    );
    fixture.world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_destruction_before_dragon_creation_aborts_completion() {
    let fixture = Fixture::new(DragonRespawnStage::SummoningDragon, 100);
    let crystal = fixture.crystal(Vector3::new(0.5, 65.0, 3.5), true);
    let reader = reader(&fixture);
    let world = fixture.world.clone();
    BEFORE_ACTIONS.with_borrow_mut(|observer| {
        *observer = Some(Box::new(move || {
            // Control the seam between recording the final tick and running its effects.
            world
                .pending_destroyed_crystals
                .lock()
                .unwrap()
                .push(crystal.get_entity().entity_uuid);
        }));
    });
    DragonFight::tick(fixture.world.dragon_fight.as_ref().unwrap(), &fixture.world);
    assert_aborted(&fixture);
    assert!(reader.spawns.lock().unwrap().is_empty());
    fixture.world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scan_removes_legacy_dragon_after_releasing_fight() {
    let fixture = Fixture::new(DragonRespawnStage::Start, 0);
    fixture.world.set_block_state(
        &BlockPos::new(1, 64, 0),
        Block::END_PORTAL.default_state.id,
        BlockFlags::NOTIFY_LISTENERS,
    );
    let dragon = crate::entity::r#type::from_type(
        &EntityType::ENDER_DRAGON,
        Vector3::new(0.5, 128.0, 0.5),
        &fixture.world,
        Uuid::new_v4(),
    );
    fixture.world.add_entity_silent(dragon.clone());
    DragonFight::with_fight(
        fixture.world.dragon_fight.as_ref().unwrap(),
        &fixture.world,
        |fight| {
            fight.scan_state(&fixture.world);
            assert!(!dragon.get_entity().is_removed());
        },
    );
    assert!(dragon.get_entity().is_removed());
    fixture.world.level.shutdown().await.unwrap();
}
