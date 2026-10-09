use std::sync::Arc;

use pumpkin_data::{Block, BlockState, entity::EntityType, tag::Taggable};
use pumpkin_util::math::{
    boundingbox::{BoundingBox, EntityDimensions},
    position::BlockPos,
    vector3::Vector3,
};
use rand::RngExt;
use uuid::Uuid;

use crate::entity::{Entity, EntityBase, mob::Mob};
use crate::world::World;

// BlockCollisions and VoxelShape.calculateFace use this tolerance.
const SHAPE_EPSILON: f64 = 1.0e-7;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SpawnStrategy {
    OnTopOfCollider,
    OnTopOfColliderNoLeaves,
}

impl SpawnStrategy {
    fn can_spawn_on(
        self,
        world: &World,
        pos: &BlockPos,
        state: &BlockState,
        above_state: &BlockState,
    ) -> bool {
        // SpawnUtil.Strategy.ON_TOP_OF_COLLIDER[_NO_LEAVES] uses the empty context.
        if self == Self::OnTopOfColliderNoLeaves
            && state
                .id
                .to_block()
                .has_tag(&pumpkin_data::tag::Block::MINECRAFT_LEAVES)
        {
            return false;
        }
        let mut above_empty = true;
        world.for_each_collision_shape(above_state, &pos.up(), None, &mut |_| {
            above_empty = false;
        });
        above_empty && is_face_full_up(world, pos, state)
    }
}

fn is_face_full_up(world: &World, pos: &BlockPos, state: &BlockState) -> bool {
    // Block.isFaceFull -> VoxelShape.calculateFace slices the union just below y=1.
    let mut face = Vec::new();
    world.for_each_collision_shape(state, pos, None, &mut |shape| {
        if shape.min.y <= 1.0 - SHAPE_EPSILON && shape.max.y > 1.0 - SHAPE_EPSILON {
            face.push(shape);
        }
    });
    // Block.isShapeFullBlock compares the face with the unit cube using NOT_SAME.
    if face.iter().any(|shape| {
        shape.min.x < -SHAPE_EPSILON
            || shape.min.z < -SHAPE_EPSILON
            || shape.max.x > 1.0 + SHAPE_EPSILON
            || shape.max.z > 1.0 + SHAPE_EPSILON
    }) {
        return false;
    }
    let mut xs = vec![0.0, 1.0];
    let mut zs = vec![0.0, 1.0];
    for shape in &face {
        xs.extend([shape.min.x.clamp(0.0, 1.0), shape.max.x.clamp(0.0, 1.0)]);
        zs.extend([shape.min.z.clamp(0.0, 1.0), shape.max.z.clamp(0.0, 1.0)]);
    }
    xs.sort_unstable_by(f64::total_cmp);
    zs.sort_unstable_by(f64::total_cmp);
    xs.dedup();
    zs.dedup();
    xs.windows(2).all(|x| {
        zs.windows(2).all(|z| {
            face.iter().any(|shape| {
                shape.min.x <= x[0] + SHAPE_EPSILON
                    && shape.max.x >= x[1] - SHAPE_EPSILON
                    && shape.min.z <= z[0] + SHAPE_EPSILON
                    && shape.max.z >= z[1] - SHAPE_EPSILON
            })
        })
    })
}

fn loaded_block_state_or_void_air(world: &World, pos: &BlockPos) -> Option<&'static BlockState> {
    // Level.getBlockState returns void air outside the dimension's build height.
    if world.is_in_height_limit(pos.0.y) {
        world.get_block_state_if_loaded(pos)
    } else {
        Some(Block::VOID_AIR.default_state)
    }
}

fn is_clear_loaded_spawn_space(world: &World, bounds: BoundingBox) -> bool {
    // SpawnUtil.trySpawnMob -> level.noCollision uses empty-context dynamic collision shapes.
    // BlockCollisions.java:55-60 pads every axis for shapes extending from neighboring cells.
    let padded = bounds.expand(SHAPE_EPSILON, SHAPE_EPSILON, SHAPE_EPSILON);
    let min = padded.min_block_pos().add(-1, -1, -1);
    let max = BlockPos::floored_v(padded.max).add(1, 1, 1);
    for pos in BlockPos::iterate(min, max) {
        let Some(state) = loaded_block_state_or_void_air(world, &pos) else {
            return false;
        };
        let mut collided = false;
        world.for_each_collision_shape(state, &pos, None, &mut |shape| {
            collided |= shape.at_pos(pos).intersects(&bounds);
        });
        if collided {
            return false;
        }
    }
    true
}

#[expect(clippy::too_many_arguments)]
pub fn try_spawn_mob<T: Mob + 'static>(
    entity_type: &'static EntityType,
    spawn_reason: crate::entity::mob::spawn::SpawnReason,
    create: fn(Entity) -> Arc<T>,
    world: &Arc<World>,
    start: &BlockPos,
    spawn_attempts: i32,
    spawn_range_xz: i32,
    spawn_range_y: i32,
    strategy: SpawnStrategy,
    check_collisions: bool,
) -> Option<Arc<T>> {
    try_spawn_mob_with_random(
        entity_type,
        spawn_reason,
        create,
        world,
        start,
        spawn_attempts,
        spawn_range_xz,
        spawn_range_y,
        strategy,
        check_collisions,
        &mut rand::rng(),
    )
}

/// Runs SpawnUtil.trySpawnMob with caller-owned randomness, preserving finalization and admission.
#[expect(
    clippy::too_many_arguments,
    reason = "SpawnUtil's arguments plus the random source"
)]
pub(crate) fn try_spawn_mob_with_random<T: Mob + 'static>(
    entity_type: &'static EntityType,
    spawn_reason: crate::entity::mob::spawn::SpawnReason,
    create: fn(Entity) -> Arc<T>,
    world: &Arc<World>,
    start: &BlockPos,
    spawn_attempts: i32,
    spawn_range_xz: i32,
    spawn_range_y: i32,
    strategy: SpawnStrategy,
    check_collisions: bool,
    random: &mut impl rand::Rng,
) -> Option<Arc<T>> {
    for _ in 0..spawn_attempts {
        let dx = random.random_range(-spawn_range_xz..=spawn_range_xz);
        let dz = random.random_range(-spawn_range_xz..=spawn_range_xz);
        let search_pos = start.add(dx, spawn_range_y, dz);
        let in_border = world
            .worldborder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_block(search_pos.0.x, search_pos.0.z);
        if !in_border {
            continue;
        }
        let Some(spawn_pos) =
            move_to_possible_spawn_position(world, spawn_range_y, search_pos, strategy)
        else {
            continue;
        };

        let position = Vector3::new(
            f64::from(spawn_pos.0.x) + 0.5,
            f64::from(spawn_pos.0.y),
            f64::from(spawn_pos.0.z) + 0.5,
        );
        if check_collisions {
            let dimensions = EntityDimensions::new(
                entity_type.dimension[0],
                entity_type.dimension[1],
                entity_type.eye_height,
            );
            let bounding_box =
                BoundingBox::new_from_pos(position.x, position.y, position.z, &dimensions);
            let clear = match strategy {
                SpawnStrategy::OnTopOfCollider => world.is_space_empty(bounding_box),
                SpawnStrategy::OnTopOfColliderNoLeaves => {
                    is_clear_loaded_spawn_space(world, bounding_box)
                }
            };
            if !clear {
                continue;
            }
        }

        let mob = create(Entity::from_uuid(
            Uuid::new_v4(),
            world.clone(),
            position,
            entity_type,
        ));
        let entity = mob.clone() as Arc<dyn EntityBase>;
        // SpawnUtil.trySpawnMob uses EntityType.create, which finalizes before instance checks.
        crate::entity::mob::spawn::finalize_spawn_with_reason(&entity, world, spawn_reason, None);
        if !mob.check_spawn_rules(world, spawn_reason) || !mob.check_spawn_obstruction(world) {
            continue;
        }
        if !world.spawn_entity(entity) {
            continue;
        }
        return Some(mob);
    }

    None
}

fn move_to_possible_spawn_position(
    world: &World,
    spawn_range_y: i32,
    mut search_pos: BlockPos,
    strategy: SpawnStrategy,
) -> Option<BlockPos> {
    // Vanilla reads load chunks; the new golem strategy rejects missing terrain on tick workers.
    let read = |pos: &BlockPos| match strategy {
        SpawnStrategy::OnTopOfCollider => Some(world.get_block_state(pos)),
        SpawnStrategy::OnTopOfColliderNoLeaves => loaded_block_state_or_void_air(world, pos),
    };
    let mut above_state = read(&search_pos)?;

    for _ in -spawn_range_y..=spawn_range_y {
        search_pos = search_pos.down();
        let current_state = read(&search_pos)?;
        if strategy.can_spawn_on(world, &search_pos, current_state, above_state) {
            return Some(search_pos.up());
        }
        above_state = current_state;
    }

    None
}
