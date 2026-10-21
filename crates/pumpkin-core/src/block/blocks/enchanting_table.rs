use pumpkin_inventory::screen_handler::ScreenMutex;
use std::sync::Arc;

use crate::block::entities::enchanting_table::EnchantingTableBlockEntity;
use crate::block::registry::BlockActionResult;
use crate::block::{
    BlockBehaviour, GetScreenHandlerFactoryArgs, NormalUseArgs, PathComputationType, PlacedArgs,
};
use pumpkin_data::{
    Block, BlockState,
    tag::{Block as BlockTag, Taggable},
    translation,
};
use pumpkin_inventory::enchanting::enchanting_screen_handler::EnchantingTableScreenHandler;
use pumpkin_inventory::player::player_inventory::PlayerInventory;
use pumpkin_inventory::screen_handler::{
    InventoryPlayer, ScreenHandlerFactory, SharedScreenHandler,
};
use pumpkin_inventory::{Inventory, SimpleInventory};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::text::TextComponent;

#[pumpkin_block("minecraft:enchanting_table")]
pub struct EnchantingTableBlock;

impl BlockBehaviour for EnchantingTableBlock {
    fn placed(&self, args: PlacedArgs<'_>) {
        let entity = EnchantingTableBlockEntity::new(*args.position);
        args.world
            .add_block_entity_from_observed(Arc::new(entity), args.state_id);
    }

    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        if let Some(factory) = self.get_screen_handler_factory(GetScreenHandlerFactoryArgs {
            server: args.server,
            world: args.world,
            block: args.block,
            position: args.position,
            player: args.player,
        }) {
            args.player
                .open_handled_screen(factory.as_ref(), Some(*args.position));
        }
        BlockActionResult::Success
    }

    fn get_screen_handler_factory(
        &self,
        args: GetScreenHandlerFactoryArgs<'_>,
    ) -> Option<Box<dyn ScreenHandlerFactory>> {
        let mut bookshelf_count = 0;

        // Mirrors EnchantingTableBlock.BOOKSHELF_OFFSETS and isValidBookShelf.
        // Each provider offset checks only its matching-height transmitter.
        for off_y in 0..=1 {
            for off_z in -1..=1 {
                for off_x in -1..=1 {
                    if off_x == 0 && off_z == 0 {
                        continue;
                    }
                    if Self::is_valid_bookshelf(
                        args.world,
                        args.position,
                        off_x * 2,
                        off_y,
                        off_z * 2,
                    ) {
                        bookshelf_count += 1;
                    }
                    if off_x != 0 && off_z != 0 {
                        if Self::is_valid_bookshelf(
                            args.world,
                            args.position,
                            off_x * 2,
                            off_y,
                            off_z,
                        ) {
                            bookshelf_count += 1;
                        }
                        if Self::is_valid_bookshelf(
                            args.world,
                            args.position,
                            off_x,
                            off_y,
                            off_z * 2,
                        ) {
                            bookshelf_count += 1;
                        }
                    }
                }
            }
        }
        let bookshelf_count = bookshelf_count.min(15);

        let seed = args.player.enchantment_seed();
        Some(Box::new(EnchantingTableScreenFactory {
            bookshelf_count,
            seed,
        }))
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}

impl EnchantingTableBlock {
    fn is_valid_bookshelf(
        world: &Arc<crate::world::World>,
        table_pos: &BlockPos,
        offset_x: i32,
        offset_y: i32,
        offset_z: i32,
    ) -> bool {
        let provider = Block::from_state_id(
            world
                .get_block_state(&table_pos.add(offset_x, offset_y, offset_z))
                .id,
        );
        let transmitter = Block::from_state_id(
            world
                .get_block_state(&table_pos.add(offset_x / 2, offset_y, offset_z / 2))
                .id,
        );
        provider.has_tag(&BlockTag::MINECRAFT_ENCHANTMENT_POWER_PROVIDER)
            && transmitter.has_tag(&BlockTag::MINECRAFT_ENCHANTMENT_POWER_TRANSMITTER)
    }
}

#[cfg(test)]
mod tests;

struct EnchantingTableScreenFactory {
    bookshelf_count: i32,
    seed: i32,
}

impl ScreenHandlerFactory for EnchantingTableScreenFactory {
    fn create_screen_handler(
        &self,
        sync_id: u8,
        player_inventory: &Arc<PlayerInventory>,
        _player: &dyn InventoryPlayer,
    ) -> Option<SharedScreenHandler> {
        let inventory: Arc<dyn Inventory> = Arc::new(SimpleInventory::new(2));
        let handler = EnchantingTableScreenHandler::new(
            sync_id,
            player_inventory,
            &inventory,
            self.seed,
            self.bookshelf_count,
        );
        let screen_handler_arc = Arc::new(ScreenMutex::new(handler));
        Some(screen_handler_arc as SharedScreenHandler)
    }

    fn get_display_name(&self) -> TextComponent {
        pumpkin_macros::translate_cross!(
            translation::java::CONTAINER_ENCHANT,
            translation::bedrock::CONTAINER_ENCHANT
        )
    }
}
