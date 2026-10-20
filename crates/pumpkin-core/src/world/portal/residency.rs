use std::sync::Arc;

use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use pumpkin_world::{chunk_system::residency::ChunkResidency, level::Level};

pub async fn run_portal_job<T: Send + 'static>(
    world: &Arc<super::World>,
    job: impl FnOnce() -> Option<T> + Send + 'static,
) -> Option<T> {
    let server = world.server.upgrade()?;
    // Keep started blocking work in server drainage even if its requesting future is dropped.
    match server.spawn_task(tokio::task::spawn_blocking(job)).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) | Err(error) => {
            tracing::error!("Portal blocking task failed: {error}");
            None
        }
    }
}

pub struct PortalChunkResidency {
    level: Arc<Level>,
    tickets: ChunkResidency,
}

pub(super) struct ResidentPortal {
    pub(super) portal: super::PortalSearchResult,
    pub(super) residency: PortalChunkResidency,
}

impl PortalChunkResidency {
    pub(crate) fn new(level: Arc<Level>) -> Self {
        Self {
            tickets: ChunkResidency::new(level.chunk_loading.clone()),
            level,
        }
    }

    pub(crate) fn add_area(&mut self, center: BlockPos, radius: i32) {
        self.tickets.add_all(
            ((center.0.x - radius) >> 4..=(center.0.x + radius) >> 4).flat_map(|x| {
                ((center.0.z - radius) >> 4..=(center.0.z + radius) >> 4)
                    .map(move |z| Vector2::new(x, z))
            }),
        );
    }

    pub(crate) async fn load(&self) -> Option<()> {
        for pos in self.tickets.chunks() {
            self.level.get_or_fetch_chunk(*pos, |_| ()).await.ok()?;
        }
        Some(())
    }
}
