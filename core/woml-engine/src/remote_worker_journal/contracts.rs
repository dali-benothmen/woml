//! Private remote worker journal contracts.

use crate::remote_protocol::RemoteDispatchPackageV1;
use crate::remote_protocol::RemoteEffectiveAuthorityV1;
use crate::remote_protocol::REMOTE_TERMINAL_RESULT_MAX_BYTES;
use crate::remote_worker_journal::sha256;
use crate::CapabilityFailure;
use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json_canonicalizer::to_vec as canonical_json;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use thiserror::Error;

pub const WORKER_JOURNAL_RECOVERY_LIMIT: usize = 10_000;

pub const REMOTE_CANCELLATION_CONFIRMATION_TIMEOUT_MS: u64 = 2_000;

#[derive(Debug, Clone, Default)]
pub struct RemoteWorkerCancellationTokenV1(pub(in crate::remote_worker_journal) Arc<AtomicBool>);

impl RemoteWorkerCancellationTokenV1 {
  pub fn cancel(&self) {
    self.0.store(true, Ordering::Release);
  }

  pub fn is_cancelled(&self) -> bool {
    self.0.load(Ordering::Acquire)
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerAttemptStateV1 {
  Accepted,
  Started,
  Succeeded,
  Failed,
  Cancelled,
  Ambiguous,
}

impl WorkerAttemptStateV1 {
  pub(in crate::remote_worker_journal) fn as_str(self) -> &'static str {
    match self {
      Self::Accepted => "accepted",
      Self::Started => "started",
      Self::Succeeded => "succeeded",
      Self::Failed => "failed",
      Self::Cancelled => "cancelled",
      Self::Ambiguous => "ambiguous",
    }
  }

  pub(in crate::remote_worker_journal) fn parse(value: &str) -> Result<Self, RemoteWorkerError> {
    match value {
      "accepted" => Ok(Self::Accepted),
      "started" => Ok(Self::Started),
      "succeeded" => Ok(Self::Succeeded),
      "failed" => Ok(Self::Failed),
      "cancelled" => Ok(Self::Cancelled),
      "ambiguous" => Ok(Self::Ambiguous),
      _ => Err(RemoteWorkerError::CorruptJournal),
    }
  }

  pub const fn is_terminal(self) -> bool {
    matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
  }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteWorkerTerminalV1 {
  pub profile: String,
  pub dispatch_id: String,
  pub invocation_id: String,
  pub state: WorkerAttemptStateV1,
  pub outcome: Value,
  pub completed_at: DateTime<Utc>,
  pub terminal_digest: String,
}

impl RemoteWorkerTerminalV1 {
  pub(in crate::remote_worker_journal) fn new(
    dispatch_id: &str,
    invocation_id: &str,
    state: WorkerAttemptStateV1,
    outcome: Value,
    completed_at: DateTime<Utc>,
  ) -> Result<Self, RemoteWorkerError> {
    let mut terminal = Self {
      profile: "woml.worker-terminal/v1".to_string(),
      dispatch_id: dispatch_id.to_string(),
      invocation_id: invocation_id.to_string(),
      state,
      outcome,
      completed_at,
      terminal_digest: String::new(),
    };
    let bytes = canonical_json(&terminal).map_err(|_| RemoteWorkerError::InvalidResult)?;
    if bytes.len() > REMOTE_TERMINAL_RESULT_MAX_BYTES {
      return Err(RemoteWorkerError::InvalidResult);
    }
    terminal.terminal_digest = sha256(&bytes);
    Ok(terminal)
  }

  pub(in crate::remote_worker_journal) fn verify(&self) -> Result<(), RemoteWorkerError> {
    let mut value = serde_json::to_value(self).map_err(|_| RemoteWorkerError::InvalidResult)?;
    value
      .as_object_mut()
      .ok_or(RemoteWorkerError::InvalidResult)?
      .insert("terminalDigest".to_string(), Value::String(String::new()));
    let bytes = canonical_json(&value).map_err(|_| RemoteWorkerError::InvalidResult)?;
    if bytes.len() > REMOTE_TERMINAL_RESULT_MAX_BYTES || sha256(&bytes) != self.terminal_digest {
      return Err(RemoteWorkerError::InvalidResult);
    }
    Ok(())
  }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkerAttemptRecordV1 {
  pub dispatch_id: String,
  pub package_digest: String,
  pub worker_id: String,
  pub run_id: String,
  pub node_id: String,
  pub iteration_identity: String,
  pub attempt: u32,
  pub invocation_id: String,
  pub target: String,
  pub authority_digest: String,
  pub state: WorkerAttemptStateV1,
  pub terminal: Option<RemoteWorkerTerminalV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteWorkerAuthorityV1 {
  pub policy_digest: String,
  pub allowed_capabilities: BTreeMap<String, u32>,
  pub allowed_secret_names: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreparedRemoteDispatchV1 {
  pub package: RemoteDispatchPackageV1,
  pub effective_authority: RemoteEffectiveAuthorityV1,
  pub worker_id: String,
}

#[derive(Debug, Error)]
pub enum RemoteWorkerError {
  #[error("the remote dispatch package is invalid")]
  InvalidPackage,
  #[error("the remote dispatch package is expired")]
  Expired,
  #[error("the remote capability is unavailable")]
  CapabilityUnavailable,
  #[error("the remote dispatch conflicts with the worker journal")]
  Conflict,
  #[error("the worker journal is corrupt or incompatible")]
  CorruptJournal,
  #[error("the worker is draining")]
  Draining,
  #[error("the worker has no execution capacity")]
  Capacity,
  #[error("the accepted attempt has an ambiguous outcome")]
  Ambiguous,
  #[error("the remote attempt result is invalid")]
  InvalidResult,
  #[error("the remote artifact transfer failed integrity or authorization checks")]
  ArtifactIntegrity,
  #[error("the remote attempt failed to start or communicate with the script host")]
  HostUnavailable,
  #[error("the configured secret is unavailable or unauthorized")]
  SecretUnavailable,
  #[error("the worker journal is unavailable")]
  Store(#[from] rusqlite::Error),
}

impl RemoteWorkerError {
  pub const fn code(&self) -> &'static str {
    match self {
      Self::InvalidPackage | Self::CorruptJournal => "WOML_REMOTE_PROTOCOL_INVALID",
      Self::Expired | Self::CapabilityUnavailable => "WOML_TARGET_CAPABILITY_UNAVAILABLE",
      Self::Conflict => "WOML_REMOTE_DISPATCH_CONFLICT",
      Self::Draining | Self::Capacity | Self::Store(_) | Self::HostUnavailable => {
        "WOML_TARGET_UNAVAILABLE"
      }
      Self::Ambiguous => "WOML_REMOTE_DISPATCH_AMBIGUOUS",
      Self::InvalidResult => "WOML_REMOTE_RESULT_INVALID",
      Self::ArtifactIntegrity => "WOML_REMOTE_ARTIFACT_INTEGRITY_FAILED",
      Self::SecretUnavailable => "WOML_SECRET_UNAVAILABLE",
    }
  }
}

impl From<CapabilityFailure> for RemoteWorkerError {
  fn from(_: CapabilityFailure) -> Self {
    Self::CapabilityUnavailable
  }
}
