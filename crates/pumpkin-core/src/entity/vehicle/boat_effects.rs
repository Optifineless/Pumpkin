//! `AbstractBoat`'s repeated inside-block effects, without repeating fluid currents.

use std::sync::Arc;

use pumpkin_data::fluid::Fluid;
use pumpkin_util::math::position::BlockPos;

use super::BoatEntity;
use crate::{entity::EntityBase, world::World};

impl BoatEntity {
    pub(super) fn tick_boat_block_effects(&self, caller: &dyn EntityBase, world: &Arc<World>) {
        let entity = &self.vehicle.entity;
        if !entity.is_affected_by_blocks() {
            return;
        }
        entity.tick_block_collisions(caller);
        if !self.is_current_boat(world) {
            return;
        }
        let inset = -f64::from(1.0e-5f32);
        let bounds = entity.bounding_box.load().expand(inset, inset, inset);
        let mut fluids = Vec::new();
        for pos in BlockPos::iterate(bounds.min_block_pos(), bounds.max_block_pos()) {
            let (fluid, state) = world.get_fluid_and_fluid_state(&pos);
            if (fluid.matches_type(&Fluid::WATER) || fluid.matches_type(&Fluid::LAVA))
                && f64::from(pos.0.y as f32 + world.get_fluid_height(&pos, fluid, &state))
                    > bounds.min.y
            {
                fluids.push(fluid);
            }
        }
        // InsideBlockEffectApplier orders lava ignition before water extinguishing;
        // LavaFluid.entityInside queues a hurt callback for each contacted block.
        fluids.sort_by_key(|fluid| fluid.matches_type(&Fluid::WATER));
        for fluid in fluids {
            if !self.is_current_boat(world) {
                return;
            }
            world
                .block_registry
                .on_entity_collision_fluid(fluid, caller);
        }
    }
}
