//! Private remote dispatch contracts.

use crate::DurableStoreError;
use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

pub const REMOTE_DISPATCH_RECOVERY_BATCH_LIMIT: usize = 10_000;

pub const REMOTE_DISPATCH_HEARTBEAT_MAX_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteDispatchStateV1 {
  Requested,
  Offered,
  Accepted,
  Started,
  CancelRequested,
  Succeeded,
  Failed,
  Cancelled,
  Ambiguous,
  Abandoned,
}

impl RemoteDispatchStateV1 {
  pub(in crate::remote_dispatch) fn as_str(self) -> &'static str {
    match self {
      Self::Requested => "requested",
      Self::Offered => "offered",
      Self::Accepted => "accepted",
      Self::Started => "started",
      Self::CancelRequested => "cancel_requested",
      Self::Succeeded => "succeeded",
      Self::Failed => "failed",
      Self::Cancelled => "cancelled",
      Self::Ambiguous => "ambiguous",
      Self::Abandoned => "abandoned",
    }
  }

  pub(in crate::remote_dispatch) fn parse(value: &str) -> Result<Self, RemoteDispatchError> {
    match value {
      "requested" => Ok(Self::Requested),
      "offered" => Ok(Self::Offered),
      "accepted" => Ok(Self::Accepted),
      "started" => Ok(Self::Started),
      "cancel_requested" => Ok(Self::CancelRequested),
      "succeeded" => Ok(Self::Succeeded),
      "failed" => Ok(Self::Failed),
      "cancelled" => Ok(Self::Cancelled),
      "ambiguous" => Ok(Self::Ambiguous),
      "abandoned" => Ok(Self::Abandoned),
      _ => Err(RemoteDispatchError::Corrupt),
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteDispatchRequestV1 {
  pub run_id: String,
  pub node_id: String,
  pub iteration_identity: String,
  pub attempt: u32,
  pub invocation_id: String,
  pub target: String,
  pub target_digest: String,
  pub package_digest: String,
  pub requested_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDispatchRecordV1 {
  pub dispatch_id: String,
  pub run_id: String,
  pub node_id: String,
  pub iteration_identity: String,
  pub attempt: u32,
  pub invocation_id: String,
  pub target: String,
  pub target_digest: String,
  pub package_digest: String,
  pub state: RemoteDispatchStateV1,
  pub worker_id: Option<String>,
  pub session_id: Option<String>,
  pub offer_id: Option<String>,
  pub offer_expires_at: Option<DateTime<Utc>>,
  pub accepted_at: Option<DateTime<Utc>>,
  pub started_at: Option<DateTime<Utc>>,
  pub last_heartbeat_at: Option<DateTime<Utc>>,
  pub last_heartbeat_sequence: Option<u64>,
  pub terminal_digest: Option<String>,
  pub ambiguity_reason: Option<String>,
  pub created_at: DateTime<Utc>,
  pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDispatchInspectionV1 {
  pub profile: &'static str,
  pub dispatch_id: String,
  pub run_id: String,
  pub node_id: String,
  pub attempt: u32,
  pub target: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub worker_id: Option<String>,
  pub state: &'static str,
  pub ambiguous: bool,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub last_heartbeat_at: Option<DateTime<Utc>>,
  pub output_sequence: u64,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub failure_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteOutputBindingV1 {
  pub workflow_id: String,
  pub run_id: String,
  pub node_id: String,
  pub attempt: u32,
  pub invocation_id: String,
  pub phase: crate::OperationalExecutionPhaseV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteDispatchResolutionV1 {
  RecordFailure,
  AuthorizePossibleDuplicateRetry,
}

#[derive(Debug, Error)]
pub enum RemoteDispatchError {
  #[error("the remote dispatch request is invalid")]
  Invalid,
  #[error("the remote dispatch was not found")]
  NotFound,
  #[error("the remote dispatch transition conflicts with durable state")]
  Conflict,
  #[error("the remote dispatch remains ambiguous and cannot be retried")]
  Ambiguous,
  #[error("the remote dispatch store is corrupt")]
  Corrupt,
  #[error(transparent)]
  Store(#[from] DurableStoreError),
  #[error(transparent)]
  Sql(#[from] rusqlite::Error),
}

impl RemoteDispatchError {
  pub const fn code(&self) -> &'static str {
    match self {
      Self::Invalid | Self::Corrupt => "WOML_REMOTE_PROTOCOL_INVALID",
      Self::NotFound => "WOML_TARGET_UNAVAILABLE",
      Self::Conflict => "WOML_REMOTE_DISPATCH_CONFLICT",
      Self::Ambiguous => "WOML_REMOTE_DISPATCH_AMBIGUOUS",
      Self::Store(_) | Self::Sql(_) => "WOML_TARGET_UNAVAILABLE",
    }
  }
}
