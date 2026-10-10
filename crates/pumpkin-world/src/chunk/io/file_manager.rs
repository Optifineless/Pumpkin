use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use futures::future::join_all;
use pumpkin_util::math::vector2::Vector2;
use tokio::{
    join,
    sync::{OnceCell, RwLock, mpsc},
};
use tracing::{error, trace};

use crate::{
    chunk::{ChunkReadingError, ChunkWritingError, io::Dirtiable},
    level::LevelFolder,
};

use super::{ChunkSerializer, FileIO, LoadedData, run_blocking};

/// Caches serializers while watchers retain them and persists every save.
/// Lock order is `file_locks`, watchers, then the individual cached serializer.
pub struct ChunkFileManager<S: ChunkSerializer<WriteBackend = PathBuf>> {
    file_locks: RwLock<BTreeMap<PathBuf, Arc<ChunkSerializerLazyLoader<S>>>>,
    watchers: RwLock<BTreeMap<PathBuf, usize>>,
    chunk_config: S::ChunkConfig,
    dimension: pumpkin_data::dimension::Dimension,
    pending: Mutex<pending::Pending<S::Data>>,
    #[cfg(test)]
    after_drain_pass: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    #[cfg(test)]
    before_publish: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    #[cfg(any(test, feature = "test-hooks"))]
    publication_pause: Mutex<Option<PublicationPause>>,
}

#[cfg(any(test, feature = "test-hooks"))]
type PublicationPause = (
    tokio::sync::oneshot::Sender<()>,
    tokio::sync::oneshot::Receiver<()>,
);

pub(crate) trait PathFromLevelFolder {
    fn file_path(folder: &LevelFolder, file_name: &str) -> PathBuf;
}

struct ChunkSerializerLazyLoader<S: ChunkSerializer<WriteBackend = PathBuf>> {
    path: PathBuf,
    /// Initialised at most once; subsequent calls reuse the same Arc.
    internal: OnceCell<Arc<RwLock<CachedSerializer<S>>>>,
}

struct CachedSerializer<S> {
    serializer: S,
    modified: bool,
}

impl<S: ChunkSerializer<WriteBackend = PathBuf>> CachedSerializer<S> {
    async fn flush(&mut self, path: &PathBuf) -> Result<(), std::io::Error> {
        if self.modified {
            self.serializer.write(path).await?;
            self.modified = false;
        }
        Ok(())
    }
}

impl<S: ChunkSerializer<WriteBackend = PathBuf> + 'static> ChunkSerializerLazyLoader<S> {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            internal: OnceCell::new(),
        }
    }

    /// Returns `true` only when no outside caller still holds a clone of this
    /// loader *or* the inner serializer.
    ///
    /// # Safety requirement
    /// **Must be called while the write-lock on the parent `file_locks` map is
    /// held.**  That guarantees no new `Arc` clones can be issued while we
    /// inspect the strong counts.
    fn can_remove(loader: &Arc<Self>) -> bool {
        // The map itself holds 1 strong count; anything above that means an
        // active caller still has a handle.
        if Arc::strong_count(loader) > 1 {
            return false;
        }
        loader
            .internal
            .get()
            .is_none_or(|arc| Arc::strong_count(arc) == 1)
    }

    /// Returns the serializer, initialising it from disk on the first call.
    async fn get(&self) -> Result<Arc<RwLock<CachedSerializer<S>>>, ChunkReadingError> {
        self.internal
            .get_or_try_init(|| async {
                let serializer = self.read_from_disk().await?;
                Ok(Arc::new(RwLock::new(CachedSerializer {
                    serializer,
                    modified: false,
                })))
            })
            .await
            .cloned()
    }

    async fn read_from_disk(&self) -> Result<S, ChunkReadingError> {
        trace!("Opening file from disk: {}", self.path.display());

        match tokio::fs::read(&self.path).await {
            Ok(bytes) => {
                if bytes.is_empty() {
                    trace!(
                        "File is empty (0 bytes), using default for: {}",
                        self.path.display()
                    );
                    return Ok(S::default());
                }
                let path = self.path.clone();
                let value = run_blocking(move || S::read_with_path(bytes.into(), &path))
                    .await
                    .map_err(|_| {
                        ChunkReadingError::IoError(std::io::Error::other(
                            "chunk deserialization task failed",
                        ))
                    })??;
                trace!("Successfully read file from disk: {}", self.path.display());
                Ok(value)
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                trace!("File not found, using default for: {}", self.path.display());
                Ok(S::default())
            }
            Err(err) => Err(ChunkReadingError::IoError(err)),
        }
    }
}

impl<S: ChunkSerializer<WriteBackend = PathBuf>> ChunkFileManager<S> {
    /// Constructs an Overworld manager. Other dimensions must use `in_dimension`.
    pub fn new(chunk_config: S::ChunkConfig) -> Self {
        Self::in_dimension(chunk_config, pumpkin_data::dimension::Dimension::OVERWORLD)
    }

    /// Uses the supplied dimension bounds when decoding every terrain chunk.
    pub fn in_dimension(
        chunk_config: S::ChunkConfig,
        dimension: pumpkin_data::dimension::Dimension,
    ) -> Self {
        Self {
            file_locks: RwLock::new(BTreeMap::new()),
            watchers: RwLock::new(BTreeMap::new()),
            chunk_config,
            dimension,
            pending: Mutex::new(pending::Pending::default()),
            #[cfg(test)]
            after_drain_pass: Mutex::new(None),
            #[cfg(test)]
            before_publish: Mutex::new(None),
            #[cfg(any(test, feature = "test-hooks"))]
            publication_pause: Mutex::new(None),
        }
    }
}

impl<S: ChunkSerializer<WriteBackend = PathBuf>> ChunkFileManager<S> {
    /// Returns the serializer for `path`, inserting a lazy-loader if absent.
    ///
    /// Uses an optimistic read-first pattern: in the common case (cache hit)
    /// we never need a write-lock on the map.
    async fn get_serializer(
        &self,
        path: &Path,
    ) -> Result<Arc<RwLock<CachedSerializer<S>>>, ChunkReadingError> {
        {
            let locks = self.file_locks.read().await;
            if let Some(loader) = locks.get(path) {
                // Clone the Arc *before* releasing the lock so it stays alive.
                let loader = loader.clone();
                drop(locks);
                return loader.get().await;
            }
        }

        let loader = {
            let mut locks = self.file_locks.write().await;
            locks
                .entry(path.into())
                .or_insert_with(|| Arc::new(ChunkSerializerLazyLoader::new(path.into())))
                .clone()
            // Write-lock dropped here — `loader.get()` may block on I/O and
            // must not hold the map lock.
        };

        loader.get().await
    }

    async fn invalidate_failed_read(&self, path: &Path) {
        let mut loaders = self.file_locks.write().await;
        if let Some(loader) = loaders.get(path)
            && ChunkSerializerLazyLoader::can_remove(loader)
            && let Some(serializer) = loader.internal.get()
            && !serializer.read().await.modified
            && !self
                .pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .regions
                .contains_key(path)
        {
            // Retry must reopen repaired storage, rather than reuse cached read errors.
            loaders.remove(path);
        }
    }

    /// Attempt to evict the cached serializer for `path`.
    ///
    /// The entry is only removed when *both* conditions hold:
    /// 1. No watcher still references the path.
    /// 2. No other `Arc` clone is live (ensured via `can_remove`).
    async fn maybe_evict(&self, path: &PathBuf) {
        // Snapshot first: filesystem work must not hold either global map lock.
        let loader = self.file_locks.read().await.get(path).cloned();
        if self
            .watchers
            .read()
            .await
            .get(path)
            .is_some_and(|&count| count > 0)
        {
            return;
        }
        let Some(loader) = loader else { return };
        if self.flush_region(path).await.is_err() {
            return;
        }
        drop(loader);
        let mut locks = self.file_locks.write().await;
        let watchers = self.watchers.read().await;
        if watchers.get(path).is_none_or(|&count| count == 0)
            && locks
                .get(path)
                .is_some_and(ChunkSerializerLazyLoader::can_remove)
            && !self
                .pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .regions
                .contains_key(path)
        {
            locks.remove(path);
            trace!("Evicted serializer cache for {}", path.display());
        }
    }
}

impl<P, S> FileIO for ChunkFileManager<S>
where
    P: PathFromLevelFolder + Send + Sync + Sized + Dirtiable + 'static,
    S: ChunkSerializer<Data = P, WriteBackend = PathBuf>,
    S::ChunkConfig: Send + Sync,
{
    type Data = Arc<S::Data>;

    async fn watch_chunks<'a>(&'a self, folder: &'a LevelFolder, chunks: &'a [Vector2<i32>]) {
        let paths: Vec<_> = chunks
            .iter()
            .map(|c| P::file_path(folder, &S::get_chunk_key(c)))
            .collect();

        let mut watchers = self.watchers.write().await;
        for path in paths {
            *watchers.entry(path).or_insert(0) += 1;
        }
    }

    async fn unwatch_chunks<'a>(&'a self, folder: &'a LevelFolder, chunks: &'a [Vector2<i32>]) {
        let paths: Vec<_> = chunks
            .iter()
            .map(|c| P::file_path(folder, &S::get_chunk_key(c)))
            .collect();

        let mut paths_to_evict = Vec::new();
        {
            let mut watchers = self.watchers.write().await;
            for path in paths {
                if let std::collections::btree_map::Entry::Occupied(mut e) = watchers.entry(path) {
                    let count = e.get_mut();
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        let (path, _) = e.remove_entry();
                        paths_to_evict.push(path);
                    }
                }
            }
        }

        for path in paths_to_evict {
            self.maybe_evict(&path).await;
        }
    }

    async fn clear_watched_chunks(&self) {
        self.watchers.write().await.clear();
        let paths: Vec<PathBuf> = self.file_locks.read().await.keys().cloned().collect();
        for path in paths {
            self.maybe_evict(&path).await;
        }
    }

    async fn fetch_chunks<'a>(
        &'a self,
        folder: &'a LevelFolder,
        chunk_coords: &'a [Vector2<i32>],
        stream: mpsc::Sender<LoadedData<Self::Data, ChunkReadingError>>,
    ) {
        // Group requested chunk coords by their region file.
        let mut regions_chunks: BTreeMap<String, Vec<Vector2<i32>>> = BTreeMap::new();
        for at in chunk_coords {
            let path = P::file_path(folder, &S::get_chunk_key(at));
            let pending = self.pending_chunk(&path, *at);
            if let Some(chunk) = pending {
                let _ = stream.send(LoadedData::Loaded(chunk)).await;
                continue;
            }
            regions_chunks
                .entry(S::get_chunk_key(at))
                .or_default()
                .push(*at);
        }

        let region_tasks = regions_chunks.into_iter().map(|(file_name, chunks)| {
            let task_stream = stream.clone();
            async move {
                let path = P::file_path(folder, &file_name);

                let chunk_serializer = match self.get_serializer(&path).await {
                    Ok(s) => s,
                    Err(ChunkReadingError::ChunkNotExist) => {
                        return;
                    }
                    Err(err) => {
                        for pos in chunks {
                            let error =
                                ChunkReadingError::IoError(std::io::Error::other(err.to_string()));
                            let _ = task_stream.send(LoadedData::Error((pos, error))).await;
                        }
                        return;
                    }
                };

                // A bounded channel of 1 keeps backpressure between the
                // serializer and the caller without unbounded buffering.
                let (send, mut recv) = mpsc::channel::<LoadedData<S::Data, ChunkReadingError>>(1);

                let failed_read = std::sync::atomic::AtomicBool::new(false);
                let failure = &failed_read;
                // Forward received chunks, wrapping them in `Arc`.
                // Captured move is intentional — `task_stream` is consumed here.
                let forward = async move {
                    while let Some(data) = recv.recv().await {
                        if matches!(data, LoadedData::Error(_)) {
                            failure.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                        let wrapped = data.map_loaded(Arc::new);
                        if task_stream.send(wrapped).await.is_err() {
                            // Receiver dropped; abort early to avoid wasted work.
                            return;
                        }
                    }
                };

                // Hold the read lock only for the duration of `get_chunks`.
                let read = async move {
                    let serializer = chunk_serializer.read().await;
                    serializer
                        .serializer
                        .get_chunks(chunks, send, self.dimension.clone())
                        .await;
                };

                join!(forward, read);
                if failed_read.load(std::sync::atomic::Ordering::Relaxed) {
                    self.invalidate_failed_read(&path).await;
                }

                // Evict if not watched and references are dropped
                self.maybe_evict(&path).await;
            }
        });

        join_all(region_tasks).await;
    }

    fn queue_chunks(&self, folder: &LevelFolder, chunks: Vec<(Vector2<i32>, Self::Data)>) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (pos, chunk) in chunks {
            let path = P::file_path(folder, &S::get_chunk_key(&pos));
            pending.insert(path, pos, chunk);
        }
    }

    async fn save_chunks<'a>(
        &'a self,
        folder: &'a LevelFolder,
        chunks_data: Vec<(Vector2<i32>, Self::Data)>,
    ) -> Result<(), ChunkWritingError> {
        let positions: Vec<_> = chunks_data.iter().map(|(pos, _)| *pos).collect();
        self.queue_chunks(folder, chunks_data);
        self.flush_chunks(folder, &positions).await
    }

    async fn flush_chunks<'a>(
        &'a self,
        folder: &'a LevelFolder,
        chunks: &'a [Vector2<i32>],
    ) -> Result<(), ChunkWritingError> {
        let paths: std::collections::BTreeSet<_> = chunks
            .iter()
            .map(|pos| P::file_path(folder, &S::get_chunk_key(pos)))
            .collect();
        let mut failure = None;
        for path in paths {
            if let Err(error) = self.flush_region(&path).await {
                failure = Some(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }

    async fn block_and_await_ongoing_tasks(&self) -> Result<(), ChunkWritingError> {
        // IOWorker.synchronize / RegionFileStorage.flush, extended to retained failures.
        // Producers must be fenced by the caller. A pass can still observe a mutation
        // that raced the previous publication; do not report success until empty.
        const MAX_DRAIN_PASSES: usize = 16;
        let mut last_failure = None;
        for _ in 0..MAX_DRAIN_PASSES {
            let mut paths: std::collections::BTreeSet<_> =
                self.file_locks.read().await.keys().cloned().collect();
            paths.extend(
                self.pending
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .regions
                    .keys()
                    .cloned(),
            );
            let mut failure = None;
            for path in paths {
                if let Err(error) = self.flush_region(&path).await {
                    error!("Failed to drain {}: {error}", path.display());
                    failure = Some(error);
                }
            }
            last_failure = failure;
            #[cfg(test)]
            let hook = self.after_drain_pass.lock().unwrap().take();
            #[cfg(test)]
            if let Some(hook) = hook {
                hook();
            }
            if self
                .pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .regions
                .is_empty()
                && last_failure.is_none()
            {
                return Ok(());
            }
        }
        error!("Chunk storage still has pending work after {MAX_DRAIN_PASSES} drain passes");
        Err(last_failure.unwrap_or_else(|| {
            ChunkWritingError::IoError(std::io::Error::other(
                "Chunk drain did not reach an empty state",
            ))
        }))
    }
}

pub enum LevelFileIO<Linear, Anvil, Pump>
where
    Linear: ChunkSerializer<WriteBackend = PathBuf>,
    Anvil: ChunkSerializer<WriteBackend = PathBuf>,
    Pump: ChunkSerializer<WriteBackend = PathBuf>,
{
    Linear(ChunkFileManager<Linear>),
    Anvil(ChunkFileManager<Anvil>),
    Pump(ChunkFileManager<Pump>),
}

impl<P, Linear, Anvil, Pump> FileIO for LevelFileIO<Linear, Anvil, Pump>
where
    P: PathFromLevelFolder + Send + Sync + Sized + Dirtiable + 'static,
    Linear: ChunkSerializer<Data = P, WriteBackend = PathBuf>,
    Anvil: ChunkSerializer<Data = P, WriteBackend = PathBuf>,
    Pump: ChunkSerializer<Data = P, WriteBackend = PathBuf>,
    Linear::ChunkConfig: Send + Sync,
    Anvil::ChunkConfig: Send + Sync,
    Pump::ChunkConfig: Send + Sync,
{
    type Data = Arc<P>;

    async fn fetch_chunks<'a>(
        &'a self,
        folder: &'a LevelFolder,
        chunk_coords: &'a [Vector2<i32>],
        stream: tokio::sync::mpsc::Sender<LoadedData<Self::Data, ChunkReadingError>>,
    ) {
        match self {
            Self::Linear(io) => io.fetch_chunks(folder, chunk_coords, stream).await,
            Self::Anvil(io) => io.fetch_chunks(folder, chunk_coords, stream).await,
            Self::Pump(io) => io.fetch_chunks(folder, chunk_coords, stream).await,
        }
    }

    fn queue_chunks(&self, folder: &LevelFolder, chunks: Vec<(Vector2<i32>, Self::Data)>) {
        match self {
            Self::Linear(io) => io.queue_chunks(folder, chunks),
            Self::Anvil(io) => io.queue_chunks(folder, chunks),
            Self::Pump(io) => io.queue_chunks(folder, chunks),
        }
    }

    async fn save_chunks<'a>(
        &'a self,
        folder: &'a LevelFolder,
        chunks_data: Vec<(Vector2<i32>, Self::Data)>,
    ) -> Result<(), ChunkWritingError> {
        match self {
            Self::Linear(io) => io.save_chunks(folder, chunks_data).await,
            Self::Anvil(io) => io.save_chunks(folder, chunks_data).await,
            Self::Pump(io) => io.save_chunks(folder, chunks_data).await,
        }
    }

    async fn watch_chunks<'a>(&'a self, folder: &'a LevelFolder, chunks: &'a [Vector2<i32>]) {
        match self {
            Self::Linear(io) => io.watch_chunks(folder, chunks).await,
            Self::Anvil(io) => io.watch_chunks(folder, chunks).await,
            Self::Pump(io) => io.watch_chunks(folder, chunks).await,
        }
    }

    async fn flush_chunks<'a>(
        &'a self,
        folder: &'a LevelFolder,
        chunks: &'a [Vector2<i32>],
    ) -> Result<(), ChunkWritingError> {
        match self {
            Self::Linear(io) => io.flush_chunks(folder, chunks).await,
            Self::Anvil(io) => io.flush_chunks(folder, chunks).await,
            Self::Pump(io) => io.flush_chunks(folder, chunks).await,
        }
    }

    async fn unwatch_chunks<'a>(&'a self, folder: &'a LevelFolder, chunks: &'a [Vector2<i32>]) {
        match self {
            Self::Linear(io) => io.unwatch_chunks(folder, chunks).await,
            Self::Anvil(io) => io.unwatch_chunks(folder, chunks).await,
            Self::Pump(io) => io.unwatch_chunks(folder, chunks).await,
        }
    }

    async fn clear_watched_chunks(&self) {
        match self {
            Self::Linear(io) => io.clear_watched_chunks().await,
            Self::Anvil(io) => io.clear_watched_chunks().await,
            Self::Pump(io) => io.clear_watched_chunks().await,
        }
    }

    async fn block_and_await_ongoing_tasks(&self) -> Result<(), ChunkWritingError> {
        match self {
            Self::Linear(io) => io.block_and_await_ongoing_tasks().await,
            Self::Anvil(io) => io.block_and_await_ongoing_tasks().await,
            Self::Pump(io) => io.block_and_await_ongoing_tasks().await,
        }
    }
}

#[cfg(test)]
#[path = "file_manager_tests.rs"]
mod tests;

#[path = "pending.rs"]
mod pending;

#[cfg(any(test, feature = "test-hooks"))]
#[path = "publication_test_hook.rs"]
mod publication_test_hook;
#[cfg(any(test, feature = "test-hooks"))]
pub use publication_test_hook::PublicationBarrier;

#[cfg(test)]
#[path = "serialization_failure_tests.rs"]
mod serialization_failure_tests;

#[cfg(test)]
#[path = "heap_stack_tests.rs"]
mod heap_stack_tests;
