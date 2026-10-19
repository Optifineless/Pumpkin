#![expect(
    clippy::unwrap_used,
    reason = "Portal regressions require successful fixtures and bounded completion"
)]

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use pumpkin_data::{Block, block_properties::HorizontalAxis, dimension::Dimension};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use tokio::sync::oneshot;

use super::{NetherPortal, residency::PortalChunkResidency, tests::save_destination};
use crate::{
    entity::{EntityBase, death_test_world::DeathTestWorld},
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::player::player_changed_world::PlayerChangedWorldEvent,
    },
    server::Server,
    world::{World, guard_effects::GuardEffects},
};

const PORTAL: BlockPos = BlockPos::new(8, 80, 8);
const DEADLINE: Duration = Duration::from_secs(45);
type Arrival = (Arc<World>, Arc<World>);

struct Arrived {
    player_id: i32,
    completed: Mutex<Option<oneshot::Sender<Arrival>>>,
}

impl EventHandler<PlayerChangedWorldEvent> for Arrived {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerChangedWorldEvent,
    ) -> BoxFuture<'a, ()> {
        if event.player.entity_id() == self.player_id {
            let completed = self.completed.lock().unwrap().take();
            if let Some(completed) = completed {
                let _ = completed.send((event.from_world.clone(), event.to_world.clone()));
            }
        }
        Box::pin(async {})
    }
}

fn center(axis: HorizontalAxis) -> Vector3<f64> {
    match axis {
        HorizontalAxis::X => Vector3::new(9.0, 80.0, 8.5),
        HorizontalAxis::Z => Vector3::new(8.5, 80.0, 9.0),
    }
}

async fn prepare_frame(world: &Arc<World>, axis: HorizontalAxis) -> PortalChunkResidency {
    tokio::time::timeout(DEADLINE, save_destination(world, false))
        .await
        .unwrap();
    let mut residency = PortalChunkResidency::new(world.level.clone());
    // Lookup inspects the complete possible 21-block portal footprint.
    residency.add_area(PORTAL, 22);
    tokio::time::timeout(DEADLINE, residency.load())
        .await
        .unwrap()
        .unwrap();
    let effects = GuardEffects::new();
    NetherPortal::build_portal_frame(world, PORTAL, axis, false);
    drop(effects);
    assert!(
        NetherPortal::get_on_axis(world, &PORTAL, axis)
            .unwrap()
            .was_already_valid(),
        "fixture must contain a real complete portal frame"
    );
    residency
}

async fn transfer(
    source_axis: HorizontalAxis,
    destination_axis: HorizontalAxis,
    expected_yaw: f32,
) {
    let fixture = tokio::time::timeout(DEADLINE, DeathTestWorld::new())
        .await
        .unwrap();
    let source = fixture.world();
    let destination = fixture
        .server
        .get_world_from_dimension(&Dimension::THE_NETHER);
    let source_residency = prepare_frame(&source, source_axis).await;
    let destination_residency = prepare_frame(&destination, destination_axis).await;
    source.level_info.rcu(|info| {
        let mut info = (**info).clone();
        info.game_rules.players_nether_portal_default_delay = 1;
        info
    });
    let mut client = TestPlayer::new(&source);
    let player = client.player.clone();
    let entity = player.get_entity();
    entity.set_pos(center(source_axis));
    entity.yaw.store(35.0);
    let velocity = Vector3::new(0.25, -0.1, 0.75);
    entity.velocity.store(velocity);
    let (completed, arrival) = oneshot::channel();
    fixture
        .server
        .plugin_manager
        .register::<PlayerChangedWorldEvent, _>(
            Arc::new(Arrived {
                player_id: player.entity_id(),
                completed: Mutex::new(Some(completed)),
            }),
            EventPriority::Normal,
            true,
        );

    source.block_registry.on_entity_collision(
        &Block::NETHER_PORTAL,
        &source,
        player.as_ref(),
        &PORTAL,
        source.get_block_state(&PORTAL),
        &fixture.server,
    );
    let captured_axis = {
        let manager = entity.portal_manager.lock().unwrap();
        manager
            .as_ref()
            .unwrap()
            .source_portal
            .as_ref()
            .unwrap()
            .axis
    };
    assert_eq!(
        captured_axis, source_axis,
        "registered contact must capture the actual source frame axis"
    );
    // This real Entity tick schedules destination lookup and the player's async transfer.
    entity.tick(player.as_ref(), &fixture.server);
    let (from, to) = tokio::time::timeout(DEADLINE, client.with_outgoing_writer(arrival))
        .await
        .unwrap()
        .unwrap();
    assert!(Arc::ptr_eq(&from, &source));
    assert!(Arc::ptr_eq(&to, &destination));
    assert!(Arc::ptr_eq(&player.world(), &destination));
    assert!(source.get_player_by_id(player.entity_id()).is_none());
    assert!(Arc::ptr_eq(
        &destination.get_player_by_id(player.entity_id()).unwrap(),
        &player,
    ));
    assert_eq!(
        player.position(),
        center(destination_axis),
        "the actual transfer must arrive at the linked destination portal"
    );
    assert_eq!(
        entity.velocity.load(),
        velocity,
        "portal yaw correction must preserve existing velocity semantics"
    );
    assert_eq!(
        entity.yaw.load().rem_euclid(360.0),
        expected_yaw.rem_euclid(360.0),
        "different portal axes must add positive ninety degrees to final player yaw"
    );
    drop((source_residency, destination_residency));
    tokio::time::timeout(DEADLINE, fixture.server.shutdown())
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g16_nether_x_to_z_rotates_positive_ninety() {
    transfer(HorizontalAxis::X, HorizontalAxis::Z, 125.0).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g16_nether_z_to_x_rotates_positive_ninety() {
    transfer(HorizontalAxis::Z, HorizontalAxis::X, 125.0).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g16_nether_same_x_axis_retains_yaw() {
    transfer(HorizontalAxis::X, HorizontalAxis::X, 35.0).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1019_g16_nether_same_z_axis_retains_yaw() {
    transfer(HorizontalAxis::Z, HorizontalAxis::Z, 35.0).await;
}
