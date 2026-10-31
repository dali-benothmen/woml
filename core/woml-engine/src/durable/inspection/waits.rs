//! Payload-free bounded wait inspection using the existing projection and clock.

use crate::durable::{
  DurableEventStore, DurableStoreError, InspectedWaitStatusV1, InspectedWaitV1, WaitInspectionV1,
};
use chrono::Utc;
use serde_json::Value;

impl DurableEventStore {
  pub fn inspect_waits_v1(&self, run_id: &str) -> Result<WaitInspectionV1, DurableStoreError> {
    let binding = self.run_binding(run_id)?;
    let workflow = self.definition(&binding.definition_hash)?;
    let projection = self.projection(run_id)?;
    let now = Utc::now();
    let mut waits = projection
      .durable_waits
      .values()
      .map(|wait| {
        let winning_signal = match wait.status {
          crate::projection::DurableWaitStatus::Satisfied {
            winner: crate::DurableWaitWinner::Signal,
          } => projection
            .external_signals
            .iter()
            .rev()
            .find(|signal| signal.wait_id == wait.wait_id)
            .map(|signal| signal.signal.clone()),
          _ => None,
        };
        let status = match wait.status {
          crate::projection::DurableWaitStatus::Registered => InspectedWaitStatusV1::Pending,
          crate::projection::DurableWaitStatus::Satisfied {
            winner: crate::DurableWaitWinner::Signal,
          } => InspectedWaitStatusV1::Signaled,
          crate::projection::DurableWaitStatus::Satisfied {
            winner: crate::DurableWaitWinner::Timer,
          } => InspectedWaitStatusV1::Elapsed,
          crate::projection::DurableWaitStatus::TimedOut { .. } => InspectedWaitStatusV1::TimedOut,
          crate::projection::DurableWaitStatus::Cancelled => InspectedWaitStatusV1::Cancelled,
        };
        let name = workflow
          .node(&wait.node_id)
          .and_then(|node| node.metadata.as_ref())
          .and_then(|metadata| metadata.get("name"))
          .and_then(Value::as_str)
          .unwrap_or(&wait.wait_id)
          .to_string();
        InspectedWaitV1 {
          run_id: run_id.to_string(),
          wait_id: wait.wait_id.clone(),
          name,
          kind: wait.kind,
          status,
          registered_at: wait.registered_at,
          signals: wait.signals.clone(),
          correlation_digest: wait.correlation_digest.clone(),
          deadline: wait.deadline,
          resolved_at: wait.resolved_at,
          winning_signal,
        }
      })
      .collect::<Vec<_>>();
    waits.sort_by(|left, right| {
      left
        .registered_at
        .cmp(&right.registered_at)
        .then_with(|| left.wait_id.cmp(&right.wait_id))
    });
    let pending = waits
      .iter()
      .filter(|wait| wait.status == InspectedWaitStatusV1::Pending)
      .collect::<Vec<_>>();
    let oldest_pending_wait_age_ms = pending
      .iter()
      .map(|wait| now.signed_duration_since(wait.registered_at))
      .filter_map(|age| u64::try_from(age.num_milliseconds().max(0)).ok())
      .max();
    Ok(WaitInspectionV1 {
      profile: "woml.wait-inspection/v1",
      run_id: run_id.to_string(),
      pending_waits: u32::try_from(pending.len()).unwrap_or(u32::MAX),
      oldest_pending_wait_age_ms,
      waits,
    })
  }
}
