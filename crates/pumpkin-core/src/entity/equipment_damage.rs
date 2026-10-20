use super::living::damage_transaction::DamageToken;
use crate::entity::{
    EntityBase, equipment_break_status,
    player::{Player, statistics},
};
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::{DamageResult, ItemStack};
use pumpkin_inventory::player::player_inventory::PlayerInventory;
use pumpkin_util::GameMode;

/// Mutates one inventory slot under its guard; the action must not call plugins or synchronize.
pub(super) fn with_slot<T>(
    inventory: &PlayerInventory,
    index: usize,
    action: impl FnOnce(&mut ItemStack) -> T,
) -> Option<T> {
    let owner = pumpkin_inventory::Inventory::slot_transaction(inventory, index);
    let _guard = owner.lock();
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
#[cfg(test)]
pub(super) fn damage_inventory_slot(
    inventory: &PlayerInventory,
    index: usize,
    select_amount: impl Fn(&ItemStack) -> Option<i32>,
    before_damage: impl FnOnce(&ItemStack, i32) -> Option<i32>,
) -> Option<(DamageResult, ItemStack, &'static Item)> {
    damage_inventory_slot_following(inventory, index, select_amount, before_damage, false)
        .map(|(result, stack, item, _)| (result, stack, item))
}

fn damage_inventory_slot_following(
    inventory: &PlayerInventory,
    index: usize,
    select_amount: impl Fn(&ItemStack) -> Option<i32>,
    before_damage: impl FnOnce(&ItemStack, i32) -> Option<i32>,
    follow_uid: bool,
) -> Option<(DamageResult, ItemStack, &'static Item, usize)> {
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
    let owner = pumpkin_inventory::Inventory::slot_transaction(inventory, index);
    let _guard = owner.lock();
    if !owner.is_live() {
        return None;
    }
    let index = if follow_uid {
        use pumpkin_inventory::Inventory;
        (0..inventory.size()).find(|slot| {
            let stack = inventory.get_stack(*slot);
            !stack.is_empty() && stack.uid == before.uid && stack.item == before.item
        })?
    } else {
        index
    };
    with_slot(inventory, index, |stack| {
        // Clones preserve uid; a replacement of the same item type must also be rejected.
        if stack.is_empty()
            || stack.uid != before.uid
            || stack.item.id != before.item.id
            || select_amount(stack).is_none_or(|amount| amount <= 0)
        {
            return None;
        }
        let result = stack.damage_item(amount);
        (result != DamageResult::Untouched).then(|| (result, stack.clone(), before.item, index))
    })?
}

/// Retains the originating stack and inventory location across combat callbacks.
#[derive(Clone)]
pub struct EquippedItem {
    pub stack: ItemStack,
    pub slot: EquipmentSlot,
    pub inventory_index: Option<usize>,
}

impl EquippedItem {
    /// Captures a slot before effects run; later wear revalidates its stack UID.
    pub fn capture(owner: &dyn EntityBase, slot: &EquipmentSlot) -> Self {
        let index = owner
            .get_player()
            .and_then(|player| inventory_index(player, slot));
        let stack = if let (Some(player), Some(index)) = (owner.get_player(), index) {
            player.inventory.get_slot(index)
        } else {
            owner.get_living_entity().map_or_else(
                || ItemStack::EMPTY.clone(),
                |living| {
                    living
                        .entity_equipment
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .get(slot)
                },
            )
        };
        Self {
            stack,
            slot: slot.clone(),
            inventory_index: index,
        }
    }
}

fn inventory_index(player: &Player, slot: &EquipmentSlot) -> Option<usize> {
    match slot {
        EquipmentSlot::MainHand(_) => Some(player.inventory.get_selected_slot() as usize),
        _ => player
            .living_entity
            .equipment_slots
            .iter()
            .find(|(_, equipped)| *equipped == slot)
            .map(|(index, _)| *index),
    }
}

/// Applies wear to the originating stack, revalidating its identity after unlocked callbacks.
/// Returns false for cancelled wear, ineligible equipment or a replaced stack.
pub fn damage_equipped_item(owner: &dyn EntityBase, item: &EquippedItem, amount: i32) -> bool {
    damage_equipped_item_if(owner, item, None, |_| Some(amount))
}

/// Selects wear on the current originating stack under its guard, before and after callbacks.
/// Hurt-pipeline callers pass their owner's token to reject wear after a life replacement.
pub(crate) fn damage_equipped_item_if(
    owner: &dyn EntityBase,
    item: &EquippedItem,
    life: Option<&DamageToken>,
    select_amount: impl Fn(&ItemStack) -> Option<i32>,
) -> bool {
    if item.stack.is_empty() {
        return false;
    }
    // LivingEntity.doHurtEquipment / ChangeItemDamage.apply keep the originating ItemStack.
    let select_original = |stack: &ItemStack| {
        if stack.is_empty() || stack.uid != item.stack.uid || stack.item.id != item.stack.item.id {
            return None;
        }
        select_amount(stack)
    };
    if let Some(player) = owner.get_player() {
        return item.inventory_index.is_some_and(|index| {
            player.damage_inventory_item_if(&item.slot, index, life, select_original)
        });
    }
    let Some(living) = owner.get_living_entity() else {
        return false;
    };
    let (result, updated) = {
        let mut equipment = living
            .entity_equipment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(stack) = equipment.equipment.get_mut(&item.slot) else {
            return false;
        };
        let Some(amount) = select_original(stack) else {
            return false;
        };
        let result = stack.damage_item(amount);
        if result == DamageResult::Untouched {
            return false;
        }
        (result, stack.clone())
    };
    if result == DamageResult::Broken {
        living.entity.world.load().send_entity_status(
            &living.entity,
            equipment_break_status(&item.slot),
            None,
        );
    }
    living.send_equipment_changes(&[(item.slot.clone(), updated)]);
    true
}

impl Player {
    /// Applies `amount` durability damage to the item in `slot`.
    /// Broadcasts an [`EntityStatus`] break event and syncs the slot if the item is destroyed.
    pub fn damage_item_in_slot(&self, slot: &EquipmentSlot, amount: i32) -> bool {
        self.damage_item_in_slot_if(slot, |_| Some(amount))
    }

    /// Selects durability damage on the guarded slot and rechecks eligibility after plugins run.
    pub(crate) fn damage_item_in_slot_if(
        &self,
        slot: &EquipmentSlot,
        select_amount: impl Fn(&ItemStack) -> Option<i32>,
    ) -> bool {
        let Some(slot_index) = inventory_index(self, slot) else {
            return false;
        };
        self.damage_inventory_item_if(slot, slot_index, None, select_amount)
    }

    fn damage_inventory_item_if(
        &self,
        slot: &EquipmentSlot,
        slot_index: usize,
        life: Option<&DamageToken>,
        select_amount: impl Fn(&ItemStack) -> Option<i32>,
    ) -> bool {
        self.damage_inventory_item_following(slot, slot_index, life, select_amount, false)
            .is_some()
    }

    // Returns false after an actual break, even if its callback restores the same UID.
    pub(crate) fn damage_projectile_weapon(&self, item: &EquippedItem, amount: i32) -> bool {
        item.inventory_index.is_none_or(|index| {
            self.damage_inventory_item_following(
                &item.slot,
                index,
                None,
                |stack| {
                    (!stack.is_empty()
                        && stack.uid == item.stack.uid
                        && stack.item == item.stack.item)
                        .then_some(amount)
                },
                true,
            ) != Some(DamageResult::Broken)
        })
    }

    fn damage_inventory_item_following(
        &self,
        slot: &EquipmentSlot,
        slot_index: usize,
        life: Option<&DamageToken>,
        select_amount: impl Fn(&ItemStack) -> Option<i32>,
        follow_uid: bool,
    ) -> Option<DamageResult> {
        // Player.hurtArmor/hurtHelmet and BlocksAttacks.hurtBlockingItem supply combat ownership.
        // ItemStack.hurtAndBreak also serves ordinary tools, which need no combat segment.
        if matches!(
            self.gamemode.load(),
            GameMode::Creative | GameMode::Spectator
        ) {
            return None;
        }
        let updated = damage_inventory_slot_following(
            &self.inventory,
            slot_index,
            select_amount,
            |stack, amount| {
                if let Some(server) = self.world().server.upgrade()
                    && let Some(player_arc) = self.world().get_player_by_uuid(self.gameprofile.id)
                {
                    let mut event = crate::plugin::api::events::player::player_item_damage::PlayerItemDamageEvent::new(
                        player_arc,
                        stack.item.registry_key.to_string(),
                        amount,
                    );
                    server.plugin_manager.fire_blocking(&server, &mut event);
                    if event.cancelled || !life.is_none_or(DamageToken::is_current_life) {
                        return None;
                    }
                    return Some(event.damage);
                }
                Some(amount)
            },
            follow_uid,
        );

        if let Some((result, _, original_item, committed_index)) = updated {
            if result == pumpkin_data::item_stack::DamageResult::Broken {
                if let Some(server) = self.world().server.upgrade()
                    && let Some(player_arc) = self.world().get_player_by_uuid(self.gameprofile.id)
                {
                    let mut event = crate::plugin::api::events::player::player_item_break::PlayerItemBreakEvent::new(
                        player_arc,
                        original_item.registry_key.to_string(),
                    );
                    server.plugin_manager.fire_blocking(&server, &mut event);
                }
                self.increment_stat(
                    statistics::StatisticCategory::Broken,
                    original_item.id as i32,
                    1,
                );
                self.world().send_entity_status(
                    &self.living_entity.entity,
                    super::equipment_break_status(slot),
                    None,
                );
            }

            // A damage or break callback may move the retained weapon or replace either slot.
            self.sync_hand_slot(slot_index, self.inventory.get_slot(slot_index));
            if committed_index != slot_index {
                self.sync_hand_slot(committed_index, self.inventory.get_slot(committed_index));
            }
            if slot.is_armor_slot() {
                self.living_entity
                    .send_equipment_changes(&[(slot.clone(), self.inventory.get_slot(slot_index))]);
            }

            return Some(result);
        }

        None
    }
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn verification4_tool_wear_does_not_acquire_combat_ownership() {
        use crate::{
            net::java::combat_test_support::TestPlayer,
            server::combat_test_support::{server, world},
        };
        let dir = tempfile::tempdir().unwrap();
        let server = server(dir.path());
        let world = world(&server, dir.path());
        let player = TestPlayer::new(&world).player;
        world.players.store(Arc::new(vec![player.clone()]));
        player
            .inventory
            .set_slot(0, ItemStack::new(1, &Item::IRON_PICKAXE));
        let before = player.living_entity.damage_entry_count();
        assert!(player.damage_item_in_slot(&EquipmentSlot::MAIN_HAND, 1));
        assert_eq!(player.inventory.get_slot(0).get_damage(), 1);
        assert_eq!(player.living_entity.damage_entry_count(), before);
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
    #[tokio::test]
    async fn melee_captured_wear_accumulates_and_preserves_replacement_equipment() {
        use crate::entity::{
            Entity,
            living::{LivingEntity, tests::armor_test_world},
        };
        use pumpkin_data::entity::EntityType;
        use pumpkin_util::math::vector3::Vector3;
        let temp = tempfile::tempdir().unwrap();
        let living = LivingEntity::new(Entity::new(
            armor_test_world(temp.path()),
            Vector3::default(),
            &EntityType::ZOMBIE,
        ));
        let sword = ItemStack::new(1, &Item::DIAMOND_SWORD);
        living
            .entity_equipment
            .lock()
            .unwrap()
            .put(&EquipmentSlot::MAIN_HAND, sword);
        let captured = EquippedItem::capture(&living, &EquipmentSlot::MAIN_HAND);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for _ in 0..100 {
                        assert!(damage_equipped_item(&living, &captured, 1));
                    }
                });
            }
        });
        assert_eq!(
            living
                .entity_equipment
                .lock()
                .unwrap()
                .get(&EquipmentSlot::MAIN_HAND)
                .get_damage(),
            800
        );
        let replacement = ItemStack::new(1, &Item::DIAMOND_SWORD);
        living
            .entity_equipment
            .lock()
            .unwrap()
            .put(&EquipmentSlot::MAIN_HAND, replacement.clone());
        assert!(!damage_equipped_item(&living, &captured, 9));
        let remaining = living
            .entity_equipment
            .lock()
            .unwrap()
            .get(&EquipmentSlot::MAIN_HAND);
        assert_eq!(remaining.uid, replacement.uid);
        assert_eq!(remaining.get_damage(), 0);
    }
}
