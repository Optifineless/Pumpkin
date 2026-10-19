use std::any::Any;
use std::sync::Arc;

use crate::entity::player::Player;
use crate::entity::projectile::arrow::ArrowPickup;
use crate::entity::projectile::trident::TridentEntity;
use crate::entity::{Entity, EntityBase};
use crate::item::{ItemBehaviour, ItemMetadata};
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::Sound;
use pumpkin_util::GameMode;
use pumpkin_util::math::vector3::Vector3;

pub struct TridentItem;

impl TridentItem {
    fn damage_thrown_trident(player: &Player, trident: &TridentEntity, paid: bool) {
        if !paid || player.gamemode.load() == GameMode::Creative {
            return;
        }
        let damageable = {
            let stack = trident
                .item_stack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            stack.is_damageable() && !stack.is_unbreakable()
        };
        if !damageable {
            return;
        }
        // TridentItem.releaseUsing / ItemStack.hurtWithoutBreaking retain a usable trident.
        let mut amount = 1;
        if let Some(server) = player.world().server.upgrade()
            && let Some(player_arc) = player.world().get_player_by_uuid(player.gameprofile.id)
        {
            let mut event =
                crate::plugin::api::events::player::player_item_damage::PlayerItemDamageEvent::new(
                    player_arc,
                    Item::TRIDENT.registry_key.to_owned(),
                    amount,
                );
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return;
            }
            amount = event.damage;
        }
        let mut stack = trident
            .item_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Self::hurt_without_breaking(&mut stack, amount);
    }

    fn hurt_without_breaking(stack: &mut ItemStack, amount: i32) {
        if !stack.is_damageable() || stack.is_unbreakable() {
            return;
        }
        // ItemStack.hurtWithoutBreaking processes Unbreaking before capping the final damage.
        let unbreaking = stack.get_enchantment_level(&pumpkin_data::Enchantment::UNBREAKING);
        let mut applied = 0;
        for _ in 0..amount.max(0) {
            let mut damage = 1.0;
            if unbreaking > 0 {
                pumpkin_data::Enchantment::UNBREAKING.modify_durability_damage(
                    unbreaking,
                    false,
                    &mut damage,
                );
            }
            applied += i32::from(damage > 0.0);
        }
        let maximum = stack.get_max_damage().unwrap_or(0);
        stack.set_damage(
            stack
                .get_damage()
                .saturating_add(applied)
                .min((maximum - 1).max(0)),
        );
    }

    fn throw_trident(player: &Player, stack: &ItemStack) {
        let world = player.world();
        let Some((source, thrown_stack)) = crate::item::item_utils::claim_item_stack(player, stack)
        else {
            return;
        };
        let paid = !thrown_stack.is_empty();
        player.sync_hand_slot(source, player.inventory().get_slot(source));
        let (yaw, pitch) = player.rotation();
        let entity = Entity::new(world.clone(), player.position(), &EntityType::TRIDENT);
        let trident = Arc::new(TridentEntity::new_shot(
            entity,
            player.get_entity(),
            stack.copy_with_count(1),
            if paid {
                ArrowPickup::Allowed
            } else {
                ArrowPickup::CreativeOnly
            },
        ));
        trident.set_velocity_from_rotation(pitch, yaw, 0.0, 2.5, 1.0);
        super::projectile_weapon::ProjectileWeaponItem::add_shooter_movement(
            &trident.entity,
            player.get_entity(),
        );
        trident.apply_on_projectile_spawned(stack);
        if !crate::entity::projectile::spawn_projectile(
            &world,
            trident.clone(),
            Some(player.get_entity()),
        ) {
            let excess = pumpkin_inventory::inventory::slot_transaction::refund(
                player.inventory().as_ref(),
                source,
                stack,
                thrown_stack,
            );
            crate::item::item_utils::give_or_drop(player, excess);
            player.sync_hand_slot(source, player.inventory().get_slot(source));
            player.living_entity.clear_active_hand();
            return;
        }
        Self::damage_thrown_trident(player, &trident, paid);

        world.play_sound(
            Sound::ItemTridentThrow,
            pumpkin_data::sound::SoundCategory::Players,
            &player.position(),
        );

        player.living_entity.clear_active_hand();
    }
}

impl ItemMetadata for TridentItem {
    fn ids() -> Box<[u16]> {
        [Item::TRIDENT.id].into()
    }
}

impl ItemBehaviour for TridentItem {
    fn normal_use(&self, _item: &Item, player: &Player) {
        let inventory = player.inventory();
        let stack = inventory.held_item();

        player
            .living_entity
            .set_active_hand(pumpkin_util::Hand::Right, stack, 72000);
    }

    fn on_stopped_using(&self, stack: &ItemStack, player: &Player) {
        let use_ticks = player
            .living_entity
            .item_use_time
            .load(std::sync::atomic::Ordering::Relaxed);
        let use_ticks = 72000 - use_ticks;

        if use_ticks < 10
            || (stack.is_damageable()
                && stack
                    .get_max_damage()
                    .is_some_and(|maximum| stack.get_damage() >= maximum - 1))
        {
            return;
        }

        let world = player.world();
        let stack_guard = stack.clone();

        // Check Riptide level
        let mut riptide_level = 0u32;
        if let Some(enchantments) =
            stack_guard.get_data_component::<pumpkin_data::data_component_impl::EnchantmentsImpl>()
        {
            for (enchantment, level) in enchantments.enchantment.iter() {
                if **enchantment == pumpkin_data::Enchantment::RIPTIDE {
                    riptide_level = *level as u32;
                }
            }
        }

        if riptide_level > 0 {
            let in_water = world.get_block_state(&player.position().to_block_pos()).id
                == pumpkin_data::Block::WATER.default_state.id;
            if !in_water {
                player.living_entity.clear_active_hand();
                return;
            }

            if let Some(player_arc) = world.get_player_by_uuid(player.gameprofile.id)
                && let Some(server) = world.server.upgrade()
            {
                let mut event =
                    crate::plugin::api::events::player::player_riptide::PlayerRiptideEvent {
                        player: player_arc,
                        item_name: "trident".to_string(),
                        cancelled: false,
                    };
                server.plugin_manager.fire_blocking(&server, &mut event);
                if event.cancelled {
                    player.living_entity.clear_active_hand();
                    return;
                }
            }

            let f = f64::from(riptide_level);
            let (yaw, pitch) = player.rotation();
            let f_yaw = f32::to_radians(yaw);
            let f_pitch = f32::to_radians(pitch);

            let vx = f64::from(-f32::sin(f_yaw) * f32::cos(f_pitch));
            let vy = f64::from(-f32::sin(f_pitch));
            let vz = f64::from(f32::cos(f_yaw) * f32::cos(f_pitch));

            let sq = (vx * vx + vy * vy + vz * vz).sqrt();
            if sq > 0.0 {
                let mult = (1.0 + f * 0.75) / sq;
                player.living_entity.entity.velocity.store(Vector3::new(
                    vx * mult,
                    vy * mult,
                    vz * mult,
                ));
            }

            player.damage_held_item(1);
            player.living_entity.clear_active_hand();
            return;
        }

        Self::throw_trident(player, &stack_guard);
    }

    fn can_mine(&self, player: &Player) -> bool {
        player.gamemode.load() != GameMode::Creative
    }

    fn get_use_duration(&self) -> i32 {
        72000
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
