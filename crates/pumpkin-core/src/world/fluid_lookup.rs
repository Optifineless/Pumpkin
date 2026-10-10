use pumpkin_data::{BlockStateId, fluid::Fluid};

/// Resolves the fluid family queried by `SugarCaneBlock#canSurvive`.
pub fn get_fluid_from_state_id(id: BlockStateId) -> &'static Fluid {
    if let Some(fluid) = Fluid::from_state_id(id) {
        return fluid.to_flowing();
    }
    // These blocks contain source water without a `waterlogged` property.
    if matches!(
        id.to_block_id(),
        pumpkin_data::BlockId::KELP
            | pumpkin_data::BlockId::KELP_PLANT
            | pumpkin_data::BlockId::SEAGRASS
            | pumpkin_data::BlockId::TALL_SEAGRASS
            | pumpkin_data::BlockId::BUBBLE_COLUMN
    ) || id.is_waterlogged()
    {
        &Fluid::FLOWING_WATER
    } else {
        &Fluid::EMPTY
    }
}
