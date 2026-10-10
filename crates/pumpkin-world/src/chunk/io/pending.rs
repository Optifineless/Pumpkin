use super::{
    Arc, BTreeMap, ChunkFileManager, ChunkSerializer, ChunkWritingError, Dirtiable, Path, PathBuf,
    Vector2,
};

type RegionPending<P> = BTreeMap<(i32, i32), (u64, Arc<P>)>;

pub(super) struct Pending<P> {
    sequence: u64,
    pub regions: BTreeMap<PathBuf, RegionPending<P>>,
}

impl<P> Default for Pending<P> {
    fn default() -> Self {
        Self {
            sequence: 0,
            regions: BTreeMap::new(),
        }
    }
}

impl<P: Dirtiable> Pending<P> {
    pub fn insert(&mut self, path: PathBuf, pos: Vector2<i32>, chunk: Arc<P>) {
        if chunk.is_dirty() {
            self.sequence += 1;
            self.regions
                .entry(path)
                .or_default()
                .insert((pos.x, pos.y), (self.sequence, chunk));
        }
    }
}

impl<S: ChunkSerializer<WriteBackend = PathBuf>> ChunkFileManager<S> {
    pub(super) fn pending_chunk(&self, path: &Path, pos: Vector2<i32>) -> Option<Arc<S::Data>> {
        // IOWorker.loadAsync returns pending data before consulting region storage.
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .regions
            .get(path)?
            .get(&(pos.x, pos.y))
            .map(|(_, chunk)| chunk.copy_for_load())
    }

    pub(super) async fn flush_region(&self, path: &PathBuf) -> Result<(), ChunkWritingError> {
        let serializer = self.get_serializer(path).await.map_err(|error| {
            ChunkWritingError::IoError(std::io::Error::other(error.to_string()))
        })?;
        let mut writer = serializer.write_owned().await;
        // Take the snapshot after the region lock: delayed workers cannot publish older work.
        let chunks = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .regions
            .get(path)
            .cloned()
            .unwrap_or_default();
        let mut serialized = Vec::new();
        let mut failure = None;
        for (pos, (sequence, chunk)) in chunks {
            let version = chunk.dirty_version();
            match writer
                .serializer
                .update_chunk(chunk.clone(), &self.chunk_config)
                .await
            {
                Ok(()) => {
                    writer.modified = true;
                    serialized.push((pos, sequence, version, chunk));
                }
                Err(error) => failure = Some(error),
            }
        }
        #[cfg(test)]
        let hook = self.before_publish.lock().unwrap().take();
        #[cfg(test)]
        if let Some(hook) = hook {
            hook();
        }
        let publication_path = path.clone();
        #[cfg(any(test, feature = "test-hooks"))]
        let pause = self
            .publication_pause
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        // A queued filesystem operation cannot be cancelled. Keep its region lock until
        // publication finishes even if the reader/unload caller is cancelled (IOWorker.close).
        let (_writer, publication) = tokio::spawn(async move {
            #[cfg(any(test, feature = "test-hooks"))]
            if let Some((started, resume)) = pause {
                let _ = started.send(());
                let _ = resume.await;
            }
            let result = writer.flush(&publication_path).await;
            (writer, result)
        })
        .await
        .map_err(|error| ChunkWritingError::IoError(std::io::Error::other(error)))?;
        publication.map_err(ChunkWritingError::IoError)?;

        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(region) = pending.regions.get_mut(path) {
            for (pos, sequence, version, chunk) in serialized {
                if region
                    .get(&pos)
                    .is_some_and(|(current, _)| *current == sequence)
                {
                    let Some(version) = version else {
                        failure = Some(ChunkWritingError::IoError(std::io::Error::other(
                            "Cannot acknowledge unversioned dirty data",
                        )));
                        continue;
                    };
                    chunk.clear_published(version);
                    if !chunk.is_dirty() {
                        region.remove(&pos);
                    }
                }
            }
            if region.is_empty() {
                pending.regions.remove(path);
            }
        }
        failure.map_or(Ok(()), Err)
    }
}
