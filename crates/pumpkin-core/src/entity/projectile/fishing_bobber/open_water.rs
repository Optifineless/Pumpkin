use super::FishingBobberEntity;
use crate::world::World;
use pumpkin_data::{Block, fluid::Fluid};
use pumpkin_util::math::position::BlockPos;

#[derive(Clone, Copy, PartialEq, Eq)]
enum OpenWaterType {
    AboveWater,
    InsideWater,
    Invalid,
}

impl FishingBobberEntity {
    // FishingHook.calculateOpenWater: four uniform 5x5 layers, water followed by air.
    pub(super) fn calculate_open_water(&self, pos: &BlockPos) -> bool {
        let world = self.entity.world.load();
        let mut previous = OpenWaterType::Invalid;
        for y in -1..=2 {
            let layer = open_water_type_for_area(
                &world,
                &pos.offset(pumpkin_util::math::vector3::Vector3::new(-2, y, -2)),
            );
            match layer {
                OpenWaterType::AboveWater if previous == OpenWaterType::Invalid => return false,
                OpenWaterType::InsideWater if previous == OpenWaterType::AboveWater => {
                    return false;
                }
                OpenWaterType::Invalid => return false,
                _ => {}
            }
            previous = layer;
        }
        true
    }
}

fn open_water_type_for_area(world: &World, from: &BlockPos) -> OpenWaterType {
    let mut layer = None;
    for z in 0..5 {
        for x in 0..5 {
            let pos = from.offset(pumpkin_util::math::vector3::Vector3::new(x, 0, z));
            let kind = open_water_type_for_block(world, &pos);
            if kind == OpenWaterType::Invalid || layer.is_some_and(|layer| layer != kind) {
                return OpenWaterType::Invalid;
            }
            layer = Some(kind);
        }
    }
    layer.unwrap_or(OpenWaterType::Invalid)
}

// FishingHook.getOpenWaterTypeForBlock permits source water in blocks with no collision.
fn open_water_type_for_block(world: &World, pos: &BlockPos) -> OpenWaterType {
    let state = world.get_block_state(pos);
    let block = state.id.to_block();
    if block.is_air() || block == &Block::LILY_PAD {
        return OpenWaterType::AboveWater;
    }
    let (fluid, fluid_state) = world.get_fluid_and_fluid_state(pos);
    if fluid.matches_type(&Fluid::WATER)
        && fluid_state.is_source
        && state.collision_shapes.is_empty()
    {
        OpenWaterType::InsideWater
    } else {
        OpenWaterType::Invalid
    }
}
