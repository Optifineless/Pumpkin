//! Filled-bucket admission and payment for CauldronInteractions.addDefaultInteractions.
use std::sync::{Arc, atomic::Ordering};

use super::{UseWithItemArgs, fire_cauldron_change};
use crate::{
    block::registry::BlockActionResult,
    entity::{EntityBase, player::Player},
};
use pumpkin_data::{
    Block, BlockId, BlockStateId,
    block_properties::WaterCauldronLikeProperties,
    data_component_impl::EquipmentSlot,
    item::Item,
    item_stack::ItemStack,
    sound::{Sound, SoundCategory},
    statistic::{CustomStatistic, StatisticCategory},
    tag::Taggable,
};
use pumpkin_inventory::{Inventory, inventory::slot_transaction, screen_handler::InventoryPlayer};
use pumpkin_util::Hand;

// Claim, admit the raw world write and insert the result under one inventory owner.
// No owner survives world callbacks, plugins, drops or packet sends.
struct BucketClaim {
    slot: usize,
    original: ItemStack,
    remaining: ItemStack,
    paid: bool,
}

impl BucketClaim {
    fn claim(player: &Player, slot: usize, original: &ItemStack) -> Option<Self> {
        if original.is_empty() {
            return None;
        }
        let paid = !player.has_infinite_materials();
        let mut remaining = original.clone();
        if paid {
            remaining.decrement(1);
            if remaining.is_empty() {
                remaining = ItemStack::EMPTY.clone();
            }
        }
        slot_transaction::exchange(
            player.inventory().as_ref(),
            slot,
            original,
            remaining.clone(),
        )
        .then(|| Self {
            slot,
            original: original.clone(),
            remaining,
            paid,
        })
    }

    fn refund(self, player: &Player) -> ItemStack {
        if !self.paid {
            return ItemStack::EMPTY.clone();
        }
        let inventory = player.inventory();
        if slot_transaction::exchange(
            inventory.as_ref(),
            self.slot,
            &self.remaining,
            self.original.clone(),
        ) {
            return ItemStack::EMPTY.clone();
        }
        slot_transaction::refund(
            inventory.as_ref(),
            self.slot,
            &self.original,
            self.original.copy_with_count(1),
        )
    }

    // ItemUtils.createFilledResult, retaining the admitted physical slot across callbacks.
    fn finish(self, player: &Player, input: &mut ItemStack) -> BucketResult {
        let inventory = player.inventory();
        let mut output = ItemStack::new(1, &Item::BUCKET);
        if !self.paid
            && (0..inventory.size()).any(|slot| {
                inventory
                    .get_stack(slot)
                    .are_items_and_components_equal(&output)
            })
        {
            output = ItemStack::EMPTY.clone();
        }
        if self.paid
            && self.remaining.is_empty()
            && slot_transaction::exchange(
                inventory.as_ref(),
                self.slot,
                &self.remaining,
                output.clone(),
            )
        {
            output = ItemStack::EMPTY.clone();
        }
        inventory.insert_stack_anywhere(&mut output);
        *input = inventory.get_stack(self.slot);
        BucketResult {
            slot: self.slot,
            paid: self.paid,
            overflow: output,
        }
    }
}

struct BucketResult {
    slot: usize,
    paid: bool,
    overflow: ItemStack,
}

impl BucketResult {
    fn publish(self, player: &Player) {
        player.inventory_changed.store(true, Ordering::Relaxed);
        player.sync_hand_slot(self.slot, player.inventory().get_stack(self.slot));
        if self.paid && !self.overflow.is_empty() {
            crate::item::item_utils::give_or_drop(player, self.overflow);
        }
    }
}

fn is_under_water(args: &UseWithItemArgs<'_>) -> bool {
    args.world
        .get_fluid(&args.position.up())
        .has_tag(&pumpkin_data::tag::Fluid::MINECRAFT_WATER)
}

// CauldronInteractions.fillWaterInteraction / fillLavaInteraction / fillPowderSnowInteraction.
pub(super) fn try_empty_bucket(args: &mut UseWithItemArgs<'_>) -> Option<BlockActionResult> {
    let (new_state, sound, rejects_water) = match args.item_stack.item.id {
        id if id == Item::WATER_BUCKET.id => (
            Block::WATER_CAULDRON
                .from_properties(&[("level", "3")])
                .to_state_id(&Block::WATER_CAULDRON),
            Sound::ItemBucketEmpty,
            false,
        ),
        id if id == Item::LAVA_BUCKET.id => (
            Block::LAVA_CAULDRON.default_state.id,
            Sound::ItemBucketEmptyLava,
            true,
        ),
        id if id == Item::POWDER_SNOW_BUCKET.id => (
            Block::POWDER_SNOW_CAULDRON
                .from_properties(&[("level", "3")])
                .to_state_id(&Block::POWDER_SNOW_CAULDRON),
            Sound::ItemBucketEmptyPowderSnow,
            true,
        ),
        _ => return None,
    };
    if rejects_water && is_under_water(args) {
        return Some(BlockActionResult::Consume);
    }
    Some(empty_bucket(args, new_state, sound, rejects_water))
}

fn empty_bucket(
    args: &mut UseWithItemArgs<'_>,
    new_state: BlockStateId,
    sound: Sound,
    rejects_water: bool,
) -> BlockActionResult {
    let old_state = args.world.get_block_state_id(args.position);
    if Block::from_state_id(old_state).id != args.block.id {
        return BlockActionResult::Consume;
    }
    let old_level = match args.block.id {
        BlockId::CAULDRON => 0,
        BlockId::LAVA_CAULDRON => 3,
        _ => i32::from(WaterCauldronLikeProperties::from_state_id(old_state).level),
    };
    let hand = if args.equipment_slot == &EquipmentSlot::MAIN_HAND {
        Hand::Right
    } else {
        Hand::Left
    };
    let source = crate::net::java::play::hand_use_result::hand_slot(args.player, hand);
    let original = args.item_stack.clone();
    let lifecycle = args.player.living_entity.damage_lifecycle();
    if !fire_cauldron_change(
        args.world,
        *args.position,
        old_level,
        3,
        crate::plugin::block::cauldron_level_change::CauldronChangeReason::BucketEmpty,
        Some(args.player.clone() as Arc<dyn EntityBase>),
    ) {
        return BlockActionResult::Consume;
    }
    let Some(mutation) = args.world.block_entity_mutation_permit(*args.position) else {
        return BlockActionResult::Consume;
    };
    let inventory = args.player.inventory();
    let owner = inventory.slot_transaction(source);
    let guard = owner.lock();
    if args.player.living_entity.damage_lifecycle() != lifecycle
        || args.player.client.closed()
        || !args.player.living_entity.is_alive()
        || !Arc::ptr_eq(&args.player.world(), args.world)
        || !args
            .world
            .players
            .load()
            .iter()
            .any(|player| Arc::ptr_eq(player, args.player))
        || (rejects_water && is_under_water(args))
        || args.world.get_block_state_id(args.position) != old_state
    {
        return BlockActionResult::Consume;
    }
    let Some(claim) = BucketClaim::claim(args.player, source, &original) else {
        return BlockActionResult::Consume;
    };
    let Some(effects) = args
        .world
        .prepare_cauldron_fill(*args.position, old_state, new_state)
    else {
        let excess = claim.refund(args.player);
        drop(guard);
        drop(mutation);
        if !excess.is_empty() {
            crate::item::item_utils::give_or_drop(args.player, excess);
        }
        return BlockActionResult::Consume;
    };
    let result = claim.finish(args.player, args.item_stack);
    drop(guard);
    drop(mutation);
    result.publish(args.player);
    args.player.increment_stat(
        StatisticCategory::Custom,
        CustomStatistic::FillCauldron as i32,
        1,
    );
    args.player
        .increment_stat(StatisticCategory::Used, i32::from(original.item.id), 1);
    effects();
    args.world.play_sound(
        sound,
        SoundCategory::Blocks,
        &args.position.to_centered_f64(),
    );
    args.world.emit_game_event(
        pumpkin_data::game_event::GameEvent::FluidPlace.name(),
        args.position.to_centered_f64(),
    );
    BlockActionResult::Success
}
