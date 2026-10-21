use std::any::Any;
use std::sync::atomic::Ordering;

use crate::entity::EntityBase;
use crate::entity::equipment_damage::EquippedItem;
use crate::entity::player::Player;
use crate::entity::projectile::arrow::ArrowEntity;
use crate::item::items::projectile_weapon::ProjectileWeaponItem;
use crate::item::projectile_stock::ProjectileSource;
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_inventory::{Inventory, player::player_inventory::PlayerInventory};
use pumpkin_protocol::IdOr;
use pumpkin_protocol::java::client::play::CSoundEffect;
use pumpkin_util::{GameMode, Hand};

pub struct BowItem;

struct BowDraw {
    projectiles: Vec<ItemStack>,
    payment: ProjectileSource,
}

impl ItemMetadata for BowItem {
    fn ids() -> Box<[u16]> {
        Box::new([Item::BOW.id])
    }
}

impl ItemBehaviour for BowItem {
    fn normal_use(&self, item: &Item, player: &Player) {
        self.normal_use_with_hand(item, player, 0.0, 0.0, Hand::Right);
    }

    fn normal_use_with_hand(
        &self,
        _item: &Item,
        player: &Player,
        _yaw: f32,
        _pitch: f32,
        hand: Hand,
    ) {
        // Check if player has arrows (or is in creative mode)
        let has_arrows = Self::has_arrows(player);
        let gamemode = player.gamemode.load();

        if !has_arrows && gamemode != GameMode::Creative {
            return;
        }

        // Get the held item stack
        let inventory = player.inventory();
        let stack = inventory.get_stack_in_hand(hand);

        // Start the bow drawing animation
        player
            .living_entity
            .set_active_hand(hand, stack, Self::USE_DURATION);
    }

    fn use_observed_stack(
        &self,
        stack: &ItemStack,
        player: &Player,
        yaw: f32,
        pitch: f32,
        hand: Hand,
    ) -> bool {
        self.normal_use_with_hand(stack.item, player, yaw, pitch, hand);
        false
    }

    fn on_stopped_using(&self, stack: &ItemStack, player: &Player) {
        Self::release_bow(player, stack);
    }

    fn get_use_duration(&self) -> i32 {
        Self::USE_DURATION
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl BowItem {
    /// The maximum number of ticks a bow can be drawn for
    pub const USE_DURATION: i32 = 72000;
    const MAX_DRAW_DURATION: f32 = 20.0;
    pub const ARROW_SPEED_MULTIPLIER: f32 = 3.0;

    /// Called when the player releases the bow
    pub fn release_bow(player: &Player, weapon: &ItemStack) {
        // Get the used ticks
        let use_ticks = player.living_entity.item_use_time.load(Ordering::Relaxed);
        let use_ticks = Self::USE_DURATION - use_ticks;

        // Check minimum draw time
        if use_ticks < 3 {
            return;
        }

        // BowItem.releaseUsing draws through ProjectileWeaponItem.draw/useAmmo before shooting.
        let inventory = player.inventory();
        let hand = *player
            .living_entity
            .active_hand
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .unwrap_or(&Hand::Right);
        let weapon_slot = match hand {
            Hand::Right => usize::from(inventory.get_selected_slot()),
            Hand::Left => PlayerInventory::OFF_HAND_SLOT,
        };
        let Some(mut drawn) = Self::draw_for_release(player, weapon, weapon_slot) else {
            return;
        };

        // Calculate power and fire
        let power = Self::get_power_for_time(use_ticks);

        let is_crit = (power - 1.0).abs() < f32::EPSILON;
        let equipment = if hand == Hand::Left {
            EquipmentSlot::OFF_HAND
        } else {
            EquipmentSlot::MAIN_HAND
        };
        let equipped = EquippedItem {
            stack: weapon.clone(),
            slot: equipment,
            inventory_index: Some(weapon_slot),
        };
        let admitted = Self::shoot_with_results(
            player,
            weapon,
            &drawn.projectiles,
            power,
            1.0,
            is_crit,
            Some((&equipped, &mut drawn.payment)),
        );
        drawn.payment.publish(player);
        if admitted.contains(&true) {
            ProjectileWeaponItem::finish_shooting(player, weapon);
        }
        player.sync_hand_slot(weapon_slot, inventory.get_stack(weapon_slot));
    }

    fn draw_for_release(
        player: &Player,
        weapon: &ItemStack,
        weapon_slot: usize,
    ) -> Option<BowDraw> {
        let inventory = player.inventory();
        let owner = inventory.slot_transaction(weapon_slot);
        let _guard = owner.lock();
        let current_weapon = inventory.get_stack(weapon_slot);
        if !owner.is_live()
            || current_weapon.item != &Item::BOW
            || current_weapon.uid != weapon.uid
            || !current_weapon.are_equal(weapon)
        {
            return None;
        }

        let arrow_slot = player.find_arrow();
        let gamemode = player.gamemode.load();

        if arrow_slot.is_none() && gamemode != GameMode::Creative {
            return None;
        }

        let projectile = arrow_slot.map_or_else(
            || ItemStack::new(1, &Item::ARROW),
            |slot| inventory.get_stack(slot),
        );

        let (projectiles, payment) = ProjectileWeaponItem::draw_with_ammo_cost(
            &current_weapon,
            &projectile,
            gamemode == GameMode::Creative,
        );
        let ammo_update = if let Some(slot) = arrow_slot {
            ProjectileSource::plan(player, slot, &projectile, payment)?.with_weapon(weapon)
        } else {
            ProjectileSource::free_weapon(player, weapon_slot, weapon)?
        };
        Some(BowDraw {
            projectiles,
            payment: ammo_update,
        })
    }

    /// Check if player has arrows in their inventory
    fn has_arrows(player: &Player) -> bool {
        player.find_arrow().is_some()
    }

    /// Calculate the power/charge of the bow based on time held
    #[must_use]
    pub fn get_power_for_time(time_held: i32) -> f32 {
        let mut power = time_held as f32 / Self::MAX_DRAW_DURATION;
        power = (power * power + power * 2.0) / 3.0;
        if power > 1.0 {
            power = 1.0;
        }
        power
    }

    /// Creates projectile matching vanilla `ProjectileWeaponItem::createProjectile`.
    pub fn create_projectile(
        player: &Player,
        weapon: &ItemStack,
        projectile: &ItemStack,
        is_crit: bool,
    ) -> ArrowEntity {
        let world = player.world();
        let is_creative = player.gamemode.load() == GameMode::Creative;
        ProjectileWeaponItem::create_projectile(
            world,
            player.get_entity(),
            weapon,
            projectile,
            is_crit,
            is_creative,
        )
    }

    /// Shoot projectile matching vanilla `ProjectileWeaponItem::shoot`.
    pub fn shoot(
        player: &Player,
        weapon: &ItemStack,
        projectiles: &[ItemStack],
        power: f32,
        uncertainty: f32,
        is_crit: bool,
    ) {
        Self::shoot_with_results(
            player,
            weapon,
            projectiles,
            power,
            uncertainty,
            is_crit,
            None,
        );
    }

    fn shoot_with_results(
        player: &Player,
        weapon: &ItemStack,
        projectiles: &[ItemStack],
        power: f32,
        uncertainty: f32,
        is_crit: bool,
        attempt: Option<(&EquippedItem, &mut ProjectileSource)>,
    ) -> Vec<bool> {
        let (equipped, mut payment) = match attempt {
            Some((equipped, source)) => (Some(equipped), Some(source)),
            None => (None, None),
        };
        if power < 0.1 || projectiles.is_empty() {
            return vec![false; projectiles.len()];
        }

        let world = player.world();
        let is_creative = player.gamemode.load() == GameMode::Creative;
        let speed = power * ProjectileWeaponItem::ARROW_SPEED_MULTIPLIER;

        let session = payment.as_ref().map(|source| source.session());
        let admitted = ProjectileWeaponItem::shoot_projectiles_with_callback(
            &world,
            player.get_entity(),
            weapon,
            projectiles,
            speed,
            uncertainty,
            is_crit,
            is_creative,
            |projectile| {
                payment.as_mut().map_or_else(
                    || world.insert_admitted_projectile(projectile.clone(), |append| append()),
                    |source| source.insert(player, projectile.clone()),
                )
            },
            |projectile| {
                session
                    .as_ref()
                    .is_none_or(|session| session.is_current(player))
                    && equipped.is_none_or(|equipped| {
                        ProjectileWeaponItem::hurt_shooting_weapon(
                            player,
                            equipped,
                            projectile,
                            session.as_ref(),
                        )
                    })
            },
        );

        if !admitted.contains(&true) {
            return admitted;
        }

        let sound_pitch = 1.0 / (rand::random::<f32>() * 0.4 + 1.2) + power * 0.5;
        let sound_packet = CSoundEffect::new(
            IdOr::Id(Sound::EntityArrowShoot as u16),
            SoundCategory::Neutral,
            &player.position(),
            1.0,
            sound_pitch,
            0,
        );
        let chunk_pos = player.get_entity().chunk_pos.load();
        world.broadcast_to_chunk(chunk_pos, &sound_packet);
        admitted
    }

    /// Fire an arrow from the bow with explicit critical flag
    pub fn fire_arrow_with_crit(
        player: &Player,
        power: f32,
        projectile: &ItemStack,
        is_crit: bool,
    ) {
        let held = player.inventory().held_item();
        Self::shoot(
            player,
            &held,
            std::slice::from_ref(projectile),
            power,
            1.0,
            is_crit,
        );
    }

    /// Fire an arrow from the bow
    pub fn fire_arrow(player: &Player, power: f32, projectile: &ItemStack) {
        Self::fire_arrow_with_crit(player, power, projectile, power >= 1.0);
    }
}
