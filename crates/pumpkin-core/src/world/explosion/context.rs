use super::{Explosion, ExplosionInteraction, World};
use crate::entity::{EntityBase, tnt::TNTEntity};
use pumpkin_data::damage::DamageType;
use pumpkin_util::math::vector3::Vector3;
use std::sync::Arc;

impl Explosion {
    /// Retains the direct entity and resolves its living owner, like Explosion.getIndirectSourceEntity.
    #[must_use]
    pub fn with_source(mut self, source: Option<Arc<dyn EntityBase>>) -> Self {
        self.cause = source.as_ref().and_then(|source| {
            if source.get_living_entity().is_some() {
                Some(source.clone())
            } else if let Some(tnt) = source.cast_any().downcast_ref::<TNTEntity>() {
                tnt.owner()
            } else {
                source
                    .projectile_owner()
                    .filter(|owner| owner.get_living_entity().is_some())
            }
        });
        if self.damage_calculator.is_none()
            && let Some(source) = &source
        {
            self.damage_calculator = Some(Arc::new(super::EntityBasedExplosionDamageCalculator {
                source: source.clone(),
            }));
        }
        self.source = source;
        self
    }

    /// Sets custom damage attribution without changing chained TNT ownership or block XP.
    #[must_use]
    pub fn with_cause(mut self, cause: Option<Arc<dyn EntityBase>>) -> Self {
        // EndCrystal.hurtServer / MinecartTNT.explode supply a custom DamageSource.
        self.custom_cause = Some(super::ExplosionDamageSource { cause });
        self
    }

    /// Overrides the damage identity (bed/anchor or enchantment) without changing attribution.
    #[must_use]
    pub const fn with_damage_type(mut self, damage_type: DamageType) -> Self {
        self.damage_type = Some(damage_type);
        self
    }

    #[must_use]
    pub const fn with_fire(mut self, fire: bool) -> Self {
        self.fire = fire;
        self
    }

    /// Returns the living entity credited for this explosion and chained TNT.
    #[must_use]
    pub fn indirect_source(&self) -> Option<Arc<dyn EntityBase>> {
        self.cause.clone()
    }

    pub(crate) fn play_sound(&self) -> bool {
        self.source.as_ref().is_none_or(|source| {
            !source
                .get_entity()
                .silent
                .load(std::sync::atomic::Ordering::Relaxed)
        })
    }

    pub(crate) fn packet(
        &self,
        count: u32,
        knockback: Option<Vector3<f64>>,
    ) -> pumpkin_protocol::java::client::play::CExplosion {
        use pumpkin_protocol::{
            IdOr, SoundEvent,
            codec::var_int::VarInt,
            java::client::play::{CExplosion, ExplosionParticleInfo},
        };
        let particle = if self.is_small() {
            self.small_particle
        } else {
            self.large_particle
        };
        let mut packet = CExplosion::new(
            self.pos,
            self.power,
            count as i32,
            knockback,
            VarInt(particle as i32),
            IdOr::<SoundEvent>::Id(self.sound as u16),
        );
        packet.play_sound = self.play_sound();
        // Level.DEFAULT_EXPLOSION_BLOCK_PARTICLES; wind bursts supply an empty pool.
        if self.sound == pumpkin_data::sound::Sound::EntityGenericExplode {
            packet.block_particles = vec![
                ExplosionParticleInfo {
                    particle: VarInt(pumpkin_data::particle::Particle::Poof as i32),
                    scaling: 0.5,
                    speed: 1.0,
                    weight: VarInt(1),
                },
                ExplosionParticleInfo {
                    particle: VarInt(pumpkin_data::particle::Particle::Smoke as i32),
                    scaling: 1.0,
                    speed: 1.0,
                    weight: VarInt(1),
                },
            ];
        }
        packet
    }

    pub(crate) fn source_id(&self) -> i32 {
        self.source
            .as_ref()
            .map_or(0, |source| source.get_entity().entity_id)
    }

    pub(crate) const fn is_small(&self) -> bool {
        self.power < 2.0 || matches!(self.block_interaction, super::BlockInteraction::Keep)
    }

    pub(super) fn teams_allow_damage(&self, victim: &dyn EntityBase) -> bool {
        // Entity.doTeamsAllowDamage uses the direct source's team, independently of knockback.
        const TEAM_OPTION_FRIENDLY_FIRE: i8 = 0x01;
        self.source
            .as_ref()
            .and_then(|source| source.get_team())
            .is_none_or(|team| {
                victim
                    .get_team()
                    .is_none_or(|other| team.name != other.name)
                    || team.options & TEAM_OPTION_FRIENDLY_FIRE != 0
            })
    }

    pub(super) fn hurt_from_explosion(&self, victim: &dyn EntityBase, damage: f32) {
        let cause = self.damage_cause();
        // DamageSources.explosion uses PLAYER_EXPLOSION for any attributed living cause.
        let damage_type = self
            .damage_type
            .unwrap_or(if self.source.is_some() && cause.is_some() {
                DamageType::PLAYER_EXPLOSION
            } else {
                DamageType::EXPLOSION
            });
        let position = (damage_type == DamageType::BAD_RESPAWN_POINT).then_some(self.pos);
        victim.damage_with_context(
            victim,
            damage,
            damage_type,
            position,
            self.source.as_deref(),
            cause,
        );
    }

    // ServerExplosion.hurtEntities uses DamageSource.getEntity for damage and redirection.
    pub(super) fn damage_cause(&self) -> Option<&dyn EntityBase> {
        self.custom_cause
            .as_ref()
            .map_or(self.cause.as_deref(), |source| source.cause.as_deref())
    }
}

impl World {
    /// Explodes with a direct source retained through removal and TNT chain reactions.
    pub fn explode_from(
        self: &Arc<Self>,
        source: &dyn EntityBase,
        position: Vector3<f64>,
        power: f32,
        interaction: ExplosionInteraction,
        fire: bool,
    ) {
        let explosion = Explosion::new(power, position, self.get_block_interaction(interaction))
            .with_source(self.get_entity_by_id(source.get_entity().entity_id))
            .with_fire(fire);
        self.run_explosion(&explosion);
    }

    /// `BedBlock.destroyOnUse` and `RespawnAnchorBlock.explode` share the bad-respawn damage source.
    pub fn explode_bad_respawn(self: &Arc<Self>, position: Vector3<f64>) {
        let explosion = Explosion::new(
            5.0,
            position,
            self.get_block_interaction(ExplosionInteraction::Block),
        )
        .with_damage_type(DamageType::BAD_RESPAWN_POINT)
        .with_fire(true);
        self.run_explosion(&explosion);
    }
    /// `RespawnAnchorBlock.explode`: adjacent flowing water can protect the removed origin block.
    pub fn explode_respawn_anchor(
        self: &Arc<Self>,
        origin: pumpkin_util::math::position::BlockPos,
    ) {
        use pumpkin_data::{
            BlockDirection,
            tag::{self, Taggable},
        };
        let is_water = |pos: &pumpkin_util::math::position::BlockPos| {
            self.get_fluid(pos).has_tag(&tag::Fluid::MINECRAFT_WATER)
        };
        let in_water = is_water(&origin.up())
            || [
                BlockDirection::North,
                BlockDirection::South,
                BlockDirection::East,
                BlockDirection::West,
            ]
            .into_iter()
            .any(|direction| {
                let pos = origin.offset(direction.to_offset());
                let (fluid, state) = self.get_fluid_and_fluid_state(&pos);
                fluid.has_tag(&tag::Fluid::MINECRAFT_WATER)
                    && (state.is_source || state.level >= 2 && !is_water(&pos.down()))
            });
        let explosion = Explosion::new(
            5.0,
            origin.to_centered_f64(),
            self.get_block_interaction(ExplosionInteraction::Block),
        )
        .with_damage_type(DamageType::BAD_RESPAWN_POINT)
        .with_fire(true)
        .with_damage_calculator(Arc::new(
            super::calculator::RespawnAnchorDamageCalculator { origin, in_water },
        ));
        self.run_explosion(&explosion);
    }
}
