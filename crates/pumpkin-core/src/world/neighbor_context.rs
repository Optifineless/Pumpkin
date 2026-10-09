use std::{
    cell::RefCell,
    future::Future,
    sync::atomic::{AtomicU64, Ordering},
};

use super::World;

// CollectingNeighborUpdater.runUpdates queues recursive work on vanilla's single server thread.
// Preserve that cascade identity when Wasm callbacks hop to host workers.
static NEXT_CASCADE: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static SYNC_CONTEXT: RefCell<NeighborUpdateContext> = const { RefCell::new(NeighborUpdateContext(Vec::new())) };
    #[cfg(test)]
    static CAPTURES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
tokio::task_local! {
    static ASYNC_CONTEXT: NeighborUpdateContext;
}

/// Carries recursive neighbour submissions through synchronous plugin callbacks and worker hops.
#[derive(Clone, Default)]
pub struct NeighborUpdateContext(Vec<(usize, u64)>);

struct ContextGuard(NeighborUpdateContext);
impl Drop for ContextGuard {
    fn drop(&mut self) {
        SYNC_CONTEXT.with(|context| *context.borrow_mut() = std::mem::take(&mut self.0));
    }
}

impl NeighborUpdateContext {
    /// Captures only the calling cascade; independent work must start with its own context.
    #[must_use]
    pub fn capture() -> Self {
        #[cfg(test)]
        CAPTURES.with(|count| count.set(count.get() + 1));
        let mut context = ASYNC_CONTEXT.try_with(Clone::clone).unwrap_or_default();
        SYNC_CONTEXT.with(|sync| {
            for cascade in &sync.borrow().0 {
                if !context.0.contains(cascade) {
                    context.0.push(*cascade);
                }
            }
        });
        context
    }

    #[cfg(test)]
    pub(super) fn capture_count() -> usize {
        CAPTURES.with(std::cell::Cell::get)
    }

    /// Runs a callback's worker operation as part of its captured neighbour cascade.
    pub fn with<T>(self, operation: impl FnOnce() -> T) -> T {
        let _guard = self.enter();
        operation()
    }

    /// Retains a callback's cascade while its future moves between executor threads.
    pub async fn scope<T>(self, future: impl Future<Output = T>) -> T {
        ASYNC_CONTEXT.scope(self, future).await
    }

    /// Retains callback ownership, reusing a task scope that already carries its cascades.
    pub(crate) async fn scope_current<T>(future: impl Future<Output = T>) -> T {
        // CollectingNeighborUpdater.runUpdates keeps callbacks within the executing cascade.
        let scoped = ASYNC_CONTEXT
            .try_with(|context| {
                SYNC_CONTEXT.with(|sync| {
                    sync.borrow()
                        .0
                        .iter()
                        .all(|owner| context.0.contains(owner))
                })
            })
            .unwrap_or(false);
        if scoped {
            future.await
        } else {
            Self::capture().scope(future).await
        }
    }

    pub(super) fn next_cascade() -> u64 {
        NEXT_CASCADE.fetch_add(1, Ordering::Relaxed)
    }

    pub(super) fn contains(&self, world: &World, cascade: u64) -> bool {
        self.0
            .contains(&(std::ptr::from_ref(world) as usize, cascade))
    }

    pub(super) fn in_cascade<T>(world: &World, cascade: u64, operation: impl FnOnce() -> T) -> T {
        let mut context = Self::capture();
        context
            .0
            .push((std::ptr::from_ref(world) as usize, cascade));
        context.with(operation)
    }

    fn enter(self) -> ContextGuard {
        ContextGuard(SYNC_CONTEXT.with(|context| context.replace(self)))
    }
}
