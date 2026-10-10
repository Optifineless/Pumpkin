use std::sync::Arc;

use crate::block::{UseWithItemArgs, registry::BlockActionResult};
use crate::entity::EntityBase;
use crate::item::item_utils::create_filled_result;
use crate::net::java::play::hand_use_result::{HandMutation, hand_slot, write_back_hand_item};
use crate::plugin::block::cauldron_level_change::{CauldronChangeReason, CauldronLevelChangeEvent};
use pumpkin_data::block_properties::WaterCauldronLikeProperties;
use pumpkin_data::data_component::DataComponent;
use pumpkin_data::data_component_impl::{
    BannerPatternsImpl, DyedColorImpl, EquipmentSlot, PotionContentsImpl,
};
use pumpkin_data::fluid::Fluid;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::statistic::{CustomStatistic, StatisticCategory};
use pumpkin_data::tag::Taggable;
use pumpkin_data::{Block, BlockId, BlockStateId};
use pumpkin_inventory::screen_handler::InventoryPlayer;
use pumpkin_util::Hand;
use pumpkin_world::world::BlockFlags;

struct HandContext {
    hand: Hand,
    source_slot: usize,
    before: ItemStack,
}

impl HandContext {
    fn capture(args: &UseWithItemArgs<'_>) -> Self {
        let hand = if args.equipment_slot == &EquipmentSlot::MAIN_HAND {
            Hand::Right
        } else {
            Hand::Left
        };
        Self {
            hand,
            source_slot: hand_slot(args.player, hand),
            before: args.item_stack.clone(),
        }
    }

    // CauldronInteractions updates the live hand before state changes and game-event callbacks.
    fn publish(&self, args: &UseWithItemArgs<'_>) {
        write_back_hand_item(
            args.player,
            self.hand,
            self.source_slot,
            &self.before,
            args.item_stack,
            HandMutation::ItemUse,
        );
    }
}

pub(super) fn interact(mut args: UseWithItemArgs<'_>) -> BlockActionResult {
    // Capture before the cancellable event: a plugin may replace the hand or select another slot.
    let hand = HandContext::capture(&args);
    // CauldronInteractions.addDefaultInteractions registers filled buckets on all four maps.
    if let Some(result) = try_empty_bucket(&mut args, &hand) {
        return result;
    }
    let item = args.item_stack.get_item();
    if item == &Item::BUCKET {
        return fill_bucket(&mut args, &hand);
    }
    if item == &Item::POTION && matches!(args.block.id, BlockId::CAULDRON | BlockId::WATER_CAULDRON)
    {
        return empty_bottle(&mut args, &hand);
    }
    if args.block.id != BlockId::WATER_CAULDRON {
        return BlockActionResult::PassToDefaultBlockAction;
    }
    if item == &Item::GLASS_BOTTLE {
        return fill_bottle(&mut args, &hand);
    }
    if item.has_tag(&pumpkin_data::tag::Item::MINECRAFT_SHULKER_BOXES) && item != &Item::SHULKER_BOX
    {
        return shulker_box_interaction(&mut args, &hand);
    }
    if item.has_tag(&pumpkin_data::tag::Item::MINECRAFT_BANNERS) {
        return banner_interaction(&mut args, &hand);
    }
    if item.has_tag(&pumpkin_data::tag::Item::MINECRAFT_CAULDRON_CAN_REMOVE_DYE) {
        return dyed_item_interaction(&mut args, &hand);
    }
    BlockActionResult::PassToDefaultBlockAction
}

fn fill_level(state: BlockStateId) -> u8 {
    match state.to_block_id() {
        BlockId::WATER_CAULDRON | BlockId::POWDER_SNOW_CAULDRON => {
            WaterCauldronLikeProperties::from_state_id(state).level
        }
        BlockId::LAVA_CAULDRON => 3,
        _ => 0,
    }
}

fn water_state(level: u8) -> BlockStateId {
    if level == 0 {
        Block::CAULDRON.default_state.id
    } else {
        WaterCauldronLikeProperties { level }.to_state_id(&Block::WATER_CAULDRON)
    }
}

fn allow_change(args: &UseWithItemArgs<'_>, new_level: u8, reason: CauldronChangeReason) -> bool {
    let mut event = CauldronLevelChangeEvent::new(
        *args.position,
        args.world.clone(),
        i32::from(fill_level(args.world.get_block_state_id(args.position))),
        i32::from(new_level),
        reason,
        Some(Arc::clone(args.player) as Arc<dyn EntityBase>),
    );
    if let Some(server) = args.world.server.upgrade() {
        server.plugin_manager.fire_blocking(&server, &mut event);
    }
    !event.cancelled
}

fn award_custom(args: &UseWithItemArgs<'_>, statistic: CustomStatistic) {
    args.player
        .increment_stat(StatisticCategory::Custom, statistic as i32, 1);
}

fn exchange_item(
    args: &mut UseWithItemArgs<'_>,
    hand: &HandContext,
    output: ItemStack,
    statistic: CustomStatistic,
) {
    // WATER's potion lambda reads the consumed input alias; EMPTY snapshots its item first.
    let item_id = if args.block.id == BlockId::WATER_CAULDRON
        && args.item_stack.get_item() == &Item::POTION
        && args.item_stack.item_count == 1
        && !args.player.has_infinite_materials()
    {
        Item::AIR.id
    } else {
        args.item_stack.get_item().id
    };
    create_filled_result(args.item_stack, args.player, output, true);
    hand.publish(args);
    award_custom(args, statistic);
    args.player
        .increment_stat(StatisticCategory::Used, i32::from(item_id), 1);
}

fn set_state(args: &UseWithItemArgs<'_>, state: BlockStateId) {
    args.world
        .set_block_state(args.position, state, BlockFlags::NOTIFY_ALL);
}

fn fluid_effects(args: &UseWithItemArgs<'_>, sound: Sound, event: GameEvent) {
    args.world
        .play_block_sound(sound, SoundCategory::Blocks, *args.position);
    args.world
        .emit_game_event(event.name(), args.position.to_centered_f64());
}

fn try_empty_bucket(
    args: &mut UseWithItemArgs<'_>,
    hand: &HandContext,
) -> Option<BlockActionResult> {
    let item = args.item_stack.get_item();
    let (state, sound) = if item == &Item::WATER_BUCKET {
        (water_state(3), Sound::ItemBucketEmpty)
    } else if item == &Item::LAVA_BUCKET {
        (
            Block::LAVA_CAULDRON.default_state.id,
            Sound::ItemBucketEmptyLava,
        )
    } else if item == &Item::POWDER_SNOW_BUCKET {
        (
            WaterCauldronLikeProperties { level: 3 }.to_state_id(&Block::POWDER_SNOW_CAULDRON),
            Sound::ItemBucketEmptyPowderSnow,
        )
    } else {
        return None;
    };
    // fillLavaInteraction / fillPowderSnowInteraction consume underwater use without effects.
    if item != &Item::WATER_BUCKET
        && Fluid::WATER.matches_type(args.world.get_fluid(&args.position.up()))
    {
        return Some(BlockActionResult::Consume);
    }
    Some(empty_bucket(args, hand, state, sound))
}

// CauldronInteractions.emptyBucket exchanges even when the destination is already full.
fn empty_bucket(
    args: &mut UseWithItemArgs<'_>,
    hand: &HandContext,
    state: BlockStateId,
    sound: Sound,
) -> BlockActionResult {
    if !allow_change(args, 3, CauldronChangeReason::BucketEmpty) {
        return BlockActionResult::Consume;
    }
    exchange_item(
        args,
        hand,
        ItemStack::new(1, &Item::BUCKET),
        CustomStatistic::FillCauldron,
    );
    set_state(args, state);
    fluid_effects(args, sound, GameEvent::FluidPlace);
    BlockActionResult::Success
}

// CauldronInteractions.fillBucket requires a full layered cauldron, or any lava cauldron.
fn fill_bucket(args: &mut UseWithItemArgs<'_>, hand: &HandContext) -> BlockActionResult {
    let level = fill_level(args.world.get_block_state_id(args.position));
    let (item, sound) = match args.block.id {
        BlockId::WATER_CAULDRON if level == 3 => (&Item::WATER_BUCKET, Sound::ItemBucketFill),
        BlockId::LAVA_CAULDRON => (&Item::LAVA_BUCKET, Sound::ItemBucketFillLava),
        BlockId::POWDER_SNOW_CAULDRON if level == 3 => {
            (&Item::POWDER_SNOW_BUCKET, Sound::ItemBucketFillPowderSnow)
        }
        _ => return BlockActionResult::PassToDefaultBlockAction,
    };
    if !allow_change(args, 0, CauldronChangeReason::BucketFill) {
        return BlockActionResult::Consume;
    }
    exchange_item(
        args,
        hand,
        ItemStack::new(1, item),
        CustomStatistic::UseCauldron,
    );
    set_state(args, Block::CAULDRON.default_state.id);
    fluid_effects(args, sound, GameEvent::FluidPickup);
    BlockActionResult::Success
}

// CauldronInteractions.bootStrap accepts only PotionContents.is(WATER).
fn empty_bottle(args: &mut UseWithItemArgs<'_>, hand: &HandContext) -> BlockActionResult {
    let level = fill_level(args.world.get_block_state_id(args.position));
    let is_water = args
        .item_stack
        .get_data_component::<PotionContentsImpl>()
        .is_some_and(|contents| {
            contents.potion_id == Some(i32::from(pumpkin_data::potion::Potion::WATER.id))
                && contents.custom_effects.is_empty()
        });
    if level == 3 || !is_water {
        return BlockActionResult::PassToDefaultBlockAction;
    }
    if !allow_change(args, level + 1, CauldronChangeReason::BottleEmpty) {
        return BlockActionResult::Consume;
    }
    exchange_item(
        args,
        hand,
        ItemStack::new(1, &Item::GLASS_BOTTLE),
        CustomStatistic::UseCauldron,
    );
    set_state(args, water_state(level + 1));
    fluid_effects(args, Sound::ItemBottleEmpty, GameEvent::FluidPlace);
    BlockActionResult::Success
}

fn fill_bottle(args: &mut UseWithItemArgs<'_>, hand: &HandContext) -> BlockActionResult {
    let level = fill_level(args.world.get_block_state_id(args.position)) - 1;
    if !allow_change(args, level, CauldronChangeReason::BottleFill) {
        return BlockActionResult::Consume;
    }
    exchange_item(
        args,
        hand,
        crate::item::items::glass_bottle::water_bottle(),
        CustomStatistic::UseCauldron,
    );
    lower_fill_level(args, level);
    fluid_effects(args, Sound::ItemBottleFill, GameEvent::FluidPickup);
    BlockActionResult::Success
}

// LayeredCauldronBlock.lowerFillLevel notifies the new block state once.
fn lower_fill_level(args: &UseWithItemArgs<'_>, new_level: u8) {
    set_state(args, water_state(new_level));
    args.world.emit_game_event(
        GameEvent::BlockChange.name(),
        args.position.to_centered_f64(),
    );
}

// CauldronInteractions.shulkerBoxInteraction uses a one-item transmuted copy and unlimited output.
fn shulker_box_interaction(
    args: &mut UseWithItemArgs<'_>,
    hand: &HandContext,
) -> BlockActionResult {
    let level = fill_level(args.world.get_block_state_id(args.position)) - 1;
    if !allow_change(args, level, CauldronChangeReason::Unknown) {
        return BlockActionResult::Consume;
    }
    let mut cleaned = args.item_stack.copy_with_count(1);
    cleaned.item = &Item::SHULKER_BOX;
    create_filled_result(args.item_stack, args.player, cleaned, false);
    hand.publish(args);
    award_custom(args, CustomStatistic::CleanShulkerBox);
    lower_fill_level(args, level);
    BlockActionResult::Success
}

fn banner_interaction(args: &mut UseWithItemArgs<'_>, hand: &HandContext) -> BlockActionResult {
    if args
        .item_stack
        .get_data_component::<BannerPatternsImpl>()
        .is_none_or(|patterns| patterns.layers.is_empty())
    {
        return BlockActionResult::PassToDefaultBlockAction;
    }
    let level = fill_level(args.world.get_block_state_id(args.position)) - 1;
    if !allow_change(args, level, CauldronChangeReason::Unknown) {
        return BlockActionResult::Consume;
    }
    // CauldronInteractions.bannerInteraction removes the final layer from one copied banner.
    let mut cleaned = args.item_stack.copy_with_count(1);
    if let Some(patterns) = cleaned.get_data_component_mut::<BannerPatternsImpl>() {
        patterns.layers.pop();
    }
    create_filled_result(args.item_stack, args.player, cleaned, false);
    hand.publish(args);
    award_custom(args, CustomStatistic::CleanBanner);
    lower_fill_level(args, level);
    BlockActionResult::Success
}

fn dyed_item_interaction(args: &mut UseWithItemArgs<'_>, hand: &HandContext) -> BlockActionResult {
    if args
        .item_stack
        .get_data_component::<DyedColorImpl>()
        .is_none()
    {
        return BlockActionResult::PassToDefaultBlockAction;
    }
    let level = fill_level(args.world.get_block_state_id(args.position)) - 1;
    if !allow_change(args, level, CauldronChangeReason::Unknown) {
        return BlockActionResult::Consume;
    }
    args.item_stack
        .remove_data_component(DataComponent::DyedColor);
    hand.publish(args);
    award_custom(args, CustomStatistic::CleanArmor);
    lower_fill_level(args, level);
    BlockActionResult::Success
}
