use super::{ChunkFileManager, ChunkSerializer, LevelFileIO, PathBuf, PublicationPause};

/// Pauses the next real region publication after serialization, holding its writer ownership.
pub struct PublicationBarrier {
    started: tokio::sync::oneshot::Receiver<()>,
    resume: tokio::sync::oneshot::Sender<()>,
}

impl PublicationBarrier {
    /// Waits until the storage worker has serialized the snapshot but has not published it.
    pub async fn wait(&mut self) -> Result<(), tokio::sync::oneshot::error::RecvError> {
        (&mut self.started).await
    }

    /// Releases publication; dropping the barrier also releases the worker.
    pub fn resume(self) {
        let _ = self.resume.send(());
    }
}

impl<S: ChunkSerializer<WriteBackend = PathBuf>> ChunkFileManager<S> {
    /// Installs a one-shot storage barrier for deterministic lifecycle regression tests.
    pub fn pause_next_publication(&self) -> PublicationBarrier {
        let (started_tx, started) = tokio::sync::oneshot::channel();
        let (resume, resume_rx) = tokio::sync::oneshot::channel();
        let pause: PublicationPause = (started_tx, resume_rx);
        *self
            .publication_pause
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(pause);
        PublicationBarrier { started, resume }
    }
}

impl<L, A, P> LevelFileIO<L, A, P>
where
    L: ChunkSerializer<WriteBackend = PathBuf>,
    A: ChunkSerializer<WriteBackend = PathBuf>,
    P: ChunkSerializer<WriteBackend = PathBuf>,
{
    /// Pauses publication by the configured region backend, including its real disk write.
    pub fn pause_next_publication(&self) -> PublicationBarrier {
        match self {
            Self::Linear(io) => io.pause_next_publication(),
            Self::Anvil(io) => io.pause_next_publication(),
            Self::Pump(io) => io.pause_next_publication(),
        }
    }
}
