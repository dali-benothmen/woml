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
