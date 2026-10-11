use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

thread_local! {
    pub(super) static COUNT_ALLOCATIONS: Cell<bool> = const { Cell::new(false) };
}
pub(super) static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

struct Counter;

#[global_allocator]
static ALLOCATOR: Counter = Counter;

fn count_allocation() {
    if COUNT_ALLOCATIONS.try_with(Cell::get).unwrap_or(false) {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
    }
}

// SAFETY: forwards each unchanged layout/pointer to System, with diagnostic counting.
unsafe impl GlobalAlloc for Counter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count_allocation();
        // SAFETY: the caller supplies GlobalAlloc's valid layout.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count_allocation();
        // SAFETY: the caller supplies GlobalAlloc's valid layout.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: pointer and layout are passed through from the allocating caller.
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        count_allocation();
        // SAFETY: the caller supplies GlobalAlloc's valid allocation and size.
        unsafe { System.realloc(pointer, layout, size) }
    }
}
