//! Durable records / correction.

use serde::Deserialize;
use serde::Serialize;

pub const CORRECTION_INSPECTION_PROFILE: &str = "woml.correction-inspection/v1";

pub const CORRECTION_RECOVERY_REQUEST_PROFILE: &str = "woml.correction-recovery-request/v1";

pub const CORRECTION_RECOVERY_RESULT_PROFILE: &str = "woml.correction-recovery-result/v1";

pub const CORRECTION_OBSERVATION_PROFILE: &str = "woml.correction-observation/v1";

pub const CORRECTION_RECOVERY_BATCH_LIMIT: usize = 100;

pub const CORRECTION_UNRESOLVED_STORE_LIMIT: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionSagaInspectionV1 {
  pub saga_id: String,
  pub status: String,
  pub registered_compensations: u32,
  pub completed_compensations: u32,
  pub completed_cleanup: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnresolvedCorrectionActionV1 {
  pub action_id: String,
  pub kind: crate::CorrectiveActionKind,
  pub code: String,
  pub ambiguous: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionInspectionV1 {
  pub profile: &'static str,
  pub run_id: String,
  pub phase: String,
  pub original_outcome: String,
  pub sagas: Vec<CorrectionSagaInspectionV1>,
  pub unresolved_actions: Vec<UnresolvedCorrectionActionV1>,
  pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CorrectionRecoveryActionV1 {
  Retry,
  ExternallyResolve,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CorrectionRecoveryRequestV1 {
  pub profile: String,
  pub run_id: String,
  pub action: CorrectionRecoveryActionV1,
  pub action_id: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CorrectionRecoveryStatusV1 {
  RetryScheduled,
  ExternallyResolved,
  Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionRecoveryResultV1 {
  pub profile: &'static str,
  pub run_id: String,
  pub action_id: String,
  pub status: CorrectionRecoveryStatusV1,
  pub audit_recorded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionRecoveryCandidateV1 {
  pub run_id: String,
  pub saga_id: String,
  pub action_id: String,
  pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionObservationV1 {
  pub profile: &'static str,
  pub open_sagas: u64,
  pub pending_actions: u64,
  pub active_attempts: u64,
  pub manual_recovery_actions: u64,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub oldest_manual_recovery_age_ms: Option<u64>,
}
