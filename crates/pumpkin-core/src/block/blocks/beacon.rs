use pumpkin_inventory::screen_handler::ScreenMutex;
use std::sync::Arc;

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

struct BeaconScreenFactory {
    world: std::sync::Weak<crate::world::World>,
    entity: Arc<dyn BlockEntity>,
}

// Container.DEFAULT_DISTANCE_BUFFER.
const DEFAULT_DISTANCE_BUFFER: f64 = 4.0;

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
        player: &dyn InventoryPlayer,
    ) -> Option<SharedScreenHandler> {
        let player = player
            .as_any()
            .downcast_ref::<crate::entity::player::Player>()?;
        let world = self.world.upgrade()?;
        // BeaconBlockEntity.createMenu checks the main-hand key after the open callback.
        if !crate::block::entities::container_lock::can_open_named(
            self.entity.as_ref(),
            player,
            &world,
            self.get_display_name(),
        ) {
            return None;
        }
        let beacon_data: Arc<dyn PropertyDelegate> =
            Arc::new(BeaconPropertyDelegate(self.entity.clone()));
        let mut concrete_handler = create_beacon_handler(sync_id, player_inventory, beacon_data);
        concrete_handler.block_entity = Some(self.entity.clone());
        let world = self.world.clone();
        let entity = self.entity.clone();
        // BeaconMenu.stillValid: a broken or replaced beacon closes the menu, so the payment
        // is dropped instead of being spent on a detached block entity.
        concrete_handler.validity_check = Some(Box::new(move |player| {
            let Some(player) = player
                .as_any()
                .downcast_ref::<crate::entity::player::Player>()
            else {
                return false;
            };
            let Some(world) = world.upgrade() else {
                return false;
            };
            Arc::ptr_eq(&world, &player.world())
                && world
                    .get_block_entity(&entity.get_position())
                    .is_some_and(|live| Arc::ptr_eq(&live, &entity))
                && player
                    .can_interact_with_block_at(&entity.get_position(), DEFAULT_DISTANCE_BUFFER)
        }));
        let concrete_arc = Arc::new(ScreenMutex::new(concrete_handler));

        Some(concrete_arc as SharedScreenHandler)
    }

    fn get_display_name(&self) -> TextComponent {
        self.entity
            .as_any()
            .downcast_ref::<BeaconBlockEntity>()
            .map_or_else(TextComponent::empty, BeaconBlockEntity::menu_display_name)
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
        Some(Box::new(BeaconScreenFactory {
            world: Arc::downgrade(args.world),
            entity: block_entity,
        }))
    }
}
