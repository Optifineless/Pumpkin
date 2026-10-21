use super::*;
use crate::plugin::{
    BoxFuture, EventHandler, EventPriority,
    api::events::{player::player_move::PlayerMoveEvent, world::generic_game::GenericGameEvent},
};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

struct Transition {
    player: Arc<Player>,
    world: Option<Arc<World>>,
    once: AtomicBool,
}
impl Transition {
    async fn apply(&self) {
        if self.once.swap(true, Relaxed) {
            return;
        }
        if let Some(world) = &self.world {
            self.player
                .teleport_world(world.clone(), SPAWN, Some(0.0), Some(0.0))
                .await;
        } else {
            self.player.respawn().await;
        }
    }
}
impl EventHandler<PlayerMoveEvent> for Transition {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut PlayerMoveEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(self.apply())
    }
}
impl EventHandler<GenericGameEvent> for Transition {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut GenericGameEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if event.event_key == "step" {
                self.apply().await;
            }
        })
    }
}

async fn accepted(s: &Scenario, p: &Session, turn: bool) {
    let actor = p.player.clone();
    let server = s.server.clone();
    let to = actor.position() + Vector3::new(1.75, 0.0, 0.0);
    s.drive(
        "movement callback completion",
        tokio::task::spawn_blocking(move || {
            if turn {
                actor.client.java().unwrap().handle_position_rotation(
                    &actor,
                    &server,
                    &pumpkin_protocol::java::server::play::SPlayerPositionRotation {
                        position: to,
                        yaw: 0.0,
                        pitch: 0.0,
                        collision: 1,
                    },
                );
            } else {
                actor.client.java().unwrap().handle_position(
                    &actor,
                    &server,
                    &pumpkin_protocol::java::server::play::SPlayerPosition {
                        position: to,
                        collision: 1,
                    },
                );
            }
        }),
    )
    .await
    .unwrap();
}

fn settle(s: &Scenario, p: &Session) {
    let pos = p.player.position();
    let grounded = Vector3::new(pos.x, pos.y.floor(), pos.z);
    let support = BlockPos::new(
        grounded.x.floor() as i32,
        grounded.y as i32 - 1,
        grounded.z.floor() as i32,
    );
    assert_eq!(
        p.player.world().get_block_state_id_if_loaded(&support),
        Some(Block::STONE.default_state.id),
        "destination fixture ground must be loaded"
    );
    p.send(
        &s.server,
        &pumpkin_protocol::java::server::play::SPlayerPosition {
            position: grounded,
            collision: 1,
        },
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s3_real_move_callbacks_replace_life_and_world() {
    for before_commit in [true, false] {
        for transfer in [false, true] {
            run(async move {
                let s = Scenario::new().await;
                let p = s.join(Uuid::new_v4(), "MoveLifecycle").await;
                p.player.set_respawn_point(
                    Dimension::OVERWORLD,
                    BlockPos::new(3, 64, 3),
                    0.0,
                    0.0,
                    true,
                );
                let old_world = p.player.world();
                let old_life = p.player.living_entity.damage_lifecycle();
                let sink = crate::world::game_events::test_support::attach(&old_world);
                let hook = Arc::new(Transition {
                    player: p.player.clone(),
                    world: transfer.then(|| s.world(&Dimension::THE_NETHER)),
                    once: AtomicBool::new(false),
                });
                if before_commit {
                    s.server.plugin_manager.register::<PlayerMoveEvent, _>(
                        hook,
                        EventPriority::High,
                        true,
                    );
                } else {
                    s.server.plugin_manager.register::<GenericGameEvent, _>(
                        hook,
                        EventPriority::High,
                        true,
                    );
                }
                accepted(&s, &p, before_commit).await;
                assert!(
                    sink.lock().unwrap().forwarded.is_empty(),
                    "stale original life/world must never forward STEP"
                );
                if transfer {
                    assert!(!Arc::ptr_eq(&old_world, &p.player.world()));
                } else {
                    assert_ne!(old_life, p.player.living_entity.damage_lifecycle());
                }
                p.loaded(&s.server);
                settle(&s,&p);
                let next_world = p.player.world();
                let next_sink = if Arc::ptr_eq(&next_world, &old_world) {
                    sink.clone()
                } else {
                    crate::world::game_events::test_support::attach(&next_world)
                };
                accepted(&s, &p, !before_commit).await;
                assert_eq!(
                    next_sink.lock().unwrap().forwarded.len(),
                    1,
                    "next independent accepted movement works: before_commit={before_commit}, transfer={transfer}, position={:?}", p.player.position()
                );
                s.stop().await;
            }).await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_warden_sculk_s3_real_rejoin_and_dimension_roundtrip() {
    run(async {
        let s = Scenario::new().await;
        let id = Uuid::new_v4();
        let old = s.join(id, "MoveRejoin").await;
        accepted(&s, &old, false).await;
        s.leave(&old).await;
        let current = s.join(id, "MoveRejoin").await;
        assert!(!Arc::ptr_eq(&old.player, &current.player));
        let sink = crate::world::game_events::test_support::attach(&current.player.world());
        accepted(&s, &old, true).await;
        assert!(
            sink.lock().unwrap().forwarded.is_empty(),
            "retained prior connection is stale"
        );
        accepted(&s, &current, true).await;
        assert_eq!(
            sink.lock().unwrap().forwarded.len(),
            1,
            "new same-UUID session emits once"
        );
        for dim in ["the_nether", "overworld"] {
            super::transfer::transfer(&s, &current, dim).await;
            settle(&s, &current);
            let world = current.player.world();
            let sink = crate::world::game_events::test_support::attach(&world);
            accepted(&s, &current, false).await;
            assert_eq!(
                sink.lock().unwrap().forwarded.len(),
                1,
                "world replacement resets sampling"
            );
        }
        let entity = current.player.get_entity();
        let world = current.player.world();
        let sink = crate::world::game_events::test_support::attach(&world);
        let start = current.player.position();
        let packet = |dx| pumpkin_protocol::java::server::play::SPlayerPosition {
            position: start + Vector3::new(dx, 0.0, 0.0),
            collision: 1,
        };
        current.send(&s.server, &packet(1.5));
        assert!(sink.lock().unwrap().forwarded.is_empty());
        entity.set_world(s.world(&Dimension::THE_NETHER));
        entity.set_world(world);
        current.send(&s.server, &packet(1.75));
        assert!(
            sink.lock().unwrap().forwarded.is_empty(),
            "native same-position world roundtrip discards earlier distance"
        );
        current.send(&s.server, &packet(3.25));
        assert_eq!(sink.lock().unwrap().forwarded.len(), 1);
        let world = current.player.world();
        let sink = crate::world::game_events::test_support::attach(&world);
        let notifications = super::movement_game_events::capture(&s);
        world.remove_player(&current.player, false).await;
        assert!(
            world
                .entity_tracker
                .get_tracked_entity(entity.entity_id)
                .is_none()
        );
        accepted(&s, &current, true).await;
        assert_eq!(
            notifications.load(Relaxed),
            0,
            "detached player cannot notify"
        );
        assert!(sink.lock().unwrap().forwarded.is_empty());
        s.stop().await;
    })
    .await;
}
