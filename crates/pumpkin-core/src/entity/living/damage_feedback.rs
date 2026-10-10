use super::{LivingEntity, damage::HurtContext};
use crate::entity::{EntityBase, projectile::is_projectile};
use pumpkin_data::{
    damage::{DamageEffects, DamageType},
    entity::EntityType,
    item_stack::ItemStack,
    sound::{Sound, SoundCategory},
    tag::{self, Taggable},
};
use pumpkin_protocol::{
    bedrock::server::actor_event::{ActorEventID, SActorEvent},
    codec::{var_int::VarInt, var_ulong::VarULong},
    java::client::play::CHurtAnimation,
};
use pumpkin_util::math::vector3::Vector3;
use std::sync::atomic::Ordering::Relaxed;

#[cfg(test)]
mod tests;

impl LivingEntity {
    // LivingEntity.hurtServer (1239-1252): any blocked portion selects BlocksAttacks.onBlocked.
    pub(super) fn full_hit_feedback(
        &self,
        caller: &dyn EntityBase,
        context: HurtContext<'_>,
        blocked: f32,
        damage: f32,
        blocking_item: Option<&ItemStack>,
    ) {
        let HurtContext {
            damage_type,
            position,
            source,
            cause,
            ..
        } = context;
        let world = self.entity.world.load();
        if let Some(item) = blocking_item.filter(|_| blocked > 0.0) {
            self.on_item_blocked(caller, item);
        } else {
            world.broadcast_damage_event(
                &self.entity,
                i32::from(damage_type.id),
                cause.map(|e| e.get_entity().entity_id),
                source.map(|e| e.get_entity().entity_id),
                position,
            );
            let hurt_event = SActorEvent {
                target_runtime_id: VarULong(self.entity.entity_id as u64),
                event_id: ActorEventID::Hurt,
                data: VarInt(0),
                fire_at_position: None,
            };
            world.send_to_tracking_players_bedrock(&self.entity, &hurt_event);
            if let Some(player) = caller.get_player()
                && matches!(
                    player.client.as_ref(),
                    crate::net::ClientPlatform::Bedrock(_)
                )
            {
                player.try_enqueue_packet_editioned(
                    &CHurtAnimation::new(self.entity.entity_id.into(), 0.0),
                    &hurt_event,
                );
            }
        }
        // Pumpkin's Java explosion packet carries client-applied motion; a server-motion replay
        // would override that launch. Task 4's explosion path owns that impulse.
        let client_applies_explosion_motion = damage_type
            .has_tag(&tag::DamageType::MINECRAFT_IS_EXPLOSION)
            && caller.get_player().is_some_and(|player| {
                matches!(player.client.as_ref(), crate::net::ClientPlatform::Java(_))
            });
        if !client_applies_explosion_motion
            && !damage_type.has_tag(&tag::DamageType::MINECRAFT_NO_IMPACT)
            && (blocked <= 0.0 || damage > 0.0)
        {
            self.mark_hurt();
        }
        if !damage_type.has_tag(&tag::DamageType::MINECRAFT_NO_KNOCKBACK)
            && (blocked <= 0.0 || damage > 0.0)
        {
            let direction = self.default_knockback_direction(position, source);
            self.knockback(f64::from(0.4f32), direction.x, direction.z);
            if blocked <= 0.0
                && let Some(player) = caller.get_player()
                && world
                    .server
                    .upgrade()
                    .is_none_or(|server| server.advanced_config.pvp.hurt_animation)
            {
                // ServerPlayer.indicateDamage (2169-2171): only the victim receives direction.
                let hurt_dir =
                    direction.z.atan2(direction.x).to_degrees() as f32 - self.entity.yaw.load();
                player.try_send_client_packet(&CHurtAnimation::new(
                    self.entity.entity_id.into(),
                    hurt_dir,
                ));
            }
        }
    }

    // LivingEntity.dealDefaultKnockback and Projectile.calculateHorizontalHurtKnockbackDirection.
    fn default_knockback_direction(
        &self,
        position: Option<Vector3<f64>>,
        source: Option<&dyn EntityBase>,
    ) -> Vector3<f64> {
        if let Some(source) = source.filter(|source| {
            let kind = source.get_entity().entity_type;
            is_projectile(kind)
                || kind.has_tag(&tag::EntityType::MINECRAFT_ARROWS)
                || kind == &EntityType::BREEZE_WIND_CHARGE
        }) {
            let entity = source.get_entity();
            // FireworkRocketEntity:316 / AbstractThrownPotion:138 override travel direction.
            if entity.entity_type == &EntityType::FIREWORK_ROCKET
                || entity.entity_type == &EntityType::SPLASH_POTION
                || entity.entity_type == &EntityType::LINGERING_POTION
            {
                return entity.pos.load() - self.entity.pos.load();
            }
            let motion = entity.velocity.load();
            return Vector3::new(-motion.x, 0.0, -motion.z);
        }
        position
            .or_else(|| source.map(|entity| entity.get_entity().pos.load()))
            .map_or(Vector3::default(), |position| {
                position - self.entity.pos.load()
            })
    }

    // LivingEntity.hurtServer (1255-1267): totem before death sounds and die; no sound on excess.
    pub(super) fn finish_hurt(
        &self,
        caller: &dyn EntityBase,
        damage_type: DamageType,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
        full_hit: bool,
    ) {
        let lifecycle = self.damage_owner.lifecycle();
        if self.dead.load(Relaxed) {
            return;
        }
        if self.health.load() <= 0.0 {
            if self.try_use_death_protector(caller, &damage_type) {
                return;
            }
            if self.damage_owner.lifecycle() != lifecycle {
                return;
            }
            if full_hit {
                // Player.getDeathSound overrides the generic entity-data fallback.
                let sound = if caller.get_player().is_some() {
                    Sound::EntityPlayerDeath
                } else {
                    self.death_sound(caller)
                };
                self.make_damage_sound(caller, sound);
                self.play_secondary_hurt_sound(caller, damage_type);
            }
            let mut death_event =
                crate::plugin::api::events::entity::entity_death::EntityDeathEvent::new(
                    self.entity.entity_id,
                    0,
                );
            if let Some(server) = self.entity.world.load().server.upgrade() {
                server
                    .plugin_manager
                    .fire_blocking(&server, &mut death_event);
            }
            // A death callback can resurrect or reset this entity on another thread.
            if self.damage_owner.lifecycle() == lifecycle
                && self.health.load() <= 0.0
                && !self.dead.load(Relaxed)
            {
                self.on_death(damage_type, source, cause);
            }
        } else if full_hit {
            self.make_damage_sound(
                caller,
                if caller.get_player().is_some() {
                    player_hurt_sound(damage_type.effects)
                } else {
                    self.hurt_sound(caller)
                },
            );
            self.play_secondary_hurt_sound(caller, damage_type);
        }
    }

    fn make_damage_sound(&self, caller: &dyn EntityBase, sound: Sound) {
        if let Some(player) = caller.get_player() {
            // Player.playSound excludes itself and bypasses Entity.playSound's Silent guard.
            self.entity.world.load().play_sound_expect(
                player,
                sound,
                SoundCategory::Players,
                &self.entity.pos.load(),
            );
        } else if !self.entity.is_silent() {
            self.entity.world.load().play_sound_fine(
                sound,
                self.item_effect_sound_category(caller),
                &self.entity.pos.load(),
                1.0,
                self.get_pitch(),
            );
        }
    }

    fn play_secondary_hurt_sound(&self, caller: &dyn EntityBase, damage_type: DamageType) {
        if damage_type == DamageType::THORNS {
            self.entity.world.load().play_sound(
                Sound::EnchantThornsHit,
                if caller.get_player().is_some() {
                    SoundCategory::Players
                } else {
                    SoundCategory::Hostile
                },
                &self.entity.pos.load(),
            );
        }
    }
}

// Player.getHurtSound / DamageEffects.sound: these Java enum constants are sounds, not item data.
const fn player_hurt_sound(effects: Option<DamageEffects>) -> Sound {
    match effects {
        Some(DamageEffects::Drowning) => Sound::EntityPlayerHurtDrown,
        Some(DamageEffects::Burning) => Sound::EntityPlayerHurtOnFire,
        Some(DamageEffects::Poking) => Sound::EntityPlayerHurtSweetBerryBush,
        Some(DamageEffects::Freezing) => Sound::EntityPlayerHurtFreeze,
        _ => Sound::EntityPlayerHurt,
    }
}
