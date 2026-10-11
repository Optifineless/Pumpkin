use std::{
    num::NonZero,
    sync::{Arc, Mutex},
    time::Duration,
};

use pumpkin_data::{dimension::Dimension, entity::EntityType};
use pumpkin_protocol::MultiVersionJavaPacket;
use pumpkin_util::math::{vector2::Vector2, vector3::Vector3};
use pumpkin_world::cylindrical_chunk_iterator::Cylindrical;

use super::{
    Entity, EntityBase, PortalType,
    review_tests::{finish_trip, gated_player_trip, tick_trip},
};
use crate::{
    entity::{death_test_world::DeathTestWorld, player::Player},
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::{
            player::player_change_world::PlayerChangeWorldEvent, world::chunk_send::ChunkSend,
        },
    },
    server::Server,
    world::World,
};

struct RespawnInterrupt {
    respawns: Arc<std::sync::atomic::AtomicUsize>,
    interrupt_respawn: bool,
}

impl EventHandler<crate::plugin::server::packet::PacketSentEvent> for RespawnInterrupt {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut crate::plugin::server::packet::PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        if event.packet_id
            == pumpkin_protocol::java::client::play::CRespawn::to_id(
                pumpkin_data::packet::CURRENT_MC_VERSION,
            )
        {
            if self.interrupt_respawn {
                event
                    .player
                    .request_teleport(Vector3::new(10.0, 82.0, 11.0), 42.0, 15.0);
            }
            self.respawns
                .fetch_add(1, std::sync::atomic::Ordering::Release);
        } else if !self.interrupt_respawn
            && event.packet_id
                == pumpkin_protocol::java::client::play::CUnloadChunk::to_id(
                    pumpkin_data::packet::CURRENT_MC_VERSION,
                )
        {
            event
                .player
                .request_teleport(Vector3::new(10.0, 82.0, 11.0), 42.0, 15.0);
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dimension_packet_callback_observes_committed_membership_and_cannot_overwrite_newer_teleport()
 {
    dimension_packet_interrupt(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_unload_packet_callback_still_publishes_the_committed_dimension() {
    dimension_packet_interrupt(false).await;
}

async fn dimension_packet_interrupt(interrupt_respawn: bool) {
    let fixture = DeathTestWorld::new().await;
    let (player, destination, resume) = gated_player_trip(&fixture, "portal-respawn-write").await;
    player.config.rcu(|config| {
        let mut config = (**config).clone();
        config.view_distance = NonZero::new(2).unwrap();
        config
    });
    let watched = Cylindrical::new(Vector2::new(0, 0), NonZero::new(2).unwrap());
    player.watched_section.store(watched);
    fixture
        .world()
        .level
        .mark_chunks_as_newly_watched(&watched.all_chunks_within().collect::<Vec<_>>())
        .await;
    let client = player.client.java().unwrap();
    client.player.store(Arc::new(Some(player.clone())));
    client
        .version
        .store(pumpkin_util::version::JavaMinecraftVersion::V_1_21);
    let respawns = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<crate::plugin::server::packet::PacketSentEvent, _>(
            Arc::new(RespawnInterrupt {
                respawns: respawns.clone(),
                interrupt_respawn,
            }),
            EventPriority::Normal,
            true,
        );
    resume.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(60), client.await_tasks())
        .await
        .unwrap();
    assert_eq!(respawns.load(std::sync::atomic::Ordering::Acquire), 1);
    assert!(
        Arc::ptr_eq(&player.world(), &destination),
        "client switched dimension before transfer committed"
    );
    assert!(destination.get_player_by_id(player.entity_id()).is_some());
    assert!(
        fixture
            .world()
            .get_player_by_id(player.entity_id())
            .is_none()
    );
    assert_eq!(player.position(), Vector3::new(10.0, 82.0, 11.0));
    assert_eq!(player.get_entity().yaw.load(), 42.0);
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalidated_search_releases_gate_without_clearing_new_trip() {
    let fixture = DeathTestWorld::new().await;
    let destination = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    super::tests::save_destination(&destination, true).await;
    let traveler = fixture.mob(&EntityType::COW);
    let entity = traveler.get_entity();
    let (ready, started) = tokio::sync::oneshot::channel();
    let (old_resume, gate) = tokio::sync::oneshot::channel();
    *entity.portal_travel.continuation_gate.lock().unwrap() = Some((ready, gate));
    tick_trip(&fixture, traveler.as_ref(), destination.clone());
    tokio::time::timeout(Duration::from_secs(60), started)
        .await
        .unwrap()
        .unwrap();
    entity.teleport(Vector3::new(12.0, 80.0, 12.0), None, None, &fixture.world());
    let (ready, started) = tokio::sync::oneshot::channel();
    let (resume, gate) = tokio::sync::oneshot::channel();
    *entity.portal_travel.continuation_gate.lock().unwrap() = Some((ready, gate));
    entity.teleport_through_portal(PortalType::Nether, destination.clone(), None);
    tokio::time::timeout(Duration::from_secs(60), started)
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(60), async {
        while !old_resume.is_closed() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        entity.portal_travel_pending_for_test(),
        "old destructor cleared the new trip"
    );
    resume.send(()).unwrap();
    finish_trip(entity).await;
    assert!(Arc::ptr_eq(&entity.world.load_full(), &destination));
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejected_transfer_preserves_source_watches_and_entities() {
    let fixture = DeathTestWorld::new().await;
    let player = fixture.player_with_packet_sink("portal-watches");
    player.config.rcu(|config| {
        let mut config = (**config).clone();
        config.view_distance = NonZero::new(2).unwrap();
        config
    });
    let destination = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    super::tests::save_destination(&destination, true).await;
    fixture.server.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.game_rules.players_nether_portal_default_delay = 0;
        info
    });
    let source = fixture.world();
    let watched = Cylindrical::new(Vector2::new(0, 0), NonZero::new(2).unwrap());
    player.watched_section.store(watched);
    let chunks: Vec<_> = watched.all_chunks_within().collect();
    source.level.mark_chunks_as_newly_watched(&chunks).await;
    let cow = fixture.mob(&EntityType::COW);
    let id = cow.get_entity().entity_id;
    let (ready, started) = tokio::sync::oneshot::channel();
    let (resume, gate) = tokio::sync::oneshot::channel();
    *player
        .get_entity()
        .portal_travel
        .transfer_gate
        .lock()
        .unwrap() = Some((ready, gate));
    tick_trip(&fixture, player.as_ref(), destination.clone());
    tokio::time::timeout(Duration::from_secs(60), started)
        .await
        .unwrap()
        .unwrap();
    assert!(
        player.watched_section.load() == watched,
        "source watches changed before publication"
    );
    assert!(
        chunks
            .iter()
            .all(|chunk| source.level.is_chunk_watched(chunk))
    );
    player.request_teleport(Vector3::new(9.0, 80.0, 9.0), 31.0, 12.0);
    let _ = resume.send(());
    tokio::time::timeout(
        Duration::from_secs(60),
        player.client.java().unwrap().await_tasks(),
    )
    .await
    .unwrap();
    assert!(player.watched_section.load() == watched);
    for chunk in chunks {
        assert!(
            source.level.mark_chunk_as_not_watched(chunk).await,
            "rejected transfer lost its source watch"
        );
        source.level.mark_chunks_as_newly_watched(&[chunk]).await;
    }
    assert!(
        source.get_entity_by_id(id).is_some(),
        "rejected transfer unloaded a source entity"
    );
    assert!(source.get_player_by_id(player.entity_id()).is_some());
    assert!(destination.get_player_by_id(player.entity_id()).is_none());
    fixture.server.shutdown().await;
}

struct ChunkGate {
    ready: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    resume: tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
}

impl EventHandler<ChunkSend> for ChunkGate {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        _event: &'a mut ChunkSend,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let ready = self.ready.lock().unwrap().take();
            if let Some(ready) = ready {
                ready.send(()).unwrap();
                let resume = self.resume.lock().await.take().unwrap();
                let _ = resume.await;
            }
        })
    }
}

async fn arrival_in_chunk_send(close: bool) {
    let fixture = DeathTestWorld::new().await;
    let player = fixture.player_with_packet_sink("portal-chunk-send");
    if close {
        let (session, _) = fixture
            .server
            .player_data_storage
            .load_data(&player.gameprofile.id)
            .await
            .unwrap();
        *player.storage_session.lock().unwrap() = Some(session);
    }
    player
        .client
        .java()
        .unwrap()
        .player
        .store(Arc::new(Some(player.clone())));
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
    fixture.server.plugin_manager.register::<ChunkSend, _>(
        Arc::new(ChunkGate {
            ready: Mutex::new(Some(ready)),
            resume: tokio::sync::Mutex::new(Some(gate)),
        }),
        EventPriority::Normal,
        true,
    );
    tick_trip(&fixture, player.as_ref(), destination.clone());
    tokio::time::timeout(Duration::from_secs(60), started)
        .await
        .unwrap()
        .unwrap();
    assert!(Arc::ptr_eq(&player.world(), &destination));
    if close {
        tokio::time::timeout(
            Duration::from_secs(60),
            player.finish_java_connection(&fixture.server),
        )
        .await
        .unwrap();
        assert!(destination.get_player_by_id(player.entity_id()).is_none());
        let (_session, saved) = fixture
            .server
            .player_data_storage
            .load_data(&player.gameprofile.id)
            .await
            .unwrap();
        assert_eq!(
            saved.unwrap().get_string("Dimension"),
            Some(destination.dimension.minecraft_name)
        );
    } else {
        let newer = Vector3::new(10.0, 82.0, 11.0);
        player.request_teleport(newer, 42.0, 15.0);
        let _ = resume.send(());
        tokio::time::timeout(
            Duration::from_secs(60),
            player.client.java().unwrap().await_tasks(),
        )
        .await
        .unwrap();
        assert_eq!(player.position(), newer);
        assert_eq!(player.get_entity().yaw.load(), 42.0);
        assert_eq!(player.get_entity().pitch.load(), 15.0);
        fixture.server.shutdown().await;
        return;
    }
    tokio::time::timeout(Duration::from_secs(60), async {
        while !resume.is_closed() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    fixture.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn newer_teleport_cancels_arrival_inside_real_chunk_send() {
    arrival_in_chunk_send(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn logout_cancels_arrival_inside_real_chunk_send_without_releasing_gate() {
    arrival_in_chunk_send(true).await;
}

struct Redirect(Arc<World>);

impl EventHandler<PlayerChangeWorldEvent> for Redirect {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut PlayerChangeWorldEvent,
    ) -> BoxFuture<'a, ()> {
        event.new_world = self.0.clone();
        event.position = Vector3::new(5.0, 81.0, 6.0);
        event.yaw = 80.0;
        event.pitch = 11.0;
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn redirected_player_moves_passenger_to_applied_world_and_rotation() {
    let fixture = DeathTestWorld::new().await;
    let player: Arc<Player> = fixture.player_with_packet_sink("portal-redirect");
    let destination = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    let selected = fixture
        .server
        .create_world("portal-redirected".to_owned(), Dimension::THE_NETHER);
    super::tests::save_destination(&destination, true).await;
    super::tests::save_destination(&selected, true).await;
    fixture.server.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.game_rules.players_nether_portal_default_delay = 0;
        info
    });
    fixture
        .server
        .plugin_manager
        .register::<PlayerChangeWorldEvent, _>(
            Arc::new(Redirect(selected.clone())),
            EventPriority::Normal,
            true,
        );
    player.get_entity().yaw.store(30.0);
    player.get_entity().pitch.store(4.0);
    let passenger = Arc::new(Entity::new(
        fixture.world(),
        player.position(),
        &EntityType::COW,
    ));
    passenger.yaw.store(15.0);
    passenger.pitch.store(7.0);
    fixture.world().add_entity_silent(passenger.clone());
    player
        .get_entity()
        .add_passenger(player.clone(), passenger.clone());
    let nested = Arc::new(Entity::new(
        fixture.world(),
        player.position(),
        &EntityType::COW,
    ));
    nested.yaw.store(5.0);
    nested.pitch.store(3.0);
    fixture.world().add_entity_silent(nested.clone());
    passenger.add_passenger(passenger.clone(), nested.clone());
    tick_trip(&fixture, player.as_ref(), destination);
    finish_trip(player.get_entity()).await;
    assert!(Arc::ptr_eq(&player.world(), &selected));
    assert!(Arc::ptr_eq(&passenger.world.load_full(), &selected));
    assert_eq!(passenger.pos.load(), player.position());
    assert_eq!(passenger.yaw.load(), 65.0);
    assert_eq!(passenger.pitch.load(), 14.0);
    assert!(Arc::ptr_eq(&nested.world.load_full(), &selected));
    assert_eq!(nested.pos.load(), player.position());
    assert_eq!(nested.yaw.load(), 55.0);
    assert_eq!(nested.pitch.load(), 10.0);
    for entity in [&passenger, &nested] {
        assert!(selected.get_entity_by_id(entity.entity_id).is_some());
        assert!(fixture.world().get_entity_by_id(entity.entity_id).is_none());
    }
    fixture.server.shutdown().await;
}

struct CountRespawns(Arc<std::sync::atomic::AtomicUsize>);

impl EventHandler<crate::plugin::server::packet::PacketSentEvent> for CountRespawns {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut crate::plugin::server::packet::PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        if event.packet_id
            == pumpkin_protocol::java::client::play::CRespawn::to_id(
                pumpkin_data::packet::CURRENT_MC_VERSION,
            )
        {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_selected_by_world_change_keeps_watches_and_attached_mob() {
    let fixture = DeathTestWorld::new().await;
    let source = fixture.world();
    let destination = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    super::tests::save_destination(&destination, true).await;
    fixture.server.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.game_rules.players_nether_portal_default_delay = 0;
        info
    });
    let player = fixture.player_with_packet_sink("portal-source-redirect");
    player.config.rcu(|config| {
        let mut config = (**config).clone();
        config.view_distance = NonZero::new(2).unwrap();
        config
    });
    let watched = Cylindrical::new(Vector2::new(0, 0), NonZero::new(2).unwrap());
    player.watched_section.store(watched);
    let chunks: Vec<_> = watched.all_chunks_within().collect();
    source.level.mark_chunks_as_newly_watched(&chunks).await;
    let java = player.client.java().unwrap();
    java.player.store(Arc::new(Some(player.clone())));
    java.version
        .store(pumpkin_util::version::JavaMinecraftVersion::V_1_21);
    let cow = fixture.mob(&EntityType::COW);
    player
        .get_entity()
        .add_passenger(player.clone(), cow.clone());
    let respawns = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<PlayerChangeWorldEvent, _>(
            Arc::new(Redirect(source.clone())),
            EventPriority::Normal,
            true,
        );
    fixture
        .server
        .plugin_manager
        .register::<crate::plugin::server::packet::PacketSentEvent, _>(
            Arc::new(CountRespawns(respawns.clone())),
            EventPriority::Normal,
            true,
        );
    tick_trip(&fixture, player.as_ref(), destination);
    tokio::time::timeout(Duration::from_secs(60), java.await_tasks())
        .await
        .unwrap();
    // Drain source cleanup too: admission release is not an arrival completion signal.
    fixture.server.shutdown().await;
    assert_eq!(respawns.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert!(Arc::ptr_eq(&player.world(), &source));
    assert!(player.watched_section.load() == watched);
    assert!(
        chunks
            .iter()
            .all(|chunk| source.level.is_chunk_watched(chunk))
    );
    assert!(source.get_player_by_id(player.entity_id()).is_some());
    assert!(
        source
            .get_entity_by_id(cow.get_entity().entity_id)
            .is_some()
    );
    assert!(
        player
            .get_entity()
            .has_passenger(cow.get_entity().entity_id)
    );
    assert_eq!(cow.get_entity().pos.load(), player.position());
    assert_eq!(player.position(), Vector3::new(5.0, 81.0, 6.0));
}
