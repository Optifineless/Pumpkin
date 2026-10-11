use crate::block::registry::BlockActionResult;
use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::server::Server;
use pumpkin_data::block_properties::{ChestLikeProperties, ChestType};
use pumpkin_data::block_transformer::{AXE, TransformType};
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::SoundCategory;
use pumpkin_data::{Block, BlockDirection, tag};
use pumpkin_protocol::java::client::play::CWorldEvent;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::{GameMode, Hand};
use pumpkin_world::world::BlockFlags;

pub struct AxeItem;

impl AxeItem {
    // BlockTransformer.transformBlock sends effects for both halves of a connected copper chest.
    fn send_transform_effects(
        player: &Player,
        location: BlockPos,
        current_state_id: pumpkin_data::BlockStateId,
        result: pumpkin_data::block_transformer::TransformResult,
    ) {
        let world = player.world();
        if let Some(sound) = result.entry.sound {
            world.play_block_sound_expect(player, sound, SoundCategory::Blocks, location);
        }
        let emit = |position: BlockPos| {
            if let Some(particle) = result.entry.particle {
                world.broadcast_to_chunk_except(
                    position.chunk_position(),
                    &[player.gameprofile.id],
                    &CWorldEvent::new(particle as i32, position, 0, false),
                );
            }
            world.emit_game_event("block_change", position.to_centered_f64());
        };
        emit(location);
        if result.entry.transform_type == Some(TransformType::CopperChest) {
            let props = ChestLikeProperties::from_state_id(current_state_id);
            if props.r#type != ChestType::Single {
                let direction = if props.r#type == ChestType::Left {
                    props.facing.rotate_clockwise()
                } else {
                    props.facing.rotate_counter_clockwise()
                };
                emit(location.offset(direction.to_offset()));
            }
        }
    }
}

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
            // Item.useOn -> BlockTransformer.transformBlock wears the tool before the change.
            if player.gamemode.load() != GameMode::Creative {
                let _ = item.damage_item(i32::from(result.entry.item_damage_per_use));
            }
            world.set_block_state(&location, result.new_state_id, BlockFlags::NOTIFY_ALL);
            Self::send_transform_effects(player, location, current_state_id, result);
            return BlockActionResult::Success;
        }
        BlockActionResult::Pass
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
