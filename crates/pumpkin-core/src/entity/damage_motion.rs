use super::living::damage_transaction::DamageToken;
use super::{Entity, EntityBase, living::LivingEntity, player::Player};
use pumpkin_data::attributes::Attributes;
use pumpkin_protocol::{
    bedrock::client::CSetActorMotion, codec::var_ulong::VarULong,
    java::client::play::CEntityVelocity,
};
use pumpkin_util::math::vector3::Vector3;
use std::sync::atomic::Ordering::Relaxed;

#[cfg(test)]
mod review3_tests;

/// Adds tracker motion while owning each living recipient separately.
// Entity.push runs serially in vanilla; cross-entity ownership suspends the previous owner.
pub(super) fn push_collision_impulse<T: EntityBase + ?Sized>(target: &T, impulse: Vector3<f64>) {
    if let Some(living) = target.get_living_entity() {
        living.push_hurt(impulse);
    } else {
        target.get_entity().push_impulse(impulse);
    }
}

impl LivingEntity {
    /// Flushes completed player motion and metadata in the final tracking phase.
    pub(crate) fn flush_tracked_player_motion(&self) {
        let _owner = self.own_damage();
        if !self.is_respawning() {
            // ServerEntity.sendChanges:211-214, after all entity ticks and projectile follow-ups.
            self.entity.flush_player_motion_owned();
            if self.entity.synched_data.is_dirty() {
                self.entity.send_dirty_entity_data();
            }
        }
    }

    /// Applies vanilla living knockback, including the effective resistance attribute.
    pub fn knockback(&self, strength: f64, x: f64, z: f64) {
        // LivingEntity.knockback (1647-1664); Entity.apply_knockback supplies direction and motion math.
        let _owner = self.own_damage();
        let strength = super::combat::knockback_after_resistance(
            strength,
            self.get_attribute_value(&Attributes::KNOCKBACK_RESISTANCE),
        );
        if !self.defer_hurt_knockback(strength, x, z) {
            self.entity.apply_knockback(strength, x, z);
        }
    }
}

impl Entity {
    /// Consumes pending motion flags when motion is delivered, including client-applied explosion impulses.
    pub fn acknowledge_motion_delivery(&self) {
        self.hurt_marked.store(false, Relaxed);
        if self.entity_type == &pumpkin_data::entity::EntityType::PLAYER {
            self.velocity_dirty
                .store(false, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Flushes damage motion while the caller owns this entity, as ServerEntity.sendChanges does.
    pub(crate) fn flush_hurt_motion_owned(&self) {
        // ServerEntity.sendChanges:211-214 sends syncVelocity to tracking players and self.
        if self.hurt_marked.load(Relaxed) {
            self.send_velocity();
        }
    }

    pub(crate) fn flush_player_motion_owned(&self) {
        #[cfg(test)]
        super::living::damage_transaction::test_hooks::reach(
            super::living::damage_transaction::test_hooks::Point::PlayerMotionFlush,
        );
        // ServerEntity.sendChanges sends needsSync to observers, syncVelocity to observers and self.
        // Entity.push is horizontal, but replaying the entire stored motion to self repeats old hurt Y.
        if self.hurt_marked.load(Relaxed) {
            self.send_velocity();
        } else if self
            .velocity_dirty
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            let world = self.world.load();
            if let Some(tracked) = world.entity_tracker.get_tracked_entity(self.entity_id) {
                tracked.send_motion(self, &world);
            }
        }
    }

    /// Adds an impulse and marks tracker synchronization without sending motion immediately.
    pub fn push_impulse(&self, impulse: Vector3<f64>) {
        // Entity.push (1960-1965), distinct from Pumpkin's immediate add_velocity/set_velocity.
        self.velocity.store(self.velocity.load() + impulse);
        self.velocity_dirty.store(true, Relaxed);
    }
}

impl Player {
    // Player.damageStatsAndHearts (1066-1075): actual health lost, after item interaction.
    pub(crate) fn damage_stats_and_hearts(&self, victim: &dyn EntityBase, damage: f32) {
        use super::player::statistics::{CustomStatistic, StatisticCategory};
        if let Some(living) = victim.get_living_entity() {
            self.increment_stat(
                StatisticCategory::Custom,
                CustomStatistic::DamageDealt as i32,
                (damage * 10.0 + 0.5).floor() as i32,
            );
            if damage > 2.0 {
                let position = living.entity.pos.load();
                self.world().broadcast_packet_all(
                    &pumpkin_protocol::java::client::play::CParticle::new(
                        false,
                        false,
                        Vector3::new(
                            position.x,
                            position.y + f64::from(living.entity.entity_type.dimension[1]) * 0.5,
                            position.z,
                        ),
                        Vector3::new(0.1, 0.0, 0.1),
                        0.2,
                        (damage * 0.5) as i32,
                        pumpkin_protocol::codec::var_int::VarInt(
                            pumpkin_data::particle::Particle::DamageIndicator.to_id() as i32,
                        ),
                        &[],
                    ),
                );
            }
        }
    }

    /// Sends the completed melee impulse to a player victim, then restores predicted server motion.
    pub fn send_hurt_motion(&self, victim: &dyn EntityBase, old_movement: Vector3<f64>) {
        // Player.causeExtraKnockback (1140-1144), including attacks with zero extra knockback.
        let _owner = victim.get_living_entity().map(LivingEntity::own_damage);
        self.send_hurt_motion_owned(victim, old_movement, false, None);
    }

    /// Sends and restores owned victim motion, then damps the attacker when requested.
    /// The token must belong to the victim; false aborts continuation after a life change.
    pub(crate) fn send_hurt_motion_owned(
        &self,
        victim: &dyn EntityBase,
        old_movement: Vector3<f64>,
        damp_attacker: bool,
        attack: Option<&DamageToken>,
    ) -> bool {
        if !attack.is_none_or(DamageToken::is_current_life) {
            return false;
        }
        let entity = victim.get_entity();
        if let Some(player) = victim.get_player()
            // Player.causeExtraKnockback only delivers syncVelocity, never needsSync.
            && entity.hurt_marked.swap(false, Relaxed)
        {
            let motion = entity.velocity.load();
            #[cfg(test)]
            super::living::damage_transaction::test_hooks::reach(
                super::living::damage_transaction::test_hooks::Point::MotionReady,
            );
            player.try_enqueue_packet_editioned(
                &CEntityVelocity::new(entity.entity_id.into(), motion),
                &CSetActorMotion {
                    target_runtime_id: VarULong(entity.entity_id as u64),
                    motion: Vector3::new(motion.x as f32, motion.y as f32, motion.z as f32),
                    tick: VarULong(0),
                },
            );
            entity.velocity.store(old_movement);
            // Pumpkin's player tick otherwise sends the restored velocity back to the predicting client.
            entity.velocity_dirty.store(false, Relaxed);
        }
        if damp_attacker {
            self.damp_attack_motion(1);
        }
        attack.is_none_or(DamageToken::is_current_life)
    }

    /// Damps attacker motion after the victim's apply/send/restore segment has completed.
    pub(crate) fn damp_attack_motion(&self, times: usize) {
        // Player.causeExtraKnockback:1135. Opposing hits must not see temporary victim motion.
        let _released = super::living::damage_transaction::suspend_damage();
        #[cfg(test)]
        super::living::damage_transaction::test_hooks::reach(
            super::living::damage_transaction::test_hooks::Point::Damping,
        );
        let _owner = self.living_entity.own_damage();
        let entity = self.get_entity();
        for _ in 0..times {
            entity
                .velocity
                .store(entity.velocity.load().multiply(0.6, 1.0, 0.6));
        }
        self.living_entity.set_sprinting(false);
    }

    pub(crate) fn brake_mace_motion(&self) {
        // MaceItem.hurtEnemy:55-58, after any opposing melee's send/restore segment.
        let _released = super::living::damage_transaction::suspend_damage();
        let _owner = self.living_entity.own_damage();
        let velocity = self.get_entity().velocity.load();
        self.living_entity.protect_mace_landing();
        self.set_velocity(Vector3::new(velocity.x, f64::from(0.01f32), velocity.z));
    }
}

impl LivingEntity {
    #[cfg(test)]
    pub(crate) fn flush_player_motion(&self) {
        let _owner = self.own_damage();
        self.entity.flush_player_motion_owned();
    }
}
