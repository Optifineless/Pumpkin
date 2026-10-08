use std::sync::Arc;
use std::sync::Mutex;

use crate::block::{
    GetComparatorOutputArgs, GetScreenHandlerFactoryArgs, OnPlaceArgs, OnSyncedBlockEventArgs,
    PlacedArgs,
};
use crate::block::{
    registry::BlockActionResult,
    {BlockBehaviour, NormalUseArgs},
};

use crate::block::entities::shulker_box::ShulkerBoxBlockEntity;
use pumpkin_data::BlockStateId;
use pumpkin_data::translation;
use pumpkin_inventory::Inventory;
use pumpkin_inventory::player::player_inventory::PlayerInventory;
use pumpkin_inventory::screen_handler::{
    InventoryPlayer, ScreenHandlerFactory, SharedScreenHandler,
};
use pumpkin_inventory::shulker_box_screen_handler::ShulkerBoxScreenHandler;
use pumpkin_macros::pumpkin_block_from_tag;
use pumpkin_util::text::TextComponent;

struct ShulkerBoxScreenFactory {
    inventory: Arc<dyn Inventory>,
    world: std::sync::Weak<crate::world::World>,
    entity: Arc<dyn crate::block::entities::BlockEntity>,
}

// Container.DEFAULT_DISTANCE_BUFFER.
const DEFAULT_DISTANCE_BUFFER: f64 = 4.0;

impl ScreenHandlerFactory for ShulkerBoxScreenFactory {
    fn create_screen_handler(
        &self,
        sync_id: u8,
        player_inventory: &Arc<PlayerInventory>,
        player: &dyn InventoryPlayer,
    ) -> Option<SharedScreenHandler> {
        let mut handler =
            ShulkerBoxScreenHandler::new(sync_id, player_inventory, self.inventory.clone(), player);
        let world = self.world.clone();
        let entity = self.entity.clone();
        handler.validity_check = Some(Box::new(move |player| {
            let Some(player) = player
                .as_any()
                .downcast_ref::<crate::entity::player::Player>()
            else {
                return false;
            };
            let Some(world) = world.upgrade() else {
                return false;
            };
            // Container.stillValidBlockEntity: identity, level, then interaction range + 4.
            Arc::ptr_eq(&world, &player.world())
                && world
                    .get_block_entity(&entity.get_position())
                    .is_some_and(|live| Arc::ptr_eq(&live, &entity))
                && player
                    .can_interact_with_block_at(&entity.get_position(), DEFAULT_DISTANCE_BUFFER)
        }));
        let screen_handler_arc = Arc::new(Mutex::new(handler));

        Some(screen_handler_arc as SharedScreenHandler)
    }

    fn get_display_name(&self) -> TextComponent {
        pumpkin_macros::translate_cross!(
            translation::java::CONTAINER_SHULKERBOX,
            translation::bedrock::CONTAINER_SHULKERBOX
        )
    }
}

#[pumpkin_block_from_tag("minecraft:shulker_boxes")]
pub struct ShulkerBoxBlock;

type EndRodLikeProperties = pumpkin_data::block_properties::EndRodLikeProperties;

impl BlockBehaviour for ShulkerBoxBlock {
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let mut props = EndRodLikeProperties::default(args.block);
        props.facing = args.direction.to_facing().opposite();
        props.to_state_id(args.block)
    }

    fn on_synced_block_event(&self, args: OnSyncedBlockEventArgs<'_>) -> bool {
        // On the server, we don't need the Animation steps for now, because the client is responsible for that.
        // TODO: Do not open the shulker box when it is currently closing
        args.r#type == Self::OPEN_ANIMATION_EVENT_TYPE
    }

    fn placed(&self, args: PlacedArgs<'_>) {
        {
            let barrel_block_entity = ShulkerBoxBlockEntity::new(*args.position);
            args.world.add_block_entity(Arc::new(barrel_block_entity));
        }
    }

    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        if let Some(factory) = self.get_screen_handler_factory(GetScreenHandlerFactoryArgs {
            server: args.server,
            world: args.world,
            block: args.block,
            position: args.position,
            player: args.player,
        }) {
            args.player.increment_stat(
                pumpkin_data::statistic::StatisticCategory::Custom,
                pumpkin_data::statistic::CustomStatistic::OpenShulkerBox as i32,
                1,
            );
            args.player
                .open_handled_screen(factory.as_ref(), Some(*args.position));
        }

        BlockActionResult::Success
    }

    fn get_screen_handler_factory(
        &self,
        args: GetScreenHandlerFactoryArgs<'_>,
    ) -> Option<Box<dyn ScreenHandlerFactory>> {
        let block_entity = args.world.get_block_entity(args.position)?;
        if !crate::block::entities::container_lock::can_open(
            &*block_entity,
            args.player,
            args.world,
            "container.shulkerBox",
        ) {
            return None;
        }
        let inventory = block_entity.clone().get_inventory()?;
        Some(Box::new(ShulkerBoxScreenFactory {
            inventory,
            world: Arc::downgrade(args.world),
            entity: block_entity,
        }))
    }

    fn get_comparator_output(&self, args: GetComparatorOutputArgs<'_>) -> Option<u8> {
        crate::block::container_comparator_output(&args)
    }
}

impl ShulkerBoxBlock {
    pub const OPEN_ANIMATION_EVENT_TYPE: u8 = 1;
}

#[cfg(test)]
#[path = "shulker_box_tests.rs"]
mod tests;
