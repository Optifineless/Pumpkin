use pumpkin_inventory::screen_handler::ScreenMutex;
use std::sync::Arc;

use pumpkin_data::translation;
use pumpkin_inventory::beacon_screen_handler::create_beacon_handler;
use pumpkin_inventory::player::player_inventory::PlayerInventory;
use pumpkin_inventory::screen_handler::{
    InventoryPlayer, ScreenHandlerFactory, SharedScreenHandler,
};
use pumpkin_inventory::window_property::PropertyDelegate;
use pumpkin_macros::pumpkin_block;
use pumpkin_util::text::TextComponent;

use crate::block::entities::{BlockEntity, beacon::BeaconBlockEntity};
use crate::block::registry::BlockActionResult;
use crate::block::{BlockBehaviour, GetScreenHandlerFactoryArgs, NormalUseArgs};

struct BeaconScreenFactory(Arc<dyn BlockEntity>);

struct BeaconPropertyDelegate(Arc<dyn BlockEntity>);

impl PropertyDelegate for BeaconPropertyDelegate {
    fn get_property(&self, index: i32) -> i32 {
        if index >= 0 && index < BeaconBlockEntity::NUM_DATA_VALUES as i32 {
            self.0
                .as_any()
                .downcast_ref::<BeaconBlockEntity>()
                .map_or(0, |beacon| beacon.get_data(index as usize))
        } else {
            0
        }
    }

    fn set_property(&self, index: i32, value: i32) {
        if index >= 0
            && index < BeaconBlockEntity::NUM_DATA_VALUES as i32
            && let Some(beacon) = self.0.as_any().downcast_ref::<BeaconBlockEntity>()
        {
            beacon.set_data(index as usize, value);
        }
    }

    fn get_properties_size(&self) -> i32 {
        BeaconBlockEntity::NUM_DATA_VALUES as i32
    }
}

impl ScreenHandlerFactory for BeaconScreenFactory {
    fn create_screen_handler(
        &self,
        sync_id: u8,
        player_inventory: &Arc<PlayerInventory>,
        _player: &dyn InventoryPlayer,
    ) -> Option<SharedScreenHandler> {
        let beacon_data: Arc<dyn PropertyDelegate> =
            Arc::new(BeaconPropertyDelegate(self.0.clone()));
        let concrete_handler = create_beacon_handler(sync_id, player_inventory, beacon_data);
        let concrete_arc = Arc::new(ScreenMutex::new(concrete_handler));

        Some(concrete_arc as SharedScreenHandler)
    }

    fn get_display_name(&self) -> TextComponent {
        pumpkin_macros::translate_cross!(
            translation::java::CONTAINER_BEACON,
            translation::bedrock::CONTAINER_BEACON
        )
    }
}

#[pumpkin_block("minecraft:beacon")]
pub struct BeaconBlock;

impl BlockBehaviour for BeaconBlock {
    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        self.get_screen_handler_factory(GetScreenHandlerFactoryArgs {
            server: args.server,
            world: args.world,
            block: args.block,
            position: args.position,
            player: args.player,
        })
        .map_or(BlockActionResult::Fail, |factory| {
            args.player.increment_stat(
                pumpkin_data::statistic::StatisticCategory::Custom,
                pumpkin_data::statistic::CustomStatistic::InteractWithBeacon as i32,
                1,
            );

            // Open the screen using the factory
            args.player
                .open_handled_screen(factory.as_ref(), Some(*args.position));

            BlockActionResult::Success
        })
    }

    fn get_screen_handler_factory(
        &self,
        args: GetScreenHandlerFactoryArgs<'_>,
    ) -> Option<Box<dyn ScreenHandlerFactory>> {
        let block_entity = args.world.get_block_entity(args.position)?;
        block_entity.as_any().downcast_ref::<BeaconBlockEntity>()?;
        Some(Box::new(BeaconScreenFactory(block_entity)))
    }
}
