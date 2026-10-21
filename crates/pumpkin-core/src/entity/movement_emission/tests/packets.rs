use super::*;
use crate::plugin::{
    BoxFuture, EventHandler, EventPriority,
    api::events::{player::player_move::PlayerMoveEvent, world::generic_game::GenericGameEvent},
};
use crate::server::Server;
use pumpkin_data::game_event::GameEvent;
use pumpkin_protocol::java::server::play::{SPlayerRotation, SSetPlayerGround};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering::Relaxed},
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s3_both_real_position_packets_cross_distance_threshold() {
    for mode in 0..3 {
        let s = scenario(true, true);
        send(
            &s,
            &SPlayerRotation {
                yaw: 1.0,
                pitch: 2.0,
                ground: true,
            },
        );
        send(&s, &SSetPlayerGround { on_ground: true });
        position(&s, false, START, true);
        assert_eq!(counts(&s), (0, 0), "rotation/ground/zero controls");
        for (i, dx) in [0.5, 1.0, 1.5, 5.0 / 3.0].into_iter().enumerate() {
            position(&s, mode == 1 || mode == 2 && i % 2 == 1, at(dx), true);
            assert_eq!(
                counts(&s),
                (0, 0),
                "mode {mode}: below/exact strict threshold at {dx}"
            );
        }
        position(&s, mode != 0, at(1.75), true);
        assert_eq!(
            counts(&s),
            (1, 1),
            "both packet paths share accumulated distance"
        );
        let sink = s.sink.lock().unwrap();
        assert_eq!(sink.forwarded[0].event, GameEvent::Step);
        assert_eq!(sink.forwarded[0].position, at(1.75));
        assert_eq!(
            sink.forwarded[0].affected_state,
            Some(Block::STONE.default_state.id)
        );
        s.assert_source(sink.forwarded[0].source.as_ref().unwrap(), at(1.75));
        drop(sink);
        position(&s, mode != 0, at(5.0), true);
        assert_eq!(
            counts(&s),
            (2, 2),
            "one notification for a large accepted delta"
        );
        assert_eq!(s.owner.player.position(), at(5.0), "movement control");
    }
}

enum Edit {
    Destination,
    Cancel,
    Invalid,
    Remove,
    Nested,
}
struct MoveEdit {
    edit: Edit,
    once: AtomicBool,
}
impl EventHandler<PlayerMoveEvent> for MoveEdit {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut PlayerMoveEvent,
    ) -> BoxFuture<'a, ()> {
        if !self.once.swap(true, Relaxed) {
            match self.edit {
                Edit::Destination => event.to = at(1.75),
                Edit::Cancel => event.cancelled = true,
                Edit::Invalid => event.to.x = f64::NAN,
                Edit::Remove => event.player.get_entity().remove(),
                Edit::Nested => move_player(&event.player, server, true, at(1.75), true),
            }
        }
        Box::pin(async {})
    }
}

fn edit(s: &Scenario, mode: Edit) {
    s.server.plugin_manager.register::<PlayerMoveEvent, _>(
        Arc::new(MoveEdit {
            edit: mode,
            once: AtomicBool::new(false),
        }),
        EventPriority::Normal,
        true,
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s3_packet_admission_and_plugin_destination() {
    for turn in [false, true] {
        let s = scenario(true, true);
        edit(&s, Edit::Destination);
        position(&s, turn, at(0.25), true);
        assert_eq!(s.owner.player.position(), at(1.75));
        assert_eq!(
            counts(&s),
            (1, 1),
            "plugin accepted destination determines distance"
        );
        for mode in [Edit::Cancel, Edit::Invalid, Edit::Remove] {
            let s = scenario(true, true);
            edit(&s, mode);
            position(&s, turn, at(3.0), true);
            assert_eq!(counts(&s), (0, 0), "cancel/nonfinite/stale controls");
        }
        let s = scenario(true, true);
        s.owner.player.is_movement_locked.store(true, Relaxed);
        position(&s, turn, at(3.0), true);
        assert_eq!(s.owner.player.position(), START);
        s.owner.player.is_movement_locked.store(false, Relaxed);
        position(&s, turn, at(3.0), true);
        assert_eq!(
            counts(&s),
            (0, 0),
            "awaiting teleport excludes displacement"
        );
        confirm(&s);
        s.owner.player.request_teleport(at(3.0), 0.0, 0.0);
        confirm(&s);
        position(&s, turn, at(3.25), true);
        assert_eq!(counts(&s), (0, 0), "teleport distance is not walking");
        s.owner.player.set_client_loaded(false);
        position(&s, turn, at(5.0), true);
        assert_eq!(counts(&s), (0, 0), "unloaded client control");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s3_generic_event_mutation_preserves_threshold() {
    let s = scenario(true, true);
    s.capture(Some(Box::new(|_, event| {
        if event.event_key == "step" {
            event.event_key = "flap".into();
            event.position = at(2.5);
        }
    })));
    position(&s, false, at(1.75), true);
    position(&s, true, at(1.85), true);
    let sink = s.sink.lock().unwrap();
    assert_eq!(
        sink.forwarded.len(),
        1,
        "mutated event does not replay threshold"
    );
    assert_eq!(sink.forwarded[0].event, GameEvent::Flap);
    assert_eq!(sink.forwarded[0].position, at(2.5));
    assert_eq!(
        sink.forwarded[0].affected_state,
        Some(Block::STONE.default_state.id)
    );
    assert_eq!(s.owner.player.position(), at(1.85));
}

struct CancelStep {
    once: AtomicBool,
    player: Arc<crate::entity::player::Player>,
    nested: bool,
}
impl EventHandler<GenericGameEvent> for CancelStep {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut GenericGameEvent,
    ) -> BoxFuture<'a, ()> {
        if event.event_key == "step" && !self.once.swap(true, Relaxed) {
            event.cancelled = true;
            if self.nested {
                move_player(&self.player, server, true, at(3.5), true);
            }
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s3_lifecycle_reentry_and_switch_matrix() {
    for movement in [false, true] {
        for sculk in [false, true] {
            let s = scenario(movement, sculk);
            position(&s, false, at(1.75), true);
            assert_eq!(s.owner.player.position(), at(1.75));
            assert_eq!(
                counts(&s),
                (usize::from(movement), usize::from(movement && sculk)),
                "two-switch matrix"
            );
            s.rod(Some(true));
            s.use_item();
            assert!(s.owner.player.fishing_bobber.load(Relaxed) > 0);
            let sink = s.sink.lock().unwrap();
            assert_eq!(
                sink.plugins.last().unwrap().key,
                "item_interact_start",
                "OFF retains unrelated fishing notification"
            );
        }
    }
    for nested in [false, true] {
        let s = scenario(true, true);
        s.server.plugin_manager.register::<GenericGameEvent, _>(
            Arc::new(CancelStep {
                once: AtomicBool::new(false),
                player: s.owner.player.clone(),
                nested,
            }),
            EventPriority::High,
            true,
        );
        position(&s, false, at(1.75), true);
        assert_eq!(
            counts(&s),
            if nested { (2, 1) } else { (1, 0) },
            "threshold commits before reentry/cancellation"
        );
        position(&s, true, if nested { at(3.6) } else { at(1.85) }, true);
        assert_eq!(
            counts(&s),
            if nested { (2, 1) } else { (1, 0) },
            "cancelled step cannot replay"
        );
    }
    let s = scenario(true, true);
    edit(&s, Edit::Nested);
    position(&s, false, at(1.75), true);
    assert_eq!(
        counts(&s),
        (1, 1),
        "nested PlayerMove commits one threshold"
    );
    position(&s, false, at(1.85), true);
    assert_eq!(
        counts(&s),
        (1, 1),
        "outer move cannot overwrite successor counters"
    );
}
