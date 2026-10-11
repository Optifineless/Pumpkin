use super::LockClass;
use std::{
    any::TypeId,
    cell::RefCell,
    ops::{Deref, DerefMut},
    panic::Location,
    sync::{LockResult, PoisonError, TryLockError, TryLockResult},
};

thread_local! {
    static HELD: RefCell<Vec<Acquisition>> = const { RefCell::new(Vec::new()) };
}

pub(super) struct Acquisition {
    identity: usize,
    class: TypeId,
    name: &'static str,
    level: u8,
    location: &'static Location<'static>,
}

impl Acquisition {
    #[track_caller]
    pub(super) fn check<C: LockClass + 'static>(identity: usize) -> Self {
        let new = Self {
            identity,
            class: TypeId::of::<C>(),
            name: C::NAME,
            level: C::LEVEL,
            location: Location::caller(),
        };
        HELD.with_borrow(|held| {
            for old in held {
                let recursion = old.identity == identity;
                let ordered = C::ORDERED_INSTANCES
                    && old.class == new.class
                    && old.identity < identity;
                assert!(!recursion && (new.level > old.level || ordered),
                        "{}: held {} (level {}, instance {:#x}) at {}; acquiring {} (level {}, instance {:#x}) at {}",
                        if recursion { "recursive lock acquisition" } else { "lock order violation" },
                        old.name, old.level, old.identity, old.location,
                        new.name, new.level, identity, new.location,
                    );
            }
        });
        new
    }

    pub(super) fn record(self) -> Held {
        let identity = self.identity;
        HELD.with_borrow_mut(|held| held.push(self));
        Held(identity)
    }
}

pub(super) struct Held(usize);
impl Drop for Held {
    fn drop(&mut self) {
        // Guards need not be released in reverse acquisition order.
        HELD.with_borrow_mut(|held| held.retain(|entry| entry.identity != self.0));
    }
}

#[track_caller]
#[expect(
    clippy::panic,
    reason = "Debug callback boundaries must reject held guards"
)]
pub(super) fn assert_empty(boundary: &'static str) {
    let location = Location::caller();
    HELD.with_borrow(|held| {
        if !held.is_empty() {
            let locations: Vec<_> = held
                .iter()
                .map(|entry| {
                    format!(
                        "{} (instance {:#x}) at {}",
                        entry.name, entry.identity, entry.location
                    )
                })
                .collect();
            panic!(
                "guard crossed boundary {boundary} at {}: held {}",
                location,
                locations.join(", ")
            );
        }
    });
}

pub(super) fn map_lock<G, H>(result: LockResult<G>, map: impl FnOnce(G) -> H) -> LockResult<H> {
    match result {
        Ok(guard) => Ok(map(guard)),
        Err(error) => Err(PoisonError::new(map(error.into_inner()))),
    }
}

pub(super) fn map_try_lock<G, H>(
    result: TryLockResult<G>,
    map: impl FnOnce(G) -> H,
) -> TryLockResult<H> {
    match result {
        Ok(guard) => Ok(map(guard)),
        Err(TryLockError::Poisoned(error)) => Err(TryLockError::Poisoned(PoisonError::new(map(
            error.into_inner(),
        )))),
        Err(TryLockError::WouldBlock) => Err(TryLockError::WouldBlock),
    }
}

macro_rules! guard {
    ($name:ident, $std:ident) => {
        #[must_use = "dropping a guard immediately releases the lock"]
        pub struct $name<'a, T: ?Sized> {
            // Unlock std first, then remove the entry from TLS.
            pub(super) inner: std::sync::$std<'a, T>,
            pub(super) _held: Held,
        }
        impl<T: ?Sized> Deref for $name<'_, T> {
            type Target = T;
            fn deref(&self) -> &T {
                &self.inner
            }
        }
        impl<T: ?Sized + std::fmt::Debug> std::fmt::Debug for $name<'_, T> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.inner.fmt(f)
            }
        }
    };
}
guard!(MutexGuard, MutexGuard);
guard!(RwLockReadGuard, RwLockReadGuard);
guard!(RwLockWriteGuard, RwLockWriteGuard);

macro_rules! mutable_guard {
    ($guard:ident) => {
        impl<T: ?Sized> DerefMut for $guard<'_, T> {
            fn deref_mut(&mut self) -> &mut T {
                &mut self.inner
            }
        }
    };
}
mutable_guard!(MutexGuard);
mutable_guard!(RwLockWriteGuard);
