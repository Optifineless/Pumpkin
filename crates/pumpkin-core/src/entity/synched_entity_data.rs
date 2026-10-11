use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use bytes::BufMut;
use pumpkin_data::meta_data_type::MetaDataType;
use pumpkin_data::tracked_data::{TrackedData, TrackedId};
use pumpkin_protocol::java::client::play::{Metadata, MetadataSerializer};
use pumpkin_protocol::ser::WritingError;
use pumpkin_util::version::JavaMinecraftVersion;

pub trait ErasedSerializer: Send + Sync {
    fn write(
        &self,
        index: TrackedId,
        r#type: MetaDataType,
        writer: &mut dyn std::io::Write,
        version: &JavaMinecraftVersion,
    ) -> Result<(), WritingError>;

    fn write_canonical(&self, index: TrackedId, r#type: MetaDataType) -> Vec<u8>;
}

struct SerializerHolder<T> {
    value: T,
}

impl<T: MetadataSerializer + Clone + Send + Sync + 'static> ErasedSerializer
    for SerializerHolder<T>
{
    fn write(
        &self,
        index: TrackedId,
        r#type: MetaDataType,
        writer: &mut dyn std::io::Write,
        version: &JavaMinecraftVersion,
    ) -> Result<(), WritingError> {
        let meta = Metadata::new_raw(index, r#type, &self.value);
        meta.write(writer, version)
    }

    fn write_canonical(&self, index: TrackedId, r#type: MetaDataType) -> Vec<u8> {
        let mut buf = Vec::new();
        let meta = Metadata::new_raw(index, r#type, &self.value);
        let _ = meta.write(&mut buf, &JavaMinecraftVersion::V_26_3);
        buf
    }
}

pub struct DataItem {
    pub tracked: TrackedData,
    pub serializer: Arc<dyn ErasedSerializer>,
    pub canonical_bytes: Vec<u8>,
    pub dirty: bool,
    pub is_default: bool,
}

pub struct SynchedEntityData {
    items: Mutex<HashMap<TrackedData, DataItem>>,
    is_dirty: AtomicBool,
    #[cfg(test)]
    pub(super) before_dirty_clear: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

/// Immutable metadata revisions shared by every recipient of one tracker flush.
pub struct DirtyBatch(Vec<(TrackedData, Arc<dyn ErasedSerializer>)>);

impl DirtyBatch {
    /// Serializes the captured values without reading or locking the entity again.
    #[must_use]
    pub fn pack_for_version(&self, version: &JavaMinecraftVersion) -> Option<Box<[u8]>> {
        let mut buf = Vec::new();
        for (tracked, serializer) in &self.0 {
            let before_len = buf.len();
            if serializer
                .write(tracked.id, tracked.r#type, &mut buf, version)
                .is_err()
            {
                buf.truncate(before_len);
            }
        }
        if buf.is_empty() {
            return None;
        }
        buf.put_u8(255);
        Some(buf.into_boxed_slice())
    }
}

impl Default for SynchedEntityData {
    fn default() -> Self {
        Self::new()
    }
}

impl SynchedEntityData {
    /// An empty tracker; values are added by [`Self::define`] and [`Self::set`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            items: Mutex::new(HashMap::new()),
            is_dirty: AtomicBool::new(false),
            #[cfg(test)]
            before_dirty_clear: Mutex::new(None),
        }
    }

    /// Registers a tracked value with the default the client already assumes, so it
    /// is only sent once something actually sets it.
    pub fn define<T: MetadataSerializer + Clone + Send + Sync + 'static>(
        &self,
        tracked: TrackedData,
        value: T,
    ) {
        let holder = SerializerHolder { value };
        let canonical_bytes = holder.write_canonical(tracked.id, tracked.r#type);
        let mut items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        items.insert(
            tracked,
            DataItem {
                tracked,
                serializer: Arc::new(holder),
                canonical_bytes,
                dirty: false,
                is_default: true,
            },
        );
    }

    /// Stores a tracked value and marks it dirty, returning whether it differs from
    /// the value that was already stored.
    pub fn set<T: MetadataSerializer + Clone + Send + Sync + 'static>(
        &self,
        tracked: TrackedData,
        value: T,
    ) -> bool {
        let holder = SerializerHolder { value };
        let new_canonical = holder.write_canonical(tracked.id, tracked.r#type);

        let mut items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(item) = items.get_mut(&tracked) {
            if item.canonical_bytes == new_canonical {
                return false;
            }
            item.canonical_bytes = new_canonical;
            item.serializer = Arc::new(holder);
            item.dirty = true;
            item.is_default = false;
        } else {
            items.insert(
                tracked,
                DataItem {
                    tracked,
                    serializer: Arc::new(holder),
                    canonical_bytes: new_canonical,
                    dirty: true,
                    is_default: false,
                },
            );
        }
        self.is_dirty.store(true, Ordering::Release);
        true
    }

    /// Whether an unsent revision remains.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.is_dirty.load(Ordering::Acquire)
    }

    /// Captures the dirty revisions once, before serialization for any recipient.
    pub fn pack_dirty(&self) -> Option<DirtyBatch> {
        if !self.is_dirty.load(Ordering::Acquire) {
            return None;
        }

        let items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Some(DirtyBatch(
            items
                .values()
                .filter(|item| item.dirty)
                .map(|item| (item.tracked, item.serializer.clone()))
                .collect(),
        ))
    }

    /// Serializes a dirty snapshot without acknowledging it.
    pub fn pack_dirty_for_version(&self, version: &JavaMinecraftVersion) -> Option<Box<[u8]>> {
        self.pack_dirty()?.pack_for_version(version)
    }

    #[cfg(test)]
    pub(crate) fn clear_dirty_for_test(&self) {
        if let Some(batch) = self.pack_dirty() {
            self.clear_dirty(&batch);
        }
    }

    /// Acknowledges only the captured revisions, retaining writes made during the flush.
    pub fn clear_dirty(&self, batch: &DirtyBatch) {
        #[cfg(test)]
        {
            let hook = self.before_dirty_clear.lock().unwrap().take();
            if let Some(hook) = hook {
                hook();
            }
        }
        let mut items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // SynchedEntityData.packDirty clears the values it copies. Arc identity is the revision:
        // a changed value installs a new serializer, while the batch pins the old allocation.
        for (tracked, revision) in &batch.0 {
            if let Some(item) = items.get_mut(tracked)
                && Arc::ptr_eq(&item.serializer, revision)
            {
                item.dirty = false;
            }
        }
        self.is_dirty
            .store(items.values().any(|item| item.dirty), Ordering::Release);
    }

    /// Captures the non-default revisions for spawning or refreshing an entity.
    pub fn get_non_default_values(&self) -> DirtyBatch {
        // SynchedEntityData.getNonDefaultValues copies the values before building a packet.
        let items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        DirtyBatch(
            items
                .values()
                .filter(|item| !item.is_default)
                .map(|item| (item.tracked, item.serializer.clone()))
                .collect(),
        )
    }

    /// Serializes every value that differs from the default the client assumes.
    pub fn get_non_default_values_for_version(
        &self,
        version: &JavaMinecraftVersion,
    ) -> Option<Box<[u8]>> {
        self.get_non_default_values().pack_for_version(version)
    }
}

#[cfg(test)]
mod test {
    use pumpkin_data::tracked_data::player::PLAYER_MODE_CUSTOMISATION;

    use super::*;

    #[test]
    fn captured_revision_stays_immutable_and_cannot_clear_a_later_revision() {
        let data = SynchedEntityData::new();
        data.set(PLAYER_MODE_CUSTOMISATION, 1u8);
        let batch = data.pack_dirty().unwrap();
        let old = batch.pack_for_version(&JavaMinecraftVersion::V_26_3);
        data.set(PLAYER_MODE_CUSTOMISATION, 2u8);
        assert_eq!(batch.pack_for_version(&JavaMinecraftVersion::V_1_20_5), old);
        data.clear_dirty(&batch);
        assert!(data.is_dirty());
        let newer = data.pack_dirty().unwrap();
        assert_ne!(newer.pack_for_version(&JavaMinecraftVersion::V_26_3), old);
        // Even changing back to the captured bytes is a new revision, rather than an ABA match.
        data.set(PLAYER_MODE_CUSTOMISATION, 1u8);
        data.clear_dirty(&batch);
        assert!(data.is_dirty());
    }

    /// Metadata ids no longer vary per version: core serializes the current format for
    /// every client and `pumpkin-java-multiversion` translates. So the senders need no
    /// version check of their own to decide whether a value reaches a client.
    #[test]
    fn a_set_value_is_serialized_for_every_version() {
        let data = SynchedEntityData::new();
        data.define(PLAYER_MODE_CUSTOMISATION, 0u8);
        assert!(data.set(PLAYER_MODE_CUSTOMISATION, 0x7Fu8));

        let current = data.get_non_default_values_for_version(&JavaMinecraftVersion::V_26_3);
        let legacy = data.get_non_default_values_for_version(&JavaMinecraftVersion::V_1_20_5);
        assert!(current.is_some());
        assert_eq!(current, legacy);
        assert_eq!(
            data.pack_dirty()
                .unwrap()
                .pack_for_version(&JavaMinecraftVersion::V_1_20_5),
            current
        );
    }

    /// A value still at the default the client assumes is not sent.
    #[test]
    fn default_values_are_not_serialized() {
        let data = SynchedEntityData::new();
        data.define(PLAYER_MODE_CUSTOMISATION, 0u8);

        assert!(
            data.get_non_default_values_for_version(&JavaMinecraftVersion::V_26_3)
                .is_none()
        );
    }
}
