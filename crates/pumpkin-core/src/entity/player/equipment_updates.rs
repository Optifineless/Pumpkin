use super::Player;
use pumpkin_data::{
    attributes::Attributes, data_component_impl::EquipmentSlot, item_stack::ItemStack,
};

#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct EquipmentUpdates {
    pending: Vec<EquipmentSlot>,
    combat_attributes: Vec<Attributes>,
    publishing: bool,
}

struct EquipmentPublisher<'a> {
    updates: &'a std::sync::Mutex<EquipmentUpdates>,
    active: bool,
}

impl Drop for EquipmentPublisher<'_> {
    fn drop(&mut self) {
        if self.active {
            self.updates
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .publishing = false;
        }
    }
}

#[cfg(test)]
type EquipmentHook = Box<dyn FnOnce(&Player)>;
#[cfg(test)]
thread_local! {
    static EQUIPMENT_HOOK: std::cell::RefCell<Option<EquipmentHook>> = const { std::cell::RefCell::new(None) };
}

impl Player {
    /// Queues slots for detection on the player tick, using their current contents.
    pub(crate) fn queue_equipment_updates(&self, slots: &[EquipmentSlot]) {
        let mut updates = self
            .equipment_updates
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for slot in slots {
            if !updates.pending.contains(slot) {
                updates.pending.push(slot.clone());
            }
        }
    }

    /// Applies queued slots from committed inventory before combat, leaving publication queued.
    pub(crate) fn reconcile_equipment_attributes(&self) {
        // Player.attack/getAttackStrengthScale and LivingEntity.getDamageAfterArmorAbsorb
        // read equipment attributes. Pumpkin batches packets before aiStep detection, so
        // reconcile only invalidated slots at those reads, without effects or packets.
        let slots = self
            .equipment_updates
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pending
            .clone();
        if slots.is_empty() {
            return;
        }
        let touched = self
            .living_entity
            .apply_equipment_attribute_modifiers(&self.collect_equipment_changes(slots));
        let mut updates = self
            .equipment_updates
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for attribute in touched {
            if !updates
                .combat_attributes
                .iter()
                .any(|old| old.id == attribute.id)
            {
                updates.combat_attributes.push(attribute);
            }
        }
    }

    pub(crate) fn detect_equipment_updates(&self) {
        // ServerPlayer.doTick -> LivingEntity.detectEquipmentUpdates/collectEquipmentChanges.
        // Packet-driven equips and slot switches run before detection; commands queue no stacks.
        let mut slots = {
            let mut updates = self
                .equipment_updates
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if updates.publishing || updates.pending.is_empty() {
                return;
            }
            updates.publishing = true;
            std::mem::take(&mut updates.pending)
        };
        let mut publisher = EquipmentPublisher {
            updates: &self.equipment_updates,
            active: true,
        };
        loop {
            let equipment = self.collect_equipment_changes(slots);
            #[cfg(test)]
            if let Some(hook) = EQUIPMENT_HOOK.with(|hook| hook.borrow_mut().take()) {
                hook(self);
            }
            // Callbacks may queue another batch; only this publisher applies modifiers and sends.
            self.living_entity.publish_equipment_changes(&equipment);
            self.sync_bedrock_equipment_to_owner(&equipment);
            let attributes = std::mem::take(
                &mut self
                    .equipment_updates
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .combat_attributes,
            );
            if !attributes.is_empty() {
                crate::entity::attributes::send_attribute_updates_for_living(
                    &self.living_entity,
                    attributes,
                );
            }
            let mut updates = self
                .equipment_updates
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if updates.pending.is_empty() {
                updates.publishing = false;
                // Release under the queue lock; Drop must not reset a subsequent publisher.
                publisher.active = false;
                return;
            }
            slots = std::mem::take(&mut updates.pending);
        }
    }

    fn collect_equipment_changes(
        &self,
        slots: Vec<EquipmentSlot>,
    ) -> Vec<(EquipmentSlot, ItemStack)> {
        // LivingEntity.collectEquipmentChanges -> PlayerEquipment.get reads current slots.
        // Match PlayerInventory's main-inventory -> equipment lock order; guards end on return.
        let main = self
            .inventory
            .main_inventory
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let stored = self
            .inventory
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        slots
            .into_iter()
            .map(|slot| {
                let stack = if slot == EquipmentSlot::MAIN_HAND {
                    main[self.inventory.get_selected_slot() as usize].clone()
                } else {
                    stored.get(&slot)
                };
                (slot, stack)
            })
            .collect()
    }

    fn sync_bedrock_equipment_to_owner(&self, equipment: &[(EquipmentSlot, ItemStack)]) {
        use crate::net::ClientPlatform;
        use pumpkin_protocol::bedrock::{
            client::inventory_slot::CInventorySlot,
            network_item::{ContainerName, FullContainerName, NetworkItemStackDescriptor},
            server::inventory_transaction::{WINDOW_ID_ARMOUR, WINDOW_ID_OFF_HAND},
        };
        use pumpkin_protocol::codec::var_uint::VarUInt;

        let ClientPlatform::Bedrock(client) = self.client.as_ref() else {
            return;
        };
        // ClearInventoryCommands.clearInventory -> AbstractContainerMenu.broadcastChanges
        // corrects the owner too; Bedrock keeps armor/offhand outside container 0.
        for (slot, stack) in equipment {
            let (container_id, container_name, slot) = match slot {
                EquipmentSlot::Head(_) => (WINDOW_ID_ARMOUR, ContainerName::Armor, 0),
                EquipmentSlot::Chest(_) => (WINDOW_ID_ARMOUR, ContainerName::Armor, 1),
                EquipmentSlot::Legs(_) => (WINDOW_ID_ARMOUR, ContainerName::Armor, 2),
                EquipmentSlot::Feet(_) => (WINDOW_ID_ARMOUR, ContainerName::Armor, 3),
                EquipmentSlot::OffHand(_) => (WINDOW_ID_OFF_HAND, ContainerName::Offhand, 0),
                _ => continue,
            };
            let packet = CInventorySlot {
                container_id: VarUInt(container_id as u32),
                slot: VarUInt(slot),
                full_container_name: Some(FullContainerName {
                    container_name,
                    dynamic_id: None,
                }),
                storage_item: None,
                item: NetworkItemStackDescriptor::from(stack),
            };
            if let Ok(data) = client.serialize_packet(&packet) {
                client.try_enqueue_packet(data);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn set_equipment_update_test_hook(hook: impl FnOnce(&Self) + 'static) {
        EQUIPMENT_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
    }
}
