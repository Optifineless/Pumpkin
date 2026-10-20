//! `AbstractBoat.isUnderwater`, distinct from contact at the hull's bottom.

use pumpkin_data::fluid::Fluid;
use pumpkin_util::math::position::BlockPos;

use super::boat::BoatEntity;
use crate::world::World;

impl BoatEntity {
    pub(super) fn is_underwater(&self, world: &World) -> bool {
        let bounds = self.vehicle.entity.bounding_box.load();
        let top = bounds.max.y + 0.001;
        for x in bounds.min.x.floor() as i32..bounds.max.x.ceil() as i32 {
            for y in bounds.max.y.floor() as i32..top.ceil() as i32 {
                for z in bounds.min.z.floor() as i32..bounds.max.z.ceil() as i32 {
                    let pos = BlockPos::new(x, y, z);
                    let (fluid, state) = world.get_fluid_and_fluid_state(&pos);
                    if fluid.matches_type(&Fluid::WATER)
                        && top < f64::from(y as f32 + world.get_fluid_height(&pos, fluid, &state))
                    {
                        return true;
                    }
                }
            }
        }
        false
    }
}
