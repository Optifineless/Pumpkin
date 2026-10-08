use super::{
    ProjectileHit,
    arrow::{ArrowEntity, can_hit_player},
};
use crate::entity::{Entity, EntityBase};
use pumpkin_util::math::vector3::Vector3;
use std::{sync::Arc, sync::atomic::Ordering};

impl ArrowEntity {
    // AbstractArrow.stepMoveAndHit / hitTargetsOrDeflectSelf: every hit shares the first impact position.
    pub(super) fn step_move_and_hit(
        &self,
        caller: &dyn EntityBase,
        start_pos: Vector3<f64>,
        new_pos: Vector3<f64>,
        movement: Vector3<f64>,
    ) {
        let entity = &self.entity;
        let block_hit = super::collision::first_block_hit(caller, start_pos, movement);
        let end = block_hit.as_ref().map_or(new_pos, ProjectileHit::hit_pos);
        while entity.is_alive() {
            let from = entity.pos.load();
            let hits = self.find_hit_entities(from, end);
            entity.set_pos(hits.first().map_or(end, ProjectileHit::hit_pos));
            super::block_effects::apply(caller, from, entity.pos.load());
            if !entity.is_alive() {
                return;
            }
            if hits.is_empty() {
                if let Some(hit) = block_hit
                    && !super::deflection::hit_target_or_deflect_self(caller, &hit)
                {
                    super::collision::on_hit(caller, hit);
                    entity.velocity_dirty.store(true, Ordering::Relaxed);
                }
                return;
            }
            if self.is_no_physics() {
                return;
            }
            for hit in hits {
                if super::deflection::hit_target_or_deflect_self(caller, &hit) {
                    return;
                }
                super::collision::on_hit(caller, hit);
                entity.velocity_dirty.store(true, Ordering::Relaxed);
                if !entity.is_alive() {
                    return;
                }
            }
            if self.pierce_level.load(Ordering::Relaxed) == 0 || self.is_no_physics() {
                return;
            }
        }
    }

    // ProjectileUtil.getManyEntityHitResult checks the actual box before its margin, then visibility to the surface.
    pub(super) fn find_hit_entities(
        &self,
        from: Vector3<f64>,
        to: Vector3<f64>,
    ) -> Vec<ProjectileHit> {
        let search = self
            .entity
            .bounding_box
            .load()
            .expand_towards(
                self.entity.velocity.load().x,
                self.entity.velocity.load().y,
                self.entity.velocity.load().z,
            )
            .expand_all(1.0);
        let world = self.entity.world.load();
        let owner = self.projectile_owner();
        let mut hits: Vec<_> = world
            .get_all_at_box(&search)
            .into_iter()
            .filter(|target| {
                !self.should_skip_collision(&self.entity, target, owner.as_ref())
                    && can_hit_player(&self.entity, owner.as_deref(), target)
            })
            .filter_map(|target| {
                let location = many_entity_hit(&self.entity, &target, from, to)?;
                Some((
                    from.squared_distance_to_vec(&target.get_entity().pos.load()),
                    target,
                    location,
                ))
            })
            .collect();
        hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        hits.into_iter()
            .map(|(_, entity, hit_pos)| ProjectileHit::Entity {
                entity,
                hit_pos,
                normal: (to - from).normalize() * -1.0,
            })
            .collect()
    }
}

fn many_entity_hit(
    source: &Entity,
    target: &Arc<dyn EntityBase>,
    from: Vector3<f64>,
    to: Vector3<f64>,
) -> Option<Vector3<f64>> {
    let bounds = target.get_entity().bounding_box.load();
    let movement = to - from;
    if let Some((t, _)) = super::clip::clip_box(from, movement, bounds) {
        return Some(from + movement * t);
    }
    let margin = super::collision::compute_margin(source);
    if margin <= 0.0 {
        return None;
    }
    let (t, _) = super::clip::clip_box(from, movement, bounds.expand_all(margin))?;
    let outside = from + movement * t;
    let center = (bounds.min + bounds.max) * 0.5;
    let world = source.world.load();
    let clipped = crate::world::World::traverse_blocks(outside, center, |pos, _| {
        let (t, _) = super::clip::clip_block(world.get_block_state(pos), pos, outside, center)?;
        Some(outside + (center - outside) * t)
    })
    .unwrap_or(center);
    super::clip::clip_box(outside, clipped - outside, bounds).map(|_| outside)
}
