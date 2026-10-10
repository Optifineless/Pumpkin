use std::sync::Arc;

use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use pumpkin_world::{chunk_system::residency::ChunkResidency, level::Level};

/// Keeps loaded chunks available until the portal's synchronous work finishes.
pub(super) struct PortalChunkResidency {
    level: Arc<Level>,
    tickets: ChunkResidency,
}

pub(super) struct ResidentPortal {
    pub(super) portal: super::PortalSearchResult,
    pub(super) _residency: PortalChunkResidency,
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

    pub(super) fn add_creation_area(&mut self, origin: BlockPos) {
        // PortalForcer.createPortal scans 16 blocks; canHostFrame reads widths -1..3.
        let radius = super::nether::CREATE_RADIUS;
        let start = super::nether::FRAME_WIDTH_START;
        let end = super::nether::FRAME_WIDTH_END - 1;
        let min_x = (origin.0.x - radius + start) >> 4;
        let max_x = (origin.0.x + radius + end) >> 4;
        let min_z = (origin.0.z - radius + start) >> 4;
        let max_z = (origin.0.z + radius + end) >> 4;
        for x in min_x..=max_x {
            for z in min_z..=max_z {
                self.add(Vector2::new(x, z));
            }
        }
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
