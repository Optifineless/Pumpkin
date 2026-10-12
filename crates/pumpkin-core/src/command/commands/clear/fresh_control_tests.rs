#![expect(
    clippy::unwrap_used,
    reason = "Regression fixtures require successful setup"
)]

use super::{Player, tests::ClearFixture};
use crate::entity::EntityBase;
use pumpkin_data::{
    attributes::Attributes, data_component_impl::EquipmentSlot, item::Item, item_stack::ItemStack,
};
use std::sync::atomic::Ordering;

mod combat;

fn dispatch(fixture: &ClearFixture, input: &str) -> i32 {
    let source = fixture.target.player.get_command_source(&fixture.server);
    fixture
        .server
        .command_dispatcher
        .load()
        .execute_input(input, &source)
        .unwrap()
}

fn prime_sword(fixture: &mut ClearFixture) {
    assert!(
        fixture
            .target
            .player
            .world()
            .entity_tracker
            .get_tracked_entity(fixture.target.player.entity_id())
            .unwrap()
            .seen_by
            .contains(&fixture.observer.player.gameprofile.id)
    );
    fixture.equip(&[(
        EquipmentSlot::MAIN_HAND,
        ItemStack::new(1, &Item::DIAMOND_SWORD),
    )]);
    assert!(
        fixture
            .target
            .player
            .living_entity
            .get_attribute_value(&Attributes::ATTACK_DAMAGE)
            > fixture
                .target
                .player
                .living_entity
                .get_attribute_base(&Attributes::ATTACK_DAMAGE)
    );
    assert_ne!(
        fixture
            .target
            .player
            .living_entity
            .get_attribute_value(&Attributes::ATTACK_SPEED),
        fixture
            .target
            .player
            .living_entity
            .get_attribute_base(&Attributes::ATTACK_SPEED)
    );
    assert!(
        fixture
            .take_observer_equipment_updates()
            .iter()
            .any(|packet| {
                packet.entity_id.0 == fixture.target.player.entity_id()
                    && packet.equipment.iter().any(|(slot, stack)| {
                        *slot == EquipmentSlot::MAIN_HAND.discriminant()
                            && stack.0.item == &Item::DIAMOND_SWORD
                    })
            }),
        "real observer never received the setup sword"
    );
    fixture.target.take_packets();
}

async fn close_fixture(fixture: &ClearFixture) {
    for client in [fixture.target.client(), fixture.observer.client()] {
        client.close();
        client.await_tasks().await;
    }
    fixture
        .target
        .player
        .world()
        .level
        .shutdown()
        .await
        .unwrap();
}
