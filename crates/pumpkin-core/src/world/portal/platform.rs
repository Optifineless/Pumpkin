use std::sync::Arc;

use pumpkin_data::Block;
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use pumpkin_world::world::BlockFlags;

use super::{
    PortalType,
    residency::{PortalChunkResidency, run_portal_job},
};
use crate::world::World;

impl PortalType {
    pub(super) async fn build_end_platform(world: &Arc<World>, origin: BlockPos) -> Option<()> {
        let admission = world.admit_transfer()?;
        let mut residency = PortalChunkResidency::new(world.level.clone());
        // EndPlatformFeature.createEndPlatform reaches both sides of the Z chunk boundary.
        residency.add_all(
            ((origin.0.x - 2) >> 4..=(origin.0.x + 2) >> 4).flat_map(|x| {
                ((origin.0.z - 2) >> 4..=(origin.0.z + 2) >> 4).map(move |z| Vector2::new(x, z))
            }),
        );
        residency.load().await?;
        let world = world.clone();
        let job_world = world.clone();
        run_portal_job(&job_world, move || {
            let _admission = admission;
            let _residency = residency;
            if world.level.cancel_token.is_cancelled() {
                return None;
            }
            for dz in -2..=2 {
                for dx in -2..=2 {
                    for dy in -1..3 {
                        let block = if dy == -1 {
                            Block::OBSIDIAN
                        } else {
                            Block::AIR
                        };
                        world.set_block_state(
                            &BlockPos::new(origin.0.x + dx, origin.0.y + dy, origin.0.z + dz),
                            block.default_state.id,
                            BlockFlags::NOTIFY_ALL,
                        );
                    }
                }
            }
            Some(())
        })
        .await
    }
}
