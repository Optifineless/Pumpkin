#![expect(
    clippy::unwrap_used,
    reason = "Real Bedrock packet regression fixtures"
)]
use super::{
    inventory_exploit_tests::{packet_payload, transaction},
    rejected_prediction_test_support::{
        assert_same, assert_stack, content, helmet, replay_prediction, request, status, take,
    },
};
use crate::{
    entity::death_test_world::DeathTestWorld, net::bedrock::combat_test_support::TestBedrockPlayer,
};
use pumpkin_data::{Enchantment, item::Item, item_stack::ItemStack};
use pumpkin_inventory::{Inventory, screen_handler::ScreenHandler};
use pumpkin_protocol::{
    VarInt,
    bedrock::{
        network_item::{ContainerName, NetworkItemStackDescriptor},
        server::inventory_transaction::{
            LegacySetItemSlot, NormalTransactionData, TransactionData,
        },
    },
};
use pumpkin_util::GameMode;

fn legacy(fixture: &TestBedrockPlayer) {
    let mut packet = transaction(
        Vec::new(),
        TransactionData::Normal(NormalTransactionData {}),
    );
    packet.legacy_request_id = VarInt(1);
    packet.legacy_set_item_slots = vec![LegacySetItemSlot {
        container_id: 120,
        slots: vec![0],
    }];
    fixture
        .client()
        .handle_inventory_action(&fixture.player, packet);
}

async fn legacy_case(mode: GameMode, cursed: bool) {
    let world = DeathTestWorld::new().await;
    let mut fixture = TestBedrockPlayer::new(&world.world()).await;
    fixture.player.gamemode.store(mode);
    let stack = helmet(cursed);
    fixture.player.inventory().set_stack(39, stack.clone());
    crate::entity::player::Player::send_menu_updates(&fixture.player.player_screen_handler);
    fixture.take_packets();
    legacy(&fixture);
    let packets = fixture.take_packets();
    let denied = mode == GameMode::Survival && cursed;
    if denied {
        assert_same(&fixture.player.inventory().get_stack(39), &stack);
        assert!(
            world.world().entities.load().is_empty(),
            "denied armor cannot drop"
        );
        let (name, armor) = content(&packets, 120)
            .expect("unchanged denied armor needs an authoritative correction");
        assert_eq!(name.container_name, ContainerName::Armor);
        assert!(name.dynamic_id.is_none());
        assert_eq!(armor.len(), 4);
        assert_stack(&armor[0], &stack);
        let predicted = replay_prediction(
            &packets,
            120,
            ContainerName::Armor,
            0,
            NetworkItemStackDescriptor::default(),
        );
        assert_stack(&predicted, &stack);
        status(&packets, 1);
    } else {
        status(&packets, 0);
        assert!(fixture.player.inventory().get_stack(39).is_empty());
        let entities = world.world().entities.load_full();
        assert_eq!(entities.len(), 1, "allowed armor must drop once");
        let drop = entities[0]
            .get_item_entity()
            .unwrap()
            .get_item_stack()
            .lock()
            .unwrap()
            .clone();
        assert_same(&drop, &stack);
    }
    fixture.close().await;
    world.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g02_legacy_cursed_armor_rejection_corrects_unchanged_slot() {
    super::rejected_prediction_test_support::bounded(async {
        legacy_case(GameMode::Survival, true).await;
    })
    .await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g02_legacy_uncursed_survival_control() {
    super::rejected_prediction_test_support::bounded(async {
        legacy_case(GameMode::Survival, false).await;
    })
    .await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g02_legacy_cursed_creative_control() {
    super::rejected_prediction_test_support::bounded(async {
        legacy_case(GameMode::Creative, true).await;
    })
    .await;
}

async fn cursor_case(empty: bool, stack_request: bool) {
    let world = DeathTestWorld::new().await;
    let mut fixture = TestBedrockPlayer::new(&world.world()).await;
    let armor = helmet(true);
    fixture.player.inventory().set_stack(39, armor.clone());
    let mut cursor = if empty {
        ItemStack::EMPTY.clone()
    } else {
        ItemStack::new(1, &Item::DIAMOND_SWORD)
    };
    if !empty {
        cursor.add_enchantment(&Enchantment::UNBREAKING, 2);
        cursor.set_damage(17);
    }
    let handler = fixture.player.player_screen_handler.lock().unwrap();
    *handler.get_behaviour().cursor_stack.lock().unwrap() = cursor.clone();
    drop(handler);
    crate::entity::player::Player::send_menu_updates(&fixture.player.player_screen_handler);
    fixture.take_packets();
    if stack_request {
        request(
            &fixture,
            vec![take(ContainerName::Armor, 0, ContainerName::Inventory, 9)],
        );
    } else {
        legacy(&fixture);
    }
    let packets = fixture.take_packets();
    assert_same(&fixture.player.inventory().get_stack(39), &armor);
    status(&packets, 1);
    assert!(world.world().entities.load().is_empty());
    let handler = fixture.player.player_screen_handler.lock().unwrap();
    assert_same(
        &handler.get_behaviour().cursor_stack.lock().unwrap(),
        &cursor,
    );
    drop(handler);
    let (name, stacks) =
        content(&packets, 59).expect("rejected prediction must force the unchanged cursor");
    assert_eq!(name.container_name, ContainerName::Cursor);
    assert!(name.dynamic_id.is_none());
    assert_eq!(stacks.len(), 1);
    assert_stack(&stacks[0], &cursor);
    let predicted = replay_prediction(
        &packets,
        59,
        ContainerName::Cursor,
        0,
        NetworkItemStackDescriptor::default(),
    );
    assert_stack(&predicted, &cursor);
    fixture.close().await;
    world.server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g02_rejected_prediction_forces_unchanged_cursor() {
    super::rejected_prediction_test_support::bounded(async {
        cursor_case(false, true).await;
    })
    .await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g02_legacy_rejection_forces_unchanged_cursor() {
    super::rejected_prediction_test_support::bounded(async {
        cursor_case(false, false).await;
    })
    .await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g02_rejection_corrects_empty_cursor_control() {
    super::rejected_prediction_test_support::bounded(async {
        cursor_case(true, true).await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g02_accepted_unchanged_request_avoids_full_correction() {
    super::rejected_prediction_test_support::bounded(async {
        let world = DeathTestWorld::new().await;
        let mut fixture = TestBedrockPlayer::new(&world.world()).await;
        let mut cursor = ItemStack::new(1, &Item::DIAMOND_SWORD);
        cursor.add_enchantment(&Enchantment::UNBREAKING, 2);
        *fixture
            .player
            .player_screen_handler
            .lock()
            .unwrap()
            .get_behaviour()
            .cursor_stack
            .lock()
            .unwrap() = cursor;

        crate::entity::player::Player::send_menu_updates(&fixture.player.player_screen_handler);
        fixture.take_packets();
        request(&fixture, Vec::new());
        let packets = fixture.take_packets();
        status(&packets, 0);
        assert!(
            !packets
                .iter()
                .any(|packet| packet_payload(packet, 49).is_some()
                    || packet_payload(packet, 50).is_some())
        );
        fixture.close().await;
        world.server.shutdown().await;
    })
    .await;
}
