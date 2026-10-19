use std::any::Any;

use crate::block::registry::BlockActionResult;
use crate::entity::player::Player;
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::server::Server;
use pumpkin_data::data_component_impl::{LodestoneTarget, LodestoneTrackerImpl};
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::{Block, BlockDirection};
use pumpkin_util::GameMode;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

pub struct CompassItem;

impl ItemMetadata for CompassItem {
    fn ids() -> Box<[u16]> {
        Box::new([Item::COMPASS.id, Item::RECOVERY_COMPASS.id])
    }
}

impl ItemBehaviour for CompassItem {
    fn use_on_block(
        &self,
        item: &mut ItemStack,
        player: &Player,
        location: BlockPos,
        _face: BlockDirection,
        _cursor_pos: Vector3<f32>,
        block: &Block,
        _server: &Server,
    ) -> BlockActionResult {
        if block.id == Block::LODESTONE.id && item.item.id == Item::COMPASS.id {
            let world = player.world();
            world.play_sound(
                Sound::ItemLodestoneCompassLock,
                SoundCategory::Players,
                &location.to_f64(),
            );

            let gamemode = player.gamemode.load();
            let replace_existing_stack = gamemode != GameMode::Creative && item.item_count == 1;
            let tracker = LodestoneTrackerImpl {
                target: Some(LodestoneTarget {
                    dimension: world.dimension.minecraft_name.to_string(),
                    x: location.0.x,
                    y: location.0.y,
                    z: location.0.z,
                }),
                tracked: true,
            };

            if replace_existing_stack {
                item.set_data_component(tracker);
            } else {
                let mut lodestone_compass = item.copy_with_count(1);
                item.decrement_unless_creative(gamemode, 1);
                lodestone_compass.set_data_component(tracker);
                let was_added = player
                    .inventory
                    .insert_stack_anywhere(&mut lodestone_compass);
                if !was_added && !lodestone_compass.is_empty() && gamemode != GameMode::Creative {
                    world.drop_stack(&player.position().to_block_pos(), lodestone_compass);
                }
            }
            BlockActionResult::Success
        } else {
            BlockActionResult::Pass
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
#[path = "compass_tests.rs"]
mod tests;
