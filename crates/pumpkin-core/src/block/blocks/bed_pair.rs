use pumpkin_data::{
    Block, BlockStateId, HorizontalFacingExt,
    block_properties::{BedPart, WhiteBedLikeProperties},
};
use pumpkin_util::GameMode;
use pumpkin_world::world::BlockFlags;

use crate::block::{GetStateForNeighborUpdateArgs, PlayerWillDestroyArgs};

pub(super) fn update_shape(args: &GetStateForNeighborUpdateArgs<'_>) -> BlockStateId {
    // AbstractBedBlock.updateShape: only a change in the connected direction retires a half.
    let mut props = WhiteBedLikeProperties::from_state_id(args.state_id);
    let connected = if props.part == BedPart::Foot {
        props.facing
    } else {
        props.facing.opposite()
    };
    if args.direction != connected.to_block_direction() {
        return args.state_id;
    }
    if Block::from_state_id(args.neighbor_state_id) != args.block {
        return BlockStateId::AIR;
    }
    let partner = WhiteBedLikeProperties::from_state_id(args.neighbor_state_id);
    if partner.part == props.part {
        return BlockStateId::AIR;
    }
    props.occupied = partner.occupied;
    props.to_state_id(args.block)
}

pub(super) fn player_will_destroy(args: PlayerWillDestroyArgs<'_>) {
    // AbstractBedBlock.playerWillDestroy removes the loot-bearing head before a Creative foot.
    let props = WhiteBedLikeProperties::from_state_id(args.state.id);
    if args.player.gamemode.load() != GameMode::Creative || props.part != BedPart::Foot {
        return;
    }
    let head = args.position.offset(props.facing.to_offset());
    let expected = args.world.get_block_state_id(&head);
    if Block::from_state_id(expected) == args.block
        && WhiteBedLikeProperties::from_state_id(expected).part == BedPart::Head
    {
        // Keep the fork's partner BreakEvent. Shape propagation waits for the admitted foot.
        args.world.break_bed_block(
            &head,
            Some(args.player),
            Some(args.player),
            BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_DROPS | BlockFlags::UPDATE_KNOWN_SHAPE,
            crate::world::neighbor_updater::UPDATE_LIMIT,
            expected,
        );
    }
}

#[cfg(test)]
#[path = "bed_pair_tests.rs"]
mod tests;
