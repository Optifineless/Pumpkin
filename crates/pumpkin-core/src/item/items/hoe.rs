use crate::block::registry::BlockActionResult;
use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::server::Server;
use pumpkin_data::BlockDirection;
use pumpkin_data::block_transformer::{DropStrategy, HOE};
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::SoundCategory;
use pumpkin_data::{Block, tag};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::{GameMode, Hand};
use pumpkin_world::world::BlockFlags;

pub struct HoeItem;

impl ItemMetadata for HoeItem {
    fn ids() -> Box<[u16]> {
        tag::Item::MINECRAFT_HOES.1.into()
    }
}

impl ItemBehaviour for HoeItem {
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
        let result = match super::block_transformer::prepare_transform(
            &HOE, player, location, face, block, hand,
        ) {
            Ok(result) => result,
            Err(action) => return action,
        };

        if let Some(result) = result {
            if let Some(sound) = result.entry.sound {
                world.play_sound(sound, SoundCategory::Blocks, &location.to_f64());
            }
            if let Some(particle) = result.entry.particle {
                world.sync_world_event(particle, location, 0);
            }

            world.set_block_state(&location, result.new_state_id, BlockFlags::NOTIFY_ALL);

            if let Some(loot_key) = result.entry.loot
                && let Some(loot_table) = world.get_loot_table(loot_key)
            {
                let seed = rand::random::<i64>();
                let drops = loot_table.generate_loot(seed);
                for drop_stack in drops {
                    if result.entry.drop_strategy == Some(DropStrategy::ClickedFace) {
                        world.drop_stack_from_face(&location, face, drop_stack);
                    } else {
                        world.drop_stack(&location, drop_stack);
                    }
                }
            }

            if player.gamemode.load() != GameMode::Creative {
                let _ = item.damage_item(i32::from(result.entry.item_damage_per_use));
            }
            return BlockActionResult::Success;
        }
        BlockActionResult::Pass
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
