//! Legacy capture boundary; coherence, detachment and cost certification stay inactive.
use super::BlockEntity;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::sync::{CaptureIdentity, SnapshotScope, assert_no_guards_held};

/// Coverage state is independent of whether serialization returned an NBT compound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureStatus {
    Legacy,
    BoundaryCheckedLegacy,
    Certified,
}

/// Existing NBT, explicitly unable to supply an owned/complete capture stamp.
pub struct LegacyCapture {
    nbt: NbtCompound,
    status: CaptureStatus,
}
impl LegacyCapture {
    #[must_use]
    pub const fn status(&self) -> CaptureStatus {
        self.status
    }

    #[must_use]
    pub fn into_nbt(self) -> NbtCompound {
        self.nbt
    }
}

/// Calls the public legacy serializer with no instrumented guard held.
/// Custom data and pending publication belong to the caller, after this returns.
#[must_use]
pub fn capture_legacy(entity: &dyn BlockEntity) -> LegacyCapture {
    assert_no_guards_held("BlockEntity::capture_legacy");
    // Do not call virtual metadata getters before entering the leaf boundary.
    let identity = CaptureIdentity {
        kind: "BlockEntity::write_internal",
        #[cfg(debug_assertions)]
        instance: std::ptr::from_ref(entity).cast::<()>() as usize,
        #[cfg(not(debug_assertions))]
        instance: 0,
        position: None,
    };
    // ChunkMap.save -> SerializableChunkData.copyOf -> LevelChunk.getBlockEntityNbtForSaving:
    // capture live state before encoding. This foundation supplies detection, not server-thread ownership.
    let _scope = SnapshotScope::legacy(identity);
    let mut nbt = NbtCompound::new();
    entity.write_internal(&mut nbt);
    LegacyCapture {
        nbt,
        status: CaptureStatus::BoundaryCheckedLegacy,
    }
}

#[cfg(test)]
pub(crate) mod tests;
