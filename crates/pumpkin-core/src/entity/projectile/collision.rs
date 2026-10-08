use super::{ProjectileHit, ThrownItemEntity, calculate_ray_intersection, deflection};
use crate::entity::{Entity, EntityBase};
use pumpkin_data::entity::EntityType;
use pumpkin_util::math::vector3::Vector3;
use std::sync::atomic::Ordering;

// ProjectileUtil.computeMargin uses float arithmetic before expanding entity boxes.
pub(super) fn compute_margin(entity: &Entity) -> f64 {
    f64::from(((entity.age.load(Ordering::Relaxed) as f32 - 2.0) / 20.0).clamp(0.0, 0.3))
}

// ProjectileUtil.getHitResult clips blocks first so entities behind the first solid face cannot be hit.
pub(super) fn first_block_hit(
    caller: &dyn EntityBase,
    start: Vector3<f64>,
    movement: Vector3<f64>,
) -> Option<ProjectileHit> {
    let world = caller.get_entity().world.load();
    let hit = crate::world::World::traverse_blocks(start, start + movement, |pos, _| {
        let (t, face) =
            super::clip::clip_block(world.get_block_state(pos), pos, start, start + movement)?;
        Some(ProjectileHit::Block {
            pos: *pos,
            world_border: false,
            face,
            hit_pos: start + movement * t,
            normal: face.to_offset().to_f64(),
        })
    });
    let end = hit
        .as_ref()
        .map_or(start + movement, ProjectileHit::hit_pos);
    // CollisionGetter.clipIncludingBorder clamps the endpoint when the ray leaves the border.
    let border = world
        .worldborder
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if border.contains(start.x, start.z) && !border.contains(end.x, end.z) {
        let location = border.clamp_teleport_position(end);
        let face = super::clip::nearest_direction(end - start);
        Some(ProjectileHit::Block {
            pos: pumpkin_util::math::position::BlockPos::floored(
                location.x, location.y, location.z,
            ),
            world_border: true,
            face,
            hit_pos: location,
            normal: face.to_offset().to_f64(),
        })
    } else {
        hit
    }
}

impl ThrownItemEntity {
    pub(super) fn find_hit(
        &self,
        caller: &dyn EntityBase,
        start: Vector3<f64>,
        movement: Vector3<f64>,
    ) -> Option<ProjectileHit> {
        let entity = self.get_entity();
        let world = entity.world.load();
        let search = entity
            .bounding_box
            .load()
            .shift(start - entity.pos.load())
            .expand_towards(movement.x, movement.y, movement.z)
            .expand_all(1.0);
        let mut hit = first_block_hit(caller, start, movement);
        let movement = hit.as_ref().map_or(movement, |hit| hit.hit_pos() - start);
        let mut closest = 1.0;
        let margin = compute_margin(entity);
        let owner = self.projectile.owner(entity);
        for other in world.get_all_at_box(&search) {
            if self.should_skip_collision(entity, &other, owner.as_ref()) {
                continue;
            }
            let bounds = other.get_entity().bounding_box.load().expand_all(margin);
            if let Some(t) = calculate_ray_intersection(&start, &movement, &bounds)
                && t < closest
            {
                closest = t;
                hit = Some(ProjectileHit::Entity {
                    entity: other,
                    hit_pos: start + movement * t,
                    normal: movement.normalize() * -1.0,
                });
            }
        }
        hit
    }

    // Projectile.hitTargetOrDeflectSelf / onHit, keeping the existing block/plugin callback.
    pub(super) fn hit_target(&self, caller: &dyn EntityBase, hit: ProjectileHit) {
        let entity = self.get_entity();
        if entity.entity_type == &EntityType::LLAMA_SPIT {
            if deflection::hit_target_or_deflect_self(caller, &hit) {
                return;
            }
            // LlamaSpit.onHitEntity keeps flying; onHitBlock alone discards.
            on_hit_block(
                caller,
                match &hit {
                    ProjectileHit::Block { pos, hit_pos, .. } => Some((*pos, *hit_pos)),
                    ProjectileHit::Entity { .. } => None,
                },
            );
            on_hit(caller, hit);
            return;
        }
        let rocket = entity.entity_type == &EntityType::FIREWORK_ROCKET;
        if !rocket {
            entity.set_pos(hit.hit_pos());
        }
        if deflection::hit_target_or_deflect_self(caller, &hit)
            || self.has_hit.swap(true, Ordering::SeqCst)
        {
            return;
        }
        let block_hit = match &hit {
            ProjectileHit::Block { pos, hit_pos, .. } => Some((*pos, *hit_pos)),
            ProjectileHit::Entity { .. } => None,
        };
        if !rocket {
            on_hit_block(caller, block_hit);
        }
        let landing = land_position(&hit);
        let dragon = entity.entity_type == &EntityType::DRAGON_FIREBALL;
        // DragonFireball.onHit calls Projectile.onHit before creating the cloud.
        if dragon {
            emit_land(caller, landing);
        }
        caller.on_hit(hit);
        if rocket {
            on_hit_block(caller, block_hit);
        }
        if !dragon {
            emit_land(caller, landing);
        }
        if entity.entity_type == &EntityType::FIREWORK_ROCKET || dragon {
            self.has_hit.store(false, Ordering::Relaxed);
        } else {
            entity.remove();
        }
    }
}

fn on_hit_block(
    caller: &dyn EntityBase,
    hit: Option<(pumpkin_util::math::position::BlockPos, Vector3<f64>)>,
) {
    if let Some((pos, hit_pos)) = hit {
        let world = caller.get_entity().world.load();
        let state = world.get_block_state(&pos);
        if let Some(server) = world.server.upgrade() {
            world.block_registry.on_projectile_hit(
                state.id.to_block(),
                &world,
                caller,
                &pos,
                state,
                &hit_pos,
                &server,
            );
        }
    }
}

// Projectile.onHit emits PROJECTILE_LAND after the projectile's own hit effects.
pub(super) fn on_hit(caller: &dyn EntityBase, hit: ProjectileHit) {
    let landing = land_position(&hit);
    caller.on_hit(hit);
    emit_land(caller, landing);
}

fn land_position(hit: &ProjectileHit) -> Vector3<f64> {
    match hit {
        ProjectileHit::Block { pos, .. } => pos.to_centered_f64(),
        ProjectileHit::Entity { hit_pos, .. } => *hit_pos,
    }
}

fn emit_land(caller: &dyn EntityBase, landing: Vector3<f64>) {
    caller.get_entity().world.load().emit_game_event(
        pumpkin_data::game_event::GameEvent::ProjectileLand.name(),
        landing,
    );
}
