use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::Relaxed},
};

use futures::future::BoxFuture;

use super::{horse_spawn_stats_test_rng::Script, horse_spawn_stats_test_support::*};
use crate::{
    plugin::{EventHandler, EventPriority, api::events::entity::entity_spawn::EntitySpawnEvent},
    server::Server,
};

#[derive(Default)]
struct RejectHorse(AtomicUsize);

impl EventHandler<EntitySpawnEvent> for RejectHorse {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntitySpawnEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.0.fetch_add(1, Relaxed);
            assert!(event.world.get_entity_by_id(event.entity_id).is_none());
            event.cancelled = true;
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feat_horses_s1_cancelled_admission_does_not_publish_or_retry() {
    tokio::time::timeout(std::time::Duration::from_secs(90), async {
        let fixture = Fixture::new(true);
        let handler = Arc::new(RejectHorse::default());
        fixture
            .server
            .plugin_manager
            .register::<EntitySpawnEvent, _>(handler.clone(), EventPriority::Normal, true);
        let script = Script::new([3, 4], [0.25, 0.5, 0.75, 0.125, 0.25, 0.375], 1);
        fixture
            .server
            .command_dispatcher
            .load()
            .execute_input(
                "summon horse 8.5 64 8.5",
                &fixture.actor.player.get_command_source(&fixture.server),
            )
            .unwrap();
        script.consumed();
        assert_eq!(handler.0.load(Relaxed), 1);
        assert!(fixture.world.entities.load().is_empty());
        fixture.finish().await;
    })
    .await
    .unwrap();
}
