use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed},
};

use pumpkin_data::{
    Block, block_properties::WhiteBedLikeProperties, dimension::Dimension, entity::EntityPose,
    item::Item,
};
use pumpkin_inventory::Inventory;
use pumpkin_world::world::BlockFlags;

use super::position_rules::Scenario;
use super::{Callback, EntityBase, EntityPoseChangeEvent, EventPriority, FOOT, HEAD};
use crate::plugin::api::events::player::player_bed::PlayerBedLeaveEvent;

fn occupied(scenario: &Scenario, expected: bool) {
    for position in [FOOT, HEAD] {
        assert_eq!(
            WhiteBedLikeProperties::from_state_id(scenario.world.get_block_state_id(&position))
                .occupied,
            expected,
            "occupancy at {position:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_wake_cancel_coherence() {
    for item in [&Item::WHITE_BED, &Item::STRAW_BED] {
        let mut scenario = Scenario::new(Dimension::OVERWORLD, item);
        for _ in 0..100 {
            scenario.client.player.tick(&scenario.server);
        }
        assert!(scenario.world.should_skip_night());
        let cancel = Arc::new(AtomicBool::new(true));
        let cancelled = cancel.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let player = scenario.client.player.clone();
        scenario
            .server
            .plugin_manager
            .register::<EntityPoseChangeEvent, _>(
                Arc::new(Callback(move |event: &mut EntityPoseChangeEvent| {
                    if event.entity_id == player.entity_id()
                        && event.pose == (EntityPose::Standing as u8).to_string()
                    {
                        seen.fetch_add(1, Relaxed);
                        event.cancelled = cancelled.load(Relaxed);
                    }
                })),
                EventPriority::Normal,
                true,
            );
        let leaves = Arc::new(AtomicUsize::new(0));
        let left = leaves.clone();
        scenario
            .server
            .plugin_manager
            .register::<PlayerBedLeaveEvent, _>(
                Arc::new(Callback(move |event: &mut PlayerBedLeaveEvent| {
                    assert_eq!(event.bed_pos, HEAD);
                    left.fetch_add(1, Relaxed);
                    event.player.wake_up();
                })),
                EventPriority::Normal,
                true,
            );
        let timer = scenario.client.player.sleeping_since.load();
        scenario.drain();
        scenario.client.player.wake_up();
        assert_eq!(calls.load(Relaxed), 1);
        assert_eq!(leaves.load(Relaxed), 1);
        occupied(&scenario, true);
        assert!(scenario.client.player.get_entity().pose.load() == EntityPose::Sleeping);
        assert_eq!(scenario.client.player.sleeping_bed_pos.load(), Some(HEAD));
        assert_eq!(scenario.client.player.sleeping_since.load(), timer);
        assert!(
            scenario.world.should_skip_night(),
            "cancelled wake retains world sleep membership"
        );
        assert!(
            scenario.client.take_packets().is_empty(),
            "cancelled wake does not send corrections"
        );
        cancel.store(false, Relaxed);
        scenario.client.player.wake_up();
        scenario.assert_awake();
        assert_eq!(calls.load(Relaxed), 2);
        assert_eq!(leaves.load(Relaxed), 2);
        if item == &Item::WHITE_BED {
            occupied(&scenario, false);
        } else {
            assert_eq!(scenario.world.get_block(&HEAD), &Block::AIR);
        }
        scenario.client.player.wake_up();
        assert_eq!(leaves.load(Relaxed), 2, "allowed wake commits once");
        scenario.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_wake_cancel_coherence_replaced_bed() {
    let scenario = Scenario::new(Dimension::OVERWORLD, &Item::WHITE_BED);
    let world = scenario.world.clone();
    scenario
        .server
        .plugin_manager
        .register::<PlayerBedLeaveEvent, _>(
            Arc::new(Callback(move |_: &mut PlayerBedLeaveEvent| {
                world.set_block_state(
                    &HEAD,
                    Block::CHEST.default_state.id,
                    BlockFlags::FORCE_STATE,
                );
                let chest = Arc::new(crate::block::entities::chest::ChestBlockEntity::new(HEAD));
                chest.set_stack(
                    0,
                    pumpkin_data::item_stack::ItemStack::new(7, &Item::DIAMOND),
                );
                world.add_block_entity(chest);
            })),
            EventPriority::Normal,
            true,
        );
    let position = scenario.client.player.position();
    scenario.client.player.wake_up();
    scenario.assert_awake();
    assert_eq!(
        scenario.client.player.position(),
        position,
        "replacement is not the remembered bed"
    );
    assert_eq!(scenario.world.get_block(&HEAD), &Block::CHEST);
    let stack = scenario
        .world
        .get_block_entity(&HEAD)
        .unwrap()
        .get_inventory()
        .unwrap()
        .get_stack(0);
    assert_eq!(stack.item.id, Item::DIAMOND.id);
    assert_eq!(stack.item_count, 7);
    scenario.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_wake_cancel_coherence_same_state_bed_replacement() {
    let scenario = Scenario::new(Dimension::OVERWORLD, &Item::WHITE_BED);
    let world = scenario.world.clone();
    let states = [
        world.get_block_state_id(&FOOT),
        world.get_block_state_id(&HEAD),
    ];
    scenario
        .server
        .plugin_manager
        .register::<PlayerBedLeaveEvent, _>(
            Arc::new(Callback(move |_: &mut PlayerBedLeaveEvent| {
                for position in [FOOT, HEAD] {
                    world.remove_block_entity(&position);
                    world.add_block_entity(Arc::new(
                        crate::block::entities::bed::BedBlockEntity::new(position),
                    ));
                }
            })),
            EventPriority::Normal,
            true,
        );
    let position = scenario.client.player.position();
    scenario.client.player.wake_up();
    scenario.assert_awake();
    assert_eq!(
        scenario.client.player.position(),
        position,
        "a replacement has no old stand-up rights"
    );
    assert_eq!(
        [
            scenario.world.get_block_state_id(&FOOT),
            scenario.world.get_block_state_id(&HEAD)
        ],
        states,
        "same-state replacement must not lose its occupied flags"
    );
    scenario.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_wake_cancel_coherence_replaced_player_and_world() {
    for replace_world in [false, true] {
        let scenario = Scenario::new(Dimension::OVERWORLD, &Item::WHITE_BED);
        let other = Scenario::new(Dimension::OVERWORLD, &Item::WHITE_BED);
        let replacement = crate::net::java::combat_test_support::TestPlayer::with_profile(
            &scenario.world,
            scenario.client.player.gameprofile.clone(),
        );
        scenario.world.players.store(Arc::new(vec![
            scenario.client.player.clone(),
            scenario.observer.player.clone(),
        ]));
        let world = scenario.world.clone();
        let destination = other.world.clone();
        let new_player = replacement.player.clone();
        scenario
            .server
            .plugin_manager
            .register::<PlayerBedLeaveEvent, _>(
                Arc::new(Callback(move |event: &mut PlayerBedLeaveEvent| {
                    if replace_world {
                        event.player.get_entity().set_world(destination.clone());
                    } else {
                        world.players.store(Arc::new(vec![new_player.clone()]));
                    }
                })),
                EventPriority::Normal,
                true,
            );
        scenario.client.player.wake_up();
        occupied(&scenario, true);
        assert_eq!(
            scenario.client.player.sleeping_bed_pos.load(),
            Some(HEAD),
            "stale callback continuation must not commit"
        );
        assert_eq!(scenario.client.player.sleeping_since.load(), Some(0));
        assert_eq!(replacement.player.sleeping_bed_pos.load(), None);
        assert_eq!(other.client.player.sleeping_bed_pos.load(), Some(HEAD));
        occupied(&other, true);
        scenario.finish().await;
        other.finish().await;
    }
}

fn raw_bed_half(
    scenario: &Scenario,
    position: pumpkin_util::math::position::BlockPos,
    block: &Block,
    part: pumpkin_data::block_properties::BedPart,
) -> pumpkin_data::BlockStateId {
    use pumpkin_data::block_properties::HorizontalFacing;
    let mut properties = WhiteBedLikeProperties::default(block);
    properties.part = part;
    properties.facing = HorizontalFacing::North;
    properties.occupied = true;
    let state = properties.to_state_id(block);
    scenario.world.set_block_state(
        &position,
        state,
        BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
    );
    scenario
        .world
        .add_block_entity(Arc::new(crate::block::entities::bed::BedBlockEntity::new(
            position,
        )));
    state
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_wake_cancel_coherence_missing_reciprocal_foot() {
    use pumpkin_data::block_properties::BedPart;
    for (item, block) in [
        (&Item::WHITE_BED, &Block::WHITE_BED),
        (&Item::STRAW_BED, &Block::STRAW_BED),
    ] {
        for unrelated in [false, true] {
            let mut scenario = Scenario::new(Dimension::OVERWORLD, item);
            let expected_head = scenario.world.get_block_entity(&HEAD).unwrap();
            let north_head = FOOT.add(0, 0, -1);
            let retained = if unrelated {
                Some((
                    raw_bed_half(&scenario, FOOT, block, BedPart::Foot),
                    raw_bed_half(&scenario, north_head, block, BedPart::Head),
                ))
            } else {
                scenario.world.set_block_state(
                    &FOOT,
                    Block::AIR.default_state.id,
                    BlockFlags::FORCE_STATE | BlockFlags::UPDATE_KNOWN_SHAPE,
                );
                None
            };
            let head =
                WhiteBedLikeProperties::from_state_id(scenario.world.get_block_state_id(&HEAD));
            assert!(
                head.occupied,
                "raw orphan setup must retain the sleeping head"
            );
            scenario.drain();
            scenario.client.player.wake_up();
            scenario.assert_awake();
            assert_eq!(scenario.world.get_block(&HEAD), block);
            assert!(
                !WhiteBedLikeProperties::from_state_id(scenario.world.get_block_state_id(&HEAD))
                    .occupied,
                "a surviving remembered head must release occupancy even without a reciprocal foot"
            );
            assert!(Arc::ptr_eq(
                &scenario.world.get_block_entity(&HEAD).unwrap(),
                &expected_head
            ));
            if let Some((foot, head)) = retained {
                assert_eq!(
                    scenario.world.get_block_state_id(&FOOT),
                    foot,
                    "unrelated wrong-facing foot is not owned by this wake"
                );
                assert_eq!(
                    scenario.world.get_block_state_id(&north_head),
                    head,
                    "unrelated north-facing pair survives"
                );
            } else {
                assert_eq!(scenario.world.get_block(&FOOT), &Block::AIR);
            }
            let position = scenario.client.player.position();
            scenario.drain();
            scenario.client.player.wake_up();
            assert_eq!(scenario.client.player.position(), position);
            assert!(
                scenario.client.take_packets().is_empty(),
                "an awake wake-up is a no-op"
            );
            assert!(scenario.observer.take_packets().is_empty());
            scenario.finish().await;
        }
    }
}
