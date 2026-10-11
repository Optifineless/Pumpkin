//! Owner notifications cannot outlive a lock acquisition without an effect batch.

use std::{
    ops::{Deref, DerefMut},
    sync::{LockResult, PoisonError, TryLockError, TryLockResult},
};

use pumpkin_util::sync::{LockClass, Mutex, MutexGuard};

use super::guard_effects::GuardEffects;

/// Starts an effect batch before locking and delivers notifications after unlocking.
#[repr(transparent)]
pub struct OwnerMutex<T, C: LockClass>(Mutex<T, C>);

/// Releases the state guard before finishing its notification batch.
pub struct OwnerGuard<'a, T, C: LockClass> {
    // Fields drop in declaration order, including during unwinding.
    guard: MutexGuard<'a, T>,
    _effects: GuardEffects,
    _class: std::marker::PhantomData<C>,
}

impl<T, C: LockClass> OwnerMutex<T, C> {
    pub const fn new(value: T) -> Self {
        Self(Mutex::new(value))
    }

    pub fn lock(&self) -> LockResult<OwnerGuard<'_, T, C>> {
        // ServerScoreboard and WorldBorder listeners keep mutation order; only delivery unlocks.
        let effects = GuardEffects::new();
        match self.0.lock() {
            Ok(guard) => Ok(OwnerGuard::new(guard, effects)),
            Err(error) => Err(PoisonError::new(OwnerGuard::new(
                error.into_inner(),
                effects,
            ))),
        }
    }

    pub fn try_lock(&self) -> TryLockResult<OwnerGuard<'_, T, C>> {
        let effects = GuardEffects::new();
        match self.0.try_lock() {
            Ok(guard) => Ok(OwnerGuard::new(guard, effects)),
            Err(TryLockError::Poisoned(error)) => Err(TryLockError::Poisoned(PoisonError::new(
                OwnerGuard::new(error.into_inner(), effects),
            ))),
            Err(TryLockError::WouldBlock) => Err(TryLockError::WouldBlock),
        }
    }
}

impl<'a, T, C: LockClass> OwnerGuard<'a, T, C> {
    const fn new(guard: MutexGuard<'a, T>, effects: GuardEffects) -> Self {
        Self {
            guard,
            _effects: effects,
            _class: std::marker::PhantomData,
        }
    }
}

impl<T, C: LockClass> Deref for OwnerGuard<'_, T, C> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<T, C: LockClass> DerefMut for OwnerGuard<'_, T, C> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.guard
    }
}
