use std::sync::Arc;

use pumpkin_util::math::position::BlockPos;

use super::{
    NetherPortal, PortalSearchResult,
    nether::{SEARCH_RADIUS_NETHER, SEARCH_RADIUS_OVERWORLD},
    poi,
    residency::{PortalChunkResidency, ResidentPortal, run_portal_job},
};
use crate::world::World;

impl NetherPortal {
    pub async fn search_for_portal(
        world: &Arc<World>,
        target_pos: BlockPos,
    ) -> Option<PortalSearchResult> {
        Self::search_for_portal_resident(world, target_pos)
            .await
            .map(|found| found.portal)
    }

    pub(super) async fn search_for_portal_resident(
        world: &Arc<World>,
        target_pos: BlockPos,
    ) -> Option<ResidentPortal> {
        // PortalForcer.findClosestPortalPosition loads POIs before inspecting terrain.
        let poi_world = world.clone();
        let positions = run_portal_job(world, move || {
            let radius = if poi_world.dimension.has_ceiling {
                SEARCH_RADIUS_NETHER
            } else {
                SEARCH_RADIUS_OVERWORLD
            };
            Some(
                poi_world
                    .portal_poi
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .get_in_square(target_pos, radius, Some(poi::POI_TYPE_NETHER_PORTAL)),
            )
        })
        .await?;
        let mut residency = PortalChunkResidency::new(world.level.clone());
        // NetherPortalBlock.getExitPortal inspects at most 21 blocks on either axis.
        for pos in &positions {
            residency.add_area(*pos, Self::MAX_WIDTH as i32);
        }
        residency.load().await?;
        let scan_world = world.clone();
        run_portal_job(world, move || {
            Self::scan_portal_positions(&scan_world, target_pos, positions)
                .map(|portal| ResidentPortal { portal, residency })
        })
        .await
    }
}
