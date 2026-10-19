#![allow(clippy::unwrap_used)]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::test_support::{Fixture, align_watched_section, initialization, lerps};
use crate::{
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority, api::events::server::packet::PacketSentEvent,
    },
    server::Server,
};
use pumpkin_util::version::JavaMinecraftVersion;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020_g04_shrinking_border_uses_live_bounds() {
    let fixture = Fixture::new();
    fixture.resize(100, 20);
    for progress in [0, 50, 150] {
        fixture.ticks(if progress == 150 { 100 } else { progress });
        let inside = progress < 150;
        fixture.assert_bottle_use(inside);
        let border = fixture.world.worldborder.lock().unwrap();
        assert_eq!(
            border.contains(30.0, 0.0),
            inside,
            "shrink at tick {progress}"
        );
        assert_eq!(border.contains_block(30, 0), inside);
        drop(border);
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020_g04_expanding_border_uses_live_bounds() {
    let fixture = Fixture::new();
    fixture.resize(20, 100);
    for progress in [0, 50, 150] {
        fixture.ticks(if progress == 150 { 100 } else { progress });
        let inside = progress == 150;
        fixture.assert_bottle_use(inside);
        let border = fixture.world.worldborder.lock().unwrap();
        assert_eq!(
            border.contains(30.0, 0.0),
            inside,
            "expand at tick {progress}"
        );
        assert_eq!(border.contains_block(30, 0), inside);
        drop(border);
    }
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020_g04_retarget_and_add_start_from_current_size() {
    let mut fixture = Fixture::new();
    fixture.resize(100, 20);
    fixture.ticks(50);
    assert_eq!(
        fixture.run("worldborder get"),
        80,
        "get reports the live diameter"
    );
    fixture.actor.take_packets();
    assert_eq!(fixture.run("worldborder set 60 40t"), -20);
    assert_eq!(lerps(&mut fixture.actor), vec![(80.0, 60.0, 40)]);
    fixture.ticks(10);
    assert_eq!(fixture.run("worldborder get"), 75);
    fixture.finish().await;

    let mut fixture = Fixture::new();
    fixture.resize(100, 20);
    fixture.ticks(50);
    fixture.actor.take_packets();
    assert_eq!(fixture.run("worldborder add 10 20t"), 10);
    assert_eq!(
        lerps(&mut fixture.actor),
        vec![(80.0, 90.0, 170)],
        "timed add retains remaining ticks"
    );
    fixture.ticks(170);
    assert_eq!(fixture.run("worldborder get"), 90);
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020_g04_command_lerp_duration_uses_game_ticks() {
    let mut fixture = Fixture::new();
    fixture.resize(100, 20);
    assert_eq!(
        lerps(&mut fixture.actor),
        vec![(100.0, 20.0, 200)],
        "26.3 duration is ticks"
    );
    fixture.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020_g04_mid_resize_client_receives_current_state() {
    let mut fixture = Fixture::new();
    fixture.resize(100, 20);
    fixture.actor.take_packets();
    fixture.ticks(50);
    let mut arriving = TestPlayer::new(&fixture.world);
    arriving.client().set_player(arriving.player.clone());
    align_watched_section(&arriving);
    arriving.take_packets();
    fixture.world.send_world_info(&arriving.player);
    let state = initialization(&mut arriving);
    assert_eq!(state.center, (0.5, 0.5));
    assert_eq!(
        state.sizes,
        (80.0, 20.0, 150),
        "joining client receives the current extent"
    );
    assert_eq!(state.limit, 29_999_984);
    assert_eq!(state.warning, (5, 300));
    fixture.ticks(150);
    fixture.world.send_world_info(&arriving.player);
    assert_eq!(initialization(&mut arriving).sizes, (20.0, 20.0, 0));
    fixture.ticks(2);
    assert_eq!(fixture.run("worldborder get"), 20);
    arriving.client().close();
    fixture.finish().await;
}

struct ReadBorder(Arc<AtomicUsize>);

impl EventHandler<PacketSentEvent> for ReadBorder {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        if [
            pumpkin_data::packet::clientbound::play::SET_BORDER_LERP_SIZE.0,
            pumpkin_data::packet::clientbound::play::INITIALIZE_BORDER.0,
        ]
        .contains(&event.packet_id)
        {
            let world = event.player.world();
            let border = world.worldborder.try_lock().unwrap();
            assert!(border.get_size().is_finite());
            self.0.fetch_add(1, Ordering::Relaxed);
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020_g04_command_and_initialization_callbacks_read_border_unlocked() {
    let fixture = Fixture::new();
    // Production packet hooks operate on plugin-admitted older clients and receive 26.3 bytes.
    fixture
        .actor
        .client()
        .version
        .store(JavaMinecraftVersion::V_1_21_2);
    let calls = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<PacketSentEvent, _>(
            Arc::new(ReadBorder(calls.clone())),
            EventPriority::Normal,
            true,
        );
    fixture.resize(100, 20);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    fixture.ticks(50);
    fixture.world.send_world_info(&fixture.actor.player);
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    fixture.finish().await;
}
