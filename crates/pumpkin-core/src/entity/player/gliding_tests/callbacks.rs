use super::*;

#[derive(Clone, Copy, Debug)]
enum Change {
    Cancel,
    Redirect,
    Gear,
    Ground,
    Water,
    Ability,
    Unloaded,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g08_bedrock_glide_toggle_cancellation_and_live_eligibility() {
    for route in [
        Route::Auth,
        Route::Legacy,
        Route::Embedded,
        Route::Both,
        Route::Java,
    ] {
        for change in [
            Change::Cancel,
            Change::Redirect,
            Change::Gear,
            Change::Ground,
            Change::Water,
            Change::Ability,
            Change::Unloaded,
        ] {
            Fixture::check(async |f| {
                let player = f.player(route).clone();
                let calls = Arc::new(AtomicUsize::new(0));
                let called = calls.clone();
                f.server
                    .plugin_manager
                    .register::<EntityToggleGlideEvent, _>(
                        Arc::new(Callback(move |event: &mut EntityToggleGlideEvent| {
                            if called.fetch_add(1, Relaxed) != 0 {
                                return;
                            }
                            match change {
                                Change::Cancel => event.cancelled = true,
                                Change::Redirect => event.is_gliding = false,
                                Change::Gear => {
                                    player.inventory.set_slot(38, ItemStack::EMPTY.clone());
                                }
                                Change::Ground => {
                                    player.get_entity().on_ground.store(true, Relaxed);
                                }
                                Change::Water => {
                                    player.get_entity().touching_water.store(true, Relaxed);
                                }
                                Change::Ability => player.set_flying(true),
                                Change::Unloaded => player.set_client_loaded(false),
                            }
                        })),
                        EventPriority::Normal,
                        true,
                    );
                f.request(route, true);
                assert!(
                    !f.player(route).get_entity().is_fall_flying(),
                    "callback {change:?} ignored by {route:?}"
                );
                assert_eq!(calls.load(Relaxed), 1);
                let player = f.player(route);
                player
                    .inventory
                    .set_slot(38, ItemStack::new(1, &Item::ELYTRA));
                player.get_entity().on_ground.store(false, Relaxed);
                player.get_entity().touching_water.store(false, Relaxed);
                player.set_flying(false);
                player.set_client_loaded(true);
                f.request(route, true);
                assert!(
                    f.player(route).get_entity().is_fall_flying(),
                    "valid retry after {change:?}"
                );
            })
            .await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g08_bedrock_glide_stop_cancellation_and_payload() {
    for route in [Route::Auth, Route::Legacy, Route::Embedded] {
        for redirect in [false, true] {
            Fixture::check(async |f| {
                f.request(route, true);
                assert!(f.owner.player.get_entity().is_fall_flying());
                let called = Arc::new(AtomicUsize::new(0));
                f.server
                    .plugin_manager
                    .register::<EntityToggleGlideEvent, _>(
                        Arc::new(Callback(move |event: &mut EntityToggleGlideEvent| {
                            assert!(!event.is_gliding);
                            if called.fetch_add(1, Relaxed) == 0 {
                                if redirect {
                                    event.is_gliding = true;
                                } else {
                                    event.cancelled = true;
                                }
                            }
                        })),
                        EventPriority::Normal,
                        true,
                    );
                f.tick(19);
                f.request(route, false);
                assert!(
                    f.owner.player.get_entity().is_fall_flying(),
                    "cancelled stop {route:?}"
                );
                f.tick(1);
                assert_eq!(
                    f.owner.player.inventory.get_slot(38).get_damage(),
                    1,
                    "cancelled stop reset cadence"
                );
                f.request(route, false);
                assert!(!f.owner.player.get_entity().is_fall_flying());
            })
            .await;
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Transition {
    Respawn,
    Transfer,
}

struct LifecycleCallback {
    player: Arc<Player>,
    destination: Arc<World>,
    transition: Transition,
    calls: AtomicUsize,
}

impl EventHandler<EntityToggleGlideEvent> for LifecycleCallback {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut EntityToggleGlideEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                if self.calls.fetch_add(1, Relaxed) != 0 {
                    return;
                }
                match self.transition {
                    Transition::Respawn => {
                        self.player.living_entity.set_health(0.0);
                        self.player.living_entity.dead.store(true, Relaxed);
                        assert!(
                            self.player
                                .world()
                                .respawn_player_with_outcome(&self.player, false, false)
                                .await
                        );
                    }
                    Transition::Transfer => {
                        self.player
                            .teleport_world(
                                self.destination.clone(),
                                Vector3::new(4.0, 80.0, 4.0),
                                None,
                                None,
                            )
                            .await;
                        assert!(Arc::ptr_eq(&self.player.world(), &self.destination));
                    }
                }
                self.player
                    .inventory
                    .set_slot(38, ItemStack::new(1, &Item::ELYTRA));
                self.player.get_entity().on_ground.store(false, Relaxed);
                self.player.set_client_loaded(true);
            })
            .await
            .expect("glide callback lifecycle operation timed out");
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g08_bedrock_glide_completed_respawn_and_transfer() {
    for route in [Route::Auth, Route::Legacy, Route::Embedded, Route::Both] {
        for transition in [Transition::Respawn, Transition::Transfer] {
            Fixture::check(async |f| {
                let destination = test_world(&f.server, &f.directory.path().join("destination"));
                f.extra_worlds.push(destination.clone());
                let player = f.owner.player.clone();
                let server = f.server.clone();
                let handler = Arc::new(LifecycleCallback {
                    player: player.clone(),
                    destination: destination.clone(),
                    transition,
                    calls: AtomicUsize::new(0),
                });
                f.server
                    .plugin_manager
                    .register::<EntityToggleGlideEvent, _>(
                        handler.clone(),
                        EventPriority::Normal,
                        true,
                    );
                let request =
                    tokio::task::spawn_blocking(move || request(route, &player, &server, true));
                tokio::time::timeout(
                    std::time::Duration::from_secs(15),
                    f.owner.with_outgoing_writer(request),
                )
                .await
                .unwrap()
                .unwrap();
                assert_eq!(handler.calls.load(Relaxed), 1);
                assert!(
                    !f.owner.player.get_entity().is_fall_flying(),
                    "stale {route:?} after {transition:?}"
                );
                f.request(route, true);
                assert!(
                    f.owner.player.get_entity().is_fall_flying(),
                    "valid request after {transition:?}"
                );
                f.tick(20);
                assert_eq!(f.owner.player.inventory.get_slot(38).get_damage(), 1);
            })
            .await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g08_bedrock_glide_reentrant_toggle_cannot_start_new_life() {
    for route in [
        Route::Auth,
        Route::Legacy,
        Route::Embedded,
        Route::Both,
        Route::Java,
    ] {
        Fixture::check(async |f| {
            let player = f.player(route).clone();
            let calls = Arc::new(AtomicUsize::new(0));
            let called = calls.clone();
            f.server
                .plugin_manager
                .register::<EntityToggleGlideEvent, _>(
                    Arc::new(Callback(move |_: &mut EntityToggleGlideEvent| {
                        if called.fetch_add(1, Relaxed) == 0 {
                            player.living_entity.reset_state();
                        }
                    })),
                    EventPriority::Normal,
                    true,
                );
            f.request(route, true);
            assert!(
                !f.player(route).get_entity().is_fall_flying(),
                "old {route:?} request started new life"
            );
            assert_eq!(
                calls.load(Relaxed),
                1,
                "duplicate packet repeated a stale request"
            );
            f.request(route, true);
            assert!(
                f.player(route).get_entity().is_fall_flying(),
                "current life rejected {route:?}"
            );
            f.tick(20);
            assert_eq!(f.player(route).inventory.get_slot(38).get_damage(), 1);
        })
        .await;
    }
}

struct RejoinCallback {
    old: Arc<Player>,
    connection: Arc<ClientPlatform>,
    replacement: std::sync::OnceLock<Arc<Player>>,
}

impl EventHandler<EntityToggleGlideEvent> for RejoinCallback {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        _: &'a mut EntityToggleGlideEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                if self.replacement.get().is_some() {
                    return;
                }
                let world = self.old.world();
                let ClientPlatform::Bedrock(client) = self.old.client.as_ref() else {
                    panic!("Bedrock rejoin")
                };
                client.close().await;
                assert!(world.remove_player(&self.old, false).await.is_some());
                let (player, joined_world) = server
                    .add_player(self.connection.clone(), self.old.gameprofile.clone(), None)
                    .await
                    .unwrap();
                assert!(Arc::ptr_eq(&world, &joined_world));
                player.get_entity().set_pos(Vector3::new(4.0, 80.0, 4.0));
                player.set_client_loaded(true);
                player
                    .inventory
                    .set_slot(38, ItemStack::new(1, &Item::ELYTRA));
                world.players.rcu(|players| {
                    let mut players = (**players).clone();
                    players.push(player.clone());
                    players
                });
                world
                    .entity_tracker
                    .add_entity(&(player.clone() as Arc<dyn EntityBase>), &world);
                assert!(self.replacement.set(player).is_ok());
            })
            .await
            .expect("glide callback lifecycle operation timed out");
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g08_bedrock_glide_disconnect_rejoin_keeps_session_identity() {
    for route in [Route::Auth, Route::Legacy, Route::Embedded, Route::Both] {
        Fixture::check(async |f| {
            let transport = TestBedrockPlayer::new(&f.world).await;
            let connection = transport.player.client.clone();
            f.extra_bedrock.push(transport);
            f.world.players.store(Arc::new(vec![
                f.owner.player.clone(),
                f.java.player.clone(),
                f.observer.player.clone(),
            ]));
            let handler = Arc::new(RejoinCallback {
                old: f.owner.player.clone(),
                connection,
                replacement: std::sync::OnceLock::new(),
            });
            f.server
                .plugin_manager
                .register::<EntityToggleGlideEvent, _>(
                    handler.clone(),
                    EventPriority::Normal,
                    true,
                );
            f.request(route, true);
            assert!(
                !f.owner.player.get_entity().is_fall_flying(),
                "old disconnected request glided"
            );
            let replacement = handler.replacement.get().unwrap();
            assert!(
                !replacement.get_entity().is_fall_flying(),
                "old request glided rejoined session"
            );
            request(route, replacement, &f.server, true);
            assert!(
                replacement.get_entity().is_fall_flying(),
                "fresh rejoin request rejected"
            );
            for _ in 0..20 {
                replacement.tick(&f.server);
            }
            assert_eq!(replacement.inventory.get_slot(38).get_damage(), 1);
        })
        .await;
    }
}
