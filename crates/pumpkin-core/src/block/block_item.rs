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
        updates: &[(BlockPos, BlockStateId, BlockStateId, BlockFlags)],
    ) -> Option<ItemStack> {
        let claim = PlacementClaim::claim(player, source_slot, observed)?;
        // BlockItem.placeBlock returns failure when Level.setBlock cannot install the state.
        // Preserve target edits made by a placement callback instead of overwriting them.
        if updates.first().is_none_or(|(_, old, new, _)| old == new)
            || !world.set_block_states_from_observed(updates)
        {
            claim.refund(player);
            return None;
        }
        Some(claim.original)
    }

    fn chest_placement_updates(
        world: &World,
        block: &Block,
        position: BlockPos,
        old: BlockStateId,
        new: BlockStateId,
    ) -> Option<Vec<(BlockPos, BlockStateId, BlockStateId, BlockFlags)>> {
        use pumpkin_data::block_properties::{ChestLikeProperties, ChestType};
        let mut updates = vec![(position, old, new, BlockFlags::NOTIFY_ALL)];
        let chest = ChestLikeProperties::from_state_id(new);
        let (direction, partner_type) = match chest.r#type {
            ChestType::Single => return Some(updates),
            ChestType::Left => (chest.facing.rotate_clockwise(), ChestType::Right),
            ChestType::Right => (chest.facing.rotate_counter_clockwise(), ChestType::Left),
        };
        let partner = position.offset(direction.to_offset());
        let observed = world.get_block_state_id(&partner);
        if observed.to_block() != block {
            return None;
        }
        let mut partner_props = ChestLikeProperties::from_state_id(observed);
        if partner_props.facing != chest.facing || partner_props.r#type != ChestType::Single {
            return None;
        }
        partner_props.r#type = partner_type;
        updates.push((
            partner,
            observed,
            partner_props.to_state_id(block),
            BlockFlags::NOTIFY_LISTENERS,
        ));
        Some(updates)
    }

    pub(super) fn placement_updates(
        world: &World,
        block: &Block,
        position: BlockPos,
        old: BlockStateId,
        new: BlockStateId,
    ) -> Option<Vec<(BlockPos, BlockStateId, BlockStateId, BlockFlags)>> {
        use crate::block::{BlockMetadata, blocks::plant::tall_plant::TallPlantBlock};
        use pumpkin_data::{
            block_properties::{BlockProperties, ChestLikeProperties, WhiteBedLikeProperties},
            tag::Taggable,
        };
        if ChestLikeProperties::handles_block_id(block.id) {
            return Self::chest_placement_updates(world, block, position, old, new);
        }
        let mut updates = vec![(position, old, new, BlockFlags::NOTIFY_ALL)];
        let Some(properties) = block.properties(new) else {
            return Some(updates);
        };
        let mut properties = properties.to_props();
        let companion = if (block.has_tag(&pumpkin_data::tag::Block::MINECRAFT_BEDS)
            || block == &Block::STRAW_BED)
            && properties
                .iter()
                .any(|(key, value)| *key == "part" && *value == "foot")
        {
            let facing = WhiteBedLikeProperties::from_state_id(new).facing;
            properties.iter_mut().find(|(key, _)| *key == "part")?.1 = "head";
            position.offset(facing.to_offset())
        } else if (block.has_tag(&pumpkin_data::tag::Block::MINECRAFT_DOORS)
            || TallPlantBlock::ids().contains(&block.id)
            || block == &Block::SMALL_DRIPLEAF)
            && properties
                .iter()
                .any(|(key, value)| *key == "half" && *value == "lower")
        {
            properties.iter_mut().find(|(key, _)| *key == "half")?.1 = "upper";
            position.up()
        } else {
            return Some(updates);
        };
        let observed = world.get_block_state_id(&companion);
        if !world.is_in_build_limit(companion) || !observed.to_state().replaceable() {
            return None;
        }
        if block == &Block::SMALL_DRIPLEAF
            && let Some((_, waterlogged)) =
                properties.iter_mut().find(|(key, _)| *key == "waterlogged")
        {
            *waterlogged = if observed.to_block() == &Block::WATER {
                "true"
            } else {
                "false"
            };
        }
        updates.push((
            companion,
            observed,
            block.from_properties(&properties).to_state_id(block),
            BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK,
        ));
        Some(updates)
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
                new_state,
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
