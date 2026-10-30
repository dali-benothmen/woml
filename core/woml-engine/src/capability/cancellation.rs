//! Shared cooperative cancellation token and wakeup state. Preserve atomic ordering and notification behavior.

use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::sync::Notify;

#[derive(Debug, Clone, Default)]
pub struct CapabilityCancellationToken {
  pub(in crate::capability) state: Arc<CancellationState>,
}

#[derive(Debug, Default)]
pub(in crate::capability) struct CancellationState {
  pub(in crate::capability) cancelled: AtomicBool,
  pub(in crate::capability) notify: Notify,
}

impl CapabilityCancellationToken {
  pub fn cancel(&self) {
    if !self.state.cancelled.swap(true, Ordering::SeqCst) {
      self.state.notify.notify_waiters();
    }
  }

  pub fn is_cancelled(&self) -> bool {
    self.state.cancelled.load(Ordering::SeqCst)
  }

  pub async fn cancelled(&self) {
    while !self.is_cancelled() {
      self.state.notify.notified().await;
    }
  }
}
