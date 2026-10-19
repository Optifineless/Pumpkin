use crate::block::registry::BlockActionResult;
use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::server::Server;
use pumpkin_data::block_properties::{BlockProperties, DoubleBlockHalf, OakDoorLikeProperties};
use pumpkin_data::block_transformer::AXE;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::tag::Taggable;
use pumpkin_data::{Block, BlockDirection, tag};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::{GameMode, Hand};
use pumpkin_world::world::BlockFlags;

pub struct AxeItem;

impl ItemMetadata for AxeItem {
    fn ids() -> Box<[u16]> {
        tag::Item::MINECRAFT_AXES.1.into()
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
        let current_state_id = world.get_block_state_id(&location);
        let result = match super::block_transformer::prepare_transform(
            &AXE, player, location, face, block, hand,
        ) {
            Ok(result) => result,
            Err(action) => return action,
        };
        if let Some(result) = result {
            if player.gamemode.load() != GameMode::Creative {
                let _ = item.damage_item(i32::from(result.entry.item_damage_per_use));
            }
            let is_door = block.has_tag(&tag::Block::MINECRAFT_DOORS);
            if is_door {
                let door_props = OakDoorLikeProperties::from_state_id(current_state_id);
                let other_half_pos = match door_props.half {
                    DoubleBlockHalf::Lower => location.up(),
                    DoubleBlockHalf::Upper => location.down(),
                };
                let (other_block, other_state_id) = world.get_block_and_state_id(&other_half_pos);
                if other_block == block {
                    let other_props = OakDoorLikeProperties::from_state_id(other_state_id);
                    if other_props.half != door_props.half {
                        let other_new_state_id = if result.target_block.states.len() <= 1 {
                            result.target_block.default_state.id
                        } else {
                            result
                                .target_block
                                .from_properties(&other_props.to_props())
                                .to_state_id(result.target_block)
                        };
                        world.set_block_state(
                            &other_half_pos,
                            other_new_state_id,
                            BlockFlags::NOTIFY_LISTENERS
                                | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK
                                | BlockFlags::UPDATE_KNOWN_SHAPE,
                        );
                    }
                }
            }

            let flags = if is_door {
                BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK
            } else {
                BlockFlags::NOTIFY_ALL
            };
            world.set_block_state(&location, result.new_state_id, flags);
            super::block_transformer::send_transform_effects(
                player,
                location,
                current_state_id,
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
