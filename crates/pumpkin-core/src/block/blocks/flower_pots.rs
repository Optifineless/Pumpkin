use crate::block::registry::BlockActionResult;
use crate::block::{
    BlockBehaviour, NormalUseArgs, PathComputationType, RandomTickArgs, UseWithItemArgs,
};
use pumpkin_data::flower_pot_transformations::get_potted_item;
use pumpkin_data::{Block, BlockId, BlockState};
use pumpkin_macros::pumpkin_block_from_tag;
use pumpkin_world::world::BlockFlags;

#[pumpkin_block_from_tag("minecraft:flower_pots")]
pub struct FlowerPotBlock;

impl BlockBehaviour for FlowerPotBlock {
    // FlowerPotBlock.useItemOn consumes only valid plants in an empty pot.
    fn use_with_item(&self, args: UseWithItemArgs<'_>) -> BlockActionResult {
        let potted = get_potted_item(args.item_stack.item.id);
        if potted == BlockId::AIR {
            return BlockActionResult::PassToDefaultBlockAction;
        }
        if args.block != &Block::FLOWER_POT {
            return BlockActionResult::Consume;
        }
        args.world.set_block_state(
            args.position,
            Block::from_id(potted).default_state.id,
            BlockFlags::NOTIFY_ALL,
        );
        args.world
            .emit_game_event("block_change", args.position.to_centered_f64());
        args.player.increment_stat(
            pumpkin_data::statistic::StatisticCategory::Custom,
            pumpkin_data::statistic::CustomStatistic::PotFlower as i32,
            1,
        );
        args.item_stack
            .decrement_unless_creative(args.player.gamemode.load(), 1);
        BlockActionResult::Success
    }

    // FlowerPotBlock.useWithoutItem returns the plant, including with a full inventory.
    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        if args.block == &Block::FLOWER_POT {
            return BlockActionResult::Consume;
        }
        if let Some(item) = potted_content(args.block) {
            crate::item::item_utils::give_or_drop(
                args.player,
                pumpkin_data::item_stack::ItemStack::new(1, item),
            );
        }
        args.world.set_block_state(
            args.position,
            Block::FLOWER_POT.default_state.id,
            BlockFlags::NOTIFY_ALL,
        );
        args.world
            .emit_game_event("block_change", args.position.to_centered_f64());
        BlockActionResult::Success
    }

    fn random_tick(&self, args: RandomTickArgs<'_>) {
        let is_open_potted = args.block.eq(&Block::POTTED_OPEN_EYEBLOSSOM);
        let is_closed_potted = args.block.eq(&Block::POTTED_CLOSED_EYEBLOSSOM);
        if !is_open_potted && !is_closed_potted {
            return;
        }

        let is_open = is_open_potted;
        let should_be_open = args.world.eyeblossom_open(args.position).unwrap_or(is_open);

        if is_open != should_be_open {
            let next_block = if should_be_open {
                &Block::POTTED_OPEN_EYEBLOSSOM
            } else {
                &Block::POTTED_CLOSED_EYEBLOSSOM
            };
            args.world.set_block_state(
                args.position,
                next_block.default_state.id,
                BlockFlags::NOTIFY_ALL,
            );
        }
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}

// FlowerPotBlock.useWithoutItem constructs the contained block's item directly.
fn potted_content(block: &Block) -> Option<&'static pumpkin_data::item::Item> {
    let name = block.name.strip_prefix("potted_")?;
    let lookup = |name: &str| {
        #[cfg(test)]
        tests::ITEM_LOOKUPS.with(|count| count.set(count.get() + 1));
        pumpkin_data::item::Item::from_registry_key(name)
    };
    // Blocks.POTTED_AZALEA contains AZALEA; its block name ends in "_bush".
    lookup(name).or_else(|| name.strip_suffix("_bush").and_then(lookup))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    thread_local! { pub(super) static ITEM_LOOKUPS: Cell<usize> = const { Cell::new(0) }; }

    #[test]
    fn review_flower_pot_content_uses_one_lookup() {
        ITEM_LOOKUPS.set(0);
        assert_eq!(
            potted_content(&Block::POTTED_OPEN_EYEBLOSSOM),
            Some(&pumpkin_data::item::Item::OPEN_EYEBLOSSOM)
        );
        assert_eq!(ITEM_LOOKUPS.get(), 1, "item lookups per removal");

        for item in (0..=u16::MAX).filter_map(pumpkin_data::item::Item::from_id) {
            let pot = get_potted_item(item.id);
            if pot != BlockId::AIR {
                assert_eq!(potted_content(Block::from_id(pot)), Some(item));
            }
        }
    }
}
