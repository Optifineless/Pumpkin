use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering::Relaxed},
    },
    time::Duration,
};

use pumpkin_data::dimension::Dimension;
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};

use super::*;
use crate::{
    entity::death_test_world::DeathTestWorld,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::player::player_change_world::PlayerChangeWorldEvent,
    },
    server::Server,
    world::portal::PortalProcessor,
};

async fn finish_trip(entity: &Entity) {
    tokio::time::timeout(Duration::from_secs(60), async {
        while entity.portal_travel_pending_for_test() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

fn tick_trip(fixture: &DeathTestWorld, traveler: &dyn EntityBase, destination: Arc<World>) {
    let entity = traveler.get_entity();
    entity.set_pos(Vector3::new(8.5, 80.0, 9.0));
    *entity.portal_manager.lock().unwrap() = Some(PortalProcessor::new(
        PortalType::Nether,
        BlockPos::new(8, 80, 8),
        destination,
    ));
    entity.tick(traveler, &fixture.server);
}

fn keeps_cooldown_in_portal(
    fixture: &DeathTestWorld,
    traveler: &dyn EntityBase,
    destination: &Arc<World>,
) {
    let entity = traveler.get_entity();
    let generation = entity.portal_travel.state.lock().unwrap().generation;
    for _ in 0..3 {
        entity.try_use_portal(destination.clone(), BlockPos::new(8, 80, 8));
        entity.tick(traveler, &fixture.server);
        assert!(!entity.portal_travel_pending_for_test());
        assert_eq!(
            entity.portal_travel.state.lock().unwrap().generation,
            generation
        );
        assert!(entity.portal_cooldown.load(Relaxed) > 0);
    }
}

struct CancelTransfer(Arc<AtomicUsize>);

impl EventHandler<PlayerChangeWorldEvent> for CancelTransfer {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut PlayerChangeWorldEvent,
    ) -> BoxFuture<'a, ()> {
        assert!(event.player.get_entity().portal_cooldown.load(Relaxed) > 0);
        self.0.fetch_add(1, Relaxed);
        event.cancelled = true;
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_player_transfer_keeps_cooldown_and_does_not_retry() {
    let fixture = DeathTestWorld::new().await;
    let player = fixture.player("portal-cancel");
    let destination = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    super::tests::save_destination(&destination, true).await;
    fixture.server.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.game_rules.players_nether_portal_default_delay = 0;
        info
    });
    let cancelled = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<PlayerChangeWorldEvent, _>(
            Arc::new(CancelTransfer(cancelled.clone())),
            EventPriority::Normal,
            true,
        );
    tick_trip(&fixture, player.as_ref(), destination.clone());
    finish_trip(player.get_entity()).await;
    assert_eq!(cancelled.load(Relaxed), 1);
    assert!(Arc::ptr_eq(&player.world(), &fixture.world()));
    assert_eq!(
        player.get_entity().portal_cooldown.load(Relaxed),
        player.get_entity().default_portal_cooldown()
    );
    keeps_cooldown_in_portal(&fixture, player.as_ref(), &destination);
    assert_eq!(cancelled.load(Relaxed), 1);
    fixture.server.shutdown().await;
}

async fn gated_player_trip(
    fixture: &DeathTestWorld,
    name: &str,
) -> (
    Arc<crate::entity::player::Player>,
    Arc<World>,
    tokio::sync::oneshot::Sender<()>,
) {
    let player = fixture.player_with_packet_sink(name);
    let destination = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    super::tests::save_destination(&destination, true).await;
    fixture.server.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.game_rules.players_nether_portal_default_delay = 0;
        info
    });
    let (ready, started) = tokio::sync::oneshot::channel();
    let (resume, gate) = tokio::sync::oneshot::channel();
    *player
        .get_entity()
        .portal_travel
        .transfer_gate
        .lock()
        .unwrap() = Some((ready, gate));
    tick_trip(fixture, player.as_ref(), destination.clone());
    tokio::time::timeout(Duration::from_secs(60), started)
        .await
        .unwrap()
        .unwrap();
    (player, destination, resume)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn portal_disconnect_drains_transfer_before_final_save_and_removal() {
    use crate::entity::player::statistics::CustomStatistic;
    use std::{future::Future, task::Poll};

    let fixture = DeathTestWorld::new().await;
    let (player, destination, resume) = gated_player_trip(&fixture, "portal-logout").await;
    let source = fixture.world();
    let uuid = player.gameprofile.id;
    let (session, _) = fixture
        .server
        .player_data_storage
        .load_data(&uuid)
        .await
        .unwrap();
    *player.storage_session.lock().unwrap() = Some(session);
    let position = player.get_entity().pos.load();
    // This is the same close -> client tracker -> save/remove entry point used by lib.rs.
    let mut disconnect = Box::pin(player.finish_java_connection(&fixture.server));
    std::future::poll_fn(|cx| {
        assert!(disconnect.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    assert!(player.client.closed());
    assert_eq!(
        player.get_custom_stat(CustomStatistic::LeaveGame),
        0,
        "teardown saved before the connection-owned transfer drained"
    );
    assert!(source.get_player_by_id(player.entity_id()).is_some());
    resume.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(60), disconnect)
        .await
        .unwrap();
    assert!(!player.get_entity().portal_travel_pending_for_test());
    assert!(source.get_player_by_id(player.entity_id()).is_none());
    assert!(destination.get_player_by_id(player.entity_id()).is_none());
    assert!(
        Arc::ptr_eq(&player.world(), &source),
        "closed session changed worlds"
    );
    assert_eq!(player.get_entity().pos.load(), position);
    // Reload through the real login storage path after releasing the previous session.
    let (_session, saved) = fixture
        .server
        .player_data_storage
        .load_data(&uuid)
        .await
        .unwrap();
    assert_eq!(
        saved.unwrap().get_string("Dimension"),
        Some(source.dimension.minecraft_name)
    );
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn portal_transfer_rechecks_life_after_final_callback_validation() {
    let fixture = DeathTestWorld::new().await;
    let (player, destination, resume) = gated_player_trip(&fixture, "portal-life").await;
    let position = player.get_entity().pos.load();
    player.living_entity.set_health(0.0);
    player.living_entity.reset_state();
    resume.send(()).unwrap();
    finish_trip(player.get_entity()).await;
    assert!(Arc::ptr_eq(&player.world(), &fixture.world()));
    assert_eq!(player.get_entity().pos.load(), position);
    assert!(
        fixture
            .world()
            .get_player_by_id(player.entity_id())
            .is_some()
    );
    assert!(destination.get_player_by_id(player.entity_id()).is_none());
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn portal_transfer_rechecks_world_after_final_callback_validation() {
    let fixture = DeathTestWorld::new().await;
    let (player, destination, resume) = gated_player_trip(&fixture, "portal-world").await;
    let position = Vector3::new(40.0, 90.0, 40.0);
    let replacement = fixture
        .server
        .create_world("portal-other-world".to_owned(), Dimension::THE_END);
    player.get_entity().set_world(replacement.clone());
    player.get_entity().set_pos(position);
    resume.send(()).unwrap();
    finish_trip(player.get_entity()).await;
    assert!(Arc::ptr_eq(&player.world(), &replacement));
    assert_eq!(player.get_entity().pos.load(), position);
    assert!(destination.get_player_by_id(player.entity_id()).is_none());
    fixture.server.shutdown().await;
}

struct UnloadDestination {
    ready: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    resume: tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
    selected: Arc<World>,
}

impl EventHandler<PlayerChangeWorldEvent> for UnloadDestination {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut PlayerChangeWorldEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            // Exercise a callback-selected destination, rather than only the resolved world.
            event.new_world = self.selected.clone();
            self.ready.lock().unwrap().take().unwrap().send(()).unwrap();
            let resume = self.resume.lock().await.take().unwrap();
            resume.await.unwrap();
            server
                .unload_world(self.selected.get_world_name())
                .await
                .unwrap();
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn portal_transfer_rejects_destination_unloaded_by_world_change_callback() {
    let fixture = DeathTestWorld::new().await;
    let player = fixture.player("portal-unload");
    let destination = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    super::tests::save_destination(&destination, true).await;
    let selected = fixture
        .server
        .create_world("portal-selected".to_owned(), Dimension::THE_END);
    fixture.server.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.game_rules.players_nether_portal_default_delay = 0;
        info
    });
    let (ready, started) = tokio::sync::oneshot::channel();
    let (resume, gate) = tokio::sync::oneshot::channel();
    fixture
        .server
        .plugin_manager
        .register::<PlayerChangeWorldEvent, _>(
            Arc::new(UnloadDestination {
                ready: Mutex::new(Some(ready)),
                resume: tokio::sync::Mutex::new(Some(gate)),
                selected: selected.clone(),
            }),
            EventPriority::Normal,
            true,
        );
    let (admitted, mut admission_started) = tokio::sync::oneshot::channel();
    let (_release, admission_gate) = tokio::sync::oneshot::channel();
    *player
        .get_entity()
        .portal_travel
        .transfer_gate
        .lock()
        .unwrap() = Some((admitted, admission_gate));
    tick_trip(&fixture, player.as_ref(), destination.clone());
    tokio::time::timeout(Duration::from_secs(60), started)
        .await
        .unwrap()
        .unwrap();
    assert!(selected.players.load().is_empty());
    resume.send(()).unwrap();
    tokio::select! {
        () = finish_trip(player.get_entity()) => {},
        result = &mut admission_started => {
            result.unwrap();
            assert!(!selected.level.cancel_token.is_cancelled(), "transfer admitted an unloaded callback-selected world");
        },
    }

    assert!(selected.level.cancel_token.is_cancelled());
    assert!(selected.get_player_by_id(player.entity_id()).is_none());
    assert!(destination.get_player_by_id(player.entity_id()).is_none());
    assert!(Arc::ptr_eq(&player.world(), &fixture.world()));
    assert!(
        fixture
            .world()
            .get_player_by_id(player.entity_id())
            .is_some()
    );
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn portal_destination_admission_blocks_unload_until_publication() {
    let fixture = DeathTestWorld::new().await;
    let (player, destination, resume) = gated_player_trip(&fixture, "portal-admission").await;
    // The transfer is past callbacks with admission held, but has no destination membership.
    assert!(destination.players.load().is_empty());
    let result = fixture
        .server
        .unload_world(destination.dimension.minecraft_name)
        .await;
    assert!(result.is_err(), "unload stopped an admitted destination");
    assert!(!destination.level.cancel_token.is_cancelled());
    player.client.java().unwrap().close();
    resume.send(()).unwrap();
    finish_trip(player.get_entity()).await;
    // An aborted transfer releases admission exactly once.
    fixture
        .server
        .unload_world(destination.dimension.minecraft_name)
        .await
        .unwrap();
    assert!(destination.level.cancel_token.is_cancelled());
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn portal_aborted_unload_reopens_destination_admission() {
    let fixture = DeathTestWorld::new().await;
    let destination = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    let (started, _resume) = destination.gate_unload_for_test();
    let server = fixture.server.clone();
    let unloading = tokio::spawn(async move {
        server
            .unload_world(Dimension::THE_NETHER.minecraft_name)
            .await
    });
    tokio::time::timeout(Duration::from_secs(60), started)
        .await
        .unwrap()
        .unwrap();
    assert!(destination.admit_transfer().is_none());
    unloading.abort();
    assert!(unloading.await.unwrap_err().is_cancelled());
    assert!(!destination.level.cancel_token.is_cancelled());
    let admission = destination.admit_transfer();
    assert!(
        admission.is_some(),
        "aborted unload stranded a running world"
    );
    drop(admission);
    fixture.server.shutdown().await;
}
