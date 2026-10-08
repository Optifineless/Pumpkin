#[cfg(test)]
mod damage_tests;
#[cfg(test)]
mod flight_review_tests;
#[cfg(test)]
mod hit_review_tests;
#[cfg(test)]
mod parity_tests;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod verification_tests;
use super::{Entity, EntityBase, living::LivingEntity};
use pumpkin_data::BlockDirection;
use pumpkin_data::entity::EntityType;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_protocol::java::client::play::CEntityVelocity;
use pumpkin_util::math::boundingbox::BoundingBox;
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use std::{
    sync::Arc,
    sync::atomic::{AtomicBool, Ordering},
};
pub mod arrow;
mod arrow_collision;
mod arrow_hit;
mod block_effects;
pub(crate) mod clip;
mod collision;
mod damage;
pub mod deflection;
pub mod dragon_fireball;
pub mod egg;
pub mod ender_pearl;
pub mod evoker_fangs;
pub mod eye_of_ender;
pub mod fireball;
pub mod firework_rocket;
pub mod fishing_bobber;
mod hurting;
pub mod lingering_potion;
pub mod llama_spit;
pub mod ownership;
pub(crate) mod potion_effects;
pub(crate) mod potion_water;
pub mod shulker_bullet;
pub mod small_fireball;
pub mod snowball;
pub mod splash_potion;
pub mod trident;
mod trident_hit;
pub mod wind_charge;
pub mod wither_skull;

use pumpkin_data::item_stack::ItemStack;

#[must_use]
pub fn is_projectile(entity_type: &EntityType) -> bool {
    *entity_type == EntityType::ARROW
        || *entity_type == EntityType::TRIDENT
        || *entity_type == EntityType::EGG
        || *entity_type == EntityType::SNOWBALL
        || *entity_type == EntityType::FIREWORK_ROCKET
        || *entity_type == EntityType::WIND_CHARGE
        || *entity_type == EntityType::SPLASH_POTION
        || *entity_type == EntityType::LINGERING_POTION
        || *entity_type == EntityType::ENDER_PEARL
        || *entity_type == EntityType::SHULKER_BULLET
        || *entity_type == EntityType::DRAGON_FIREBALL
        || *entity_type == EntityType::FIREBALL
        || *entity_type == EntityType::SMALL_FIREBALL
        || *entity_type == EntityType::FISHING_BOBBER
        || *entity_type == EntityType::WITHER_SKULL
        || *entity_type == EntityType::LLAMA_SPIT
}

// Projectile.canHitEntity's target check: Entity.canBeHitByProjectile and isPickable.
// ProjectileState applies the owner/root-vehicle exclusion separately.
fn can_hit_entity(other: &Arc<dyn EntityBase>) -> bool {
    let entity = other.get_entity();
    if !entity.is_alive()
        || other.is_spectator()
        || entity.entity_type == &EntityType::INTERACTION
        || entity.entity_type == &EntityType::ENDER_DRAGON
    {
        return false;
    }
    if let Some(stand) = other
        .cast_any()
        .downcast_ref::<super::decoration::armor_stand::ArmorStandEntity>()
        && stand.is_marker()
    {
        return false;
    }
    if let Some(living) = other.get_living_entity() {
        return living.health.load() > 0.0;
    }

    // Nonliving isPickable overrides, including Projectile's redirectable tag.
    other.can_hit()
        || [
            &EntityType::END_CRYSTAL,
            &EntityType::ITEM_FRAME,
            &EntityType::GLOW_ITEM_FRAME,
            &EntityType::FALLING_BLOCK,
            &EntityType::TNT,
            &EntityType::SHULKER_BULLET,
        ]
        .contains(&entity.entity_type)
        || entity
            .entity_type
            .has_tag(&tag::EntityType::MINECRAFT_REDIRECTABLE_PROJECTILE)
}

/// Helper to apply projectile spawned enchantment effects matching vanilla `Projectile::applyOnProjectileSpawned`.
pub fn apply_on_projectile_spawned(
    projectile_entity: &Entity,
    pickup_item_stack: &ItemStack,
    weapon: Option<&ItemStack>,
    arrow: Option<&arrow::ArrowEntity>,
) {
    crate::enchantment::EnchantmentHelper::on_projectile_spawned(
        pickup_item_stack,
        projectile_entity,
        arrow,
    );
    if let Some(weapon) = weapon
        && weapon.item_count > 0
        && weapon.item.id != pickup_item_stack.item.id
    {
        crate::enchantment::EnchantmentHelper::on_projectile_spawned(
            weapon,
            projectile_entity,
            arrow,
        );
    }
}

pub struct ThrownItemEntity {
    pub entity: Entity,
    pub projectile: ownership::ProjectileState,
    pub has_hit: AtomicBool,
    pub gravity: f64,
}

impl ThrownItemEntity {
    pub fn new(entity: Entity, owner: &Entity, gravity: f64) -> Self {
        let mut owner_pos = owner.pos.load();
        owner_pos.y += owner.get_eye_height() - 0.1;
        entity.pos.store(owner_pos);
        Self {
            entity,
            projectile: ownership::ProjectileState::new(Some(owner.entity_uuid)),
            has_hit: AtomicBool::new(false),
            gravity,
        }
    }

    pub fn set_velocity_from(&self, pitch: f32, yaw: f32, roll: f32, speed: f32, divergence: f32) {
        let yaw_rad = yaw.to_radians();
        let pitch_rad = pitch.to_radians();
        let roll_rad = (pitch + roll).to_radians();

        let x = -yaw_rad.sin() * pitch_rad.cos();
        let y = -roll_rad.sin();
        let z = yaw_rad.cos() * pitch_rad.cos();

        self.set_velocity(
            f64::from(x),
            f64::from(y),
            f64::from(z),
            f64::from(speed),
            f64::from(divergence),
        );
    }

    pub fn set_velocity(&self, x: f64, y: f64, z: f64, power: f64, uncertainty: f64) {
        fn next_triangular(mode: f64, deviation: f64) -> f64 {
            deviation.mul_add(rand::random::<f64>() - rand::random::<f64>(), mode)
        }
        let velocity = Vector3::new(x, y, z)
            .normalize()
            .add_raw(
                next_triangular(0.0, 0.017_227_5 * uncertainty),
                next_triangular(0.0, 0.017_227_5 * uncertainty),
                next_triangular(0.0, 0.017_227_5 * uncertainty),
            )
            .multiply(power, power, power);

        self.entity.velocity.store(velocity);
        let len = velocity.horizontal_length();
        self.entity.set_rotation(
            velocity.x.atan2(velocity.z) as f32 * 57.295_776,
            velocity.y.atan2(len) as f32 * 57.295_776,
        );
    }
}

impl ThrownItemEntity {
    /// Process a tick for projectile movement and collisions
    pub fn process_tick(&self, caller: &dyn EntityBase, server: &crate::server::Server) {
        let entity = self.get_entity();
        let world = entity.world.load();

        entity.update_last_pos();

        if !hurting::before_move(caller) {
            return;
        }
        let velocity = if hurting::is_hurting(entity) {
            hurting::movement(caller, self.projectile.acceleration_power())
        } else {
            let mut velocity = entity.velocity.load();
            if !entity.has_no_gravity() {
                velocity.y -= self.get_gravity();
            }
            let inertia = if entity.is_in_water() {
                f64::from(0.8f32)
            } else {
                f64::from(0.99f32)
            };
            velocity * inertia
        };

        // Store velocity
        entity.velocity.store(velocity);

        let start_pos = entity.pos.load();
        let delta = velocity;

        // ThrowableProjectile.tick / AbstractHurtingProjectile.tick stop at the clipped impact.
        let hit = if entity.no_physics.load(Ordering::Relaxed) {
            None
        } else {
            self.find_hit(caller, start_pos, delta)
        };
        entity.set_pos(
            hit.as_ref()
                .map_or(start_pos + delta, ProjectileHit::hit_pos),
        );
        entity.tick_block_collisions(caller);
        // ThrowableProjectile.tick / AbstractHurtingProjectile.tick -> Projectile.tick -> Entity.tick.
        self.projectile.tick(entity);
        EntityBase::tick(entity, caller, server);
        // Entity.commonTick saves oldPosition before movement; ThrownEnderpearl.onHit reads it.
        if Arc::ptr_eq(&world, &entity.world.load()) {
            entity.last_pos.store(start_pos);
        }
        // AbstractHurtingProjectile.tick ignites after block effects and before hit callbacks.
        if hurting::should_burn(entity) {
            entity.set_on_fire_for(1.0);
        }

        // Send updated velocity to clients
        let packet = CEntityVelocity::new(entity.entity_id.into(), velocity);
        let chunk_pos = entity.chunk_pos.load();
        world.broadcast_to_chunk(chunk_pos, &packet);

        if entity.is_alive()
            // Entity.teleportCrossDimension removes the old instance before the hit callback.
            && Arc::ptr_eq(&world, &entity.world.load())
            && let Some(hit) = hit
        {
            self.hit_target(caller, hit);
        }
    }

    /// Returns if collision should be skipped (e.g. owner or projectile vs projectile)
    fn should_skip_collision(
        &self,
        self_ent: &Entity,
        other: &Arc<dyn EntityBase>,
        owner: Option<&Arc<dyn EntityBase>>,
    ) -> bool {
        let other_ent = other.get_entity();
        if other_ent.entity_id == self_ent.entity_id {
            return true;
        }

        if !self.projectile.can_hit_with_owner(self_ent, other, owner)
            || hurting::is_hurting(self_ent) && other_ent.no_physics.load(Ordering::Relaxed)
        {
            return true;
        }

        if [
            EntityType::WIND_CHARGE.id,
            EntityType::BREEZE_WIND_CHARGE.id,
        ]
        .contains(&self_ent.entity_type.id)
            && [
                EntityType::WIND_CHARGE.id,
                EntityType::BREEZE_WIND_CHARGE.id,
                EntityType::END_CRYSTAL.id,
            ]
            .contains(&other_ent.entity_type.id)
        {
            return true;
        }

        // Projectiles should pass through lingering clouds
        if *other_ent.entity_type == EntityType::AREA_EFFECT_CLOUD {
            return true;
        }

        false
    }

    const fn get_entity(&self) -> &Entity {
        &self.entity
    }

    #[allow(dead_code, clippy::unused_self)]
    const fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }
    const fn get_gravity(&self) -> f64 {
        self.gravity
    }
}

/// Ray intersection algorithm for AABBs, returning a t value
fn calculate_ray_intersection(
    start: &Vector3<f64>,
    dir: &Vector3<f64>,
    bb: &BoundingBox,
) -> Option<f64> {
    clip::clip_box(*start, *dir, *bb).map(|(t, _)| t)
}

pub enum ProjectileHit {
    Block {
        pos: BlockPos,
        world_border: bool,
        face: BlockDirection,
        hit_pos: Vector3<f64>,
        normal: Vector3<f64>,
    },
    Entity {
        entity: Arc<dyn EntityBase>,
        hit_pos: Vector3<f64>,
        normal: Vector3<f64>,
    },
}

impl ProjectileHit {
    /// Returns the exact impact coordinates regardless of what was hit.
    #[must_use]
    pub const fn hit_pos(&self) -> Vector3<f64> {
        match self {
            Self::Block { hit_pos, .. } | Self::Entity { hit_pos, .. } => *hit_pos,
        }
    }

    /// Returns the surface normal of the impact.
    #[must_use]
    pub const fn normal(&self) -> Vector3<f64> {
        match self {
            Self::Block { normal, .. } | Self::Entity { normal, .. } => *normal,
        }
    }

    /// Safely returns the face hit if it was a block, otherwise None.
    #[must_use]
    pub const fn face(&self) -> Option<BlockDirection> {
        match self {
            Self::Block { face, .. } => Some(*face),
            Self::Entity { .. } => None,
        }
    }
}
