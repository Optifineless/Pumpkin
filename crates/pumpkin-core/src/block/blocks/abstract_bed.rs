use pumpkin_data::{
    Block, BlockStateId, HorizontalFacingExt,
    block_properties::{BedPart, WhiteBedLikeProperties as BedProperties},
    tag::{self, Taggable},
};
use pumpkin_util::math::{
    boundingbox::{BoundingBox, EntityDimensions},
    position::BlockPos,
    vector3::Vector3,
};

use crate::{block::GetStateForNeighborUpdateArgs, entity::Entity, world::World};

/// Resolves either half to the head state, rejecting a missing or mismatched partner.
pub fn head_and_foot(
    world: &World,
    block: &Block,
    pos: BlockPos,
) -> Option<(BlockPos, BlockPos, BlockStateId)> {
    // AbstractBedBlock.useWithoutItem / getNeighbourDirection.
    let state = world.get_block_state_id(&pos);
    if Block::from_state_id(state) != block {
        return None;
    }
    let properties = BedProperties::from_state_id(state);
    let direction = if properties.part == BedPart::Foot {
        properties.facing
    } else {
        properties.facing.opposite()
    };
    let other = pos.offset(direction.to_offset());
    let other_state = world.get_block_state_id(&other);
    if Block::from_state_id(other_state) != block
        || BedProperties::from_state_id(other_state).part == properties.part
    {
        return None;
    }
    Some(if properties.part == BedPart::Head {
        (pos, other, state)
    } else {
        (other, pos, other_state)
    })
}

pub(super) fn update_shape(args: &GetStateForNeighborUpdateArgs<'_>) -> BlockStateId {
    // AbstractBedBlock.updateShape: only the partner's direction changes this half.
    let mut props = BedProperties::from_state_id(args.state_id);
    let direction = if props.part == BedPart::Foot {
        props.facing
    } else {
        props.facing.opposite()
    };
    if args.direction != direction.to_block_direction() {
        return args.state_id;
    }
    if Block::from_state_id(args.neighbor_state_id) != args.block {
        return Block::AIR.default_state.id;
    }
    let other = BedProperties::from_state_id(args.neighbor_state_id);
    if other.part == props.part {
        return Block::AIR.default_state.id;
    }
    props.occupied = other.occupied;
    props.to_state_id(args.block)
}

/// Returns `LivingEntity`'s sleeping position using the extracted bed shape height.
pub fn sleep_position(world: &World, head: BlockPos) -> Option<Vector3<f64>> {
    let (block, state) = world.get_block_and_state_id(&head);
    if !is_bed(block) {
        return None;
    }
    let (_, foot, _) = head_and_foot(world, block, head)?;
    // StrawBedBlock.getSleepHeight uses the foot, below the pillow.
    let shape = if block == &Block::STRAW_BED {
        world.get_block_state(&foot)
    } else {
        state.to_state()
    };
    let height = shape
        .get_block_outline_shapes()
        .map(|shape| shape.max.y)
        .reduce(f64::max)?;
    // LivingEntity.setPosToBed: adjustHeight = 0.125.
    Some(head.to_f64().add_raw(0.5, height + 0.125, 0.5))
}

/// Finds a supported, empty place beside a bed for the standing entity.
pub fn stand_up_position(world: &World, entity: &Entity, head: BlockPos) -> Vector3<f64> {
    // AbstractBedBlock.findStandUpPosition / findBunkBedStandUpPosition.
    let state = world.get_block_state_id(&head);
    if !is_bed(Block::from_state_id(state)) {
        return head.up().to_f64().add_raw(0.5, 0.1, 0.5);
    }
    stand_up_position_with_facing(
        world,
        entity,
        head,
        BedProperties::from_state_id(state).facing,
    )
}

/// Finds a standing position using the bed direction saved before its removal.
pub fn stand_up_position_with_facing(
    world: &World,
    entity: &Entity,
    head: BlockPos,
    facing: pumpkin_data::block_properties::HorizontalFacing,
) -> Vector3<f64> {
    // LivingEntity.stopSleeping retains FACING before onStopSleeping can destroy a straw bed.
    let forward = facing.to_offset();
    let mut side = facing.rotate_clockwise().to_offset();
    let yaw = entity.yaw.load().to_radians();
    if side.x as f32 * -yaw.sin() + side.z as f32 * yaw.cos() > 0.0 {
        side = side * -1;
    }
    let surround = [
        side,
        side - forward,
        side - forward * 2,
        forward * -2,
        side * -1 - forward * 2,
        side * -1 - forward,
        side * -1,
        side * -1 + forward,
        forward,
        side + forward,
    ];
    let above = [Vector3::default(), forward * -1];
    let bunk = is_bed(world.get_block(&head.down()));
    for check_dangerous in [true, false] {
        for (base, offsets) in [
            (head, surround.as_slice()),
            (head.down(), if bunk { surround.as_slice() } else { &[] }),
            (head, above.as_slice()),
        ] {
            for offset in offsets {
                if let Some(position) =
                    safe_dismount_location(world, entity, base.offset(*offset), check_dangerous)
                {
                    return position;
                }
            }
        }
    }
    // LivingEntity.stopSleeping fallback when no stand-up position is free.
    head.up().to_f64().add_raw(0.5, 0.1, 0.5)
}

fn is_bed(block: &Block) -> bool {
    // AbstractBedBlock includes StrawBedBlock, which is absent from minecraft:beds.
    block == &Block::STRAW_BED || block.has_tag(&tag::Block::MINECRAFT_BEDS)
}

fn non_climbable_height(world: &World, pos: BlockPos) -> f64 {
    // DismountHelper.nonClimbableShape.
    let (block, state) = world.get_block_and_state_id(&pos);
    if block.has_tag(&tag::Block::MINECRAFT_CLIMBABLE)
        || block.has_tag(&tag::Block::MINECRAFT_TRAPDOORS)
            && pumpkin_data::block_properties::OakTrapdoorLikeProperties::from_state_id(state).open
    {
        return f64::NEG_INFINITY;
    }
    state
        .to_state()
        .get_block_collision_shapes_at(&pos)
        .map(|shape| shape.max.y)
        .fold(f64::NEG_INFINITY, f64::max)
}

fn safe_dismount_location(
    world: &World,
    entity: &Entity,
    pos: BlockPos,
    check_dangerous: bool,
) -> Option<Vector3<f64>> {
    // DismountHelper.findSafeDismountLocation / BlockGetter.getBlockFloorHeight.
    let dangerous = |pos| {
        crate::world::natural_spawner::is_block_dangerous(
            entity.entity_type,
            world.get_block_state(&pos),
        )
    };
    if check_dangerous && dangerous(pos) {
        return None;
    }
    let mut floor = non_climbable_height(world, pos);
    if floor == f64::NEG_INFINITY {
        let below = non_climbable_height(world, pos.down());
        floor = if below >= 1.0 {
            below - 1.0
        } else {
            f64::NEG_INFINITY
        };
    }
    if !floor.is_finite()
        || floor >= 1.0
        || check_dangerous && floor <= 0.0 && dangerous(pos.down())
    {
        return None;
    }
    if entity.entity_type == &pumpkin_data::entity::EntityType::PLAYER
        && [pos, pos.up()].into_iter().any(|pos| {
            world
                .get_block(&pos)
                .has_tag(&tag::Block::MINECRAFT_INVALID_SPAWN_INSIDE)
        })
    {
        return None;
    }
    let location = pos.to_f64().add_raw(0.5, floor, 0.5);
    let dimensions = EntityDimensions::new(
        entity.entity_type.dimension[0],
        entity.entity_type.dimension[1],
        entity.entity_type.eye_height,
    );
    let bounds = BoundingBox::new_from_pos(location.x, location.y, location.z, &dimensions);
    let border = world
        .worldborder
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // WorldBorder.isWithinBounds(AABB) subtracts 1.0E-5F from the maximum edges.
    let border_epsilon = f64::from(1.0e-5f32);
    if !border.contains(bounds.min.x, bounds.min.z)
        || !border.contains(bounds.max.x - border_epsilon, bounds.max.z - border_epsilon)
        || !world.is_space_empty(bounds)
    {
        return None;
    }
    Some(location)
}

/// Selects the environment rule belonging to this bed species.
pub fn bed_rule(world: &World, block: &Block) -> pumpkin_data::dimension::BedRule {
    // StrawBedBlock.getBedEnvironmentAttribute / AbstractBedBlock.getBedRule.
    if block == &Block::STRAW_BED {
        world.dimension.straw_bed_rule
    } else {
        world.dimension.bed_rule
    }
}

pub(super) fn head_inside_border(world: Option<&World>, head: BlockPos) -> bool {
    // AbstractBedBlock.getStateForPlacement / WorldBorder.isWithinBounds(BlockPos).
    world.is_none_or(|world| {
        world
            .worldborder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(f64::from(head.0.x), f64::from(head.0.z))
    })
}

pub(super) fn sleep_admission(
    world: &World,
    player: &crate::entity::player::Player,
    head: BlockPos,
    foot: BlockPos,
) -> Option<pumpkin_util::text::TextComponent> {
    use pumpkin_data::translation;
    // ServerPlayer.bedInRange / isReachableBedBlock: bottom centres, three horizontally and two vertically.
    if ![head, foot].into_iter().any(|pos| {
        player
            .position()
            .is_within_bounds(pos.to_f64().add_raw(0.5, 0.0, 0.5), 3.0, 2.0, 3.0)
    }) {
        return Some(pumpkin_macros::translate_cross!(
            translation::java::BLOCK_MINECRAFT_BED_TOO_FAR_AWAY,
            translation::bedrock::TILE_BED_TOOFAR
        ));
    }
    // ServerPlayer.bedBlocked / Player.freeAt / BlockBehaviour.Properties.isSuffocating.
    if [head.up(), foot.up()]
        .into_iter()
        .any(|pos| suffocates_at(world, pos))
    {
        return Some(pumpkin_macros::translate_cross!(
            translation::java::BLOCK_MINECRAFT_BED_OBSTRUCTED,
            translation::bedrock::TILE_BED_OBSTRUCTED
        ));
    }
    None
}

pub(super) fn monsters_prevent_sleep(
    world: &World,
    player: &crate::entity::player::Player,
    head: BlockPos,
) -> bool {
    use pumpkin_data::entity::{EntityType, MobCategory};
    if player.gamemode.load() == pumpkin_util::gamemode::GameMode::Creative {
        return false;
    }
    // ServerPlayer.startSleepInBed queries Monster bounding boxes around only the head's bottom centre.
    let center = head.to_f64().add_raw(0.5, 0.0, 0.5);
    let bounds = BoundingBox::new(
        center.add_raw(-8.0, -5.0, -8.0),
        center.add_raw(8.0, 5.0, 8.0),
    );
    world.get_all_at_box(&bounds).iter().any(|other| {
        let kind = other.get_entity().entity_type;
        // Monster.isPreventingPlayerRest excludes Mob, AbstractGolem, AbstractCubeMob and Animal subclasses.
        if kind.category != &MobCategory::MONSTER
            || [
                EntityType::GHAST.id,
                EntityType::PHANTOM.id,
                EntityType::SLIME.id,
                EntityType::MAGMA_CUBE.id,
                EntityType::SULFUR_CUBE.id,
                EntityType::HOGLIN.id,
                EntityType::SHULKER.id,
                EntityType::ENDER_DRAGON.id,
                EntityType::CAMEL_HUSK.id,
                EntityType::ZOMBIE_HORSE.id,
                EntityType::ZOMBIE_NAUTILUS.id,
            ]
            .contains(&kind.id)
        {
            return false;
        }
        // ZombifiedPiglin.isPreventingPlayerRest is the Monster override.
        kind != &EntityType::ZOMBIFIED_PIGLIN
            || other
                .get_mob()
                .and_then(crate::entity::mob::Mob::as_neutral)
                .is_some_and(|neutral| neutral.is_angry_at(player, world))
    })
}

fn suffocates_at(world: &World, pos: BlockPos) -> bool {
    let (block, state) = world.get_block_and_state(&pos);
    // Player.freeAt calls the predicates wired by Blocks.java, not legacy solidity.
    if [
        &Block::FARMLAND,
        &Block::SOUL_SAND,
        &Block::DIRT_PATH,
        &Block::MUD,
    ]
    .contains(&block)
    {
        return true; // Blocks::always, including these partial collision shapes.
    }
    if block == &Block::PISTON || block == &Block::STICKY_PISTON {
        return !pumpkin_data::block_properties::StickyPistonLikeProperties::from_state_id(
            state.id,
        )
        .extended;
    }
    if block.has_tag(&tag::Block::MINECRAFT_SHULKER_BOXES) {
        // Blocks.NOT_CLOSED_SHULKER; this server's container tracks its open/closed viewer state.
        return world.get_block_entity(&pos).is_none_or(|entity| {
            entity
                .as_any()
                .downcast_ref::<crate::block::entities::shulker_box::ShulkerBoxBlockEntity>()
                .is_none_or(|shulker| {
                    shulker
                        .viewers
                        .current
                        .load(std::sync::atomic::Ordering::Relaxed)
                        == 0
                })
        });
    }
    if block.has_tag(&tag::Block::C_GLASS_BLOCKS)
        || block.has_tag(&tag::Block::MINECRAFT_LEAVES)
        || block == &Block::MANGROVE_ROOTS
        || block == &Block::MOVING_PISTON
        || block.name.ends_with("copper_grate")
    {
        return false; // Blocks::never on glass, leaves, roots, moving pistons and the grate collection.
    }
    block.has_tag(&tag::Block::MINECRAFT_CAUSES_SUFFOCATION) && state.is_full_cube()
}

// AbstractBedBlock.playerWillDestroy removes the loot-bearing head before the foot cascade.
pub(super) fn player_will_destroy(args: crate::block::PlayerWillDestroyArgs<'_>) {
    let props = BedProperties::from_state_id(args.state.id);
    if args.player.gamemode.load() != pumpkin_util::GameMode::Creative
        || props.part != BedPart::Foot
    {
        return;
    }
    let head = args.position.offset(props.facing.to_offset());
    let state = args.world.get_block_state_id(&head);
    if Block::from_state_id(state) == args.block
        && BedProperties::from_state_id(state).part == BedPart::Head
    {
        super::player_destroy::remove_partner(args, head, Block::AIR.default_state.id);
    }
}
