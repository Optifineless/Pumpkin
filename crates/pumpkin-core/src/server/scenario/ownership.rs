//! Keep fixture resources alive through unwinding, then drain them on the test's runtime.
use super::*;
use futures::FutureExt;
use std::{any::Any, panic::AssertUnwindSafe};

type Failure = Box<dyn Any + Send>;

pub struct Fixture {
    pub directory: tempfile::TempDir,
    pub(super) isolation: tokio::sync::MutexGuard<'static, ()>,
}

#[derive(Default)]
struct Owners {
    scenarios: Vec<Scenario>,
    fixtures: Vec<Rc<Fixture>>,
}

tokio::task_local! {
    static OWNERS: RefCell<Owners>;
}

pub(super) fn register(scenario: &Scenario) {
    OWNERS.with(|owners| {
        let mut owners = owners.borrow_mut();
        if !owners
            .fixtures
            .iter()
            .any(|fixture| Rc::ptr_eq(fixture, &scenario.fixture))
        {
            owners.fixtures.push(scenario.fixture.clone());
        }
        owners.scenarios.push(scenario.clone());
    });
}

async fn attempt<T>(
    failure: &mut Option<Failure>,
    operation: impl Future<Output = T>,
) -> Option<T> {
    match AssertUnwindSafe(operation).catch_unwind().await {
        Ok(value) => Some(value),
        Err(error) => {
            if failure.is_none() {
                *failure = Some(error);
            }
            None
        }
    }
}

/// Run every scenario body here so assertions and polling failures get asynchronous cleanup.
pub fn run(body: impl Future<Output = ()>) -> impl Future<Output = ()> {
    Box::pin(OWNERS.scope(RefCell::default(), async move {
        let mut caught = Box::pin(AssertUnwindSafe(body).catch_unwind());
        let outcome = caught.as_mut().await;
        // Unwound future locals must drop before client cleanup and directory removal.
        drop(caught);
        let mut failure = None;
        loop {
            let scenario = OWNERS.with(|owners| owners.borrow_mut().scenarios.pop());
            let Some(scenario) = scenario else { break };
            attempt(&mut failure, scenario.stop()).await;
        }
        let fixtures = OWNERS.with(|owners| std::mem::take(&mut owners.borrow_mut().fixtures));
        for fixture in fixtures {
            attempt(&mut failure, async move {
                let Ok(Fixture {
                    directory,
                    isolation,
                }) = Rc::try_unwrap(fixture)
                else {
                    panic!("scenario fixture still owned after cleanup")
                };
                let path = directory.path().to_owned();
                directory.close().unwrap();
                assert!(
                    !path.exists(),
                    "scenario directory survived cleanup: {path:?}"
                );
                drop(isolation);
            })
            .await;
        }
        // A cleanup assertion must never replace the scenario's original failure.
        if let Err(error) = outcome {
            std::panic::resume_unwind(error);
        }
        if let Some(error) = failure {
            std::panic::resume_unwind(error);
        }
    }))
}

impl Scenario {
    async fn final_snapshots(&self, failure: &mut Option<Failure>) {
        let gate = attempt(
            failure,
            self.drive("shutdown tick gate", self.server.tick_gate.lock()),
        )
        .await;
        attempt(failure, async {
            self.drive(
                "final player snapshots",
                self.server
                    .player_data_storage
                    .save_all_players(&self.server),
            )
            .await
            .unwrap();
        })
        .await;
        attempt(failure, async {
            self.drive(
                "final advancements",
                self.server
                    .advancement_manager
                    .save_all_players(&self.server.get_all_players()),
            )
            .await
            .unwrap();
        })
        .await;
        let message = pumpkin_util::text::TextComponent::text("Server stopped");
        attempt(failure, async {
            for session in self.sessions.borrow().iter() {
                if !session.client().is_closed() {
                    session
                        .player
                        .kick(crate::net::DisconnectReason::Shutdown, &message);
                }
            }
        })
        .await;
        drop(gate);
    }

    async fn close_sessions(&self, failure: &mut Option<Failure>) {
        let sessions = self.sessions.borrow().clone();
        for session in sessions {
            // Already-closed clients may still need the connection owner's storage drain.
            let has_storage = session
                .player
                .storage_session
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_some();
            if has_storage || client_task_count(session.client()) != 0 {
                attempt(failure, self.leave(&session)).await;
            }
            session.client().close();
            attempt(
                failure,
                self.drive("closed client tasks", session.client().await_tasks()),
            )
            .await;
            session
                .player
                .storage_session
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            session.player.client.release_admission();
        }
    }

    /// Mirrors `MinecraftServer.stopServer` through `PumpkinServer::shutdown`, without
    /// cancelling the process-global `STOP_INTERRUPT` shared by unrelated unit tests.
    pub async fn stop(&self) {
        if self.stopped.get() {
            return;
        }
        let mut failure = None;
        self.final_snapshots(&mut failure).await;
        self.close_sessions(&mut failure).await;
        let shutdown = attempt(&mut failure, async {
            // Closed transports need no pump. Let scheduler joins and disk I/O wake
            // cleanup instead of exhausting cooperative polls on a multithread runtime.
            tokio::time::timeout(Duration::from_secs(30), self.server.shutdown())
                .await
                .expect("scenario server shutdown timed out");
        })
        .await;
        for world in self.server.worlds.load().iter() {
            if shutdown.is_none() {
                attempt(&mut failure, async {
                    self.drive("fallback level shutdown", world.level.shutdown())
                        .await
                        .unwrap();
                })
                .await;
            }
            // Fixture WorldPortal owns World strongly; shutdown alone does not break the cycle.
            world.level.world_portal.store(Arc::new(None));
        }
        self.sessions.borrow_mut().clear();
        self.stopped.set(true);
        OWNERS.with(|owners| {
            owners
                .borrow_mut()
                .scenarios
                .retain(|scenario| !Arc::ptr_eq(&scenario.server, &self.server));
        });
        if let Some(error) = failure {
            std::panic::resume_unwind(error);
        }
    }

    pub async fn restart(self) -> Self {
        let game_events = self.server.advanced_config.sculk_game_events.enabled;
        self.restart_with_game_events(game_events).await
    }

    pub(crate) async fn restart_with_game_events(self, game_events: bool) -> Self {
        self.stop().await;
        let hardcore = self.server.basic_config.hardcore;
        let Self {
            server, fixture, ..
        } = self;
        drop(server); // Release the real session lock before opening the same directory.
        let scenario = Self {
            server: Self::services(fixture.directory.path(), hardcore, game_events),
            fixture,
            sessions: Rc::default(),
            stopped: Rc::default(),
        };
        register(&scenario);
        scenario
    }
}
