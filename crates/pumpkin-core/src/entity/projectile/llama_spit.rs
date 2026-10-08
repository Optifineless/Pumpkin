use std::sync::atomic::AtomicBool;

use pumpkin_data::damage::DamageType;
use pumpkin_util::math::vector3::Vector3;

use crate::{
    entity::{
        Entity, EntityBase,
        living::LivingEntity,
        projectile::{ProjectileHit, ThrownItemEntity},
    },
    server::Server,
};

pub const LLAMA_SPIT_GRAVITY: f64 = 0.06;

pub struct LlamaSpitEntity {
    pub thrown: ThrownItemEntity,
}

impl LlamaSpitEntity {
    #[must_use]
    pub const fn new(entity: Entity) -> Self {
        let thrown = ThrownItemEntity {
            entity,
            projectile: crate::entity::projectile::ownership::ProjectileState::new(None),
            has_hit: AtomicBool::new(false),
            gravity: LLAMA_SPIT_GRAVITY,
        };

        Self { thrown }
    }

    #[must_use]
    pub fn new_shot(entity: Entity, shooter: &Entity) -> Self {
        let owner_pos = shooter.pos.load();
        let body_yaw_rad = f64::from(shooter.body_yaw.load()).to_radians();
        let bb_width = f64::from(shooter.entity_dimension.load().width);
        let offset = f64::midpoint(bb_width, 1.0);
        let x = owner_pos.x - offset * body_yaw_rad.sin();
        let y = owner_pos.y + shooter.get_eye_height() - 0.1;
        let z = owner_pos.z + offset * body_yaw_rad.cos();
        entity.pos.store(Vector3::new(x, y, z));

        let thrown = ThrownItemEntity {
            entity,
            projectile: super::ownership::ProjectileState::new(Some(shooter.entity_uuid)),
            has_hit: AtomicBool::new(false),
            gravity: LLAMA_SPIT_GRAVITY,
        };

        Self { thrown }
    }
}

impl EntityBase for LlamaSpitEntity {
    fn projectile_state(&self) -> Option<&super::ownership::ProjectileState> {
        Some(&self.thrown.projectile)
    }

    fn tick(&self, caller: &dyn EntityBase, server: &Server) {
        // LlamaSpit.tick calls Projectile.tick first, hits using original motion, then drag/gravity.
        let entity = self.get_entity();
        self.thrown.projectile.tick(entity);
        EntityBase::tick(entity, caller, server);
        let movement = entity.velocity.load();
        let start = entity.pos.load();
        if let Some(hit) = self.thrown.find_hit(caller, start, movement) {
            self.thrown.hit_target(caller, hit);
        }
        if !entity.is_alive() {
            return;
        }
        let bounds = entity.bounding_box.load();
        let world = entity.world.load();
        let touches_no_air = (bounds.min.x.floor() as i32..=bounds.max.x.floor() as i32).all(|x| {
            (bounds.min.y.floor() as i32..=bounds.max.y.floor() as i32).all(|y| {
                (bounds.min.z.floor() as i32..=bounds.max.z.floor() as i32).all(|z| {
                    !world
                        .get_block_state(&pumpkin_util::math::position::BlockPos::new(x, y, z))
                        .is_air()
                })
            })
        });
        if touches_no_air || entity.is_in_water() {
            entity.remove();
            return;
        }
        let mut velocity = movement * f64::from(0.99f32);
        if !entity.has_no_gravity() {
            velocity.y -= LLAMA_SPIT_GRAVITY;
        }
        entity.velocity.store(velocity);
        super::arrow::update_flight_rotation(entity, movement, true);
        entity.set_pos(start + movement);
    }

    fn get_entity(&self) -> &Entity {
        self.thrown.get_entity()
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }

    fn on_hit(&self, hit: ProjectileHit) {
        if matches!(hit, ProjectileHit::Block { .. }) {
            self.get_entity().remove();
        }
        if let ProjectileHit::Entity { ref entity, .. } = hit {
            let owner = self.projectile_owner();

            // LlamaSpit.onHitEntity only hurts with a living owner.
            if let Some(owner) = owner
                .as_deref()
                .filter(|owner| owner.get_living_entity().is_some())
                && super::damage::hurt_entity(
                    entity.as_ref(),
                    1.0,
                    DamageType::SPIT,
                    self,
                    Some(owner),
                )
            {
                super::damage::post_attack(entity.as_ref(), DamageType::SPIT, self, Some(owner));
            }
        }
    }
}
