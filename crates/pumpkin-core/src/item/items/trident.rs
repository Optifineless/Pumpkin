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

impl ItemMetadata for TridentItem {
    fn ids() -> Box<[u16]> {
        [Item::TRIDENT.id].into()
    }
}

impl TridentItem {
    // TridentItem.releaseUsing / ItemStack.consumeAndReturn acquire the projectile's stock.
    fn consume_and_return(player: &Player, expected: &ItemStack) -> Option<ItemStack> {
        let receipt = if player.gamemode.load() == GameMode::Creative {
            expected.copy_with_count(1)
        } else {
            let inventory = player.inventory();
            let owner = pumpkin_inventory::Inventory::slot_transaction(inventory.as_ref(), 0);
            let guard = owner.lock();
            let source =
                (0..pumpkin_inventory::Inventory::size(inventory.as_ref())).find(|slot| {
                    let current = inventory.get_slot(*slot);
                    current.uid == expected.uid && current.are_equal(expected)
                })?;
            let receipt = pumpkin_inventory::inventory::slot_transaction::withdraw(
                inventory.as_ref(),
                source,
                expected,
                1,
            );
            if receipt.is_empty() {
                return None;
            }
            let remaining = inventory.get_slot(source);
            drop(guard);
            player.sync_hand_slot(source, remaining);
            receipt
        };
        Some(receipt)
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

        if use_ticks < 10 {
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

        let Some(thrown_stack) = Self::consume_and_return(player, &stack_guard) else {
            return;
        };
        let (yaw, pitch) = player.rotation();
        let entity = Entity::new(world.clone(), player.position(), &EntityType::TRIDENT);
        let trident = TridentEntity::new_shot(
            entity,
            player.get_entity(),
            thrown_stack,
            if player.gamemode.load() == GameMode::Creative {
                ArrowPickup::CreativeOnly
            } else {
                ArrowPickup::Allowed
            },
        );
        trident.set_velocity_from_rotation(pitch, yaw, 0.0, 2.5, 1.0);
        super::projectile_weapon::ProjectileWeaponItem::add_shooter_movement(
            &trident.entity,
            player.get_entity(),
        );
        trident.apply_on_projectile_spawned(&stack_guard);
        world.spawn_entity(Arc::new(trident));

        world.play_sound(
            Sound::ItemTridentThrow,
            pumpkin_data::sound::SoundCategory::Players,
            &player.position(),
        );

        player.living_entity.clear_active_hand();
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
