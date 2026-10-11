#![expect(
    clippy::unwrap_used,
    reason = "The regression fixture requires successful setup"
)]

use super::*;
use crate::entity::EntityBase;
use crate::net::bedrock::combat_test_support::TestBedrockPlayer;
use crate::net::java::combat_test_support::TestPlayer;
use crate::server::{Server, combat_test_support};
use pumpkin_data::attributes::Attributes;
use pumpkin_data::data_component_impl::{AttributeModifiersImpl, Operation};
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::item::Item;
use pumpkin_data::packet::{
    CURRENT_MC_VERSION,
    clientbound::play::{CONTAINER_SET_SLOT, SET_EQUIPMENT, UPDATE_ATTRIBUTES},
};
use pumpkin_data::potion::Effect;
use pumpkin_inventory::screen_handler::InventoryPlayer;
use pumpkin_protocol::{
    ServerPacket,
    bedrock::{client::CMobArmorEquipment, network_item::NetworkItemStackDescriptor},
    codec::var_int::VarInt,
    java::client::play::{CSetContainerSlot, CSetEquipment},
};
use pumpkin_util::PermissionLvl;
use std::borrow::Cow;
use std::sync::Arc;

pub(super) struct ClearFixture {
    _directory: tempfile::TempDir,
    pub(super) server: Arc<Server>,
    pub(super) target: TestPlayer,
    pub(super) observer: TestPlayer,
}

impl ClearFixture {
    pub(super) fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = combat_test_support::server(directory.path());
        let world = combat_test_support::world(&server, directory.path());
        let target = TestPlayer::new(&world);
        let observer = TestPlayer::new(&world);
        target.player.permission_lvl.store(PermissionLvl::Four);
        server.worlds.store(Arc::new(vec![world.clone()]));
        world.players.store(Arc::new(vec![
            target.player.clone(),
            observer.player.clone(),
        ]));
        observer
            .player
            .chunk_sender
            .lock()
            .unwrap()
            .mark_sent_out_of_band(target.player.get_entity().chunk_pos.load());
        world
            .entity_tracker
            .add_entity(&(target.player.clone() as Arc<dyn EntityBase>), &world);
        Self {
            _directory: directory,
            server,
            target,
            observer,
        }
    }

    pub(super) fn open_inventory(&self) {
        self.target
            .player
            .screen_handler_sync_handler
            .store_player(self.target.player.clone());
        self.target
            .player
            .on_screen_handler_opened(&self.target.player.player_screen_handler);
    }

    pub(super) fn equip_diamond_armor(&self) {
        let equipment = [
            (
                EquipmentSlot::HEAD,
                ItemStack::new(1, &Item::DIAMOND_HELMET),
            ),
            (
                EquipmentSlot::CHEST,
                ItemStack::new(1, &Item::DIAMOND_CHESTPLATE),
            ),
            (
                EquipmentSlot::LEGS,
                ItemStack::new(1, &Item::DIAMOND_LEGGINGS),
            ),
            (EquipmentSlot::FEET, ItemStack::new(1, &Item::DIAMOND_BOOTS)),
        ];
        self.equip(&equipment);
    }

    pub(super) fn equip(&self, equipment: &[(EquipmentSlot, ItemStack)]) {
        {
            let mut stored = self
                .target
                .player
                .inventory
                .entity_equipment
                .lock()
                .unwrap();
            for (slot, stack) in equipment {
                if *slot != EquipmentSlot::MAIN_HAND {
                    stored.put(slot, stack.clone());
                }
            }
        }
        for (slot, stack) in equipment {
            if *slot == EquipmentSlot::MAIN_HAND {
                self.target.player.inventory.set_held_item(stack.clone());
            }
        }
        // PlayerScreenHandler routes actual equipment changes through this callback.
        for (slot, stack) in equipment {
            self.target.player.enqueue_equipment_change(slot, stack);
        }
        self.target.player.tick(&self.server);
        self.target.player.sync_inventory_to_client();
    }

    pub(super) fn run_clear(&self, input: &str) -> Result<i32, CommandSyntaxError> {
        let source = self.target.player.get_command_source(&self.server);
        let result = self
            .server
            .command_dispatcher
            .load()
            .execute_input(input, &source);
        self.target.player.tick(&self.server);
        result
    }

    pub(super) fn take_observer_equipment_updates(&mut self) -> Vec<CSetEquipment> {
        self.observer
            .take_packets()
            .into_iter()
            .filter_map(|packet| {
                let mut packet = packet.as_ref();
                let packet_id = VarInt::decode(&mut packet).ok()?.0;
                (packet_id == SET_EQUIPMENT.0)
                    .then(|| CSetEquipment::read(&mut packet, &CURRENT_MC_VERSION).ok())
                    .flatten()
            })
            .collect()
    }

    pub(super) fn take_owner_set_slot_updates(&mut self) -> Vec<(i8, i16, bool)> {
        self.target
            .take_packets()
            .into_iter()
            .filter_map(|packet| {
                let mut packet = packet.as_ref();
                let packet_id = VarInt::decode(&mut packet).ok()?.0;
                (packet_id == CONTAINER_SET_SLOT.0)
                    .then(|| CSetContainerSlot::read(&mut packet, &CURRENT_MC_VERSION).ok())
                    .flatten()
                    .map(|update| (update.window_id, update.slot, update.slot_data.0.is_empty()))
            })
            .collect()
    }
}

#[tokio::test]
async fn clear_removes_diamond_armor_attribute_modifiers() {
    let fixture = ClearFixture::new();
    fixture.equip_diamond_armor();
    let armor_base = fixture
        .target
        .player
        .living_entity
        .get_attribute_base(&Attributes::ARMOR);
    assert!(
        fixture
            .target
            .player
            .living_entity
            .get_attribute_value(&Attributes::ARMOR)
            > armor_base
    );

    assert_eq!(fixture.run_clear("clear @s").unwrap(), 4);
    assert_eq!(
        fixture
            .target
            .player
            .living_entity
            .get_attribute_value(&Attributes::ARMOR),
        armor_base
    );
}

#[tokio::test]
async fn clear_drops_armor_protection_after_player_ticks() {
    let fixture = ClearFixture::new();
    let world = fixture.target.player.world();
    let control = TestPlayer::new(&world);
    world.players.store(Arc::new(vec![
        fixture.target.player.clone(),
        fixture.observer.player.clone(),
        control.player.clone(),
    ]));

    fixture.equip_diamond_armor();
    assert!(
        fixture
            .target
            .player
            .living_entity
            .get_attribute_value(&Attributes::ARMOR)
            > control
                .player
                .living_entity
                .get_attribute_value(&Attributes::ARMOR)
    );
    assert_eq!(fixture.run_clear("clear @s").unwrap(), 4);

    for _ in 0..5 {
        fixture.target.player.tick(&fixture.server);
        control.player.tick(&fixture.server);
    }

    let target = &fixture.target.player;
    let control = &control.player;
    let target_health_before = target.living_entity.health.load();
    let control_health_before = control.living_entity.health.load();
    assert!(target.living_entity.damage(
        target.as_ref(),
        8.0,
        pumpkin_data::damage::DamageType::MOB_ATTACK
    ));
    assert!(control.living_entity.damage(
        control.as_ref(),
        8.0,
        pumpkin_data::damage::DamageType::MOB_ATTACK
    ));

    let target_health_loss = target_health_before - target.living_entity.health.load();
    let control_health_loss = control_health_before - control.living_entity.health.load();
    assert_eq!(target_health_loss, 8.0);
    assert_eq!(target_health_loss, control_health_loss);
}

#[tokio::test]
async fn clear_sends_empty_armor_slots_to_tracking_players() {
    let mut fixture = ClearFixture::new();
    fixture.equip_diamond_armor();
    fixture.target.take_packets();
    fixture.observer.take_packets();

    assert_eq!(fixture.run_clear("clear @s").unwrap(), 4);
    let updates = fixture.take_observer_equipment_updates();
    let armor_slots = [
        EquipmentSlot::HEAD,
        EquipmentSlot::CHEST,
        EquipmentSlot::LEGS,
        EquipmentSlot::FEET,
    ];
    let empty_slots: Vec<_> = updates
        .iter()
        .filter(|update| update.entity_id.0 == fixture.target.player.entity_id())
        .flat_map(|update| {
            update
                .equipment
                .iter()
                .filter(|(_, stack)| stack.0.is_empty())
                .map(|(slot, _)| *slot)
        })
        .collect();
    for slot in armor_slots {
        assert!(
            empty_slots.contains(&slot.discriminant()),
            "tracking player did not receive empty equipment slot {}",
            slot.discriminant()
        );
    }
}

#[tokio::test]
async fn clear_sends_empty_armor_to_bedrock_tracking_players() {
    let mut fixture = ClearFixture::new();
    let world = fixture.target.player.world();
    let mut bedrock_observer = TestBedrockPlayer::new(&world).await;
    bedrock_observer.player.watched_section.store(
        pumpkin_world::cylindrical_chunk_iterator::Cylindrical::new(
            fixture.target.player.get_entity().chunk_pos.load(),
            std::num::NonZeroU8::new(2).unwrap(),
        ),
    );
    bedrock_observer
        .player
        .chunk_sender
        .lock()
        .unwrap()
        .mark_sent_out_of_band(fixture.target.player.get_entity().chunk_pos.load());
    world.players.store(Arc::new(vec![
        fixture.target.player.clone(),
        fixture.observer.player.clone(),
        bedrock_observer.player.clone(),
    ]));
    world
        .entity_tracker
        .get_tracked_entity(fixture.target.player.entity_id())
        .unwrap()
        .update_player(&bedrock_observer.player, &world);

    fixture.equip_diamond_armor();
    fixture.target.take_packets();
    fixture.observer.take_packets();
    bedrock_observer.take_packets();

    assert_eq!(fixture.run_clear("clear @s").unwrap(), 4);

    let empty = NetworkItemStackDescriptor::default();
    let expected = CMobArmorEquipment {
        target_runtime_id: (fixture.target.player.entity_id() as u64).into(),
        head: empty.clone(),
        torso: empty.clone(),
        legs: empty.clone(),
        feet: empty.clone(),
        body: empty,
    };
    let expected_packet = bedrock_observer
        .client()
        .serialize_packet(&expected)
        .unwrap();
    assert!(
        bedrock_observer.take_packets().contains(&expected_packet),
        "Bedrock observer did not receive the cleared armor state"
    );

    bedrock_observer.close().await;
}

#[tokio::test]
async fn clear_matching_held_item_sends_empty_main_hand_to_tracking_players() {
    let mut fixture = ClearFixture::new();
    fixture
        .target
        .player
        .inventory
        .set_held_item(ItemStack::new(1, &Item::DIAMOND_SWORD));
    fixture.target.take_packets();
    fixture.observer.take_packets();

    assert_eq!(
        fixture
            .run_clear("clear @s minecraft:diamond_sword")
            .unwrap(),
        1
    );
    assert!(fixture.target.player.inventory.held_item().is_empty());
    let updates = fixture.take_observer_equipment_updates();
    assert!(updates.iter().any(|update| {
        update.entity_id.0 == fixture.target.player.entity_id()
            && update.equipment.iter().any(|(slot, stack)| {
                *slot == EquipmentSlot::MAIN_HAND.discriminant() && stack.0.is_empty()
            })
    }));
}

#[tokio::test]
async fn clear_selected_sword_removes_modifiers_and_syncs_owner_inventory() {
    let mut fixture = ClearFixture::new();
    fixture.open_inventory();
    let sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
    fixture.equip(&[(EquipmentSlot::MAIN_HAND, sword)]);
    let attack_damage_base = fixture
        .target
        .player
        .living_entity
        .get_attribute_base(&Attributes::ATTACK_DAMAGE);
    assert!(
        fixture
            .target
            .player
            .living_entity
            .get_attribute_value(&Attributes::ATTACK_DAMAGE)
            > attack_damage_base
    );
    fixture.target.player.sync_inventory_to_client();
    fixture.target.take_packets();

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
        attack_damage_base
    );
    assert!(
        fixture
            .take_owner_set_slot_updates()
            .iter()
            .any(|(window, slot, empty)| *window == 0 && *slot == 36 && *empty)
    );
}

#[tokio::test]
async fn clear_offhand_item_removes_its_attribute_modifier() {
    let fixture = ClearFixture::new();
    let mut sword = ItemStack::new(1, &Item::IRON_SWORD);
    sword.set_data_component(AttributeModifiersImpl {
        attribute_modifiers: Cow::Owned(vec![pumpkin_data::data_component_impl::Modifier {
            r#type: &Attributes::ATTACK_DAMAGE,
            id: "test:clear_offhand_attack",
            amount: 9.0,
            operation: Operation::AddValue,
            slot: pumpkin_data::AttributeModifierSlot::OffHand,
        }]),
    });
    fixture.equip(&[(EquipmentSlot::OFF_HAND, sword)]);
    let attack_damage_base = fixture
        .target
        .player
        .living_entity
        .get_attribute_base(&Attributes::ATTACK_DAMAGE);
    assert!(
        fixture
            .target
            .player
            .living_entity
            .get_attribute_value(&Attributes::ATTACK_DAMAGE)
            > attack_damage_base
    );

    assert_eq!(
        fixture.run_clear("clear @s minecraft:iron_sword").unwrap(),
        1
    );
    assert!(fixture.target.player.inventory.off_hand_item().is_empty());
    assert_eq!(
        fixture
            .target
            .player
            .living_entity
            .get_attribute_value(&Attributes::ATTACK_DAMAGE),
        attack_damage_base
    );
}

#[tokio::test]
async fn clear_count_mode_keeps_equipment_and_its_modifiers() {
    let mut fixture = ClearFixture::new();
    fixture.equip_diamond_armor();
    let armor = fixture
        .target
        .player
        .living_entity
        .get_attribute_value(&Attributes::ARMOR);
    let modifiers = fixture
        .target
        .player
        .living_entity
        .attributes
        .read()
        .unwrap()[&Attributes::ARMOR.id]
        .modifiers
        .clone();
    fixture.target.take_packets();

    assert_eq!(
        fixture
            .run_clear("clear @s minecraft:diamond_chestplate 0")
            .unwrap(),
        1
    );
    assert_eq!(
        fixture
            .target
            .player
            .inventory
            .entity_equipment
            .lock()
            .unwrap()
            .get(&EquipmentSlot::CHEST)
            .item,
        &Item::DIAMOND_CHESTPLATE
    );
    assert_eq!(
        fixture
            .target
            .player
            .living_entity
            .get_attribute_value(&Attributes::ARMOR),
        armor
    );
    assert_eq!(
        fixture
            .target
            .player
            .living_entity
            .attributes
            .read()
            .unwrap()[&Attributes::ARMOR.id]
            .modifiers,
        modifiers
    );
    assert!(fixture.target.take_packets().into_iter().all(|packet| {
        let mut packet = packet.as_ref();
        VarInt::decode(&mut packet).is_ok_and(|packet_id| packet_id.0 != UPDATE_ATTRIBUTES.0)
    }));
}

#[tokio::test]
async fn clear_preserves_unrelated_effect_attribute_modifiers() {
    let fixture = ClearFixture::new();
    fixture.equip_diamond_armor();
    fixture.target.player.living_entity.add_effect(Effect {
        effect_type: &StatusEffect::SPEED,
        duration: 1_200,
        amplifier: 0,
        ambient: false,
        show_particles: true,
        show_icon: true,
        blend: false,
    });
    let movement_speed = fixture
        .target
        .player
        .living_entity
        .get_attribute_value(&Attributes::MOVEMENT_SPEED);
    assert!(
        movement_speed
            > fixture
                .target
                .player
                .living_entity
                .get_attribute_base(&Attributes::MOVEMENT_SPEED)
    );

    assert_eq!(fixture.run_clear("clear @s").unwrap(), 4);

    assert_eq!(
        fixture
            .target
            .player
            .living_entity
            .get_attribute_value(&Attributes::MOVEMENT_SPEED),
        movement_speed
    );
    assert!(
        fixture
            .target
            .player
            .living_entity
            .has_effect(&StatusEffect::SPEED)
    );
}
