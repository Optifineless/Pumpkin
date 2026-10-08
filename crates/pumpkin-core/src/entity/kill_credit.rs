use super::{EntityBase, combat::FallLocation, living::LivingEntity};
use pumpkin_data::{
    damage::DamageType,
    entity::EntityType,
    tag::{self, Taggable},
};
use pumpkin_nbt::compound::NbtCompound;
use std::sync::{Arc, atomic::Ordering::Relaxed};
use uuid::Uuid;

const PLAYER_HURT_MEMORY_TIME: i32 = 100; // LivingEntity.resolvePlayerResponsibleForDamage

#[derive(Default)]
pub(super) struct HurtByMemory {
    pub(super) player: Option<Uuid>,
    pub(super) player_memory_time: i32,
    pub(super) mob: Option<Uuid>,
    pub(super) mob_timestamp: i64,
}

impl HurtByMemory {
    const fn set_player(&mut self, player: Option<Uuid>, time_to_remember: i32) {
        self.player = player;
        self.player_memory_time = time_to_remember;
    }

    // LivingEntity.baseTick clears the reference one tick after the timer reaches zero.
    const fn tick_player(&mut self) {
        if self.player_memory_time > 0 {
            self.player_memory_time -= 1;
        } else {
            self.player = None;
        }
    }

    fn write_nbt(&self, nbt: &mut NbtCompound, current_tick: i64) {
        // LivingEntity.addAdditionalSaveData stores EntityReference UUIDs and elapsed ticks.
        if let Some(player) = self.player {
            nbt.put_uuid("last_hurt_by_player", player);
            nbt.put_int("last_hurt_by_player_memory_time", self.player_memory_time);
        }
        if let Some(mob) = self.mob {
            nbt.put_uuid("last_hurt_by_mob", mob);
            nbt.put_int(
                "ticks_since_last_hurt_by_mob",
                (current_tick - self.mob_timestamp) as i32,
            );
        }
    }

    fn read_nbt(nbt: &NbtCompound, current_tick: i64) -> Self {
        // LivingEntity.readAdditionalSaveData
        Self {
            player: nbt.get_uuid("last_hurt_by_player"),
            player_memory_time: nbt.get_int("last_hurt_by_player_memory_time").unwrap_or(0),
            mob: nbt.get_uuid("last_hurt_by_mob"),
            mob_timestamp: current_tick
                - i64::from(nbt.get_int("ticks_since_last_hurt_by_mob").unwrap_or(0)),
        }
    }
}

impl LivingEntity {
    /// Returns the remembered player, or the remembered living attacker if no player is remembered.
    pub fn get_kill_credit(&self) -> Option<Arc<dyn EntityBase>> {
        // LivingEntity.getKillCredit does not fall back to combat entries or test the XP timer.
        let memory = self
            .hurt_by
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(player) = memory.player {
            return self
                .resolve_hurt_by_player(player)
                .map(|p| p as Arc<dyn EntityBase>);
        }
        memory.mob.and_then(|id| self.resolve_hurt_by_mob(id))
    }

    pub(super) fn resolve_hurt_by_player(&self, id: Uuid) -> Option<Arc<super::player::Player>> {
        // EntityReference.getPlayer resolves players in any dimension.
        self.entity
            .world
            .load()
            .server
            .upgrade()?
            .get_player_by_uuid(id)
            .filter(|player| !player.get_entity().is_removed())
    }

    fn resolve_hurt_by_mob(&self, id: Uuid) -> Option<Arc<dyn EntityBase>> {
        // EntityReference.getLivingEntity also searches every loaded dimension.
        let server = self.entity.world.load().server.upgrade()?;
        server
            .get_player_by_uuid(id)
            .map(|p| p as Arc<dyn EntityBase>)
            .or_else(|| {
                server.worlds.load().iter().find_map(|world| {
                    world
                        .get_entity_by_uuid(id)
                        .filter(|entity| entity.get_living_entity().is_some())
                })
            })
            .filter(|entity| !entity.get_entity().is_removed())
    }

    /// Sets player credit and its remaining entity ticks, including an unloaded wolf owner.
    pub fn set_last_hurt_by_player(&self, player: Option<Uuid>, time_to_remember: i32) {
        // LivingEntity.setLastHurtByPlayer
        self.hurt_by
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set_player(player, time_to_remember);
    }

    /// Records an accepted hit after cooldown handling, even if it dealt no health damage.
    /// `source` is the direct entity; `cause` is the responsible entity (the projectile owner).
    pub fn record_hurt_by(
        &self,
        damage_type: DamageType,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) {
        // LivingEntity.hurtServer -> resolveMobResponsibleForDamage / resolvePlayerResponsibleForDamage
        let Some(attacker) = cause.or(source) else {
            return;
        };
        if attacker.get_living_entity().is_some()
            && !damage_type.has_tag(&tag::DamageType::MINECRAFT_NO_ANGER)
            && (damage_type != DamageType::WIND_CHARGE
                || !self
                    .entity
                    .entity_type
                    .has_tag(&tag::EntityType::MINECRAFT_NO_ANGER_FROM_WIND_CHARGE))
        {
            let mut memory = self
                .hurt_by
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            memory.mob = Some(attacker.get_entity().entity_uuid);
            memory.mob_timestamp = self.combat_ticks.load(Relaxed);
            self.last_attacker_id
                .store(attacker.get_entity().entity_id, Relaxed);
            self.last_attacked_time
                .store(self.combat_ticks.load(Relaxed) as i32, Relaxed);
        }
        if let Some(player) = attacker.get_player() {
            self.set_last_hurt_by_player(Some(player.gameprofile.id), PLAYER_HURT_MEMORY_TIME);
        } else if attacker.get_entity().entity_type == &EntityType::WOLF
            && let Some(wolf) = attacker.get_mob().filter(|mob| mob.is_tamed())
        {
            let owner = wolf.get_owner_uuid();
            self.set_last_hurt_by_player(
                owner,
                if owner.is_some() {
                    PLAYER_HURT_MEMORY_TIME
                } else {
                    0
                },
            );
        }
    }

    /// Remembers an attacked living target for owner retaliation goals, clearing nonliving targets.
    pub fn set_last_hurt_mob(&self, target: &dyn EntityBase) {
        // LivingEntity.setLastHurtMob
        self.last_attacking_id.store(
            if target.get_living_entity().is_some() {
                target.get_entity().entity_id
            } else {
                0
            },
            Relaxed,
        );
        self.last_attack_time
            .store(self.combat_ticks.load(Relaxed) as i32, Relaxed);
    }

    pub(super) fn tick_combat_memory(&self) {
        let current_tick = self.combat_ticks.load(Relaxed);
        let world = self.entity.world.load();
        let mut memory = self
            .hurt_by
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        memory.tick_player();
        if let Some(mob) = memory.mob.and_then(|id| self.resolve_hurt_by_mob(id))
            && (mob.get_living_entity().is_none_or(|living| {
                living.health.load() <= 0.0
                    || living.dead.load(Relaxed)
                    || !living.entity.is_alive()
            }) || current_tick - memory.mob_timestamp > i64::from(PLAYER_HURT_MEMORY_TIME))
        {
            memory.mob = None;
            memory.mob_timestamp = current_tick;
            self.last_attacker_id.store(0, Relaxed);
            self.last_attacked_time.store(current_tick as i32, Relaxed);
        }
        if let Some(attacker) = memory.mob.and_then(|id| self.resolve_hurt_by_mob(id)) {
            self.last_attacker_id
                .store(attacker.get_entity().entity_id, Relaxed);
            self.last_attacked_time
                .store(memory.mob_timestamp as i32, Relaxed);
        } else {
            self.last_attacker_id.store(0, Relaxed);
        }
        let target = self.last_attacking_id.load(Relaxed);
        if target != 0
            && world.get_entity_by_id(target).is_some_and(|target| {
                target.get_living_entity().is_none_or(|living| {
                    living.health.load() <= 0.0
                        || living.dead.load(Relaxed)
                        || !living.entity.is_alive()
                })
            })
        {
            self.last_attacking_id.store(0, Relaxed);
        }
        if current_tick % 20 == 0 {
            // LivingEntity.tick checks combat status every 20 entity ticks.
            self.combat_tracker
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .recheck_status(
                    current_tick,
                    self.health.load() > 0.0 && !self.dead.load(Relaxed),
                );
        }
    }

    /// Returns the living entity tick counter used by incoming and outgoing attack memory.
    pub fn combat_tick_count(&self) -> i64 {
        self.combat_ticks.load(Relaxed)
    }

    pub(super) fn write_hurt_by_nbt(&self, nbt: &mut NbtCompound) {
        self.hurt_by
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .write_nbt(nbt, self.combat_ticks.load(Relaxed));
    }

    pub(super) fn read_hurt_by_nbt(&self, nbt: &NbtCompound) {
        let memory = HurtByMemory::read_nbt(nbt, self.combat_ticks.load(Relaxed));
        self.last_attacker_id.store(
            memory
                .mob
                .and_then(|id| self.resolve_hurt_by_mob(id))
                .map_or(0, |entity| entity.get_entity().entity_id),
            Relaxed,
        );
        self.last_attacked_time
            .store(memory.mob_timestamp as i32, Relaxed);
        *self
            .hurt_by
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = memory;
    }

    pub(super) fn reset_combat_memory(&self) {
        // PlayerList.respawn constructs a fresh player; Pumpkin reuses its LivingEntity.
        *self
            .hurt_by
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = HurtByMemory::default();
        self.last_attacker_id.store(0, Relaxed);
        self.last_attacked_time.store(0, Relaxed);
        self.last_attacking_id.store(0, Relaxed);
        self.last_attack_time.store(0, Relaxed);
        self.experience_consumed.store(false, Relaxed);
        self.combat_ticks.store(0, Relaxed);
        *self
            .combat_tracker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            super::combat::CombatTracker::new();
    }

    /// Records nonzero health damage before changing health, preserving the history of a lethal hit.
    pub fn record_health_damage(
        &self,
        damage_type: DamageType,
        damage: f32,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) {
        // LivingEntity.actuallyHurt / CombatTracker.recordDamage
        if damage == 0.0 {
            return;
        }
        let world = self.entity.world.load();
        self.combat_tracker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .record_damage(
                self.combat_ticks.load(Relaxed),
                self.fall_distance.load(),
                FallLocation::get_current_fall_location(self, &world),
                damage_type,
                damage,
                source,
                cause,
            );
    }
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    reason = "Regression tests require existing entities and successful locks"
)]
mod tests;
