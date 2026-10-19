//! ItemFrame.removeFramedMap / MapItemSavedData.removedFromFrame ownership.
use super::{MapData, MapManager};
use pumpkin_util::math::position::BlockPos;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct FrameAttachment {
    pub map_id: i32,
    pub pos: BlockPos,
    pub frame: uuid::Uuid,
    // Runtime incarnation only: never compare this to a persisted MapFrame entity_id.
    pub runtime_id: i32,
    pub revision: u64,
}

type Key = (i32, BlockPos);

#[derive(Default)]
pub(super) struct FrameCleanup {
    bindings: HashMap<Key, FrameAttachment>,
    // At most one obligation per saved attachment, even across repeated replacements.
    pending: HashMap<Key, FrameAttachment>,
}

impl MapManager {
    // Records a committed item transaction or retires its retained token.
    // Only owner bookkeeping: no map locks, callbacks, I/O, or notifications.
    pub(crate) fn change_frame_attachment(
        &self,
        old: Option<FrameAttachment>,
        new: Option<FrameAttachment>,
        cleanup: bool,
    ) {
        let mut state = self
            .frame_cleanup
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut was_current = false;
        if let Some(old) = old {
            let key = (old.map_id, old.pos);
            if state.bindings.get(&key) == Some(&old) {
                was_current = true;
                state.bindings.remove(&key);
                if cleanup {
                    state.pending.insert(key, old);
                }
            }
        }
        if let Some(new) = new {
            let key = (new.map_id, new.pos);
            let continuous = was_current
                && old.is_some_and(|old| {
                    old.map_id == new.map_id
                        && old.pos == new.pos
                        && old.frame == new.frame
                        && old.runtime_id == new.runtime_id
                });
            if continuous {
                if let Some(pending) = state.pending.get_mut(&key) {
                    *pending = new;
                }
            } else {
                state.pending.remove(&key);
            }
            state.bindings.insert(key, new);
        }
    }

    pub(super) fn report_pending_frame_cleanup(&self) {
        let unresolved = self
            .frame_cleanup
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pending
            .len();
        if unresolved != 0 {
            tracing::error!(
                "{unresolved} map frame cleanups remain unresolved after map load drain"
            );
            crate::SERVER_EXIT_CODE.store(1, std::sync::atomic::Ordering::Release);
        }
    }

    pub(super) fn retry_frame_cleanup(&self) {
        let ids: std::collections::HashSet<_> = self
            .frame_cleanup
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pending
            .keys()
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.finish_frame_cleanup(id);
        }
    }

    pub(crate) fn finish_frame_cleanup(&self, id: i32) {
        if !self
            .frame_cleanup
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pending
            .keys()
            .any(|(map_id, _)| *map_id == id)
        {
            return;
        }
        let map = self.maps.get(&id).map(|entry| entry.value().clone());
        if let Some(map) = map {
            apply_pending(&self.frame_cleanup, id, &map);
        } else {
            self.request_load(id);
        }
    }
}

pub(super) fn apply_pending(cleanup: &Mutex<FrameCleanup>, id: i32, map: &Arc<Mutex<MapData>>) {
    let mut state = cleanup
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut map = map
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state.pending.retain(|&(map_id, pos), _| {
        if map_id != id {
            return true;
        }
        // A newer installation cancels its predecessor under the same owner.
        map.removed_from_frame(pos);
        false
    });
}

pub(super) fn discard_missing(cleanup: &Mutex<FrameCleanup>, id: i32) {
    cleanup
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pending
        .retain(|(map_id, _), _| *map_id != id);
}
