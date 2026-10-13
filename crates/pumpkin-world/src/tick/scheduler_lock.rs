use std::{
    ops::{Deref, DerefMut},
    sync::{Mutex, MutexGuard},
};

#[cfg(debug_assertions)]
thread_local! {
    static HELD_LEVELS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

// Local instrumentation until the fork-wide lock checker (#149) reaches this branch.
// Scheduler state is container level 30; its nested save observation is leaf level 40.
pub(super) struct SchedulerMutex<T, const LEVEL: u32>(Mutex<T>);

impl<T, const LEVEL: u32> SchedulerMutex<T, LEVEL> {
    pub(super) const fn new(value: T) -> Self {
        Self(Mutex::new(value))
    }

    pub(super) fn lock(&self) -> SchedulerGuard<'_, T, LEVEL> {
        #[cfg(debug_assertions)]
        HELD_LEVELS.with(|held| {
            // Strictly increasing levels also reject scheduler recursion before it blocks.
            assert!(
                held.get() < (1u64 << LEVEL),
                "scheduler lock order inversion"
            );
            held.set(held.get() | (1u64 << LEVEL));
        });
        SchedulerGuard(
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }
}

pub(super) struct SchedulerGuard<'a, T, const LEVEL: u32>(MutexGuard<'a, T>);

impl<T, const LEVEL: u32> Deref for SchedulerGuard<'_, T, LEVEL> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T, const LEVEL: u32> DerefMut for SchedulerGuard<'_, T, LEVEL> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T, const LEVEL: u32> Drop for SchedulerGuard<'_, T, LEVEL> {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        HELD_LEVELS.with(|held| held.set(held.get() & !(1u64 << LEVEL)));
    }
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "scheduler lock order inversion")]
    fn rejects_inversion_before_blocking() {
        let leaf = SchedulerMutex::<_, 40>::new(());
        let container = SchedulerMutex::<_, 30>::new(());
        let _leaf = leaf.lock();
        let _container = container.lock();
    }
}
