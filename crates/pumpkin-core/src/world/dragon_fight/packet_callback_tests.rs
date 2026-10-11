use super::{respawn_tests::Fixture, *};
use crate::{
    net::ClientPlatform,
    plugin::{EventHandler, EventPriority, api::events::server::packet::PacketSentEvent},
    server::Server,
};
use futures::future::BoxFuture;
use pumpkin_util::{text::TextComponent, version::JavaMinecraftVersion};
use std::sync::atomic::{AtomicUsize, Ordering};

struct PacketReader(AtomicUsize);

impl EventHandler<PacketSentEvent> for PacketReader {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let world = event.player.world();
            assert!(
                world.dragon_fight.as_ref().unwrap().try_lock().is_ok(),
                "packet callback would block on dragon_fight"
            );
            self.0.fetch_add(1, Ordering::Relaxed);
        })
    }
}

fn reader(fixture: &Fixture) -> Arc<PacketReader> {
    let player = fixture.world.players.load()[0].clone();
    let ClientPlatform::Java(client) = player.client.as_ref() else {
        panic!("Java fixture required");
    };
    client.player.store(Arc::new(Some(player.clone())));
    client.version.store(JavaMinecraftVersion::V_26_2);
    let reader = Arc::new(PacketReader(AtomicUsize::new(0)));
    fixture
        .server
        .plugin_manager
        .register::<PacketSentEvent, _>(reader.clone(), EventPriority::Normal, true);
    reader
}

fn clear_client_player(fixture: &Fixture) {
    if let ClientPlatform::Java(client) = fixture.world.players.load()[0].client.as_ref() {
        client.player.store(Arc::new(None));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bossbar_packets_can_lock_fight() {
    let fixture = Fixture::new(DragonRespawnStage::Start, 0);
    let dragon = crate::entity::r#type::from_type(
        &EntityType::ENDER_DRAGON,
        Vector3::new(0.5, 128.0, 0.5),
        &fixture.world,
        Uuid::new_v4(),
    );
    fixture.world.add_entity_silent(dragon.clone());
    {
        let mut fight = fixture.world.dragon_fight.as_ref().unwrap().lock().unwrap();
        fight.respawn_stage = None;
        fight.dragon_killed = false;
        fight.dragon_uuid = Some(dragon.get_entity().entity_uuid);
    };
    let reader = reader(&fixture);
    DragonFight::tick(fixture.world.dragon_fight.as_ref().unwrap(), &fixture.world);
    assert_eq!(reader.0.swap(0, Ordering::Relaxed), 1);

    dragon
        .get_entity()
        .custom_name
        .store(Arc::new(Some(TextComponent::text("dragon"))));
    dragon.tick(dragon.as_ref(), &fixture.server);
    assert!(reader.0.swap(0, Ordering::Relaxed) >= 2);

    // EnderDragonFight.updatePlayers removes the boss bar when the player leaves the arena.
    fixture.world.players.load()[0]
        .get_entity()
        .set_pos(Vector3::new(1000.0, 128.0, 0.0));
    DragonFight::with_fight(
        fixture.world.dragon_fight.as_ref().unwrap(),
        &fixture.world,
        |fight| {
            fight.ticks_since_last_player_scan = TIME_BETWEEN_PLAYER_SCANS;
        },
    );
    DragonFight::tick(fixture.world.dragon_fight.as_ref().unwrap(), &fixture.world);
    assert_eq!(reader.0.load(Ordering::Relaxed), 1);
    clear_client_player(&fixture);
    fixture.world.level.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn respawn_roar_packets_can_lock_fight() {
    for (stage, time) in [
        (DragonRespawnStage::PreparingToSummonPillars, 0),
        (DragonRespawnStage::SummoningDragon, 1),
        (DragonRespawnStage::SummoningDragon, 80),
    ] {
        let fixture = Fixture::new(stage, time);
        fixture.crystal(Vector3::new(0.5, 65.0, 3.5), true);
        let reader = reader(&fixture);
        DragonFight::tick(fixture.world.dragon_fight.as_ref().unwrap(), &fixture.world);
        assert_eq!(reader.0.load(Ordering::Relaxed), 1);
        clear_client_player(&fixture);
        fixture.world.level.shutdown().await.unwrap();
    }
}

struct ReentrantBossbar {
    actions: std::sync::Mutex<Vec<u8>>,
    duplicate_kill: Option<Uuid>,
    repeated_effects: std::sync::Mutex<Vec<&'static str>>,
}

impl EventHandler<PacketSentEvent> for ReentrantBossbar {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if event.packet_id != pumpkin_data::packet::clientbound::play::BOSS_EVENT.0 {
                return;
            }
            // ClientboundBossEventPacket: UUID, then the operation's VarInt.
            let action = event.payload[16];
            self.actions.lock().unwrap().push(action);
            if action == 1 {
                let world = event.player.world();
                let mutex = world.dragon_fight.as_ref().unwrap();
                assert!(mutex.try_lock().is_ok());
                DragonFight::with_fight(mutex, &world, |fight| {
                    if let Some(uuid) = self.duplicate_kill {
                        fight.set_dragon_killed(&world, uuid);
                        self.repeated_effects.lock().unwrap().extend(
                            fight.actions.iter().filter_map(|action| match action {
                                FightAction::PlacePortal(..) => Some("portal"),
                                FightAction::SpawnGateway(..) => Some("gateway"),
                                FightAction::PlaceEgg(..) => Some("egg"),
                                _ => None,
                            }),
                        );
                    }
                    fight.ticks_since_last_player_scan = TIME_BETWEEN_PLAYER_SCANS;
                });
                DragonFight::tick(mutex, &world);
            }
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kill_packet_reentry_cannot_restore_bossbar() {
    kill_packet_reentry(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn duplicate_kill_during_death_callback_has_no_effects() {
    kill_packet_reentry(true).await;
}

fn assert_dead_boss_bar_actions(observer: &ReentrantBossbar) {
    let actions = observer.actions.lock().unwrap();
    assert!(actions.contains(&1));
    assert!(
        !actions.contains(&0),
        "dead dragon's boss bar was added again"
    );
}

async fn kill_packet_reentry(duplicate_kill: bool) {
    use crate::entity::boss::ender_dragon::{DEATH_TIMER_MAX, phase::EnderDragonPhase};

    let fixture = Fixture::new(DragonRespawnStage::Start, 0);
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
        fight.dragon_uuid = Some(dragon.get_entity().entity_uuid);
        fight.previously_killed = false;
        fight.gateways = vec![1, 0];
    });
    fixture.publish_area(GATEWAY_DISTANCE as i32, 0);
    reader(&fixture);
    DragonFight::tick(fixture.world.dragon_fight.as_ref().unwrap(), &fixture.world);
    let observer = Arc::new(ReentrantBossbar {
        actions: std::sync::Mutex::default(),
        duplicate_kill: duplicate_kill.then_some(dragon.get_entity().entity_uuid),
        repeated_effects: std::sync::Mutex::default(),
    });
    fixture
        .server
        .plugin_manager
        .register::<PacketSentEvent, _>(observer.clone(), EventPriority::Normal, true);
    let dragon_entity = dragon
        .cast_any()
        .downcast_ref::<crate::entity::boss::ender_dragon::EnderDragonEntity>()
        .unwrap();
    dragon_entity.set_phase(EnderDragonPhase::Dying);
    *dragon_entity.dragon_death_time.lock().unwrap() = DEATH_TIMER_MAX - 1;
    // Drive the mob tick through DyingPhase's actual kill entry point.
    dragon.tick(dragon.as_ref(), &fixture.server);
    assert!(dragon.get_entity().is_removed());
    assert!(observer.repeated_effects.lock().unwrap().is_empty());
    fixture.with_locked_fight(|fight| {
        assert_eq!(
            fight.gateways,
            vec![1],
            "one death must consume one gateway"
        );
        assert!(fight.dragon_killed && fight.previously_killed && fight.dragon_uuid.is_none());
    });
    assert_eq!(
        fixture.world.get_block(&BlockPos::new(0, 68, 0)),
        &Block::DRAGON_EGG
    );
    assert_eq!(
        fixture.world.get_block(&BlockPos::new(1, 64, 0)),
        &Block::END_PORTAL
    );
    assert_dead_boss_bar_actions(&observer);
    observer.actions.lock().unwrap().clear();
    DragonFight::tick(fixture.world.dragon_fight.as_ref().unwrap(), &fixture.world);
    assert!(observer.actions.lock().unwrap().is_empty());
    DragonFight::with_fight(
        fixture.world.dragon_fight.as_ref().unwrap(),
        &fixture.world,
        |fight| fight.set_dragon_killed(&fixture.world, dragon.get_entity().entity_uuid),
    );
    assert_eq!(
        fixture
            .world
            .dragon_fight
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .gateways,
        vec![1]
    );
    clear_client_player(&fixture);
    fixture.world.level.shutdown().await.unwrap();
}

struct KillPublishedDragon(AtomicUsize);

impl EventHandler<PacketSentEvent> for KillPublishedDragon {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if event.packet_id != pumpkin_data::packet::clientbound::play::ADD_ENTITY.0 {
                return;
            }
            let world = event.player.world();
            let mutex = world.dragon_fight.as_ref().unwrap();
            assert!(mutex.try_lock().is_ok());
            DragonFight::with_fight(mutex, &world, |fight| {
                if let Some(uuid) = fight.pending_dragon_spawn
                    && fight.dragon_uuid == Some(uuid)
                    && world.get_entity_by_uuid(uuid).is_some()
                {
                    self.0.fetch_add(1, Ordering::Relaxed);
                    fight.set_dragon_killed(&world, uuid);
                }
            });
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn publication_death_does_not_skip_respawn_cleanup() {
    let fixture = Fixture::new(DragonRespawnStage::SummoningDragon, 100);
    // ServerEntity.addPairing requires the observer to have received the dragon's chunk.
    fixture.world.players.load()[0]
        .chunk_sender
        .lock()
        .unwrap()
        .mark_sent_out_of_band(Vector2::new(0, 0));
    let crystals: Vec<_> = [(0.5, -2.5), (0.5, 3.5), (-2.5, 0.5), (3.5, 0.5)]
        .into_iter()
        .map(|(x, z)| fixture.crystal(Vector3::new(x, 65.0, z), true))
        .collect();
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
    reader(&fixture);
    let observer = Arc::new(KillPublishedDragon(AtomicUsize::new(0)));
    fixture
        .server
        .plugin_manager
        .register::<PacketSentEvent, _>(observer.clone(), EventPriority::Normal, true);
    DragonFight::tick(fixture.world.dragon_fight.as_ref().unwrap(), &fixture.world);
    assert_eq!(observer.0.load(Ordering::Relaxed), 1);
    assert!(
        crystals
            .iter()
            .all(|crystal| crystal.get_entity().is_removed())
    );
    assert!(!spike_crystal.is_invulnerable() && spike_crystal.beam_target().is_none());
    fixture.with_locked_fight(|fight| {
        assert!(
            fight.respawn_stage.is_none() && fight.dragon_killed && fight.dragon_uuid.is_none()
        );
    });
    clear_client_player(&fixture);
    fixture.world.level.shutdown().await.unwrap();
}
