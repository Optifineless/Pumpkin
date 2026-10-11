//! ServerChunkCache.tick's main-thread exclusion, extended to scoped Rayon workers.
use std::{cell::Cell, marker::PhantomData, rc::Rc};

use crate::level::Level;

thread_local! {
    static TICK_LEVEL: Cell<usize> = const { Cell::new(0) };
}

/// A thread-local token valid only while the caller owns the world's tick read barrier.
///
/// Never move it to another thread or retain it through an await.
/// All permits acquired in this scope must end before the read barrier is released.
pub struct TickMutationScope {
    previous: usize,
    _thread: PhantomData<Rc<()>>,
}

impl Drop for TickMutationScope {
    fn drop(&mut self) {
        TICK_LEVEL.set(self.previous);
    }
}

impl Level {
    /// Enters synchronous tick admission under the world's tick read barrier.
    /// Rayon callbacks must each enter their own scope while that barrier remains held.
    #[must_use]
    pub fn enter_tick_mutations(&self) -> TickMutationScope {
        let previous = TICK_LEVEL.replace(std::ptr::from_ref(self) as usize);
        TickMutationScope {
            previous,
            _thread: PhantomData,
        }
    }

    /// Reports this thread's read-barrier protection; unrelated worlds remain counted.
    #[must_use]
    pub fn has_tick_mutation_scope(&self) -> bool {
        #[cfg(any(test, feature = "test-hooks"))]
        if self.chunk_lifecycles.benchmark_legacy_admission() {
            return false;
        }
        TICK_LEVEL.get() == std::ptr::from_ref(self) as usize
    }
}
