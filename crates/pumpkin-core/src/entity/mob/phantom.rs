use crate::entity::{
    Entity, EntityBase,
    ai::control::phantom_move_control::PhantomMoveControl,
    mob::{Mob, MobEntity},
};
use crossbeam::atomic::AtomicCell;
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use std::sync::{Arc, atomic::AtomicBool};

pub struct PhantomEntity {
    pub mob_entity: MobEntity,
    /// `PhantomMoveControl`'s destination; attack goals update this while swooping.
    pub move_target_point: AtomicCell<Vector3<f64>>,
    /// Spawn and attack goals set the anchor around which idle flight circles.
    pub anchor_point: AtomicCell<Option<BlockPos>>,
    /// Stub for `PhantomSweepAttackGoal`, not yet registered; selects swoop steering.
    pub swooping: AtomicBool,
}
impl PhantomEntity {
    pub fn new(entity: Entity) -> Arc<Self> {
        let mob_entity = MobEntity::new(entity);
        // Phantom constructor / registerGoals (movement only).
        *mob_entity
            .move_control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Box::new(PhantomMoveControl::default());
        mob_entity.add_goal(
            3,
            super::phantom_movement::PhantomCircleAroundAnchorGoal::default(),
        );
        Arc::new(Self {
            mob_entity,
            move_target_point: AtomicCell::new(Vector3::new(0.0, 0.0, 0.0)),
            anchor_point: AtomicCell::new(None),
            swooping: AtomicBool::new(false),
        })
    }
}
impl Mob for PhantomEntity {
    fn finalize_spawn(
        &self,
        world: &Arc<crate::world::World>,
        view: &crate::world::spawn_view::SpawnView<'_>,
        group: Option<super::spawn::SpawnGroupData>,
    ) -> Option<super::spawn::SpawnGroupData> {
        // Phantom.finalizeSpawn: idle anchor is five blocks above the spawn position.
        self.anchor_point
            .store(Some(self.get_entity().block_pos.load().add(0, 5, 0)));
        super::movement::finalize_spawn_after_species(self, world, view, group)
    }

    fn get_mob_entity(&self) -> &MobEntity {
        &self.mob_entity
    }
    fn get_mob_gravity(&self) -> f64 {
        0.0
    }
    // PhantomLookControl.tick is deliberately empty.
    fn can_tick_look_control(&self) -> bool {
        false
    }
    fn custom_body_rotation(&self) -> bool {
        // PhantomBodyRotationControl.clientTick.
        let entity = self.get_entity();
        entity.head_yaw.store(entity.body_yaw.load());
        entity.body_yaw.store(entity.yaw.load());
        true
    }
    fn custom_travel(&self, caller: &dyn EntityBase) -> bool {
        super::movement::travel_flying(self, caller, 0.02, 0.02, 0.2)
    }
}
