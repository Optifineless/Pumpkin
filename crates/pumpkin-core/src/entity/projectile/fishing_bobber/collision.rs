use super::{FishingBobberEntity, HookState};
use crate::{
    entity::{
        EntityBase,
        projectile::{ProjectileHit, calculate_ray_intersection},
    },
    world::World,
};
use pumpkin_data::{entity::EntityType, game_event::GameEvent, tracked_data};
use pumpkin_util::math::vector3::Vector3;
use std::sync::atomic::Ordering::Relaxed;

impl FishingBobberEntity {
    // FishingHook.checkCollision/onHitBlock: the closest ray hit wins, before gravity.
    pub(super) fn check_collision(&self, world: &World, velocity: &mut Vector3<f64>) {
        // BlockGetter.clip/VoxelShape.clip miss stationary and sub-1e-7 squared-length rays.
        if velocity.length_squared() < 1.0e-7 {
            return;
        }
        let start = self.entity.pos.load();
        let search = self
            .entity
            .bounding_box
            .load()
            .stretch(*velocity)
            .expand(1.0, 1.0, 1.0);
        // Safety divergence: FishingHook.checkCollision has no cap; bound our volume query.
        if !self.entity.accept_collision_sweep(search, *velocity) {
            *velocity = Vector3::default();
            return;
        }
        let (blocks, positions) = world.get_block_collisions(search, self);
        let mut closest = 1.0;
        let mut block_hit = None;
        for (index, shape) in blocks.iter().enumerate() {
            if let Some(t) = calculate_ray_intersection(&start, velocity, shape)
                && t < closest
            {
                closest = t;
                block_hit = positions
                    .iter()
                    .find(|(end, _)| index < *end)
                    .map(|(_, pos)| *pos);
            }
        }
        let mut hooked = None;
        let owner = self.projectile_owner();
        // ProjectileUtil.computeMargin grows the hit margin over the first eight ticks.
        let margin = f64::from(((self.tick_count.load(Relaxed) - 2) as f32 / 20.0).clamp(0.0, 0.3));
        for candidate in world.get_all_at_box(&search) {
            let other = candidate.get_entity();
            // FishingHook.canHitEntity adds live items to Projectile.canHitEntity.
            if other.entity_id == self.entity.entity_id
                || !(self
                    .projectile
                    .can_hit_with_owner(&self.entity, &candidate, owner.as_ref())
                    || (other.entity_type == &EntityType::ITEM && other.is_alive()))
            {
                continue;
            }
            let hit_box = other.bounding_box.load().expand(margin, margin, margin);
            if let Some(t) = calculate_ray_intersection(&start, velocity, &hit_box)
                // AABB.clipPoint requires a strictly positive distance along the ray.
                && t > 0.0 && t < closest
            {
                closest = t;
                hooked = Some(candidate);
                block_hit = None;
            }
        }
        let hit_pos = start + *velocity * closest;
        if (hooked.is_some() || block_hit.is_some())
            && !self.fire_hit_event(
                world,
                hit_pos,
                hooked.as_ref().map(|entity| entity.get_entity().entity_id),
            )
        {
            return;
        }
        if let Some(hooked) = hooked {
            let hit = ProjectileHit::Entity {
                entity: hooked.clone(),
                hit_pos,
                normal: velocity.normalize() * -1.0,
            };
            // FishingHook.checkCollision calls Projectile.hitTargetOrDeflectSelf before hooking.
            if super::super::deflection::hit_target_or_deflect_self(self, &hit) {
                *velocity = self.entity.velocity.load();
                return;
            }
            self.set_hooked_entity(Some(hooked.get_entity().entity_id));
            world.emit_game_event(GameEvent::ProjectileLand.name(), hit_pos);
        } else if let Some(pos) = block_hit {
            // Projectile.onHitBlock calls the block callback before FishingHook clips its motion.
            if let Some(server) = world.server.upgrade() {
                let state = world.get_block_state(&pos);
                world.block_registry.on_projectile_hit(
                    state.id.to_block(),
                    &self.entity.world.load_full(),
                    self,
                    &pos,
                    state,
                    &hit_pos,
                    &server,
                );
            }
            world.emit_game_event(GameEvent::ProjectileLand.name(), hit_pos);
            *velocity = *velocity * closest;
        }
    }

    fn fire_hit_event(&self, world: &World, pos: Vector3<f64>, entity_id: Option<i32>) -> bool {
        let mut event = crate::plugin::api::events::entity::projectile_hit::ProjectileHitEvent::new(
            self.entity.entity_id,
            pos,
            entity_id,
        );
        if let Some(server) = world.server.upgrade() {
            server.plugin_manager.fire_blocking(&server, &mut event);
        }
        !event.cancelled
    }

    // FishingHook.setHookedEntity's tracked id encodes absence as zero.
    pub(super) fn set_hooked_entity(&self, id: Option<i32>) {
        self.hooked_entity_id.store(id.unwrap_or(-1), Relaxed);
        self.entity.set_synced_data(
            tracked_data::fishing_bobber::HOOKED_ENTITY,
            id.map_or(0, |id| id + 1),
        );
    }

    pub(super) fn follow_hooked_entity(&self, world: &World) {
        if let Some(hooked) = world.get_entity_by_id(self.hooked_entity_id.load(Relaxed))
            && super::can_interact_with_level(hooked.as_ref())
            && hooked.get_entity().world.load().dimension == world.dimension
        {
            let entity = hooked.get_entity();
            self.entity.set_pos(entity.pos.load().add_raw(
                0.0,
                f64::from(entity.height()) * 0.8,
                0.0,
            ));
        } else {
            self.set_hooked_entity(None);
            self.state.store(HookState::Flying);
        }
    }
}
