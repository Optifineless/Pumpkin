use super::LivingEntity;
use pumpkin_data::{data_component_impl::EquipmentSlot, item_stack::ItemStack};
use pumpkin_protocol::{
    bedrock::{
        client::{CMobArmorEquipment, CMobEquipment},
        network_item::NetworkItemStackDescriptor,
    },
    codec::var_ulong::VarULong,
    java::client::play::CSetEquipment,
};

/// Sends equipment changes to trackers in the packet format for their edition.
pub(super) fn send_equipment_changes(
    living: &LivingEntity,
    equipment: &[(EquipmentSlot, ItemStack)],
    java_packet: &CSetEquipment,
) {
    let world = living.entity.world.load();
    // Mirrors vanilla LivingEntity.updatePlayersWithNewEquipment.
    // Bedrock armor requires one separate packet carrying every armor slot.
    world.send_to_tracking_players(&living.entity, java_packet);

    for (slot, stack) in equipment {
        if *slot != EquipmentSlot::MAIN_HAND && *slot != EquipmentSlot::OFF_HAND {
            continue;
        }

        let container_id = if *slot == EquipmentSlot::OFF_HAND {
            120
        } else {
            0
        };
        let bedrock_packet = CMobEquipment {
            target_runtime_id: VarULong(living.entity_id() as u64),
            item: NetworkItemStackDescriptor::from(stack),
            slot: 0,
            selected_slot: 0,
            container_id,
        };
        world.send_to_tracking_players_bedrock(&living.entity, &bedrock_packet);
    }

    if equipment.iter().any(|(slot, _)| is_armor_slot(slot)) {
        let equipment = living
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let head = equipment.get(&EquipmentSlot::HEAD);
        let torso = equipment.get(&EquipmentSlot::CHEST);
        let legs = equipment.get(&EquipmentSlot::LEGS);
        let feet = equipment.get(&EquipmentSlot::FEET);
        let body = equipment.get(&EquipmentSlot::BODY);
        drop(equipment);

        let bedrock_packet = CMobArmorEquipment {
            target_runtime_id: VarULong(living.entity_id() as u64),
            head: NetworkItemStackDescriptor::from(&head),
            torso: NetworkItemStackDescriptor::from(&torso),
            legs: NetworkItemStackDescriptor::from(&legs),
            feet: NetworkItemStackDescriptor::from(&feet),
            body: NetworkItemStackDescriptor::from(&body),
        };
        world.send_to_tracking_players_bedrock(&living.entity, &bedrock_packet);
    }
}

fn is_armor_slot(slot: &EquipmentSlot) -> bool {
    *slot == EquipmentSlot::HEAD
        || *slot == EquipmentSlot::CHEST
        || *slot == EquipmentSlot::LEGS
        || *slot == EquipmentSlot::FEET
        || *slot == EquipmentSlot::BODY
}
