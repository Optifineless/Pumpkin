use crate::block::registry::BlockActionResult;
use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::server::Server;
use crate::world::World;
use pumpkin_data::block_properties::{
    BlockProperties, ChestLikeProperties, ChestType, DoubleBlockHalf, OakDoorLikeProperties,
};
use pumpkin_data::block_transformer::{AXE, TransformResult, TransformType};
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::tag::Taggable;
use pumpkin_data::{Block, BlockDirection, tag};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::{GameMode, Hand};
use pumpkin_world::world::BlockFlags;
use std::sync::Arc;

pub struct AxeItem;

const TRANSFORM_ATTEMPTS: usize = 3;

impl ItemMetadata for AxeItem {
    fn ids() -> Box<[u16]> {
        tag::Item::MINECRAFT_AXES.1.into()
    }
}

impl AxeItem {
    fn transform_updates(
        world: &Arc<World>,
        location: BlockPos,
        block: &Block,
        current_state_id: pumpkin_data::BlockStateId,
        result: &TransformResult,
    ) -> Vec<(
        BlockPos,
        pumpkin_data::BlockStateId,
        pumpkin_data::BlockStateId,
        BlockFlags,
    )> {
        let mut updates = Vec::new();
        if block.has_tag(&tag::Block::MINECRAFT_DOORS) {
            let props = OakDoorLikeProperties::from_state_id(current_state_id);
            let partner = if props.half == DoubleBlockHalf::Lower {
                location.up()
            } else {
                location.down()
            };
            let observed = world.get_block_state_id(&partner);
            if observed.to_block() == block {
                let other_props = OakDoorLikeProperties::from_state_id(observed);
                if other_props.half != props.half {
                    let next = result
                        .target_block
                        .from_properties(&other_props.to_props())
                        .to_state_id(result.target_block);
                    updates.push((
                        partner,
                        observed,
                        next,
                        BlockFlags::NOTIFY_LISTENERS
                            | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK
                            | BlockFlags::UPDATE_KNOWN_SHAPE,
                    ));
                }
            }
        } else if result.entry.transform_type == Some(TransformType::CopperChest) {
            let props = ChestLikeProperties::from_state_id(current_state_id);
            let direction = match props.r#type {
                ChestType::Single => None,
                ChestType::Left => Some((props.facing.rotate_clockwise(), ChestType::Right)),
                ChestType::Right => {
                    Some((props.facing.rotate_counter_clockwise(), ChestType::Left))
                }
            };
            if let Some((direction, expected_type)) = direction {
                let partner = location.offset(direction.to_offset());
                let observed = world.get_block_state_id(&partner);
                if observed.to_block() == block {
                    let other_props = ChestLikeProperties::from_state_id(observed);
                    if other_props.facing == props.facing && other_props.r#type == expected_type {
                        let next = result
                            .target_block
                            .from_properties(&other_props.to_props())
                            .to_state_id(result.target_block);
                        updates.push((partner, observed, next, BlockFlags::NOTIFY_LISTENERS));
                    }
                }
            }
        }
        let flags = if block.has_tag(&tag::Block::MINECRAFT_DOORS) {
            BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK
        } else {
            BlockFlags::NOTIFY_ALL
        };
        let clicked_state = if result.entry.transform_type == Some(TransformType::CopperChest)
            && updates.is_empty()
        {
            let mut props = ChestLikeProperties::from_state_id(result.new_state_id);
            props.r#type = ChestType::Single;
            props.to_state_id(result.target_block)
        } else {
            result.new_state_id
        };
        updates.push((location, current_state_id, clicked_state, flags));
        updates
    }
}

impl ItemBehaviour for AxeItem {
    fn use_on_block(
        &self,
        item: &mut ItemStack,
        player: &Player,
        location: BlockPos,
        face: BlockDirection,
        cursor_pos: Vector3<f32>,
        block: &Block,
        server: &Server,
    ) -> BlockActionResult {
        self.use_on_block_with_hand(
            item,
            player,
            location,
            face,
            cursor_pos,
            block,
            server,
            Hand::Right,
        )
    }

    fn use_on_block_with_hand(
        &self,
        item: &mut ItemStack,
        player: &Player,
        location: BlockPos,
        face: BlockDirection,
        _cursor_pos: Vector3<f32>,
        block: &Block,
        _server: &Server,
        hand: Hand,
    ) -> BlockActionResult {
        let world = player.world();
        for _ in 0..TRANSFORM_ATTEMPTS {
            // BlockTransformer's CopyPropertiesProvider reads live properties on each attempt.
            let current_state_id = world.get_block_state_id(&location);
            let result = match super::block_transformer::prepare_transform(
                &AXE,
                player,
                location,
                face,
                block,
                hand,
                current_state_id,
            ) {
                Ok(Some(result)) => result,
                Ok(None) => return BlockActionResult::Pass,
                Err(action) => return action,
            };
            let updates =
                Self::transform_updates(&world, location, block, current_state_id, &result);
            #[cfg(test)]
            for (position, _, _, _) in &updates {
                super::block_transformer::before_transform_write(&world, *position);
            }
            if !world.set_block_states_from_observed(&updates) {
                // Retry from the current pair when callbacks changed either observation.
                continue;
            }
            if player.gamemode.load() != GameMode::Creative {
                let _ = item.damage_item(i32::from(result.entry.item_damage_per_use));
            }
            let effect_state = if result.entry.transform_type == Some(TransformType::CopperChest)
                && updates.len() == 1
            {
                let mut props = ChestLikeProperties::from_state_id(current_state_id);
                props.r#type = ChestType::Single;
                props.to_state_id(block)
            } else {
                current_state_id
            };
            super::block_transformer::send_transform_effects(
                player,
                location,
                effect_state,
                result,
            );
            return BlockActionResult::Success;
        }
        BlockActionResult::Pass
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
#[path = "axe_review_tests.rs"]
mod review_tests;
