use std::sync::Arc;

use pumpkin_data::block_properties::HorizontalAxis;
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};

use super::{
    NetherPortal, PortalSearchResult, PortalType, SourcePortalInfo, TeleportTransition,
    nether::CREATE_RADIUS,
    residency::{PortalChunkResidency, ResidentPortal, run_portal_job},
};
use crate::{
    entity::EntityBase,
    world::{World, guard_effects::GuardEffects},
};

impl PortalType {
    pub(super) async fn nether_destination(
        current_level: &World,
        dest_world: Arc<World>,
        caller: &dyn EntityBase,
        source_portal: Option<&SourcePortalInfo>,
    ) -> Option<TeleportTransition> {
        // NetherPortalBlock.getPortalDestination scales and clamps before getExitPortal.
        let pos = caller.get_entity().pos.load();
        let yaw = caller.get_entity().yaw.load();
        let dimensions = caller.get_entity().entity_dimension.load();
        let scale =
            current_level.dimension.coordinate_scale / dest_world.dimension.coordinate_scale;
        let (x, z) = {
            let border = dest_world
                .worldborder
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            border.clamp_block(
                (pos.x * scale).floor() as i32,
                (pos.z * scale).floor() as i32,
            )
        };
        let approximate_exit = BlockPos::new(x, pos.y.floor() as i32, z);
        let source_axis = source_portal.map(|source| source.axis);
        let axis = source_axis.unwrap_or(HorizontalAxis::X);
        // Vanilla's NetherPortalBlock.getExitPortal serializes lookup then create on the
        // server thread. This async coordination mutex owns that sequence, not game data.
        // Move it into a started build so cancellation cannot admit a competing creator.
        let creation = dest_world.portal_creation.clone().lock_owned().await;
        let found = if let Some(found) =
            NetherPortal::search_for_portal_resident(&dest_world, approximate_exit).await
        {
            drop(creation);
            found
        } else {
            Self::create_nether_portal(&dest_world, approximate_exit, axis, creation).await?
        };
        let relative = source_portal.map_or_else(
            || Vector3::new(0.5, 0.0, 0.0),
            |source| {
                PortalSearchResult {
                    lower_corner: source.lower_corner,
                    axis: source.axis,
                    width: source.width,
                    height: source.height,
                }
                .entity_pos_in_portal(pos, &dimensions)
            },
        );
        let target = found.portal.calculate_exit_position(relative, &dimensions);
        let mut residency = found.residency;
        residency.add_area(
            BlockPos::new(
                target.x.floor() as i32,
                target.y.floor() as i32,
                target.z.floor() as i32,
            ),
            (f64::from(dimensions.width) / 2.0 + 1.0).ceil() as i32,
        );
        residency.load().await?;
        let job_world = dest_world.clone();
        run_portal_job(&dest_world, move || {
            let _residency = residency;
            let position = found
                .portal
                .find_open_position(&job_world, target, &dimensions);
            Some(TeleportTransition {
                new_world: job_world,
                position,
                yaw: Some(found.portal.calculate_teleport_yaw(yaw, source_axis)),
                pitch: None,
            })
        })
        .await
    }

    async fn create_nether_portal(
        world: &Arc<World>,
        origin: BlockPos,
        axis: HorizontalAxis,
        creation: tokio::sync::OwnedMutexGuard<()>,
    ) -> Option<ResidentPortal> {
        let mut residency = PortalChunkResidency::new(world.level.clone());
        // PortalForcer.createPortal scans 16 blocks; canHostFrame reads two farther.
        residency.add_area(origin, CREATE_RADIUS + 2);
        residency.load().await?;
        let build_world = world.clone();
        run_portal_job(world, move || {
            let effects = GuardEffects::new();
            let (build_pos, axis, fallback) =
                NetherPortal::find_safe_location(&build_world, origin, axis)?;
            NetherPortal::build_portal_frame(&build_world, build_pos, axis, fallback);
            // Publish all portal POIs before reentrant physics callbacks can search again.
            drop(creation);
            drop(effects);
            Some(ResidentPortal {
                portal: PortalSearchResult {
                    lower_corner: build_pos,
                    axis,
                    width: 2,
                    height: 3,
                },
                residency,
            })
        })
        .await
    }
}
