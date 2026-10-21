use std::sync::Arc;

use pumpkin_data::{
    Block, BlockDirection, BlockId, BlockStateId, block_properties::WallTorchLikeProperties,
    entity::EntityType, world::WorldEvent,
};
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;

use crate::{
    block::{BlockBehaviour, BlockMetadata, OnPlaceArgs, PlacedArgs},
    entity::{
        Entity, EntityBase,
        passive::{iron_golem::IronGolemEntity, snow_golem::SnowGolemEntity},
    },
    world::World,
};

pub struct GolemPattern {
    entity_type: &'static EntityType,
    body: &'static Block,
    blocks: Vec<(BlockPos, BlockStateId)>,
    base: BlockPos,
}

// Mojang uses some BlockPattern magic, way too complex tbh
#[must_use]
pub fn find_golem_pattern(world: &Arc<World>, pos: &BlockPos) -> Option<GolemPattern> {
    let head = world.get_block_state_id(pos);
    let down_pos = pos.down();
    let upper = world.get_block(&down_pos);
    let lower = world.get_block(&down_pos.down());

    if upper == &Block::SNOW_BLOCK && lower == &Block::SNOW_BLOCK {
        return Some(GolemPattern {
            entity_type: &EntityType::SNOW_GOLEM,
            body: &Block::SNOW_BLOCK,
            blocks: vec![
                (*pos, head),
                (down_pos, upper.default_state.id),
                (down_pos.down(), lower.default_state.id),
            ],
            base: down_pos.down(),
        });
    }

    if upper != &Block::IRON_BLOCK || lower != &Block::IRON_BLOCK {
        return None;
    }

    for dir in [BlockDirection::North, BlockDirection::West] {
        let arm1 = down_pos.offset(dir.to_offset());
        let arm2 = down_pos.offset(dir.opposite().to_offset());

        if world.get_block(&arm1) == &Block::IRON_BLOCK
            && world.get_block(&arm2) == &Block::IRON_BLOCK
        {
            return Some(GolemPattern {
                entity_type: &EntityType::IRON_GOLEM,
                body: &Block::IRON_BLOCK,
                blocks: vec![
                    (*pos, head),
                    (down_pos, upper.default_state.id),
                    (down_pos.down(), lower.default_state.id),
                    (arm1, Block::IRON_BLOCK.default_state.id),
                    (arm2, Block::IRON_BLOCK.default_state.id),
                ],
                base: down_pos.down(),
            });
        }
    }

    None
}

fn spawn_golem(world: &Arc<World>, pattern: GolemPattern) {
    let updates: Vec<_> = pattern
        .blocks
        .iter()
        .map(|(pos, expected)| {
            (
                *pos,
                *expected,
                BlockStateId::AIR,
                BlockFlags::NOTIFY_LISTENERS,
            )
        })
        .collect();
    if !world.set_block_states_from_observed(&updates) {
        return;
    }
    for (pos, _) in pattern.blocks {
        world.sync_world_event(
            WorldEvent::ParticlesDestroyBlock,
            pos,
            pattern.body.default_state.id.as_u16().into(),
        );
    }

    let entity = Entity::new(
        world.clone(),
        pattern.base.to_centered_f64(),
        pattern.entity_type,
    );
    let golem: Arc<dyn EntityBase> = if pattern.entity_type == &EntityType::SNOW_GOLEM {
        SnowGolemEntity::new(entity)
    } else {
        IronGolemEntity::new(entity)
    };
    world.spawn_entity(golem);
}

pub struct CarvedPumpkinBlock;

impl BlockMetadata for CarvedPumpkinBlock {
    fn ids() -> Box<[BlockId]> {
        [BlockId::JACK_O_LANTERN, BlockId::CARVED_PUMPKIN].into()
    }
}

impl BlockBehaviour for CarvedPumpkinBlock {
    fn on_place(&self, args: OnPlaceArgs<'_>) -> BlockStateId {
        let mut props = WallTorchLikeProperties::default(args.block);
        props.facing = args
            .player
            .living_entity
            .entity
            .get_horizontal_facing()
            .opposite();
        props.to_state_id(args.block)
    }

    fn placed(&self, args: PlacedArgs<'_>) {
        if let Some(pattern) = find_golem_pattern(args.world, args.position) {
            spawn_golem(args.world, pattern);
        }
    }
}
