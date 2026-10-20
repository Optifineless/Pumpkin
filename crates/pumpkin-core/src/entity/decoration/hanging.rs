use std::sync::{Arc, LazyLock, atomic::AtomicU32, atomic::Ordering};

use crate::{
    block::blocks::redstone::is_diode,
    entity::{Entity, EntityBase},
    plugin::api::events::hanging::{HangingBreakByEntityEvent, HangingBreakEvent},
    world::World,
};
use pumpkin_data::BlockDirection;
use pumpkin_data::painting_variant::PaintingVariant;
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos, vector3::Vector3};

use super::{item_frame::ItemFrameEntity, painting::PaintingEntity};

const CHECK_INTERVAL: u32 = 100;
pub(super) const WALL_OFFSET: f64 = 0.46875;
pub(super) const DEPTH: f64 = 0.0625;
// Generic teleports can leave a decoration's cached box at its entity dimensions.
// Expand by the largest possible hanging extent before rejecting a candidate.
static MAX_HANGING_EXTENT: LazyLock<f64> = LazyLock::new(|| {
    f64::from(PaintingVariant::all().iter().fold(2, |size, variant| {
        size.max(variant.width()).max(variant.height())
    })) / 2.0
});

/// BlockAttachedEntity.tick uses a postincrement, checking on the 101st fresh tick.
pub(super) fn at_check_interval(ticks: &AtomicU32) -> bool {
    if ticks.fetch_add(1, Ordering::Relaxed) >= CHECK_INTERVAL {
        ticks.store(0, Ordering::Relaxed);
        true
    } else {
        false
    }
}

pub(super) const fn box_of_size(center: Vector3<f64>, size: Vector3<f64>) -> BoundingBox {
    BoundingBox::new(
        Vector3::new(
            center.x - size.x / 2.0,
            center.y - size.y / 2.0,
            center.z - size.z / 2.0,
        ),
        Vector3::new(
            center.x + size.x / 2.0,
            center.y + size.y / 2.0,
            center.z + size.z / 2.0,
        ),
    )
}

// HangingEntity.calculateSupportBox; Pumpkin paintings already store the wall-shifted center.
pub(super) fn support_box(bounds: BoundingBox, facing: BlockDirection) -> BoundingBox {
    bounds
        .shift(facing.to_offset().to_f64() * -0.5)
        .contract_all(1.0e-7)
}

pub(super) fn is_supported(world: &World, bounds: BoundingBox) -> bool {
    BlockPos::iterate(bounds.min_block_pos(), bounds.max_block_pos()).all(|pos| {
        let (block, state) = world.get_block_and_state(&pos);
        state.is_solid() || is_diode(block)
    })
}

/// Missing neighboring terrain postpones the check until a loaded snapshot is available.
pub(super) fn terrain_available(world: &World, bounds: BoundingBox) -> bool {
    BlockPos::iterate(bounds.min_block_pos(), bounds.max_block_pos())
        .all(|pos| !world.is_in_build_limit(pos) || world.is_loaded(&pos))
}

// HangingEntity.hasLevelCollision: use the entity collision context, including non-solid shapes.
pub(super) fn has_level_collision(entity: &dyn EntityBase, bounds: BoundingBox) -> bool {
    let world = entity.get_entity().world.load_full();
    let scan = bounds.expand_all(1.0);
    for pos in BlockPos::iterate(scan.min_block_pos(), scan.max_block_pos()) {
        let mut collision = false;
        world.for_each_collision_shape(
            world.get_block_state(&pos),
            &pos,
            Some(entity),
            &mut |shape| {
                collision |= shape.at_pos(pos).intersects(&bounds);
            },
        );
        if collision {
            return true;
        }
    }
    has_border_collision(entity.get_entity(), bounds)
}

// CollisionGetter.noBorderCollision and WorldBorder.isInsideCloseToBorder.
fn has_border_collision(entity: &Entity, bounds: BoundingBox) -> bool {
    let world = entity.world.load_full();
    let (min_x, max_x, min_z, max_z) = {
        let border = world
            .worldborder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (min_x, min_z, max_x, max_z) = border.bounds(0.0);
        (min_x, max_x, min_z, max_z)
    };
    let position = entity.pos.load();
    let margin = (bounds.max.x - bounds.min.x)
        .max(bounds.max.z - bounds.min.z)
        .max(1.0);
    let distance = (position.x - min_x)
        .min(max_x - position.x)
        .min(position.z - min_z)
        .min(max_z - position.z);
    distance < margin * 2.0
        && position.x >= min_x - margin
        && position.x < max_x + margin
        && position.z >= min_z - margin
        && position.z < max_z + margin
        && (bounds.min.x < min_x.floor()
            || bounds.max.x > max_x.ceil()
            || bounds.min.z < min_z.floor()
            || bounds.max.z > max_z.ceil())
}

// HangingEntity.canCoexist: frames permit crossing frame types; same-facing decorations never overlap.
pub(super) fn can_coexist(
    entity: &Entity,
    bounds: BoundingBox,
    facing: BlockDirection,
    allow_same_type: bool,
) -> bool {
    let world = entity.world.load_full();
    let entities = world.entities.load_full();
    !entities.iter().any(|other| {
        let base = other.get_entity();
        if std::ptr::eq(base, entity)
            || base.is_removed()
            || !base
                .bounding_box
                .load()
                .expand_all(*MAX_HANGING_EXTENT)
                .intersects(&bounds)
        {
            return false;
        }
        let geometry = other
            .cast_any()
            .downcast_ref::<PaintingEntity>()
            .map(|painting| (painting.hanging_bounding_box(), painting.facing()))
            .or_else(|| {
                other
                    .cast_any()
                    .downcast_ref::<ItemFrameEntity>()
                    .map(|frame| (frame.hanging_bounding_box(), frame.get_facing()))
            });
        geometry.is_some_and(|(other_bounds, other_facing)| {
            ((!allow_same_type && base.entity_type == entity.entity_type) || other_facing == facing)
                && other_bounds.intersects(&bounds)
        })
    })
}

/// Dispatches hanging callbacks before the removal claim, with no retained registry guard.
pub(super) fn break_allowed(entity: &Entity, caused_by: Option<&dyn EntityBase>) -> bool {
    if entity.is_removed() {
        return false;
    }
    let world = entity.world.load_full();
    let Some(hanging) = world.get_entity_by_uuid(entity.entity_uuid) else {
        return true;
    };
    if !std::ptr::eq(hanging.get_entity(), entity) {
        return false;
    }
    let remover = caused_by.and_then(|cause| {
        let id = cause.get_entity().entity_uuid;
        world
            .get_entity_by_uuid(id)
            .or_else(|| {
                world
                    .get_player_by_uuid(id)
                    .map(|player| player as Arc<dyn EntityBase>)
            })
            .filter(|current| std::ptr::eq(current.get_entity(), cause.get_entity()))
    });
    if let Some(server) = world.server.upgrade() {
        let mut event = HangingBreakEvent::new(hanging.clone(), remover.clone());
        server.plugin_manager.fire_blocking(&server, &mut event);
        if event.cancelled || !is_current(entity, &world) {
            return false;
        }
        if let Some(remover) = remover {
            let mut event = HangingBreakByEntityEvent::new(hanging, remover);
            server.plugin_manager.fire_blocking(&server, &mut event);
            if event.cancelled {
                return false;
            }
        }
    }
    is_current(entity, &world)
}

fn is_current(entity: &Entity, world: &Arc<World>) -> bool {
    !entity.is_removed()
        && Arc::ptr_eq(world, &entity.world.load_full())
        && world
            .get_entity_by_uuid(entity.entity_uuid)
            .is_some_and(|current| std::ptr::eq(current.get_entity(), entity))
}
