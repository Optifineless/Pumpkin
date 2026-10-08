use std::sync::atomic::{AtomicU64, Ordering};

/// Tracks changes during serialization so publication cannot clear a later mutation.
#[derive(Default)]
pub struct DirtyFlag(AtomicU64);

impl DirtyFlag {
    #[must_use]
    pub const fn new(dirty: bool) -> Self {
        Self(AtomicU64::new(dirty as u64))
    }

    pub fn load(&self, ordering: Ordering) -> bool {
        self.0.load(ordering) & 1 != 0
    }

    pub fn store(&self, dirty: bool, ordering: Ordering) {
        if dirty {
            let _ = self.0.try_update(ordering, Ordering::Relaxed, |value| {
                Some(value.wrapping_add(2) | 1)
            });
        } else {
            self.0.fetch_and(!1, ordering);
        }
    }

    pub fn version(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }

    pub fn clear_published(&self, version: u64) {
        let _ = self
            .0
            .compare_exchange(version, version & !1, Ordering::AcqRel, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_never_clears_a_mutation_during_serialization() {
        let dirty = DirtyFlag::new(true);
        let snapshot = dirty.version();
        dirty.store(true, Ordering::Relaxed);
        dirty.clear_published(snapshot);
        assert!(dirty.load(Ordering::Relaxed));
        let retry = dirty.version();
        dirty.clear_published(retry);
        assert!(!dirty.load(Ordering::Relaxed));
    }
}
