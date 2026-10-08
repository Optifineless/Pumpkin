use super::LivingEntity;
use crate::entity::EntityBase;
use crate::entity::combat::knockback_after_resistance;
use crate::entity::player::statistics::{CustomStatistic, StatisticCategory};
use pumpkin_data::attributes::Attributes;
use pumpkin_data::damage::DamageType;
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::entity::EntityType;
use pumpkin_data::sound::SoundCategory;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_protocol::bedrock::server::actor_event::{ActorEventID, SActorEvent};
use pumpkin_protocol::codec::{var_int::VarInt, var_ulong::VarULong};
use pumpkin_protocol::java::client::play::CHurtAnimation;
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use std::sync::atomic::Ordering::Relaxed;

impl LivingEntity {
    // LivingEntity.hurtServer: blocking, cooldown, actuallyHurt, credit, then death protection.
    pub(super) fn hurt_server(
        &self,
        caller: &dyn EntityBase,
        amount: f32,
        damage_type: DamageType,
        position: Option<Vector3<f64>>,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        if self.entity.is_invulnerable_to(&damage_type, cause)
            || self.health.load() <= 0.0
            || self.dead.load(Relaxed)
            || amount < 0.0
        {
            return false;
        }
        let Some(mut amount) =
            self.damage_after_plugin_events(amount, damage_type, position, source, cause)
        else {
            return false;
        };
        let world = self.entity.world.load();
        if damage_type.has_tag(&tag::DamageType::MINECRAFT_IS_FIRE)
            && ((self.entity.entity_type == &EntityType::PLAYER
                && !world.level_info.load().game_rules.fire_damage)
                || (self.has_effect(&StatusEffect::FIRE_RESISTANCE)
                    && !damage_type.has_tag(&tag::DamageType::MINECRAFT_BYPASSES_EFFECTS)))
        {
            return false;
        }
        if let Some(player) = caller.get_player()
            && player.sleeping_since.load().is_some()
        {
            player.wake_up();
        }

        // LivingEntity.hurtServer resets inactivity before blocking and cooldown rejection.
        self.no_action_time.store(0, Relaxed);

        let blocking_item = self.get_item_blocking_with();
        let blocked = self.apply_item_blocking(caller, &damage_type, amount, position, source);
        amount -= blocked;
        if damage_type == DamageType::FREEZE
            && self
                .entity
                .entity_type
                .has_tag(&tag::EntityType::MINECRAFT_FREEZE_HURTS_EXTRA_TYPES)
        {
            amount *= 5.0;
        }
        let Some((damage_amount, took_full_damage)) =
            self.damage_after_cooldown(amount, &damage_type)
        else {
            return false;
        };
        let Some(server) = world.server.upgrade() else {
            return false;
        };
        self.actually_hurt(caller, damage_amount, damage_type, source, cause);
        // Fully blocked admitted hits still resolve credit exactly once.
        self.record_hurt_by(damage_type, source, cause);
        let success = blocked <= 0.0 || amount > 0.0;
        if took_full_damage {
            if let Some(item) = blocking_item.as_ref().filter(|_| blocked > 0.0) {
                self.on_item_blocked(caller, item);
            } else {
                self.hurt_feedback(
                    damage_type,
                    position,
                    source,
                    cause,
                    server.advanced_config.pvp.hurt_animation,
                );
            }
            if success
                && self.health.load() > 0.0
                && let Some(source) = source
            {
                let source_pos = source.get_entity().pos.load();
                let target_pos = self.entity.pos.load();
                let resistance = self.get_attribute_value(&Attributes::KNOCKBACK_RESISTANCE);
                self.entity.apply_knockback(
                    knockback_after_resistance(0.4, resistance),
                    source_pos.x - target_pos.x,
                    source_pos.z - target_pos.z,
                );
            }
        }
        self.finish_hurt(caller, damage_type, source, cause, took_full_damage);
        if success {
            *self
                .last_damage_type
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(damage_type);
            self.last_damage_stamp.store(world.get_world_age(), Relaxed);
        }
        if blocked > 0.0
            && let Some(player) = caller.get_player()
        {
            player.increment_stat(
                StatisticCategory::Custom,
                CustomStatistic::DamageBlockedByShield as i32,
                (blocked * 10.0).round() as i32,
            );
        }
        success
    }

    fn damage_after_plugin_events(
        &self,
        mut amount: f32,
        damage_type: DamageType,
        position: Option<Vector3<f64>>,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> Option<f32> {
        let mut damage_event =
            crate::plugin::api::events::entity::entity_damage::EntityDamageEvent::new(
                self.entity.entity_id,
                damage_type,
                amount,
            );
        if let Some(server) = self.entity.world.load().server.upgrade() {
            server
                .plugin_manager
                .fire_blocking(&server, &mut damage_event);
        }
        if damage_event.cancelled {
            return None;
        }
        amount = damage_event.damage;

        if let Some(damager) = source.or(cause) {
            let mut by_entity_event =
                crate::plugin::api::events::entity::entity_damage_by_entity::EntityDamageByEntityEvent {
                    entity_id: self.entity.entity_id,
                    damager_id: damager.get_entity().entity_id,
                    damage: amount,
                    cause: format!("{damage_type:?}"),
                    cancelled: false,
                };
            if let Some(server) = self.entity.world.load().server.upgrade() {
                server
                    .plugin_manager
                    .fire_blocking(&server, &mut by_entity_event);
            }
            if by_entity_event.cancelled {
                return None;
            }
            amount = by_entity_event.damage;
        } else if position.is_some()
            || matches!(
                damage_type,
                DamageType::CACTUS
                    | DamageType::SWEET_BERRY_BUSH
                    | DamageType::CAMPFIRE
                    | DamageType::HOT_FLOOR
                    | DamageType::STALAGMITE
            )
        {
            let damager_pos = position.map(|p| {
                BlockPos(Vector3::new(
                    p.x.floor() as i32,
                    p.y.floor() as i32,
                    p.z.floor() as i32,
                ))
            });
            let mut by_block_event =
                crate::plugin::api::events::entity::entity_damage_by_block::EntityDamageByBlockEvent {
                    entity_id: self.entity.entity_id,
                    damager_pos,
                    damage: amount,
                    cause: format!("{damage_type:?}"),
                    cancelled: false,
                };
            if let Some(server) = self.entity.world.load().server.upgrade() {
                server
                    .plugin_manager
                    .fire_blocking(&server, &mut by_block_event);
            }
            if by_block_event.cancelled {
                return None;
            }
            amount = by_block_event.damage;
        }

        Some(amount)
    }

    // LivingEntity.hurtServer sends damage feedback only on the full cooldown path.
    fn hurt_feedback(
        &self,
        damage_type: DamageType,
        position: Option<Vector3<f64>>,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
        hurt_animation: bool,
    ) {
        let world = self.entity.world.load();
        if hurt_animation {
            let entity_id = self.entity.entity_id;
            let hurt_yaw = source.map_or(0.0, |source| {
                let src = source.get_entity().pos.load();
                let tgt = self.entity.pos.load();
                (src.z - tgt.z).atan2(src.x - tgt.x).to_degrees() as f32 - self.entity.yaw.load()
            });
            let hurt_event = SActorEvent {
                target_runtime_id: VarULong(entity_id as u64),
                event_id: ActorEventID::Hurt,
                data: VarInt(0),
                fire_at_position: None,
            };
            let hurt_animation = CHurtAnimation::new(entity_id.into(), hurt_yaw);
            world.send_to_tracking_players_and_self_editioned(
                &self.entity,
                &hurt_animation,
                &hurt_event,
            );
        }
        world.broadcast_damage_event(
            &self.entity,
            i32::from(damage_type.id),
            cause.map(|e| e.get_entity().entity_id),
            source.map(|e| e.get_entity().entity_id),
            position,
        );
    }

    // LivingEntity.hurtServer checks checkTotemDeathProtection before die and all death events.
    fn finish_hurt(
        &self,
        caller: &dyn EntityBase,
        damage_type: DamageType,
        source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
        took_full_damage: bool,
    ) {
        let world = self.entity.world.load();
        if self.health.load() <= 0.0 {
            if self.try_use_death_protector(caller, &damage_type) {
                return;
            }
            let mut death_event =
                crate::plugin::api::events::entity::entity_death::EntityDeathEvent::new(
                    self.entity.entity_id,
                    0,
                );
            if let Some(server) = world.server.upgrade() {
                server
                    .plugin_manager
                    .fire_blocking(&server, &mut death_event);
            }
            self.on_death(damage_type, source, cause);
        } else if took_full_damage {
            world.play_sound_fine(
                self.hurt_sound(caller),
                SoundCategory::Players,
                &self.entity.pos.load(),
                1.0,
                self.get_pitch(),
            );
        }
    }
}
