//! Test-binary allocator observation, disabled except explicit synchronous scopes.
//! Requested live bytes, not physical heap/RSS or realloc's transient peak.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Usage {
  pub(super) allocated: usize,
  pub(super) freed: usize,
  pub(super) live: i64,
  pub(super) peak: i64,
}

thread_local! {
  static USAGE: Cell<Option<Usage>> = const { Cell::new(None) };
}

fn record(allocated: usize, freed: usize) {
  // TLS can already be destroyed during thread exit. Constant initialization
  // and Cell access do not allocate; never panic from allocator callbacks.
  let _ = USAGE.try_with(|cell| {
    if let Some(mut usage) = cell.get() {
      usage.allocated += allocated;
      usage.freed += freed;
      usage.live += allocated as i64 - freed as i64;
      usage.peak = usage.peak.max(usage.live);
      cell.set(Some(usage));
    }
  });
}

struct ObservedSystem;

// SAFETY: every operation forwards the original pointer/layout to System;
// observation never dereferences memory or changes allocation ownership.
unsafe impl GlobalAlloc for ObservedSystem {
  unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
    let pointer = unsafe { System.alloc(layout) };
    if !pointer.is_null() {
      record(layout.size(), 0);
    }
    pointer
  }
  unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
    let pointer = unsafe { System.alloc_zeroed(layout) };
    if !pointer.is_null() {
      record(layout.size(), 0);
    }
    pointer
  }
  unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
    record(0, layout.size());
    unsafe {
      System.dealloc(pointer, layout);
    }
  }
  unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
    let result = unsafe { System.realloc(pointer, layout, size) };
    if !result.is_null() {
      record(size, layout.size());
    }
    result
  }
}

#[global_allocator]
static ALLOCATOR: ObservedSystem = ObservedSystem;

pub(super) fn measure<T>(operation: impl FnOnce() -> T) -> (T, Usage) {
  struct Reset;
  impl Drop for Reset {
    fn drop(&mut self) {
      USAGE.with(|cell| cell.set(None));
    }
  }
  USAGE.with(|cell| assert!(cell.replace(Some(Usage::default())).is_none()));
  let _reset = Reset;
  let value = operation();
  (value, USAGE.with(|cell| cell.get().unwrap()))
}
