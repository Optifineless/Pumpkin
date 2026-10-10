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
    tokio::time::timeout(Duration::from_secs(20), async {
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
