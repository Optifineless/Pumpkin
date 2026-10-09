use super::{LivingEntity, random_teleport_coordinate};
use crate::entity::{EntityBase, ageable::AgeableMob};
use pumpkin_data::Block;
use pumpkin_data::tag::{self, Taggable};
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos, vector3::Vector3};
use rand::RngExt;

const CONSUMABLE_DOES_NOT_TELEPORT_TO: &str = "minecraft:consumable_does_not_teleport_to";
const ENTITY_COLLISION_EPSILON: f64 = 1.0e-7;
const ENTITIES_CAN_TELEPORT_TO: &str = "minecraft:entities_can_teleport_to";

impl LivingEntity {
    // TeleportRandomlyConsumeEffect.apply samples within the dimension's logical height.
    pub(super) fn find_consumable_teleport_target(
        &self,
        diameter: f32,
    ) -> Option<(Vector3<f64>, Vector3<f64>)> {
        let world = self.entity.world.load();
        let min_y = world.get_bottom_y();
        let max_y = min_y + world.dimension.logical_height - 1;
        let mut rng = rand::rng();
        for _ in 0..Self::RANDOM_TELEPORT_ATTEMPTS {
            let center = self.entity.pos.load();
            let x = random_teleport_coordinate(center.x, diameter, rng.random());
            let y = random_teleport_coordinate(center.y, diameter, rng.random())
                .clamp(f64::from(min_y), f64::from(max_y));
            let z = random_teleport_coordinate(center.z, diameter, rng.random());
            // TeleportRandomlyConsumeEffect.apply samples before LivingEntity.stopRiding.
            let vehicle = self
                .entity
                .vehicle
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            if let Some(vehicle) = vehicle {
                vehicle.get_entity().remove_passenger(self.entity.entity_id);
            }
            if self.entity.has_vehicle() {
                return None;
            }
            let departure = self.entity.pos.load();
            let target = world
                .worldborder
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clamp_teleport_position(Vector3::new(x, y, z));
            if let Some(target) = self.check_consumable_teleport_target(&world, target) {
                return Some((departure, target));
            }
        }
        None
    }

    // LivingEntity.randomTeleport / checkPositionAndTeleport retain the sampled fractional Y.
    fn check_consumable_teleport_target(
        &self,
        world: &crate::world::World,
        mut target: Vector3<f64>,
    ) -> Option<Vector3<f64>> {
        let server = world.server.upgrade();
        let has_tag = |block: &Block, tag: &str| {
            server.as_ref().map_or_else(
                || block.is_tagged_with(tag).unwrap_or(false),
                |server| {
                    server
                        .datapack_manager
                        .consume_effect_block_has_tag(block, tag)
                },
            )
        };
        let mut pos = BlockPos::floored_v(target);
        world.get_block_state_if_loaded(&pos)?;
        while pos.0.y > world.get_bottom_y() {
            pos = pos.down();
            let state = world.get_block_state_if_loaded(&pos)?;
            let block = Block::from_state_id(state.id);
            if has_tag(block, ENTITIES_CAN_TELEPORT_TO) {
                if has_tag(block, CONSUMABLE_DOES_NOT_TELEPORT_TO) {
                    return None;
                }
                let bounds = BoundingBox::new_from_pos(
                    target.x,
                    target.y,
                    target.z,
                    &self.entity.entity_dimension.load(),
                );
                // CollisionGetter.noCollision(null, aabb) includes collidable entities.
                let entity_bounds = bounds.expand(
                    ENTITY_COLLISION_EPSILON,
                    ENTITY_COLLISION_EPSILON,
                    ENTITY_COLLISION_EPSILON,
                );
                if !teleport_space_empty(world, bounds)
                    || world.contains_any_liquid(bounds)
                    || world
                        .get_entities_at_box(&entity_bounds)
                        .iter()
                        .any(|entity| blocks_teleport(entity.as_ref()))
                {
                    return None;
                }
                for pos in BlockPos::iterate(bounds.min_block_pos(), bounds.max_block_pos()) {
                    let state = world.get_block_state_if_loaded(&pos)?;
                    if has_tag(
                        Block::from_state_id(state.id),
                        CONSUMABLE_DOES_NOT_TELEPORT_TO,
                    ) {
                        return None;
                    }
                }
                // Enderman.canRandomlyTeleportTo rejects water in the support block.
                if self.entity.entity_type == &pumpkin_data::entity::EntityType::ENDERMAN
                    && world.get_fluid(&pos).has_tag(&tag::Fluid::MINECRAFT_WATER)
                {
                    return None;
                }
                return Some(target);
            }
            target.y -= 1.0;
        }
        None
    }

    // LivingEntity.checkPositionAndTeleport stops PathfinderMob navigation after success.
    pub(super) fn finish_random_teleport(caller: &dyn EntityBase) {
        if let Some(mob) = caller.get_mob() {
            mob.get_mob_entity()
                .navigator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .stop();
        }
    }
}

// EntityGetter.getEntityCollisions(null): vanilla canBeCollidedWith(null) overrides.
// Pumpkin's generic Mob.is_collidable currently includes ordinary mobs, so do not use it here.
fn blocks_teleport(entity: &dyn EntityBase) -> bool {
    if entity.is_spectator() || !entity.get_entity().is_alive() {
        return false;
    }
    let kind = entity.get_entity().entity_type;
    if kind.has_tag(&tag::EntityType::MINECRAFT_BOAT) {
        return true;
    }
    if kind == &pumpkin_data::entity::EntityType::SHULKER {
        return entity
            .get_living_entity()
            .is_some_and(|living| living.health.load() > 0.0);
    }
    if let Some(ghast) = entity
        .cast_any()
        .downcast_ref::<crate::entity::passive::happy_ghast::HappyGhastEntity>()
    {
        return !ghast.is_baby()
            && ghast.mob_entity.living_entity.health.load() > 0.0
            && ghast.is_on_still_timeout();
    }
    false
}

// BlockCollisions expands by one cell plus epsilon, since fences reach into the next cell.
fn teleport_space_empty(world: &crate::world::World, bounds: BoundingBox) -> bool {
    let expanded = bounds.expand(
        ENTITY_COLLISION_EPSILON,
        ENTITY_COLLISION_EPSILON,
        ENTITY_COLLISION_EPSILON,
    );
    let min = expanded.min_block_pos().0 - Vector3::new(1, 1, 1);
    let max = expanded.max_block_pos().0 + Vector3::new(1, 1, 1);
    for pos in BlockPos::iterate(BlockPos(min), BlockPos(max)) {
        let Some(state) = world.get_block_state_if_loaded(&pos) else {
            // BlockCollisions.computeNext skips a missing collision chunk in the outer ring.
            continue;
        };
        if crate::world::World::check_collision(&bounds, pos, state, false, |_| ()) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::Entity;
    use crate::entity::living::test_support::armor_test_world;
    use pumpkin_data::entity::EntityType;
    use pumpkin_util::math::vector2::Vector2;
    use pumpkin_world::chunk::ChunkData;

    #[tokio::test]
    async fn teleport_destination_rejects_a_fence_protruding_from_the_cell_below() {
        let dir = tempfile::tempdir().unwrap();
        let world = armor_test_world(dir.path());
        let chunk = ChunkData::empty_sync(0, 0);
        chunk.set_block_absolute_y(8, 64, 8, Block::OAK_FENCE.default_state.id);
        world
            .level
            .loaded_chunks
            .insert(Vector2::new(0, 0), chunk.clone());
        let living = LivingEntity::new(Entity::new(
            world.clone(),
            Vector3::new(1.0, 70.0, 1.0),
            &EntityType::COW,
        ));
        let target = Vector3::new(8.5, 65.2, 8.5);
        assert!(
            living
                .check_consumable_teleport_target(&world, target)
                .is_none()
        );
        chunk.set_block_absolute_y(8, 64, 8, Block::STONE.default_state.id);
        assert_eq!(
            living.check_consumable_teleport_target(&world, target),
            Some(target)
        );
        chunk.set_block_absolute_y(15, 64, 8, Block::STONE.default_state.id);
        let edge = Vector3::new(15.5, 65.2, 8.5);
        assert_eq!(
            living.check_consumable_teleport_target(&world, edge),
            Some(edge)
        );
        crate::server::fixture_lifecycle::finish().await;
    }
}
