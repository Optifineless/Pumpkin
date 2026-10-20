use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::Relaxed},
};

use pumpkin_data::{
    Block, block_properties::WhiteBedLikeProperties, dimension::Dimension, item::Item,
};
use pumpkin_util::math::vector3::Vector3;

use super::{
    Callback, EntityBase, EventPriority, FOOT, HEAD, PlayerBedLeaveEvent, position_rules::Scenario,
};
use crate::plugin::api::events::block::block_break::BlockBreakEvent;

const DESTINATION: Vector3<f64> = Vector3::new(12.25, 65.0, 12.75);
const YAW: f32 = 27.0;
const PITCH: f32 = 19.0;

fn assert_teleport(scenario: &Scenario, position: Vector3<f64>, yaw: f32, pitch: f32) {
    scenario.assert_awake();
    assert_eq!(
        scenario.client.player.position(),
        position,
        "wake movement must yield to the accepted plugin teleport"
    );
    assert_eq!(scenario.client.player.get_entity().yaw.load(), yaw);
    assert_eq!(scenario.client.player.get_entity().pitch.load(), pitch);
    assert_eq!(
        scenario
            .client
            .player
            .awaiting_teleport
            .lock()
            .unwrap()
            .unwrap()
            .1,
        position,
        "owner correction must retain the accepted destination"
    );
}

async fn leave_teleport(unchanged: bool) {
    let scenario = Scenario::new(Dimension::OVERWORLD, &Item::WHITE_BED);
    let (position, yaw, pitch) = if unchanged {
        (
            scenario.client.player.position(),
            scenario.client.player.get_entity().yaw.load(),
            scenario.client.player.get_entity().pitch.load(),
        )
    } else {
        (DESTINATION, YAW, PITCH)
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    scenario
        .server
        .plugin_manager
        .register::<PlayerBedLeaveEvent, _>(
            Arc::new(Callback(move |event: &mut PlayerBedLeaveEvent| {
                assert_eq!(event.bed_pos, HEAD);
                seen.fetch_add(1, Relaxed);
                assert_eq!(
                    event.player.request_teleport(position, yaw, pitch),
                    Some(position)
                );
            })),
            EventPriority::Normal,
            true,
        );
    scenario.client.player.wake_up();
    assert_eq!(calls.load(Relaxed), 1);
    assert_teleport(&scenario, position, yaw, pitch);
    for position in [FOOT, HEAD] {
        assert_eq!(scenario.world.get_block(&position), &Block::WHITE_BED);
        assert!(
            !WhiteBedLikeProperties::from_state_id(scenario.world.get_block_state_id(&position))
                .occupied
        );
    }
    scenario.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_review_leave_same_world_teleport() {
    leave_teleport(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_review_leave_unchanged_teleport() {
    leave_teleport(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_review_straw_break_same_world_teleport() {
    let mut dimension = Dimension::OVERWORLD;
    dimension.straw_bed_rule.destroy_on_leave = true;
    let scenario = Scenario::new(dimension, &Item::STRAW_BED);
    let player = Arc::downgrade(&scenario.client.player);
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    scenario
        .server
        .plugin_manager
        .register::<BlockBreakEvent, _>(
            Arc::new(Callback(move |event: &mut BlockBreakEvent| {
                if event.block != &Block::STRAW_BED || event.block_position != HEAD {
                    return;
                }
                seen.fetch_add(1, Relaxed);
                let player = player.upgrade().unwrap();
                assert_eq!(player.sleeping_bed_pos.load(), None);
                assert_eq!(player.sleeping_since.load(), None);
                assert_eq!(
                    player.request_teleport(DESTINATION, YAW, PITCH),
                    Some(DESTINATION)
                );
            })),
            EventPriority::Normal,
            true,
        );
    scenario.client.player.wake_up();
    assert_eq!(calls.load(Relaxed), 1);
    assert_teleport(&scenario, DESTINATION, YAW, PITCH);
    assert_eq!(scenario.world.get_block(&HEAD), &Block::AIR);
    assert_eq!(scenario.world.get_block(&FOOT), &Block::AIR);
    scenario.finish().await;
}
