use pumpkin_macros::pumpkin_block;
use pumpkin_util::math::vector3::Vector3;
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use crate::block::{BlockBehaviour, OnNeighborUpdateArgs, PlacedArgs};
use crate::world::World;
use pumpkin_data::particle::Particle;
use pumpkin_data::sound::{Sound, SoundCategory};
use pumpkin_data::{Block, BlockState, BlockStateId, fluid::Fluid};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;

const MAX_DEPTH: u8 = 6;
const MAX_VISITED: usize = 65;
const DIRECTIONS: [(i32, i32, i32); 6] = [
    (0, -1, 0),
    (0, 1, 0),
    (0, 0, -1),
    (0, 0, 1),
    (-1, 0, 0),
    (1, 0, 0),
];

#[derive(Clone, Copy)]
enum Absorption {
    SetState(BlockStateId),
    Remove,
    DropResources,
}

#[derive(Clone, Copy)]
struct WaterNode {
    position: BlockPos,
    state_id: BlockStateId,
    absorption: Absorption,
}

#[pumpkin_block("minecraft:sponge")]
pub struct SpongeBlock;

impl SpongeBlock {
    pub fn absorb_water(world: &Arc<crate::world::World>, position: &BlockPos) -> bool {
        let sponge_state_id = world.get_block_state_id(position);
        if Block::from_state_id(sponge_state_id) != &Block::SPONGE
            || Self::find_water(world, *position).is_empty()
        {
            return false;
        }

        // SpongeBlock.removeWaterBreadthFirstSearch fires its Pumpkin event before effects.
        let mut event =
            crate::plugin::api::events::block::sponge_absorb::SpongeAbsorbEvent::new(*position);
        if let Some(server) = world.server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
        if event.cancelled || world.get_block_state_id(position) != sponge_state_id {
            return false;
        }

        // The event may have changed a candidate or disconnected part of the water graph.
        let water_nodes = Self::find_water(world, *position);
        if water_nodes.is_empty() {
            return false;
        }
        Self::absorb_nodes(world, position, sponge_state_id, water_nodes)
    }

    fn absorb_nodes(
        world: &Arc<World>,
        sponge_pos: &BlockPos,
        sponge_state_id: BlockStateId,
        water_nodes: Vec<WaterNode>,
    ) -> bool {
        let mut changed = Vec::new();
        for discovered_node in water_nodes {
            if let Some(node) = Self::water_node(world, discovered_node.position)
                && Self::absorb_node(world, node)
            {
                changed.push((node.position, Block::from_state_id(node.state_id)));
            }
        }
        if changed.is_empty() {
            return false;
        }

        let sponge_wet = world
            .set_block_state_if(
                sponge_pos,
                Block::WET_SPONGE.default_state.id,
                BlockFlags::NOTIFY_ALL,
                |current| current == sponge_state_id,
            )
            .is_some();

        // Defer ordinary neighbor callbacks until the sponge is wet so its own water removals
        // cannot recursively start a second absorption pass.
        for (water_pos, old_block) in changed {
            world.update_neighbors_at(&water_pos, old_block, None);
        }
        if !sponge_wet {
            return false;
        }
        world.play_block_sound(Sound::BlockSpongeAbsorb, SoundCategory::Blocks, *sponge_pos);
        true
    }

    fn absorb_node(world: &Arc<World>, node: WaterNode) -> bool {
        let block = Block::from_state_id(node.state_id);
        match node.absorption {
            Absorption::SetState(state_id) => world
                .set_block_state_if(
                    &node.position,
                    state_id,
                    BlockFlags::NOTIFY_LISTENERS,
                    |current| current == node.state_id,
                )
                .is_some(),
            Absorption::Remove => world
                .set_block_state_if(
                    &node.position,
                    BlockStateId::AIR,
                    BlockFlags::NOTIFY_LISTENERS,
                    |current| current == node.state_id,
                )
                .is_some(),
            Absorption::DropResources => Self::drop_and_remove_plant(world, node, block),
        }
    }

    fn drop_and_remove_plant(world: &Arc<World>, node: WaterNode, block: &Block) -> bool {
        let params = crate::world::loot::LootContextParameters {
            block_state: Some(BlockState::from_id(node.state_id)),
            position: Some(node.position.to_f64()),
            ..Default::default()
        };
        let drops = crate::block::block_drops(world, block, &node.position, &params);
        if world.get_block_state_id(&node.position) != node.state_id {
            return false;
        }
        for drop in drops {
            world.drop_stack(&node.position, drop);
            if world.get_block_state_id(&node.position) != node.state_id {
                return false;
            }
        }
        world
            .set_block_state_if(
                &node.position,
                BlockStateId::AIR,
                BlockFlags::NOTIFY_LISTENERS,
                |current| current == node.state_id,
            )
            .is_some()
    }

    /// Mirrors SpongeBlock.removeWaterBreadthFirstSearch: accepted nodes include the sponge at
    /// depth zero, and only drainable water nodes are eligible for traversal beyond themselves.
    fn find_water(world: &Arc<World>, start: BlockPos) -> Vec<WaterNode> {
        let mut water_nodes = Vec::new();
        let mut visited = HashSet::from([start]);
        let mut queue = VecDeque::from([(start, 0u8)]);
        let mut accepted_nodes = 1;

        while let Some((position, depth)) = queue.pop_front() {
            if depth == MAX_DEPTH || accepted_nodes == MAX_VISITED {
                continue;
            }
            for direction in DIRECTIONS {
                if accepted_nodes == MAX_VISITED {
                    break;
                }
                let next = BlockPos::new(
                    position.0.x + direction.0,
                    position.0.y + direction.1,
                    position.0.z + direction.2,
                );
                if !visited.insert(next) {
                    continue;
                }

                let Some(node) = Self::water_node(world, next) else {
                    continue;
                };

                accepted_nodes += 1;
                water_nodes.push(node);
                queue.push_back((next, depth + 1));
            }
        }
        water_nodes
    }

    fn water_node(world: &Arc<World>, position: BlockPos) -> Option<WaterNode> {
        let state_id = world.get_block_state_id(&position);
        if !world.get_fluid(&position).matches_type(&Fluid::WATER) {
            return None;
        }
        let block = Block::from_state_id(state_id);
        let absorption = if block.is_waterlogged(state_id) {
            block
                .set_waterlogged(state_id, false)
                .map(Absorption::SetState)
        } else if block == &Block::WATER {
            Some(Absorption::Remove)
        } else if matches!(
            block.id,
            pumpkin_data::BlockId::KELP
                | pumpkin_data::BlockId::KELP_PLANT
                | pumpkin_data::BlockId::SEAGRASS
                | pumpkin_data::BlockId::TALL_SEAGRASS
        ) {
            Some(Absorption::DropResources)
        } else {
            None
        }?;
        Some(WaterNode {
            position,
            state_id,
            absorption,
        })
    }
}

impl BlockBehaviour for SpongeBlock {
    fn placed(&self, args: PlacedArgs<'_>) {
        // Attempt to absorb water on placement
        Self::absorb_water(args.world, args.position);
    }

    fn on_neighbor_update(&self, args: OnNeighborUpdateArgs<'_>) {
        // A waterlogged host or aquatic plant can change without the source block being WATER.
        Self::absorb_water(args.world, args.position);
    }
}

#[pumpkin_block("minecraft:wet_sponge")]
pub struct WetSpongeBlock;

impl BlockBehaviour for WetSpongeBlock {
    fn placed(&self, args: PlacedArgs<'_>) {
        {
            // Check if placed where water evaporates (EnvironmentAttributes.WATER_EVAPORATES), if so, dry out
            if args.world.dimension.water_evaporates {
                args.world.set_block_state(
                    args.position,
                    Block::SPONGE.default_state.id,
                    BlockFlags::NOTIFY_ALL,
                );

                // Play dry sound and spawn smoke particles
                args.world.play_block_sound(
                    Sound::BlockWetSpongeDries,
                    SoundCategory::Blocks,
                    *args.position,
                );

                args.world.spawn_particle(
                    Vector3::new(
                        args.position.0.x as f64 + 0.5,
                        args.position.0.y as f64 + 1.0,
                        args.position.0.z as f64 + 0.5,
                    ),
                    Vector3::new(0.25, 0.0, 0.25),
                    0.01,
                    16,
                    Particle::Cloud,
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "sponge_tests.rs"]
mod tests;
