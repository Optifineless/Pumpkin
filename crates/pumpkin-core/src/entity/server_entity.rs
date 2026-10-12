use std::sync::Mutex;

use super::{
    Entity,
    synched_entity_data::{DirtyBatch, SynchedEntityData},
};
use crate::{net::java::JavaClient, world::World};
use pumpkin_data::entity::EntityType;
use pumpkin_protocol::java::client::play::CSetEntityMetadata;

#[derive(Default)]
pub(super) struct MetadataFlush {
    running: bool,
    dirty_pending: bool,
    refresh_pending: bool,
    batch: DirtyBatch,
    #[cfg(test)]
    pub(super) before_enqueue: Option<Box<dyn FnOnce() + Send>>,
}

struct MetadataFlushOwner<'a> {
    state: &'a Mutex<MetadataFlush>,
    data: &'a SynchedEntityData,
    batch: DirtyBatch,
    released: bool,
}

impl Drop for MetadataFlushOwner<'_> {
    fn drop(&mut self) {
        if !self.released {
            // An unwound sender may have acknowledged its snapshot before enqueueing it. Admit a
            // normal tracker retry of current values, retaining a queued refresh for the next owner.
            self.data.retry_dirty(&self.batch);
            self.batch.clear();
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.batch = std::mem::take(&mut self.batch);
            state.running = false;
        }
    }
}

impl Entity {
    pub(super) fn flush_dirty_entity_data(&self) {
        if self.synched_data.is_dirty() {
            self.flush_synced_entity_data(false);
        }
    }

    pub(super) fn resend_synced_entity_data(&self) {
        self.flush_synced_entity_data(true);
    }

    fn flush_synced_entity_data(&self, mut refresh: bool) {
        let mut owner = {
            let mut state = self
                .metadata_flush
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.running {
                state.refresh_pending |= refresh;
                state.dirty_pending |= !refresh;
                return;
            }
            state.running = true;
            refresh |= std::mem::take(&mut state.refresh_pending);
            state.dirty_pending = false;
            MetadataFlushOwner {
                state: &self.metadata_flush,
                data: &self.synched_data,
                batch: std::mem::take(&mut state.batch),
                released: false,
            }
        };
        loop {
            // ServerEntity.sendDirtyEntityData has one sender. Re-entrant/concurrent requests
            // defer capture until this sender enqueues its batch, without holding a callback lock.
            self.send_synced_entity_data_batch(refresh, &mut owner.batch);
            owner.batch.clear();
            let mut state = self
                .metadata_flush
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.refresh_pending {
                refresh = true;
                state.refresh_pending = false;
            } else if state.dirty_pending {
                refresh = false;
                state.dirty_pending = false;
            } else {
                state.batch = std::mem::take(&mut owner.batch);
                state.running = false;
                owner.released = true;
                return;
            }
        }
    }

    fn send_synced_entity_data_batch(&self, refresh: bool, batch: &mut DirtyBatch) {
        let world = self.world.load();
        // Vanilla packDirty also consumes changes without viewers. Do this under the data lock,
        // before allocating a snapshot or scanning players.
        if !refresh
            && self.entity_type != &EntityType::PLAYER
            && world
                .entity_tracker
                .get_tracked_entity(self.entity_id)
                .is_some_and(|tracked| tracked.seen_by.is_empty())
        {
            self.synched_data.discard_dirty();
            return;
        }
        if refresh {
            self.synched_data.get_non_default_values_into(batch);
        } else {
            if !self.synched_data.pack_dirty_into(batch) {
                return;
            }
            self.synched_data.clear_dirty(batch);
        }
        let players = world.players.load();
        let java_recipients = self.java_metadata_recipients(&world, &players);
        if java_recipients.is_empty() {
            return;
        }
        if refresh {
            self.synched_data.clear_dirty(batch);
        }
        #[cfg(test)]
        {
            let hook = self.metadata_flush.lock().unwrap().before_enqueue.take();
            if let Some(hook) = hook {
                hook();
            }
        }
        let recipients_by_version =
            World::collect_java_recipients_by_version(java_recipients.into_iter());
        for (version, recipients) in recipients_by_version {
            if let Some(buf) = batch.pack_for_version(&version) {
                let packet = CSetEntityMetadata::new(self.entity_id.into(), buf);
                if let Ok(packet_data) = JavaClient::serialize_packet_for_version(&packet, version)
                {
                    for recipient in recipients {
                        recipient.try_enqueue_packet(packet_data.clone());
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "metadata_flush_tests.rs"]
mod tests;
