#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Real-entry beacon regression fixtures require successful setup"
)]

use std::sync::{Arc, atomic::Ordering};

use pumpkin_data::{
    Block, data_component_impl::ContainerImpl, effect::StatusEffect, item::Item,
    item_stack::ItemStack,
};
use pumpkin_inventory::{
    Inventory, beacon_screen_handler::BeaconScreenHandler, screen_handler::SharedScreenHandler,
};
use pumpkin_protocol::{
    VarInt,
    bedrock::{
        network_item::{ContainerName, FullContainerName, NetworkItemDescriptor},
        server::{
            inventory_transaction::{
                HandSlot, SInventoryTransaction, TransactionData, UseItemTransactionData,
            },
            item_stack_request::{
                ItemStackRequest, ItemStackRequestAction as Action, ItemStackRequestSlotInfo,
                SItemStackRequest,
            },
        },
    },
    codec::var_uint::VarUInt,
    java::server::play::SSetBeacon,
    serial::PacketRead,
};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::world::BlockFlags;

use crate::{
    block::entities::{BlockEntity, beacon::BeaconBlockEntity},
    entity::EntityBase,
    net::{
        bedrock::combat_test_support::TestBedrockPlayer,
        java::{combat_test_support::TestPlayer, sound_test_support::decode_sounds},
    },
    server::{Server, combat_test_support},
    world::World,
};

use super::inventory_exploit_tests::packet_payload;

#[path = "inventory_request_beacon_test_support.rs"]
mod support;
use support::{Fixture, assert_payment_response, dispatch, request, selection};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g06_bedrock_beacon_payment_selects_and_persists() {
    for (levels, primary) in [
        (1, &StatusEffect::SPEED),
        (2, &StatusEffect::RESISTANCE),
        (3, &StatusEffect::STRENGTH),
        (4, &StatusEffect::HASTE),
    ] {
        let mut fixture = Fixture::new(levels).await;
        let secondary = if levels == 4 {
            StatusEffect::REGENERATION.to_bedrock_id()
        } else {
            0
        };
        // Primary implementation sources document zero absence and Destroy/27.
        for _ in 0..2 {
            fixture.pay();
            fixture.sounds();
            let actions = fixture.selection_actions(primary.to_bedrock_id(), secondary);
            let (packets, status) = dispatch(&mut fixture.owner, actions);
            assert!(
                fixture.payment().get_stack(0).is_empty(),
                "paid selection must consume exactly one payment"
            );
            assert_eq!(status, 0);
            assert_eq!(fixture.owner.player.inventory.get_stack(9).item_count, 1);
            assert_eq!(
                fixture.effects(),
                (
                    i32::from(primary.id),
                    if levels == 4 {
                        i32::from(StatusEffect::REGENERATION.id)
                    } else {
                        -1
                    }
                )
            );
            assert_eq!(
                fixture.sounds(),
                1,
                "each paid selection, including reselection, sounds once"
            );
            assert_payment_response(&packets);
        }
        let saved = fixture
            .world
            .level
            .read_chunk_sync(&fixture.beacon.position.chunk_position(), |chunk| {
                chunk
                    .pending_block_entities
                    .lock()
                    .unwrap()
                    .get(&fixture.beacon.position)
                    .cloned()
            })
            .flatten()
            .expect("committed saved image");
        assert_eq!(
            saved.get_string("primary_effect"),
            Some(primary.minecraft_name)
        );
        // Compare the normal Java entry with the same concrete menu and payment.
        let shared_menu = fixture.screen();
        fixture
            .observer
            .player
            .current_screen_handler
            .lock()
            .unwrap()
            .clone_from(&shared_menu);
        fixture
            .payment()
            .set_stack(0, ItemStack::new(1, &Item::EMERALD));
        fixture.observer.client().handle_set_beacon(
            &fixture.observer.player,
            &SSetBeacon {
                primary_effect: Some(VarInt(i32::from(primary.id))),
                secondary_effect: (levels == 4)
                    .then_some(VarInt(i32::from(StatusEffect::REGENERATION.id))),
            },
        );
        assert!(
            fixture.payment().get_stack(0).is_empty(),
            "Java paid-selection control"
        );
        assert_eq!(fixture.effects().0, i32::from(primary.id));
        fixture.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g06_beacon_request_rejects_without_partial_debit() {
    let mut fixture = Fixture::new(1).await;
    request(
        &mut fixture.owner,
        vec![selection(StatusEffect::SPEED.to_bedrock_id(), 0)],
        1,
    );
    fixture.pay();
    for actions in [
        vec![selection(StatusEffect::STRENGTH.to_bedrock_id(), 0)],
        vec![selection(999, 0)],
        vec![Action::Grindstone {
            recipe_id: VarUInt(1),
            repair_cost: VarInt(0),
            repetitions: 1,
        }],
        vec![selection(
            StatusEffect::SPEED.to_bedrock_id(),
            StatusEffect::REGENERATION.to_bedrock_id(),
        )],
        vec![
            selection(StatusEffect::SPEED.to_bedrock_id(), 0),
            Action::CraftNonImplemented,
        ],
        vec![
            selection(StatusEffect::SPEED.to_bedrock_id(), 0),
            selection(StatusEffect::SPEED.to_bedrock_id(), 0),
        ],
    ] {
        let before = fixture.payment().get_stack(0);
        fixture.sounds();
        let (_, result) = dispatch(&mut fixture.owner, actions);
        assert!(
            fixture.payment().get_stack(0).are_equal(&before),
            "rejected beacon sequence debited its payment"
        );
        assert_eq!(fixture.payment().get_stack(0).uid, before.uid);
        assert_eq!(fixture.effects(), (-1, -1));
        assert_eq!(fixture.sounds(), 0);
        assert_eq!(result, 1, "invalid beacon sequence must report failure");
    }
    reject_predicted_destroy(&mut fixture);
    request(
        &mut fixture.owner,
        vec![selection(StatusEffect::SPEED.to_bedrock_id(), 0)],
        0,
    );
    assert!(
        fixture.payment().get_stack(0).is_empty(),
        "valid one-payment positive control"
    );
    fixture.pay();
    reject_callback_replacement(&mut fixture);
    reject_panicking_validity(&mut fixture);
    fixture.pay();
    reject_replaced_beacon(&mut fixture);
    fixture.beacon.levels.store(0, Ordering::Relaxed);
    request(
        &mut fixture.owner,
        vec![selection(StatusEffect::SPEED.to_bedrock_id(), 0)],
        1,
    );
    assert_eq!(fixture.payment().get_stack(0).item_count, 1);
    let original = fixture.screen();
    *fixture.owner.player.current_screen_handler.lock().unwrap() =
        fixture.owner.player.player_screen_handler.clone();
    request(
        &mut fixture.owner,
        vec![selection(StatusEffect::SPEED.to_bedrock_id(), 0)],
        1,
    );
    assert_eq!(
        original.lock().unwrap().get_behaviour().slots[0]
            .get_cloned_stack()
            .item_count,
        1
    );
    *fixture.owner.player.current_screen_handler.lock().unwrap() = original;
    fixture.world.remove_block_entity(&fixture.beacon.position);
    request(
        &mut fixture.owner,
        vec![selection(StatusEffect::SPEED.to_bedrock_id(), 0)],
        1,
    );
    assert_eq!(fixture.payment().get_stack(0).item_count, 1);
    fixture.finish().await;
}

fn reject_predicted_destroy(fixture: &mut Fixture) {
    for invalid in 0..7 {
        let mut actions = fixture.selection_actions(StatusEffect::SPEED.to_bedrock_id(), 0);
        if let Action::Destroy { count, source } = &mut actions[1] {
            match invalid {
                0 => *count = 2,
                1 => source.stack_id += 1,
                2 => source.slot_id = 0,
                3 => source.container_name.container_name = ContainerName::Inventory,
                4 => source.container_name.dynamic_id = Some(1),
                5 => *count = 0,
                _ => {}
            }
        }
        if invalid == 6 {
            actions.extend(
                fixture
                    .selection_actions(StatusEffect::SPEED.to_bedrock_id(), 0)
                    .into_iter()
                    .skip(1),
            );
        }
        let before = fixture.payment().get_stack(0);
        request(&mut fixture.owner, actions, 1);
        assert!(fixture.payment().get_stack(0).are_equal(&before));
        assert_eq!(fixture.payment().get_stack(0).uid, before.uid);
        assert_eq!(fixture.effects(), (-1, -1));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020c_g06_beacon_disconnect_retains_payment_and_effects() {
    let mut fixture = Fixture::new(1).await;
    fixture.pay();
    let before = fixture.payment().get_stack(0);
    let actions = fixture.selection_actions(StatusEffect::SPEED.to_bedrock_id(), 0);
    fixture.sounds();
    fixture.owner.close().await;
    fixture.owner.client().handle_item_stack_request(
        &fixture.owner.player,
        SItemStackRequest {
            requests: vec![ItemStackRequest {
                request_id: VarInt(-1),
                actions,
                filter_strings: Vec::new(),
                filter_cause: 0,
            }],
        },
    );
    let after = fixture.payment().get_stack(0);
    assert!(after.are_equal(&before));
    assert_eq!(after.uid, before.uid);
    assert_eq!(fixture.effects(), (-1, -1));
    assert_eq!(fixture.sounds(), 0);
    fixture.finish().await;
}

fn reject_callback_replacement(fixture: &mut Fixture) {
    let original = fixture.screen();
    let player = Arc::downgrade(&fixture.owner.player);
    original
        .lock()
        .unwrap()
        .as_any_mut()
        .downcast_mut::<BeaconScreenHandler>()
        .unwrap()
        .validity_check = Some(Box::new(move |_| {
        let player = player.upgrade().unwrap();
        assert!(
            player
                .current_screen_handler
                .lock()
                .unwrap()
                .try_lock()
                .is_ok(),
            "validity callback retained menu guard"
        );
        *player.current_screen_handler.lock().unwrap() = player.player_screen_handler.clone();
        true
    }));
    request(
        &mut fixture.owner,
        vec![selection(StatusEffect::SPEED.to_bedrock_id(), 0)],
        1,
    );
    assert_eq!(
        original.lock().unwrap().get_behaviour().slots[0]
            .get_cloned_stack()
            .item_count,
        1,
        "callback-replaced menu must retain payment"
    );
    original
        .lock()
        .unwrap()
        .as_any_mut()
        .downcast_mut::<BeaconScreenHandler>()
        .unwrap()
        .validity_check = None;
    *fixture.owner.player.current_screen_handler.lock().unwrap() = original;
}

fn reject_panicking_validity(fixture: &mut Fixture) {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let callback_calls = calls.clone();
    let screen = fixture.screen();
    screen
        .lock()
        .unwrap()
        .as_any_mut()
        .downcast_mut::<BeaconScreenHandler>()
        .unwrap()
        .validity_check = Some(Box::new(move |_| {
        assert_ne!(
            callback_calls.fetch_add(1, Ordering::SeqCst),
            0,
            "bounded validity callback panic"
        );
        true
    }));
    let before = fixture.payment().get_stack(0);
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        request(
            &mut fixture.owner,
            vec![selection(StatusEffect::SPEED.to_bedrock_id(), 0)],
            0,
        );
    }));
    assert!(failed.is_err());
    assert!(fixture.payment().get_stack(0).are_equal(&before));
    request(
        &mut fixture.owner,
        vec![selection(StatusEffect::SPEED.to_bedrock_id(), 0)],
        0,
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "original validity check must be restored after panic"
    );
    assert!(fixture.payment().get_stack(0).is_empty());
    screen
        .lock()
        .unwrap()
        .as_any_mut()
        .downcast_mut::<BeaconScreenHandler>()
        .unwrap()
        .validity_check = None;
}

fn reject_replaced_beacon(fixture: &mut Fixture) {
    let world = Arc::downgrade(&fixture.world);
    let position = fixture.beacon.position;
    let screen = fixture.screen();
    screen
        .lock()
        .unwrap()
        .as_any_mut()
        .downcast_mut::<BeaconScreenHandler>()
        .unwrap()
        .validity_check = Some(Box::new(move |_| {
        let replacement = Arc::new(BeaconBlockEntity::new(position));
        replacement.levels.store(1, Ordering::Relaxed);
        world.upgrade().unwrap().add_block_entity(replacement);
        true
    }));
    let before = fixture.payment().get_stack(0);
    let effects = fixture.effects();
    fixture.sounds();
    request(
        &mut fixture.owner,
        vec![selection(StatusEffect::SPEED.to_bedrock_id(), 0)],
        1,
    );
    assert!(fixture.payment().get_stack(0).are_equal(&before));
    assert_eq!(fixture.payment().get_stack(0).uid, before.uid);
    assert_eq!(fixture.effects(), effects);
    assert_eq!(fixture.sounds(), 0);
    screen
        .lock()
        .unwrap()
        .as_any_mut()
        .downcast_mut::<BeaconScreenHandler>()
        .unwrap()
        .validity_check = None;
    fixture.world.add_block_entity(fixture.beacon.clone());
}
