use super::*;
use crate::{
    net::java::sound_test_support::decode_sounds,
    plugin::api::events::player::{
        player_change_world::PlayerChangeWorldEvent, player_teleport::PlayerTeleportEvent,
    },
};
use bytes::Bytes;
use pumpkin_data::{
    damage::DamageType,
    dimension::Dimension,
    packet::clientbound::play::{ADD_ENTITY, DAMAGE_EVENT, PLAYER_POSITION, SET_HEALTH, SOUND},
    sound::Sound,
};
use pumpkin_protocol::ser::NetworkReadExt;

#[path = "ender_pearl_owner_review_tests/round2_tests.rs"]
mod round2_tests;

struct Fixture {
    server: Arc<Server>,
    source: Arc<crate::world::World>,
    target: Arc<crate::world::World>,
    owner: TestPlayer,
    spectator: TestPlayer,
    _dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let server = server(dir.path());
        let source = server.create_world("overworld".into(), Dimension::OVERWORLD);
        let target = server.create_world("nether".into(), Dimension::THE_NETHER);
        for world in [&source, &target] {
            world.set_difficulty(Difficulty::Peaceful);
            world.level.loaded_chunks.insert(
                Vector2::new(0, 0),
                pumpkin_world::chunk::ChunkData::empty_sync(0, 0),
            );
        }
        let mut spectator = TestPlayer::new(&target);
        spectator
            .player
            .set_gamemode(pumpkin_util::GameMode::Spectator);
        spectator
            .player
            .get_entity()
            .set_pos(Vector3::new(10.5, 64.0, 8.5));
        spectator
            .player
            .chunk_sender
            .lock()
            .unwrap()
            .mark_sent_out_of_band(Vector2::new(0, 0));
        spectator.take_packets();
        let owner = TestPlayer::new(&source);
        owner
            .player
            .get_entity()
            .set_pos(Vector3::new(1.0, 64.0, 1.0));
        owner.player.living_entity.set_health(8.0);
        owner.player.living_entity.fall_distance.store(20.0);
        Self {
            server,
            source,
            target,
            owner,
            spectator,
            _dir: dir,
        }
    }

    fn fire(&self) {
        fire_test_pearl(&self.owner.player, &self.target, &self.server);
    }

    async fn finish(&mut self) -> Vec<Bytes> {
        let player = self.owner.player.clone();
        let mut packets = Vec::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            self.owner.with_outgoing_writer_and_capture(
                async move {
                    player.client.java().unwrap().await_tasks().await;
                },
                &mut packets,
            ),
        )
        .await
        .unwrap();
        packets.extend(self.owner.take_packets());
        packets
    }
}

fn packet_id(packet: &Bytes) -> i32 {
    packet.as_ref().get_var_int().unwrap().0
}

fn pearl_sounds(packets: &[Bytes]) -> Vec<Vector3<i32>> {
    decode_sounds(packets)
        .into_iter()
        .filter(|sound| sound.sound_id == Sound::EntityPlayerTeleport as u16)
        .map(|sound| sound.position)
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owner_review_cross_world_pearl_preserves_low_health_and_hurt_feedback() {
    let mut fixture = Fixture::new();
    let gate = register_gate::<PlayerChangeWorldEvent>(&fixture.server);
    fixture.fire();
    gate.wait_until_entered().await;
    assert_eq!(fixture.owner.player.living_entity.health.load(), 8.0);
    assert!(pearl_sounds(&fixture.spectator.take_packets()).is_empty());
    // Falling can continue in the old world while the transfer awaits plugins.
    fixture.owner.player.living_entity.fall_distance.store(31.0);
    gate.release();
    let owner_packets = fixture.finish().await;
    let arrival = owner_packets
        .iter()
        .position(|packet| packet_id(packet) == PLAYER_POSITION.0)
        .unwrap();
    let arrival_sound = owner_packets
        .iter()
        .position(|packet| {
            packet_id(packet) == SOUND.0 && !pearl_sounds(std::slice::from_ref(packet)).is_empty()
        })
        .unwrap();
    assert!(
        arrival < arrival_sound,
        "owner receives destination coordinates before the spatial teleport sound"
    );

    let player = &fixture.owner.player;
    assert!(Arc::ptr_eq(&player.world(), &fixture.target));
    assert_eq!(player.living_entity.health.load(), 3.0);
    assert_eq!(player.living_entity.fall_distance.load(), 0.0);
    assert_eq!(
        owner_packets
            .iter()
            .filter(|packet| packet_id(packet) == DAMAGE_EVENT.0)
            .count(),
        1
    );
    let health: Vec<_> = owner_packets
        .iter()
        .filter_map(|packet| {
            let mut data = packet.as_ref();
            (data.get_var_int().unwrap().0 == SET_HEALTH.0).then(|| data.get_f32_be().unwrap())
        })
        .collect();
    assert_eq!(
        health.last(),
        Some(&3.0),
        "owner receives post-pearl health before loading"
    );
    let packets = fixture.spectator.take_packets();
    let damage: Vec<_> = packets
        .iter()
        .enumerate()
        .filter(|(_, packet)| packet_id(packet) == DAMAGE_EVENT.0)
        .collect();
    assert_eq!(
        damage.len(),
        1,
        "spectator must receive the pearl hurt event once"
    );
    let mut data = damage[0].1.as_ref();
    data.get_var_int().unwrap();
    assert_eq!(data.get_var_int().unwrap().0, player.entity_id());
    assert_eq!(
        data.get_var_int().unwrap().0,
        i32::from(DamageType::ENDER_PEARL.id)
    );
    let spawn = packets
        .iter()
        .position(|packet| packet_id(packet) == ADD_ENTITY.0)
        .unwrap();
    let sound = packets
        .iter()
        .position(|packet| {
            packet_id(packet) == SOUND.0 && !pearl_sounds(std::slice::from_ref(packet)).is_empty()
        })
        .unwrap();
    assert!(spawn < damage[0].0 && damage[0].0 < sound);
    assert_eq!(pearl_sounds(&packets), vec![Vector3::new(59, 513, 68)]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owner_review_ordinary_world_transfer_preserves_low_health() {
    let mut fixture = Fixture::new();
    let player = fixture.owner.player.clone();
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        fixture.owner.with_outgoing_writer(player.teleport_world(
            fixture.target.clone(),
            Vector3::new(4.0, 70.0, 4.0),
            None,
            None,
        )),
    )
    .await
    .unwrap();
    assert!(Arc::ptr_eq(&player.world(), &fixture.target));
    assert_eq!(player.living_entity.health.load(), 8.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owner_review_cross_world_position_veto_preserves_owner_and_has_no_sound() {
    let mut fixture = Fixture::new();
    crate::entity::harvest_tests::register::<PlayerTeleportEvent>(&fixture.server, |event| {
        event.cancelled = true;
    });
    fixture.fire();
    fixture.finish().await;
    let player = &fixture.owner.player;
    assert!(Arc::ptr_eq(&player.world(), &fixture.source));
    assert_eq!(player.position(), Vector3::new(1.0, 64.0, 1.0));
    assert_eq!(player.living_entity.health.load(), 8.0);
    assert_eq!(player.living_entity.fall_distance.load(), 20.0);
    assert!(pearl_sounds(&fixture.spectator.take_packets()).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owner_review_cancelled_world_transfer_is_silent() {
    let mut fixture = Fixture::new();
    crate::entity::harvest_tests::register::<PlayerChangeWorldEvent>(&fixture.server, |event| {
        event.cancelled = true;
    });
    fixture.fire();
    fixture.finish().await;
    assert_eq!(fixture.owner.player.living_entity.health.load(), 8.0);
    assert!(pearl_sounds(&fixture.spectator.take_packets()).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owner_review_redirect_to_source_uses_accepted_position_and_sound() {
    let mut fixture = Fixture::new();
    // Observe the accepted source world without replacing its owner membership.
    let mut observer = TestPlayer::new(&fixture.source);
    fixture.source.players.store(Arc::new(vec![
        fixture.owner.player.clone(),
        observer.player.clone(),
    ]));
    observer
        .player
        .get_entity()
        .set_pos(Vector3::new(10.5, 64.0, 8.5));
    observer.take_packets();
    crate::entity::harvest_tests::register::<PlayerChangeWorldEvent>(&fixture.server, |event| {
        event.new_world = event.previous_world.clone();
        event.position = Vector3::new(4.0, 70.0, 4.0);
    });
    let calls = Arc::new(AtomicU32::new(0));
    let observed_calls = calls.clone();
    crate::entity::harvest_tests::register::<PlayerTeleportEvent>(&fixture.server, move |event| {
        observed_calls.fetch_add(1, Ordering::Relaxed);
        event.to = Vector3::new(6.0, 70.0, 6.0);
    });
    fixture.fire();
    fixture.finish().await;
    let player = &fixture.owner.player;
    assert!(Arc::ptr_eq(&player.world(), &fixture.source));
    assert_eq!(player.position(), Vector3::new(6.0, 70.0, 6.0));
    assert_eq!(player.living_entity.health.load(), 3.0);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        pearl_sounds(&observer.take_packets()),
        vec![Vector3::new(48, 560, 48)]
    );
    assert!(pearl_sounds(&fixture.spectator.take_packets()).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owner_review_lethal_cross_world_pearl_completes_cancellable_death() {
    use crate::plugin::api::events::entity::entity_death::PlayerDeathEvent;
    use pumpkin_data::{
        item::Item, item_stack::ItemStack, packet::clientbound::play::PLAYER_COMBAT_KILL,
    };

    for cancel in [false, true] {
        let mut fixture = Fixture::new();
        let player = fixture.owner.player.clone();
        player.living_entity.set_health(4.0);
        player
            .inventory
            .set_slot(0, ItemStack::new(3, &Item::DIAMOND));
        let deaths = Arc::new(AtomicU32::new(0));
        let observed_deaths = deaths.clone();
        crate::entity::harvest_tests::register::<PlayerDeathEvent>(&fixture.server, move |event| {
            observed_deaths.fetch_add(1, Ordering::Relaxed);
            assert_eq!(event.player.inventory.get_slot(0).item_count, 3);
            event.cancelled = cancel;
        });
        fixture.fire();
        let packets = fixture.finish().await;
        assert_eq!(player.living_entity.health.load(), 0.0);
        assert!(player.living_entity.dead.load(Ordering::Relaxed));
        assert_eq!(deaths.load(Ordering::Relaxed), 1);
        assert_eq!(player.inventory.get_slot(0).is_empty(), !cancel);
        assert_eq!(
            packets
                .iter()
                .filter(|packet| packet_id(packet) == PLAYER_COMBAT_KILL.0)
                .count(),
            usize::from(!cancel)
        );
        let dropped: u32 = fixture
            .target
            .entities
            .load()
            .iter()
            .filter_map(|entity| entity.get_item_entity())
            .map(|item| item.get_item_stack().lock().unwrap().clone())
            .filter(|stack| stack.item == &Item::DIAMOND)
            .map(|stack| u32::from(stack.item_count))
            .sum();
        assert_eq!(dropped, if cancel { 0 } else { 3 });
    }
}
