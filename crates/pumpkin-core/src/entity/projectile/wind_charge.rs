use pumpkin_data::damage::DamageType;
use pumpkin_data::particle::Particle;
use pumpkin_data::sound::Sound;
use pumpkin_data::tag;
use pumpkin_util::math::vector3::Vector3;
use std::sync::LazyLock;
use std::{
    f64,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};

use crate::{
    entity::{
        Entity, EntityBase,
        living::LivingEntity,
        projectile::{ProjectileHit, ThrownItemEntity},
        projectile_deflection::ProjectileDeflectionType,
    },
    server::Server,
    world::{BlockInteraction, Explosion, SimpleExplosionDamageCalculator},
};

const DEFAULT_DEFLECT_COOLDOWN: u8 = 5;
const JUMP_SCALE: f64 = 0.25;
pub const WIND_CHARGE_GRAVITY: f64 = 0.0;

enum WindChargeKind {
    Normal { deflect_cooldown: AtomicU8 },
    Breeze,
}

pub struct WindChargeEntity {
    kind: WindChargeKind,
    thrown_item_entity: ThrownItemEntity,
}

pub static WIND_CHARGE_EXPLOSION_DAMAGE_CALCULATOR: LazyLock<Arc<SimpleExplosionDamageCalculator>> =
    LazyLock::new(|| {
        Arc::new(SimpleExplosionDamageCalculator::new(
            false,
            true,
            Some(1.22),
            Some(&tag::Block::MINECRAFT_BLOCKS_WIND_CHARGE_EXPLOSIONS),
        ))
    });

pub static BREEZE_WIND_CHARGE_EXPLOSION_DAMAGE_CALCULATOR: LazyLock<
    Arc<SimpleExplosionDamageCalculator>,
> = LazyLock::new(|| {
    Arc::new(SimpleExplosionDamageCalculator::new(
        false,
        true,
        None,
        Some(&tag::Block::MINECRAFT_BLOCKS_WIND_CHARGE_EXPLOSIONS),
    ))
});

impl WindChargeEntity {
    #[must_use]
    pub fn new_normal(thrown_item_entity: ThrownItemEntity) -> Self {
        thrown_item_entity.projectile.set_acceleration_power(0.0);
        Self {
            kind: WindChargeKind::Normal {
                deflect_cooldown: AtomicU8::new(DEFAULT_DEFLECT_COOLDOWN),
            },
            thrown_item_entity,
        }
    }

    #[must_use]
    pub fn new_breeze(thrown_item_entity: ThrownItemEntity) -> Self {
        thrown_item_entity.projectile.set_acceleration_power(0.0);
        Self {
            kind: WindChargeKind::Breeze,
            thrown_item_entity,
        }
    }

    pub const fn deflect_cooldown(&self) -> Option<&AtomicU8> {
        if let WindChargeKind::Normal {
            deflect_cooldown, ..
        } = &self.kind
        {
            Some(deflect_cooldown)
        } else {
            None
        }
    }

    pub fn create_explosion(&self, position: Vector3<f64>) {
        let (power, calculator, sound) = match self.kind {
            WindChargeKind::Normal { .. } => (
                1.2,
                WIND_CHARGE_EXPLOSION_DAMAGE_CALCULATOR.clone(),
                Sound::EntityWindChargeWindBurst,
            ),
            WindChargeKind::Breeze => (
                3.0,
                BREEZE_WIND_CHARGE_EXPLOSION_DAMAGE_CALCULATOR.clone(),
                Sound::EntityBreezeWindBurst,
            ),
        };
        let world = self.get_entity().world.load();
        let explosion = Explosion::new(power, position, BlockInteraction::TriggerBlock)
            .with_source(world.get_entity_by_id(self.get_entity().entity_id))
            .with_damage_calculator(calculator)
            .with_particles_and_sound(
                Particle::GustEmitterSmall,
                Particle::GustEmitterLarge,
                sound,
            );
        self.get_entity().world.load().run_explosion(&explosion);
    }

    pub fn deflect(
        &self,
        deflection: &ProjectileDeflectionType,
        deflector: Option<&dyn EntityBase>,
    ) -> bool {
        if let Some(cooldown) = self.deflect_cooldown()
            && cooldown.load(Ordering::Relaxed) > 0
        {
            return false;
        }

        super::deflection::deflect(
            self,
            *deflection,
            deflector,
            deflector,
            true,
            Vector3::new(1.0, 1.0, 1.0),
        )
    }
}

impl EntityBase for WindChargeEntity {
    fn can_hit(&self) -> bool {
        !self.get_entity().is_removed()
    }

    fn projectile_state(&self) -> Option<&super::ownership::ProjectileState> {
        Some(&self.thrown_item_entity.projectile)
    }

    fn tick(&self, caller: &dyn EntityBase, server: &Server) {
        self.thrown_item_entity.process_tick(caller, server);

        if let Some(cooldown) = self.deflect_cooldown() {
            let cooldown_ticks = cooldown.load(Ordering::Relaxed);
            if cooldown_ticks > 0 {
                cooldown.store(cooldown_ticks - 1, Ordering::Relaxed);
            }
        }
    }

    fn get_entity(&self) -> &Entity {
        &self.thrown_item_entity.entity
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }

    fn on_hit(&self, hit: ProjectileHit) {
        let hit_pos = hit.hit_pos();
        if let ProjectileHit::Entity { ref entity, .. } = hit {
            let owner = self.projectile_owner();
            if let Some(living) = owner.as_deref().and_then(EntityBase::get_living_entity) {
                living.set_last_hurt_mob(entity.as_ref());
            }

            let damaged = super::damage::hurt_entity(
                entity.as_ref(),
                1.0,
                DamageType::WIND_CHARGE,
                self,
                owner
                    .as_deref()
                    .filter(|owner| owner.get_living_entity().is_some()),
            );
            if damaged && entity.get_living_entity().is_some() {
                super::damage::post_attack(
                    entity.as_ref(),
                    DamageType::WIND_CHARGE,
                    self,
                    owner.as_deref(),
                );
            }
        }
        let explosion_pos = if let ProjectileHit::Block { face, .. } = hit {
            hit_pos + face.to_offset().to_f64() * JUMP_SCALE
        } else {
            hit_pos
        };
        self.create_explosion(explosion_pos);
    }
}
