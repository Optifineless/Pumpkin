use std::any::Any;
use std::sync::atomic::Ordering;

use crate::entity::EntityBase;
use crate::entity::equipment_damage::EquippedItem;
use crate::entity::player::Player;
use crate::item::items::projectile_weapon::ProjectileWeaponItem;
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::data_component::DataComponent;
use pumpkin_data::data_component_impl::{ChargedProjectilesImpl, EquipmentSlot};
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_inventory::{
    Inventory, inventory::slot_transaction, player::player_inventory::PlayerInventory,
};
use pumpkin_util::{GameMode, Hand};

pub struct CrossbowItem;

impl ItemMetadata for CrossbowItem {
    fn ids() -> Box<[u16]> {
        Box::new([Item::CROSSBOW.id])
    }
}

impl ItemBehaviour for CrossbowItem {
    fn normal_use(&self, _item: &Item, player: &Player) {
        Self::use_in_hand(player, Hand::Right);
    }

    fn normal_use_with_hand(
        &self,
        _item: &Item,
        player: &Player,
        _yaw: f32,
        _pitch: f32,
        hand: Hand,
    ) {
        Self::use_in_hand(player, hand);
    }

    fn on_stopped_using(&self, stack: &ItemStack, player: &Player) {
        if Self::try_load_projectiles(player, stack) {
            player.world().play_sound(
                Sound::ItemCrossbowLoadingEnd,
                SoundCategory::Players,
                &player.position(),
            );
        }
        player.living_entity.clear_active_hand();
    }

    fn get_use_duration(&self) -> i32 {
        Self::USE_DURATION
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl CrossbowItem {
    const USE_DURATION: i32 = 72000;
    pub const ARROW_POWER: f32 = 3.15;

    fn hand_slot(inventory: &PlayerInventory, hand: Hand) -> usize {
        match hand {
            Hand::Right => usize::from(inventory.get_selected_slot()),
            Hand::Left => PlayerInventory::OFF_HAND_SLOT,
        }
    }

    fn use_in_hand(player: &Player, hand: Hand) {
        let inventory = player.inventory();
        let slot = Self::hand_slot(inventory, hand);
        let stack = inventory.get_stack(slot);
        // CrossbowItem.use checks the contents, not just the default component's presence.
        if stack
            .get_data_component::<ChargedProjectilesImpl>()
            .is_some_and(|charged| !charged.projectiles.is_empty())
        {
            Self::fire_projectiles(player, hand, slot, stack);
            return;
        }
        let has_ammo =
            crate::entity::projectile::firework_rocket::FireworkRocketEntity::crossbow_ammunition(
                player,
            )
            .is_some();
        if !has_ammo && player.gamemode.load() != GameMode::Creative {
            return;
        }
        player
            .living_entity
            .set_active_hand(hand, stack, Self::USE_DURATION);
    }

    fn try_load_projectiles(player: &Player, observed: &ItemStack) -> bool {
        let inventory = player.inventory();
        let hand = player
            .living_entity
            .active_hand
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .unwrap_or(Hand::Right);
        let slot = Self::hand_slot(inventory, hand);
        let owner = inventory.slot_transaction(slot);
        let guard = owner.lock();
        let mut weapon = inventory.get_stack(slot);
        if !owner.is_live()
            || weapon.is_empty()
            || weapon.item != &Item::CROSSBOW
            || weapon.uid != observed.uid
            || !weapon.are_equal(observed)
            || weapon
                .get_data_component::<ChargedProjectilesImpl>()
                .is_some_and(|charged| !charged.projectiles.is_empty())
        {
            return false;
        }
        let use_ticks =
            Self::USE_DURATION - player.living_entity.item_use_time.load(Ordering::Relaxed);
        let charge_time =
            crate::enchantment::EnchantmentHelper::modify_crossbow_charge_time(&weapon, 25);
        if use_ticks < charge_time {
            return false;
        }

        // CrossbowItem.tryLoadProjectiles / ProjectileWeaponItem.draw: the same player owner
        // validates the weapon, draws actual ammunition and commits both slots before effects.
        let ammo_slot =
            crate::entity::projectile::firework_rocket::FireworkRocketEntity::crossbow_ammunition(
                player,
            );
        let creative = player.gamemode.load() == GameMode::Creative;
        if ammo_slot.is_none() && !creative {
            return false;
        }
        let mut ammo = ammo_slot.map_or_else(
            || ItemStack::new(1, &Item::ARROW),
            |slot| inventory.get_stack(slot),
        );
        let (drawn, payment) = ProjectileWeaponItem::draw_with_ammo_cost(&weapon, &ammo, creative);
        if drawn.is_empty() {
            return false;
        }
        let projectiles = drawn
            .into_iter()
            .map(|item| {
                let mut nbt = pumpkin_nbt::compound::NbtCompound::new();
                item.write_item_stack(&mut nbt);
                nbt
            })
            .collect();
        // Older loading paths appended patches; replace every charged entry without losing other edits.
        weapon
            .patch
            .retain(|(id, _)| *id != DataComponent::ChargedProjectiles);
        weapon.set_data_component(ChargedProjectilesImpl { projectiles });
        if payment > 0
            && let Some(ammo_slot) = ammo_slot
        {
            ammo.decrement(payment);
            inventory.set_stack(
                ammo_slot,
                if ammo.is_empty() {
                    ItemStack::EMPTY.clone()
                } else {
                    ammo
                },
            );
        }
        inventory.set_stack(slot, weapon);
        drop(guard);
        true
    }

    fn fire_projectiles(player: &Player, hand: Hand, slot: usize, mut held: ItemStack) {
        let expected = held.clone();
        let Some(charged) = held.get_data_component::<ChargedProjectilesImpl>() else {
            return;
        };
        let projectiles: Vec<_> = charged
            .projectiles
            .iter()
            .filter_map(ItemStack::read_item_stack)
            .collect();
        if projectiles.is_empty() {
            return;
        }
        // CrossbowItem.performShooting takes and empties the charge before shoot can call plugins.
        held.patch
            .retain(|(id, _)| *id != DataComponent::ChargedProjectiles);
        if !slot_transaction::exchange(player.inventory().as_ref(), slot, &expected, held.clone()) {
            return;
        }
        let world = player.world();
        world.play_sound(
            Sound::ItemCrossbowShoot,
            SoundCategory::Players,
            &player.position(),
        );
        let equipment = match hand {
            Hand::Right => EquipmentSlot::MAIN_HAND,
            Hand::Left => EquipmentSlot::OFF_HAND,
        };
        let equipped = EquippedItem {
            stack: held.clone(),
            slot: equipment,
            inventory_index: Some(slot),
        };
        let admitted = ProjectileWeaponItem::shoot_projectiles_with_callback(
            &world,
            player.get_entity(),
            &held,
            &projectiles,
            Self::ARROW_POWER,
            1.0,
            false,
            player.gamemode.load() == GameMode::Creative,
            |projectile| ProjectileWeaponItem::hurt_shooting_weapon(player, &equipped, projectile),
        );
        let rejected: Vec<_> = projectiles
            .into_iter()
            .zip(admitted)
            .filter_map(|(projectile, admitted)| (!admitted).then_some(projectile))
            .collect();
        if !rejected.is_empty() {
            Self::restore_projectiles(player, slot, &held, rejected);
        }
    }

    fn restore_projectiles(
        player: &Player,
        slot: usize,
        expected: &ItemStack,
        rejected: Vec<ItemStack>,
    ) {
        let inventory = player.inventory();
        let owner = inventory.slot_transaction(slot);
        let guard = owner.lock();
        let mut current = inventory.get_stack(slot);
        let restore = owner.is_live()
            && !current.is_empty()
            && current.uid == expected.uid
            && current.item == expected.item
            && current
                .get_data_component::<ChargedProjectilesImpl>()
                .is_none_or(|charged| charged.projectiles.is_empty());
        if restore {
            // Restore only this attempt's rejected rounds, preserving unrelated callback edits.
            current
                .patch
                .retain(|(id, _)| *id != DataComponent::ChargedProjectiles);
            current.set_data_component(ChargedProjectilesImpl {
                projectiles: rejected
                    .iter()
                    .map(|projectile| {
                        let mut nbt = pumpkin_nbt::compound::NbtCompound::new();
                        projectile.write_item_stack(&mut nbt);
                        nbt
                    })
                    .collect(),
            });
            inventory.set_stack(slot, current);
        }
        drop(guard);
        if restore {
            player.sync_hand_slot(slot, inventory.get_stack(slot));
        } else {
            // A replaced or broken weapon cannot own the refund. Free multishot/Creative
            // copies must never become physical ammunition in the player's inventory.
            for projectile in rejected {
                if projectile.get_data_component::<pumpkin_data::data_component_impl::IntangibleProjectileImpl>().is_none() {
                    crate::item::item_utils::give_or_drop(player, projectile);
                }
            }
        }
    }
}
