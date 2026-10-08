use super::{
    EntityBase,
    experience_orb::ExperienceOrbEntity,
    living::{LivingEntity, attribute_modifier_slot_matches},
    mob::equipment::DEFAULT_EQUIPMENT_DROP_CHANCE,
};
use crate::world::loot::{LootContextParameters, build_entity_death_loot_context};
use pumpkin_data::{
    Enchantment,
    attributes::Attributes,
    damage::DamageType,
    data_component_impl::{EnchantmentsImpl, EquipmentSlot},
    entity::{EntityPose, EntityStatus, EntityType},
    item_stack::ItemStack,
};
use pumpkin_protocol::bedrock::server::actor_event::ActorEventID;
use pumpkin_util::{GameMode, math::vector3::Vector3};
use rand::RngExt;
use std::sync::atomic::Ordering::Relaxed;

/// Returns equipment slots in the order used by vanilla EquipmentSlot.VALUES.
pub(crate) const fn equipment_slots_in_vanilla_order() -> [EquipmentSlot; 8] {
    // EquipmentSlot.VALUES
    [
        EquipmentSlot::MAIN_HAND,
        EquipmentSlot::OFF_HAND,
        EquipmentSlot::FEET,
        EquipmentSlot::LEGS,
        EquipmentSlot::CHEST,
        EquipmentSlot::HEAD,
        EquipmentSlot::BODY,
        EquipmentSlot::SADDLE,
    ]
}

fn visit_equipment_enchantments(
    slot: &EquipmentSlot,
    stack: &ItemStack,
    mut visitor: impl FnMut(&'static Enchantment, i32),
) {
    if !stack.is_empty()
        && let Some(enchantments) = stack.get_data_component::<EnchantmentsImpl>()
    {
        for (enchantment, level) in enchantments.enchantment.iter() {
            if enchantment
                .slots
                .iter()
                .any(|group| attribute_modifier_slot_matches(group, slot))
            {
                visitor(enchantment, *level);
            }
        }
    }
}

#[expect(
    clippy::fn_params_excessive_bools,
    reason = "LivingEntity.dropExperience has five independent eligibility gates"
)]
const fn experience_drop_eligible(
    always: bool,
    consumed: bool,
    recent_player: bool,
    should_drop: bool,
    mob_drops: bool,
) -> bool {
    // LivingEntity.dropExperience
    !consumed && (always || recent_player && should_drop && mob_drops)
}

/// Checks the enchantment effect that prevents an item from dropping on death, regardless of slot.
#[must_use]
pub fn prevents_equipment_drop(stack: &ItemStack) -> bool {
    // Player.destroyVanishingCursedItems / EnchantmentHelper.has(PREVENT_EQUIPMENT_DROP)
    stack
        .get_data_component::<EnchantmentsImpl>()
        .is_some_and(|enchantments| {
            enchantments
                .enchantment
                .iter()
                .any(|(enchantment, _)| enchantment.effects.prevent_equipment_drop)
        })
}

fn equipment_drop_requirement_matches(
    effect: &pumpkin_data::EquipmentDropEffect,
    killer_type: &EntityType,
    direct_type: Option<&EntityType>,
) -> bool {
    // EnchantmentHelper.processEquipmentDropChance builds its damage context with the living killer.
    effect
        .required_entity_type
        .is_none_or(|(target, required)| {
            let actual = match target {
                pumpkin_data::EnchantmentTarget::Attacker
                | pumpkin_data::EnchantmentTarget::Victim => Some(killer_type),
                pumpkin_data::EnchantmentTarget::DamagingEntity => direct_type,
            };
            actual == Some(required)
        })
}

fn mob_equipment_drop_eligible(stack: &ItemStack, player_killed: bool, preserved: bool) -> bool {
    // Mob.dropCustomDeathLoot permits uncredited equipment only when its drop chance preserves it.
    !stack.is_empty() && !prevents_equipment_drop(stack) && (player_killed || preserved)
}

impl LivingEntity {
    /// Marks the entity as dead exactly once and runs the server-side death
    /// flow: stop movement input, attribute the kill, drop loot, broadcast the
    /// `Death` (3) entity event, and hand out XP. Safe to call on every lethal
    /// damage event; only the first call has an effect.
    pub fn on_death(
        &self,
        damage_type: DamageType,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) {
        let _owner = self.own_damage();
        let lifecycle = self.damage_lifecycle();
        let world = self.entity.world.load();
        let Some(dyn_self) = world.get_entity_by_id(self.entity.entity_id) else {
            return;
        };
        if self
            .dead
            .compare_exchange(false, true, Relaxed, Relaxed)
            .is_ok()
        {
            self.movement_input.store(Vector3::default());
            self.jumping.store(false, Relaxed);

            let kill_credit = self.get_kill_credit();
            let killer = kill_credit.as_deref();

            if !self.update_death_stats(&*dyn_self, killer, lifecycle) {
                return;
            }

            // LivingEntity.hurtServer plays the death sound only for a fresh cooldown hit.
            world.send_entity_status(&self.entity, EntityStatus::Death, Some(ActorEventID::Death));
            let death_message = self.prepare_death_message(&*dyn_self, damage_type, source, cause);
            if !self.death_lifecycle_current(lifecycle) {
                return;
            }
            let Some(death_message) = death_message else {
                return;
            };
            self.drop_all_death_loot(&*dyn_self, damage_type, source, cause, lifecycle);
            if !self.death_lifecycle_current(lifecycle) {
                return;
            }
            self.finish_death(&*dyn_self, damage_type, death_message, lifecycle);
        }
    }

    fn finish_death(
        &self,
        caller: &dyn EntityBase,
        damage_type: DamageType,
        death_message: pumpkin_util::text::TextComponent,
        lifecycle: u64,
    ) {
        self.entity.pose.store(EntityPose::Dying);

        // Broadcast death message if it's a player and the gamerule is enabled
        self.broadcast_death_message(caller, death_message, lifecycle);
        if !self.death_lifecycle_current(lifecycle) {
            return;
        }
        if caller.get_player().is_some() {
            // ServerPlayer.die rechecks status after constructing and sending the death message.
            self.combat_tracker
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .recheck_status(self.combat_ticks.load(Relaxed), false);
        }

        // Trigger on_mob_death for active status effects
        let active_effects_vec: Vec<_> = {
            let effects = self
                .active_effects
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            effects
                .values()
                .map(|e| (e.effect_type, e.amplifier))
                .collect()
        };
        for (effect_type, amplifier) in active_effects_vec {
            if let Some(mob_effect) = crate::entity::effect::get_mob_effect(effect_type) {
                mob_effect.on_mob_death(self, amplifier, &damage_type);
                if !self.death_lifecycle_current(lifecycle) {
                    return;
                }
            }
        }

        self.reset_effects_and_attributes();
    }

    // LivingEntity.dropAllDeathLoot: mob equipment belongs to dropCustomDeathLoot;
    // Player.dropEquipment is unconditional here and owns inventory retention.
    fn drop_all_death_loot(
        &self,
        caller: &dyn EntityBase,
        damage_type: DamageType,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
        lifecycle: u64,
    ) {
        let world = self.entity.world.load();
        if caller
            .get_player()
            .is_some_and(|player| player.gamemode.load() == GameMode::Spectator)
        {
            return; // ServerPlayer.die
        }
        let (player_killed, last_player) = {
            let memory = self
                .hurt_by
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (
                memory.player_memory_time > 0,
                memory.player.and_then(|id| self.resolve_hurt_by_player(id)),
            )
        };
        let killer = cause.or(source);
        let should_drop_loot = caller.get_mob().map_or_else(
            || world.level_info.load().game_rules.mob_drops,
            crate::entity::mob::Mob::should_drop_loot,
        );
        if should_drop_loot {
            // LivingEntity.dropFromLootTable keeps the current killer separate from LAST_DAMAGE_PLAYER.
            let params = LootContextParameters {
                killed_by_player: Some(player_killed && last_player.is_some()),
                luck: last_player
                    .as_ref()
                    .filter(|_| player_killed)
                    .map_or(0.0, |player| {
                        player.living_entity.get_attribute_value(&Attributes::LUCK) as f32
                    }),
                this_entity: Some(self.entity.entity_type),
                killer_entity: killer.map(|entity| entity.get_entity().entity_type),
                direct_killer_entity: source.map(|entity| entity.get_entity().entity_type),
                position: Some(self.entity.pos.load()),
                world_time: world.level_info.load().day_time as u64,
                damage_type: Some(damage_type),
                is_raining: Some(world.is_raining()),
                is_thundering: Some(world.is_thundering()),
                is_on_fire: Some(self.entity.fire_ticks.load(Relaxed) > 0),
                ..Default::default()
            };
            let params = build_entity_death_loot_context(
                caller,
                killer,
                source,
                last_player
                    .as_deref()
                    .filter(|_| player_killed)
                    .map(|player| player as &dyn EntityBase),
                &params,
            );
            self.drop_loot_for_life(&params, lifecycle);
            if !self.death_lifecycle_current(lifecycle) {
                return;
            }
            if let Some(mob) = caller.get_mob() {
                self.drop_equipment(player_killed, source, killer, lifecycle);
                if !self.death_lifecycle_current(lifecycle) {
                    return;
                }
                mob.drop_custom_death_loot();
                if !self.death_lifecycle_current(lifecycle) {
                    return;
                }
            }
        }
        if let Some(player) = caller.get_player() {
            player.drop_equipment_on_death(lifecycle);
            if !self.death_lifecycle_current(lifecycle) {
                return;
            }
        }
        self.drop_experience(caller, killer, player_killed);
    }

    fn drop_loot_for_life(&self, params: &LootContextParameters, lifecycle: u64) {
        // LivingEntity.dropFromLootTable: finish delivering loot already generated for this life.
        if !self.death_lifecycle_current(lifecycle) {
            return;
        }
        let key = format!(
            "minecraft:entities/{}",
            self.entity.entity_type.resource_name
        );
        let world = self.entity.world.load();
        if let Some(loot_table) = world.get_loot_table(&key) {
            // LivingEntity.getLootTableSeed returns zero, selecting the table's named sequence.
            let pos = self.entity.block_pos.load();
            for stack in crate::world::loot::generate_loot_from_handle(&loot_table, 0, params) {
                world.drop_stack(&pos, stack);
            }
        }
    }

    fn drop_equipment(
        &self,
        player_killed: bool,
        source: Option<&dyn EntityBase>,
        killer: Option<&dyn EntityBase>,
        lifecycle: u64,
    ) {
        // Mob.dropCustomDeathLoot / EnchantmentHelper.processEquipmentDropChance
        let world = self.entity.world.load();
        let mut rng = rand::rng();
        for slot in equipment_slots_in_vanilla_order() {
            let chance = self
                .equipment_drop_chances
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&slot)
                .copied()
                .unwrap_or(DEFAULT_EQUIPMENT_DROP_CHANCE);
            if chance == 0.0 {
                continue;
            }
            let preserved = chance > 1.0;
            let mut modified_chance = chance;
            if let Some(killer) = killer.filter(|entity| entity.get_living_entity().is_some()) {
                // Vanilla calls this helper with the responsible living entity. Evaluate
                // victim effects first, then attacker effects, against the damage context.
                for target in [
                    pumpkin_data::EnchantmentTarget::Victim,
                    pumpkin_data::EnchantmentTarget::Attacker,
                ] {
                    Self::for_each_equipment_enchantment(killer, |enchantment, level| {
                        for effect in enchantment.effects.equipment_drops {
                            if effect.enchanted == Some(target)
                                && effect
                                    .affected
                                    .unwrap_or(pumpkin_data::EnchantmentTarget::Victim)
                                    == pumpkin_data::EnchantmentTarget::Victim
                                && equipment_drop_requirement_matches(
                                    effect,
                                    killer.get_entity().entity_type,
                                    source.map(|entity| entity.get_entity().entity_type),
                                )
                            {
                                modified_chance = effect.effect.process(level, modified_chance);
                            }
                        }
                    });
                }
            }
            let item = self
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&slot);
            if !mob_equipment_drop_eligible(&item, player_killed, preserved)
                || rng.random::<f32>() >= modified_chance
            {
                continue;
            }
            let mut item = self
                .entity_equipment
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .put(&slot, ItemStack::EMPTY.clone());
            if !preserved
                && item.is_damageable()
                && !item.is_unbreakable()
                && let Some(max_damage) = item.get_max_damage()
            {
                let inner = rng.random_range(0..(max_damage - 3).max(1));
                let outer = rng.random_range(0..=inner);
                item.set_damage(max_damage - outer);
            }
            world.drop_stack(&self.entity.block_pos.load(), item);
            if !self.death_lifecycle_current(lifecycle) {
                return;
            }
        }
    }

    /// Prevents this entity's experience from being awarded again after another consumer takes it.
    pub fn skip_drop_experience(&self) {
        // LivingEntity.skipDropExperience / wasExperienceConsumed
        self.experience_consumed.store(true, Relaxed);
    }

    fn drop_experience(
        &self,
        caller: &dyn EntityBase,
        killer: Option<&dyn EntityBase>,
        player_killed: bool,
    ) {
        // LivingEntity.dropExperience; players are always experience droppers.
        let world = self.entity.world.load();
        if experience_drop_eligible(
            caller.get_player().is_some(),
            self.experience_consumed.load(Relaxed),
            player_killed,
            caller
                .get_mob()
                .is_some_and(crate::entity::mob::Mob::should_drop_experience),
            world.level_info.load().game_rules.mob_drops,
        ) {
            let amount = caller.get_experience_reward(killer);
            if amount > 0 {
                ExperienceOrbEntity::spawn(&world, self.entity.pos.load(), amount);
            }
        }
    }

    /// Visits active equipment enchantments in vanilla slot order, without holding inventory locks.
    pub fn for_each_equipment_enchantment(
        caller: &dyn EntityBase,
        mut visitor: impl FnMut(&'static Enchantment, i32),
    ) {
        // EnchantmentHelper.runIterationOnEquipment / Enchantment.matchingSlot
        let Some(living) = caller.get_living_entity() else {
            return;
        };
        let equipment: Vec<_> = equipment_slots_in_vanilla_order()
            .into_iter()
            .map(|slot| {
                let stack = caller.get_player().map_or_else(
                    || {
                        living
                            .entity_equipment
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .get(&slot)
                    },
                    |player| match slot {
                        EquipmentSlot::MainHand(_) => player.inventory.held_item(),
                        _ => living
                            .entity_equipment
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .get(&slot),
                    },
                );
                (slot, stack)
            })
            .collect();
        for (slot, stack) in equipment {
            visit_equipment_enchantments(&slot, &stack, &mut visitor);
        }
    }

    /// Applies the killer's slot-matching mob-experience effects, truncating only after all effects.
    #[must_use]
    pub fn process_mob_experience(killer: Option<&dyn EntityBase>, amount: u32) -> u32 {
        // LivingEntity.getExperienceReward / EnchantmentHelper.processMobExperience
        let mut result = amount as f32;
        if let Some(killer) = killer {
            Self::for_each_equipment_enchantment(killer, |enchantment, level| {
                enchantment.modify_mob_experience(level, &mut result);
            });
        }
        result.max(0.0) as u32
    }

    fn prepare_death_message(
        &self,
        caller: &dyn EntityBase,
        damage_type: DamageType,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> Option<pumpkin_util::text::TextComponent> {
        let message = Self::get_death_message(caller, damage_type, source, cause);
        if let Some(player) = caller.get_player() {
            let world = self.entity.world.load();
            if let Some(player_arc) = world.get_player_by_uuid(player.gameprofile.id)
                && let Some(server) = world.server.upgrade()
            {
                // ServerPlayer.die builds its message before inventory drops; honor Pumpkin's cancellation.
                let mut event =
                    crate::plugin::api::events::entity::entity_death::PlayerDeathEvent::new(
                        player_arc, message, 0,
                    );
                server.plugin_manager.fire_blocking(&server, &mut event);
                return (!event.cancelled).then_some(event.death_message);
            }
        }
        Some(message)
    }

    fn broadcast_death_message(
        &self,
        caller: &dyn EntityBase,
        message: pumpkin_util::text::TextComponent,
        lifecycle: u64,
    ) {
        let world = self.entity.world.load();
        if let Some(player) = caller.get_player() {
            player.handle_killed_for_life(&message, lifecycle);
            if !self.death_lifecycle_current(lifecycle) {
                return;
            }
            if world.level_info.load().game_rules.show_death_messages
                && let Some(server) = world.server.upgrade()
            {
                for recipient in server.get_all_players() {
                    recipient.send_system_message(&message);
                }
            }
        } else if self.entity.custom_name.load().is_some() {
            tracing::info!(
                "Named entity {} died: {}",
                caller.get_display_name().to_pretty_console(),
                message.to_pretty_console()
            );
        }
    }
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    reason = "Regression tests require existing entities and successful locks"
)]
mod tests;
