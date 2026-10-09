use super::{Explosion, World};
use crate::block::blocks::{
    doors::DoorBlock,
    fence_gates,
    redstone::{bell, buttons, lever},
    trapdoor,
};
use pumpkin_data::{
    Block, BlockState,
    block_properties::{
        DoubleBlockHalf, OakDoorLikeProperties, OakFenceGateLikeProperties,
        OakTrapdoorLikeProperties,
    },
    entity::EntityType,
    game_event::GameEvent,
    sound::SoundCategory,
    tag::{self, Taggable},
};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;
use std::sync::Arc;

impl Explosion {
    // ServerExplosion.canTriggerBlocks and each block's onExplosionHit override.
    pub(super) fn trigger_block(
        &self,
        world: &Arc<World>,
        pos: &BlockPos,
        block: &Block,
        state: &BlockState,
    ) {
        if self.source.as_ref().is_some_and(|source| {
            source.get_entity().entity_type == &EntityType::BREEZE_WIND_CHARGE
        }) && !world.level_info.load().game_rules.mob_griefing
        {
            return;
        }
        if block == &Block::LEVER {
            lever::toggle_lever(world, pos);
        } else if block.has_tag(&tag::Block::MINECRAFT_BUTTONS) {
            if buttons::click_button(None, world, pos) {
                world.emit_game_event(GameEvent::BlockActivate.name(), pos.to_centered_f64());
            }
        } else if block == &Block::BELL {
            bell::ring_bell(*pos, world, None, None);
        } else if block.has_tag(&tag::Block::MINECRAFT_DOORS) {
            let props = OakDoorLikeProperties::from_state_id(state.id);
            // BlockSetType.IRON alone cannot open by wind charge (including copper/gold).
            if block != &Block::IRON_DOOR && !props.powered && props.half == DoubleBlockHalf::Lower
            {
                DoorBlock::set_open(world, pos, !props.open);
            }
        } else if block.has_tag(&tag::Block::MINECRAFT_TRAPDOORS) {
            let mut props = OakTrapdoorLikeProperties::from_state_id(state.id);
            if block != &Block::IRON_TRAPDOOR && !props.powered {
                props.open = !props.open;
                world.set_block_state(pos, props.to_state_id(block), BlockFlags::NOTIFY_LISTENERS);
                world.play_sound_fine(
                    trapdoor::get_sound(block, props.open),
                    SoundCategory::Blocks,
                    &pos.to_centered_f64(),
                    1.0,
                    rand::random::<f32>() * 0.1 + 0.9,
                );
                emit_open(world, pos, props.open);
                if props.waterlogged {
                    world.schedule_fluid_tick(
                        &pumpkin_data::fluid::Fluid::WATER,
                        *pos,
                        crate::block::fluid::water::WATER_FLOW_SPEED,
                        pumpkin_world::tick::TickPriority::Normal,
                    );
                }
            }
        } else if block.has_tag(&tag::Block::MINECRAFT_FENCE_GATES) {
            let mut props = OakFenceGateLikeProperties::from_state_id(state.id);
            if !props.powered {
                props.open = !props.open;
                world.set_block_state(pos, props.to_state_id(block), BlockFlags::NOTIFY_ALL);
                world.play_sound_fine(
                    fence_gates::get_sound(block, props.open),
                    SoundCategory::Blocks,
                    &pos.to_centered_f64(),
                    1.0,
                    rand::random::<f32>() * 0.1 + 0.9,
                );
                emit_open(world, pos, props.open);
            }
        } else if block.has_tag(&tag::Block::MINECRAFT_CANDLES)
            || block.has_tag(&tag::Block::MINECRAFT_CANDLE_CAKES)
        {
            crate::entity::projectile::potion_water::douse_fire(world, pos);
        }
        // BlockBehaviour.onExplosionHit does nothing else for TRIGGER_BLOCK, including TNT.
    }
}

fn emit_open(world: &World, pos: &BlockPos, open: bool) {
    world.emit_game_event(
        if open {
            GameEvent::BlockOpen
        } else {
            GameEvent::BlockClose
        }
        .name(),
        pos.to_centered_f64(),
    );
}

#[cfg(test)]
mod tests;
