use super::{ProjectileHit, ThrownItemEntity, ownership::ProjectileState};
use crate::{
    entity::{Entity, EntityBase, area_effect_cloud::AreaEffectCloudEntity},
    server::Server,
};
use pumpkin_data::entity::EntityType;
use pumpkin_protocol::java::client::play::CWorldEvent;
use pumpkin_util::math::vector3::Vector3;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct DragonFireballEntity {
    pub thrown: ThrownItemEntity,
}

impl DragonFireballEntity {
    const SPLASH_RANGE: f64 = 4.0;

    pub const fn new(entity: Entity) -> Self {
        Self {
            thrown: ThrownItemEntity {
                entity,
                projectile: ProjectileState::new(None),
                has_hit: AtomicBool::new(false),
                gravity: 0.0,
            },
        }
    }

    pub fn new_shot(entity: Entity, owner: &Entity, direction: Vector3<f64>) -> Self {
        // AbstractHurtingProjectile.assignDirectionalMovement.
        entity
            .velocity
            .store(direction.normalize() * super::fireball::INITIAL_ACCELERATION_POWER);
        let fireball = Self::new(entity);
        fireball.thrown.projectile.set_owner(Some(owner));
        fireball
    }
}

impl EntityBase for DragonFireballEntity {
    fn get_entity(&self) -> &Entity {
        &self.thrown.entity
    }
    fn get_living_entity(&self) -> Option<&crate::entity::living::LivingEntity> {
        None
    }
    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
    fn projectile_state(&self) -> Option<&ProjectileState> {
        Some(&self.thrown.projectile)
    }
    fn tick(&self, caller: &dyn EntityBase, server: &Server) {
        self.thrown.process_tick(caller, server);
        // AbstractHurtingProjectile.createParticleTrail / DragonFireball.getTrailParticle.
        let entity = self.get_entity();
        entity.world.load().broadcast_to_chunk(
            entity.chunk_pos.load(),
            &pumpkin_protocol::java::client::play::CParticle::new(
                false,
                false,
                entity.pos.load() + Vector3::new(0.0, 0.5, 0.0),
                Vector3::default(),
                0.0,
                1,
                pumpkin_protocol::codec::var_int::VarInt(
                    pumpkin_data::particle::Particle::DragonBreath as i32,
                ),
                &1.0f32.to_be_bytes(),
            ),
        );
    }

    fn on_hit(&self, hit: ProjectileHit) {
        // DragonFireball.onHit creates the cloud only at impact, then relocates to nearby living entities.
        if let ProjectileHit::Entity { entity, .. } = &hit
            && self.thrown.projectile.owned_by(entity.get_entity())
        {
            return;
        }
        let entity = self.get_entity();
        let world = entity.world.load();
        let owner = self.projectile_owner();
        let cloud = AreaEffectCloudEntity::dragon_cloud(
            Entity::new(
                world.clone(),
                entity.pos.load(),
                &EntityType::AREA_EFFECT_CLOUD,
            ),
            owner.as_deref().unwrap_or(self),
            true,
        );
        cloud.set_owner(owner.as_deref());
        for target in world.get_all_at_box(&entity.bounding_box.load().expand(
            Self::SPLASH_RANGE,
            2.0,
            Self::SPLASH_RANGE,
        )) {
            if target.get_living_entity().is_some()
                && entity
                    .pos
                    .load()
                    .squared_distance_to_vec(&target.get_entity().pos.load())
                    < Self::SPLASH_RANGE * Self::SPLASH_RANGE
            {
                cloud.get_entity().set_pos(target.get_entity().pos.load());
                break;
            }
        }
        world.broadcast_to_chunk(
            entity.chunk_pos.load(),
            &CWorldEvent::new(
                2006,
                entity.block_pos.load(),
                if entity.silent.load(Ordering::Relaxed) {
                    -1
                } else {
                    1
                },
                false,
            ),
        );
        world.spawn_entity(cloud);
        entity.remove();
    }
}
