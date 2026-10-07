use super::LivingEntity;
use crate::entity::EntityBase;
use crate::entity::combat::CombatRules;
use crate::entity::player::statistics::{CustomStatistic, StatisticCategory};
use pumpkin_data::attributes::Attributes;
use pumpkin_data::damage::DamageType;
use pumpkin_data::data_component_impl::{
    DamageImpl, DamageResistantImpl, EnchantmentsImpl, EquipmentSlot, EquippableImpl,
};
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::{DamageResult, ItemStack};
use pumpkin_data::particle::Particle;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::{self, Taggable};
use pumpkin_protocol::codec::item_stack_seralizer::ItemStackSerializer;
use pumpkin_protocol::codec::var_int::VarInt;
use pumpkin_protocol::ser::WritingError;
use pumpkin_util::math::vector3::Vector3;
use tracing::warn;

#[cfg(test)]
mod tests;

impl LivingEntity {
    /// Damages the player's helmet for a `damages_helmet` source before cooldown selection.
    /// The caller must check the source tag and nonempty helmet, then scale damage by 0.75.
    pub fn hurt_helmet(&self, caller: &dyn EntityBase, damage_type: &DamageType, damage: f32) {
        // Player.hurtHelmet overrides LivingEntity's empty hook.
        if caller.get_player().is_some() {
            self.do_hurt_equipment(caller, damage_type, damage, &[EquipmentSlot::HEAD]);
        }
    }

    fn hurt_armor(&self, caller: &dyn EntityBase, damage_type: &DamageType, damage: f32) {
        // Player.hurtArmor, Horse.hurtArmor and Wolf.hurtArmor; the base hook is empty.
        let slots = if caller.get_player().is_some() {
            &[
                EquipmentSlot::FEET,
                EquipmentSlot::LEGS,
                EquipmentSlot::CHEST,
                EquipmentSlot::HEAD,
            ][..]
        } else if self.entity.entity_type == &EntityType::HORSE
            || self.entity.entity_type == &EntityType::WOLF
        {
            &[EquipmentSlot::BODY][..]
        } else {
            return;
        };
        self.do_hurt_equipment(caller, damage_type, damage, slots);
    }

    /// Absorbs an admitted wolf hit into body armor, returning whether health damage is skipped.
    /// Call after cooldown admission, before armor, magic and absorption-heart mitigation.
    /// A true result skips those reductions and health damage, but preserves normal hit feedback.
    /// A false result must continue through the normal path, including `hurtArmor(BODY)`.
    pub fn try_absorb_wolf_armor_damage(&self, damage_type: &DamageType, damage: f32) -> bool {
        // Wolf.actuallyHurt and canArmorAbsorb override LivingEntity.actuallyHurt.
        if self.entity.entity_type != &EntityType::WOLF
            || damage_type.has_tag(&tag::DamageType::MINECRAFT_BYPASSES_WOLF_ARMOR)
        {
            return false;
        }
        let (result, updated, cracked) = {
            let mut equipment = self
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(stack) = equipment.equipment.get_mut(&EquipmentSlot::BODY) else {
                return false;
            };
            if stack.is_empty() || stack.item.id != Item::WOLF_ARMOR.id {
                return false;
            }
            let before = wolf_armor_crackiness_by_damage(
                stack.get_damage(),
                stack.get_max_damage().unwrap_or(0),
            );
            let result = if stack.get_data_component::<DamageImpl>().is_some() {
                stack.damage_item(damage.ceil() as i32)
            } else {
                DamageResult::Untouched
            };
            (
                result,
                stack.clone(),
                before != wolf_armor_crackiness(stack),
            )
        };
        let world = self.entity.world.load();
        if result == DamageResult::Broken {
            world.send_entity_status(
                &self.entity,
                crate::entity::equipment_break_status(&EquipmentSlot::BODY),
                None,
            );
        }
        if result != DamageResult::Untouched {
            self.send_equipment_changes(&[(EquipmentSlot::BODY, updated)]);
        }
        if cracked {
            let pos = self.entity.pos.load();
            if !self.entity.is_silent() {
                world.play_sound(Sound::ItemWolfArmorCrack, SoundCategory::Neutral, &pos);
            }
            match wolf_armor_crack_particle_data() {
                Err(error) => warn!("Failed to serialize wolf armor crack particles: {error}"),
                Ok(data) => {
                    // Wolf.actuallyHurt (Wolf.java:420-421) sends these scute particles.
                    world.broadcast_packet_all(
                        &pumpkin_protocol::java::client::play::CParticle::new(
                            false,
                            false,
                            Vector3::new(pos.x, pos.y + 1.0, pos.z),
                            Vector3::new(0.2, 0.1, 0.2),
                            0.1,
                            20,
                            VarInt(Particle::Item.to_id() as i32),
                            &data,
                        ),
                    );
                }
            }
        }
        true
    }

    fn do_hurt_equipment(
        &self,
        caller: &dyn EntityBase,
        damage_type: &DamageType,
        damage: f32,
        slots: &[EquipmentSlot],
    ) {
        // LivingEntity.doHurtEquipment delegates to ItemStack.hurtAndBreak for eligible slots.
        let mut equipment_updates = Vec::new();
        for slot in slots {
            if let Some(player) = caller.get_player() {
                player.damage_item_in_slot_if(slot, |stack| {
                    equipment_damage_amount(stack, damage_type, damage)
                });
            } else {
                let mut equipment = self
                    .entity_equipment
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let Some(stack) = equipment.equipment.get_mut(slot) else {
                    continue;
                };
                let Some(amount) = equipment_damage_amount(stack, damage_type, damage) else {
                    continue;
                };
                let result = stack.damage_item(amount);
                if result != DamageResult::Untouched {
                    equipment_updates.push((slot.clone(), stack.clone()));
                }
                drop(equipment);
                if result == DamageResult::Broken {
                    self.entity.world.load().send_entity_status(
                        &self.entity,
                        crate::entity::equipment_break_status(slot),
                        None,
                    );
                }
            }
        }
        self.send_equipment_changes(&equipment_updates);
    }

    /// Calculates armor absorption for existing callers using their attacker context.
    /// New damage orchestration should pass the victim and direct-source weapon explicitly.
    pub fn get_damage_after_armor_absorb(
        &self,
        damage: f32,
        damage_type: &DamageType,
        attacker: Option<&dyn EntityBase>,
    ) -> f32 {
        let caller = self
            .entity
            .world
            .load()
            .get_entity_by_id(self.entity.entity_id);
        // Older callers pass the projectile's owner here; never use that owner's current weapon.
        let direct = attacker.filter(|entity| {
            entity.get_living_entity().is_none()
                || !(damage_type.has_tag(&tag::DamageType::MINECRAFT_IS_PROJECTILE)
                    || damage_type.has_tag(&tag::DamageType::MINECRAFT_IS_EXPLOSION))
        });
        let weapon = Self::damage_source_weapon(direct);
        self.get_damage_after_armor_absorb_with_weapon(
            caller.as_deref().unwrap_or(self),
            damage,
            damage_type,
            weapon.as_ref(),
        )
    }

    /// Applies armor wear and absorption using the victim's live attributes.
    /// Pass the victim as `caller` and the weapon returned for the direct damage entity.
    /// Call this only for damage admitted by the hurt cooldown, before magic absorption.
    pub fn get_damage_after_armor_absorb_with_weapon(
        &self,
        caller: &dyn EntityBase,
        damage: f32,
        damage_type: &DamageType,
        weapon: Option<&ItemStack>,
    ) -> f32 {
        // LivingEntity.getDamageAfterArmorAbsorb and getArmorValue: wear precedes attribute reads.
        if damage_type.has_tag(&tag::DamageType::MINECRAFT_BYPASSES_ARMOR) {
            return damage;
        }
        self.hurt_armor(caller, damage_type, damage);
        CombatRules::get_damage_after_absorb(
            damage,
            self.get_armor_attribute_value(&Attributes::ARMOR).floor() as f32,
            self.get_armor_attribute_value(&Attributes::ARMOR_TOUGHNESS) as f32,
            weapon,
        )
    }

    fn get_armor_attribute_value(&self, attribute: &Attributes) -> f64 {
        // Attributes.java:13-16 defines these ranges; generated Attributes omits them.
        const ARMOR_MAX: f64 = 30.0;
        const ARMOR_TOUGHNESS_MAX: f64 = 20.0;
        let max = if attribute.id == Attributes::ARMOR.id {
            ARMOR_MAX
        } else {
            ARMOR_TOUGHNESS_MAX
        };
        let attributes = self
            .attributes
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        attributes
            .get(&attribute.id)
            .map_or(attribute.default_value, |instance| {
                instance.value_in_range(0.0, max)
            })
    }

    /// Returns the direct entity's weapon, never the causing entity's current item.
    /// Mirrors `DamageSource.getWeaponItem`, including stored arrow and trident stacks.
    pub fn damage_source_weapon(direct: Option<&dyn EntityBase>) -> Option<ItemStack> {
        let direct = direct?;
        if let Some(arrow) = direct
            .cast_any()
            .downcast_ref::<crate::entity::projectile::arrow::ArrowEntity>()
        {
            return arrow.get_weapon_item();
        }
        if let Some(trident) = direct
            .cast_any()
            .downcast_ref::<crate::entity::projectile::trident::TridentEntity>()
        {
            return Some(
                trident
                    .item_stack
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone(),
            );
        }
        direct
            .get_living_entity()
            .map(|living| living.held_item(direct))
    }

    /// Calculates damage after magic/resistance/enchantment reduction, mirroring vanilla `LivingEntity.getDamageAfterMagicAbsorb`.
    pub fn get_damage_after_magic_absorb(
        &self,
        mut damage: f32,
        damage_type: &DamageType,
        caller: &dyn EntityBase,
        cause: Option<&dyn EntityBase>,
    ) -> f32 {
        if damage_type.has_tag(&tag::DamageType::MINECRAFT_BYPASSES_EFFECTS) {
            return damage;
        }

        // 1. Resistance Effect (evaluated before enchantments)
        if !damage_type.has_tag(&tag::DamageType::MINECRAFT_BYPASSES_RESISTANCE)
            && let Some(effect) = self.get_effect(&StatusEffect::RESISTANCE)
        {
            let absorb_value = (effect.amplifier + 1) * 5;
            let absorb = 25 - absorb_value;
            let v = damage * absorb as f32;
            let old_damage = damage;
            damage = (v / 25.0).max(0.0);
            let damage_resisted = old_damage - damage;
            if damage_resisted > 0.0 {
                if let Some(victim_player) = caller.get_player() {
                    victim_player.increment_stat(
                        StatisticCategory::Custom,
                        CustomStatistic::DamageResisted as i32,
                        (damage_resisted * 10.0).round() as i32,
                    );
                } else if let Some(attacker_player) = cause.and_then(|c| c.get_player()) {
                    attacker_player.increment_stat(
                        StatisticCategory::Custom,
                        CustomStatistic::DamageDealtResisted as i32,
                        (damage_resisted * 10.0).round() as i32,
                    );
                }
            }
        }

        if damage <= 0.0 {
            return 0.0;
        }

        // 2. Enchantment Protection
        if damage_type.has_tag(&tag::DamageType::MINECRAFT_BYPASSES_ENCHANTMENTS) {
            return damage;
        }

        let mut epf = 0.0f32;
        {
            let equipment_lock = self
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for slot in [
                EquipmentSlot::HEAD,
                EquipmentSlot::CHEST,
                EquipmentSlot::LEGS,
                EquipmentSlot::FEET,
            ] {
                if let Some(stack) = equipment_lock.equipment.get(&slot)
                    && !stack.is_empty()
                    && let Some(enchantments) = stack.get_data_component::<EnchantmentsImpl>()
                {
                    for (enchantment, level) in enchantments.enchantment.iter() {
                        enchantment.modify_damage_protection_against(*level, damage_type, &mut epf);
                    }
                }
            }
        }

        if epf > 0.0 {
            damage = CombatRules::get_damage_after_magic_absorb(damage, epf);
        }

        damage
    }
}

// LivingEntity.doHurtEquipment and ItemStack.canBeHurtBy (vanilla 26.3 lines 1894-1903).
fn equipment_damage_amount(
    stack: &ItemStack,
    damage_type: &DamageType,
    damage: f32,
) -> Option<i32> {
    if damage <= 0.0
        || stack.is_empty()
        || !stack.is_damageable()
        || stack.is_unbreakable()
        || stack.get_data_component::<DamageImpl>().is_none()
        || !stack
            .get_data_component::<EquippableImpl>()
            .is_some_and(|equippable| equippable.damage_on_hurt)
        || stack
            .get_data_component::<DamageResistantImpl>()
            .is_some_and(|res| damage_type.is_tagged_with(res.res_type.as_str()) == Some(true))
    {
        return None;
    }
    Some((damage / 4.0).max(1.0) as i32)
}

#[derive(Debug, PartialEq, Eq)]
enum WolfArmorCrackiness {
    None,
    Low,
    Medium,
    High,
}

fn wolf_armor_crack_particle_data() -> Result<Vec<u8>, WritingError> {
    // ItemParticleOption.streamCodec uses ItemStackTemplate.STREAM_CODEC in 26.3.
    let mut data = Vec::new();
    ItemStackSerializer::from(ItemStack::new(1, &Item::ARMADILLO_SCUTE))
        .write_template_with_version(
            &mut data,
            &pumpkin_util::version::JavaMinecraftVersion::V_26_3,
        )?;
    Ok(data)
}

fn wolf_armor_crackiness(stack: &ItemStack) -> WolfArmorCrackiness {
    if !stack.is_damageable()
        || stack.is_unbreakable()
        || stack.get_data_component::<DamageImpl>().is_none()
    {
        return WolfArmorCrackiness::None;
    }
    wolf_armor_crackiness_by_damage(stack.get_damage(), stack.get_max_damage().unwrap_or(0))
}

fn wolf_armor_crackiness_by_damage(damage: i32, max_damage: i32) -> WolfArmorCrackiness {
    // Crackiness.java:7 defines WOLF_ARMOR; byDamage / byFraction use strict boundaries.
    const FRACTION_LOW: f32 = 0.95;
    const FRACTION_MEDIUM: f32 = 0.69;
    const FRACTION_HIGH: f32 = 0.32;
    // ItemStack.getDamageValue clamps before Crackiness.byDamage reads the damage.
    let damage = damage.clamp(0, max_damage.max(0));
    let fraction = (max_damage - damage) as f32 / max_damage as f32;
    if fraction < FRACTION_HIGH {
        WolfArmorCrackiness::High
    } else if fraction < FRACTION_MEDIUM {
        WolfArmorCrackiness::Medium
    } else if fraction < FRACTION_LOW {
        WolfArmorCrackiness::Low
    } else {
        WolfArmorCrackiness::None
    }
}
