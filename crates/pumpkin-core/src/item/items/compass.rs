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
use pumpkin_inventory::{Inventory, inventory::slot_transaction};
use pumpkin_util::GameMode;
use pumpkin_util::Hand;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

pub struct CompassItem;

impl CompassItem {
    fn use_on_lodestone(
        item: &mut ItemStack,
        player: &Player,
        location: BlockPos,
        block: &Block,
        hand: Hand,
    ) -> BlockActionResult {
        if block.id != Block::LODESTONE.id || item.item.id != Item::COMPASS.id {
            return BlockActionResult::Pass;
        }

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
            let (mut lodestone_compass, source_slot) = if gamemode == GameMode::Creative {
                (item.copy_with_count(1), None)
            } else {
                let inventory = player.inventory();
                let hand_slot = crate::net::java::play::hand_use_result::hand_slot(player, hand);
                let owner = inventory.slot_transaction(hand_slot);
                let guard = owner.lock();
                // CompassItem.useOn consumes the source before Inventory.add may merge into it.
                let lodestone_compass =
                    slot_transaction::withdraw(inventory.as_ref(), hand_slot, item, 1);
                if lodestone_compass.is_empty() {
                    return BlockActionResult::Pass;
                }
                drop(guard);

                // Keep the handler's detached snapshot unchanged after charging the live hand.
                (lodestone_compass, Some(hand_slot))
            };
            lodestone_compass.set_data_component(tracker);
            let was_added = player
                .inventory
                .insert_stack_anywhere(&mut lodestone_compass);
            if !was_added && !lodestone_compass.is_empty() && gamemode != GameMode::Creative {
                world.drop_stack(&player.position().to_block_pos(), lodestone_compass);
            }
            if let Some(source_slot) = source_slot {
                player.sync_hand_slot(source_slot, player.inventory().get_stack(source_slot));
            }
        }
        BlockActionResult::Success
    }
}

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
        Self::use_on_lodestone(item, player, location, block, Hand::Right)
    }

    fn use_on_block_with_hand(
        &self,
        item: &mut ItemStack,
        player: &Player,
        location: BlockPos,
        _face: BlockDirection,
        _cursor_pos: Vector3<f32>,
        block: &Block,
        _server: &Server,
        hand: Hand,
    ) -> BlockActionResult {
        Self::use_on_lodestone(item, player, location, block, hand)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
#[path = "compass_tests.rs"]
mod tests;
