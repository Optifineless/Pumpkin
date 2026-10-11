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
