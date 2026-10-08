use super::{
    BlockInteraction, DefaultExplosionDamageCalculator, Explosion, ExplosionDamageCalculator, World,
};
use crate::{
    block::{ExplodeArgs, blocks::fire::FireBlockBase},
    world::loot::LootContextParameters,
};
use pumpkin_data::{Block, BlockState, BlockStateId, item_stack::ItemStack};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::world::BlockFlags;
use rand::{RngExt, seq::SliceRandom};
use rustc_hash::FxHashSet;
use std::sync::Arc;

impl Explosion {
    // ServerExplosion.calculateExplodedPositions includes air for fire placement and packet count.
    pub(super) fn calculate_exploded_positions(&self, world: &World) -> Vec<BlockPos> {
        let mut positions = FxHashSet::default();
        let default_calc = DefaultExplosionDamageCalculator;
        let calc = self.damage_calculator.as_deref().unwrap_or(&default_calc);
        for x in 0..16 {
            for y in 0..16 {
                for z in 0..16 {
                    if x != 0 && x != 15 && y != 0 && y != 15 && z != 0 && z != 15 {
                        continue;
                    }
                    let direction = Vector3::new(
                        f64::from(x as f32 / 15.0 * 2.0 - 1.0),
                        f64::from(y as f32 / 15.0 * 2.0 - 1.0),
                        f64::from(z as f32 / 15.0 * 2.0 - 1.0),
                    )
                    .normalize();
                    self.trace_block_ray(world, calc, direction, &mut positions);
                }
            }
        }
        positions.into_iter().collect()
    }

    fn trace_block_ray(
        &self,
        world: &World,
        calc: &dyn ExplosionDamageCalculator,
        direction: Vector3<f64>,
        positions: &mut FxHashSet<BlockPos>,
    ) {
        let mut power = self.power * (0.7 + rand::random::<f32>() * 0.6);
        let mut cursor = self.pos;
        while power > 0.0 {
            let pos = BlockPos::floored(cursor.x, cursor.y, cursor.z);
            if !world.is_in_build_limit(pos) {
                break;
            }
            // Clamp ServerExplosion.calculateExplodedPositions to loaded terrain; missing chunks stop rays.
            if !world.level.is_chunk_loaded(&pos.chunk_position()) {
                break;
            }
            let state = world.get_block_state(&pos);
            let block = state.id.to_block();
            let (_, fluid) = world.get_fluid_and_fluid_state(&pos);
            let protects_rail = self.protects_rail(world, &pos, block);
            let resistance = if protects_rail {
                Some(0.0)
            } else {
                calc.get_block_explosion_resistance(self, world, &pos, block, &fluid)
            };
            if let Some(resistance) = resistance {
                power -= (resistance + 0.3) * 0.3;
            }
            if power > 0.0
                && !protects_rail
                && calc.should_block_explode(self, world, &pos, block, power)
            {
                positions.insert(pos);
            }
            cursor += direction * f64::from(0.3f32);
            power -= 0.225_000_01;
        }
    }

    pub(super) fn interact_with_blocks(&self, world: &Arc<World>, positions: &[BlockPos]) {
        if self.block_interaction == BlockInteraction::Keep {
            return;
        }
        if self.block_interaction != BlockInteraction::TriggerBlock {
            let mut event =
                crate::plugin::api::events::block::block_explode::BlockExplodeEvent::new(
                    BlockPos::floored(self.pos.x, self.pos.y, self.pos.z),
                    if self.power > 0.0 {
                        1.0 / self.power
                    } else {
                        1.0
                    },
                );
            if let Some(server) = world.server.upgrade() {
                server.plugin_manager.fire_blocking(&server, &mut event);
            }
            if event.cancelled {
                return;
            }
        }
        let mut positions = positions.to_vec();
        positions.shuffle(&mut rand::rng());
        let mut drops = Vec::new();
        for pos in positions {
            // ServerExplosion.interactWithBlocks rereads after every neighbor update/chain reaction.
            let state = world.get_block_state(&pos);
            let block = state.id.to_block();
            if state.is_air() {
                continue;
            }
            let behavior = world.block_registry.get_pumpkin_block(block.id);
            let args = ExplodeArgs {
                world,
                block,
                position: &pos,
                explosion: Some(self),
            };
            if self.block_interaction == BlockInteraction::TriggerBlock {
                self.trigger_block(world, &pos, block, state);
                continue;
            }
            if behavior.is_none_or(|behavior| behavior.should_drop_items_on_explosion()) {
                // BlockBehaviour.onExplosionHit / spawnAfterBreak: only player-caused blasts drop XP.
                if world.level_info.load().game_rules.block_drops
                    && self
                        .cause
                        .as_ref()
                        .is_some_and(|cause| cause.get_player().is_some())
                {
                    crate::block::drop_experience(world, block, &pos, None);
                }
                for stack in self.block_drops(world, &pos, block, state) {
                    add_or_append_stack(&mut drops, stack, pos);
                }
            }
            // Block.onExplosionHit: loot before removal, then wasExploded (TNT uses the cause).
            world.set_block_state(&pos, BlockStateId::AIR, BlockFlags::NOTIFY_ALL);
            world.close_container_screens_at(&pos);
            if let Some(behavior) = behavior {
                behavior.explode(args);
            }
        }
        // Block.popResource obeys block_drops after the loot and block callbacks have run.
        if world.level_info.load().game_rules.block_drops {
            for (pos, stack) in drops {
                world.drop_stack(&pos, stack);
            }
        }
    }

    fn block_drops(
        &self,
        world: &Arc<World>,
        pos: &BlockPos,
        block: &Block,
        state: &'static BlockState,
    ) -> Vec<ItemStack> {
        let params = LootContextParameters {
            block_state: Some(state),
            tool: Some(ItemStack::EMPTY.clone()),
            this_entity: self
                .source
                .as_ref()
                .map(|source| source.get_entity().entity_type),
            explosion_radius: (self.block_interaction == BlockInteraction::DestroyWithDecay)
                .then_some(self.power),
            position: Some(pos.to_centered_f64()),
            world_time: world.level_info.load().day_time as u64,
            is_raining: Some(world.is_raining()),
            is_thundering: Some(world.is_thundering()),
            ..Default::default()
        };
        // BlockBehaviour.onExplosionHit uses the same Block.getDrops context as harvesting.
        crate::block::block_drops(world, block, pos, &params)
    }

    pub(super) fn create_fire(&self, world: &Arc<World>, positions: &[BlockPos]) {
        if !self.fire {
            return;
        }
        // ServerExplosion.createFire samples every affected position, including original air.
        for pos in positions {
            if rand::rng().random_range(0..3) == 0
                && world.get_block_state(pos).is_air()
                && world.get_block_state(&pos.down()).is_solid_render()
            {
                let block = FireBlockBase::get_fire_type(world, pos);
                let state = if block.id == Block::FIRE.id {
                    crate::block::blocks::fire::fire::FireBlock
                        .get_state_for_position(world, &block, pos)
                } else {
                    block.default_state.id
                };
                world.set_block_state(pos, state, BlockFlags::NOTIFY_ALL);
            }
        }
    }
}

// ServerExplosion.StackCollector.tryMerge / ItemEntity.merge, capped at 16 per collected stack.
fn add_or_append_stack(
    stacks: &mut Vec<(BlockPos, ItemStack)>,
    mut input: ItemStack,
    pos: BlockPos,
) {
    const MAX_DROPS_PER_COMBINED_STACK: u8 = 16;
    for (_, stack) in stacks.iter_mut() {
        if crate::entity::item::ItemEntity::are_mergeable(stack, &input) {
            // ItemEntity.merge can split an oversized loot stack back down to the collection cap.
            let room = i16::from(MAX_DROPS_PER_COMBINED_STACK.min(stack.get_max_stack_size()))
                - i16::from(stack.item_count);
            let count = room.min(i16::from(input.item_count));
            stack.item_count = (i16::from(stack.item_count) + count) as u8;
            input.item_count = (i16::from(input.item_count) - count) as u8;
            if input.is_empty() {
                return;
            }
        }
    }
    stacks.push((pos, input));
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumpkin_data::item::Item;

    #[test]
    fn explosion_drop_collection_respects_item_limits_and_the_sixteen_item_cap() {
        let first = BlockPos::new(0, 0, 0);
        let second = BlockPos::new(1, 0, 0);
        let mut drops = Vec::new();
        add_or_append_stack(&mut drops, ItemStack::new(12, &Item::COBBLESTONE), first);
        add_or_append_stack(&mut drops, ItemStack::new(10, &Item::COBBLESTONE), second);
        assert_eq!(drops.len(), 2);
        assert_eq!((drops[0].0, drops[0].1.item_count), (first, 16));
        assert_eq!((drops[1].0, drops[1].1.item_count), (second, 6));
        drops.clear();
        add_or_append_stack(&mut drops, ItemStack::new(1, &Item::DIAMOND_SWORD), first);
        add_or_append_stack(&mut drops, ItemStack::new(1, &Item::DIAMOND_SWORD), second);
        assert_eq!(drops.len(), 2);
        assert!(drops.iter().all(|(_, stack)| stack.item_count == 1));
    }
}

#[cfg(test)]
#[path = "block_review_tests.rs"]
mod block_review_tests;
