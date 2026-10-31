//! projection correction.

use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SagaPhase {
  Forward,
  ForwardSucceeded,
  Correcting,
  CorrectionCompleted,
  ManualRecovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CorrectionActionStatus {
  Registered,
  Started,
  RetryScheduled,
  Succeeded,
  Failed,
  Interrupted,
  ManualRecovery,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionActionProjection {
  pub action_id: String,
  pub action_kind: crate::CorrectiveActionKind,
  pub for_step_id: Option<String>,
  pub completion_sequence: Option<u64>,
  pub status: CorrectionActionStatus,
  pub attempts: u32,
  pub active_attempt: Option<u32>,
  pub retry_at: Option<DateTime<Utc>>,
  pub failure_code: Option<String>,
  pub ambiguous: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SagaProjection {
  pub saga_id: String,
  pub phase: SagaPhase,
  pub reason: Option<crate::SagaCorrectionReason>,
  pub started_sequence: u64,
  pub forward_succeeded_sequence: Option<u64>,
  pub correction_completed_sequence: Option<u64>,
  pub actions: BTreeMap<String, CorrectionActionProjection>,
}

/// A corrective effect belongs to one exact durable attempt, never merely to
/// an action name. Iteration-qualified action IDs remain part of that identity.
pub(crate) fn active_correction<'a>(
  projection: &'a super::RunProjection,
  node_id: &str,
  attempt: u32,
  invocation_id: &str,
) -> Option<&'a CorrectionActionProjection> {
  let run_id = projection.run_id.as_deref()?;
  projection.sagas.values().find_map(|saga| {
    let action = saga.actions.get(node_id)?;
    (matches!(
      saga.phase,
      SagaPhase::Correcting | SagaPhase::ManualRecovery
    ) && action.status == CorrectionActionStatus::Started
      && action.active_attempt == Some(attempt)
      && crate::durable::correction_invocation_id(run_id, &saga.saga_id, node_id, attempt)
        == invocation_id)
      .then_some(action)
  })
}
