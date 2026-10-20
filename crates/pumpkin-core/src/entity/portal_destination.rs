use std::sync::Arc;

use super::Entity;
use crate::entity::vehicle::boat::BoatEntity;
use crate::world::{
    World,
    portal::{
        PortalType, SourcePortalInfo,
        residency::{PortalChunkResidency, run_portal_job},
    },
};

impl Entity {
    pub(super) fn prepare_portal_destination(
        world: Arc<World>,
        portal_type: PortalType,
        destination: Arc<World>,
        source_portal: Option<SourcePortalInfo>,
        entity_id: i32,
        yaw: f32,
    ) {
        let Some(server) = world.server.upgrade() else {
            return;
        };
        // Entity.handlePortal: prepare the destination without occupying simulation workers.
        server.spawn_task(async move {
            let Some(entity) = world.get_entity_by_id(entity_id) else {
                return;
            };
            let Some(transition) = portal_type
                .get_portal_destination(
                    &world,
                    destination,
                    entity.as_ref(),
                    source_portal.as_ref(),
                )
                .await
            else {
                return;
            };
            let boat_transfer =
                entity.cast_any().is::<BoatEntity>() && !Arc::ptr_eq(&world, &transition.new_world);
            let residency = if boat_transfer {
                let mut residency = PortalChunkResidency::new(transition.new_world.level.clone());
                let position = pumpkin_util::math::position::BlockPos::new(
                    transition.position.x.floor() as i32,
                    transition.position.y.floor() as i32,
                    transition.position.z.floor() as i32,
                );
                residency.add_area(position, 2);
                if residency.load().await.is_none() {
                    return;
                }
                let Ok(storage) = transition
                    .new_world
                    .level
                    .get_entity_chunk(position.chunk_position())
                    .await
                else {
                    return;
                };
                Some((residency, storage))
            } else {
                None
            };
            let job_world = world.clone();
            run_portal_job(&job_world, move || {
                if let Some((_residency, storage)) = residency {
                    world.transfer_boat_portal(&entity, &transition, &storage);
                    return Some(());
                }
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
                Some(())
            })
            .await;
        });
    }
}
