use super::ChunkPos;
use crate::level::SyncChunk;
use crossbeam::channel::{Receiver, Sender};
use std::sync::Arc;
use std::sync::{Mutex, Weak};
use tokio::sync::oneshot;

#[expect(clippy::type_complexity)]
pub struct ChunkListener {
    single: Mutex<Vec<(ChunkPos, oneshot::Sender<Result<SyncChunk, String>>)>>,
    failures: Mutex<std::collections::HashMap<ChunkPos, String>>,
    global: Mutex<Vec<Sender<(ChunkPos, Weak<crate::chunk::ChunkData>)>>>,
}

impl Default for ChunkListener {
    fn default() -> Self {
        Self::new()
    }
}

impl ChunkListener {
    #[must_use]
    pub fn new() -> Self {
        Self {
            single: Mutex::new(Vec::new()),
            failures: Mutex::new(std::collections::HashMap::new()),
            global: Mutex::new(Vec::new()),
        }
    }

    pub fn add_single_chunk_listener(
        &self,
        pos: ChunkPos,
    ) -> oneshot::Receiver<Result<SyncChunk, String>> {
        let (tx, rx) = oneshot::channel();
        let failures = self
            .failures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(error) = failures.get(&pos) {
            let _ = tx.send(Err(error.clone()));
        } else {
            self.single
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push((pos, tx));
        }
        rx
    }

    pub fn process_failed_chunk(&self, pos: ChunkPos, error: &str) {
        let mut failures = self
            .failures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        failures.insert(pos, error.to_owned());
        self.fail_current_listeners(pos, error);
    }

    pub(super) fn fail_current_listeners(&self, pos: ChunkPos, error: &str) {
        let mut single = self
            .single
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut index = 0;
        while index < single.len() {
            if single[index].0 == pos {
                let (_, sender) = single.swap_remove(index);
                let _ = sender.send(Err(error.to_owned()));
            } else {
                index += 1;
            }
        }
    }

    pub fn clear_failure(&self, pos: ChunkPos) {
        self.failures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&pos);
    }

    pub fn add_global_chunk_listener(&self) -> Receiver<(ChunkPos, Weak<crate::chunk::ChunkData>)> {
        let (tx, rx) = crossbeam::channel::unbounded();
        self.global
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(tx);
        rx
    }

    pub fn process_new_chunk(&self, pos: ChunkPos, chunk: &SyncChunk) {
        self.clear_failure(pos);
        {
            let mut single = self
                .single
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut i = 0;
            let mut len = single.len();
            while i < len {
                if single[i].0 == pos {
                    let (_, send) = single.remove(i);
                    let _ = send.send(Ok(chunk.clone()));
                    // log::debug!("single listener {i} send {pos:?}");
                    len -= 1;
                    continue;
                }
                i += 1;
            }
        }
        {
            let weak = Arc::downgrade(chunk);
            let mut global = self
                .global
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut i = 0;
            let mut len = global.len();
            while i < len {
                if matches!(global[i].send((pos, weak.clone())), Ok(())) {
                    // log::debug!("global listener {i} send {pos:?}");
                } else {
                    // log::debug!("one global listener dropped");
                    global.remove(i);
                    len -= 1;
                    continue;
                }
                i += 1;
            }
        }
    }
}
