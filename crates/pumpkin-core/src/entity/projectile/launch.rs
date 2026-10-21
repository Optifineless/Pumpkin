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
    launch_allowed(world, projectile.as_ref(), shooter) && world.spawn_entity(projectile)
}

fn launch_allowed(
    world: &Arc<World>,
    projectile: &dyn EntityBase,
    shooter: Option<&Entity>,
) -> bool {
    let mut event = ProjectileLaunchEvent::new(
        projectile.get_entity().entity_id,
        shooter.map(|entity| entity.entity_id),
    );
    if let Some(server) = world.server.upgrade() {
        server.plugin_manager.fire_blocking(&server, &mut event);
    }
    !event.cancelled
}

// Item candidates remain unpublished and unreserved throughout both admission callbacks.
pub fn prepare_projectile(
    world: &Arc<World>,
    projectile: Arc<dyn EntityBase>,
    shooter: Option<&Entity>,
) -> Option<Arc<dyn EntityBase>> {
    if !launch_allowed(world, projectile.as_ref(), shooter) {
        return None;
    }
    let entity = projectile.get_entity();
    let mut spawn = crate::plugin::api::events::entity::entity_spawn::EntitySpawnEvent::new(
        entity.entity_id,
        entity.entity_type.id.to_string(),
        entity.pos.load(),
        world.clone(),
    );
    if let Some(server) = world.server.upgrade() {
        server.plugin_manager.fire_blocking(&server, &mut spawn);
    }
    (!spawn.cancelled).then_some(projectile)
}
