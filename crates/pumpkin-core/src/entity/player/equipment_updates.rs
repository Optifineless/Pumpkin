use super::Player;
use pumpkin_data::{data_component_impl::EquipmentSlot, item_stack::ItemStack};

#[derive(Default)]
pub(super) struct EquipmentUpdates {
    pending: Vec<EquipmentSlot>,
    publishing: bool,
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
        loop {
            let equipment = self.collect_equipment_changes(slots);
            #[cfg(test)]
            if let Some(hook) = EQUIPMENT_HOOK.with(|hook| hook.borrow_mut().take()) {
                hook(self);
            }
            // Callbacks may queue another batch; only this publisher applies modifiers and sends.
            self.living_entity.publish_equipment_changes(&equipment);
            let mut updates = self
                .equipment_updates
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if updates.pending.is_empty() {
                updates.publishing = false;
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

    #[cfg(test)]
    pub(crate) fn set_equipment_update_test_hook(hook: impl FnOnce(&Self) + 'static) {
        EQUIPMENT_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
    }
}
