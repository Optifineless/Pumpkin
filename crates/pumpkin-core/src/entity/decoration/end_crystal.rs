use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::entity::{Entity, EntityBase, living::LivingEntity};
use pumpkin_data::{
    damage::DamageType,
    tag::{self, Taggable},
};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};

pub struct EndCrystalEntity {
    entity: Entity,
    beam_target: Mutex<Option<BlockPos>>,
    show_bottom: AtomicBool,
    invulnerable: AtomicBool,
}

impl EndCrystalEntity {
    #[must_use]
    pub const fn new(entity: Entity) -> Self {
        Self {
            entity,
            beam_target: Mutex::new(None),
            show_bottom: AtomicBool::new(false),
            invulnerable: AtomicBool::new(false),
        }
    }

    pub fn set_show_bottom(&self, show_bottom: bool) {
        self.show_bottom.store(show_bottom, Ordering::Relaxed);
        self.entity.set_synced_data(
            pumpkin_data::tracked_data::end_crystal::SHOW_BOTTOM,
            show_bottom,
        );
    }

    pub fn show_bottom(&self) -> bool {
        self.show_bottom.load(Ordering::Relaxed)
    }

    pub fn set_beam_target(&self, beam_target: Option<BlockPos>) {
        *self
            .beam_target
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = beam_target;
        self.entity.set_synced_data(
            pumpkin_data::tracked_data::end_crystal::BEAM_TARGET,
            beam_target,
        );
    }

    pub fn beam_target(&self) -> Option<BlockPos> {
        *self
            .beam_target
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn set_invulnerable(&self, invulnerable: bool) {
        self.invulnerable.store(invulnerable, Ordering::Relaxed);
    }

    pub fn is_invulnerable(&self) -> bool {
        self.invulnerable.load(Ordering::Relaxed)
    }
}

impl EntityBase for EndCrystalEntity {
    fn get_entity(&self) -> &Entity {
        &self.entity
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        None
    }

    fn can_hit(&self) -> bool {
        !self.entity.is_removed()
    }

    fn damage_with_context(
        &self,
        _caller: &dyn EntityBase,
        _amount: f32,
        damage_type: DamageType,
        _position: Option<Vector3<f64>>,
        _source: Option<&dyn EntityBase>,
        cause: Option<&dyn EntityBase>,
    ) -> bool {
        if self.is_invulnerable() {
            return false;
        }

        let world = self.entity.world.load();
        let direct = world.get_entity_by_id(self.entity.entity_id);
        self.entity.remove();
        if !damage_type.has_tag(&tag::DamageType::MINECRAFT_IS_EXPLOSION) {
            let pos = self.entity.pos.load();
            // EndCrystal.hurtServer preserves the triggering damage source's causing entity.
            let cause = cause.and_then(|cause| {
                crate::entity::projectile::ownership::resolve_owner(
                    &world,
                    cause.get_entity().entity_uuid,
                )
            });
            let explosion = crate::world::Explosion::new(
                6.0,
                pos,
                world.get_block_interaction(crate::world::ExplosionInteraction::Block),
            )
            .with_source(direct)
            .with_cause(cause);
            world.run_explosion(&explosion);
        }

        if let Some(ref fight_mutex) = world.dragon_fight {
            // EndCrystal.onDestroyedBy: spawn callbacks can re-enter a locked fight.
            match fight_mutex.try_lock() {
                Ok(mut fight) => fight.on_crystal_destroyed(&world, self.entity.entity_uuid),
                Err(std::sync::TryLockError::Poisoned(error)) => error
                    .into_inner()
                    .on_crystal_destroyed(&world, self.entity.entity_uuid),
                Err(std::sync::TryLockError::WouldBlock) => {
                    world
                        .pending_destroyed_crystals
                        .push(self.entity.entity_uuid);
                }
            }
        }

        true
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
}
