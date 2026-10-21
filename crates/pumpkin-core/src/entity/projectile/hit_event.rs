//! Plugin admission before a projectile's destructive decorated-pot callback.

use super::ProjectileHit;
use crate::{entity::EntityBase, plugin::api::events::entity::projectile_hit::ProjectileHitEvent};

pub(super) fn cancelled(projectile: &dyn EntityBase, hit: &ProjectileHit) -> bool {
    let target = match hit {
        ProjectileHit::Entity { entity, .. } => Some(entity.get_entity().entity_id),
        ProjectileHit::Block { .. } => None,
    };
    let entity = projectile.get_entity();
    let mut event = ProjectileHitEvent::new(entity.entity_id, hit.hit_pos(), target);
    if let Some(server) = entity.world.load().server.upgrade() {
        server.plugin_manager.fire_blocking(&server, &mut event);
    }
    event.cancelled
}
