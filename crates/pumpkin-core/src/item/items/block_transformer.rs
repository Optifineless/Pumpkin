use crate::{
    block::registry::BlockActionResult,
    entity::{EntityBase, player::Player},
    net::ClientPlatform,
};
use pumpkin_data::{
    Block, BlockDirection, BlockStateId,
    block_properties::{ChestLikeProperties, ChestType},
    block_transformer::{BlockTransformer, TransformResult, TransformType},
    data_component_impl::BlocksAttacksImpl,
    game_event::GameEvent,
};
use pumpkin_protocol::java::client::play::CWorldEvent;
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
    observed: BlockStateId,
) -> Result<Option<TransformResult>, BlockActionResult> {
    if Block::from_state_id(observed) != block {
        return Ok(None);
    }
    if matches!(player.client.as_ref(), ClientPlatform::Java(_))
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
    let mut result = transformer.transform(block, observed, face, &get_block);
    if let Some(result) = &mut result
        && result.entry.update_from_neighbors
    {
        result.new_state_id = world.update_from_neighbor_shapes(result.new_state_id, &location);
    }
    Ok(result)
}

#[cfg(test)]
pub(super) fn before_transform_write(world: &crate::world::World, position: BlockPos) {
    let hook = world
        .persistence_test_hooks
        .transform_write
        .lock()
        .unwrap()
        .clone();
    if let Some(hook) = hook {
        hook(position);
    }
}

pub(super) fn send_transform_effects(
    player: &Player,
    location: BlockPos,
    current_state_id: BlockStateId,
    result: TransformResult,
    action: Option<&crate::world::game_events::CapturedSource<'_>>,
) {
    let world = player.world();
    if let Some(sound) = result.entry.sound {
        world.play_block_sound_expect(
            player,
            sound,
            pumpkin_data::sound::SoundCategory::Blocks,
            location,
        );
    }

    let emit = |position: BlockPos| {
        if let Some(particle) = result.entry.particle {
            world.broadcast_to_chunk_except(
                position.chunk_position(),
                &[player.gameprofile.id],
                &CWorldEvent::new(particle as i32, position, 0, false),
            );
        }
        world.emit_game_event_after_action(
            GameEvent::BlockChange.name(),
            position.to_centered_f64(),
            || crate::world::game_events::GameEventContext {
                source: Some(player.get_entity()),
                affected_state: Some(if position == location {
                    result.new_state_id
                } else {
                    world.get_block_state_id(&position)
                }),
            },
            action,
        );
    };
    emit(location);

    if result.entry.transform_type == Some(TransformType::CopperChest) {
        let properties = ChestLikeProperties::from_state_id(current_state_id);
        if properties.r#type != ChestType::Single {
            let direction = if properties.r#type == ChestType::Left {
                properties.facing.rotate_clockwise()
            } else {
                properties.facing.rotate_counter_clockwise()
            };
            emit(location.offset(direction.to_offset()));
        }
    }
}
