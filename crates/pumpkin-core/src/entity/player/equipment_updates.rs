use super::Player;
use crate::entity::EntityBase;
use pumpkin_data::{
    attributes::Attributes, data_component_impl::EquipmentSlot, item_stack::ItemStack,
};
use pumpkin_util::sync::{LockClass, Mutex, assert_no_guards_held};

#[cfg(test)]
mod tests;

pub(super) enum EquipmentPublication {}

impl LockClass for EquipmentPublication {
    const NAME: &'static str = "player equipment publication";
    const LEVEL: u8 = 30;
}

#[derive(Default)]
pub(super) struct EquipmentUpdates {
    pending: Vec<EquipmentSlot>,
    pending_attributes: Vec<Attributes>,
    publishing: bool,
}

// One initial batch and one correction; callbacks cannot keep the tick in a resend loop.
const MAX_SYNCHRONOUS_EQUIPMENT_PUBLICATIONS: u8 = 2;

struct EquipmentPublisher<'a> {
    updates: &'a Mutex<EquipmentUpdates, EquipmentPublication>,
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
        // Menu notifications can precede the slot write (AbstractContainerMenu.broadcastChanges).
        // Admit the entity/container queue only after the menu owner releases its guard.
        if crate::world::guard_effects::is_active() {
            let slots = slots.to_vec();
            if self.defer_menu_effect(move |player| player.queue_equipment_updates(&slots)) {
                return;
            }
        }
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
        assert_no_guards_held("Player::detect_equipment_updates");
        if self.client.closed() || self.get_entity().is_removed() || !self.menu_player_is_current()
        {
            return;
        }
        // ServerPlayer.doTick -> LivingEntity.detectEquipmentUpdates/collectEquipmentChanges.
        // Packet-driven equips and slot switches run before detection; commands queue no stacks.
        let (mut slots, mut attributes) = {
            let mut updates = self
                .equipment_updates
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if updates.publishing
                || (updates.pending.is_empty() && updates.pending_attributes.is_empty())
            {
                return;
            }
            updates.publishing = true;
            (
                std::mem::take(&mut updates.pending),
                std::mem::take(&mut updates.pending_attributes),
            )
        };
        let mut publisher = EquipmentPublisher {
            updates: &self.equipment_updates,
            active: true,
        };
        let mut publications = 0;
        loop {
            publications += 1;
            let selected = self.inventory.get_selected_slot();
            let hands = self
                .collect_equipment_changes(vec![EquipmentSlot::MAIN_HAND, EquipmentSlot::OFF_HAND]);
            let equipment = self.collect_equipment_changes(slots);
            #[cfg(test)]
            if let Some(hook) = EQUIPMENT_HOOK.with(|hook| hook.borrow_mut().take()) {
                hook(self);
            }
            if self.client.closed()
                || self.get_entity().is_removed()
                || !self.menu_player_is_current()
            {
                return;
            }
            // Callbacks may queue another batch; only this publisher applies modifiers and sends.
            self.living_entity.publish_equipment_changes(&equipment);
            if !attributes.is_empty() {
                crate::entity::attributes::send_attribute_updates_for_living(
                    &self.living_entity,
                    std::mem::take(&mut attributes),
                );
            }
            self.sync_bedrock_equipment_to_owner(&equipment);
            self.reconcile_published_hands(selected, &hands);
            // At most one corrective publication; further callback writes stay queued for next tick.
            // No callback runs after this final live modifier refresh.
            let refreshed_attributes = self.refresh_live_equipment_modifiers();
            let mut updates = self
                .equipment_updates
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // AttributeMap.onAttributeModified retains keys until ServerEntity.sendDirtyEntityData.
            // Silent removal clears modifier IDs too, so the next batch must retain these keys.
            if !updates.pending.is_empty() {
                for attribute in refreshed_attributes {
                    if !updates
                        .pending_attributes
                        .iter()
                        .any(|pending| pending.id == attribute.id)
                    {
                        updates.pending_attributes.push(attribute);
                    }
                }
            }
            if (updates.pending.is_empty() && updates.pending_attributes.is_empty())
                || publications == MAX_SYNCHRONOUS_EQUIPMENT_PUBLICATIONS
            {
                updates.publishing = false;
                // Release under the queue lock; Drop must not reset a subsequent publisher.
                publisher.active = false;
                return;
            }
            slots = std::mem::take(&mut updates.pending);
            attributes = std::mem::take(&mut updates.pending_attributes);
        }
    }

    fn refresh_live_equipment_modifiers(&self) -> Vec<Attributes> {
        // PlayerEquipment.get uses this player's inventory, even if a callback replaced its session.
        let mut slots: Vec<_> = self.inventory.equipment_slots.values().cloned().collect();
        {
            let stored = self
                .inventory
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for slot in stored.equipment.keys() {
                if !slots.contains(slot) {
                    slots.push(slot.clone());
                }
            }
        }
        if !slots.contains(&EquipmentSlot::MAIN_HAND) {
            slots.push(EquipmentSlot::MAIN_HAND);
        }
        slots.sort_unstable_by_key(EquipmentSlot::discriminant);
        let live = self.collect_equipment_changes(slots);
        self.living_entity
            .apply_equipment_attribute_modifiers(&live)
    }

    fn reconcile_published_hands(&self, selected: u8, hands: &[(EquipmentSlot, ItemStack)]) {
        // LivingEntity.collectEquipmentChanges reads the slot values, not merely the selected index.
        let current =
            self.collect_equipment_changes(vec![EquipmentSlot::MAIN_HAND, EquipmentSlot::OFF_HAND]);
        let current_selected = self.inventory.get_selected_slot();
        for ((slot, old), (_, new)) in hands.iter().zip(&current) {
            if old.uid != new.uid
                || !old.are_equal(new)
                || (*slot == EquipmentSlot::MAIN_HAND && selected != current_selected)
            {
                self.queue_equipment_updates(std::slice::from_ref(slot));
                let inventory_slot = if *slot == EquipmentSlot::MAIN_HAND {
                    usize::from(current_selected)
                } else {
                    pumpkin_inventory::player::player_inventory::PlayerInventory::OFF_HAND_SLOT
                };
                if matches!(self.client.as_ref(), crate::net::ClientPlatform::Bedrock(_)) {
                    self.try_send_slot_set_packet(
                    &pumpkin_protocol::java::client::play::CSetPlayerInventory::new(
                        (inventory_slot as i32).into(),
                        &pumpkin_protocol::codec::item_stack_seralizer::ItemStackSerializer::from(
                            new.clone(),
                        ),
                    ),
                );
                }
            }
        }
        if selected != current_selected
            && let crate::net::ClientPlatform::Bedrock(client) = self.client.as_ref()
        {
            client.send_live_hotbar_selection(self);
        }
    }

    pub(super) fn collect_equipment_changes(
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

        assert_no_guards_held("Player::sync_bedrock_equipment_to_owner");

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

    pub(super) fn play_equipment_sound_after_menu(&self, stack: &ItemStack) {
        // Preserve the equip sound after AbstractContainerMenu.broadcastChanges unlocks.
        if crate::world::guard_effects::is_active() {
            let stack = stack.clone();
            if self.defer_menu_effect(move |player| player.play_equipment_sound_after_menu(&stack))
            {
                return;
            }
        }
        assert_no_guards_held("Player::play_equipment_sound_after_menu");
        if let Some(equippable) =
            stack.get_data_component::<pumpkin_data::data_component_impl::EquippableImpl>()
        {
            self.world().play_sound_event(
                &equippable.equip_sound,
                pumpkin_data::sound::SoundCategory::Players,
                &self.position(),
            );
        }
    }

    #[cfg(test)]
    pub(crate) fn set_equipment_update_test_hook(hook: impl FnOnce(&Self) + 'static) {
        EQUIPMENT_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
    }
}
