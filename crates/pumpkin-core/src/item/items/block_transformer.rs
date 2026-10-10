use crate::{
    block::registry::BlockActionResult,
    entity::{EntityBase, player::Player},
};
use pumpkin_data::{
    Block, BlockDirection,
    block_transformer::{BlockTransformer, TransformResult},
    data_component_impl::BlocksAttacksImpl,
};
use pumpkin_util::{
    Hand,
    math::{position::BlockPos, vector3::Vector3},
};

// BlockTransformer.transformBlock / playerHasBlockingItemUseIntent.
pub(super) fn prepare_transform(
    transformer: &BlockTransformer,
    player: &Player,
    location: BlockPos,
    face: BlockDirection,
    block: &Block,
    hand: Hand,
) -> Result<Option<TransformResult>, BlockActionResult> {
    // Bedrock use-on-block reports only Right; it has no Java offhand blocking intent.
    if matches!(player.client.as_ref(), crate::net::ClientPlatform::Java(_))
        && hand == Hand::Right
        && player
            .inventory
            .off_hand_item()
            .get_data_component::<BlocksAttacksImpl>()
            .is_some()
        && !player.get_entity().is_sneaking()
    {
        return Err(BlockActionResult::Pass);
    }
    let world = player.world();
    let get_block = |dx: i8, dy: i8, dz: i8| {
        world.get_block(&BlockPos(
            location.0 + Vector3::new(i32::from(dx), i32::from(dy), i32::from(dz)),
        ))
    };
    let mut result =
        transformer.transform(block, world.get_block_state_id(&location), face, &get_block);
    if let Some(result) = &mut result
        && result.entry.update_from_neighbors
    {
        result.new_state_id = world.update_from_neighbor_shapes(result.new_state_id, &location);
    }
    Ok(result)
}
