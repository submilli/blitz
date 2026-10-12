//! The allocator of blitz-dom's unit test binary, for every test in it: the
//! system allocator, counting what a thread holds while it asks to.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

/// Counts the bytes the current thread holds while it is tracking.
struct Counting;

thread_local! {
    /// Bytes held since tracking began, and the most held at once.
    static HELD: Cell<Option<(isize, isize)>> = const { Cell::new(None) };
}

fn note(change: isize) {
    // Never panics or allocates: the allocator calls it.
    let _ = HELD.try_with(|held| {
        if let Some((now, peak)) = held.get() {
            let now = now + change;
            held.set(Some((now, peak.max(now))));
        }
    });
}

// SAFETY: every call forwards to `System` unchanged, with the same layout;
// counting only reads the layout's size and touches a thread-local `Cell`.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note(layout.size() as isize);
        // SAFETY: the caller's contract for `alloc` is `System::alloc`'s.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note(-(layout.size() as isize));
        // SAFETY: `ptr` was allocated by `System` with `layout`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note(new_size as isize - layout.size() as isize);
        // SAFETY: the caller's contract for `realloc` is `System::realloc`'s.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

/// The most bytes the current thread held at once while running `work`,
/// besides what it held before; what `work` returns is dropped after.
pub(crate) fn peak_held<T>(work: impl FnOnce() -> T) -> usize {
    HELD.with(|held| held.set(Some((0, 0))));
    let result = work();
    let (_, peak) = HELD.with(|held| held.take()).unwrap_or_default();
    drop(result);
    peak.max(0) as usize
}
