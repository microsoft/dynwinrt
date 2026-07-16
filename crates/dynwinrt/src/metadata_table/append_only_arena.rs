// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::sync::RwLock;

/// Thread-safe, append-only arena that returns stable heap pointers.
///
/// Once a value is pushed:
/// * its address never changes (values are boxed on the heap), and
/// * it is never removed until the arena itself is dropped.
///
/// Callers may therefore take a raw `*const T` via [`stable_ptr`] while
/// holding the read guard, drop the guard, and later dereference the
/// pointer without risk of UAF.
///
/// The public surface intentionally exposes only `push`, `stable_ptr`,
/// and `len` — there is no way to `pop`, `remove`, `retain`, `clear`,
/// or otherwise invalidate a previously handed-out pointer through
/// this type. This is a compile-time enforcement of the invariant that
/// the raw-pointer callers rely on; changing the invariant requires
/// changing this type, not any downstream caller.
///
/// [`stable_ptr`]: AppendOnlyBoxArena::stable_ptr
pub(super) struct AppendOnlyBoxArena<T> {
    inner: RwLock<Vec<Box<T>>>,
}

impl<T> AppendOnlyBoxArena<T> {
    pub(super) fn new() -> Self {
        Self {
            inner: RwLock::new(Vec::new()),
        }
    }

    /// Append a value and return its arena index.
    pub(super) fn push(&self, value: T) -> u32 {
        let mut g = self.inner.write().unwrap();
        let idx = g.len() as u32;
        g.push(Box::new(value));
        idx
    }

    /// Get a raw pointer to the boxed value at `index`. The pointer is
    /// stable for the lifetime of the arena — see the type-level docs.
    ///
    /// Panics if `index` is out of range.
    pub(super) fn stable_ptr(&self, index: u32) -> *const T {
        let g = self.inner.read().unwrap();
        &**g.get(index as usize)
            .expect("AppendOnlyBoxArena index out of range")
    }

    #[allow(dead_code)]
    pub(super) fn len(&self) -> usize {
        self.inner.read().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointers_stay_valid_across_growth() {
        let arena: AppendOnlyBoxArena<u64> = AppendOnlyBoxArena::new();
        let mut ptrs = Vec::new();
        for i in 0..1024u64 {
            let idx = arena.push(i);
            ptrs.push((idx, arena.stable_ptr(idx)));
        }
        // Force many more pushes (Vec likely reallocates its backing
        // buffer several times); the boxed values must remain at their
        // original heap addresses.
        for i in 1024..8192u64 {
            arena.push(i);
        }
        for (idx, p) in ptrs {
            unsafe {
                assert_eq!(*p, idx as u64);
            }
        }
    }
}
