use std::sync::Arc;

use crate::{
    entity::{Entity, EntityBase},
    plugin::api::events::entity::projectile_launch::ProjectileLaunchEvent,
    world::World,
};

/// Projectile.spawnProjectile admission, with the plugin veto before level insertion.
pub fn spawn_projectile(
    world: &Arc<World>,
    projectile: Arc<dyn EntityBase>,
    shooter: Option<&Entity>,
) -> bool {
    let mut event = ProjectileLaunchEvent::new(
        projectile.get_entity().entity_id,
        shooter.map(|entity| entity.entity_id),
    );
    if let Some(server) = world.server.upgrade() {
        server.plugin_manager.fire_blocking(&server, &mut event);
    }
    !event.cancelled && world.spawn_entity(projectile)
}
