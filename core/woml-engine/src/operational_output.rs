//! Bounded, non-authoritative operational output persistence.
//!
//! Output Store v1 is deliberately separate from the durable workflow event
//! store. Losing this database can remove logs and progress, but cannot change
//! a workflow projection, retry decision, business result, or recovery path.

use std::{
  fs::{self, OpenOptions},
  path::{Path, PathBuf},
  sync::{Arc, Mutex},
};

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const OPERATIONAL_RECORD_PROFILE: &str = "woml.operational-record/v1";
pub const OPERATIONAL_PAGE_PROFILE: &str = "woml.operational-page/v1";
pub const OPERATIONAL_STREAM_PROFILE: &str = "woml.operational-stream/v1";
pub const OPERATIONAL_OUTPUT_STORE_SCHEMA_VERSION: u32 = 1;

pub const MAX_OPERATIONAL_CHUNK_BYTES: usize = 16 * 1024;
pub const MAX_OPERATIONAL_TEXT_BYTES: usize = 16 * 1024;
pub const MAX_OPERATIONAL_PAGE_RECORDS: usize = 1_000;
pub const MAX_OPERATIONAL_PAGE_BYTES: usize = 1024 * 1024;
pub const MAX_OPERATIONAL_FOLLOWERS: usize = 8;
pub const DEFAULT_OPERATIONAL_INLINE_RUN_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_OPERATIONAL_INLINE_RUN_BYTES: u64 = 256 * 1024 * 1024;
pub const DEFAULT_OPERATIONAL_RETENTION_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
pub const DEFAULT_OPERATIONAL_STORE_BYTES: u64 = 1024 * 1024 * 1024;
pub const OPERATIONAL_PROGRESS_CADENCE_MS: i64 = 250;
pub type OperationalOutputReporter = Arc<dyn Fn(u64) + Send + Sync>;

const MAX_ID_BYTES: usize = 320;
const MAX_REASON_BYTES: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationalOutputLimitsV1 {
  pub max_attempt_bytes: u64,
  pub max_run_bytes: u64,
  pub max_store_bytes: u64,
  pub retention_ms: u64,
}

impl Default for OperationalOutputLimitsV1 {
  fn default() -> Self {
    Self {
      max_attempt_bytes: DEFAULT_OPERATIONAL_INLINE_RUN_BYTES,
      max_run_bytes: DEFAULT_OPERATIONAL_INLINE_RUN_BYTES,
      max_store_bytes: DEFAULT_OPERATIONAL_STORE_BYTES,
      retention_ms: DEFAULT_OPERATIONAL_RETENTION_MS,
    }
  }
}

#[derive(Debug, Clone, PartialEq)]
pub enum OperationalAppendOutcomeV1 {
  Appended(OperationalRecordV1),
  Truncated(OperationalRecordV1),
  Dropped { marker_sequence: u64 },
}

#[derive(Debug, Clone, PartialEq)]
pub enum RemoteOperationalAppendOutcomeV1 {
  Appended { local_sequence: u64 },
  Duplicate { local_sequence: u64 },
}

#[derive(Debug, Clone, Copy)]
struct RemoteOperationalIdentity<'a> {
  dispatch_id: &'a str,
  sequence: u64,
  digest: &'a str,
}

impl OperationalAppendOutcomeV1 {
  pub fn sequence(&self) -> u64 {
    match self {
      Self::Appended(record) | Self::Truncated(record) => record.sequence,
      Self::Dropped { marker_sequence } => *marker_sequence,
    }
  }

  pub fn retained(&self) -> bool {
    matches!(self, Self::Appended(_))
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OperationalSourceV1 {
  Stdout,
  Stderr,
  Script,
  Runtime,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OperationalExecutionPhaseV1 {
  #[default]
  Forward,
  Corrective,
}

impl OperationalExecutionPhaseV1 {
  fn as_str(self) -> &'static str {
    match self {
      Self::Forward => "forward",
      Self::Corrective => "corrective",
    }
  }

  fn is_forward(value: &Self) -> bool {
    matches!(value, Self::Forward)
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OperationalLogLevelV1 {
  Debug,
  Info,
  Warn,
  Error,
}

impl OperationalSourceV1 {
  fn as_str(self) -> &'static str {
    match self {
      Self::Stdout => "stdout",
      Self::Stderr => "stderr",
      Self::Script => "script",
      Self::Runtime => "runtime",
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationalCloseStatusV1 {
  Succeeded,
  Failed,
  Cancelled,
  TimedOut,
  Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OperationalProgressStatusV1 {
  Active,
  Closed,
}

impl OperationalProgressStatusV1 {
  fn as_str(self) -> &'static str {
    match self {
      Self::Active => "active",
      Self::Closed => "closed",
    }
  }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OperationalBodyV1 {
  Output {
    source: OperationalSourceV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    level: Option<OperationalLogLevelV1>,
    text: String,
    #[serde(rename = "byteCount")]
    byte_count: u64,
  },
  Progress {
    #[serde(rename = "progressId")]
    progress_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    current: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    status: OperationalProgressStatusV1,
  },
  Truncated {
    source: OperationalSourceV1,
    #[serde(rename = "droppedBytes")]
    dropped_bytes: u64,
    reason: String,
  },
  Artifact {
    #[serde(rename = "artifactId")]
    artifact_id: String,
    name: String,
    size: u64,
    sha256: String,
    #[serde(rename = "contentType")]
    content_type: String,
  },
  Closed {
    status: OperationalCloseStatusV1,
    #[serde(rename = "reasonCode", skip_serializing_if = "Option::is_none")]
    reason_code: Option<String>,
  },
  Gap {
    #[serde(rename = "requestedAfter")]
    requested_after: u64,
    #[serde(rename = "prunedThrough")]
    pruned_through: u64,
    #[serde(rename = "earliestAvailable", skip_serializing_if = "Option::is_none")]
    earliest_available: Option<u64>,
  },
}

impl OperationalBodyV1 {
  fn kind(&self) -> &'static str {
    match self {
      Self::Output { .. } => "output",
      Self::Progress { .. } => "progress",
      Self::Truncated { .. } => "truncated",
      Self::Artifact { .. } => "artifact",
      Self::Closed { .. } => "closed",
      Self::Gap { .. } => "gap",
    }
  }

  fn source(&self) -> Option<&'static str> {
    match self {
      Self::Output { source, .. } | Self::Truncated { source, .. } => Some(source.as_str()),
      _ => None,
    }
  }

  fn accounted_bytes(&self) -> u64 {
    match self {
      Self::Output { byte_count, .. } => *byte_count,
      _ => 0,
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationalIdentityV1 {
  pub workflow_id: String,
  pub run_id: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub node_id: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub attempt: Option<u32>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub invocation_id: Option<String>,
  #[serde(
    default,
    skip_serializing_if = "OperationalExecutionPhaseV1::is_forward"
  )]
  pub phase: OperationalExecutionPhaseV1,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationalRecordDraftV1 {
  pub identity: OperationalIdentityV1,
  pub body: OperationalBodyV1,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationalRecordV1 {
  pub profile: String,
  pub deployment_id: String,
  pub sequence: u64,
  pub occurred_at: String,
  #[serde(flatten)]
  pub identity: OperationalIdentityV1,
  #[serde(flatten)]
  pub body: OperationalBodyV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationalGapV1 {
  pub requested_after: u64,
  pub pruned_through: u64,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub earliest_available: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationalPageV1 {
  pub profile: String,
  pub deployment_id: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub run_id: Option<String>,
  pub requested_after: u64,
  pub next_after: u64,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub earliest_available: Option<u64>,
  pub latest_sequence: u64,
  pub has_more: bool,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub gap: Option<OperationalGapV1>,
  pub records: Vec<OperationalRecordV1>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum OperationalStreamEventV1 {
  Record {
    profile: String,
    cursor: u64,
    record: Box<OperationalRecordV1>,
  },
  Gap {
    profile: String,
    cursor: u64,
    gap: OperationalGapV1,
  },
  Heartbeat {
    profile: String,
    cursor: u64,
  },
  Closed {
    profile: String,
    cursor: u64,
  },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationalProgressSnapshotV1 {
  pub run_id: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub node_id: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub attempt: Option<u32>,
  pub progress_id: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub current: Option<f64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub total: Option<f64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub unit: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub message: Option<String>,
  pub status: OperationalProgressStatusV1,
  pub sequence: u64,
  pub occurred_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationalOutputStoreInspectionV1 {
  pub profile: String,
  pub schema_version: u32,
  pub deployment_id: String,
  pub next_sequence: u64,
  pub earliest_available: Option<u64>,
  pub latest_sequence: u64,
  pub pruned_through: u64,
  pub record_count: u64,
  pub retained_output_bytes: u64,
  pub active_runs: u64,
  pub terminal_runs: u64,
  pub truncated_streams: u64,
  pub max_attempt_bytes: u64,
  pub max_run_bytes: u64,
  pub max_store_bytes: u64,
  pub retention_ms: u64,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub last_pruned_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationalOutputPruneResultV1 {
  pub profile: String,
  pub pruned_runs: u64,
  pub pruned_records: u64,
  pub pruned_output_bytes: u64,
  pub pruned_through: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationalOutputPrunePlanV1 {
  pub profile: String,
  pub eligible_runs: u64,
  pub eligible_records: u64,
  pub eligible_output_bytes: u64,
}

#[derive(Debug, Error)]
pub enum OperationalOutputError {
  #[error("the operational output store path is unsafe")]
  UnsafePath,
  #[error("the operational output store is unavailable")]
  Unavailable,
  #[error("the operational output store failed its integrity check")]
  Corrupt,
  #[error("unsupported operational output store schema version {0}")]
  UnsupportedVersion(String),
  #[error("the operational output store belongs to another deployment")]
  DeploymentMismatch,
  #[error("the operational output record is invalid: {0}")]
  InvalidRecord(&'static str),
  #[error("the operational output cursor request is invalid")]
  InvalidCursor,
}

impl OperationalOutputError {
  pub fn code(&self) -> &'static str {
    match self {
      Self::UnsafePath => "WOML_OUTPUT_STORE_PATH_UNSAFE",
      Self::Unavailable => "WOML_OUTPUT_STORE_UNAVAILABLE",
      Self::Corrupt => "WOML_OUTPUT_STORE_CORRUPT",
      Self::UnsupportedVersion(_) => "WOML_OUTPUT_STORE_VERSION_UNSUPPORTED",
      Self::DeploymentMismatch => "WOML_OUTPUT_STORE_DEPLOYMENT_MISMATCH",
      Self::InvalidRecord(_) => "WOML_OUTPUT_RECORD_INVALID",
      Self::InvalidCursor => "WOML_OUTPUT_CURSOR_INVALID",
    }
  }
}

/// Rust-owned, serialized access to one deployment's operational database.
pub struct OperationalOutputStore {
  path: PathBuf,
  deployment_id: String,
  connection: Mutex<Connection>,
  reporter: Option<OperationalOutputReporter>,
  limits: OperationalOutputLimitsV1,
}

impl std::fmt::Debug for OperationalOutputStore {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    formatter
      .debug_struct("OperationalOutputStore")
      .field("path", &self.path)
      .field("deployment_id", &self.deployment_id)
      .finish_non_exhaustive()
  }
}

impl OperationalOutputStore {
  pub fn open(
    path: impl AsRef<Path>,
    deployment_id: impl Into<String>,
  ) -> Result<Self, OperationalOutputError> {
    Self::open_with_reporter(path, deployment_id, None)
  }

  pub fn open_with_reporter(
    path: impl AsRef<Path>,
    deployment_id: impl Into<String>,
    reporter: Option<OperationalOutputReporter>,
  ) -> Result<Self, OperationalOutputError> {
    Self::open_with_limits_and_reporter(
      path,
      deployment_id,
      OperationalOutputLimitsV1::default(),
      reporter,
    )
  }

  pub fn open_with_limits(
    path: impl AsRef<Path>,
    deployment_id: impl Into<String>,
    limits: OperationalOutputLimitsV1,
  ) -> Result<Self, OperationalOutputError> {
    Self::open_with_limits_and_reporter(path, deployment_id, limits, None)
  }

  fn open_with_limits_and_reporter(
    path: impl AsRef<Path>,
    deployment_id: impl Into<String>,
    limits: OperationalOutputLimitsV1,
    reporter: Option<OperationalOutputReporter>,
  ) -> Result<Self, OperationalOutputError> {
    validate_limits(limits)?;
    let deployment_id = deployment_id.into();
    validate_id(&deployment_id)?;
    let path = prepare_private_store_path(path.as_ref())?;
    let connection = Connection::open(&path).map_err(|_| OperationalOutputError::Unavailable)?;
    connection
      .busy_timeout(std::time::Duration::from_secs(5))
      .map_err(|_| OperationalOutputError::Unavailable)?;
    connection
      .execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = DELETE;
         PRAGMA synchronous = FULL;
         PRAGMA trusted_schema = OFF;",
      )
      .map_err(|_| OperationalOutputError::Unavailable)?;
    ensure_integrity(&connection)?;
    initialize_schema(&connection, &deployment_id)?;
    validate_schema(&connection, &deployment_id)?;
    harden_file_permissions(&path)?;
    Ok(Self {
      path,
      deployment_id,
      connection: Mutex::new(connection),
      reporter,
      limits,
    })
  }

  pub fn path(&self) -> &Path {
    &self.path
  }

  pub fn deployment_id(&self) -> &str {
    &self.deployment_id
  }

  pub fn append(
    &self,
    draft: OperationalRecordDraftV1,
  ) -> Result<OperationalRecordV1, OperationalOutputError> {
    match self.append_bounded(draft)? {
      OperationalAppendOutcomeV1::Appended(record)
      | OperationalAppendOutcomeV1::Truncated(record) => Ok(record),
      OperationalAppendOutcomeV1::Dropped { .. } => Err(OperationalOutputError::InvalidRecord(
        "the operational output lane is already truncated",
      )),
    }
  }

  pub fn append_bounded(
    &self,
    draft: OperationalRecordDraftV1,
  ) -> Result<OperationalAppendOutcomeV1, OperationalOutputError> {
    self
      .append_bounded_internal(draft, None)
      .map(|(outcome, _)| outcome)
  }

  pub fn append_remote_bounded(
    &self,
    dispatch_id: &str,
    remote_sequence: u64,
    record_digest: &str,
    draft: OperationalRecordDraftV1,
  ) -> Result<RemoteOperationalAppendOutcomeV1, OperationalOutputError> {
    if !valid_remote_dispatch_id(dispatch_id)
      || remote_sequence == 0
      || !valid_sha256(record_digest)
    {
      return Err(OperationalOutputError::InvalidRecord(
        "the remote output identity is invalid",
      ));
    }
    let (outcome, duplicate) = self.append_bounded_internal(
      draft,
      Some(RemoteOperationalIdentity {
        dispatch_id,
        sequence: remote_sequence,
        digest: record_digest,
      }),
    )?;
    if duplicate {
      Ok(RemoteOperationalAppendOutcomeV1::Duplicate {
        local_sequence: outcome.sequence(),
      })
    } else {
      Ok(RemoteOperationalAppendOutcomeV1::Appended {
        local_sequence: outcome.sequence(),
      })
    }
  }

  fn append_bounded_internal(
    &self,
    mut draft: OperationalRecordDraftV1,
    remote: Option<RemoteOperationalIdentity<'_>>,
  ) -> Result<(OperationalAppendOutcomeV1, bool), OperationalOutputError> {
    validate_draft(&draft)?;
    let occurred_at = now_string();
    let mut connection = self
      .connection
      .lock()
      .map_err(|_| OperationalOutputError::Unavailable)?;
    let transaction = connection
      .transaction_with_behavior(TransactionBehavior::Immediate)
      .map_err(|_| OperationalOutputError::Unavailable)?;
    if let Some(remote) = remote {
      let existing: Option<(String, u64)> = transaction
        .query_row(
          "SELECT record_digest, local_sequence FROM woml_operational_remote_sequences
           WHERE dispatch_id = ?1 AND remote_sequence = ?2",
          params![remote.dispatch_id, remote.sequence],
          |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|_| OperationalOutputError::Corrupt)?;
      if let Some((digest, local_sequence)) = existing {
        if digest != remote.digest {
          return Err(OperationalOutputError::InvalidRecord(
            "the remote output replay conflicts with retained output",
          ));
        }
        transaction
          .commit()
          .map_err(|_| OperationalOutputError::Unavailable)?;
        return Ok((
          OperationalAppendOutcomeV1::Dropped {
            marker_sequence: local_sequence,
          },
          true,
        ));
      }
      let latest: u64 = transaction
        .query_row(
          "SELECT COALESCE(MAX(remote_sequence), 0) FROM woml_operational_remote_sequences
           WHERE dispatch_id = ?1",
          [remote.dispatch_id],
          |row| row.get(0),
        )
        .map_err(|_| OperationalOutputError::Corrupt)?;
      if remote.sequence != latest.saturating_add(1) {
        return Err(OperationalOutputError::InvalidRecord(
          "remote output records must be appended in sequence",
        ));
      }
    }
    let sequence = read_metadata_u64(&transaction, "next_sequence")?;
    if sequence == 0 || sequence >= i64::MAX as u64 {
      return Err(OperationalOutputError::Unavailable);
    }
    let existing_active: Option<bool> = transaction
      .query_row(
        "SELECT active FROM woml_operational_runs WHERE run_id = ?1",
        [&draft.identity.run_id],
        |row| row.get(0),
      )
      .optional()
      .map_err(|_| OperationalOutputError::Corrupt)?;
    if existing_active == Some(false) {
      return Err(OperationalOutputError::InvalidRecord(
        "the operational stream is already closed",
      ));
    }
    let mut truncated = false;
    if let OperationalBodyV1::Output {
      source, byte_count, ..
    } = &draft.body
    {
      let run_bytes: u64 = transaction
        .query_row(
          "SELECT COALESCE(retained_output_bytes, 0) FROM woml_operational_runs WHERE run_id = ?1",
          [&draft.identity.run_id],
          |row| row.get(0),
        )
        .optional()
        .map_err(|_| OperationalOutputError::Corrupt)?
        .unwrap_or(0);
      let attempt_bytes: u64 = transaction
        .query_row(
          "SELECT COALESCE(SUM(accounted_bytes), 0) FROM woml_operational_records
           WHERE run_id = ?1 AND node_id IS ?2 AND attempt_number IS ?3",
          params![
            draft.identity.run_id,
            draft.identity.node_id,
            draft.identity.attempt,
          ],
          |row| row.get(0),
        )
        .map_err(|_| OperationalOutputError::Corrupt)?;
      let store_bytes: u64 = transaction
        .query_row(
          "SELECT COALESCE(SUM(retained_output_bytes), 0) FROM woml_operational_runs",
          [],
          |row| row.get(0),
        )
        .map_err(|_| OperationalOutputError::Corrupt)?;
      let reason = if attempt_bytes.saturating_add(*byte_count) > self.limits.max_attempt_bytes {
        Some("inline_attempt_budget")
      } else if run_bytes.saturating_add(*byte_count) > self.limits.max_run_bytes {
        Some("inline_run_budget")
      } else if store_bytes.saturating_add(*byte_count) > self.limits.max_store_bytes {
        Some("output_store_budget")
      } else {
        None
      };
      if let Some(reason) = reason {
        let marker_sequence = transaction
          .query_row(
            "SELECT sequence FROM woml_operational_records
             WHERE run_id = ?1 AND node_id IS ?2 AND attempt_number IS ?3
               AND kind = 'truncated'
             ORDER BY sequence LIMIT 1",
            params![
              draft.identity.run_id,
              draft.identity.node_id,
              draft.identity.attempt,
            ],
            |row| row.get::<_, u64>(0),
          )
          .optional()
          .map_err(|_| OperationalOutputError::Corrupt)?;
        if let Some(marker_sequence) = marker_sequence {
          if let Some(remote) = remote {
            transaction
              .execute(
                "INSERT INTO woml_operational_remote_sequences(
                   dispatch_id, remote_sequence, record_digest, local_sequence, run_id
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                  remote.dispatch_id,
                  remote.sequence,
                  remote.digest,
                  marker_sequence,
                  draft.identity.run_id,
                ],
              )
              .map_err(|_| OperationalOutputError::Unavailable)?;
          }
          transaction
            .commit()
            .map_err(|_| OperationalOutputError::Unavailable)?;
          return Ok((
            OperationalAppendOutcomeV1::Dropped { marker_sequence },
            false,
          ));
        }
        draft.body = OperationalBodyV1::Truncated {
          source: *source,
          dropped_bytes: *byte_count,
          reason: reason.to_string(),
        };
        truncated = true;
      }
    }
    let record = OperationalRecordV1 {
      profile: OPERATIONAL_RECORD_PROFILE.to_string(),
      deployment_id: self.deployment_id.clone(),
      sequence,
      occurred_at: occurred_at.clone(),
      identity: draft.identity,
      body: draft.body,
    };
    let encoded =
      serde_json::to_string(&record).map_err(|_| OperationalOutputError::Unavailable)?;
    if encoded.len() > MAX_OPERATIONAL_PAGE_BYTES {
      return Err(OperationalOutputError::InvalidRecord(
        "encoded record is too large",
      ));
    }
    let source = record.body.source();
    transaction
      .execute(
        "INSERT INTO woml_operational_records(
           sequence, deployment_id, occurred_at, workflow_id, run_id, node_id, attempt_number,
           invocation_id, phase, kind, source, accounted_bytes, record_json,
           remote_dispatch_id, remote_sequence, remote_record_digest
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
          sequence,
          self.deployment_id,
          record.occurred_at,
          record.identity.workflow_id,
          record.identity.run_id,
          record.identity.node_id,
          record.identity.attempt,
          record.identity.invocation_id,
          record.identity.phase.as_str(),
          record.body.kind(),
          source,
          record.body.accounted_bytes(),
          encoded,
          remote.map(|value| value.dispatch_id),
          remote.map(|value| value.sequence),
          remote.map(|value| value.digest),
        ],
      )
      .map_err(|_| OperationalOutputError::Unavailable)?;
    if let Some(remote) = remote {
      transaction
        .execute(
          "INSERT INTO woml_operational_remote_sequences(
             dispatch_id, remote_sequence, record_digest, local_sequence, run_id
           ) VALUES (?1, ?2, ?3, ?4, ?5)",
          params![
            remote.dispatch_id,
            remote.sequence,
            remote.digest,
            sequence,
            record.identity.run_id,
          ],
        )
        .map_err(|_| OperationalOutputError::Unavailable)?;
    }
    update_run_accounting(&transaction, &record)?;
    update_progress_projection(&transaction, &record)?;
    transaction
      .execute(
        "UPDATE woml_operational_metadata SET value = ?1 WHERE key = 'next_sequence'",
        [(sequence + 1).to_string()],
      )
      .map_err(|_| OperationalOutputError::Unavailable)?;
    transaction
      .commit()
      .map_err(|_| OperationalOutputError::Unavailable)?;
    if let Some(reporter) = &self.reporter {
      reporter(record.sequence);
    }
    Ok((
      if truncated {
        OperationalAppendOutcomeV1::Truncated(record)
      } else {
        OperationalAppendOutcomeV1::Appended(record)
      },
      false,
    ))
  }

  /// Durably updates an existing latest-progress lane without adding another
  /// replay record when the lane is still inside the frozen 250 ms cadence.
  /// The caller must force-append the newest body before attempt closure.
  pub fn coalesce_progress_snapshot(
    &self,
    draft: &OperationalRecordDraftV1,
  ) -> Result<bool, OperationalOutputError> {
    validate_draft(draft)?;
    let OperationalBodyV1::Progress {
      progress_id,
      current,
      total,
      unit,
      message,
      status,
    } = &draft.body
    else {
      return Ok(false);
    };
    let occurred_at = now_string();
    let now =
      DateTime::parse_from_rfc3339(&occurred_at).map_err(|_| OperationalOutputError::Corrupt)?;
    let mut connection = self
      .connection
      .lock()
      .map_err(|_| OperationalOutputError::Unavailable)?;
    let transaction = connection
      .transaction_with_behavior(TransactionBehavior::Immediate)
      .map_err(|_| OperationalOutputError::Unavailable)?;
    let existing_active: Option<bool> = transaction
      .query_row(
        "SELECT active FROM woml_operational_runs WHERE run_id = ?1",
        [&draft.identity.run_id],
        |row| row.get(0),
      )
      .optional()
      .map_err(|_| OperationalOutputError::Corrupt)?;
    if existing_active == Some(false) {
      return Err(OperationalOutputError::InvalidRecord(
        "the operational stream is already closed",
      ));
    }
    let previous: Option<(String, u64)> = transaction
      .query_row(
        "SELECT occurred_at, sequence FROM woml_operational_progress
         WHERE run_id = ?1 AND node_key = ?2 AND attempt_key = ?3 AND progress_id = ?4",
        params![
          draft.identity.run_id,
          draft.identity.node_id.as_deref().unwrap_or(""),
          draft.identity.attempt.unwrap_or(0),
          progress_id,
        ],
        |row| Ok((row.get(0)?, row.get(1)?)),
      )
      .optional()
      .map_err(|_| OperationalOutputError::Corrupt)?;
    let within_cadence = previous
      .as_ref()
      .and_then(|(value, _)| DateTime::parse_from_rfc3339(value).ok())
      .is_some_and(|previous| {
        let elapsed = now.signed_duration_since(previous).num_milliseconds();
        (0..OPERATIONAL_PROGRESS_CADENCE_MS).contains(&elapsed)
      });
    if !within_cadence {
      return Ok(false);
    }
    let changed = transaction
      .execute(
        "UPDATE woml_operational_progress SET
           current = ?1, total = ?2, unit = ?3, message = ?4, status = ?5, occurred_at = ?6
         WHERE run_id = ?7 AND node_key = ?8 AND attempt_key = ?9 AND progress_id = ?10",
        params![
          current,
          total,
          unit,
          message,
          status.as_str(),
          occurred_at,
          draft.identity.run_id,
          draft.identity.node_id.as_deref().unwrap_or(""),
          draft.identity.attempt.unwrap_or(0),
          progress_id,
        ],
      )
      .map_err(|_| OperationalOutputError::Unavailable)?;
    if changed != 1 {
      return Err(OperationalOutputError::Corrupt);
    }
    transaction
      .commit()
      .map_err(|_| OperationalOutputError::Unavailable)?;
    if let (Some(reporter), Some((_, sequence))) = (&self.reporter, previous) {
      reporter(sequence);
    }
    Ok(true)
  }

  pub fn read_page(
    &self,
    after: u64,
    limit: usize,
  ) -> Result<OperationalPageV1, OperationalOutputError> {
    self.read_scoped_page(None, after, limit)
  }

  pub fn read_run_page(
    &self,
    run_id: &str,
    after: u64,
    limit: usize,
  ) -> Result<OperationalPageV1, OperationalOutputError> {
    validate_id(run_id)?;
    self.read_scoped_page(Some(run_id), after, limit)
  }

  fn read_scoped_page(
    &self,
    run_id: Option<&str>,
    after: u64,
    limit: usize,
  ) -> Result<OperationalPageV1, OperationalOutputError> {
    if after > i64::MAX as u64 || !(1..=MAX_OPERATIONAL_PAGE_RECORDS).contains(&limit) {
      return Err(OperationalOutputError::InvalidCursor);
    }
    let connection = self
      .connection
      .lock()
      .map_err(|_| OperationalOutputError::Unavailable)?;
    let pruned_through = match run_id {
      Some(run_id) => connection
        .query_row(
          "SELECT pruned_through FROM woml_operational_pruned_runs WHERE run_id = ?1",
          [run_id],
          |row| row.get::<_, u64>(0),
        )
        .optional()
        .map_err(|_| OperationalOutputError::Unavailable)?
        .unwrap_or(0),
      None => read_metadata_u64(&connection, "pruned_through")?,
    };
    let (earliest_available, latest_available) = sequence_bounds(&connection, run_id)?;
    let latest_sequence = latest_available
      .unwrap_or(pruned_through)
      .max(pruned_through);
    let gap = (after < pruned_through).then(|| OperationalGapV1 {
      requested_after: after,
      pruned_through,
      earliest_available,
    });
    let effective_after = if run_id.is_some() {
      after.max(pruned_through)
    } else {
      after
    };
    let mut records = Vec::with_capacity(limit.min(64));
    let mut page_bytes = 0usize;
    let sql = if run_id.is_some() {
      "SELECT record_json FROM woml_operational_records
       WHERE run_id = ?1 AND sequence > ?2 ORDER BY sequence LIMIT ?3"
    } else {
      "SELECT record_json FROM woml_operational_records
       WHERE (?1 IS NULL) AND sequence > ?2 ORDER BY sequence LIMIT ?3"
    };
    let mut statement = connection
      .prepare(sql)
      .map_err(|_| OperationalOutputError::Corrupt)?;
    let rows = statement
      .query_map(
        params![run_id, effective_after, limit.saturating_add(1)],
        |row| row.get::<_, String>(0),
      )
      .map_err(|_| OperationalOutputError::Corrupt)?;
    for encoded in rows {
      let encoded = encoded.map_err(|_| OperationalOutputError::Corrupt)?;
      if records.len() == limit
        || page_bytes.saturating_add(encoded.len()) > MAX_OPERATIONAL_PAGE_BYTES
      {
        break;
      }
      let record: OperationalRecordV1 =
        serde_json::from_str(&encoded).map_err(|_| OperationalOutputError::Corrupt)?;
      validate_persisted_record(&record, &self.deployment_id)?;
      page_bytes = page_bytes.saturating_add(encoded.len());
      records.push(record);
    }
    let next_after = records
      .last()
      .map(|record| record.sequence)
      .unwrap_or(effective_after);
    Ok(OperationalPageV1 {
      profile: OPERATIONAL_PAGE_PROFILE.to_string(),
      deployment_id: self.deployment_id.clone(),
      run_id: run_id.map(str::to_string),
      requested_after: after,
      next_after,
      earliest_available,
      latest_sequence,
      has_more: next_after < latest_sequence,
      gap,
      records,
    })
  }

  pub fn latest_progress(
    &self,
    run_id: &str,
  ) -> Result<Vec<OperationalProgressSnapshotV1>, OperationalOutputError> {
    validate_id(run_id)?;
    let connection = self
      .connection
      .lock()
      .map_err(|_| OperationalOutputError::Unavailable)?;
    let mut statement = connection
      .prepare(
        "SELECT run_id, node_id, attempt_number, progress_id, current,
                total, unit, message, status, sequence, occurred_at
         FROM woml_operational_progress WHERE run_id = ?1
         ORDER BY node_id, attempt_number, progress_id LIMIT 1000",
      )
      .map_err(|_| OperationalOutputError::Corrupt)?;
    let snapshots = statement
      .query_map([run_id], |row| {
        let status = match row.get::<_, String>(8)?.as_str() {
          "active" => OperationalProgressStatusV1::Active,
          "closed" => OperationalProgressStatusV1::Closed,
          _ => return Err(rusqlite::Error::InvalidQuery),
        };
        Ok(OperationalProgressSnapshotV1 {
          run_id: row.get(0)?,
          node_id: row.get(1)?,
          attempt: row.get(2)?,
          progress_id: row.get(3)?,
          current: row.get(4)?,
          total: row.get(5)?,
          unit: row.get(6)?,
          message: row.get(7)?,
          status,
          sequence: row.get(9)?,
          occurred_at: row.get(10)?,
        })
      })
      .map_err(|_| OperationalOutputError::Corrupt)?
      .collect::<Result<Vec<_>, _>>()
      .map_err(|_| OperationalOutputError::Corrupt)?;
    Ok(snapshots)
  }

  pub fn inspect(&self) -> Result<OperationalOutputStoreInspectionV1, OperationalOutputError> {
    let connection = self
      .connection
      .lock()
      .map_err(|_| OperationalOutputError::Unavailable)?;
    ensure_integrity(&connection)?;
    let (earliest_available, latest_available) = sequence_bounds(&connection, None)?;
    let record_count = connection
      .query_row("SELECT COUNT(*) FROM woml_operational_records", [], |row| {
        row.get(0)
      })
      .map_err(|_| OperationalOutputError::Corrupt)?;
    let retained_output_bytes = connection
      .query_row(
        "SELECT COALESCE(SUM(accounted_bytes), 0) FROM woml_operational_records",
        [],
        |row| row.get(0),
      )
      .map_err(|_| OperationalOutputError::Corrupt)?;
    let (active_runs, terminal_runs) = connection
      .query_row(
        "SELECT COALESCE(SUM(active), 0), COALESCE(SUM(1 - active), 0)
         FROM woml_operational_runs",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
      )
      .map_err(|_| OperationalOutputError::Corrupt)?;
    let truncated_streams = connection
      .query_row(
        "SELECT COUNT(DISTINCT run_id) FROM woml_operational_records WHERE kind = 'truncated'",
        [],
        |row| row.get(0),
      )
      .map_err(|_| OperationalOutputError::Corrupt)?;
    Ok(OperationalOutputStoreInspectionV1 {
      profile: "woml.operational-output-store/v1".to_string(),
      schema_version: OPERATIONAL_OUTPUT_STORE_SCHEMA_VERSION,
      deployment_id: self.deployment_id.clone(),
      next_sequence: read_metadata_u64(&connection, "next_sequence")?,
      earliest_available,
      latest_sequence: latest_available.unwrap_or(0),
      pruned_through: read_metadata_u64(&connection, "pruned_through")?,
      record_count,
      retained_output_bytes,
      active_runs,
      terminal_runs,
      truncated_streams,
      max_attempt_bytes: self.limits.max_attempt_bytes,
      max_run_bytes: self.limits.max_run_bytes,
      max_store_bytes: self.limits.max_store_bytes,
      retention_ms: self.limits.retention_ms,
      last_pruned_at: read_metadata_optional(&connection, "last_pruned_at")?,
    })
  }

  pub fn prune_terminal_before(
    &self,
    cutoff: DateTime<Utc>,
    max_runs: usize,
  ) -> Result<OperationalOutputPruneResultV1, OperationalOutputError> {
    if !(1..=1_000).contains(&max_runs) {
      return Err(OperationalOutputError::InvalidCursor);
    }
    let cutoff = cutoff.to_rfc3339_opts(SecondsFormat::Millis, true);
    let mut connection = self
      .connection
      .lock()
      .map_err(|_| OperationalOutputError::Unavailable)?;
    let transaction = connection
      .transaction_with_behavior(TransactionBehavior::Immediate)
      .map_err(|_| OperationalOutputError::Unavailable)?;
    let candidates = {
      let mut statement = transaction
        .prepare(
          "SELECT run_id, last_sequence, record_count, retained_output_bytes
           FROM woml_operational_runs
           WHERE active = 0 AND terminal_at < ?1
           ORDER BY terminal_at, run_id LIMIT ?2",
        )
        .map_err(|_| OperationalOutputError::Corrupt)?;
      let candidates = statement
        .query_map(params![cutoff, max_runs], |row| {
          Ok((
            row.get::<_, String>(0)?,
            row.get::<_, u64>(1)?,
            row.get::<_, u64>(2)?,
            row.get::<_, u64>(3)?,
          ))
        })
        .map_err(|_| OperationalOutputError::Corrupt)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| OperationalOutputError::Corrupt)?;
      candidates
    };
    let mut pruned_records = 0u64;
    let mut pruned_output_bytes = 0u64;
    let mut pruned_through = read_metadata_u64(&transaction, "pruned_through")?;
    for (run_id, last_sequence, record_count, output_bytes) in &candidates {
      transaction
        .execute(
          "INSERT INTO woml_operational_pruned_runs(run_id, pruned_through, pruned_at)
           VALUES (?1, ?2, ?3)
           ON CONFLICT(run_id) DO UPDATE SET
             pruned_through = MAX(pruned_through, excluded.pruned_through),
             pruned_at = excluded.pruned_at",
          params![run_id, last_sequence, now_string()],
        )
        .map_err(|_| OperationalOutputError::Unavailable)?;
      transaction
        .execute(
          "DELETE FROM woml_operational_progress WHERE run_id = ?1",
          [run_id],
        )
        .map_err(|_| OperationalOutputError::Unavailable)?;
      transaction
        .execute(
          "DELETE FROM woml_operational_remote_sequences WHERE run_id = ?1",
          [run_id],
        )
        .map_err(|_| OperationalOutputError::Unavailable)?;
      transaction
        .execute(
          "DELETE FROM woml_operational_records WHERE run_id = ?1",
          [run_id],
        )
        .map_err(|_| OperationalOutputError::Unavailable)?;
      transaction
        .execute(
          "DELETE FROM woml_operational_runs WHERE run_id = ?1",
          [run_id],
        )
        .map_err(|_| OperationalOutputError::Unavailable)?;
      pruned_records = pruned_records.saturating_add(*record_count);
      pruned_output_bytes = pruned_output_bytes.saturating_add(*output_bytes);
      pruned_through = pruned_through.max(*last_sequence);
    }
    transaction
      .execute(
        "UPDATE woml_operational_metadata SET value = ?1 WHERE key = 'pruned_through'",
        [pruned_through.to_string()],
      )
      .map_err(|_| OperationalOutputError::Unavailable)?;
    if !candidates.is_empty() {
      transaction
        .execute(
          "INSERT INTO woml_operational_metadata(key, value) VALUES ('last_pruned_at', ?1)
           ON CONFLICT(key) DO UPDATE SET value = excluded.value",
          [now_string()],
        )
        .map_err(|_| OperationalOutputError::Unavailable)?;
    }
    transaction
      .commit()
      .map_err(|_| OperationalOutputError::Unavailable)?;
    Ok(OperationalOutputPruneResultV1 {
      profile: "woml.operational-output-prune/v1".to_string(),
      pruned_runs: candidates.len() as u64,
      pruned_records,
      pruned_output_bytes,
      pruned_through,
    })
  }

  pub fn plan_prune_terminal_before(
    &self,
    cutoff: DateTime<Utc>,
    max_runs: usize,
  ) -> Result<OperationalOutputPrunePlanV1, OperationalOutputError> {
    if !(1..=1_000).contains(&max_runs) {
      return Err(OperationalOutputError::InvalidCursor);
    }
    let cutoff = cutoff.to_rfc3339_opts(SecondsFormat::Millis, true);
    let connection = self
      .connection
      .lock()
      .map_err(|_| OperationalOutputError::Unavailable)?;
    let (eligible_runs, eligible_records, eligible_output_bytes) = connection
      .query_row(
        "SELECT COUNT(*), COALESCE(SUM(record_count), 0),
                COALESCE(SUM(retained_output_bytes), 0)
         FROM (
           SELECT record_count, retained_output_bytes
           FROM woml_operational_runs
           WHERE active = 0 AND terminal_at < ?1
           ORDER BY terminal_at, run_id LIMIT ?2
         )",
        params![cutoff, max_runs],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
      )
      .map_err(|_| OperationalOutputError::Corrupt)?;
    Ok(OperationalOutputPrunePlanV1 {
      profile: "woml.operational-output-prune-plan/v1".to_string(),
      eligible_runs,
      eligible_records,
      eligible_output_bytes,
    })
  }

  pub fn recover_interrupted_streams(
    &self,
  ) -> Result<Vec<OperationalRecordV1>, OperationalOutputError> {
    self.recover_interrupted_streams_scoped(None)
  }

  pub fn recover_interrupted_run(
    &self,
    run_id: &str,
  ) -> Result<Vec<OperationalRecordV1>, OperationalOutputError> {
    validate_id(run_id)?;
    self.recover_interrupted_streams_scoped(Some(run_id))
  }

  fn recover_interrupted_streams_scoped(
    &self,
    run_id: Option<&str>,
  ) -> Result<Vec<OperationalRecordV1>, OperationalOutputError> {
    let lanes = {
      let connection = self
        .connection
        .lock()
        .map_err(|_| OperationalOutputError::Unavailable)?;
      let mut statement = connection
        .prepare(
          "SELECT r.workflow_id, r.run_id, r.node_id, r.attempt_number, r.invocation_id, r.phase
           FROM woml_operational_records r
           JOIN woml_operational_runs runs ON runs.run_id = r.run_id AND runs.active = 1
           WHERE r.node_id IS NOT NULL AND r.attempt_number IS NOT NULL
             AND (?1 IS NULL OR r.run_id = ?1)
           GROUP BY r.run_id, r.node_id, r.attempt_number
           HAVING SUM(CASE WHEN r.kind = 'closed' THEN 1 ELSE 0 END) = 0
           ORDER BY MIN(r.sequence) LIMIT 1000",
        )
        .map_err(|_| OperationalOutputError::Corrupt)?;
      let lanes = statement
        .query_map([run_id], |row| {
          Ok(OperationalIdentityV1 {
            workflow_id: row.get(0)?,
            run_id: row.get(1)?,
            node_id: row.get(2)?,
            attempt: row.get(3)?,
            invocation_id: row.get(4)?,
            phase: if row.get::<_, String>(5)? == "corrective" {
              OperationalExecutionPhaseV1::Corrective
            } else {
              OperationalExecutionPhaseV1::Forward
            },
          })
        })
        .map_err(|_| OperationalOutputError::Corrupt)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| OperationalOutputError::Corrupt)?;
      lanes
    };
    lanes
      .into_iter()
      .map(|identity| {
        self.append(OperationalRecordDraftV1 {
          identity,
          body: OperationalBodyV1::Closed {
            status: OperationalCloseStatusV1::Interrupted,
            reason_code: Some("WOML_OUTPUT_RECOVERED_INTERRUPTION".to_string()),
          },
        })
      })
      .collect()
  }
}

/// Defers filesystem and SQLite activation until a workflow actually emits
/// operational output or progress.
pub struct LazyOperationalOutputStore {
  path: PathBuf,
  deployment_id: String,
  active: Mutex<Option<Arc<OperationalOutputStore>>>,
  reporter: Option<OperationalOutputReporter>,
}

impl std::fmt::Debug for LazyOperationalOutputStore {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    formatter
      .debug_struct("LazyOperationalOutputStore")
      .field("path", &self.path)
      .field("deployment_id", &self.deployment_id)
      .field("active", &self.is_active())
      .finish_non_exhaustive()
  }
}

impl LazyOperationalOutputStore {
  pub fn new(path: impl Into<PathBuf>, deployment_id: impl Into<String>) -> Self {
    Self {
      path: path.into(),
      deployment_id: deployment_id.into(),
      active: Mutex::new(None),
      reporter: None,
    }
  }

  pub fn with_reporter(mut self, reporter: Option<OperationalOutputReporter>) -> Self {
    self.reporter = reporter;
    self
  }

  pub fn is_active(&self) -> bool {
    self
      .active
      .lock()
      .map(|active| active.is_some())
      .unwrap_or(false)
  }

  pub fn path(&self) -> &Path {
    &self.path
  }

  pub fn active_store(&self) -> Option<Arc<OperationalOutputStore>> {
    self.active.lock().ok()?.clone()
  }

  pub fn activate_if_exists(
    &self,
  ) -> Result<Option<Arc<OperationalOutputStore>>, OperationalOutputError> {
    if !self.path.exists() {
      return Ok(None);
    }
    self.activate().map(Some)
  }

  pub fn activate(&self) -> Result<Arc<OperationalOutputStore>, OperationalOutputError> {
    let mut active = self
      .active
      .lock()
      .map_err(|_| OperationalOutputError::Unavailable)?;
    if let Some(store) = active.as_ref() {
      return Ok(Arc::clone(store));
    }
    let store = Arc::new(OperationalOutputStore::open_with_reporter(
      &self.path,
      self.deployment_id.clone(),
      self.reporter.clone(),
    )?);
    *active = Some(Arc::clone(&store));
    Ok(store)
  }
}

/// Derives the private operational-output authority associated with one
/// durable state database. The default `.woml/state.sqlite` maps to
/// `.woml/output.sqlite`; custom names remain adjacent and cannot collide with
/// the authoritative state file.
pub fn operational_output_store_for_state(
  state_path: &Path,
) -> Result<LazyOperationalOutputStore, OperationalOutputError> {
  let file_name = state_path
    .file_name()
    .and_then(|value| value.to_str())
    .ok_or(OperationalOutputError::UnsafePath)?;
  let output_name = if file_name == "state.sqlite" {
    "output.sqlite".to_string()
  } else {
    let stem = state_path
      .file_stem()
      .and_then(|value| value.to_str())
      .ok_or(OperationalOutputError::UnsafePath)?;
    format!("{stem}.output.sqlite")
  };
  let output_path = state_path.with_file_name(output_name);
  if output_path == state_path {
    return Err(OperationalOutputError::UnsafePath);
  }
  let identity_source = state_path
    .canonicalize()
    .unwrap_or_else(|_| state_path.to_path_buf());
  let digest = Sha256::digest(identity_source.to_string_lossy().as_bytes());
  Ok(LazyOperationalOutputStore::new(
    output_path,
    format!("deployment_{:x}", digest),
  ))
}

fn validate_draft(draft: &OperationalRecordDraftV1) -> Result<(), OperationalOutputError> {
  validate_id(&draft.identity.workflow_id)?;
  validate_id(&draft.identity.run_id)?;
  if let Some(node_id) = &draft.identity.node_id {
    validate_id(node_id)?;
  }
  if let Some(invocation_id) = &draft.identity.invocation_id {
    validate_id(invocation_id)?;
  }
  if draft.identity.attempt == Some(0) {
    return Err(OperationalOutputError::InvalidRecord(
      "attempt must be positive",
    ));
  }
  match &draft.body {
    OperationalBodyV1::Output {
      text, byte_count, ..
    } => {
      if text.len() > MAX_OPERATIONAL_TEXT_BYTES || *byte_count > MAX_OPERATIONAL_CHUNK_BYTES as u64
      {
        return Err(OperationalOutputError::InvalidRecord(
          "output chunk exceeds its budget",
        ));
      }
    }
    OperationalBodyV1::Progress {
      progress_id,
      current,
      total,
      unit,
      message,
      ..
    } => {
      validate_id(progress_id)?;
      if progress_id.len() > 128 {
        return Err(OperationalOutputError::InvalidRecord(
          "progress ID exceeds its budget",
        ));
      }
      if current.is_none() && message.is_none()
        || current.is_some_and(|current| !current.is_finite() || current.is_sign_negative())
        || total.is_some_and(|total| {
          !total.is_finite() || total <= 0.0 || current.is_some_and(|current| current > total)
        })
      {
        return Err(OperationalOutputError::InvalidRecord(
          "progress values are invalid",
        ));
      }
      if let Some(unit) = unit {
        if current.is_none() {
          return Err(OperationalOutputError::InvalidRecord(
            "progress units require a current value",
          ));
        }
        validate_text(unit, 32, false)?;
      }
      if let Some(message) = message {
        validate_text(message, 512, false)?;
      }
    }
    OperationalBodyV1::Truncated {
      dropped_bytes,
      reason,
      ..
    } => {
      if *dropped_bytes == 0 {
        return Err(OperationalOutputError::InvalidRecord(
          "truncation must drop bytes",
        ));
      }
      validate_text(reason, MAX_REASON_BYTES, false)?;
    }
    OperationalBodyV1::Artifact {
      artifact_id,
      name,
      sha256,
      content_type,
      ..
    } => {
      validate_id(artifact_id)?;
      validate_text(name, 256, false)?;
      if sha256.len() != 64
        || !sha256
          .bytes()
          .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
      {
        return Err(OperationalOutputError::InvalidRecord(
          "artifact digest is invalid",
        ));
      }
      validate_text(content_type, 255, false)?;
    }
    OperationalBodyV1::Closed { reason_code, .. } => {
      if let Some(reason_code) = reason_code {
        if reason_code.len() > MAX_REASON_BYTES
          || !reason_code.starts_with("WOML_")
          || !reason_code
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        {
          return Err(OperationalOutputError::InvalidRecord(
            "closure reason is invalid",
          ));
        }
      }
    }
    OperationalBodyV1::Gap { .. } => {
      return Err(OperationalOutputError::InvalidRecord(
        "gap records are synthesized and cannot be appended",
      ));
    }
  }
  Ok(())
}

fn validate_limits(limits: OperationalOutputLimitsV1) -> Result<(), OperationalOutputError> {
  if limits.max_attempt_bytes == 0
    || limits.max_attempt_bytes > MAX_OPERATIONAL_INLINE_RUN_BYTES
    || limits.max_run_bytes < limits.max_attempt_bytes
    || limits.max_run_bytes > MAX_OPERATIONAL_INLINE_RUN_BYTES
    || limits.max_store_bytes < limits.max_run_bytes
    || limits.retention_ms == 0
  {
    return Err(OperationalOutputError::InvalidRecord(
      "operational output limits are invalid",
    ));
  }
  Ok(())
}

fn validate_persisted_record(
  record: &OperationalRecordV1,
  deployment_id: &str,
) -> Result<(), OperationalOutputError> {
  if record.profile != OPERATIONAL_RECORD_PROFILE
    || record.deployment_id != deployment_id
    || record.sequence == 0
  {
    return Err(OperationalOutputError::Corrupt);
  }
  validate_draft(&OperationalRecordDraftV1 {
    identity: record.identity.clone(),
    body: record.body.clone(),
  })
  .map_err(|_| OperationalOutputError::Corrupt)
}

fn validate_id(value: &str) -> Result<(), OperationalOutputError> {
  if value.is_empty()
    || value.len() > MAX_ID_BYTES
    || value
      .bytes()
      .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
  {
    return Err(OperationalOutputError::InvalidRecord("identity is invalid"));
  }
  Ok(())
}

fn validate_text(
  value: &str,
  max_bytes: usize,
  allow_empty: bool,
) -> Result<(), OperationalOutputError> {
  if (!allow_empty && value.is_empty()) || value.len() > max_bytes || value.contains('\0') {
    return Err(OperationalOutputError::InvalidRecord(
      "text field is invalid",
    ));
  }
  Ok(())
}

fn prepare_private_store_path(path: &Path) -> Result<PathBuf, OperationalOutputError> {
  let file_name = path.file_name().ok_or(OperationalOutputError::UnsafePath)?;
  if file_name.is_empty() || path.as_os_str().is_empty() {
    return Err(OperationalOutputError::UnsafePath);
  }
  let parent = path.parent().unwrap_or_else(|| Path::new("."));
  let created_parent = !parent.exists();
  if created_parent {
    fs::create_dir_all(parent).map_err(|_| OperationalOutputError::UnsafePath)?;
  }
  let parent_metadata =
    fs::symlink_metadata(parent).map_err(|_| OperationalOutputError::UnsafePath)?;
  if !parent_metadata.is_dir()
    || parent_metadata.file_type().is_symlink()
    || is_windows_reparse(&parent_metadata)
  {
    return Err(OperationalOutputError::UnsafePath);
  }
  #[cfg(unix)]
  if created_parent {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
      .map_err(|_| OperationalOutputError::UnsafePath)?;
  }
  match fs::symlink_metadata(path) {
    Ok(metadata) => {
      if !metadata.is_file() || metadata.file_type().is_symlink() || is_windows_reparse(&metadata) {
        return Err(OperationalOutputError::UnsafePath);
      }
      #[cfg(unix)]
      {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
          return Err(OperationalOutputError::UnsafePath);
        }
      }
    }
    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
      let mut options = OpenOptions::new();
      options.create_new(true).read(true).write(true);
      #[cfg(unix)]
      {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
      }
      options
        .open(path)
        .map_err(|_| OperationalOutputError::UnsafePath)?;
    }
    Err(_) => return Err(OperationalOutputError::UnsafePath),
  }
  harden_file_permissions(path)?;
  Ok(path.to_path_buf())
}

fn initialize_schema(
  connection: &Connection,
  deployment_id: &str,
) -> Result<(), OperationalOutputError> {
  connection
    .execute_batch(
      "CREATE TABLE IF NOT EXISTS woml_operational_metadata(
         key TEXT PRIMARY KEY NOT NULL,
         value TEXT NOT NULL
       );
       INSERT OR IGNORE INTO woml_operational_metadata(key, value) VALUES ('schema_version', '1');
       INSERT OR IGNORE INTO woml_operational_metadata(key, value) VALUES ('next_sequence', '1');
       INSERT OR IGNORE INTO woml_operational_metadata(key, value) VALUES ('pruned_through', '0');
       CREATE TABLE IF NOT EXISTS woml_operational_records(
         sequence INTEGER PRIMARY KEY CHECK(sequence > 0),
         deployment_id TEXT NOT NULL,
         occurred_at TEXT NOT NULL,
         workflow_id TEXT NOT NULL,
         run_id TEXT NOT NULL,
         node_id TEXT,
         attempt_number INTEGER CHECK(attempt_number IS NULL OR attempt_number > 0),
         invocation_id TEXT,
         phase TEXT NOT NULL DEFAULT 'forward' CHECK(phase IN ('forward','corrective')),
         kind TEXT NOT NULL CHECK(kind IN ('output','progress','truncated','artifact','closed')),
         source TEXT CHECK(source IS NULL OR source IN ('stdout','stderr','script','runtime')),
         accounted_bytes INTEGER NOT NULL CHECK(accounted_bytes >= 0),
         record_json TEXT NOT NULL,
         remote_dispatch_id TEXT,
         remote_sequence INTEGER CHECK(remote_sequence IS NULL OR remote_sequence > 0),
         remote_record_digest TEXT,
         CHECK (
           (remote_dispatch_id IS NULL AND remote_sequence IS NULL AND remote_record_digest IS NULL)
           OR
           (remote_dispatch_id IS NOT NULL AND remote_sequence IS NOT NULL AND remote_record_digest IS NOT NULL)
         )
       );
       CREATE INDEX IF NOT EXISTS woml_operational_records_deployment ON woml_operational_records(deployment_id, sequence);
       CREATE INDEX IF NOT EXISTS woml_operational_records_workflow ON woml_operational_records(workflow_id, sequence);
       CREATE INDEX IF NOT EXISTS woml_operational_records_run ON woml_operational_records(run_id, sequence);
       CREATE INDEX IF NOT EXISTS woml_operational_records_node ON woml_operational_records(node_id, sequence);
       CREATE INDEX IF NOT EXISTS woml_operational_records_attempt ON woml_operational_records(run_id, node_id, attempt_number, sequence);
       CREATE INDEX IF NOT EXISTS woml_operational_records_invocation ON woml_operational_records(invocation_id, sequence);
       CREATE INDEX IF NOT EXISTS woml_operational_records_source ON woml_operational_records(source, sequence);
       CREATE INDEX IF NOT EXISTS woml_operational_records_kind ON woml_operational_records(kind, sequence);
       CREATE INDEX IF NOT EXISTS woml_operational_records_time ON woml_operational_records(occurred_at, sequence);
       CREATE TABLE IF NOT EXISTS woml_operational_remote_sequences(
         dispatch_id TEXT NOT NULL,
         remote_sequence INTEGER NOT NULL CHECK(remote_sequence > 0),
         record_digest TEXT NOT NULL,
         local_sequence INTEGER NOT NULL CHECK(local_sequence > 0),
         run_id TEXT NOT NULL,
         PRIMARY KEY(dispatch_id, remote_sequence)
       );
       CREATE INDEX IF NOT EXISTS woml_operational_remote_sequences_run
         ON woml_operational_remote_sequences(run_id);
       CREATE TABLE IF NOT EXISTS woml_operational_progress(
         run_id TEXT NOT NULL,
         node_key TEXT NOT NULL,
         node_id TEXT,
         attempt_key INTEGER NOT NULL,
         attempt_number INTEGER,
         progress_id TEXT NOT NULL,
         current REAL,
         total REAL,
         unit TEXT,
         message TEXT,
         status TEXT NOT NULL CHECK(status IN ('active','closed')),
         sequence INTEGER NOT NULL REFERENCES woml_operational_records(sequence) ON DELETE CASCADE,
         occurred_at TEXT NOT NULL,
         PRIMARY KEY(run_id, node_key, attempt_key, progress_id)
       );
       CREATE TABLE IF NOT EXISTS woml_operational_runs(
         run_id TEXT PRIMARY KEY NOT NULL,
         workflow_id TEXT NOT NULL,
         first_sequence INTEGER NOT NULL,
         last_sequence INTEGER NOT NULL,
         record_count INTEGER NOT NULL CHECK(record_count >= 0),
         retained_output_bytes INTEGER NOT NULL CHECK(retained_output_bytes >= 0),
         active INTEGER NOT NULL CHECK(active IN (0,1)),
         updated_at TEXT NOT NULL,
         terminal_at TEXT
       );
       CREATE INDEX IF NOT EXISTS woml_operational_runs_terminal ON woml_operational_runs(active, terminal_at, run_id);
       CREATE TABLE IF NOT EXISTS woml_operational_pruned_runs(
         run_id TEXT PRIMARY KEY NOT NULL,
         pruned_through INTEGER NOT NULL CHECK(pruned_through > 0),
         pruned_at TEXT NOT NULL
       );",
    )
    .map_err(|_| OperationalOutputError::Unavailable)?;
  let has_phase = connection
    .prepare("PRAGMA table_info(woml_operational_records)")
    .and_then(|mut statement| {
      let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
      Ok(columns.iter().any(|column| column == "phase"))
    })
    .map_err(|_| OperationalOutputError::Corrupt)?;
  if !has_phase {
    connection
      .execute(
        "ALTER TABLE woml_operational_records ADD COLUMN phase TEXT NOT NULL DEFAULT 'forward' CHECK(phase IN ('forward','corrective'))",
        [],
      )
      .map_err(|_| OperationalOutputError::Unavailable)?;
  }
  let columns = connection
    .prepare("PRAGMA table_info(woml_operational_records)")
    .and_then(|mut statement| {
      statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()
    })
    .map_err(|_| OperationalOutputError::Corrupt)?;
  if !columns.iter().any(|column| column == "remote_dispatch_id") {
    connection
      .execute_batch(
        "ALTER TABLE woml_operational_records ADD COLUMN remote_dispatch_id TEXT;
         ALTER TABLE woml_operational_records ADD COLUMN remote_sequence INTEGER CHECK(remote_sequence IS NULL OR remote_sequence > 0);
         ALTER TABLE woml_operational_records ADD COLUMN remote_record_digest TEXT;",
      )
      .map_err(|_| OperationalOutputError::Unavailable)?;
  }
  connection
    .execute(
      "CREATE INDEX IF NOT EXISTS woml_operational_records_phase ON woml_operational_records(phase, sequence)",
      [],
    )
    .map_err(|_| OperationalOutputError::Unavailable)?;
  connection
    .execute(
      "CREATE UNIQUE INDEX IF NOT EXISTS woml_operational_records_remote_sequence
       ON woml_operational_records(remote_dispatch_id, remote_sequence)
       WHERE remote_dispatch_id IS NOT NULL",
      [],
    )
    .map_err(|_| OperationalOutputError::Unavailable)?;
  connection
    .execute(
      "INSERT OR IGNORE INTO woml_operational_metadata(key, value) VALUES ('deployment_id', ?1)",
      [deployment_id],
    )
    .map_err(|_| OperationalOutputError::Unavailable)?;
  Ok(())
}

fn validate_schema(
  connection: &Connection,
  deployment_id: &str,
) -> Result<(), OperationalOutputError> {
  let version: Option<String> = connection
    .query_row(
      "SELECT value FROM woml_operational_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )
    .optional()
    .map_err(|_| OperationalOutputError::Corrupt)?;
  match version.as_deref() {
    Some("1") => {}
    Some(version) => {
      return Err(OperationalOutputError::UnsupportedVersion(
        version.to_string(),
      ))
    }
    None => return Err(OperationalOutputError::Corrupt),
  }
  let stored_deployment: String = connection
    .query_row(
      "SELECT value FROM woml_operational_metadata WHERE key = 'deployment_id'",
      [],
      |row| row.get(0),
    )
    .map_err(|_| OperationalOutputError::Corrupt)?;
  if stored_deployment != deployment_id {
    return Err(OperationalOutputError::DeploymentMismatch);
  }
  for table in [
    "woml_operational_records",
    "woml_operational_remote_sequences",
    "woml_operational_progress",
    "woml_operational_runs",
    "woml_operational_pruned_runs",
  ] {
    let present: bool = connection
      .query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get(0),
      )
      .map_err(|_| OperationalOutputError::Corrupt)?;
    if !present {
      return Err(OperationalOutputError::Corrupt);
    }
  }
  let next_sequence = read_metadata_u64(connection, "next_sequence")?;
  if next_sequence == 0 || next_sequence > i64::MAX as u64 {
    return Err(OperationalOutputError::Corrupt);
  }
  Ok(())
}

fn ensure_integrity(connection: &Connection) -> Result<(), OperationalOutputError> {
  let result: String = connection
    .query_row("PRAGMA quick_check", [], |row| row.get(0))
    .map_err(|_| OperationalOutputError::Corrupt)?;
  if result != "ok" {
    return Err(OperationalOutputError::Corrupt);
  }
  Ok(())
}

fn read_metadata_u64(connection: &Connection, key: &str) -> Result<u64, OperationalOutputError> {
  let value: String = connection
    .query_row(
      "SELECT value FROM woml_operational_metadata WHERE key = ?1",
      [key],
      |row| row.get(0),
    )
    .map_err(|_| OperationalOutputError::Corrupt)?;
  value.parse().map_err(|_| OperationalOutputError::Corrupt)
}

fn valid_remote_dispatch_id(value: &str) -> bool {
  value.len() == 73
    && value.starts_with("dispatch_")
    && value[9..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_sha256(value: &str) -> bool {
  value.len() == 71
    && value.starts_with("sha256:")
    && value[7..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn read_metadata_optional(
  connection: &Connection,
  key: &str,
) -> Result<Option<String>, OperationalOutputError> {
  connection
    .query_row(
      "SELECT value FROM woml_operational_metadata WHERE key = ?1",
      [key],
      |row| row.get(0),
    )
    .optional()
    .map_err(|_| OperationalOutputError::Corrupt)
}

fn sequence_bounds(
  connection: &Connection,
  run_id: Option<&str>,
) -> Result<(Option<u64>, Option<u64>), OperationalOutputError> {
  let sql = if run_id.is_some() {
    "SELECT MIN(sequence), MAX(sequence) FROM woml_operational_records WHERE run_id = ?1"
  } else {
    "SELECT MIN(sequence), MAX(sequence) FROM woml_operational_records WHERE ?1 IS NULL"
  };
  connection
    .query_row(sql, [run_id], |row| Ok((row.get(0)?, row.get(1)?)))
    .map_err(|_| OperationalOutputError::Corrupt)
}

fn update_run_accounting(
  transaction: &rusqlite::Transaction<'_>,
  record: &OperationalRecordV1,
) -> Result<(), OperationalOutputError> {
  // A node-scoped closure ends one attempt stream. Only a run-scoped closure
  // makes the complete operational run terminal.
  let (active, terminal_at) = match (&record.body, &record.identity.node_id) {
    (OperationalBodyV1::Closed { .. }, None) => (0, Some(record.occurred_at.as_str())),
    _ => (1, None),
  };
  transaction
    .execute(
      "INSERT INTO woml_operational_runs(
         run_id, workflow_id, first_sequence, last_sequence, record_count,
         retained_output_bytes, active, updated_at, terminal_at
       ) VALUES (?1, ?2, ?3, ?3, 1, ?4, ?5, ?6, ?7)
       ON CONFLICT(run_id) DO UPDATE SET
         last_sequence = excluded.last_sequence,
         record_count = woml_operational_runs.record_count + 1,
         retained_output_bytes = woml_operational_runs.retained_output_bytes + excluded.retained_output_bytes,
         active = excluded.active,
         updated_at = excluded.updated_at,
         terminal_at = COALESCE(excluded.terminal_at, woml_operational_runs.terminal_at)",
      params![
        record.identity.run_id,
        record.identity.workflow_id,
        record.sequence,
        record.body.accounted_bytes(),
        active,
        record.occurred_at,
        terminal_at,
      ],
    )
    .map_err(|_| OperationalOutputError::Unavailable)?;
  if matches!(record.body, OperationalBodyV1::Closed { .. }) {
    match (&record.identity.node_id, record.identity.attempt) {
      (Some(node_id), Some(attempt)) => transaction.execute(
        "UPDATE woml_operational_progress SET status = 'closed'
         WHERE run_id = ?1 AND node_key = ?2 AND attempt_key = ?3",
        params![record.identity.run_id, node_id, attempt],
      ),
      _ => transaction.execute(
        "UPDATE woml_operational_progress SET status = 'closed' WHERE run_id = ?1",
        [&record.identity.run_id],
      ),
    }
    .map_err(|_| OperationalOutputError::Unavailable)?;
  }
  Ok(())
}

fn update_progress_projection(
  transaction: &rusqlite::Transaction<'_>,
  record: &OperationalRecordV1,
) -> Result<(), OperationalOutputError> {
  let OperationalBodyV1::Progress {
    progress_id,
    current,
    total,
    unit,
    message,
    status,
  } = &record.body
  else {
    return Ok(());
  };
  transaction
    .execute(
      "INSERT INTO woml_operational_progress(
         run_id, node_key, node_id, attempt_key, attempt_number, progress_id,
         current, total, unit, message, status, sequence, occurred_at
       ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
       ON CONFLICT(run_id, node_key, attempt_key, progress_id) DO UPDATE SET
         current = excluded.current,
         total = excluded.total,
         unit = excluded.unit,
         message = excluded.message,
         status = excluded.status,
         sequence = excluded.sequence,
         occurred_at = excluded.occurred_at",
      params![
        record.identity.run_id,
        record.identity.node_id.as_deref().unwrap_or(""),
        record.identity.node_id,
        record.identity.attempt.unwrap_or(0),
        record.identity.attempt,
        progress_id,
        current,
        total,
        unit,
        message,
        status.as_str(),
        record.sequence,
        record.occurred_at,
      ],
    )
    .map_err(|_| OperationalOutputError::Unavailable)?;
  Ok(())
}

fn now_string() -> String {
  Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(unix)]
fn harden_file_permissions(path: &Path) -> Result<(), OperationalOutputError> {
  use std::os::unix::fs::PermissionsExt;
  fs::set_permissions(path, fs::Permissions::from_mode(0o600))
    .map_err(|_| OperationalOutputError::UnsafePath)
}

#[cfg(not(unix))]
fn harden_file_permissions(_path: &Path) -> Result<(), OperationalOutputError> {
  Ok(())
}

#[cfg(windows)]
fn is_windows_reparse(metadata: &fs::Metadata) -> bool {
  use std::os::windows::fs::MetadataExt;
  metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn is_windows_reparse(_metadata: &fs::Metadata) -> bool {
  false
}
