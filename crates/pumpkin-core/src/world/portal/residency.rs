use std::sync::Arc;

use pumpkin_util::math::{position::BlockPos, vector2::Vector2, vector3::Vector3};
use pumpkin_world::{chunk_system::residency::ChunkResidency, level::Level};

pub(super) async fn run_portal_job<T: Send + 'static>(
    world: &Arc<super::World>,
    job: impl FnOnce() -> Option<T> + Send + 'static,
) -> Option<T> {
    let server = world.server.upgrade()?;
    // MinecraftServer.stopServer waits for started world work before saving. Cancelling
    // its connection await must not detach a blocking portal writer from server drainage.
    match server.spawn_task(tokio::task::spawn_blocking(job)).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) | Err(error) => {
            tracing::error!("Portal blocking task failed: {error}");
            None
        }
    }
}

/// Keeps loaded chunks available until the portal's synchronous work finishes.
pub(super) struct PortalChunkResidency {
    level: Arc<Level>,
    tickets: ChunkResidency,
}

pub(super) struct ResidentPortal {
    pub(super) portal: super::PortalSearchResult,
    pub(super) residency: PortalChunkResidency,
}

impl PortalChunkResidency {
    pub(super) fn new(level: Arc<Level>) -> Self {
        Self {
            tickets: ChunkResidency::new(level.chunk_loading.clone()),
            level,
        }
    }

    pub(super) fn add(&mut self, chunk: Vector2<i32>) {
        self.tickets.add(chunk);
    }

    pub(super) fn add_all(&mut self, chunks: impl IntoIterator<Item = Vector2<i32>>) {
        self.tickets.add_all(chunks);
    }

    pub(super) fn add_creation_area(&mut self, origin: BlockPos) {
        // PortalForcer.createPortal scans 16 blocks; canHostFrame reads widths -1..3.
        let radius = super::nether::CREATE_RADIUS;
        let start = super::nether::FRAME_WIDTH_START;
        let end = super::nether::FRAME_WIDTH_END - 1;
        let min_x = (origin.0.x - radius + start) >> 4;
        let max_x = (origin.0.x + radius + end) >> 4;
        let min_z = (origin.0.z - radius + start) >> 4;
        let max_z = (origin.0.z + radius + end) >> 4;
        self.add_all(
            (min_x..=max_x).flat_map(|x| (min_z..=max_z).map(move |z| Vector2::new(x, z))),
        );
    }

    pub(super) fn add_exit_area(&mut self, center: Vector3<f64>, width: f32) {
        // NetherPortalBlock.findCollisionFreePosition checks one block around the entity.
        let radius = f64::from(width) / 2.0 + 1.0;
        let min_x = (center.x - radius).floor() as i32 >> 4;
        let max_x = (center.x + radius).floor() as i32 >> 4;
        let min_z = (center.z - radius).floor() as i32 >> 4;
        let max_z = (center.z + radius).floor() as i32 >> 4;
        self.add_all(
            (min_x..=max_x).flat_map(|x| (min_z..=max_z).map(move |z| Vector2::new(x, z))),
        );
    }

    pub(super) async fn load(&self) -> Option<()> {
        for chunk in self.tickets.chunks() {
            self.level.get_or_fetch_chunk(*chunk, |_| ()).await.ok()?;
        }
        Some(())
    }
}

#[cfg(test)]
pub type PortalScanGate = (
    tokio::sync::oneshot::Sender<()>,
    tokio::sync::oneshot::Receiver<()>,
);

#[cfg(test)]
pub type PortalBlockingGate = (
    tokio::sync::oneshot::Sender<()>,
    std::sync::mpsc::Receiver<()>,
);

#[cfg(test)]
impl super::World {
    pub(crate) fn pause_portal_blocking_for_test(&self) {
        let gate = self.portal_blocking_gate.lock().unwrap().take();
        if let Some((ready, resume)) = gate {
            let _ = ready.send(());
            let _ = resume.recv();
        }
    }

    pub(crate) async fn pause_portal_scan_for_test(&self) {
        let gate = self.portal_scan_gate.lock().unwrap().take();
        if let Some((ready, resume)) = gate {
            let _ = ready.send(());
            let _ = resume.await;
        }
    }
}
