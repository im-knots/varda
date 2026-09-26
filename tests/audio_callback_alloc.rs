//! The audio capture callback runs on the device's real-time thread, where an
//! allocation can stall long enough to drop audio. With no passthrough
//! subscribed it must allocate nothing. See /spec/performance-hot-paths.md
//! item H.
//!
//! Servo installs its own global allocator, so this runs in builds without
//! `html`, such as the Windows test job.
#![cfg(not(feature = "html"))]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Counts allocations made on a thread while that thread is watching.
struct Counting;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static WATCHING: Cell<bool> = const { Cell::new(false) };
}

// SAFETY: every method defers to the system allocator; counting touches only
// an atomic and a const-initialized thread local, neither of which allocates.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if WATCHING.with(Cell::get) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if WATCHING.with(Cell::get) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

#[test]
fn the_capture_callback_allocates_nothing() {
    // The counter itself works: one allocation, watched, is one counted.
    WATCHING.with(|w| w.set(true));
    drop(std::hint::black_box(Vec::<u8>::with_capacity(8)));
    WATCHING.with(|w| w.set(false));
    assert_eq!(ALLOCATIONS.swap(0, Ordering::Relaxed), 1);

    let mut capture = varda::testing::AudioCaptureBench::new();
    capture.callback();

    WATCHING.with(|w| w.set(true));
    for _ in 0..200 {
        capture.callback();
    }
    WATCHING.with(|w| w.set(false));

    assert_eq!(
        ALLOCATIONS.load(Ordering::Relaxed),
        0,
        "the capture callback allocated on the real-time thread"
    );
}
