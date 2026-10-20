use crate::block::registry::BlockActionResult;
use crate::entity::player::Player;
use crate::entity::vehicle::minecart::MinecartEntity;
use crate::item::{ItemBehaviour, ItemMetadata};
use crate::server::Server;
use pumpkin_data::Block;
use pumpkin_data::BlockDirection;
use pumpkin_data::entity::EntityType;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_inventory::Inventory;
use pumpkin_util::Hand;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

pub struct MinecartItem;

impl MinecartItem {
    pub(crate) fn item_to_entity(item: &Item) -> &'static EntityType {
        match item.id {
            val if val == Item::MINECART.id => &EntityType::MINECART,
            val if val == Item::TNT_MINECART.id => &EntityType::TNT_MINECART,
            val if val == Item::CHEST_MINECART.id => &EntityType::CHEST_MINECART,
            val if val == Item::HOPPER_MINECART.id => &EntityType::HOPPER_MINECART,
            val if val == Item::FURNACE_MINECART.id => &EntityType::FURNACE_MINECART,
            val if val == Item::COMMAND_BLOCK_MINECART.id => &EntityType::COMMAND_BLOCK_MINECART,
            _ => {
                tracing::error!("Unknown minecart item ID: {}", item.id);
                &EntityType::MINECART
            }
        }
    }
}

impl ItemMetadata for MinecartItem {
    fn ids() -> Box<[u16]> {
        [
            Item::MINECART.id,
            Item::TNT_MINECART.id,
            Item::CHEST_MINECART.id,
            Item::HOPPER_MINECART.id,
            Item::FURNACE_MINECART.id,
            Item::COMMAND_BLOCK_MINECART.id,
        ]
        .into()
    }
}

impl ItemBehaviour for MinecartItem {
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
        _face: BlockDirection,
        _cursor_pos: Vector3<f32>,
        block: &Block,
        server: &Server,
        hand: Hand,
    ) -> BlockActionResult {
        let source = crate::net::java::play::hand_use_result::hand_slot(player, hand);
        let spawned =
            MinecartEntity::place_from_item(item, player, location, block, server, hand, source);
        player.sync_hand_slot(source, player.inventory().get_stack(source));
        if spawned {
            BlockActionResult::Success
        } else {
            BlockActionResult::Fail
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
#[path = "minecart_creation_tests.rs"]
mod creation_tests;
