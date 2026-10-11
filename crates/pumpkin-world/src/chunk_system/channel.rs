use super::{ChunkLevel, ChunkPos, HashMapType};
use crate::chunk_system::chunk_state::StagedChunkEnum;
use crate::level::Level;
use std::collections::hash_map::Entry;
use std::mem::swap;
use std::sync::atomic::Ordering::SeqCst;
use std::sync::{Arc, Condvar, Mutex};

pub type LevelChange = (
    HashMapType<ChunkPos, (StagedChunkEnum, StagedChunkEnum)>,
    ChunkLevel,
);

pub struct LevelChannel {
    pub value: Mutex<(Option<LevelChange>, Option<Vec<ChunkPos>>)>,
    pub notify: Condvar,
    unload_requests: Mutex<Option<Vec<tokio::sync::oneshot::Sender<()>>>>,
}

impl Default for LevelChannel {
    fn default() -> Self {
        Self::new()
    }
}

impl LevelChannel {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            value: Mutex::new((None, None)),
            notify: Condvar::new(),
            unload_requests: Mutex::new(Some(Vec::new())),
        }
    }
    pub fn set_both(
        &self,
        new_value: (
            HashMapType<ChunkPos, (StagedChunkEnum, StagedChunkEnum)>,
            ChunkLevel,
        ),
        pos: Vec<ChunkPos>,
    ) {
        // debug!("set new level and priority");
        let mut value = self
            .value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        value.1 = Some(pos);
        if let Some(old) = &mut value.0 {
            for (pos, change) in new_value.0 {
                match old.0.entry(pos) {
                    Entry::Occupied(mut entry) => {
                        let tmp = entry.get_mut();
                        debug_assert_eq!(tmp.1, change.0);
                        if tmp.0 == change.1 {
                            entry.remove();
                        } else {
                            tmp.1 = change.1;
                        }
                    }
                    Entry::Vacant(entry) => {
                        debug_assert_ne!(change.0, change.1);
                        entry.insert(change);
                    }
                }
            }
            old.1 = new_value.1;
        } else {
            value.0 = Some(new_value);
        }
        self.notify.notify_one();
    }
    pub fn set_level(
        &self,
        new_value: (
            HashMapType<ChunkPos, (StagedChunkEnum, StagedChunkEnum)>,
            ChunkLevel,
        ),
    ) {
        // debug!("set new level");
        let mut value = self
            .value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(old) = &mut value.0 {
            for (pos, change) in new_value.0 {
                match old.0.entry(pos) {
                    Entry::Occupied(mut entry) => {
                        let tmp = entry.get_mut();
                        debug_assert_eq!(tmp.1, change.0);
                        if tmp.0 == change.1 {
                            entry.remove();
                        } else {
                            tmp.1 = change.1;
                        }
                    }
                    Entry::Vacant(entry) => {
                        debug_assert_ne!(change.0, change.1);
                        entry.insert(change);
                    }
                }
            }
            old.1 = new_value.1;
        } else {
            value.0 = Some(new_value);
        }
        self.notify.notify_one();
    }
    pub fn set_priority(&self, pos: Vec<ChunkPos>) {
        // debug!("set new priority");
        self.value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .1 = Some(pos);
        self.notify.notify_one();
    }
    pub fn get(&self) -> (Option<LevelChange>, Option<Vec<ChunkPos>>) {
        let mut lock = self
            .value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut ret = (None, None);
        swap(&mut ret, &mut *lock);
        ret
    }
    pub fn wait_and_get(&self, level: &Arc<Level>) -> (Option<LevelChange>, Option<Vec<ChunkPos>>) {
        let mut lock = self
            .value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while lock.0.is_none()
            && lock.1.is_none()
            && !level.should_unload.load(SeqCst)
            && !level.should_save.load(SeqCst)
            && !level.shut_down_chunk_system.load(SeqCst)
            && self
                .unload_requests
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_ref()
                .is_none_or(Vec::is_empty)
        {
            let (new_lock, timeout_res) = self
                .notify
                .wait_timeout(lock, std::time::Duration::from_secs(1))
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            lock = new_lock;
            if timeout_res.timed_out() {
                break;
            }
        }
        let mut ret = (None, None);
        swap(&mut ret, &mut *lock);
        ret
    }
    pub fn notify(&self) {
        let val = self
            .value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        drop(val);
        self.notify.notify_one();
    }

    /// Requests an unload pass and acknowledges it after pending ticket changes are consumed.
    pub fn request_unload(&self, level: &Level) -> tokio::sync::oneshot::Receiver<()> {
        let (completed, completion) = tokio::sync::oneshot::channel();
        let mut pending = self
            .unload_requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(requests) = pending.as_mut() else {
            return completion;
        };
        requests.push(completed);
        drop(pending);
        level.should_unload.store(true, SeqCst);
        self.notify();
        completion
    }

    pub(crate) fn take_unload_requests(&self) -> Vec<tokio::sync::oneshot::Sender<()>> {
        self.unload_requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    }

    pub(crate) fn close_unload_requests(&self) {
        // Admission and retirement share this mutex: racing requests either drain or reject.
        self.unload_requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
    }
}
