#![expect(
    clippy::unwrap_used,
    reason = "The regression fixture requires successful setup"
)]

use super::tests::ClearFixture;
use super::*;
use crate::entity::EntityBase;
use crate::net::java::combat_test_support::TestPlayer;
use pumpkin_data::attributes::Attributes;
use pumpkin_data::item::Item;
use pumpkin_data::packet::{
    CURRENT_MC_VERSION,
    clientbound::play::{CONTAINER_SET_SLOT, SET_EQUIPMENT, UPDATE_ATTRIBUTES},
};
use pumpkin_protocol::{
    ServerPacket,
    codec::{item_stack_seralizer::OptionalItemStackHash, var_int::VarInt},
    java::{
        client::play::{CSetContainerSlot, CUpdateAttributes},
        server::play::{SClickSlot, SSetHeldItem},
    },
};
use pumpkin_util::math::vector3::Vector3;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum HookPoint {
    BeforeInventoryScan,
    BeforeEquipmentUpdates,
}

type ClearHook = (HookPoint, Box<dyn FnOnce(&Player)>);
thread_local! {
    static CLEAR_HOOK: std::cell::RefCell<Option<ClearHook>> = const { std::cell::RefCell::new(None) };
}

pub(super) fn run_hook(point: HookPoint, target: &Player) {
    let hook = CLEAR_HOOK.with(|hook| {
        let mut hook = hook.borrow_mut();
        if hook.as_ref().is_some_and(|(stage, _)| *stage == point) {
            hook.take().map(|(_, hook)| hook)
        } else {
            None
        }
    });
    if let Some(hook) = hook {
        hook(target);
    }
}

fn set_hook(point: HookPoint, hook: impl FnOnce(&Player) + 'static) {
    CLEAR_HOOK.with(|slot| *slot.borrow_mut() = Some((point, Box::new(hook))));
}

fn replacement_attribute(fixture: &ClearFixture, command: &str, attribute: &Attributes) -> f64 {
    let control = &fixture.observer.player;
    control
        .permission_lvl
        .store(pumpkin_util::PermissionLvl::Four);
    let source = control.get_command_source(&fixture.server);
    assert_eq!(
        fixture
            .server
            .command_dispatcher
            .load()
            .execute_input(command, &source)
            .unwrap(),
        1
    );
    control.tick(&fixture.server);
    control.living_entity.get_attribute_value(attribute)
}

#[tokio::test]
async fn clear_detects_selected_slot_switched_during_command() {
    let mut fixture = ClearFixture::new();
    fixture
        .target
        .player
        .inventory
        .set_slot(1, ItemStack::new(1, &Item::DIAMOND_SWORD));
    let server = fixture.server.clone();
    let player = fixture.target.player.clone();
    let client = fixture.target.player.client.clone();
    set_hook(HookPoint::BeforeInventoryScan, move |target| {
        assert!(target.inventory.main_inventory.try_write().is_ok());
        if let crate::net::ClientPlatform::Java(client) = client.as_ref() {
            client.handle_set_held_item(&server, &player, &SSetHeldItem { slot: 1 });
        }
        assert_eq!(player.inventory.get_selected_slot(), 1);
    });

    assert_eq!(
        fixture
            .run_clear("clear @s minecraft:diamond_sword")
            .unwrap(),
        1
    );
    assert!(fixture.target.player.inventory.held_item().is_empty());
    assert_eq!(
        fixture
            .target
            .player
            .living_entity
            .get_attribute_value(&Attributes::ATTACK_DAMAGE),
        fixture
            .target
            .player
            .living_entity
            .get_attribute_base(&Attributes::ATTACK_DAMAGE)
    );
    let updates = fixture.take_observer_equipment_updates();
    let last_hand = updates
        .iter()
        .filter(|update| update.entity_id.0 == fixture.target.player.entity_id())
        .flat_map(|update| &update.equipment)
        .rfind(|(slot, _)| *slot == EquipmentSlot::MAIN_HAND.discriminant());
    assert!(last_hand.is_some_and(|(_, stack)| stack.0.is_empty()));
}

#[tokio::test]
async fn clear_detects_selected_slot_switched_after_scan() {
    let mut fixture = ClearFixture::new();
    fixture.equip(&[(
        EquipmentSlot::MAIN_HAND,
        ItemStack::new(1, &Item::DIAMOND_SWORD),
    )]);
    fixture
        .target
        .player
        .inventory
        .set_slot(1, ItemStack::new(1, &Item::IRON_SWORD));
    let server = fixture.server.clone();
    let player = fixture.target.player.clone();
    let client = player.client.clone();
    let expected = replacement_attribute(
        &fixture,
        "item replace entity @s weapon.mainhand with minecraft:iron_sword",
        &Attributes::ATTACK_DAMAGE,
    );
    set_hook(HookPoint::BeforeEquipmentUpdates, move |_| {
        if let crate::net::ClientPlatform::Java(client) = client.as_ref() {
            client.handle_set_held_item(&server, &player, &SSetHeldItem { slot: 1 });
        }
        assert_eq!(player.inventory.get_selected_slot(), 1);
    });

    assert_eq!(
        fixture
            .run_clear("clear @s minecraft:diamond_sword")
            .unwrap(),
        1
    );
    assert_eq!(
        fixture.target.player.inventory.held_item().item,
        &Item::IRON_SWORD
    );
    assert_eq!(
        fixture
            .target
            .player
            .living_entity
            .get_attribute_value(&Attributes::ATTACK_DAMAGE),
        expected
    );
    let updates = fixture.take_observer_equipment_updates();
    let last_hand = updates
        .iter()
        .filter(|update| update.entity_id.0 == fixture.target.player.entity_id())
        .flat_map(|update| &update.equipment)
        .rfind(|(slot, _)| *slot == EquipmentSlot::MAIN_HAND.discriminant());
    assert!(last_hand.is_some_and(|(_, stack)| stack.0.item == &Item::IRON_SWORD));
}

#[tokio::test]
async fn clear_detects_armor_replaced_before_publication() {
    let mut fixture = ClearFixture::new();
    fixture.equip(&[(
        EquipmentSlot::CHEST,
        ItemStack::new(1, &Item::DIAMOND_CHESTPLATE),
    )]);
    fixture.target.take_packets();
    fixture.observer.take_packets();
    let server = fixture.server.clone();
    let player = fixture.target.player.clone();
    let expected = replacement_attribute(
        &fixture,
        "item replace entity @s armor.chest with minecraft:iron_chestplate",
        &Attributes::ARMOR,
    );
    set_hook(HookPoint::BeforeEquipmentUpdates, move |target| {
        // The competing command can enter only after both storage guards have ended.
        assert!(target.inventory.main_inventory.try_write().is_ok());
        assert!(target.inventory.entity_equipment.try_lock().is_ok());
        let source = player.get_command_source(&server);
        assert_eq!(
            server
                .command_dispatcher
                .load()
                .execute_input(
                    "item replace entity @s armor.chest with minecraft:iron_chestplate",
                    &source
                )
                .unwrap(),
            1
        );
    });

    assert_eq!(
        fixture
            .run_clear("clear @s minecraft:diamond_chestplate")
            .unwrap(),
        1
    );
    assert_eq!(
        fixture.target.player.inventory.get_slot(38).item,
        &Item::IRON_CHESTPLATE
    );
    assert!(
        expected
            > fixture
                .target
                .player
                .living_entity
                .get_attribute_base(&Attributes::ARMOR)
    );
    assert_eq!(
        fixture
            .target
            .player
            .living_entity
            .get_attribute_value(&Attributes::ARMOR),
        expected
    );
    let updates = fixture.take_observer_equipment_updates();
    let chest_updates: Vec<_> = updates
        .iter()
        .filter(|update| update.entity_id.0 == fixture.target.player.entity_id())
        .flat_map(|update| &update.equipment)
        .filter(|(slot, _)| *slot == EquipmentSlot::CHEST.discriminant())
        .collect();
    assert!(!chest_updates.is_empty());
    assert!(
        chest_updates
            .iter()
            .all(|(_, stack)| stack.0.item == &Item::IRON_CHESTPLATE)
    );
}

#[tokio::test]
async fn clear_rechecks_armor_replaced_during_tick_publication() {
    let mut fixture = ClearFixture::new();
    fixture.equip(&[(
        EquipmentSlot::CHEST,
        ItemStack::new(1, &Item::DIAMOND_CHESTPLATE),
    )]);
    fixture.target.take_packets();
    fixture.observer.take_packets();
    let server = fixture.server.clone();
    let player = fixture.target.player.clone();
    let expected = Arc::new(crossbeam::atomic::AtomicCell::new(None));
    let expected_for_hook = expected.clone();
    Player::set_equipment_update_test_hook(move |target| {
        assert!(target.inventory.main_inventory.try_write().is_ok());
        assert!(target.inventory.entity_equipment.try_lock().is_ok());
        let source = player.get_command_source(&server);
        assert_eq!(
            server
                .command_dispatcher
                .load()
                .execute_input(
                    "item replace entity @s armor.chest with minecraft:iron_chestplate",
                    &source
                )
                .unwrap(),
            1
        );
        // The competing publisher must queue its update without applying over this batch.
        expected_for_hook.store(Some(target.inventory.get_slot(38).item.id));
    });

    assert_eq!(
        fixture
            .run_clear("clear @s minecraft:diamond_chestplate")
            .unwrap(),
        1
    );
    assert_eq!(expected.load(), Some(Item::IRON_CHESTPLATE.id));
    assert_eq!(
        fixture.target.player.inventory.get_slot(38).item,
        &Item::IRON_CHESTPLATE
    );
    let armor = fixture
        .target
        .player
        .living_entity
        .get_attribute_value(&Attributes::ARMOR);
    assert!(
        armor
            > fixture
                .target
                .player
                .living_entity
                .get_attribute_base(&Attributes::ARMOR)
    );
    let updates = fixture.take_observer_equipment_updates();
    let last_chest = updates
        .iter()
        .filter(|update| update.entity_id.0 == fixture.target.player.entity_id())
        .flat_map(|update| &update.equipment)
        .rfind(|(slot, _)| *slot == EquipmentSlot::CHEST.discriminant());
    assert!(last_chest.is_some_and(|(_, stack)| stack.0.item == &Item::IRON_CHESTPLATE));
}

fn packet_id(packet: &bytes::Bytes) -> i32 {
    VarInt::decode(&mut packet.as_ref()).unwrap().0
}

#[tokio::test]
async fn equipment_detection_waits_for_shift_click_to_finish_writing() {
    let fixture = ClearFixture::new();
    fixture.equip(&[(
        EquipmentSlot::CHEST,
        ItemStack::new(1, &Item::DIAMOND_CHESTPLATE),
    )]);
    fixture.target.player.on_slot_click(
        SClickSlot {
            sync_id: VarInt(0),
            revision: VarInt(0),
            slot: 6,
            button: 0,
            mode: pumpkin_protocol::java::server::play::SlotActionType::QuickMove,
            length_of_array: VarInt(0),
            array_of_changed_slots: Vec::new(),
            carried_item: OptionalItemStackHash(None),
        },
        &fixture.server,
    );
    fixture.target.player.tick(&fixture.server);
    assert!(fixture.target.player.inventory.get_slot(38).is_empty());
    assert_eq!(
        fixture
            .target
            .player
            .living_entity
            .get_attribute_value(&Attributes::ARMOR),
        fixture
            .target
            .player
            .living_entity
            .get_attribute_base(&Attributes::ARMOR)
    );
}

fn assert_attribute_reset(packets: &[bytes::Bytes], target: &Player, attribute: &Attributes) {
    assert!(
        packets
            .iter()
            .filter(|packet| packet_id(packet) == UPDATE_ATTRIBUTES.0)
            .map(|packet| {
                let mut data = packet.as_ref();
                VarInt::decode(&mut data).unwrap();
                CUpdateAttributes::read(&mut data, &CURRENT_MC_VERSION).unwrap()
            })
            .any(|update| {
                update.entity_id.0 == target.entity_id()
                    && update.properties.iter().any(|property| {
                        property.id.0 == i32::from(attribute.id)
                            && property.value == target.living_entity.get_attribute_base(attribute)
                            && property.modifiers.is_empty()
                    })
            })
    );
}

#[tokio::test]
async fn clear_updates_owner_and_trackers_only() {
    let mut fixture = ClearFixture::new();
    fixture.open_inventory();
    let world = fixture.target.player.world();
    let mut non_tracker = TestPlayer::new(&world);
    non_tracker
        .player
        .get_entity()
        .set_pos(Vector3::new(4096.0, 64.0, 4096.0));
    world.players.store(Arc::new(vec![
        fixture.target.player.clone(),
        fixture.observer.player.clone(),
        non_tracker.player.clone(),
    ]));
    let tracked = world
        .entity_tracker
        .get_tracked_entity(fixture.target.player.entity_id())
        .unwrap();
    non_tracker
        .player
        .chunk_sender
        .lock()
        .unwrap()
        .mark_sent_out_of_band(fixture.target.player.get_entity().chunk_pos.load());
    tracked.update_player(&non_tracker.player, &world);
    assert!(
        tracked
            .seen_by
            .contains(&fixture.observer.player.gameprofile.id)
    );
    assert!(!tracked.seen_by.contains(&non_tracker.player.gameprofile.id));
    fixture.equip_diamond_armor();
    fixture.equip(&[(
        EquipmentSlot::MAIN_HAND,
        ItemStack::new(1, &Item::DIAMOND_SWORD),
    )]);
    // Attack damage exercises the editioned path; armor exercises the Java-only path.
    for (command, count, attribute, inventory_slots) in [
        (
            "clear @s minecraft:diamond_sword",
            1,
            &Attributes::ATTACK_DAMAGE,
            vec![36],
        ),
        ("clear @s", 4, &Attributes::ARMOR, vec![5, 6, 7, 8]),
    ] {
        fixture.target.take_packets();
        fixture.observer.take_packets();
        non_tracker.take_packets();
        assert_eq!(fixture.run_clear(command).unwrap(), count);
        let owner_packets = fixture.target.take_packets();
        let observer_packets = fixture.observer.take_packets();
        assert_attribute_reset(&owner_packets, &fixture.target.player, attribute);
        assert_attribute_reset(&observer_packets, &fixture.target.player, attribute);
        let owner_slots: Vec<_> = owner_packets
            .iter()
            .filter(|packet| packet_id(packet) == CONTAINER_SET_SLOT.0)
            .map(|packet| {
                let mut data = packet.as_ref();
                VarInt::decode(&mut data).unwrap();
                CSetContainerSlot::read(&mut data, &CURRENT_MC_VERSION).unwrap()
            })
            .collect();
        for slot in inventory_slots {
            assert!(owner_slots.iter().any(|update| update.window_id == 0
                && update.slot == slot
                && update.slot_data.0.is_empty()));
        }
        assert!(
            observer_packets
                .iter()
                .any(|packet| packet_id(packet) == SET_EQUIPMENT.0)
        );
        assert!(
            non_tracker
                .take_packets()
                .iter()
                .all(|packet| ![SET_EQUIPMENT.0, UPDATE_ATTRIBUTES.0].contains(&packet_id(packet)))
        );
    }
}
