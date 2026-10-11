use std::sync::Arc;

use pumpkin_data::{
    Block,
    block_properties::{HorizontalAxis, NetherPortalLikeProperties},
};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};

use super::{
    NetherPortal, PortalSearchResult,
    nether::{SEARCH_RADIUS_NETHER, SEARCH_RADIUS_OVERWORLD},
    poi,
    residency::{PortalChunkResidency, ResidentPortal, run_portal_job},
};
use crate::world::World;

impl NetherPortal {
    pub(super) async fn search_for_portal_resident(
        world: &Arc<World>,
        target: BlockPos,
    ) -> Option<ResidentPortal> {
        let _admission = world.admit_transfer()?;
        let positions = Self::portal_candidates(world, target).await?;
        // PortalForcer.findClosestPortalPosition compares POI distance, then POI Y.
        // Load likely candidates incrementally: each frame block is a POI, not a portal.
        for pos in positions {
            let mut residency = PortalChunkResidency::new(world.level.clone());
            residency.add(pos.chunk_position());
            residency.load().await?;
            let (block, state) = world.get_block_and_state(&pos);
            if block != &Block::NETHER_PORTAL {
                continue;
            }
            let axis = NetherPortalLikeProperties::from_state_id(state.id).axis;
            let radius = Self::MAX_WIDTH as i32;
            let (rx, rz) = if axis == HorizontalAxis::X {
                (radius, 1)
            } else {
                (1, radius)
            };
            residency.add_all(((pos.0.x - rx) >> 4..=(pos.0.x + rx) >> 4).flat_map(|x| {
                ((pos.0.z - rz) >> 4..=(pos.0.z + rz) >> 4).map(move |z| Vector2::new(x, z))
            }));
            residency.load().await?;
            #[cfg(test)]
            world.pause_portal_scan_for_test().await;
            let scan_world = world.clone();
            let admission = world.admit_transfer()?;
            let found = run_portal_job(world, move || {
                let _admission = admission;
                #[cfg(test)]
                scan_world.pause_portal_blocking_for_test();
                Self::get_on_axis(&scan_world, &pos, axis)
                    .filter(Self::was_already_valid)
                    .map(|portal| ResidentPortal {
                        portal: PortalSearchResult {
                            lower_corner: portal.lower_corner(),
                            axis: portal.axis(),
                            width: portal.width(),
                            height: portal.height(),
                        },
                        residency,
                    })
            })
            .await;
            if found.is_some() {
                return found;
            }
        }
        None
    }

    async fn portal_candidates(world: &Arc<World>, target: BlockPos) -> Option<Vec<BlockPos>> {
        let world = world.clone();
        let admission = world.admit_transfer()?;
        let job_world = world.clone();
        run_portal_job(&job_world, move || {
            let _admission = admission;
            #[cfg(test)]
            world.pause_portal_blocking_for_test();
            let radius = if world.dimension.has_ceiling {
                SEARCH_RADIUS_NETHER
            } else {
                SEARCH_RADIUS_OVERWORLD
            };
            let mut positions = world
                .portal_poi
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get_in_square(target, radius, Some(poi::POI_TYPE_NETHER_PORTAL));
            let border = world
                .worldborder
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            positions.retain(|pos| {
                pos.0.y >= world.min_y
                    && pos.0.y < world.min_y + world.dimension.height
                    && border.contains_block(pos.0.x, pos.0.z)
            });
            positions.sort_unstable_by_key(|pos| {
                (
                    target.0.squared_distance_to(pos.0.x, pos.0.y, pos.0.z),
                    pos.0.y,
                )
            });
            positions.dedup();
            Some(positions)
        })
        .await
    }
}
