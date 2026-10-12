use super::{respawn_tests::Fixture, *};
use crate::{
    block::entities::chest::ChestBlockEntity,
    entity::boss::ender_dragon::{DEATH_TIMER_MAX, EnderDragonEntity, phase::EnderDragonPhase},
    item::{ItemBehaviour, items::end_crystal::EndCrystalItem},
    plugin::{
        EventHandler, EventPriority,
        api::events::{entity::entity_spawn::EntitySpawnEvent, server::packet::PacketSentEvent},
    },
    server::Server,
};
use futures::future::BoxFuture;
use pumpkin_data::{BlockDirection, damage::DamageType, item::Item, item_stack::ItemStack};
use pumpkin_inventory::Inventory;
use pumpkin_util::version::JavaMinecraftVersion;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

pub(super) fn finish_dragon_death(dragon: &Arc<dyn EntityBase>, server: &Arc<Server>) {
    let entity = dragon
        .cast_any()
        .downcast_ref::<EnderDragonEntity>()
        .unwrap();
    entity.set_phase(EnderDragonPhase::Dying);
    *entity.dragon_death_time.lock().unwrap() = DEATH_TIMER_MAX - 1;
    dragon.tick(dragon.as_ref(), server);
    assert!(
        dragon.get_entity().is_removed(),
        "actual dragon death did not finish"
    );
}

fn assert_pending_dragon_spawn(world: &Arc<World>) {
    let fight = world.dragon_fight.as_ref().unwrap().try_lock().unwrap();
    assert!(fight.dragon_uuid.is_none() && fight.pending_dragon_spawn.is_some());
}

struct UntrackedDeathOnSpawn(AtomicUsize);

impl EventHandler<EntitySpawnEvent> for UntrackedDeathOnSpawn {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut EntitySpawnEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if event.entity_type != EntityType::ENDER_DRAGON.id.to_string() {
                return;
            }
            self.0.fetch_add(1, Ordering::Relaxed);
            let world = &event.world;
            assert_pending_dragon_spawn(world);
            let unrelated = crate::entity::r#type::from_type(
                &EntityType::ENDER_DRAGON,
                Vector3::new(8.5, 128.0, 8.5),
                world,
                Uuid::new_v4(),
            );
            world.add_entity_silent(unrelated.clone());
            finish_dragon_death(&unrelated, server);
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn untracked_dragon_death_during_spawn_cannot_consume_fight_rewards() {
    let fixture = Fixture::new(DragonRespawnStage::Start, 0);
    fixture.publish_area(GATEWAY_DISTANCE as i32, 0);
    fixture.with_locked_fight(|fight| {
        fight.respawn_stage = None;
        fight.dragon_killed = false;
        fight.previously_killed = false;
        fight.gateways = vec![0];
    });
    let observer = Arc::new(UntrackedDeathOnSpawn(AtomicUsize::new(0)));
    fixture
        .server
        .plugin_manager
        .register::<EntitySpawnEvent, _>(observer.clone(), EventPriority::Normal, true);
    DragonFight::tick(fixture.world.dragon_fight.as_ref().unwrap(), &fixture.world);
    assert_eq!(
        observer.0.load(Ordering::Relaxed),
        1,
        "actual dragon spawn callback did not run"
    );
    let state = {
        let fight = fixture.world.dragon_fight.as_ref().unwrap().lock().unwrap();
        (
            fight.gateways.clone(),
            fight.previously_killed,
            fight.dragon_killed,
            fight
                .dragon_uuid
                .and_then(|uuid| fixture.world.get_entity_by_uuid(uuid))
                .is_some(),
        )
    };
    let egg = fixture.world.get_block(&BlockPos::new(0, 68, 0)).id;
    let dragons = fixture
        .world
        .entities
        .load()
        .iter()
        .filter(|e| e.get_entity().entity_type == &EntityType::ENDER_DRAGON)
        .count();
    fixture.world.level.shutdown().await.unwrap();
    assert_eq!(
        state.0,
        vec![0],
        "untracked dragon death consumed a fight gateway"
    );
    assert!(!state.1 && !state.2 && state.3);
    assert_ne!(egg, Block::DRAGON_EGG.id);
    assert_eq!(dragons, 1);
}

struct BossActions(Mutex<Vec<u8>>);

impl EventHandler<PacketSentEvent> for BossActions {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if event.packet_id == pumpkin_data::packet::clientbound::play::BOSS_EVENT.0 {
                assert!(
                    event
                        .player
                        .world()
                        .dragon_fight
                        .as_ref()
                        .unwrap()
                        .try_lock()
                        .is_ok()
                );
                // ClientboundBossEventPacket carries its UUID before the operation VarInt.
                self.0.lock().unwrap().push(event.payload[16]);
            }
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stationary_observer_receives_bossbar_after_real_kill_and_four_crystal_respawn() {
    let fixture = Fixture::new(DragonRespawnStage::Start, 0);
    fixture.publish_area(GATEWAY_DISTANCE as i32, 0);
    for spike in DragonFight::get_spikes() {
        fixture.publish_area(spike.center_x, spike.center_z);
    }
    let player = fixture.world.players.load()[0].clone();
    let java = player.client.java().unwrap();
    java.player.store(Arc::new(Some(player.clone())));
    java.version.store(JavaMinecraftVersion::V_26_2);
    player
        .chunk_sender
        .lock()
        .unwrap()
        .mark_sent_out_of_band(Vector2::new(0, 0));
    let position = player.get_entity().pos.load();
    let dragon = crate::entity::r#type::from_type(
        &EntityType::ENDER_DRAGON,
        Vector3::new(0.5, 128.0, 0.5),
        &fixture.world,
        Uuid::new_v4(),
    );
    fixture.world.add_entity_silent(dragon.clone());
    fixture.with_locked_fight(|fight| {
        fight.respawn_stage = None;
        fight.dragon_killed = false;
        fight.previously_killed = false;
        fight.dragon_uuid = Some(dragon.get_entity().entity_uuid);
        fight.gateways = vec![0];
    });
    let observer = Arc::new(BossActions(Mutex::default()));
    fixture
        .server
        .plugin_manager
        .register::<PacketSentEvent, _>(observer.clone(), EventPriority::Normal, true);
    DragonFight::tick(fixture.world.dragon_fight.as_ref().unwrap(), &fixture.world);
    assert!(
        observer.0.lock().unwrap().contains(&0),
        "initial bossbar Add never reached the observer"
    );
    finish_dragon_death(&dragon, &fixture.server);
    assert!(
        observer.0.lock().unwrap().contains(&1),
        "actual kill did not hide the original bossbar"
    );
    observer.0.lock().unwrap().clear();
    for (x, z) in [(0, -3), (0, 3), (-3, 0), (3, 0)] {
        let base = BlockPos::new(x, 64, z);
        let mut item = ItemStack::new(1, &Item::END_CRYSTAL);
        assert!(matches!(
            EndCrystalItem.use_on_block(
                &mut item,
                &player,
                base,
                BlockDirection::Up,
                Vector3::new(0.5, 1.0, 0.5),
                &Block::BEDROCK,
                &fixture.server,
            ),
            crate::block::registry::BlockActionResult::Success
        ));
    }
    fixture.with_locked_fight(|fight| {
        assert_eq!(fight.respawn_stage, Some(DragonRespawnStage::Start));
        assert_eq!(fight.respawn_crystals.len(), 4);
    });
    for _ in 0..700 {
        DragonFight::tick(fixture.world.dragon_fight.as_ref().unwrap(), &fixture.world);
        if fixture
            .world
            .dragon_fight
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .respawn_stage
            .is_none()
        {
            break;
        }
    }
    fixture.with_locked_fight(|fight| {
        assert!(
            fight.respawn_stage.is_none() && !fight.dragon_killed && fight.dragon_uuid.is_some(),
            "actual respawn did not complete"
        );
        assert!(fight.bossbar_players.contains(&player.gameprofile.id));
    });
    assert_eq!(
        player.get_entity().pos.load(),
        position,
        "spectator moved during the test"
    );
    let received_add = observer.0.lock().unwrap().contains(&0);
    java.player.store(Arc::new(None));
    fixture.world.level.shutdown().await.unwrap();
    assert!(
        received_add,
        "stationary respawn spectator never received the new bossbar Add"
    );
}

fn assert_queued_spike(mutex: &Mutex<DragonFight>) {
    let fight = mutex.lock().unwrap();
    assert_eq!(fight.respawn_time, 40);
    assert_eq!(
        fight.respawn_stage,
        Some(DragonRespawnStage::SummoningPillars)
    );
    assert_eq!(
        fight.actions.len(),
        1,
        "nested tick did not queue the spike operation"
    );
}

struct QueueSpikeThenAbort {
    crystal: Arc<EndCrystalEntity>,
    applied: AtomicBool,
}

impl EventHandler<EntitySpawnEvent> for QueueSpikeThenAbort {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntitySpawnEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if event.entity_type != EntityType::ITEM.id.to_string()
                || self.applied.swap(true, Ordering::Relaxed)
            {
                return;
            }
            let world = &event.world;
            let mutex = world.dragon_fight.as_ref().unwrap();
            assert!(
                mutex.try_lock().is_ok(),
                "podium item-drop callback held the fight mutex"
            );
            DragonFight::tick(mutex, world);
            assert_queued_spike(mutex);
            assert!(self.crystal.damage_with_context(
                self.crystal.as_ref(),
                1.0,
                DamageType::EXPLOSION,
                None,
                None,
                None
            ));
            assert!(self.crystal.get_entity().is_removed());
            assert!(
                mutex.lock().unwrap().respawn_stage.is_none(),
                "actual crystal destruction did not abort respawn"
            );
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_podium_callback_abort_discards_already_queued_spike_effects() {
    let fixture = Fixture::new(DragonRespawnStage::SummoningPillars, 39);
    let crystal = fixture.crystal(Vector3::new(0.5, 65.0, 3.5), true);
    let spike = DragonFight::get_spikes()[0];
    fixture.publish_area(spike.center_x, spike.center_z);
    let sentinel = BlockPos::new(spike.center_x + 10, spike.height + 10, spike.center_z + 10);
    fixture.world.set_block_state(
        &sentinel,
        Block::STONE.default_state.id,
        BlockFlags::NOTIFY_LISTENERS,
    );
    let chest_pos = BlockPos::new(1, 64, 0);
    fixture.world.set_block_state(
        &chest_pos,
        Block::CHEST.default_state.id,
        BlockFlags::NOTIFY_LISTENERS,
    );
    let chest = Arc::new(ChestBlockEntity::new(chest_pos));
    chest.set_stack(0, ItemStack::new(7, &Item::DIAMOND));
    fixture.world.add_block_entity(chest);
    let observer = Arc::new(QueueSpikeThenAbort {
        crystal,
        applied: AtomicBool::new(false),
    });
    fixture
        .server
        .plugin_manager
        .register::<EntitySpawnEvent, _>(observer.clone(), EventPriority::Normal, true);
    DragonFight::with_fight(
        fixture.world.dragon_fight.as_ref().unwrap(),
        &fixture.world,
        |fight| {
            fight.spawn_exit_portal(&fixture.world, false);
        },
    );
    assert!(
        observer.applied.load(Ordering::Relaxed),
        "actual podium container-drop callback did not run"
    );
    let sentinel_block = fixture.world.get_block(&sentinel).id;
    let spike_spawned = fixture.world.entities.load().iter().any(|entity| {
        let pos = entity.get_entity().pos.load();
        entity.get_entity().entity_type == &EntityType::END_CRYSTAL
            && pos.x == f64::from(spike.center_x) + 0.5
            && pos.z == f64::from(spike.center_z) + 0.5
    });
    fixture.with_locked_fight(|fight| {
        assert!(fight.respawn_stage.is_none() && fight.dragon_killed);
        assert_eq!(fight.respawn_time, 0);
    });
    assert_eq!(
        fixture.world.get_block(&BlockPos::new(1, 64, 0)),
        &Block::END_PORTAL
    );
    fixture.world.level.shutdown().await.unwrap();
    assert_eq!(
        sentinel_block,
        Block::STONE.id,
        "aborted queued spike action destructively cleared the sentinel"
    );
    assert!(
        !spike_spawned,
        "aborted queued spike action spawned a new spike crystal"
    );
}

struct QueueSpikeThenAdvance(AtomicBool);

impl EventHandler<EntitySpawnEvent> for QueueSpikeThenAdvance {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntitySpawnEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if event.entity_type != EntityType::ITEM.id.to_string()
                || self.0.swap(true, Ordering::Relaxed)
            {
                return;
            }
            let world = &event.world;
            let mutex = world.dragon_fight.as_ref().unwrap();
            assert!(mutex.try_lock().is_ok());
            DragonFight::tick(mutex, world);
            assert_eq!(mutex.lock().unwrap().actions.len(), 1);
            // Native re-entry records later ticks behind the same ongoing world operation.
            for _ in 0..41 {
                DragonFight::tick(mutex, world);
            }
            assert_eq!(
                mutex.lock().unwrap().respawn_stage,
                Some(DragonRespawnStage::SummoningDragon)
            );
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_podium_callback_keeps_live_spike_work_after_natural_stage_advance() {
    let fixture = Fixture::new(DragonRespawnStage::SummoningPillars, 359);
    fixture.crystal(Vector3::new(0.5, 65.0, 3.5), true);
    let spike = DragonFight::get_spikes()[8];
    for spike in &DragonFight::get_spikes()[8..] {
        fixture.publish_area(spike.center_x, spike.center_z);
    }
    let sentinel = BlockPos::new(spike.center_x + 10, spike.height + 10, spike.center_z + 10);
    fixture.world.set_block_state(
        &sentinel,
        Block::STONE.default_state.id,
        BlockFlags::NOTIFY_LISTENERS,
    );
    let chest_pos = BlockPos::new(1, 64, 0);
    fixture.world.set_block_state(
        &chest_pos,
        Block::CHEST.default_state.id,
        BlockFlags::NOTIFY_LISTENERS,
    );
    let chest = Arc::new(ChestBlockEntity::new(chest_pos));
    chest.set_stack(0, ItemStack::new(7, &Item::DIAMOND));
    fixture.world.add_block_entity(chest);
    let observer = Arc::new(QueueSpikeThenAdvance(AtomicBool::new(false)));
    fixture
        .server
        .plugin_manager
        .register::<EntitySpawnEvent, _>(observer.clone(), EventPriority::Normal, true);
    DragonFight::with_fight(
        fixture.world.dragon_fight.as_ref().unwrap(),
        &fixture.world,
        |fight| {
            fight.spawn_exit_portal(&fixture.world, false);
        },
    );
    assert!(
        observer.0.load(Ordering::Relaxed),
        "actual podium callback did not run"
    );
    let block = fixture.world.get_block(&sentinel).id;
    let crystal_spawned = fixture.world.entities.load().iter().any(|entity| {
        let pos = entity.get_entity().pos.load();
        entity.get_entity().entity_type == &EntityType::END_CRYSTAL
            && pos.x == f64::from(spike.center_x) + 0.5
            && pos.z == f64::from(spike.center_z) + 0.5
    });
    fixture.world.level.shutdown().await.unwrap();
    assert_eq!(
        block,
        Block::AIR.id,
        "live queued spike vanished when the same respawn advanced stages"
    );
    assert!(
        crystal_spawned,
        "live queued spike never regenerated its crystal"
    );
}

fn published_pending_dragon(world: &Arc<World>) -> Option<Arc<dyn EntityBase>> {
    let fight = world.dragon_fight.as_ref().unwrap().try_lock().unwrap();
    world.get_entity_by_uuid(fight.pending_dragon_spawn?)
}

struct OrdinaryPublicationDeath {
    applied: AtomicBool,
    killed_uuid: Mutex<Option<Uuid>>,
}

impl EventHandler<PacketSentEvent> for OrdinaryPublicationDeath {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if event.packet_id != pumpkin_data::packet::clientbound::play::ADD_ENTITY.0
                || self.applied.load(Ordering::Relaxed)
            {
                return;
            }
            let world = event.player.world();
            let Some(dragon) = published_pending_dragon(&world) else {
                return;
            };
            assert_eq!(dragon.get_entity().entity_type, &EntityType::ENDER_DRAGON);
            if self.applied.swap(true, Ordering::Relaxed) {
                return;
            }
            *self.killed_uuid.lock().unwrap() = Some(dragon.get_entity().entity_uuid);
            // The world member is already published, but no fight guard reaches this death.
            finish_dragon_death(&dragon, server);
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordinary_admission_packet_death_keeps_completed_fight_and_rewards() {
    let fixture = Fixture::new(DragonRespawnStage::Start, 0);
    fixture.publish_area(GATEWAY_DISTANCE as i32, 0);
    let player = fixture.world.players.load()[0].clone();
    let java = player.client.java().unwrap();
    java.player.store(Arc::new(Some(player.clone())));
    java.version.store(JavaMinecraftVersion::V_26_2);
    player
        .chunk_sender
        .lock()
        .unwrap()
        .mark_sent_out_of_band(Vector2::new(0, 0));
    fixture.with_locked_fight(|fight| {
        fight.respawn_stage = None;
        fight.dragon_killed = false;
        fight.previously_killed = false;
        fight.dragon_uuid = None;
        fight.gateways = vec![0];
    });
    let observer = Arc::new(OrdinaryPublicationDeath {
        applied: AtomicBool::new(false),
        killed_uuid: Mutex::default(),
    });
    fixture
        .server
        .plugin_manager
        .register::<PacketSentEvent, _>(observer.clone(), EventPriority::Normal, true);
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
    let killed_uuid = *observer.killed_uuid.lock().unwrap();
    let removed = killed_uuid.is_some_and(|uuid| fixture.world.get_entity_by_uuid(uuid).is_none());
    let egg = fixture.world.get_block(&BlockPos::new(0, 68, 0)).id;
    let portal = fixture.world.get_block(&BlockPos::new(1, 64, 0)).id;
    java.player.store(Arc::new(None));
    fixture.world.level.shutdown().await.unwrap();
    assert!(
        observer.applied.load(Ordering::Relaxed) && removed,
        "actual ordinary ADD_ENTITY callback did not finish its admitted dragon's death"
    );
    assert!(
        state.0 && state.1 && state.2.is_none() && state.3.is_none(),
        "ordinary publication death was overwritten by spawn finalization"
    );
    assert!(
        state.4.is_empty(),
        "admitted ordinary death lost its gateway reward"
    );
    assert_eq!(egg, Block::DRAGON_EGG.id);
    assert_eq!(portal, Block::END_PORTAL.id);
}
