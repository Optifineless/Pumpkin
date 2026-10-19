use std::{
    sync::{atomic::AtomicBool, mpsc},
    time::Duration,
};

use pumpkin_data::packet::CURRENT_MC_VERSION;
use pumpkin_protocol::{
    ClientPacket, RawPacket,
    java::server::play::{Action, SAttack, SPlayerCommand},
    packet::MultiVersionJavaPacket,
};
use pumpkin_util::sync::{LockClass, Mutex};

use super::{
    Arc, AtomicUsize, Callback, EntityBase, EntityPose, EntityPoseChangeEvent, EventPriority, FOOT,
    HEAD, PlayerFixture, Relaxed, TestPlayer, VarInt, Vector3, WhiteBedLikeProperties,
    assert_awake, finish, on_leave, sleeping, tick,
};

enum WakeRecords {}
impl LockClass for WakeRecords {
    const NAME: &'static str = "sleep regression rendezvous";
    const LEVEL: u8 = 40;
}

fn queue<P: ClientPacket + MultiVersionJavaPacket>(client: &TestPlayer, packet: &P) {
    let mut payload = Vec::new();
    packet
        .write_packet_data(&mut payload, &CURRENT_MC_VERSION)
        .unwrap();
    client.player.inbound_packets.push(RawPacket {
        id: P::to_id(CURRENT_MC_VERSION),
        payload: payload.into(),
    });
}

fn leave(client: &TestPlayer) {
    queue(
        client,
        &SPlayerCommand {
            entity_id: client.player.entity_id().into(),
            action: Action::LeaveBed,
            jump_boost: VarInt(0),
        },
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_damage_wake_survives_queued_leave_bed_and_sleep_tick() {
    let fixture = PlayerFixture::new();
    let client = sleeping(&fixture);
    for _ in 0..20 {
        tick(&fixture, &client);
    }
    assert_eq!(client.player.sleeping_since.load(), Some(20));
    let attacker = TestPlayer::new(&fixture.world);
    attacker
        .player
        .get_entity()
        .set_pos(Vector3::new(5.5, 64.0, 6.5));
    fixture.world.players.store(Arc::new(vec![
        client.player.clone(),
        attacker.player.clone(),
    ]));
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let (entered, waiting) = mpsc::sync_channel(1);
    let (release, resume) = mpsc::sync_channel(1);
    let resume = Mutex::<_, WakeRecords>::new(resume);
    on_leave(&fixture, move |event| {
        assert_eq!(event.bed_pos, HEAD);
        assert_eq!(event.player.sleeping_since.load(), Some(20));
        seen.fetch_add(1, Relaxed);
        entered.send(()).unwrap();
        resume
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(20))
            .unwrap();
    });
    queue(
        &attacker,
        &SAttack {
            entity_id: client.player.entity_id().into(),
        },
    );
    let server = fixture.world.server.upgrade().unwrap();
    let attacking_player = attacker.player.clone();
    let damage = tokio::task::spawn_blocking(move || attacking_player.tick(&server));
    waiting.recv_timeout(Duration::from_secs(10)).unwrap();
    leave(&client);
    tick(&fixture, &client);
    assert!(client.player.inbound_packets.is_empty());
    assert_eq!(client.player.sleeping_since.load(), Some(21));
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(20), damage)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(calls.load(Relaxed), 1);
    assert!(client.player.living_entity.health.load() < 20.0);
    assert_awake(&client);
    for position in [FOOT, HEAD] {
        assert!(
            !WhiteBedLikeProperties::from_state_id(fixture.world.get_block_state_id(&position))
                .occupied
        );
    }
    finish(fixture).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g05_cancelled_standing_pose_keeps_sleep_state_for_leave_bed_retry() {
    let fixture = PlayerFixture::new();
    let client = sleeping(&fixture);
    let cancelled = Arc::new(AtomicBool::new(true));
    let cancel = cancelled.clone();
    let player = client.player.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    fixture
        .world
        .server
        .upgrade()
        .unwrap()
        .plugin_manager
        .register::<EntityPoseChangeEvent, _>(
            Arc::new(Callback(move |event: &mut EntityPoseChangeEvent| {
                if event.entity_id == player.entity_id()
                    && event.pose == (EntityPose::Standing as u8).to_string()
                {
                    seen.fetch_add(1, Relaxed);
                    event.cancelled = cancel.load(Relaxed);
                }
            })),
            EventPriority::Normal,
            true,
        );
    leave(&client);
    tick(&fixture, &client);
    assert_eq!(calls.load(Relaxed), 1);
    assert_eq!(client.player.sleeping_bed_pos.load(), Some(HEAD));
    assert_eq!(client.player.sleeping_since.load(), Some(1));
    assert!(client.player.get_entity().pose.load() == EntityPose::Sleeping);
    cancelled.store(false, Relaxed);
    leave(&client);
    tick(&fixture, &client);
    assert_eq!(calls.load(Relaxed), 2);
    assert_awake(&client);
    finish(fixture).await;
}
