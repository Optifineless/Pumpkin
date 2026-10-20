use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed},
};

use pumpkin_data::{
    Block, BlockStateId,
    block_properties::WhiteBedLikeProperties,
    dimension::Dimension,
    entity::{EntityPose, EntityType},
    item::Item,
    item_stack::ItemStack,
};
use pumpkin_inventory::Inventory;
use pumpkin_protocol::{
    VarInt,
    bedrock::{
        client::move_player::CMovePlayer,
        network_item::NetworkItemDescriptor,
        server::inventory_transaction::{
            HandSlot, SInventoryTransaction, TransactionData, UseItemTransactionData,
        },
    },
    codec::{var_uint::VarUInt, var_ulong::VarULong},
};
use pumpkin_util::{
    PermissionLvl,
    math::{position::BlockPos, vector3::Vector3},
};

use super::position_rules::Scenario;
use super::{Callback, EntityBase, EventPriority, FOOT, HEAD, PlayerBedLeaveEvent};
use crate::{
    net::bedrock::combat_test_support::TestBedrockPlayer,
    plugin::api::events::entity::entity_dismount::EntityDismountEvent,
};

fn occupied(scenario: &Scenario) -> [bool; 2] {
    [FOOT, HEAD].map(|position| {
        WhiteBedLikeProperties::from_state_id(scenario.world.get_block_state_id(&position)).occupied
    })
}

fn retaining_straw() -> Dimension {
    let mut dimension = Dimension::OVERWORLD;
    dimension.straw_bed_rule.destroy_on_leave = false;
    dimension
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_sleep_wake_position_rules_mounted_admission() {
    for item in [&Item::WHITE_BED, &Item::STRAW_BED] {
        let scenario = Scenario::new(retaining_straw(), item);
        scenario.client.player.wake_up();
        scenario
            .client
            .player
            .get_entity()
            .set_pos(Vector3::new(5.5, 64.0, 6.5));
        let boat = crate::entity::r#type::from_type(
            &EntityType::OAK_BOAT,
            Vector3::new(5.5, 64.0, 6.5),
            &scenario.world,
            uuid::Uuid::new_v4(),
        );
        assert!(scenario.world.spawn_entity(boat.clone()));
        boat.get_entity()
            .add_passenger(boat.clone(), scenario.client.player.clone());
        assert!(scenario.client.player.get_entity().get_vehicle().is_some());
        let cancel = Arc::new(AtomicBool::new(true));
        let cancelled = cancel.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        scenario
            .server
            .plugin_manager
            .register::<EntityDismountEvent, _>(
                Arc::new(Callback(move |event: &mut EntityDismountEvent| {
                    seen.fetch_add(1, Relaxed);
                    event.cancelled = cancelled.load(Relaxed);
                })),
                EventPriority::Normal,
                true,
            );
        scenario.use_on(FOOT);
        assert_eq!(
            calls.load(Relaxed),
            1,
            "bed entry attempts the real dismount callback"
        );
        scenario.assert_awake();
        assert_eq!(occupied(&scenario), [false, false]);
        assert!(
            boat.get_entity()
                .has_passenger(scenario.client.player.entity_id())
        );
        assert!(scenario.client.player.get_entity().get_vehicle().is_some());
        cancel.store(false, Relaxed);
        scenario.use_on(FOOT);
        assert_eq!(calls.load(Relaxed), 2);
        assert!(scenario.client.player.get_entity().get_vehicle().is_none());
        assert!(
            !boat
                .get_entity()
                .has_passenger(scenario.client.player.entity_id())
        );
        assert_eq!(scenario.client.player.sleeping_bed_pos.load(), Some(HEAD));
        assert!(scenario.client.player.get_entity().pose.load() == EntityPose::Sleeping);
        assert_eq!(occupied(&scenario), [true, true]);
        scenario.finish().await;
    }
}

fn bedrock_use(client: &TestBedrockPlayer, position: BlockPos) {
    client.client().handle_inventory_action(
        &client.player,
        SInventoryTransaction {
            legacy_request_id: VarInt(0),
            legacy_set_item_slots: Vec::new(),
            has_value: true,
            actions: Vec::new(),
            transaction_type: VarUInt(2),
            transaction_data: TransactionData::UseItem(UseItemTransactionData {
                action_type: VarInt(0),
                trigger_type: 0,
                block_position: position,
                block_face: 1,
                hot_bar_slot: VarInt(i32::from(client.player.inventory().get_selected_slot())),
                hand: HandSlot::Mainhand,
                item_in_hand: NetworkItemDescriptor::from(&client.player.inventory().held_item()),
                player_position: client.player.position().to_f32_lossy(),
                click_position: Vector3::new(0.5, 1.0, 0.5),
                block_runtime_id: VarUInt(0),
                client_prediction: 0,
                client_cooldown_state: 0,
            }),
        },
    );
}

fn assert_bedrock_correction(
    owner: &mut TestBedrockPlayer,
    observer: &mut TestBedrockPlayer,
    expected: Vector3<f64>,
    yaw: f32,
) {
    assert_eq!(owner.player.position(), expected);
    let packet = CMovePlayer::new(
        VarULong(owner.player.entity_id() as u64),
        Vector3::new(
            expected.x as f32,
            expected.y as f32 + EntityType::PLAYER.eye_height,
            expected.z as f32,
        ),
        0.0,
        yaw,
        yaw,
        CMovePlayer::MODE_TELEPORT,
        false,
        VarULong(0),
        0,
        0,
        VarULong(0),
    );
    let expected = owner.client().serialize_packet(&packet).unwrap();
    assert!(
        owner.take_packets().contains(&expected),
        "Bedrock owner receives exact teleport correction"
    );
    assert!(
        observer.take_packets().contains(&expected),
        "Bedrock observer receives exact teleport correction"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_sleep_wake_position_rules_bedrock_corrections() {
    for (item, height) in [(&Item::WHITE_BED, 64.6875), (&Item::STRAW_BED, 64.375)] {
        let scenario = Scenario::new(retaining_straw(), item);
        scenario.client.player.wake_up();
        let mut owner = TestBedrockPlayer::new(&scenario.world).await;
        let mut observer = TestBedrockPlayer::new(&scenario.world).await;
        scenario.world.players.store(Arc::new(vec![
            owner.player.clone(),
            observer.player.clone(),
        ]));
        for client in [&owner, &observer] {
            client
                .player
                .watched_section
                .store(scenario.client.player.watched_section.load());
        }
        owner.player.permission_lvl.store(PermissionLvl::Four);
        owner
            .player
            .get_entity()
            .set_pos(Vector3::new(5.5, 64.0, 6.5));
        owner.player.get_entity().set_rotation(-90.0, 0.0);
        owner.take_packets();
        observer.take_packets();
        bedrock_use(&owner, FOOT);
        assert_eq!(owner.player.sleeping_bed_pos.load(), Some(HEAD));
        assert_bedrock_correction(
            &mut owner,
            &mut observer,
            Vector3::new(6.5, height, 5.5),
            -90.0,
        );
        owner.player.wake_up();
        assert_eq!(owner.player.sleeping_bed_pos.load(), None);
        assert_eq!(owner.player.sleeping_since.load(), None);
        assert_bedrock_correction(
            &mut owner,
            &mut observer,
            Vector3::new(6.5, 64.0, 6.5),
            -180.0,
        );
        owner.close().await;
        observer.close().await;
        scenario.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020d_g01_wake_cancel_coherence_native_sleep_cannot_steal_or_switch() {
    const OTHER_FOOT: BlockPos = BlockPos::new(5, 64, 9);
    const OTHER_HEAD: BlockPos = BlockPos::new(6, 64, 9);
    let scenario = Scenario::new(Dimension::OVERWORLD, &Item::WHITE_BED);
    scenario.client.player.wake_up();
    scenario
        .client
        .player
        .get_entity()
        .set_pos(OTHER_FOOT.add(0, 0, 2).to_f64());
    scenario.client.player.get_entity().set_rotation(-90.0, 0.0);
    scenario
        .client
        .player
        .inventory()
        .set_stack(0, ItemStack::new(1, &Item::WHITE_BED));
    scenario.use_on(OTHER_FOOT.down());
    assert_eq!(scenario.world.get_block(&OTHER_HEAD), &Block::WHITE_BED);
    scenario.client.player.get_entity().set_pos(FOOT.to_f64());
    scenario.use_on(FOOT);
    let timer = scenario.client.player.sleeping_since.load();
    scenario.client.player.sleep(OTHER_HEAD);
    assert_eq!(scenario.client.player.sleeping_bed_pos.load(), Some(HEAD));
    assert_eq!(scenario.client.player.sleeping_since.load(), timer);
    assert_eq!(occupied(&scenario), [true, true]);
    let owner = scenario.client.player.clone();
    let other = scenario.observer.player.clone();
    scenario
        .server
        .plugin_manager
        .register::<PlayerBedLeaveEvent, _>(
            Arc::new(Callback(move |_: &mut PlayerBedLeaveEvent| {
                other.sleep(HEAD);
                owner.sleep(OTHER_HEAD);
            })),
            EventPriority::Normal,
            true,
        );
    scenario.client.player.wake_up();
    scenario.assert_awake();
    assert_eq!(occupied(&scenario), [false, false]);
    assert_eq!(scenario.observer.player.sleeping_bed_pos.load(), None);
    assert_eq!(scenario.observer.player.sleeping_since.load(), None);
    assert!(scenario.observer.player.get_entity().pose.load() == EntityPose::Standing);
    for position in [OTHER_FOOT, OTHER_HEAD] {
        let state: BlockStateId = scenario.world.get_block_state_id(&position);
        assert!(!WhiteBedLikeProperties::from_state_id(state).occupied);
    }
    scenario.finish().await;
}
