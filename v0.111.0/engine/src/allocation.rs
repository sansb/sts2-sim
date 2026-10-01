//! Feature-gated allocation telemetry shared by the benchmark binary and UCT.

#[cfg(feature = "allocation-counting")]
use std::alloc::{GlobalAlloc, Layout, System};
#[cfg(feature = "allocation-counting")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "allocation-counting")]
use std::{cell::Cell, thread_local};

#[cfg(feature = "allocation-counting")]
static ALLOCATION_COUNT: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "allocation-counting")]
static ALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "allocation-counting")]
thread_local! {
    static THREAD_ALLOCATION_COUNT: Cell<u64> = const { Cell::new(0) };
    static THREAD_ALLOCATED_BYTES: Cell<u64> = const { Cell::new(0) };
}

/// Process allocator used only by the separately built instrumented binary.
pub struct CountingAllocator;

#[cfg(feature = "allocation-counting")]
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);
        ALLOCATED_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        THREAD_ALLOCATION_COUNT.set(THREAD_ALLOCATION_COUNT.get().saturating_add(1));
        THREAD_ALLOCATED_BYTES.set(
            THREAD_ALLOCATED_BYTES
                .get()
                .saturating_add(layout.size() as u64),
        );
        // SAFETY: forwarding the exact allocation request to the system allocator.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` and `layout` came from the corresponding system allocation.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[cfg(not(feature = "allocation-counting"))]
unsafe impl std::alloc::GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        // SAFETY: forwarding the exact allocation request to the system allocator.
        unsafe { std::alloc::System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        // SAFETY: `ptr` and `layout` came from the corresponding system allocation.
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
}

/// Cumulative process allocation count and requested bytes.
#[inline]
pub fn snapshot() -> (u64, u64) {
    #[cfg(feature = "allocation-counting")]
    {
        (
            ALLOCATION_COUNT.load(Ordering::Relaxed),
            ALLOCATED_BYTES.load(Ordering::Relaxed),
        )
    }
    #[cfg(not(feature = "allocation-counting"))]
    {
        (0, 0)
    }
}

pub const fn instrumented() -> bool {
    cfg!(feature = "allocation-counting")
}

/// Allocation totals for the current worker thread only. UCT uses this to
/// classify a simulation without pollution from concurrently running workers.
#[inline]
pub fn thread_snapshot() -> (u64, u64) {
    #[cfg(feature = "allocation-counting")]
    {
        (THREAD_ALLOCATION_COUNT.get(), THREAD_ALLOCATED_BYTES.get())
    }
    #[cfg(not(feature = "allocation-counting"))]
    {
        (0, 0)
    }
}
