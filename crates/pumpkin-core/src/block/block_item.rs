use super::registry::BlockRegistry;
use crate::{entity::player::Player, world::World};
use pumpkin_data::{Block, BlockDirection, BlockState, BlockStateId, item_stack::ItemStack};
use pumpkin_inventory::inventory::slot_transaction;
use pumpkin_util::{GameMode, math::position::BlockPos};
use pumpkin_world::world::BlockFlags;
use std::sync::Arc;

// BlockItem.place retains and consumes the same UseOnContext stack. Claim it before
// world effects because Pumpkin callbacks can move the physical source independently.
struct PlacementClaim {
    slot: usize,
    original: ItemStack,
    remaining: ItemStack,
    paid: bool,
}

impl PlacementClaim {
    fn claim(player: &Player, slot: usize, observed: &ItemStack) -> Option<Self> {
        if observed.is_empty() {
            return None;
        }
        let paid = player.gamemode.load() != GameMode::Creative;
        let mut remaining = observed.clone();
        if paid {
            remaining.decrement(1);
            if remaining.is_empty() {
                remaining = ItemStack::EMPTY.clone();
            }
        }
        slot_transaction::exchange(
            player.inventory().as_ref(),
            slot,
            observed,
            remaining.clone(),
        )
        .then(|| Self {
            slot,
            original: observed.clone(),
            remaining,
            paid,
        })
    }

    fn refund(self, player: &Player) {
        if !self.paid {
            return;
        }
        let inventory = player.inventory();
        if slot_transaction::exchange(
            inventory.as_ref(),
            self.slot,
            &self.remaining,
            self.original.clone(),
        ) {
            return;
        }
        // Refund only the paid item if another writer changed the remainder.
        let excess = slot_transaction::refund(
            inventory.as_ref(),
            self.slot,
            &self.original,
            self.original.copy_with_count(1),
        );
        crate::item::item_utils::give_or_drop(player, excess);
    }
}

impl BlockRegistry {
    pub(super) fn install_claimed_block(
        player: &Player,
        source_slot: usize,
        observed: &ItemStack,
        world: &Arc<World>,
        position: &BlockPos,
        new_state: BlockStateId,
        previous_state: BlockStateId,
    ) -> Option<ItemStack> {
        let claim = PlacementClaim::claim(player, source_slot, observed)?;
        // BlockItem.placeBlock returns failure when Level.setBlock cannot install the state.
        // Preserve target edits made by a placement callback instead of overwriting them.
        if world
            .set_block_state_if(position, new_state, BlockFlags::NOTIFY_ALL, |current| {
                current == previous_state && current != new_state
            })
            .is_none()
        {
            claim.refund(player);
            return None;
        }
        Some(claim.original)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Retains BlockItem.place's context through post-installation effects."
    )]
    pub(super) fn finish_block_placement(
        &self,
        player: &Arc<Player>,
        world: &Arc<World>,
        placed_block: &'static Block,
        position: &BlockPos,
        new_state: BlockStateId,
        face: BlockDirection,
        claimed_stack: &ItemStack,
    ) {
        world.play_bedrock_level_sound(
            "place",
            &position.to_centered_f64(),
            BlockState::to_be_network_id(new_state) as i32,
        );
        if placed_block.default_state.block_entity_type != u16::MAX
            && let Some(block_entity) = world.get_block_entity(position)
        {
            // BlockItem.updateBlockEntityComponents reads only the successfully claimed stack.
            block_entity.apply_item_components(claimed_stack);
            crate::block::entities::sign::apply_trusted_item_data(
                &block_entity,
                claimed_stack,
                player,
                world,
            );
        }
        self.player_placed(world, placed_block, new_state, position, face, player);
        player.trigger_advancement(
            crate::entity::player::advancement::trigger::AdvancementTrigger::PlacedBlock {
                block_id: format!("minecraft:{}", placed_block.name),
            },
        );
    }
}
