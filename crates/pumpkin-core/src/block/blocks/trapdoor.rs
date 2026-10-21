use crate::block::blocks::redstone::block_receives_redstone_power;
use crate::block::registry::BlockActionResult;
use crate::block::{
    BlockBehaviour, NormalUseArgs, OnNeighborUpdateArgs, OnPlaceArgs, PathComputationType,
};
use crate::entity::EntityBase;
use crate::entity::player::Player;
use crate::world::World;
use pumpkin_data::BlockDirection;
use pumpkin_data::block_properties::Half;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::tag::Taggable;
use pumpkin_data::{Block, BlockState, BlockStateId, tag};
use pumpkin_macros::pumpkin_block_from_tag;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;
use std::sync::Arc;

type TrapDoorProperties = pumpkin_data::block_properties::OakTrapdoorLikeProperties;

fn toggle_trapdoor(player: &Player, world: &Arc<World>, block_pos: &BlockPos) {
    let (block, block_state) = world.get_block_and_state_id(block_pos);
    if !block.has_tag(&tag::Block::MINECRAFT_TRAPDOORS) {
        return;
    }
    let mut trapdoor_props = TrapDoorProperties::from_state_id(block_state);
    trapdoor_props.open = !trapdoor_props.open;

    if !world.set_block_state_from_observed(
        block_pos,
        block_state,
        trapdoor_props.to_state_id(block),
        BlockFlags::NOTIFY_LISTENERS,
    ) {
        return;
    }
    world.play_block_sound_expect(
        player,
        get_sound(block, trapdoor_props.open),
        SoundCategory::Blocks,
        *block_pos,
    );
}

fn can_open_trapdoor(block: &Block) -> bool {
    if block == &Block::IRON_TRAPDOOR {
        return false;
    }
    true
}

pub(crate) fn get_sound(block: &Block, open: bool) -> Sound {
    if open {
        if block.has_tag(&tag::Block::MINECRAFT_WOODEN_TRAPDOORS) {
            Sound::BlockWoodenTrapdoorOpen
        } else if block == &Block::IRON_TRAPDOOR {
            Sound::BlockIronTrapdoorOpen
        } else {
            Sound::BlockCopperTrapdoorOpen
        }
    } else if block.has_tag(&tag::Block::MINECRAFT_WOODEN_TRAPDOORS) {
        Sound::BlockWoodenTrapdoorClose
    } else if block == &Block::IRON_TRAPDOOR {
        Sound::BlockIronTrapdoorClose
    } else {
        Sound::BlockCopperTrapdoorClose
    }
}

#[pumpkin_block_from_tag("minecraft:trapdoors")]
pub struct TrapDoorBlock;

impl BlockBehaviour for TrapDoorBlock {
    fn normal_use(&self, args: NormalUseArgs<'_>) -> BlockActionResult {
        {
            if !can_open_trapdoor(args.block) {
                return BlockActionResult::Pass;
            }

            toggle_trapdoor(args.player, args.world, args.position);

            BlockActionResult::Success
        }
    }

    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let mut trapdoor_props = TrapDoorProperties::default(args.block);
        trapdoor_props.waterlogged = args.replacing.water_source();

        let powered = block_receives_redstone_power(args.world, args.position);

        let player_facing = args.player.get_entity().get_horizontal_facing();
        // TrapDoorBlock.getStateForPlacement uses the clicked face only for adjacent horizontal placement.
        let facing = if args.position == &args.use_item_on.position {
            player_facing.opposite()
        } else {
            args.direction
                .opposite()
                .to_horizontal_facing()
                .unwrap_or_else(|| player_facing.opposite())
        };

        trapdoor_props.facing = facing;

        trapdoor_props.half = match args.direction {
            BlockDirection::Up => Half::Top,
            BlockDirection::Down => Half::Bottom,
            _ => match args.use_item_on.cursor_pos.y {
                0.0..0.5 => Half::Bottom,
                _ => Half::Top,
            },
        };

        trapdoor_props.powered = powered;
        trapdoor_props.open = powered;

        trapdoor_props.to_state_id(args.block)
    }

    fn on_neighbor_update(&self, args: OnNeighborUpdateArgs<'_>) {
        let observed = args.world.get_block_state_id(args.position);
        if observed.to_block() != args.block {
            return;
        }
        let mut props = TrapDoorProperties::from_state_id(observed);
        let powered = block_receives_redstone_power(args.world, args.position);
        let changed_open = powered != props.powered && powered != props.open;
        if powered != props.powered {
            props.powered = powered;
            if changed_open {
                props.open = powered;
            }
        }
        if !args.world.set_block_state_from_observed(
            args.position,
            observed,
            props.to_state_id(args.block),
            BlockFlags::NOTIFY_LISTENERS,
        ) {
            return;
        }
        if changed_open {
            args.world.play_block_sound(
                get_sound(args.block, powered),
                SoundCategory::Blocks,
                *args.position,
            );
        }
    }

    fn is_pathfindable(&self, state: &BlockState, computation_type: PathComputationType) -> bool {
        let props = TrapDoorProperties::from_state_id(state.id);
        match computation_type {
            PathComputationType::Land | PathComputationType::Air => props.open,
            PathComputationType::Water => props.waterlogged,
        }
    }
}
