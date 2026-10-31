//! Durable records / waits.

use crate::RunProjection;
use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DurableWaitRecordV1 {
  pub run_id: String,
  pub wait_id: String,
  pub node_id: String,
  pub kind: crate::DurableWaitKind,
  pub registered_at: DateTime<Utc>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub deadline: Option<DateTime<Utc>>,
  pub state: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub resolved_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectedWaitStatusV1 {
  Pending,
  Signaled,
  Elapsed,
  TimedOut,
  Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectedWaitV1 {
  pub run_id: String,
  pub wait_id: String,
  pub name: String,
  pub kind: crate::DurableWaitKind,
  pub status: InspectedWaitStatusV1,
  pub registered_at: DateTime<Utc>,
  #[serde(skip_serializing_if = "Vec::is_empty")]
  pub signals: Vec<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub correlation_digest: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub deadline: Option<DateTime<Utc>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub resolved_at: Option<DateTime<Utc>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub winning_signal: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitInspectionV1 {
  pub profile: &'static str,
  pub run_id: String,
  pub waits: Vec<InspectedWaitV1>,
  pub pending_waits: u32,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub oldest_pending_wait_age_ms: Option<u64>,
}

pub const EXTERNAL_SIGNAL_ADMISSION_PROFILE: &str = "woml.external-signal-admission/v1";

pub const EXTERNAL_SIGNAL_RESULT_PROFILE: &str = "woml.signal-result/v1";

pub const EXTERNAL_SIGNAL_MAX_PAYLOAD_BYTES: usize = 1_048_576;

pub const EXTERNAL_SIGNAL_MAX_PAYLOAD_DEPTH: usize = 32;

pub const EXTERNAL_SIGNAL_MAX_EARLY_PER_RUN: usize = 64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExternalSignalAdmissionRequest {
  pub profile: String,
  pub signal: String,
  pub run_id: String,
  pub wait_id: String,
  pub correlation: String,
  pub idempotency_key: String,
  pub payload: Value,
  pub received_at: DateTime<Utc>,
  pub authentication_class: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalSignalAdmissionStatus {
  Accepted,
  Duplicate,
  IdentityConflict,
  AlreadyResolved,
  TargetNotFound,
  Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalSignalResolution {
  Early,
  Resolved,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExternalSignalAdmissionOutcome {
  pub profile: &'static str,
  pub status: ExternalSignalAdmissionStatus,
  pub signal_id: String,
  pub run_id: String,
  pub wait_id: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub resolution: Option<ExternalSignalResolution>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub code: Option<String>,
}

impl ExternalSignalAdmissionOutcome {
  pub fn resumes_run(&self) -> bool {
    self.status == ExternalSignalAdmissionStatus::Accepted
      && self.resolution == Some(ExternalSignalResolution::Resolved)
  }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SignalWaitTimeoutSettlement {
  NotDue { deadline: DateTime<Utc> },
  Continued { projection: RunProjection },
  Failed { projection: RunProjection },
  AlreadyResolved { projection: RunProjection },
}

#[derive(Debug, Clone, PartialEq)]
pub enum TimerWaitSettlement {
  NotDue { deadline: DateTime<Utc> },
  Resolved { projection: RunProjection },
  AlreadyResolved { projection: RunProjection },
}
