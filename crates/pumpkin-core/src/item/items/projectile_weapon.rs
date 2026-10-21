use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::enchantment::EnchantmentHelper;
use crate::entity::Entity;
use crate::entity::equipment_damage::EquippedItem;
use crate::entity::player::Player;
use crate::entity::projectile::arrow::{ArrowEntity, ArrowPickup};
use crate::world::World;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;

/// Abstraction for projectile-firing weapons (Bow, Crossbow), matching vanilla `ProjectileWeaponItem`.
pub struct ProjectileWeaponItem;

impl ProjectileWeaponItem {
    pub const ARROW_SPEED_MULTIPLIER: f32 = 3.0;

    pub fn add_shooter_movement(projectile: &Entity, shooter: &Entity) {
        // Projectile.shootFromRotation adds known movement after setting launch rotation.
        let mut movement = shooter
            .world
            .load()
            .get_player_by_id(shooter.entity_id)
            .map_or_else(
                || shooter.velocity.load(),
                |player| player.get_known_movement(),
            );
        if shooter.on_ground.load(Ordering::Relaxed) {
            movement.y = 0.0;
        }
        projectile
            .velocity
            .store(projectile.velocity.load().add(&movement));
    }

    #[must_use]
    pub const fn is_arrow(item: &Item) -> bool {
        item.id == Item::ARROW.id
            || item.id == Item::TIPPED_ARROW.id
            || item.id == Item::SPECTRAL_ARROW.id
    }

    #[must_use]
    pub const fn is_arrow_or_firework(item: &Item) -> bool {
        Self::is_arrow(item) || item.id == Item::FIREWORK_ROCKET.id
    }

    /// Creates a projectile matching vanilla `ProjectileWeaponItem::createProjectile`.
    pub fn create_projectile(
        world: Arc<World>,
        shooter: &Entity,
        weapon: &ItemStack,
        projectile: &ItemStack,
        is_crit: bool,
        is_creative: bool,
    ) -> ArrowEntity {
        let arrow_entity = Entity::new(
            world,
            shooter.pos.load(),
            ArrowEntity::entity_type_for_item(projectile.item),
        );
        // AbstractArrow's constructor makes receipt-marked unpaid projectiles creative-only.
        let pickup = if is_creative
            || projectile
                .get_data_component::<pumpkin_data::data_component_impl::IntangibleProjectileImpl>()
                .is_some()
        {
            ArrowPickup::CreativeOnly
        } else {
            ArrowPickup::Allowed
        };
        let arrow =
            ArrowEntity::new_shot_with_weapon(arrow_entity, shooter, projectile, weapon, pickup);

        arrow.set_pierce_level(EnchantmentHelper::process_projectile_piercing(weapon, 0));

        if is_crit {
            arrow.set_critical(true);
        }

        arrow
    }

    /// Draws ammo for projectile weapon matching vanilla `ProjectileWeaponItem::draw`.
    #[must_use]
    pub fn draw(weapon: &ItemStack, projectile: &ItemStack, is_creative: bool) -> Vec<ItemStack> {
        Self::draw_with_ammo_cost(weapon, projectile, is_creative).0
    }

    /// Returns the drawn projectiles and the exact first-projectile payment from the same draw.
    /// Callers must own the observed ammo until that payment is committed.
    #[must_use]
    pub(crate) fn draw_with_ammo_cost(
        weapon: &ItemStack,
        projectile: &ItemStack,
        is_creative: bool,
    ) -> (Vec<ItemStack>, u8) {
        if projectile.item_count == 0 {
            return (Vec::new(), 0);
        }
        let num_projectiles = EnchantmentHelper::process_projectile_count(weapon, 1);
        let mut drawn = Vec::with_capacity(num_projectiles);
        let projectile_copy = projectile.copy_with_count(1);
        let mut payment = 0;
        for i in 0..num_projectiles {
            let (drawn_stack, cost) = Self::use_ammo_with_cost(
                weapon,
                if i == 0 { projectile } else { &projectile_copy },
                is_creative,
                i > 0,
            );
            if i == 0 {
                payment = cost;
            }
            if drawn_stack.item_count > 0 {
                drawn.push(drawn_stack);
            }
        }
        (drawn, payment)
    }

    /// Handles ammo usage logic matching vanilla `ProjectileWeaponItem::useAmmo`.
    #[must_use]
    pub fn use_ammo(
        weapon: &ItemStack,
        projectile: &ItemStack,
        is_creative: bool,
        force_infinite: bool,
    ) -> ItemStack {
        Self::use_ammo_with_cost(weapon, projectile, is_creative, force_infinite).0
    }

    fn use_ammo_with_cost(
        weapon: &ItemStack,
        projectile: &ItemStack,
        is_creative: bool,
        force_infinite: bool,
    ) -> (ItemStack, u8) {
        let ammo_to_use = if force_infinite || is_creative {
            0
        } else {
            EnchantmentHelper::process_ammo_use(weapon, projectile, 1)
        };
        let Ok(ammo_to_use) = u8::try_from(ammo_to_use) else {
            return (ItemStack::EMPTY.clone(), 0);
        };
        if ammo_to_use > projectile.item_count {
            return (ItemStack::EMPTY.clone(), 0);
        }
        let mut copy = projectile.copy_with_count(1);
        if ammo_to_use == 0 {
            copy.set_data_component(pumpkin_data::data_component_impl::IntangibleProjectileImpl);
        }
        (copy, ammo_to_use)
    }

    /// Shoots projectiles matching vanilla `ProjectileWeaponItem::shoot`.
    #[allow(clippy::too_many_arguments)]
    pub fn shoot_projectiles(
        world: &Arc<World>,
        shooter: &Entity,
        weapon: &ItemStack,
        projectiles: &[ItemStack],
        power: f32,
        uncertainty: f32,
        is_crit: bool,
        is_creative: bool,
    ) {
        Self::shoot_projectiles_with_callback(
            world,
            shooter,
            weapon,
            projectiles,
            power,
            uncertainty,
            is_crit,
            is_creative,
            |projectile| world.insert_admitted_projectile(projectile, |append| append()),
            |_| true,
        );
    }

    // ProjectileWeaponItem.shoot applies wear after each spawn and stops if the weapon breaks.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn shoot_projectiles_with_callback(
        world: &Arc<World>,
        shooter: &Entity,
        weapon: &ItemStack,
        projectiles: &[ItemStack],
        power: f32,
        uncertainty: f32,
        is_crit: bool,
        is_creative: bool,
        mut insert: impl FnMut(Arc<dyn crate::entity::EntityBase>) -> bool,
        mut on_spawn: impl FnMut(&ItemStack) -> bool,
    ) -> Vec<bool> {
        let mut admitted = vec![false; projectiles.len()];
        if projectiles.is_empty() {
            return admitted;
        }

        let max_angle = EnchantmentHelper::process_projectile_spread(weapon, 0.0);
        let angle_step = if projectiles.len() == 1 {
            0.0
        } else {
            2.0 * max_angle / (projectiles.len() as f32 - 1.0)
        };
        let angle_offset = ((projectiles.len() - 1) % 2) as f32 * angle_step / 2.0;
        let mut direction = 1.0f32;

        let (yaw, pitch) = (shooter.yaw.load(), shooter.pitch.load());

        // All callbacks see the unconsumed source, no earlier publication and no wear.
        let prepared: Vec<_> = projectiles.iter().enumerate().map(|(i, projectile)| {
            if projectile.is_empty() {
                return None;
            }
            let angle = angle_offset + direction * (i.div_ceil(2) as f32) * angle_step;
            direction = -direction;
            let entity: Arc<dyn crate::entity::EntityBase> = if projectile.item == &Item::FIREWORK_ROCKET {
                Arc::new(crate::entity::projectile::firework_rocket::FireworkRocketEntity::prepare_shot(
                    world, shooter, projectile, pitch, yaw + angle, uncertainty,
                ))
            } else {
                let arrow = Self::create_projectile(
                    world.clone(), shooter, weapon, projectile, is_crit, is_creative,
                );
                arrow.set_velocity_from_rotation(pitch, yaw + angle, 0.0, power, uncertainty);
                if weapon.item.id != Item::CROSSBOW.id {
                    Self::add_shooter_movement(&arrow.entity, shooter);
                }
                Arc::new(arrow)
            };
            crate::entity::projectile::launch::prepare_projectile(world, entity, Some(shooter))
        }).collect();
        for (i, entity) in prepared.into_iter().enumerate() {
            let Some(entity) = entity else {
                continue;
            };
            if !insert(entity.clone()) {
                continue;
            }
            admitted[i] = true;
            if let Some(arrow) = entity.cast_any().downcast_ref::<ArrowEntity>() {
                arrow.apply_on_projectile_spawned(&projectiles[i]);
            }
            if !on_spawn(&projectiles[i]) {
                break;
            }
        }
        admitted
    }

    pub(crate) fn hurt_shooting_weapon(
        player: &Player,
        weapon: &EquippedItem,
        projectile: &ItemStack,
        session: Option<&crate::item::projectile_stock::ProjectileSession>,
    ) -> bool {
        // CrossbowItem.getDurabilityUse costs three for rockets; arrows cost one.
        let amount =
            if weapon.stack.item == &Item::CROSSBOW && projectile.item == &Item::FIREWORK_ROCKET {
                3
            } else {
                1
            };
        let Some(index) = Self::weapon_slot(player, &weapon.stack) else {
            return false;
        };
        let retained = EquippedItem {
            inventory_index: Some(index),
            ..weapon.clone()
        };
        player.damage_projectile_weapon(&retained, amount, || {
            session.is_none_or(|session| session.is_current(player))
        }) && Self::weapon_slot(player, &weapon.stack).is_some()
    }

    pub(crate) fn finish_shooting(player: &Player, weapon: &ItemStack) {
        player.increment_stat(
            pumpkin_data::statistic::StatisticCategory::Used,
            i32::from(weapon.item.id),
            1,
        );
        if let Some(cooldown) = weapon.get_use_cooldown() {
            player.start_cooldown(
                cooldown
                    .cooldown_group
                    .clone()
                    .unwrap_or_else(|| weapon.item.registry_key.to_owned()),
                (cooldown.seconds * 20.0) as i32,
            );
        }
    }

    pub(crate) fn weapon_slot(player: &Player, weapon: &ItemStack) -> Option<usize> {
        use pumpkin_inventory::Inventory;
        let inventory = player.inventory();
        let owner = inventory.slot_transaction(0);
        let _guard = owner.lock();
        if !owner.is_live() {
            return None;
        }
        (0..inventory.size()).find(|slot| {
            let stack = inventory.get_stack(*slot);
            !stack.is_empty() && stack.uid == weapon.uid && stack.item == weapon.item
        })
    }
}
