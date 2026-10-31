//! Invocation/global permit accounting and Drop-based capacity release on the existing shared registry.

use crate::capability::CapabilityRegistry;
use std::sync::atomic::Ordering;

pub(in crate::capability) struct CapabilityPermit<'a> {
  pub(in crate::capability) registry: &'a CapabilityRegistry,
  pub(in crate::capability) invocation_id: String,
}

impl Drop for CapabilityPermit<'_> {
  fn drop(&mut self) {
    let mut per_invocation = self
      .registry
      .in_flight_by_invocation
      .lock()
      .expect("capability in-flight lock");
    if let Some(count) = per_invocation.get_mut(&self.invocation_id) {
      *count -= 1;
      if *count == 0 {
        per_invocation.remove(&self.invocation_id);
      }
    }
    self
      .registry
      .in_flight_global
      .fetch_sub(1, Ordering::SeqCst);
  }
}

impl CapabilityRegistry {
  pub(in crate::capability) fn acquire(&self, invocation_id: &str) -> Option<CapabilityPermit<'_>> {
    let mut per_invocation = self
      .in_flight_by_invocation
      .lock()
      .expect("capability in-flight lock");
    let current = per_invocation.get(invocation_id).copied().unwrap_or(0);
    if current >= self.max_per_invocation
      || self.in_flight_global.load(Ordering::SeqCst) >= self.max_global
    {
      return None;
    }
    per_invocation.insert(invocation_id.to_string(), current + 1);
    self.in_flight_global.fetch_add(1, Ordering::SeqCst);
    Some(CapabilityPermit {
      registry: self,
      invocation_id: invocation_id.to_string(),
    })
  }
}
