//! Synchronous lock ordering and callback boundaries, checked before blocking.
//!
//! | Level | Owner |
//! |---|---|
//! | 10 | Tick/session admission (reserved; Tokio `tick_gate` is excluded) |
//! | 20 | Registry/index, including the player's current-menu selection |
//! | 30 | Entity/container state: screen, weather, scoreboard, border, advancements |
//! | 40 | Leaf storage, including the menu cursor |
//!
//! Equal levels are forbidden unless the *same class* opts into increasing instance
//! address order. An address is stable while borrowed by a guard; order shared locks
//! by their Arc allocation addresses before acquiring them. Recursive reads are also
//! forbidden. Guards must never cross await: this stack describes an OS thread,
//! not an async task. Debug guards retain std's non-Send property.
//!
//! Release wrappers are repr(transparent) around std locks plus `PhantomData`; their
//! guards are the std guards themselves. No metadata, TLS or release hook remains.
//! Checked Debug shows lock metadata without invoking a value formatter under an
//! untracked std guard. Formatting an explicit guard retains its tracking.

use std::marker::PhantomData;
use std::sync::{LockResult, TryLockResult};

mod snapshot_scope;
#[cfg(test)]
mod snapshot_scope_tests;
#[cfg(test)]
mod tests;
pub use snapshot_scope::{
    CaptureIdentity, OwnLock, SnapshotBoundary, SnapshotCall, SnapshotScope, assert_not_in_snapshot,
};
#[cfg(debug_assertions)]
mod tracking;

/// Names an ownership class and its position in the synchronous lock hierarchy.
pub trait LockClass: 'static {
    const NAME: &'static str;
    const LEVEL: u8;
    const ORDERED_INSTANCES: bool = false;
}

macro_rules! classes {
    ($($class:ident, $name:literal, $level:literal, $ordered:literal;)+) => {
        $(pub enum $class {}
        impl LockClass for $class {
            const NAME: &'static str = $name;
            const LEVEL: u8 = $level;
            const ORDERED_INSTANCES: bool = $ordered;
        })+
    };
}

classes! {
    ScreenSelection, "screen selection", 20, false;
    Screen, "screen handler", 30, true;
    Cursor, "screen cursor", 40, false;
    Weather, "world weather", 30, false;
    Scoreboard, "world scoreboard", 30, false;
    WorldBorder, "world border", 30, false;
    Advancements, "player advancements", 30, false;
}

/// Rejects arbitrary code execution while an instrumented synchronous guard is held.
#[inline]
#[cfg_attr(debug_assertions, track_caller)]
pub fn assert_no_guards_held(boundary: &'static str) {
    #[cfg(debug_assertions)]
    tracking::assert_empty(boundary);
    #[cfg(not(debug_assertions))]
    let _ = boundary;
}

#[cfg(not(debug_assertions))]
pub type MutexGuard<'a, T> = std::sync::MutexGuard<'a, T>;
#[cfg(not(debug_assertions))]
pub type RwLockReadGuard<'a, T> = std::sync::RwLockReadGuard<'a, T>;
#[cfg(not(debug_assertions))]
pub type RwLockWriteGuard<'a, T> = std::sync::RwLockWriteGuard<'a, T>;
#[cfg(debug_assertions)]
pub use tracking::{MutexGuard, RwLockReadGuard, RwLockWriteGuard};

/// A std mutex with a compile-time lock class.
#[repr(transparent)]
pub struct Mutex<T: ?Sized, C: LockClass> {
    class: PhantomData<C>,
    inner: std::sync::Mutex<T>,
}

/// A std reader/writer lock with a compile-time lock class.
#[repr(transparent)]
pub struct RwLock<T: ?Sized, C: LockClass> {
    class: PhantomData<C>,
    inner: std::sync::RwLock<T>,
}

macro_rules! common {
    ($lock:ident) => {
        impl<T, C: LockClass> $lock<T, C> {
            pub const fn new(value: T) -> Self {
                Self {
                    class: PhantomData,
                    inner: std::sync::$lock::new(value),
                }
            }
            pub fn into_inner(self) -> LockResult<T> {
                self.inner.into_inner()
            }
        }
        impl<T: Default, C: LockClass> Default for $lock<T, C> {
            fn default() -> Self {
                Self::new(T::default())
            }
        }
        impl<T: ?Sized, C: LockClass> $lock<T, C> {
            pub fn get_mut(&mut self) -> LockResult<&mut T> {
                self.inner.get_mut()
            }
            pub fn is_poisoned(&self) -> bool {
                self.inner.is_poisoned()
            }
            pub fn clear_poison(&self) {
                self.inner.clear_poison();
            }
        }
        impl<T: ?Sized + std::fmt::Debug, C: LockClass> std::fmt::Debug for $lock<T, C> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                #[cfg(debug_assertions)]
                {
                    // std's lock formatter takes its own untracked guard before calling T::fmt.
                    f.debug_struct(stringify!($lock))
                        .field("class", &C::NAME)
                        .field("poisoned", &self.is_poisoned())
                        .finish_non_exhaustive()
                }
                #[cfg(not(debug_assertions))]
                self.inner.fmt(f)
            }
        }
    };
}
common!(Mutex);
common!(RwLock);

macro_rules! acquisition {
    ($method:ident, $result:ident, $guard:ident, $map:ident) => {
        #[inline]
        #[cfg_attr(debug_assertions, track_caller)]
        pub fn $method(&self) -> $result<$guard<'_, T>> {
            #[cfg(debug_assertions)]
            {
                let acquisition = tracking::Acquisition::check::<C>(
                    std::ptr::from_ref(self).cast::<()>() as usize,
                );
                tracking::$map(self.inner.$method(), |inner| $guard {
                    inner,
                    _held: acquisition.record(),
                })
            }
            #[cfg(not(debug_assertions))]
            self.inner.$method()
        }
    };
}

impl<T: ?Sized, C: LockClass> Mutex<T, C> {
    acquisition!(lock, LockResult, MutexGuard, map_lock);
    acquisition!(try_lock, TryLockResult, MutexGuard, map_try_lock);
}
impl<T: ?Sized, C: LockClass> RwLock<T, C> {
    acquisition!(read, LockResult, RwLockReadGuard, map_lock);
    acquisition!(try_read, TryLockResult, RwLockReadGuard, map_try_lock);
    acquisition!(write, LockResult, RwLockWriteGuard, map_lock);
    acquisition!(try_write, TryLockResult, RwLockWriteGuard, map_try_lock);
}
