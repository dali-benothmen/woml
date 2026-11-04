//! Thread-local, scoped structural replay counts. Test-only, never telemetry.
use std::cell::RefCell;

thread_local! {
  static PASSES: RefCell<Option<Vec<usize>>> = const { RefCell::new(None) };
}

pub(super) fn record(events: usize) {
  PASSES.with(|passes| {
    if let Some(passes) = passes.borrow_mut().as_mut() {
      passes.push(events);
    }
  });
}

pub(crate) fn observe<T>(operation: impl FnOnce() -> T) -> (T, Vec<usize>) {
  struct Reset;
  impl Drop for Reset {
    fn drop(&mut self) {
      PASSES.with(|passes| *passes.borrow_mut() = None);
    }
  }
  PASSES.with(|passes| {
    assert!(passes.borrow().is_none(), "Fold observations must not nest");
    *passes.borrow_mut() = Some(Vec::new());
  });
  let _reset = Reset;
  let result = operation();
  let counts = PASSES.with(|passes| passes.borrow_mut().take().unwrap());
  (result, counts)
}
