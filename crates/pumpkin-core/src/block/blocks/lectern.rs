use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::block::entities::BlockEntity;
use crate::block::entities::lectern::LecternBlockEntity;
use crate::block::registry::BlockActionResult;
use crate::block::{
    BlockBehaviour, EmitsRedstonePowerArgs, GetComparatorOutputArgs, GetRedstonePowerArgs,
    GetScreenHandlerFactoryArgs, NormalUseArgs, OnPlaceArgs, OnScheduledTickArgs,
    OnStateReplacedArgs, PathComputationType, PlacedArgs, UseWithItemArgs,
};
use crate::world::World;
use crate::world::guard_effects::defer;
use pumpkin_data::block_properties::LecternLikeProperties;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::Taggable;
use pumpkin_data::world::WorldEvent;
use pumpkin_data::{Block, BlockDirection, BlockState, BlockStateId, tag, translation};
use pumpkin_inventory::Inventory;
use pumpkin_inventory::lectern_screen_handler::{LecternController, LecternScreenHandler};
use pumpkin_inventory::player::player_inventory::PlayerInventory;
use pumpkin_inventory::screen_handler::ScreenMutex;
use pumpkin_inventory::screen_handler::{
    InventoryPlayer, ScreenHandlerFactory, SharedScreenHandler,
};
use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::text::TextComponent;
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::BlockFlags;

#[path = "lectern_book.rs"]
mod book;

/// Bridges the screen handler back into the world: page changes emit the
/// vanilla redstone pulse and taking the book clears `has_book`.
struct LecternPageController {
    world: Arc<World>,
    position: BlockPos,
    block_entity: Arc<dyn BlockEntity>,
}

impl LecternPageController {
    fn entity(&self) -> Option<&LecternBlockEntity> {
        self.block_entity
            .as_any()
            .downcast_ref::<LecternBlockEntity>()
    }
}

impl LecternController for LecternPageController {
    fn current_page(&self) -> i32 {
        self.entity()
            .map_or(0, |entity| entity.page.load(Ordering::Relaxed) as i32)
    }

    fn set_page(&self, page: i32) {
        let world = self.world.clone();
        let block_entity = self.block_entity.clone();
        let position = self.position;
        // LecternMenu.setData / LecternBlockEntity.setPage commit after the menu unlocks.
        let effect = move || {
            let Some(mutation) = world.block_entity_mutation_permit(position) else {
                return;
            };
            if !world.is_live_block_entity(&block_entity) {
                return;
            }
            let Some(entity) = block_entity.as_any().downcast_ref::<LecternBlockEntity>() else {
                return;
            };
            let page_count = entity.page_count();
            let page = page.clamp(0, (page_count - 1).max(0));
            if page == entity.page.load(Ordering::Relaxed) as i32 {
                return;
            }
            entity.page.store(page as usize, Ordering::Relaxed);
            entity.mark_dirty();
            let update = world.prepare_lectern_state_update(position, true, None);
            drop(mutation);
            if let Some(update) = update {
                update();
                LecternBlock::finish_page_turn(&world, &position);
            }
        };
        if let Err(effect) = defer(effect) {
            effect();
        }
    }

    fn on_book_taken(&self) {
        let world = self.world.clone();
        let block_entity = self.block_entity.clone();
        let position = self.position;
        let effect = move || {
            let Some(mutation) = world.block_entity_mutation_permit(position) else {
                return;
            };
            if !world.is_live_block_entity(&block_entity) {
                return;
            }
            if let Some(entity) = block_entity.as_any().downcast_ref::<LecternBlockEntity>() {
                entity.page.store(0, Ordering::Relaxed);
            }
            let update = world.prepare_lectern_state_update(position, false, Some(false));
            drop(mutation);
            if let Some(update) = update {
                update();
                LecternBlock::update_neighbors_below(&world, &position);
            }
        };
        if let Err(effect) = defer(effect) {
            effect();
        }
    }
}

struct LecternScreenFactory {
    inventory: Arc<dyn Inventory>,
    controller: Arc<dyn LecternController>,
}

impl ScreenHandlerFactory for LecternScreenFactory {
    fn create_screen_handler(
        &self,
        sync_id: u8,
        _player_inventory: &Arc<PlayerInventory>,
        _player: &dyn InventoryPlayer,
    ) -> Option<SharedScreenHandler> {
        let handler =
            LecternScreenHandler::new(sync_id, self.inventory.clone(), self.controller.clone());
        Some(Arc::new(ScreenMutex::new(handler)) as SharedScreenHandler)
    }

    fn get_display_name(&self) -> TextComponent {
        pumpkin_macros::translate_cross!(
            translation::java::CONTAINER_LECTERN,
            translation::bedrock::TILE_LECTERN_NAME
        )
    }
}

#[pumpkin_block("minecraft:lectern")]
pub struct LecternBlock;

impl LecternBlock {
    /// Vanilla pulse length of a page-turn signal, in game ticks.
    const PAGE_TURN_PULSE_TICKS: u8 = 2;

    /// The lectern strongly powers the block below it, so its neighbors need
    /// updating whenever the power or book state changes.
    fn update_neighbors_below(world: &Arc<World>, position: &BlockPos) {
        world.update_neighbors(&position.down(), None);
    }

    /// Notifies the vanilla two-tick page-turn pulse after mutation admission ends.
    fn finish_page_turn(world: &Arc<World>, position: &BlockPos) {
        Self::update_neighbors_below(world, position);
        world.schedule_block_tick(
            &Block::LECTERN,
            *position,
            Self::PAGE_TURN_PULSE_TICKS,
            TickPriority::Normal,
        );
        world.sync_world_event(WorldEvent::SoundPageTurn, *position, 0);
    }
}

impl BlockBehaviour for LecternBlock {
    fn placed(&self, args: PlacedArgs<'_>) {
        let block_entity = LecternBlockEntity::new(*args.position);
        args.world
            .add_block_entity_from_observed(Arc::new(block_entity), args.state_id);
    }

    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let mut props = LecternLikeProperties::default(args.block);
        props.facing = args
            .player
            .living_entity
            .entity
            .get_horizontal_facing()
            .opposite();
        props.to_state_id(args.block)
    }

    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        self.get_screen_handler_factory(GetScreenHandlerFactoryArgs {
            server: args.server,
            world: args.world,
            block: args.block,
            position: args.position,
            player: args.player,
        })
        .map_or(BlockActionResult::Pass, |factory| {
            args.player.increment_stat(
                pumpkin_data::statistic::StatisticCategory::Custom,
                pumpkin_data::statistic::CustomStatistic::InteractWithLectern as i32,
                1,
            );

            args.player
                .open_handled_screen(factory.as_ref(), Some(*args.position));

            BlockActionResult::Success
        })
    }

    fn get_screen_handler_factory(
        &self,
        args: GetScreenHandlerFactoryArgs<'_>,
    ) -> Option<Box<dyn ScreenHandlerFactory>> {
        let props =
            LecternLikeProperties::from_state_id(args.world.get_block_state(args.position).id);
        if !props.has_book {
            return None;
        }

        let block_entity = args.world.get_block_entity(args.position)?;
        let inventory = block_entity.clone().get_inventory()?;

        let controller = Arc::new(LecternPageController {
            world: args.world.clone(),
            position: *args.position,
            block_entity,
        });

        Some(Box::new(LecternScreenFactory {
            inventory,
            controller,
        }))
    }

    fn use_with_item(&self, mut args: UseWithItemArgs<'_>) -> BlockActionResult {
        if !args
            .item_stack
            .item
            .has_tag(&tag::Item::MINECRAFT_LECTERN_BOOKS)
        {
            return BlockActionResult::PassToDefaultBlockAction;
        }
        let observed = args.world.get_block_state_id(args.position);
        if observed.to_block() != args.block {
            return BlockActionResult::PassToDefaultBlockAction;
        }
        let props = LecternLikeProperties::from_state_id(observed);
        if props.has_book || !book::place_book(&mut args, observed) {
            return BlockActionResult::PassToDefaultBlockAction;
        }
        args.world
            .play_block_sound(Sound::ItemBookPut, SoundCategory::Blocks, *args.position);
        BlockActionResult::Success
    }

    fn on_scheduled_tick(&self, args: OnScheduledTickArgs<'_>) {
        let observed = args.world.get_block_state_id(args.position);
        if observed.to_block() != args.block {
            return;
        }
        let mut props = LecternLikeProperties::from_state_id(observed);
        props.powered = false;
        args.world.set_block_state_from_observed(
            args.position,
            observed,
            props.to_state_id(args.block),
            BlockFlags::NOTIFY_ALL,
        );
    }

    fn emits_redstone_power(&self, _args: EmitsRedstonePowerArgs<'_>) -> bool {
        true
    }

    fn get_weak_redstone_power(&self, args: GetRedstonePowerArgs<'_>) -> u8 {
        let props = LecternLikeProperties::from_state_id(args.state.id);
        if props.powered { 15 } else { 0 }
    }

    fn get_strong_redstone_power(&self, args: GetRedstonePowerArgs<'_>) -> u8 {
        let props = LecternLikeProperties::from_state_id(args.state.id);
        if props.powered && args.direction == BlockDirection::Up {
            15
        } else {
            0
        }
    }

    fn on_state_replaced(&self, args: OnStateReplacedArgs<'_>) {
        if !args.moved {
            let props = LecternLikeProperties::from_state_id(args.old_state_id);
            if props.powered {
                Self::update_neighbors_below(args.world, args.position);
            }
        }
    }

    fn get_comparator_output(&self, args: GetComparatorOutputArgs<'_>) -> Option<u8> {
        if let Some(block_entity) = args.world.get_block_entity(args.position)
            && let Some(lectern_entity) = block_entity.as_any().downcast_ref::<LecternBlockEntity>()
        {
            Some(lectern_entity.comparator_output())
        } else {
            Some(0)
        }
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}
