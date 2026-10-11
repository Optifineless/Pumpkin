use std::sync::atomic::Ordering;

use pumpkin_nbt::compound::NbtCompound;

use super::ChunkEntityData;

impl ChunkEntityData {
    #[must_use]
    pub(crate) fn has_live_snapshot(&self) -> bool {
        self.live.load(Ordering::Acquire)
            || self
                .dormant_records
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_some()
    }

    /// Restores untouched dormant records before merging the current live snapshot.
    /// Pass this chunk's locked data and merge fresh using the returned activation state
    /// before unlocking. Fully activated chunks replace all records.
    #[must_use]
    pub fn prepare_snapshot(&self, data: &mut Vec<NbtCompound>, fresh: &[NbtCompound]) -> bool {
        let live = self.live.load(Ordering::Acquire);
        if !live {
            // PersistentEntitySectionManager.processPendingLoads/storeChunkSections keeps loaded
            // entities separate from storage. A cancelled live snapshot must never become dormant.
            let mut dormant = self
                .dormant_records
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let untouched = dormant.get_or_insert_with(|| data.clone());
            // addEntity rejects a loaded UUID already owned by a live entity.
            for record in fresh {
                if let Some(uuid) = record.get_uuid("UUID") {
                    untouched.retain(|saved| saved.get_uuid("UUID") != Some(uuid));
                }
            }
            data.clone_from(untouched);
        }
        live
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, atomic::AtomicBool};

    #[test]
    fn followup_review_dormant_activation_keeps_only_unclaimed_storage_records() {
        let mut dormant = NbtCompound::new();
        dormant.put_uuid("UUID", uuid::Uuid::from_u128(1));
        let mut claimed = NbtCompound::new();
        claimed.put_uuid("UUID", uuid::Uuid::from_u128(2));
        let chunk = ChunkEntityData {
            x: 0,
            z: 0,
            data: Mutex::new(vec![dormant.clone(), claimed.clone()]),
            dormant_records: Mutex::new(None),
            live: AtomicBool::new(false),
            dirty: super::super::io::DirtyFlag::new(false),
        };
        {
            let mut data = chunk.data.lock().unwrap();
            assert!(!chunk.prepare_snapshot(&mut data, std::slice::from_ref(&claimed)));
            // A serialized snapshot contains both storage and the claimed live root.
            claimed.put_string("CustomName", "live root".into());
            *data = vec![dormant.clone(), claimed];
            assert!(!chunk.prepare_snapshot(&mut data, &[]));
            assert_eq!(*data, vec![dormant.clone()]);
        };
        assert_eq!(chunk.entities_for_activation(), vec![dormant]);
        assert!(chunk.entities_for_activation().is_empty());
    }
}
