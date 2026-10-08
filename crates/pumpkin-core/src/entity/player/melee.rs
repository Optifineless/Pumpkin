use super::{
    Animation, Arc, Attributes, CEntityAnimation, ClientPlatform, DamageType, Entity, EntityBase,
    EntityType, EquipmentSlot, ItemStack, Ordering, Player, Sound, SoundCategory, VarULong,
    Vector3, statistics,
};
use crate::entity::combat::{
    self, AttackType, can_smash_attack, mace_smash_damage_bonus, mace_smash_knockback,
    player_attack_sound,
};
use crate::entity::equipment_damage::{EquippedItem, damage_equipped_item};
use pumpkin_data::data_component_impl::{EnchantmentsImpl, WeaponImpl};

impl Player {
    pub fn attack(&self, victim: &Arc<dyn EntityBase>) {
        let world = self.world();
        let Some(server) = world.server.upgrade() else {
            return;
        };
        let victim_entity = victim.get_entity();
        let attacker_entity = &self.living_entity.entity;
        let config = &server.advanced_config.pvp;

        let attacking_item = EquippedItem::capture(self, &EquipmentSlot::MAIN_HAND);
        let item_stack = &attacking_item.stack;
        let attribute_damage = self
            .living_entity
            .get_attribute_value(&Attributes::ATTACK_DAMAGE) as f32;
        let enchanted_damage =
            Self::get_enchanted_damage(victim_entity.entity_type, item_stack, attribute_damage);
        let is_bedrock = matches!(self.client.as_ref(), ClientPlatform::Bedrock(_));
        let charge = if is_bedrock {
            1.0
        } else {
            self.get_attack_strength_scale(0.5)
        };
        self.last_attacked_ticks.store(0, Ordering::Relaxed);

        let attack_type = AttackType::new(self, victim.as_ref(), item_stack, charge);
        let is_mace_smash = item_stack.item.id == pumpkin_data::item::Item::MACE.id
            && can_smash_attack(&self.living_entity);
        let item_bonus = if is_mace_smash {
            let fall_distance = f64::from(self.living_entity.fall_distance.load());
            let per_block =
                crate::enchantment::EnchantmentHelper::modify_fall_based_damage(item_stack, 0.0);
            per_block.mul_add(fall_distance, mace_smash_damage_bonus(fall_distance)) as f32
        } else {
            0.0
        };
        let Some((base_damage, damage)) = Self::melee_attack_damage(
            attribute_damage,
            enchanted_damage,
            item_bonus,
            charge,
            attack_type,
        ) else {
            return;
        };
        let damage_type = if is_mace_smash {
            DamageType::MACE_SMASH
        } else {
            DamageType::PLAYER_ATTACK
        };
        // Player.attack plays the sprint sound before hurtOrSimulate.
        if attack_type == AttackType::Knockback {
            player_attack_sound(&attacker_entity.pos.load(), &world, attack_type);
        }

        if victim
            .get_player()
            .is_some_and(|player| !self.can_attack_player(player))
            || !victim.damage_with_context(
                victim.as_ref(),
                damage,
                damage_type,
                None,
                Some(self),
                Some(self),
            )
        {
            world.play_sound(
                Sound::EntityPlayerAttackNodamage,
                SoundCategory::Players,
                &self.living_entity.entity.pos.load(),
            );
            return;
        }

        if is_mace_smash && damage >= 100.0 {
            self.trigger_advancement(crate::entity::player::advancement::trigger::AdvancementTrigger::DealtOverkillDamage);
        }

        if config.knockback {
            self.cause_extra_knockback(victim.as_ref(), item_stack, attack_type);
        }

        if attack_type == AttackType::Sweeping {
            self.do_sweep_attack(
                victim.as_ref(),
                &attacking_item,
                base_damage,
                damage_type,
                charge,
            );
        }
        self.attack_visual_effects(
            victim_entity,
            attack_type,
            enchanted_damage > attribute_damage && charge > 0.0,
        );

        self.living_entity.set_last_hurt_mob(victim.as_ref());

        self.item_attack_interaction(victim.as_ref(), &attacking_item, damage_type, is_mace_smash);

        // Vanilla `Player#attack` ends the successful-hit branch with
        // `causeFoodExhaustion(0.1F)`. Only landed hits exhaust; the miss/no-damage
        // case returned early above.
        self.add_exhaustion(0.1);
    }

    // Player.attackVisualEffects keeps successful-hit feedback after sweeping.
    fn attack_visual_effects(
        &self,
        victim_entity: &Entity,
        attack_type: AttackType,
        magic_boost: bool,
    ) {
        let attacker_entity = self.get_entity();
        let world = self.world();
        // Player.attackVisualEffects plays the ordinary strong sound after a sprint hit.
        if attack_type != AttackType::Sweeping {
            player_attack_sound(
                &attacker_entity.pos.load(),
                &world,
                if attack_type == AttackType::Knockback {
                    AttackType::Strong
                } else {
                    attack_type
                },
            );
        }

        if matches!(attack_type, AttackType::Critical) {
            let je_packet =
                CEntityAnimation::new(victim_entity.entity_id.into(), Animation::CriticalEffect);
            let be_packet = pumpkin_protocol::bedrock::server::animate::SAnimate {
                action: pumpkin_protocol::bedrock::server::animate::AnimateAction::CriticalHit,
                target_actor_runtime_id: VarULong(victim_entity.entity_id as u64),
                data: 0.0,
                swing_source: None,
            };
            world.broadcast_editioned(&je_packet, &be_packet);
        }
        if magic_boost {
            // Player.attackVisualEffects calls magicCrit for a positive magic boost.
            let je_packet = CEntityAnimation::new(
                victim_entity.entity_id.into(),
                Animation::MagicCriticaleffect,
            );
            let be_packet = pumpkin_protocol::bedrock::server::animate::SAnimate {
                action: pumpkin_protocol::bedrock::server::animate::AnimateAction::MagicCriticalHit,
                target_actor_runtime_id: VarULong(victim_entity.entity_id as u64),
                data: 0.0,
                swing_source: None,
            };
            world.broadcast_editioned(&je_packet, &be_packet);
        }
    }

    // Player.causeExtraKnockback. Java victim motion delivery belongs to hurt orchestration.
    fn cause_extra_knockback(
        &self,
        victim: &dyn EntityBase,
        weapon: &ItemStack,
        attack_type: AttackType,
    ) {
        let base_knockback = self
            .living_entity
            .get_attribute_value(&Attributes::ATTACK_KNOCKBACK) as f32;
        // LivingEntity.getKnockback applies enchantments to the effective base before halving.
        let strength = f64::from(
            crate::enchantment::EnchantmentHelper::modify_knockback(weapon, base_knockback) / 2.0,
        ) + if attack_type == AttackType::Knockback {
            0.5
        } else {
            0.0
        };
        if strength <= 0.0 {
            return;
        }
        let attacker = self.get_entity();
        if victim.get_living_entity().is_some() {
            combat::handle_knockback(attacker, victim, strength * 2.0);
        } else {
            let yaw = attacker.yaw.load().to_radians();
            victim.get_entity().add_velocity(Vector3::new(
                -f64::from(yaw.sin()) * strength,
                0.1,
                f64::from(yaw.cos()) * strength,
            ));
            let velocity = attacker.velocity.load();
            attacker
                .velocity
                .store(Vector3::new(velocity.x * 0.6, velocity.y, velocity.z * 0.6));
        }
        self.living_entity.set_sprinting(false);
    }

    /// Checks melee player-versus-player permission for both the primary target and sweep victims.
    /// Includes creative protection and vanilla Player.canHarmPlayer team rules.
    pub fn can_attack_player(&self, target: &Self) -> bool {
        let world = self.world();
        let Some(server) = world.server.upgrade() else {
            return false;
        };
        let config = &server.advanced_config.pvp;
        if !config.enabled
            || target.living_entity.health.load() <= 0.0
            || (config.protect_creative && target.is_creative())
        {
            return false;
        }
        // ServerPlayer.hurtServer asks the victim's canHarmPlayer at admission.
        target.get_team().is_none_or(|team| {
            // Team's first packet option is friendly fire (ScoreboardTeam.packOptions).
            team.options & 0x01 != 0 || self.get_team_name().as_deref() != Some(team.name.as_str())
        })
    }

    // ServerPlayer.getEnchantedDamage, evaluated separately for every sweep victim.
    fn get_enchanted_damage(target: &EntityType, weapon: &ItemStack, base: f32) -> f32 {
        let mut damage = f64::from(base);
        if let Some(enchantments) = weapon.get_data_component::<EnchantmentsImpl>() {
            for (enchantment, level) in enchantments.enchantment.iter() {
                enchantment.modify_damage_against(*level, &mut damage, Some(target));
            }
        }
        damage as f32
    }

    // Player.doSweepAttack. Item hooks and durability apply only to the primary hit.
    fn do_sweep_attack(
        &self,
        target: &dyn EntityBase,
        weapon: &EquippedItem,
        base_damage: f32,
        damage_type: DamageType,
        charge: f32,
    ) {
        let world = self.world();
        let attacker = self.get_entity();
        player_attack_sound(&attacker.pos.load(), &world, AttackType::Sweeping);
        let sweeping_ratio =
            self.living_entity
                .get_attribute_value(&Attributes::SWEEPING_DAMAGE_RATIO) as f32;
        let search_box = target
            .get_entity()
            .bounding_box
            .load()
            .expand(1.0, 0.25, 1.0);
        let yaw = attacker.yaw.load().to_radians();
        for nearby in world.get_all_at_box(&search_box) {
            let Some(living) = nearby.get_living_entity() else {
                continue;
            };
            let entity = nearby.get_entity();
            if entity.entity_id == attacker.entity_id
                || entity.entity_id == target.get_entity().entity_id
                || (self as &dyn EntityBase).is_allied_to(nearby.as_ref())
                || nearby
                    .cast_any()
                    .downcast_ref::<crate::entity::decoration::armor_stand::ArmorStandEntity>()
                    .is_some_and(
                        crate::entity::decoration::armor_stand::ArmorStandEntity::is_marker,
                    )
                || attacker
                    .pos
                    .load()
                    .squared_distance_to_vec(&entity.pos.load())
                    >= 9.0
            {
                continue;
            }
            let damage = Self::sweep_damage(
                entity.entity_type,
                &weapon.stack,
                base_damage,
                sweeping_ratio,
                charge,
            );
            if nearby
                .get_player()
                .is_none_or(|player| self.can_attack_player(player))
                && nearby.damage_with_context(
                    nearby.as_ref(),
                    damage,
                    damage_type,
                    None,
                    Some(self),
                    Some(self),
                )
            {
                let resistance = living.get_attribute_value(&Attributes::KNOCKBACK_RESISTANCE);
                entity.apply_knockback(
                    combat::knockback_after_resistance(f64::from(0.4f32), resistance),
                    f64::from(yaw.sin()),
                    f64::from(-yaw.cos()),
                );
                crate::enchantment::EnchantmentHelper::on_post_attack(
                    nearby.as_ref(),
                    crate::enchantment::post_attack::AttackEffectContext::melee(self, damage_type),
                    Some(weapon),
                );
            }
        }
        combat::spawn_sweep_particle(attacker, &world, &attacker.pos.load());
    }

    // Player.doSweepAttack scales the complete, per-victim enchanted sweep damage.
    pub(crate) fn sweep_damage(
        target: &EntityType,
        weapon: &ItemStack,
        base_damage: f32,
        ratio: f32,
        charge: f32,
    ) -> f32 {
        Self::get_enchanted_damage(target, weapon, 1.0 + ratio * base_damage) * charge
    }

    // Player.itemAttackInteraction: hurtEnemy, enchantments, then postHurtEnemy.
    fn item_attack_interaction(
        &self,
        victim: &dyn EntityBase,
        weapon: &EquippedItem,
        damage_type: DamageType,
        is_mace_smash: bool,
    ) {
        let stack = &weapon.stack;
        let living_target = victim.get_living_entity().is_some();
        let item_hurt_enemy = living_target && stack.get_data_component::<WeaponImpl>().is_some();
        Self::run_item_attack_interaction(
            || {
                if living_target && is_mace_smash {
                    // MaceItem.hurtEnemy. Wind Burst runs after the smash braking packet.
                    let world = self.world();
                    let attacker = self.get_entity();
                    let velocity = attacker.velocity.load();
                    self.living_entity.protect_mace_landing();
                    self.set_velocity(Vector3::new(velocity.x, f64::from(0.01f32), velocity.z));
                    let grounded = victim.get_entity().on_ground.load(Ordering::Relaxed);
                    let sound = if !grounded {
                        Sound::ItemMaceSmashAir
                    } else if self.living_entity.fall_distance.load() > 5.0 {
                        Sound::ItemMaceSmashGroundHeavy
                    } else {
                        Sound::ItemMaceSmashGround
                    };
                    if grounded {
                        self.spawn_extra_particles_on_fall
                            .store(true, Ordering::Relaxed);
                    }
                    world.play_sound(sound, SoundCategory::Players, &attacker.pos.load());
                    mace_smash_knockback(&world, self, victim);
                }
                if item_hurt_enemy {
                    self.increment_stat(
                        statistics::StatisticCategory::Used,
                        stack.item.id as i32,
                        1,
                    );
                }
            },
            || {
                crate::enchantment::EnchantmentHelper::on_post_attack(
                    victim,
                    crate::enchantment::post_attack::AttackEffectContext::melee(self, damage_type),
                    Some(weapon),
                );
            },
            || {
                if item_hurt_enemy {
                    // MaceItem.postHurtEnemy, followed by ItemStack.postHurtEnemy durability.
                    if is_mace_smash {
                        self.living_entity.fall_distance.store(0.0);
                    }
                    damage_equipped_item(self, weapon, Self::combat_weapon_durability_cost(stack));
                }
            },
        );
    }

    // Player.itemAttackInteraction orders item hooks around configured enchantment effects.
    fn run_item_attack_interaction(
        hurt_enemy: impl FnOnce(),
        enchantments: impl FnOnce(),
        post_hurt_enemy: impl FnOnce(),
    ) {
        hurt_enemy();
        enchantments();
        post_hurt_enemy();
    }

    /// Returns the durability cost for using the held item as a weapon in combat.
    /// Derived from the `Weapon` data component: items without it (e.g. shears, tools
    /// not designed for combat) take no durability damage on attack.
    /// Items with the component use its `item_damage_per_attack` value (default 1;
    /// axes, pickaxes, shovels, and hoes carry a value of 2).
    fn combat_weapon_durability_cost(stack: &ItemStack) -> i32 {
        stack
            .get_data_component::<WeaponImpl>()
            .map_or(0, |w| w.item_damage_per_attack as i32)
    }

    // Player inventory loading restores hand modifiers alongside living equipment.
    pub(super) fn restore_melee_equipment_attributes(&self) {
        self.living_entity
            .apply_current_equipment_attribute_modifiers();
        self.living_entity
            .apply_and_send_equipment_attribute_modifiers(&[(
                EquipmentSlot::MAIN_HAND,
                self.inventory.held_item(),
            )]);
    }

    /// Returns attack charge from the effective attribute, in game ticks.
    pub fn get_attack_strength_scale(&self, partial_tick: f32) -> f32 {
        Self::attack_strength_scale(
            self.last_attacked_ticks.load(Ordering::Acquire),
            partial_tick,
            self.get_current_item_attack_strength_delay(),
        )
    }

    /// Returns vanilla's attack delay using the effective main-hand attack-speed attribute.
    pub fn get_current_item_attack_strength_delay(&self) -> f32 {
        Self::current_item_attack_strength_delay(
            self.living_entity
                .get_attribute_value(&Attributes::ATTACK_SPEED),
        )
    }

    fn current_item_attack_strength_delay(attack_speed: f64) -> f32 {
        (20.0 / attack_speed) as f32
    }

    // Player.getCurrentItemAttackStrengthDelay and Player.getAttackStrengthScale.
    fn attack_strength_scale(ticks: u32, partial_tick: f32, delay: f32) -> f32 {
        ((ticks as f32 + partial_tick) / delay).clamp(0.0, 1.0)
    }

    /// Compatibility entry point for spear charge using the effective attack-speed attribute.
    /// The legacy speed and TPS arguments are ignored, since charge uses game ticks.
    pub fn get_attack_cooldown_progress(
        &self,
        _tps: f64,
        base_time: f64,
        _attack_speed: f64,
    ) -> f64 {
        f64::from(self.get_attack_strength_scale(base_time as f32))
    }

    // Player.baseDamageScaleFactor.
    fn base_damage_scale_factor(charge: f32) -> f32 {
        0.2 + charge * charge * 0.8
    }

    // Player.attack keeps magic separate; MaceItem.getAttackDamageBonus is added
    // after charge scaling, and only the base receives the critical multiplier.
    fn melee_attack_damage(
        attribute_damage: f32,
        enchanted_damage: f32,
        item_bonus: f32,
        charge: f32,
        attack_type: AttackType,
    ) -> Option<(f32, f32)> {
        let magic_boost = (enchanted_damage - attribute_damage) * charge;
        let scaled_base = attribute_damage * Self::base_damage_scale_factor(charge);
        if scaled_base <= 0.0 && magic_boost <= 0.0 {
            return None;
        }
        let mut base = scaled_base + item_bonus;
        if attack_type == AttackType::Critical {
            base *= 1.5;
        }
        Some((base, base + magic_boost))
    }
}

#[cfg(test)]
#[path = "melee_tests.rs"]
mod tests;
