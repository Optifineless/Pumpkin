use std::sync::Arc;

use super::Entity;
use crate::world::{
    World,
    portal::{PortalType, SourcePortalInfo},
};

impl Entity {
    /// Queues destination loading and teleporting without waiting in the world tick.
    pub(super) fn teleport_through_portal(
        &self,
        portal_type: PortalType,
        destination_world: Arc<World>,
        source_portal: Option<SourcePortalInfo>,
    ) {
        let world = self.world.load_full();
        let Some(server) = world.server.upgrade() else {
            return;
        };
        let entity_id = self.entity_id;
        let yaw = self.yaw.load();

        // Entity.handlePortal / teleport: resolve the exit before moving the entity and passengers.
        // Chunk decoding uses the tick's Rayon pool, so chunk waits must yield on Tokio.
        server.spawn_task(async move {
            let Some(entity) = world.get_entity_by_id(entity_id) else {
                return;
            };
            let transition = portal_type
                .get_portal_destination(
                    &world,
                    destination_world,
                    entity.as_ref(),
                    source_portal.as_ref(),
                )
                .await;
            if let Some(transition) = transition {
                let destination = transition.new_world;
                entity.teleport(
                    transition.position,
                    transition.yaw,
                    transition.pitch,
                    destination.clone(),
                );
                Self::teleport_passengers_recursive(
                    entity.get_entity(),
                    transition.position,
                    transition.yaw.map(|new_yaw| new_yaw - yaw),
                    &destination,
                );
            }
        });
    }
}

#[cfg(test)]
mod tests;
