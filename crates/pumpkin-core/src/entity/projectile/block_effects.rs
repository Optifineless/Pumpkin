use crate::{entity::EntityBase, world::World};
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos, vector3::Vector3};
use rustc_hash::FxHashSet;

const EPSILON: f64 = 1.0E-5f32 as f64;
const MAX_MOVEMENT_ITERATIONS: usize = 16;

// Entity.applyEffectsFromBlocks / checkInsideBlocks for an AbstractArrow's straight movement.
pub(super) fn apply(caller: &dyn EntityBase, from: Vector3<f64>, to: Vector3<f64>) {
    let entity = caller.get_entity();
    if !entity.is_affected_by_blocks() {
        return;
    }
    let world = entity.world.load();
    let Some(server) = world.server.upgrade() else {
        return;
    };
    let target_box = entity.bounding_box.load().expand_all(-EPSILON);
    let movement = to - from;
    let mut visited = FxHashSet::default();
    let mut visit = |pos: BlockPos| {
        if !entity.is_alive() || !visited.insert(pos) {
            return;
        }
        let (block, state) = world.get_block_and_state(&pos);
        if state.is_air() {
            return;
        }
        let shape = world
            .block_registry
            .get_inside_collision_shape(block, &world, state, &pos)
            .at_pos(pos);
        let offset_min = target_box.min - to;
        let offset_max = target_box.max - to;
        let expanded = BoundingBox::new(shape.min - offset_max, shape.max - offset_min);
        if expanded.intersects(&BoundingBox::new(from, from))
            || super::clip::clip_box(from, movement, expanded).is_some()
        {
            world
                .block_registry
                .on_entity_collision(block, &world, caller, &pos, state, &server);
        }
    };
    for_each_block_intersected_between(from, to, target_box, &mut visit);
}

// BlockGetter.forEachBlockIntersectedBetween / addCollisionsAlongTravel, with Entity's 16-step limit.
fn for_each_block_intersected_between(
    from: Vector3<f64>,
    to: Vector3<f64>,
    target: BoundingBox,
    visitor: &mut impl FnMut(BlockPos),
) {
    let delta = to - from;
    let start = target.shift(delta * -1.0);
    visit_box(start, delta, visitor);
    if delta.length_squared() >= EPSILON * EPSILON {
        let dir = furthest_corner(delta);
        let size = target.max - target.min;
        let center = (target.min + target.max) * 0.5;
        let corner = center + size.multiply(dir.x, dir.y, dir.z) * 0.5;
        let origin = corner - delta;
        let iterations = std::cell::Cell::new(0);
        let visitor = std::cell::RefCell::new(&mut *visitor);
        // traverse_blocks uses the same DDA axis ordering as BlockGetter.addCollisionsAlongTravel.
        World::traverse_blocks(origin, corner, |pos, _| {
            let bounds = BoundingBox::full_block().at_pos(*pos);
            if let Some((t, _)) = super::clip::clip_box(origin, delta, bounds) {
                iterations.set(iterations.get() + 1);
                if iterations.get() >= MAX_MOVEMENT_ITERATIONS {
                    return Some(());
                }
                let hit = origin + delta * t;
                let clamp = |v: f64, low: f64| v.clamp(low + EPSILON, low + 1.0 - EPSILON);
                let hit = Vector3::new(
                    clamp(hit.x, f64::from(pos.0.x)),
                    clamp(hit.y, f64::from(pos.0.y)),
                    clamp(hit.z, f64::from(pos.0.z)),
                );
                let opposite = hit - size.multiply(dir.x, dir.y, dir.z);
                let opposite = Vector3::new(
                    opposite.x.floor() as i32,
                    opposite.y.floor() as i32,
                    opposite.z.floor() as i32,
                );
                let corner_pos = pos.0;
                let min = Vector3::new(
                    corner_pos.x.min(opposite.x),
                    corner_pos.y.min(opposite.y),
                    corner_pos.z.min(opposite.z),
                );
                let max = Vector3::new(
                    corner_pos.x.max(opposite.x),
                    corner_pos.y.max(opposite.y),
                    corner_pos.z.max(opposite.z),
                );
                visit_positions(min, max, delta, &mut **visitor.borrow_mut());
            }
            None
        });
    }
    visit_box(target, delta, visitor);
}

fn visit_box(bounds: BoundingBox, direction: Vector3<f64>, visitor: &mut impl FnMut(BlockPos)) {
    visit_positions(
        bounds.min_block_pos().0,
        bounds.max_block_pos().0,
        direction,
        visitor,
    );
}

fn visit_positions(
    min: Vector3<i32>,
    max: Vector3<i32>,
    direction: Vector3<f64>,
    visitor: &mut impl FnMut(BlockPos),
) {
    let coordinate = |low: i32, high: i32, offset: i32, motion: f64| {
        if motion >= 0.0 {
            low + offset
        } else {
            high - offset
        }
    };
    for z in 0..=max.z - min.z {
        for y in 0..=max.y - min.y {
            for x in 0..=max.x - min.x {
                visitor(BlockPos::new(
                    coordinate(min.x, max.x, x, direction.x),
                    coordinate(min.y, max.y, y, direction.y),
                    coordinate(min.z, max.z, z, direction.z),
                ));
            }
        }
    }
}

// BlockGetter.getFurthestCorner chooses a perpendicular corner, including its tie ordering.
fn furthest_corner(delta: Vector3<f64>) -> Vector3<f64> {
    let sign = |v: f64| if v >= 0.0 { 1.0 } else { -1.0 };
    let (x, y, z) = (sign(delta.x), sign(delta.y), sign(delta.z));
    if delta.x.abs() <= delta.y.abs() && delta.x.abs() <= delta.z.abs() {
        Vector3::new(-x, -z, y)
    } else if delta.y.abs() <= delta.z.abs() {
        Vector3::new(z, -y, -x)
    } else {
        Vector3::new(-y, x, -z)
    }
}
