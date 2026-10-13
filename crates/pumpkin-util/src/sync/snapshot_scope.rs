//! Debug capture diagnostics, never gameplay ownership or writer exclusion.
#[cfg(debug_assertions)]
use std::{any::TypeId, cell::Cell};
use std::{marker::PhantomData, rc::Rc};

/// Identifies the legacy object being serialized, without granting authority.
#[derive(Clone, Copy, Debug)]
pub struct CaptureIdentity {
    pub kind: &'static str,
    pub instance: usize,
    pub position: Option<[i32; 3]>,
}

/// The central operation a leaf capture must not re-enter.
#[derive(Clone, Copy, Debug)]
pub enum SnapshotBoundary {
    Capture,
    WorldRead,
    Mutation,
    Packet,
    Event,
    Effect,
}

/// An instrumented own lock declared by an explicitly audited adapter.
#[derive(Clone, Copy)]
#[must_use]
pub struct OwnLock {
    #[cfg(debug_assertions)]
    class: TypeId,
    #[cfg(debug_assertions)]
    instance: usize,
}
impl OwnLock {
    /// Describes an instrumented own lock whose address stays stable through capture.
    pub const fn new<C: super::LockClass>(instance: usize) -> Self {
        #[cfg(not(debug_assertions))]
        let _ = instance;
        Self {
            #[cfg(debug_assertions)]
            class: TypeId::of::<C>(),
            #[cfg(debug_assertions)]
            instance,
        }
    }
}

#[cfg(debug_assertions)]
const MAX_OWN_LOCKS: usize = 8;

#[cfg(debug_assertions)]
#[derive(Clone, Copy)]
struct Metadata {
    identity: CaptureIdentity,
    own_locks: Option<[Option<OwnLock>; MAX_OWN_LOCKS]>,
    transitive: Option<&'static str>,
}
#[cfg(debug_assertions)]
thread_local! {
    static ACTIVE: Cell<Option<Metadata>> = const { Cell::new(None) };
}

/// An OS-thread-local, non-Send diagnostic scope that must not cross await.
#[must_use]
pub struct SnapshotScope {
    _thread: PhantomData<Rc<()>>,
}
impl SnapshotScope {
    /// Legacy raw own locks are untracked; no adapter is certified by entry.
    #[inline]
    pub fn legacy(identity: CaptureIdentity) -> Self {
        Self::enter(identity, None)
    }

    /// Checks supplied own locks with normal ordering; debug builds require no tracked guards at entry.
    /// This does not certify adapters or exclude writers.
    pub fn with_own_locks(identity: CaptureIdentity, own_locks: &[OwnLock]) -> Self {
        super::assert_no_guards_held("SnapshotScope::with_own_locks");
        Self::enter(identity, Some(own_locks))
    }

    #[inline]
    fn enter(identity: CaptureIdentity, own_locks: Option<&[OwnLock]>) -> Self {
        assert_not_in_snapshot(SnapshotBoundary::Capture, "SnapshotScope::enter");
        #[cfg(debug_assertions)]
        let own_locks = own_locks.map(|locks| {
            assert!(
                locks.len() <= MAX_OWN_LOCKS,
                "snapshot own-lock descriptor exceeds 8 entries"
            );
            let mut bounded = [None; MAX_OWN_LOCKS];
            for (slot, lock) in bounded.iter_mut().zip(locks) {
                *slot = Some(*lock);
            }
            bounded
        });
        #[cfg(debug_assertions)]
        ACTIVE.set(Some(Metadata {
            identity,
            own_locks,
            transitive: None,
        }));
        #[cfg(not(debug_assertions))]
        let _ = (identity, own_locks);
        Self {
            _thread: PhantomData,
        }
    }
}
impl Drop for SnapshotScope {
    #[inline]
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        ACTIVE.set(None);
    }
}

/// Rejects a mapped boundary before any fast path or callback can run.
#[inline]
#[cfg_attr(debug_assertions, track_caller)]
pub fn assert_not_in_snapshot(kind: SnapshotBoundary, boundary: &'static str) {
    #[cfg(debug_assertions)]
    {
        let active = ACTIVE.get();
        assert!(
            active.is_none(),
            "snapshot crossed {kind:?} boundary {boundary}: capture {:?}, transitive {:?}",
            active.map(|a| a.identity),
            active.and_then(|a| a.transitive)
        );
    }
    #[cfg(not(debug_assertions))]
    let _ = (kind, boundary);
}

#[cfg(debug_assertions)]
pub(super) fn check_lock<C: super::LockClass>(instance: usize) {
    if let Some(active) = ACTIVE.get()
        && let Some(locks) = active.own_locks
    {
        assert!(
            locks
                .iter()
                .flatten()
                .any(|lock| lock.instance == instance && lock.class == TypeId::of::<C>()),
            "snapshot foreign lock {}: capture {:?}, transitive {:?}",
            C::NAME,
            active.identity,
            active.transitive
        );
    }
}

/// Marks a virtual component call for diagnostics and restores it on unwind.
#[must_use]
pub struct SnapshotCall {
    #[cfg(debug_assertions)]
    previous: Option<Metadata>,
    _thread: PhantomData<Rc<()>>,
}
impl SnapshotCall {
    #[inline]
    pub fn enter(call: &'static str) -> Self {
        #[cfg(debug_assertions)]
        let previous = ACTIVE.get();
        #[cfg(debug_assertions)]
        if let Some(mut active) = previous {
            active.transitive = Some(call);
            ACTIVE.set(Some(active));
        }
        #[cfg(not(debug_assertions))]
        let _ = call;
        Self {
            #[cfg(debug_assertions)]
            previous,
            _thread: PhantomData,
        }
    }
}
impl Drop for SnapshotCall {
    #[inline]
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        ACTIVE.set(self.previous);
    }
}
