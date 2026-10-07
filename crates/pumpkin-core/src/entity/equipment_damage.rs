use pumpkin_data::item::Item;
use pumpkin_data::item_stack::{DamageResult, ItemStack};
use pumpkin_inventory::player::player_inventory::PlayerInventory;

fn with_slot<T>(
    inventory: &PlayerInventory,
    index: usize,
    action: impl FnOnce(&mut ItemStack) -> T,
) -> Option<T> {
    if index < PlayerInventory::MAIN_SIZE {
        let mut items = inventory
            .main_inventory
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Some(action(&mut items[index]))
    } else {
        let slot = inventory.equipment_slots.get(&index)?;
        let mut equipment = inventory
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        equipment.equipment.get_mut(slot).map(action)
    }
}

/// Damages the authoritative slot after an unlocked, cancellable callback.
/// Rechecks stack identity and eligibility, then mutates the current durability under its guard.
pub(super) fn damage_inventory_slot(
    inventory: &PlayerInventory,
    index: usize,
    select_amount: impl Fn(&ItemStack) -> Option<i32>,
    before_damage: impl FnOnce(&ItemStack, i32) -> Option<i32>,
) -> Option<(DamageResult, ItemStack, &'static Item)> {
    // LivingEntity.doHurtEquipment / ItemStack.hurtAndBreak are serial in vanilla.
    let (before, amount) = with_slot(inventory, index, |stack| {
        let amount = select_amount(stack)?;
        (amount > 0 && !stack.is_empty() && stack.is_damageable() && !stack.is_unbreakable())
            .then(|| (stack.clone(), amount))
    })??;
    let amount = before_damage(&before, amount)?;
    if amount <= 0 {
        return None;
    }
    with_slot(inventory, index, |stack| {
        // Clones preserve uid; a replacement of the same item type must also be rejected.
        if stack.uid != before.uid
            || stack.item.id != before.item.id
            || select_amount(stack).is_none_or(|amount| amount <= 0)
        {
            return None;
        }
        let result = stack.damage_item(amount);
        (result != DamageResult::Untouched).then(|| (result, stack.clone(), before.item))
    })?
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumpkin_data::data_component_impl::EquipmentSlot;
    use pumpkin_inventory::{build_equipment_slots, entity_equipment::EntityEquipment};
    use std::sync::{Arc, Barrier, Mutex};

    fn inventory() -> PlayerInventory {
        let inventory = PlayerInventory::new(
            Arc::new(Mutex::new(EntityEquipment::new())),
            Arc::new(build_equipment_slots()),
        );
        inventory.set_slot(38, ItemStack::new(1, &Item::IRON_CHESTPLATE));
        inventory.set_slot(0, ItemStack::new(1, &Item::IRON_PICKAXE));
        inventory
    }

    #[test]
    fn concurrent_hits_accumulate_on_the_authoritative_stack() {
        let inventory = inventory();
        for index in [0, 38] {
            let barrier = Barrier::new(2);
            std::thread::scope(|scope| {
                for _ in 0..2 {
                    scope.spawn(|| {
                        let result = damage_inventory_slot(
                            &inventory,
                            index,
                            |_| Some(2),
                            |_, amount| {
                                barrier.wait();
                                Some(amount)
                            },
                        );
                        assert!(result.is_some());
                    });
                }
            });
            assert_eq!(inventory.get_slot(index).get_damage(), 4);
        }
    }

    #[test]
    fn callbacks_can_cancel_or_change_damage_before_commit() {
        let inventory = inventory();
        for index in [0, 38] {
            assert!(damage_inventory_slot(&inventory, index, |_| Some(2), |_, _| None).is_none());
            assert_eq!(inventory.get_slot(index).get_damage(), 0);
            let result = damage_inventory_slot(&inventory, index, |_| Some(2), |_, _| Some(5));
            assert_eq!(result.unwrap().0, DamageResult::Damaged);
            assert_eq!(inventory.get_slot(index).get_damage(), 5);
        }
    }

    #[test]
    fn callback_replacements_and_changed_eligibility_are_preserved() {
        let inventory = inventory();
        for index in [0, 38] {
            let replacement = inventory.get_slot(index).copy_with_count(1);
            let result = damage_inventory_slot(
                &inventory,
                index,
                |_| Some(2),
                |_, amount| {
                    inventory.set_slot(index, replacement.clone());
                    Some(amount)
                },
            );
            assert!(result.is_none());
            assert_eq!(inventory.get_slot(index).uid, replacement.uid);
            assert_eq!(inventory.get_slot(index).get_damage(), 0);
        }
        let result = damage_inventory_slot(
            &inventory,
            38,
            |stack| (!stack.is_unbreakable()).then_some(2),
            |_, amount| {
                let mut equipment = inventory.entity_equipment.lock().unwrap();
                equipment
                    .equipment
                    .get_mut(&EquipmentSlot::CHEST)
                    .unwrap()
                    .set_data_component(pumpkin_data::data_component_impl::UnbreakableImpl);
                Some(amount)
            },
        );
        assert!(result.is_none());
        assert_eq!(inventory.get_slot(38).get_damage(), 0);
    }
}
