use pumpkin_data::block_properties::{DoubleBlockHalf, TallSeagrassLikeProperties};
use pumpkin_data::{Block, BlockId, BlockState, BlockStateId};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::random::{RandomGenerator, RandomImpl, xoroshiro128::Xoroshiro};
use pumpkin_world::generation::feature::{
    configured_features::{BONE_MEAL_FEATURES, CONFIGURED_FEATURES, ConfiguredFeature},
    placed_features::{Feature, PLACED_FEATURES},
};
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::BlockFlags;
use rand::RngExt;

use super::spreading_snowy_block::{SnowyBlock, SpreadingSnowyBlock};
use crate::block::{
    BlockBehaviour, BlockMetadata, GetStateForNeighborUpdateArgs, OnPlaceArgs, RandomTickArgs,
};

pub struct GrassBlock;

impl BlockMetadata for GrassBlock {
    fn ids() -> Box<[BlockId]> {
        [BlockId::GRASS_BLOCK].into()
    }
}

impl BlockBehaviour for GrassBlock {
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        SnowyBlock::on_place(args.block, args.world, args.position)
    }

    fn get_state_for_neighbor_update(
        &self,
        args: GetStateForNeighborUpdateArgs<'_>,
    ) -> BlockStateId {
        SnowyBlock::get_state_for_neighbor_update(&args)
    }

    fn random_tick(&self, args: RandomTickArgs<'_>) {
        let state = args.world.get_block_state(args.position);
        SpreadingSnowyBlock::random_tick(
            state,
            args.world,
            args.position,
            &Block::DIRT,
            Block::GRASS_BLOCK.default_state,
        );
    }

    fn is_valid_bonemeal_target(&self, args: crate::block::BonemealArgs<'_>) -> bool {
        let above = args.position.up();
        args.world.is_in_height_limit(above.0.y)
            && args.world.is_loaded(&above)
            && args.world.get_block_state(&above).is_air()
    }

    fn is_bonemeal_success(&self, _args: crate::block::BonemealArgs<'_>) -> bool {
        true
    }

    fn perform_bonemeal(&self, args: crate::block::BonemealArgs<'_>) -> bool {
        const SPREAD_ATTEMPTS: i32 = 128;
        const ATTEMPTS_PER_STEP: i32 = 16;
        const FLOWER_CHANCE: i32 = 8;

        if args.block != &Block::GRASS_BLOCK {
            return true;
        }

        let origin = args.position.up();
        let mut attempted = false;
        let mut committed = false;
        for attempt in 0..SPREAD_ATTEMPTS {
            let mut target = origin;
            let mut valid = true;
            for _ in 0..attempt / ATTEMPTS_PER_STEP {
                let offset_x = rand::rng().random_range(0..3) - 1;
                let offset_y =
                    ((rand::rng().random_range(0..3) - 1) * rand::rng().random_range(0..3)) / 2;
                let offset_z = rand::rng().random_range(0..3) - 1;
                target = BlockPos::new(
                    target.0.x + offset_x,
                    target.0.y + offset_y,
                    target.0.z + offset_z,
                );

                if !args.world.is_loaded(&target)
                    || args.world.get_block(&target.down()) != args.block
                    || args.world.get_block_state(&target).is_full_cube()
                {
                    valid = false;
                    break;
                }
            }

            if !valid {
                continue;
            }
            let target_state = args.world.get_block_state(&target);
            if Block::from_state_id(target_state.id) == &Block::SHORT_GRASS
                && rand::rng().random_range(0..10) == 0
            {
                let above = target.up();
                if args.world.is_in_height_limit(above.0.y)
                    && args.world.is_loaded(&above)
                    && args.world.get_block_state(&above).is_air()
                {
                    attempted = true;
                    committed |= place_tall_grass(
                        args.world,
                        (*args.position, args.state_id),
                        target,
                        target_state.id,
                    );
                }
            } else if target_state.is_air() && args.world.is_in_height_limit(target.0.y) {
                let selected = if rand::rng().random_range(0..FLOWER_CHANCE) == 0 {
                    biome_bonemeal_state(args.world, target)
                } else {
                    Some((Block::SHORT_GRASS.default_state, false))
                };
                let Some((state, schedule_tick)) = selected else {
                    continue;
                };
                let placed_block = Block::from_state_id(state.id);
                if !args.world.block_registry.can_place_at(
                    None,
                    Some(args.world),
                    args.world.as_ref(),
                    None,
                    placed_block,
                    state,
                    &target,
                    None,
                    None,
                ) {
                    continue;
                }
                attempted = true;
                let placed = args.world.set_block_states_from_observed(&[
                    (
                        *args.position,
                        args.state_id,
                        args.state_id,
                        BlockFlags::empty(),
                    ),
                    (
                        target,
                        target_state.id,
                        state.id,
                        BlockFlags::NOTIFY_LISTENERS,
                    ),
                ]);
                committed |= placed;
                if placed && schedule_tick {
                    args.world
                        .schedule_block_tick(placed_block, target, 1, TickPriority::Normal);
                }
            }
        }
        committed || !attempted
    }
}

fn place_tall_grass(
    world: &std::sync::Arc<crate::world::World>,
    source: (BlockPos, BlockStateId),
    position: BlockPos,
    observed: BlockStateId,
) -> bool {
    let lower = Block::TALL_GRASS.default_state.id;
    let upper = position.up();
    let upper_observed = world.get_block_state_id(&upper);
    if !upper_observed.to_state().is_air() {
        return false;
    }
    let mut props = TallSeagrassLikeProperties::from_state_id(lower);
    props.half = DoubleBlockHalf::Upper;
    world.set_block_states_from_observed(&[
        (source.0, source.1, source.1, BlockFlags::empty()),
        (position, observed, lower, BlockFlags::NOTIFY_LISTENERS),
        (
            upper,
            upper_observed,
            props.to_state_id(&Block::TALL_GRASS),
            BlockFlags::NOTIFY_LISTENERS,
        ),
    ])
}

fn biome_bonemeal_state(
    world: &crate::world::World,
    position: BlockPos,
) -> Option<(&'static BlockState, bool)> {
    let features: Vec<_> = world
        .get_biome(&position)
        .features
        .iter()
        .flat_map(|step| step.iter())
        .filter_map(|key| PLACED_FEATURES.get(key))
        .filter_map(|feature| match &feature.feature {
            Feature::Named(key) if BONE_MEAL_FEATURES.contains(key) => Some(*key),
            Feature::Named(_) | Feature::Inlined(_) => None,
        })
        .collect();
    if features.is_empty() {
        return None;
    }

    let mut random = RandomGenerator::Xoroshiro(Xoroshiro::from_seed(rand::rng().random()));
    let key = features[random.next_bounded_i32(features.len() as i32) as usize];
    let ConfiguredFeature::SimpleBlock(feature) = CONFIGURED_FEATURES.get(&key)? else {
        return None;
    };
    feature
        .to_place
        .get_for_bonemeal(&mut random, position)
        .map(|state| (state, feature.schedule_tick.unwrap_or(false)))
}
