use super::LivingEntity;
use crate::entity::EntityBase;
use crate::entity::equipment_damage::{EquippedItem, with_slot};
use crate::entity::player::statistics::StatisticCategory;
use pumpkin_data::damage::DamageType;
use pumpkin_data::data_component_impl::{
    DeathEffect, DeathProtectionImpl, EquipmentSlot, UseEffectsImpl,
};
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::{EntityStatus, EntityType};
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::potion::Effect;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_util::Hand;
use pumpkin_util::math::position::BlockPos;

impl LivingEntity {
    /// Consumes one main-hand or off-hand death protector for a lethal, non-bypassing hit.
    /// Returns true only after an uncancelled resurrection, synchronized inventory, health 1,
    /// ordered component effects, statistics, advancement and entity event 35.
    pub fn try_use_death_protector(
        &self,
        caller: &dyn EntityBase,
        damage_type: &DamageType,
    ) -> bool {
        // LivingEntity.checkTotemDeathProtection.
        let _owner = self.own_damage();
        let lifecycle = self.damage_owner.lifecycle();
        if damage_type.has_tag(&tag::DamageType::MINECRAFT_BYPASSES_INVULNERABILITY) {
            return false;
        }
        for hand in Hand::all() {
            let slot = match hand {
                Hand::Right => EquipmentSlot::MAIN_HAND,
                Hand::Left => EquipmentSlot::OFF_HAND,
            };
            let captured = EquippedItem::capture(caller, &slot);
            let stack = &captured.stack;
            if stack.is_empty() {
                continue;
            }
            let Some(protection) = stack.get_data_component::<DeathProtectionImpl>().cloned()
            else {
                continue;
            };
            let mut event =
                crate::plugin::api::events::entity::entity_resurrect::EntityResurrectEvent::new(
                    self.entity.entity_id,
                );
            if let Some(server) = self.entity.world.load().server.upgrade() {
                server.plugin_manager.fire_blocking(&server, &mut event);
            }
            if self.damage_owner.lifecycle() != lifecycle
                || self.health.load() > 0.0
                || self.dead.load(std::sync::atomic::Ordering::Relaxed)
            {
                return self.health.load() > 0.0;
            }
            if event.cancelled {
                return false;
            }
            // Plugins run unlocked; consume only the original UID in the original slot.
            let consume = |live: &mut ItemStack| {
                if live.is_empty() || live.uid != stack.uid || live.item.id != stack.item.id {
                    return None;
                }
                Some(consume_death_protector(live))
            };
            let protection_item = if let Some(player) = caller.get_player() {
                captured
                    .inventory_index
                    .and_then(|index| with_slot(&player.inventory, index, consume))
                    .flatten()
            } else {
                self.entity_equipment
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .equipment
                    .get_mut(&slot)
                    .and_then(consume)
            };
            let Some(protection_item) = protection_item else {
                return false;
            };
            if let Some(player) = caller.get_player() {
                player.increment_stat(
                    StatisticCategory::Used,
                    i32::from(protection_item.item.id),
                    1,
                );
                if protection_item.item == &Item::TOTEM_OF_UNDYING {
                    player.trigger_advancement_criterion(
                        pumpkin_data::advancement::Advancement::ADVENTURE_TOTEM_OF_UNDYING,
                        "used_totem",
                    );
                }
                // ItemStack.causeUseVibration requires the use_effects component.
                if protection_item
                    .get_data_component::<UseEffectsImpl>()
                    .is_some_and(|effects| effects.interact_vibrations)
                {
                    self.entity
                        .world
                        .load()
                        .game_event_item_interact_finish(&self.entity);
                }
                if let Some(index) = captured.inventory_index {
                    player.sync_hand_slot(index, player.inventory.get_slot(index));
                }
            }
            // Statistics and vibration callbacks can replace equipment too; publish the live hand.
            let live = self.get_stack_in_hand(caller, hand);
            self.update_used_item(hand, &live);
            self.send_equipment_changes(&[(slot, live)]);
            if self.damage_owner.lifecycle() != lifecycle {
                return false;
            }
            if self.health.load() <= 0.0 {
                self.set_health(1.0);
            }
            self.apply_death_effects(caller, &protection);
            self.entity.world.load().send_entity_status(
                &self.entity,
                EntityStatus::ProtectedFromDeath,
                None,
            );
            return true;
        }
        false
    }

    // DeathProtection.applyEffects / ConsumeEffect.apply: preserve the component list order.
    fn apply_death_effects(&self, caller: &dyn EntityBase, protection: &DeathProtectionImpl) {
        for effect in protection.death_effects.iter() {
            match effect {
                DeathEffect::ClearAllEffects => self.reset_effects_and_attributes(),
                DeathEffect::ApplyEffects(effects, probability) => {
                    if rand::random::<f32>() >= *probability {
                        continue;
                    }
                    for effect in effects.iter() {
                        // ApplyStatusEffectsConsumeEffect.apply copies MobEffectInstance's visible details;
                        // MobEffectInstance.setDetailsFrom deliberately omits the hidden-effect chain.
                        let effect = &effect.effect;
                        let Some(effect_type) =
                            StatusEffect::from_minecraft_name(&effect.effect_id)
                        else {
                            continue;
                        };
                        self.add_effect(Effect {
                            effect_type,
                            duration: effect.duration,
                            amplifier: effect.amplifier.clamp(0, i32::from(u8::MAX)) as u8,
                            ambient: effect.ambient,
                            show_particles: effect.show_particles,
                            show_icon: effect.show_icon,
                            blend: false,
                        });
                    }
                }
                DeathEffect::RemoveEffects(types) => match types {
                    pumpkin_data::data_component_impl::IDSet::IDs(ids) => {
                        for effect in ids.iter() {
                            self.remove_effect(effect);
                        }
                    }
                    pumpkin_data::data_component_impl::IDSet::Tag(tag) => {
                        if let Some(server) = self.entity.world.load().server.upgrade() {
                            for effect in server.datapack_manager.get_consume_effect_tag(tag) {
                                self.remove_effect(effect);
                            }
                        }
                    }
                },
                DeathEffect::PlaySound(sound) => {
                    self.entity.world.load().play_sound_event(
                        sound,
                        self.item_effect_sound_category(caller),
                        &self.entity.block_pos.load().to_centered_f64(),
                    );
                }
                DeathEffect::TeleportRandomly {
                    diameter,
                    directional_particles,
                } => {
                    self.apply_death_teleport(caller, *diameter, *directional_particles);
                }
            }
        }
    }

    // TeleportRandomlyConsumeEffect.apply / LivingEntity.randomTeleport.
    fn apply_death_teleport(
        &self,
        caller: &dyn EntityBase,
        diameter: f32,
        directional_particles: bool,
    ) {
        const POSITION_RADIUS: i32 = 127;

        if let Some((origin, target)) = self.find_consumable_teleport_target(diameter) {
            let world = self.entity.world.load_full();
            let teleport_id = caller.get_player().map(|player| {
                player
                    .teleport_id_count
                    .load(std::sync::atomic::Ordering::Relaxed)
            });
            caller.teleport(
                target,
                Some(self.entity.yaw.load()),
                Some(self.entity.pitch.load()),
                world.clone(),
            );
            // The player teleport hook may cancel; cleanup only follows a committed move.
            if self.entity.pos.load() != target
                || caller.get_player().is_some_and(|player| {
                    teleport_id
                        == Some(
                            player
                                .teleport_id_count
                                .load(std::sync::atomic::Ordering::Relaxed),
                        )
                })
            {
                return;
            }
            world.send_entity_status(&self.entity, EntityStatus::Teleport, None);
            Self::finish_random_teleport(caller);
            world.game_event_teleport(&self.entity, origin);
            let (sound, category) = if self.entity.entity_type == &EntityType::FOX {
                (Sound::EntityFoxTeleport, SoundCategory::Neutral)
            } else {
                (Sound::ItemChorusFruitTeleport, SoundCategory::Players)
            };
            world.play_sound(sound, category, &self.entity.pos.load());
            if directional_particles {
                // TeleportRandomlyConsumeEffect.apply / BlockUtil.clampedPackDifferenceInPosition.
                let from = BlockPos::floored_v(origin);
                let to = BlockPos::floored_v(self.entity.pos.load());
                let dx =
                    (to.0.x - from.0.x).clamp(-POSITION_RADIUS, POSITION_RADIUS) + POSITION_RADIUS;
                let dy =
                    (to.0.y - from.0.y).clamp(-POSITION_RADIUS, POSITION_RADIUS) + POSITION_RADIUS;
                let dz =
                    (to.0.z - from.0.z).clamp(-POSITION_RADIUS, POSITION_RADIUS) + POSITION_RADIUS;
                world.sync_world_event(
                    pumpkin_data::world::WorldEvent::ParticlesConsumeEffectTeleport,
                    from,
                    (dx << 16) | (dy << 8) | dz,
                );
            }
            // TeleportRandomlyConsumeEffect.apply's successful-teleport cleanup.
            self.fall_distance.store(0.0);
            self.impulse.reset();
        }
    }
}

// LivingEntity.checkTotemDeathProtection copies the original item, then calls shrink(1).
fn consume_death_protector(stack: &mut ItemStack) -> ItemStack {
    let original = stack.clone();
    stack.decrement(1);
    if stack.is_empty() {
        stack.clear();
    }
    original
}

#[cfg(test)]
mod tests {
    use super::consume_death_protector;
    use pumpkin_data::{item::Item, item_stack::ItemStack};

    #[test]
    fn death_protector_consumes_one_item_even_in_a_modified_stack() {
        let mut stack = ItemStack::new(3, &Item::TOTEM_OF_UNDYING);
        let original = consume_death_protector(&mut stack);
        assert_eq!(original.item_count, 3);
        assert_eq!(stack.item_count, 2);
        stack.item_count = 1;
        consume_death_protector(&mut stack);
        assert!(stack.is_empty());
    }
    #[tokio::test]
    async fn successful_consumable_teleport_resets_fall_and_impulse() {
        use super::*;
        use crate::entity::{Entity, living::test_support::armor_test_world};
        use pumpkin_util::math::{vector2::Vector2, vector3::Vector3};
        let dir = tempfile::tempdir().unwrap();
        let world = armor_test_world(dir.path());
        let chunk = pumpkin_world::chunk::ChunkData::empty_sync(0, 0);
        chunk.set_block_absolute_y(8, 64, 8, pumpkin_data::Block::STONE.default_state.id);
        world.level.loaded_chunks.insert(Vector2::new(0, 0), chunk);
        let living = LivingEntity::new(Entity::new(
            world,
            Vector3::new(8.5, 65.2, 8.5),
            &EntityType::COW,
        ));
        living.fall_distance.store(9.0);
        living.impulse.mace_impact(Vector3::new(8.5, 62.0, 8.5));
        living.apply_death_teleport(&living, 0.0, false);
        assert_eq!(living.fall_distance.load(), 0.0);
        assert_eq!(living.impulse.effective_fall_distance(10.0, 60.0), 10.0);
        crate::server::fixture_lifecycle::finish().await;
    }
}
