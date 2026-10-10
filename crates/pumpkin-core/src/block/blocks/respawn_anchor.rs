use pumpkin_data::block_properties::RespawnAnchorLikeProperties;
use pumpkin_data::data_component_impl::EquipmentSlot;
use pumpkin_data::game_event::GameEvent;
use pumpkin_data::item::Item;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::{BlockState, translation};
use pumpkin_macros::pumpkin_block;
use pumpkin_world::world::BlockFlags;

use crate::block::registry::BlockActionResult;
use crate::block::{
    BlockBehaviour, GetComparatorOutputArgs, NormalUseArgs, PathComputationType, UseWithItemArgs,
};

/// Vanilla `RespawnAnchorBlock.MAX_CHARGES`.
const MAX_CHARGES: u8 = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ItemUseAction {
    Charge,
    PassToOffHand,
    UseWithoutItem,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EmptyHandAction {
    Pass,
    Explode,
    SetSpawn,
}

// RespawnAnchorBlock.useItemOn yields the main-hand pass only when off-hand glowstone can charge.
const fn item_use_action(
    is_glowstone: bool,
    charges: u8,
    is_main_hand: bool,
    off_hand_is_glowstone: bool,
) -> ItemUseAction {
    if is_glowstone && charges < MAX_CHARGES {
        ItemUseAction::Charge
    } else if is_main_hand && off_hand_is_glowstone && charges < MAX_CHARGES {
        ItemUseAction::PassToOffHand
    } else {
        ItemUseAction::UseWithoutItem
    }
}

// RespawnAnchorBlock.useWithoutItem passes before checking the dimension when the anchor is uncharged.
const fn empty_hand_action(charges: u8, respawn_anchor_works: bool) -> EmptyHandAction {
    if charges == 0 {
        EmptyHandAction::Pass
    } else if respawn_anchor_works {
        EmptyHandAction::SetSpawn
    } else {
        EmptyHandAction::Explode
    }
}

#[pumpkin_block("minecraft:respawn_anchor")]
pub struct RespawnAnchorBlock;

impl BlockBehaviour for RespawnAnchorBlock {
    fn use_with_item(&self, args: UseWithItemArgs<'_>) -> BlockActionResult {
        let state_id = args.world.get_block_state_id(args.position);
        let mut props = RespawnAnchorLikeProperties::from_state_id(state_id);

        match item_use_action(
            args.item_stack.item.id == Item::GLOWSTONE.id,
            props.charges,
            matches!(args.equipment_slot, EquipmentSlot::MainHand(_)),
            args.player.inventory.off_hand_item().item.id == Item::GLOWSTONE.id,
        ) {
            ItemUseAction::PassToOffHand => return BlockActionResult::Pass,
            ItemUseAction::UseWithoutItem => return BlockActionResult::PassToDefaultBlockAction,
            ItemUseAction::Charge => {}
        }

        props.charges += 1;
        // RespawnAnchorBlock.charge emits block_change and sound before consuming the glowstone.
        args.world.set_block_state(
            args.position,
            props.to_state_id(args.block),
            BlockFlags::NOTIFY_ALL,
        );
        args.world.emit_game_event(
            GameEvent::BlockChange.name(),
            args.position.to_centered_f64(),
        );

        args.world.play_sound(
            Sound::BlockRespawnAnchorCharge,
            SoundCategory::Blocks,
            &args.position.to_centered_f64(),
        );
        args.item_stack
            .decrement_unless_creative(args.player.gamemode.load(), 1);

        BlockActionResult::Success
    }

    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        let state_id = args.world.get_block_state_id(args.position);
        let props = RespawnAnchorLikeProperties::from_state_id(state_id);

        match empty_hand_action(props.charges, args.world.dimension.respawn_anchor_works) {
            EmptyHandAction::Pass => return BlockActionResult::Pass,
            EmptyHandAction::Explode => {
                // RespawnAnchorBlock.explode removes the block with Level.removeBlock's notifying flags.
                if args
                    .world
                    .break_block(
                        args.position,
                        None,
                        BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_DROPS,
                    )
                    .is_none()
                {
                    return BlockActionResult::SuccessServer;
                }
                args.world.explode_respawn_anchor(*args.position);
                return BlockActionResult::SuccessServer;
            }
            EmptyHandAction::SetSpawn => {}
        }

        let player = args.player;
        let world = args.world;
        let pos = *args.position;
        // RespawnAnchorBlock.useWithoutItem returns CONSUME when the spawn point is unchanged.
        if !player.set_respawn_point(world.dimension.clone(), pos, 0.0, 0.0, false) {
            return BlockActionResult::Consume;
        }
        world.play_sound(
            Sound::BlockRespawnAnchorSetSpawn,
            SoundCategory::Blocks,
            &pos.to_centered_f64(),
        );
        player.send_system_message(&pumpkin_macros::translate_cross!(
            translation::java::BLOCK_MINECRAFT_SET_SPAWN,
            translation::bedrock::TILE_BED_RESPAWNSET
        ));

        BlockActionResult::SuccessServer
    }

    /// Charges scale over the full signal range, so each charge is worth 15 / 4.
    fn get_comparator_output(&self, args: GetComparatorOutputArgs<'_>) -> Option<u8> {
        let props = RespawnAnchorLikeProperties::from_state_id(args.state.id);
        Some(props.charges * 15 / MAX_CHARGES)
    }

    fn is_pathfindable(&self, _state: &BlockState, _computation_type: PathComputationType) -> bool {
        false
    }
}

#[cfg(test)]
#[path = "respawn_anchor_tests.rs"]
mod tests;
