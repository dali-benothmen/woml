use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Utc};
use getrandom::getrandom;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::Serialize;
use serde_json::{Map, Value};
use serde_json_canonicalizer::to_vec as canonical_json;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use thiserror::Error;
use uuid::Uuid;

use crate::engine::{
  ready_node_ids_for_projection, ready_node_ids_for_projection_at, step_effect_idempotency_key,
  validate_event_history_against_definition, validate_payload_against_definition,
};
use crate::event::{
  is_definition_hash, ApprovalDecision, ApprovalDecisionSource, ApprovalFailure,
  ApprovalRequestedData, ApprovalResolution, ApprovalResolvedData, ApprovalTimeoutPolicy,
  NotificationDeliveryAttemptStartedData, NotificationDeliveryFailedData,
  NotificationDeliveryRequestedData, NotificationDeliverySucceededData,
  NotificationMessageUpdateAttemptStartedData, NotificationMessageUpdateFailedData,
  NotificationMessageUpdatedData, NotificationRunFailure, NotificationSafeFailure,
  OperationFailedData, ParallelFailure, ParallelFailurePolicy, ParallelGroupCompletedData,
  ParallelGroupOutcome, ProviderMessageIdentity, RunFailedData, RunFailedDataV1, RunFailedDataV2,
  RunFailedDataV3, RunFailedDataV4, RunFailedDataV5, RunStartedData, RunSucceededData,
  StepAttemptFailedData, StepRetryScheduledData,
};
use crate::projection::{
  ApprovalRequestStatus, AttemptStatus, NotificationDeliveryStatus,
  NotificationMessageUpdateStatus, ParallelGroupStatus,
};
use crate::{
  fold_events, run_event_schema_version_for_model, AttemptFailure, AttemptFailureKind,
  CompiledWorkflowDefinition, FoldError, ModelValidationError, RunEvent, RunEventPayload,
  RunProjection, RunStatus, RUN_EVENT_SCHEMA_VERSION_V1, RUN_EVENT_SCHEMA_VERSION_V2,
  RUN_EVENT_SCHEMA_VERSION_V3, RUN_EVENT_SCHEMA_VERSION_V4, RUN_EVENT_SCHEMA_VERSION_V5,
  RUN_EVENT_SCHEMA_VERSION_V6, RUN_EVENT_SCHEMA_VERSION_V7, RUN_EVENT_SCHEMA_VERSION_V8,
};

pub const DURABLE_STORE_SCHEMA_VERSION: u32 = 6;
const STORE_SCHEMA_VERSION_V1: &str = "1";
const STORE_SCHEMA_VERSION_V2: &str = "2";
const STORE_SCHEMA_VERSION_V3: &str = "3";
const STORE_SCHEMA_VERSION_V4: &str = "4";
const STORE_SCHEMA_VERSION_V5: &str = "5";
const STORE_SCHEMA_VERSION_V6: &str = "6";
const DEFAULT_APPROVAL_CREDENTIAL_LIFETIME_HOURS: i64 = 24;

const CREATE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS woml_store_metadata (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS woml_definitions (
  definition_hash TEXT PRIMARY KEY,
  workflow_id TEXT NOT NULL,
  schema_version INTEGER NOT NULL,
  model_json TEXT NOT NULL,
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS woml_runs (
  run_id TEXT PRIMARY KEY,
  workflow_id TEXT NOT NULL,
  definition_hash TEXT NOT NULL,
  created_at TEXT NOT NULL,
  FOREIGN KEY (definition_hash) REFERENCES woml_definitions(definition_hash)
);

CREATE TABLE IF NOT EXISTS woml_run_events (
  run_id TEXT NOT NULL,
  sequence INTEGER NOT NULL CHECK (sequence >= 1),
  event_id TEXT NOT NULL UNIQUE,
  event_schema_version INTEGER NOT NULL,
  event_json TEXT NOT NULL,
  PRIMARY KEY (run_id, sequence),
  FOREIGN KEY (run_id) REFERENCES woml_runs(run_id)
);

CREATE INDEX IF NOT EXISTS woml_run_events_event_id
  ON woml_run_events(event_id);

CREATE TRIGGER IF NOT EXISTS woml_definitions_no_update
BEFORE UPDATE ON woml_definitions
BEGIN
  SELECT RAISE(ABORT, 'WOML compiled definitions are immutable');
END;

CREATE TRIGGER IF NOT EXISTS woml_definitions_no_delete
BEFORE DELETE ON woml_definitions
BEGIN
  SELECT RAISE(ABORT, 'WOML compiled definitions are immutable');
END;

CREATE TRIGGER IF NOT EXISTS woml_runs_no_update
BEFORE UPDATE ON woml_runs
BEGIN
  SELECT RAISE(ABORT, 'WOML run bindings are immutable');
END;

CREATE TRIGGER IF NOT EXISTS woml_runs_no_delete
BEFORE DELETE ON woml_runs
BEGIN
  SELECT RAISE(ABORT, 'WOML run bindings are immutable');
END;

CREATE TRIGGER IF NOT EXISTS woml_run_events_no_update
BEFORE UPDATE ON woml_run_events
BEGIN
  SELECT RAISE(ABORT, 'WOML run events are append-only');
END;

CREATE TRIGGER IF NOT EXISTS woml_run_events_no_delete
BEFORE DELETE ON woml_run_events
BEGIN
  SELECT RAISE(ABORT, 'WOML run events are append-only');
END;
"#;

const CREATE_APPROVAL_SCHEMA_V2: &str = r#"
CREATE TABLE woml_approval_tokens (
  token_id TEXT PRIMARY KEY,
  secret_hash BLOB NOT NULL CHECK (length(secret_hash) = 32),
  request_id TEXT NOT NULL,
  run_id TEXT NOT NULL,
  approval_id TEXT NOT NULL,
  issued_at TEXT NOT NULL,
  credential_expires_at TEXT NOT NULL,
  FOREIGN KEY (run_id) REFERENCES woml_runs(run_id)
);

CREATE INDEX woml_approval_tokens_request
  ON woml_approval_tokens(run_id, approval_id, request_id);

CREATE TRIGGER woml_approval_tokens_no_update
BEFORE UPDATE ON woml_approval_tokens
BEGIN
  SELECT RAISE(ABORT, 'WOML approval credentials are append-only');
END;

CREATE TRIGGER woml_approval_tokens_no_delete
BEFORE DELETE ON woml_approval_tokens
BEGIN
  SELECT RAISE(ABORT, 'WOML approval credentials are append-only');
END;
"#;

const CREATE_NOTIFICATION_SCHEMA_V3: &str = r#"
CREATE TABLE woml_notification_capabilities (
  capability_id TEXT PRIMARY KEY,
  secret_hash BLOB NOT NULL CHECK (length(secret_hash) = 32),
  attempt_id TEXT NOT NULL UNIQUE,
  request_id TEXT NOT NULL,
  run_id TEXT NOT NULL,
  approval_id TEXT NOT NULL,
  delivery_id TEXT NOT NULL,
  issued_at TEXT NOT NULL,
  credential_expires_at TEXT NOT NULL,
  FOREIGN KEY (run_id) REFERENCES woml_runs(run_id)
);

CREATE INDEX woml_notification_capabilities_delivery
  ON woml_notification_capabilities(run_id, approval_id, request_id, delivery_id);

CREATE TRIGGER woml_notification_capabilities_no_update
BEFORE UPDATE ON woml_notification_capabilities
BEGIN
  SELECT RAISE(ABORT, 'WOML notification capabilities are append-only');
END;

CREATE TRIGGER woml_notification_capabilities_no_delete
BEFORE DELETE ON woml_notification_capabilities
BEGIN
  SELECT RAISE(ABORT, 'WOML notification capabilities are append-only');
END;
"#;

const CREATE_TRIGGER_SCHEMA_V4: &str = r#"
CREATE TABLE woml_trigger_occurrences (
  occurrence_id TEXT PRIMARY KEY,
  workflow_id TEXT NOT NULL,
  trigger_id TEXT NOT NULL,
  trigger_handler TEXT NOT NULL,
  definition_hash TEXT NOT NULL,
  source_identity_hash TEXT NOT NULL,
  payload_hash TEXT NOT NULL,
  received_at TEXT NOT NULL,
  run_id TEXT NOT NULL UNIQUE,
  UNIQUE (workflow_id, trigger_id, source_identity_hash),
  FOREIGN KEY (definition_hash) REFERENCES woml_definitions(definition_hash),
  FOREIGN KEY (run_id) REFERENCES woml_runs(run_id)
);

CREATE INDEX woml_trigger_occurrences_run
  ON woml_trigger_occurrences(run_id);

CREATE TRIGGER woml_trigger_occurrences_no_update
BEFORE UPDATE ON woml_trigger_occurrences
BEGIN
  SELECT RAISE(ABORT, 'WOML trigger occurrences are immutable');
END;

CREATE TRIGGER woml_trigger_occurrences_no_delete
BEFORE DELETE ON woml_trigger_occurrences
BEGIN
  SELECT RAISE(ABORT, 'WOML trigger occurrences are immutable');
END;
"#;

const CREATE_SCHEDULE_SCHEMA_V5: &str = r#"
CREATE TABLE woml_schedule_cursors (
  workflow_id TEXT NOT NULL,
  trigger_id TEXT NOT NULL,
  definition_hash TEXT NOT NULL,
  cron TEXT NOT NULL,
  timezone TEXT NOT NULL,
  on_missed TEXT NOT NULL CHECK (on_missed IN ('skip', 'run-once')),
  next_scheduled_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (workflow_id, trigger_id),
  FOREIGN KEY (definition_hash) REFERENCES woml_definitions(definition_hash)
);

CREATE INDEX woml_schedule_cursors_definition
  ON woml_schedule_cursors(definition_hash);
"#;

const CREATE_INTERVAL_SCHEMA_V6: &str = r#"
CREATE TABLE woml_interval_cursors (
  workflow_id TEXT NOT NULL,
  trigger_id TEXT NOT NULL,
  definition_hash TEXT NOT NULL,
  every_ms INTEGER NOT NULL CHECK (every_ms BETWEEN 1000 AND 2592000000),
  on_missed TEXT NOT NULL CHECK (on_missed IN ('skip', 'run-once')),
  anchor_at TEXT NOT NULL,
  next_sequence INTEGER NOT NULL CHECK (next_sequence >= 1),
  next_scheduled_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (workflow_id, trigger_id),
  FOREIGN KEY (definition_hash) REFERENCES woml_definitions(definition_hash)
);

CREATE INDEX woml_interval_cursors_definition
  ON woml_interval_cursors(definition_hash);
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunDefinitionBinding {
  pub run_id: String,
  pub workflow_id: String,
  pub definition_hash: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TriggerAdmissionRequest {
  pub workflow_id: String,
  pub definition_hash: String,
  pub trigger_id: String,
  pub trigger_handler: String,
  pub source_identity: String,
  pub payload: Map<String, Value>,
  pub received_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerOccurrence {
  pub occurrence_schema_version: u32,
  pub occurrence_id: String,
  pub workflow_id: String,
  pub trigger_id: String,
  pub trigger_handler: String,
  pub definition_hash: String,
  pub source_identity_hash: String,
  pub payload_hash: String,
  pub received_at: DateTime<Utc>,
  pub run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerAdmissionOutcome {
  pub occurrence_id: String,
  pub run_id: String,
  pub duplicate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleCursorRegistration {
  pub workflow_id: String,
  pub trigger_id: String,
  pub definition_hash: String,
  pub cron: String,
  pub timezone: String,
  pub on_missed: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleCursor {
  pub workflow_id: String,
  pub trigger_id: String,
  pub definition_hash: String,
  pub cron: String,
  pub timezone: String,
  pub on_missed: String,
  pub next_scheduled_at: DateTime<Utc>,
  pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleCursorRegistrationOutcome {
  pub cursor: ScheduleCursor,
  pub initialized: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntervalCursorRegistration {
  pub workflow_id: String,
  pub trigger_id: String,
  pub definition_hash: String,
  pub every_ms: u64,
  pub on_missed: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntervalCursor {
  pub workflow_id: String,
  pub trigger_id: String,
  pub definition_hash: String,
  pub every_ms: u64,
  pub on_missed: String,
  pub anchor_at: DateTime<Utc>,
  pub next_sequence: u64,
  pub next_scheduled_at: DateTime<Utc>,
  pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntervalCursorRegistrationOutcome {
  pub cursor: IntervalCursor,
  pub initialized: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerRecoveryWork {
  pub occurrence: TriggerOccurrence,
  pub trigger: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedApprovalToken {
  pub token: String,
  pub token_id: String,
  pub request_id: String,
  pub run_id: String,
  pub approval_id: String,
  pub issued_at: DateTime<Utc>,
  pub credential_expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalTokenBinding {
  pub token_id: String,
  pub request_id: String,
  pub run_id: String,
  pub approval_id: String,
  pub issued_at: DateTime<Utc>,
  pub credential_expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationDeliveryWork {
  pub run_id: String,
  pub approval_id: String,
  pub request_id: String,
  pub delivery_id: String,
  pub provider: String,
  pub destination: String,
  pub credentials: BTreeMap<String, String>,
  pub attempt: u32,
  pub attempt_id: String,
  pub idempotency_key: String,
  pub decision_capability: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationProviderDeliveryResult {
  Succeeded(ProviderMessageIdentity),
  Failed(NotificationSafeFailure),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationUpdateWork {
  pub run_id: String,
  pub approval_id: String,
  pub request_id: String,
  pub delivery_id: String,
  pub provider: String,
  pub credentials: BTreeMap<String, String>,
  pub provider_message: ProviderMessageIdentity,
  pub resolution: crate::event::NotificationResolution,
  pub update_id: String,
  pub idempotency_key: String,
  pub attempt: u32,
  pub attempt_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationProviderUpdateResult {
  Succeeded,
  Failed(NotificationSafeFailure),
}

pub trait NotificationProviderAdapter {
  fn deliver(&mut self, work: &NotificationDeliveryWork) -> NotificationProviderDeliveryResult;
  fn update(&mut self, work: &NotificationUpdateWork) -> NotificationProviderUpdateResult;
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationDispatchReport {
  pub attempted: usize,
  pub succeeded: usize,
  pub failed: usize,
  pub run_failed: bool,
  pub updates_attempted: usize,
  pub updates_succeeded: usize,
  pub updates_failed: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecisionOutcomeStatus {
  Accepted,
  AlreadyResolved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalDecisionOutcome {
  pub status: ApprovalDecisionOutcomeStatus,
  pub run_id: String,
  pub approval_id: String,
  pub request_id: String,
  pub decision: ApprovalDecision,
  pub source: ApprovalDecisionSource,
  pub decided_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalTimeoutSettlementStatus {
  Settled,
  AlreadyResolved,
  NotDue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalTimeoutSettlement {
  pub status: ApprovalTimeoutSettlementStatus,
  pub run_id: String,
  pub approval_id: String,
  pub request_id: String,
  pub resolution: Option<ApprovalResolution>,
  pub settled_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryReport {
  pub inspected_runs: usize,
  pub recovered_runs: usize,
  pub interrupted_attempts: usize,
  pub resumable_runs: usize,
}

#[derive(Debug, Error)]
pub enum DurableStoreError {
  #[error(transparent)]
  Sqlite(#[from] rusqlite::Error),
  #[error(transparent)]
  Json(#[from] serde_json::Error),
  #[error(transparent)]
  Fold(#[from] FoldError),
  #[error(transparent)]
  InvalidModel(#[from] ModelValidationError),
  #[error("unsupported WOML event-store schema version {0:?}")]
  UnsupportedStoreVersion(String),
  #[error("compiled definition {0:?} is not registered")]
  DefinitionNotFound(String),
  #[error("definition hash {0:?} is already bound to a different compiled model")]
  DefinitionConflict(String),
  #[error("run {0:?} does not exist")]
  RunNotFound(String),
  #[error("run {0:?} already exists and cannot be rebound")]
  RunAlreadyExists(String),
  #[error("trigger {trigger_id:?} is not registered for workflow {workflow_id:?}")]
  TriggerNotFound {
    workflow_id: String,
    trigger_id: String,
  },
  #[error("trigger admission does not match its registered workflow definition")]
  TriggerDefinitionMismatch,
  #[error("trigger admission handler does not match the compiled trigger")]
  TriggerHandlerMismatch,
  #[error("the trigger source identity is already bound to a different payload")]
  TriggerIdempotencyConflict,
  #[error("the durable schedule cursor changed before this operation could commit")]
  ScheduleCursorConflict,
  #[error("the durable interval cursor changed before this operation could commit")]
  IntervalCursorConflict,
  #[error("stored trigger occurrence history is contradictory: {0}")]
  TriggerHistoryInvalid(String),
  #[error("stored event is invalid: {0}")]
  InvalidStoredEvent(String),
  #[error("approval token is invalid")]
  InvalidApprovalToken,
  #[error("approval token has expired")]
  ExpiredApprovalToken,
  #[error("approval deadline has already been reached")]
  ApprovalExpired,
  #[error("a different human approval decision is already durable")]
  ApprovalDecisionConflict,
  #[error("approval timeout settlement requires a request with a deadline")]
  ApprovalHasNoDeadline,
  #[error("{0}")]
  Contract(String),
}

#[derive(Debug)]
pub struct DurableEventStore {
  connection: Connection,
}

enum RunRecovery {
  Unchanged,
  Resumable,
  Recovered { interrupted_attempts: usize },
}

fn migrate_store_v1_to_v2(connection: &mut Connection) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_APPROVAL_SCHEMA_V2)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V2, STORE_SCHEMA_VERSION_V1],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v1-to-v2 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

fn migrate_store_v2_to_v3(connection: &mut Connection) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_NOTIFICATION_SCHEMA_V3)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V3, STORE_SCHEMA_VERSION_V2],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v2-to-v3 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

fn migrate_store_v3_to_v4(connection: &mut Connection) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_TRIGGER_SCHEMA_V4)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V4, STORE_SCHEMA_VERSION_V3],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v3-to-v4 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

fn migrate_store_v4_to_v5(connection: &mut Connection) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_SCHEDULE_SCHEMA_V5)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V5, STORE_SCHEMA_VERSION_V4],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v4-to-v5 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

fn migrate_store_v5_to_v6(connection: &mut Connection) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_INTERVAL_SCHEMA_V6)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V6, STORE_SCHEMA_VERSION_V5],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v5-to-v6 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

fn validate_store_v2_schema(connection: &Connection) -> Result<(), DurableStoreError> {
  for (object_type, name) in [
    ("table", "woml_approval_tokens"),
    ("index", "woml_approval_tokens_request"),
    ("trigger", "woml_approval_tokens_no_update"),
    ("trigger", "woml_approval_tokens_no_delete"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(
         SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2
       )",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v2 is missing required {object_type} {name:?}."
      )));
    }
  }
  Ok(())
}

fn validate_store_v3_schema(connection: &Connection) -> Result<(), DurableStoreError> {
  validate_store_v2_schema(connection)?;
  for (object_type, name) in [
    ("table", "woml_notification_capabilities"),
    ("index", "woml_notification_capabilities_delivery"),
    ("trigger", "woml_notification_capabilities_no_update"),
    ("trigger", "woml_notification_capabilities_no_delete"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v3 is missing required {object_type} {name:?}."
      )));
    }
  }
  Ok(())
}

fn validate_store_v4_schema(connection: &Connection) -> Result<(), DurableStoreError> {
  validate_store_v3_schema(connection)?;
  for (object_type, name) in [
    ("table", "woml_trigger_occurrences"),
    ("index", "woml_trigger_occurrences_run"),
    ("trigger", "woml_trigger_occurrences_no_update"),
    ("trigger", "woml_trigger_occurrences_no_delete"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v4 is missing required {object_type} {name:?}."
      )));
    }
  }
  Ok(())
}

fn validate_store_v5_schema(connection: &Connection) -> Result<(), DurableStoreError> {
  validate_store_v4_schema(connection)?;
  for (object_type, name) in [
    ("table", "woml_schedule_cursors"),
    ("index", "woml_schedule_cursors_definition"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v5 is missing required {object_type} {name:?}."
      )));
    }
  }
  Ok(())
}

fn validate_store_v6_schema(connection: &Connection) -> Result<(), DurableStoreError> {
  validate_store_v5_schema(connection)?;
  for (object_type, name) in [
    ("table", "woml_interval_cursors"),
    ("index", "woml_interval_cursors_definition"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v6 is missing required {object_type} {name:?}."
      )));
    }
  }
  Ok(())
}

impl DurableEventStore {
  pub fn open(path: impl AsRef<Path>) -> Result<Self, DurableStoreError> {
    let connection = Connection::open(path)?;
    Self::initialize(connection)
  }

  pub fn open_in_memory() -> Result<Self, DurableStoreError> {
    let connection = Connection::open_in_memory()?;
    Self::initialize(connection)
  }

  fn initialize(mut connection: Connection) -> Result<Self, DurableStoreError> {
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    connection.execute_batch(CREATE_SCHEMA)?;
    let version: Option<String> = connection
      .query_row(
        "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
        [],
        |row| row.get(0),
      )
      .optional()?;
    match version.as_deref() {
      Some(STORE_SCHEMA_VERSION_V6) => validate_store_v6_schema(&connection)?,
      Some(STORE_SCHEMA_VERSION_V5) => {
        validate_store_v5_schema(&connection)?;
        migrate_store_v5_to_v6(&mut connection)?;
      }
      Some(STORE_SCHEMA_VERSION_V4) => {
        validate_store_v4_schema(&connection)?;
        migrate_store_v4_to_v5(&mut connection)?;
        migrate_store_v5_to_v6(&mut connection)?;
      }
      Some(STORE_SCHEMA_VERSION_V3) => {
        validate_store_v3_schema(&connection)?;
        migrate_store_v3_to_v4(&mut connection)?;
        migrate_store_v4_to_v5(&mut connection)?;
        migrate_store_v5_to_v6(&mut connection)?;
      }
      Some(STORE_SCHEMA_VERSION_V2) => {
        validate_store_v2_schema(&connection)?;
        migrate_store_v2_to_v3(&mut connection)?;
        migrate_store_v3_to_v4(&mut connection)?;
        migrate_store_v4_to_v5(&mut connection)?;
        migrate_store_v5_to_v6(&mut connection)?;
      }
      Some(STORE_SCHEMA_VERSION_V1) => {
        migrate_store_v1_to_v2(&mut connection)?;
        validate_store_v2_schema(&connection)?;
        migrate_store_v2_to_v3(&mut connection)?;
        migrate_store_v3_to_v4(&mut connection)?;
        migrate_store_v4_to_v5(&mut connection)?;
        migrate_store_v5_to_v6(&mut connection)?;
      }
      Some(version) => {
        return Err(DurableStoreError::UnsupportedStoreVersion(
          version.to_string(),
        ));
      }
      None => {
        connection.execute(
          "INSERT INTO woml_store_metadata(key, value) VALUES ('schema_version', ?1)",
          [STORE_SCHEMA_VERSION_V1],
        )?;
        migrate_store_v1_to_v2(&mut connection)?;
        validate_store_v2_schema(&connection)?;
        migrate_store_v2_to_v3(&mut connection)?;
        migrate_store_v3_to_v4(&mut connection)?;
        migrate_store_v4_to_v5(&mut connection)?;
        migrate_store_v5_to_v6(&mut connection)?;
      }
    }
    Ok(Self { connection })
  }

  pub fn register_definition(
    &mut self,
    workflow: &CompiledWorkflowDefinition,
    definition_hash: &str,
  ) -> Result<(), DurableStoreError> {
    workflow.validate_structure()?;
    if !is_definition_hash(definition_hash) {
      return Err(DurableStoreError::Contract(
        "A durable definition requires a valid RFC 8785 SHA-256 hash.".to_string(),
      ));
    }
    let model_json = serde_json::to_string(workflow)?;
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let stored: Option<(String, String)> = transaction
      .query_row(
        "SELECT workflow_id, model_json FROM woml_definitions WHERE definition_hash = ?1",
        [definition_hash],
        |row| Ok((row.get(0)?, row.get(1)?)),
      )
      .optional()?;
    if let Some((workflow_id, stored_json)) = stored {
      let stored_value: Value = serde_json::from_str(&stored_json)?;
      let incoming_value: Value = serde_json::from_str(&model_json)?;
      if workflow_id != workflow.workflow_id || stored_value != incoming_value {
        return Err(DurableStoreError::DefinitionConflict(
          definition_hash.to_string(),
        ));
      }
    } else {
      transaction.execute(
        "INSERT INTO woml_definitions(
           definition_hash, workflow_id, schema_version, model_json, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
          definition_hash,
          workflow.workflow_id,
          i64::from(workflow.schema_version),
          model_json,
          Utc::now().to_rfc3339(),
        ],
      )?;
    }
    transaction.commit()?;
    Ok(())
  }

  pub fn definition(
    &self,
    definition_hash: &str,
  ) -> Result<CompiledWorkflowDefinition, DurableStoreError> {
    let model_json: String = self
      .connection
      .query_row(
        "SELECT model_json FROM woml_definitions WHERE definition_hash = ?1",
        [definition_hash],
        |row| row.get(0),
      )
      .optional()?
      .ok_or_else(|| DurableStoreError::DefinitionNotFound(definition_hash.to_string()))?;
    let workflow: CompiledWorkflowDefinition = serde_json::from_str(&model_json)?;
    workflow.validate_structure()?;
    Ok(workflow)
  }

  pub fn run_binding(&self, run_id: &str) -> Result<RunDefinitionBinding, DurableStoreError> {
    self
      .connection
      .query_row(
        "SELECT workflow_id, definition_hash FROM woml_runs WHERE run_id = ?1",
        [run_id],
        |row| {
          Ok(RunDefinitionBinding {
            run_id: run_id.to_string(),
            workflow_id: row.get(0)?,
            definition_hash: row.get(1)?,
          })
        },
      )
      .optional()?
      .ok_or_else(|| DurableStoreError::RunNotFound(run_id.to_string()))
  }

  pub fn trigger_occurrence(
    &self,
    occurrence_id: &str,
  ) -> Result<TriggerOccurrence, DurableStoreError> {
    load_trigger_occurrence_by_id(&self.connection, occurrence_id)?.ok_or_else(|| {
      DurableStoreError::TriggerHistoryInvalid(format!(
        "trigger occurrence {occurrence_id:?} does not exist"
      ))
    })
  }

  pub fn admit_trigger_occurrence(
    &mut self,
    request: TriggerAdmissionRequest,
  ) -> Result<TriggerAdmissionOutcome, DurableStoreError> {
    validate_trigger_admission_request(&request)?;
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let outcome = admit_trigger_occurrence_in_transaction(&transaction, &request)?;
    transaction.commit()?;
    Ok(outcome)
  }

  pub fn register_schedule_cursor(
    &mut self,
    registration: &ScheduleCursorRegistration,
    initial_next_scheduled_at: DateTime<Utc>,
    registered_at: DateTime<Utc>,
  ) -> Result<ScheduleCursorRegistrationOutcome, DurableStoreError> {
    validate_schedule_cursor_registration(registration)?;
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    validate_schedule_registration_definition(&transaction, registration)?;
    let existing = load_schedule_cursor(
      &transaction,
      &registration.workflow_id,
      &registration.trigger_id,
    )?;
    let initialized = existing.as_ref().is_none_or(|cursor| {
      cursor.definition_hash != registration.definition_hash
        || cursor.cron != registration.cron
        || cursor.timezone != registration.timezone
        || cursor.on_missed != registration.on_missed
    });
    if initialized {
      transaction.execute(
        "INSERT INTO woml_schedule_cursors(
           workflow_id, trigger_id, definition_hash, cron, timezone, on_missed,
           next_scheduled_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(workflow_id, trigger_id) DO UPDATE SET
           definition_hash = excluded.definition_hash,
           cron = excluded.cron,
           timezone = excluded.timezone,
           on_missed = excluded.on_missed,
           next_scheduled_at = excluded.next_scheduled_at,
           updated_at = excluded.updated_at",
        params![
          registration.workflow_id,
          registration.trigger_id,
          registration.definition_hash,
          registration.cron,
          registration.timezone,
          registration.on_missed,
          initial_next_scheduled_at.to_rfc3339(),
          registered_at.to_rfc3339(),
        ],
      )?;
    }
    let cursor = load_schedule_cursor(
      &transaction,
      &registration.workflow_id,
      &registration.trigger_id,
    )?
    .ok_or_else(|| DurableStoreError::Contract("Schedule cursor was not stored.".to_string()))?;
    transaction.commit()?;
    Ok(ScheduleCursorRegistrationOutcome {
      cursor,
      initialized,
    })
  }

  pub fn schedule_cursor(
    &self,
    workflow_id: &str,
    trigger_id: &str,
  ) -> Result<ScheduleCursor, DurableStoreError> {
    load_schedule_cursor(&self.connection, workflow_id, trigger_id)?.ok_or_else(|| {
      DurableStoreError::Contract(format!(
        "Schedule cursor for workflow {workflow_id:?}, trigger {trigger_id:?} does not exist."
      ))
    })
  }

  pub fn advance_schedule_cursor(
    &mut self,
    workflow_id: &str,
    trigger_id: &str,
    expected_next_scheduled_at: DateTime<Utc>,
    next_scheduled_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
  ) -> Result<ScheduleCursor, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    update_schedule_cursor(
      &transaction,
      workflow_id,
      trigger_id,
      expected_next_scheduled_at,
      next_scheduled_at,
      updated_at,
    )?;
    let cursor = load_schedule_cursor(&transaction, workflow_id, trigger_id)?
      .ok_or(DurableStoreError::ScheduleCursorConflict)?;
    transaction.commit()?;
    Ok(cursor)
  }

  pub fn claim_schedule_occurrence(
    &mut self,
    expected_next_scheduled_at: DateTime<Utc>,
    next_scheduled_at: DateTime<Utc>,
    request: TriggerAdmissionRequest,
  ) -> Result<(TriggerAdmissionOutcome, ScheduleCursor), DurableStoreError> {
    validate_trigger_admission_request(&request)?;
    if request.trigger_handler != "trigger.schedule" {
      return Err(DurableStoreError::TriggerHandlerMismatch);
    }
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let outcome = admit_trigger_occurrence_in_transaction(&transaction, &request)?;
    update_schedule_cursor(
      &transaction,
      &request.workflow_id,
      &request.trigger_id,
      expected_next_scheduled_at,
      next_scheduled_at,
      request.received_at,
    )?;
    let cursor = load_schedule_cursor(&transaction, &request.workflow_id, &request.trigger_id)?
      .ok_or(DurableStoreError::ScheduleCursorConflict)?;
    transaction.commit()?;
    Ok((outcome, cursor))
  }

  pub fn register_interval_cursor(
    &mut self,
    registration: &IntervalCursorRegistration,
    anchor_at: DateTime<Utc>,
    registered_at: DateTime<Utc>,
  ) -> Result<IntervalCursorRegistrationOutcome, DurableStoreError> {
    validate_interval_cursor_registration(registration)?;
    let interval = crate::interval::WomlInterval::new(registration.every_ms)
      .map_err(|error| DurableStoreError::Contract(error.to_string()))?;
    let anchor_at = crate::interval::WomlInterval::normalize_anchor(anchor_at)
      .map_err(|error| DurableStoreError::Contract(error.to_string()))?;
    let initial_next_scheduled_at = interval
      .planned_at(anchor_at, 1)
      .map_err(|error| DurableStoreError::Contract(error.to_string()))?;
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    validate_interval_registration_definition(&transaction, registration)?;
    let existing = load_interval_cursor(
      &transaction,
      &registration.workflow_id,
      &registration.trigger_id,
    )?;
    let initialized = existing.as_ref().is_none_or(|cursor| {
      cursor.definition_hash != registration.definition_hash
        || cursor.every_ms != registration.every_ms
        || cursor.on_missed != registration.on_missed
    });
    if initialized {
      transaction.execute(
        "INSERT INTO woml_interval_cursors(
           workflow_id, trigger_id, definition_hash, every_ms, on_missed,
           anchor_at, next_sequence, next_scheduled_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?8)
         ON CONFLICT(workflow_id, trigger_id) DO UPDATE SET
           definition_hash = excluded.definition_hash,
           every_ms = excluded.every_ms,
           on_missed = excluded.on_missed,
           anchor_at = excluded.anchor_at,
           next_sequence = excluded.next_sequence,
           next_scheduled_at = excluded.next_scheduled_at,
           updated_at = excluded.updated_at",
        params![
          registration.workflow_id,
          registration.trigger_id,
          registration.definition_hash,
          i64::try_from(registration.every_ms).map_err(|_| DurableStoreError::Contract(
            "Interval duration exceeds the durable integer range.".to_string()
          ))?,
          registration.on_missed,
          anchor_at.to_rfc3339(),
          initial_next_scheduled_at.to_rfc3339(),
          registered_at.to_rfc3339(),
        ],
      )?;
    }
    let cursor = load_interval_cursor(
      &transaction,
      &registration.workflow_id,
      &registration.trigger_id,
    )?
    .ok_or_else(|| DurableStoreError::Contract("Interval cursor was not stored.".to_string()))?;
    transaction.commit()?;
    Ok(IntervalCursorRegistrationOutcome {
      cursor,
      initialized,
    })
  }

  pub fn interval_cursor(
    &self,
    workflow_id: &str,
    trigger_id: &str,
  ) -> Result<IntervalCursor, DurableStoreError> {
    load_interval_cursor(&self.connection, workflow_id, trigger_id)?.ok_or_else(|| {
      DurableStoreError::Contract(format!(
        "Interval cursor for workflow {workflow_id:?}, trigger {trigger_id:?} does not exist."
      ))
    })
  }

  #[allow(clippy::too_many_arguments)]
  pub fn advance_interval_cursor(
    &mut self,
    workflow_id: &str,
    trigger_id: &str,
    expected_sequence: u64,
    expected_scheduled_at: DateTime<Utc>,
    next_sequence: u64,
    next_scheduled_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
  ) -> Result<IntervalCursor, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    update_interval_cursor(
      &transaction,
      workflow_id,
      trigger_id,
      expected_sequence,
      expected_scheduled_at,
      next_sequence,
      next_scheduled_at,
      updated_at,
    )?;
    let cursor = load_interval_cursor(&transaction, workflow_id, trigger_id)?
      .ok_or(DurableStoreError::IntervalCursorConflict)?;
    transaction.commit()?;
    Ok(cursor)
  }

  #[allow(clippy::too_many_arguments)]
  pub fn claim_interval_occurrence(
    &mut self,
    expected_sequence: u64,
    expected_scheduled_at: DateTime<Utc>,
    next_sequence: u64,
    next_scheduled_at: DateTime<Utc>,
    request: TriggerAdmissionRequest,
  ) -> Result<(TriggerAdmissionOutcome, IntervalCursor), DurableStoreError> {
    validate_trigger_admission_request(&request)?;
    if request.trigger_handler != "trigger.interval" {
      return Err(DurableStoreError::TriggerHandlerMismatch);
    }
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let outcome = admit_trigger_occurrence_in_transaction(&transaction, &request)?;
    update_interval_cursor(
      &transaction,
      &request.workflow_id,
      &request.trigger_id,
      expected_sequence,
      expected_scheduled_at,
      next_sequence,
      next_scheduled_at,
      request.received_at,
    )?;
    let cursor = load_interval_cursor(&transaction, &request.workflow_id, &request.trigger_id)?
      .ok_or(DurableStoreError::IntervalCursorConflict)?;
    transaction.commit()?;
    Ok((outcome, cursor))
  }

  pub fn recover_undispatched_trigger_runs(
    &self,
  ) -> Result<Vec<TriggerRecoveryWork>, DurableStoreError> {
    let mut statement = self.connection.prepare(
      "SELECT occurrences.occurrence_id
       FROM woml_trigger_occurrences AS occurrences
       JOIN woml_runs AS runs ON runs.run_id = occurrences.run_id
       WHERE (
         SELECT COUNT(*) FROM woml_run_events AS events
         WHERE events.run_id = occurrences.run_id
       ) = 1
       ORDER BY occurrences.received_at, occurrences.occurrence_id",
    )?;
    let occurrence_ids = statement
      .query_map([], |row| row.get::<_, String>(0))?
      .collect::<Result<Vec<_>, _>>()?;
    drop(statement);

    occurrence_ids
      .into_iter()
      .map(|occurrence_id| {
        let occurrence = load_trigger_occurrence_by_id(&self.connection, &occurrence_id)?
          .ok_or_else(|| {
            DurableStoreError::TriggerHistoryInvalid(format!(
              "trigger occurrence {occurrence_id:?} disappeared during recovery"
            ))
          })?;
        let trigger = validate_trigger_occurrence_history(&self.connection, &occurrence)?;
        Ok(TriggerRecoveryWork {
          occurrence,
          trigger,
        })
      })
      .collect()
  }

  pub fn start_run(
    &mut self,
    event_id: impl Into<String>,
    run_id: impl Into<String>,
    occurred_at: DateTime<Utc>,
    workflow_id: impl Into<String>,
    definition_hash: impl Into<String>,
    trigger: Map<String, Value>,
  ) -> Result<(RunEvent, RunProjection), DurableStoreError> {
    let workflow_id = workflow_id.into();
    let definition_hash = definition_hash.into();
    self.append_payload(
      run_id.into(),
      event_id.into(),
      occurred_at,
      RunEventPayload::RunStarted(RunStartedData {
        workflow_id,
        definition_hash,
        trigger_id: None,
        trigger_handler: None,
        trigger_occurrence_id: None,
        trigger,
      }),
    )
  }

  pub fn append_payload(
    &mut self,
    run_id: impl Into<String>,
    event_id: impl Into<String>,
    occurred_at: DateTime<Utc>,
    payload: RunEventPayload,
  ) -> Result<(RunEvent, RunProjection), DurableStoreError> {
    let run_id = run_id.into();
    let event_id = event_id.into();
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;

    let event_schema_version = match &payload {
      RunEventPayload::RunStarted(data) => {
        let existing: Option<String> = transaction
          .query_row(
            "SELECT run_id FROM woml_runs WHERE run_id = ?1",
            [&run_id],
            |row| row.get(0),
          )
          .optional()?;
        if existing.is_some() {
          return Err(DurableStoreError::RunAlreadyExists(run_id));
        }
        let registered_workflow: Option<(String, i64)> = transaction
          .query_row(
            "SELECT workflow_id, schema_version FROM woml_definitions WHERE definition_hash = ?1",
            [&data.definition_hash],
            |row| Ok((row.get(0)?, row.get(1)?)),
          )
          .optional()?;
        let (registered_workflow, model_schema_version) = registered_workflow
          .ok_or_else(|| DurableStoreError::DefinitionNotFound(data.definition_hash.clone()))?;
        if registered_workflow != data.workflow_id {
          return Err(DurableStoreError::Contract(
            "run_started workflowId does not match its registered definition.".to_string(),
          ));
        }
        transaction.execute(
          "INSERT INTO woml_runs(run_id, workflow_id, definition_hash, created_at)
           VALUES (?1, ?2, ?3, ?4)",
          params![
            run_id,
            data.workflow_id,
            data.definition_hash,
            occurred_at.to_rfc3339(),
          ],
        )?;
        let model_schema_version = u32::try_from(model_schema_version).map_err(|_| {
          DurableStoreError::Contract(
            "Stored compiled-model schema version is invalid.".to_string(),
          )
        })?;
        run_event_schema_version_for_model(model_schema_version)
      }
      _ => {
        ensure_run_exists(&transaction, &run_id)?;
        let existing = load_events(&transaction, &run_id)?;
        existing
          .first()
          .map(|event| event.event_schema_version)
          .ok_or_else(|| {
            DurableStoreError::Contract(
              "A non-start event cannot be appended before run_started.".to_string(),
            )
          })?
      }
    };

    let mut events = load_events(&transaction, &run_id)?;
    let workflow = definition_for_run(&transaction, &run_id)?;
    let binding = run_binding_in_transaction(&transaction, &run_id)?;
    validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
      .map_err(DurableStoreError::Contract)?;
    if let RunEventPayload::BranchSelected(data) = &payload {
      let projection = fold_events(&events)?;
      if projection.branch_selections.contains_key(&data.branch_id) {
        return Err(DurableStoreError::Contract(format!(
          "Branch {:?} already has an immutable selection.",
          data.branch_id
        )));
      }
      let selector_id = format!("__woml_branch__{}__select", data.branch_id);
      let ready = ready_node_ids_for_projection(&workflow, &binding.definition_hash, &projection)
        .map_err(DurableStoreError::Contract)?;
      if !ready.iter().any(|node_id| node_id == &selector_id) {
        return Err(DurableStoreError::Contract(format!(
          "Branch selector {selector_id:?} is not ready for selection."
        )));
      }
    }
    let event = append_to_history(
      &transaction,
      &mut events,
      &run_id,
      event_id,
      occurred_at,
      event_schema_version,
      payload,
    )?;
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = fold_events(&events)?;
    transaction.commit()?;
    Ok((event, projection))
  }

  pub(crate) fn append_payloads_atomically(
    &mut self,
    run_id: &str,
    payloads: Vec<(String, DateTime<Utc>, RunEventPayload)>,
  ) -> Result<RunProjection, DurableStoreError> {
    if payloads.is_empty() {
      return Err(DurableStoreError::Contract(
        "An atomic event batch must not be empty.".to_string(),
      ));
    }
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let mut events = load_events(&transaction, run_id)?;
    let event_schema_version = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "An atomic event batch cannot be appended before run_started.".to_string(),
        )
      })?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    for (event_id, occurred_at, payload) in payloads {
      validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
        .map_err(DurableStoreError::Contract)?;
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        event_id,
        occurred_at,
        event_schema_version,
        payload,
      )?;
    }
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = fold_events(&events)?;
    transaction.commit()?;
    Ok(projection)
  }

  pub fn events(&self, run_id: &str) -> Result<Vec<RunEvent>, DurableStoreError> {
    self.run_binding(run_id)?;
    load_events(&self.connection, run_id)
  }

  pub fn projection(&self, run_id: &str) -> Result<RunProjection, DurableStoreError> {
    let events = self.events(run_id)?;
    let binding = self.run_binding(run_id)?;
    let workflow = self.definition(&binding.definition_hash)?;
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    Ok(fold_events(&events)?)
  }

  pub fn begin_notification_delivery(
    &mut self,
    run_id: &str,
    delivery_id: &str,
    now: DateTime<Utc>,
  ) -> Result<NotificationDeliveryWork, DurableStoreError> {
    let capability_id = random_hex(16)?;
    let capability_secret = random_hex(32)?;
    let capability_hash = Sha256::digest(capability_secret.as_bytes());
    let attempt_id = format!("nattempt_{}", Uuid::new_v4().simple());
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let mut events = load_events(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    let projection = fold_events(&events)?;
    let delivery = projection
      .notification_deliveries
      .get(delivery_id)
      .ok_or_else(|| {
        DurableStoreError::Contract(format!("Unknown notification delivery {delivery_id:?}."))
      })?;
    let request = projection
      .approval_requests
      .get(&delivery.approval_id)
      .ok_or_else(|| {
        DurableStoreError::Contract("Notification delivery has no approval request.".to_string())
      })?;
    if projection.status != RunStatus::Waiting
      || !matches!(request.status, ApprovalRequestStatus::Waiting)
    {
      return Err(DurableStoreError::Contract(
        "Notification delivery may start only while its approval is waiting.".to_string(),
      ));
    }
    let attempt = next_notification_attempt(&delivery.status, now)?.ok_or_else(|| {
      DurableStoreError::Contract(
        "Notification delivery is not ready for another attempt.".to_string(),
      )
    })?;
    let approval = workflow.approval(&delivery.approval_id).ok_or_else(|| {
      DurableStoreError::Contract(
        "Notification delivery references an unknown compiled approval.".to_string(),
      )
    })?;
    let definition = approval
      .notifications
      .iter()
      .find(|item| item.delivery_id == delivery_id)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "Notification delivery is absent from its compiled approval.".to_string(),
        )
      })?;
    let idempotency_key = notification_idempotency_key(run_id, &delivery.request_id, delivery_id);
    let payload =
      RunEventPayload::NotificationDeliveryAttemptStarted(NotificationDeliveryAttemptStartedData {
        approval_id: delivery.approval_id.clone(),
        request_id: delivery.request_id.clone(),
        delivery_id: delivery_id.to_string(),
        attempt,
        attempt_id: attempt_id.clone(),
        idempotency_key: idempotency_key.clone(),
      });
    validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
      .map_err(DurableStoreError::Contract)?;
    let event_schema_version = projection.event_schema_version.ok_or_else(|| {
      DurableStoreError::Contract("Notification delivery requires run_started.".to_string())
    })?;
    append_to_history(
      &transaction,
      &mut events,
      run_id,
      generated_event_id(),
      now,
      event_schema_version,
      payload,
    )?;
    let credential_expires_at = approval_credential_expiry(now, request.expires_at)?;
    transaction.execute(
      "INSERT INTO woml_notification_capabilities(
         capability_id, secret_hash, attempt_id, request_id, run_id, approval_id,
         delivery_id, issued_at, credential_expires_at
       ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
      params![
        capability_id,
        capability_hash.as_slice(),
        attempt_id,
        delivery.request_id,
        run_id,
        delivery.approval_id,
        delivery_id,
        now.to_rfc3339(),
        credential_expires_at.to_rfc3339(),
      ],
    )?;
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    transaction.commit()?;
    Ok(NotificationDeliveryWork {
      run_id: run_id.to_string(),
      approval_id: delivery.approval_id.clone(),
      request_id: delivery.request_id.clone(),
      delivery_id: delivery_id.to_string(),
      provider: definition.provider.clone(),
      destination: definition.destination.clone(),
      credentials: definition.credentials.clone(),
      attempt,
      attempt_id,
      idempotency_key,
      decision_capability: format!("ncap_{capability_id}.{capability_secret}"),
    })
  }

  pub fn complete_notification_delivery(
    &mut self,
    work: &NotificationDeliveryWork,
    result: NotificationProviderDeliveryResult,
    now: DateTime<Utc>,
  ) -> Result<RunProjection, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut events = load_events(&transaction, &work.run_id)?;
    let workflow = definition_for_run(&transaction, &work.run_id)?;
    let binding = run_binding_in_transaction(&transaction, &work.run_id)?;
    let projection = fold_events(&events)?;
    let delivery = projection
      .notification_deliveries
      .get(&work.delivery_id)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "Notification result references an unknown delivery.".to_string(),
        )
      })?;
    if !matches!(&delivery.status, NotificationDeliveryStatus::AttemptStarted { attempt, attempt_id, idempotency_key } if *attempt == work.attempt && attempt_id == &work.attempt_id && idempotency_key == &work.idempotency_key)
    {
      return Err(DurableStoreError::Contract(
        "Notification result does not match the active durable attempt.".to_string(),
      ));
    }
    let payload = match result {
      NotificationProviderDeliveryResult::Succeeded(provider_message) => {
        RunEventPayload::NotificationDeliverySucceeded(NotificationDeliverySucceededData {
          approval_id: work.approval_id.clone(),
          request_id: work.request_id.clone(),
          delivery_id: work.delivery_id.clone(),
          attempt: work.attempt,
          attempt_id: work.attempt_id.clone(),
          provider_message,
        })
      }
      NotificationProviderDeliveryResult::Failed(failure) => {
        let final_ = !failure.retryable
          || work.attempt >= 3
          || matches!(failure.kind.as_str(), "delivery_ambiguous" | "host_crashed");
        RunEventPayload::NotificationDeliveryFailed(NotificationDeliveryFailedData {
          approval_id: work.approval_id.clone(),
          request_id: work.request_id.clone(),
          delivery_id: work.delivery_id.clone(),
          attempt: work.attempt,
          attempt_id: work.attempt_id.clone(),
          final_,
          failure,
        })
      }
    };
    validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
      .map_err(DurableStoreError::Contract)?;
    let version = projection
      .event_schema_version
      .unwrap_or(RUN_EVENT_SCHEMA_VERSION_V5);
    append_to_history(
      &transaction,
      &mut events,
      &work.run_id,
      generated_event_id(),
      now,
      version,
      payload,
    )?;
    let mut projection = fold_events(&events)?;
    if notification_request_all_failed(&workflow, &projection, &work.approval_id, &work.request_id)
    {
      let failed_delivery_ids = workflow
        .approval(&work.approval_id)
        .unwrap()
        .notifications
        .into_iter()
        .map(|delivery| delivery.delivery_id)
        .collect::<Vec<_>>();
      let payload = RunEventPayload::RunFailed(RunFailedData::V5(RunFailedDataV5::Notification {
        approval_id: work.approval_id.clone(),
        request_id: work.request_id.clone(),
        failed_delivery_ids,
        failure: NotificationRunFailure {
          kind: "all_deliveries_failed".to_string(),
          code: "WOML_NOTIFICATION_DELIVERY_FAILED".to_string(),
          message: "Every configured approval notification delivery failed.".to_string(),
        },
      }));
      append_to_history(
        &transaction,
        &mut events,
        &work.run_id,
        generated_event_id(),
        now,
        version,
        payload,
      )?;
      projection = fold_events(&events)?;
    }
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    transaction.commit()?;
    Ok(projection)
  }

  pub fn dispatch_ready_notifications(
    &mut self,
    run_id: &str,
    now: DateTime<Utc>,
    adapter: &mut dyn NotificationProviderAdapter,
  ) -> Result<NotificationDispatchReport, DurableStoreError> {
    let projection = self.projection(run_id)?;
    let ready = projection
      .notification_deliveries
      .values()
      .filter(|delivery| {
        next_notification_attempt(&delivery.status, now)
          .ok()
          .flatten()
          .is_some()
      })
      .map(|delivery| delivery.delivery_id.clone())
      .collect::<Vec<_>>();
    let mut report = NotificationDispatchReport::default();
    for delivery_id in ready {
      let work = self.begin_notification_delivery(run_id, &delivery_id, now)?;
      let result = adapter.deliver(&work);
      let succeeded = matches!(result, NotificationProviderDeliveryResult::Succeeded(_));
      let projection = self.complete_notification_delivery(&work, result, now)?;
      report.attempted += 1;
      if succeeded {
        report.succeeded += 1;
      } else {
        report.failed += 1;
      }
      report.run_failed = projection.status == RunStatus::Failed;
      if report.run_failed {
        break;
      }
    }
    Ok(report)
  }

  pub fn resolve_notification_approval(
    &mut self,
    capability: &str,
    provider_actor_id: &str,
    decision: ApprovalDecision,
    now: DateTime<Utc>,
  ) -> Result<ApprovalDecisionOutcome, DurableStoreError> {
    self.resolve_notification_approval_internal(capability, None, provider_actor_id, decision, now)
  }

  pub fn resolve_notification_approval_from_provider(
    &mut self,
    capability: &str,
    delivery_id: &str,
    provider: &str,
    provider_actor_id: &str,
    decision: ApprovalDecision,
    now: DateTime<Utc>,
  ) -> Result<ApprovalDecisionOutcome, DurableStoreError> {
    self.resolve_notification_approval_internal(
      capability,
      Some((delivery_id, provider)),
      provider_actor_id,
      decision,
      now,
    )
  }

  fn resolve_notification_approval_internal(
    &mut self,
    capability: &str,
    provider_identity: Option<(&str, &str)>,
    provider_actor_id: &str,
    decision: ApprovalDecision,
    now: DateTime<Utc>,
  ) -> Result<ApprovalDecisionOutcome, DurableStoreError> {
    let (capability_id, secret) = parse_notification_capability(capability)?;
    let candidate_hash = Sha256::digest(secret.as_bytes());
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let row: Option<(Vec<u8>, String, String, String, String, String)> = transaction
      .query_row(
        "SELECT secret_hash, request_id, run_id, approval_id, delivery_id, credential_expires_at
       FROM woml_notification_capabilities WHERE capability_id = ?1",
        [capability_id],
        |row| {
          Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
          ))
        },
      )
      .optional()?;
    let Some((stored_hash, request_id, run_id, approval_id, delivery_id, expires_at)) = row else {
      return Err(DurableStoreError::InvalidApprovalToken);
    };
    if stored_hash.len() != 32 || candidate_hash.as_slice().ct_eq(&stored_hash).unwrap_u8() != 1 {
      return Err(DurableStoreError::InvalidApprovalToken);
    }
    if now >= parse_stored_timestamp(&expires_at)? {
      return Err(DurableStoreError::ExpiredApprovalToken);
    }
    let mut events = load_events(&transaction, &run_id)?;
    let workflow = definition_for_run(&transaction, &run_id)?;
    let binding = run_binding_in_transaction(&transaction, &run_id)?;
    let projection = fold_events(&events)?;
    let request = projection
      .approval_requests
      .get(&approval_id)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "Notification capability references an unknown approval.".to_string(),
        )
      })?;
    if request.request_id != request_id {
      return Err(DurableStoreError::Contract(
        "Notification capability does not match its request.".to_string(),
      ));
    }
    if let ApprovalRequestStatus::Resolved {
      resolution:
        ApprovalResolution::Decision {
          decision: existing,
          source: ApprovalDecisionSource::Human,
        },
      resolved_at,
    } = &request.status
    {
      if *existing == decision {
        return Ok(ApprovalDecisionOutcome {
          status: ApprovalDecisionOutcomeStatus::AlreadyResolved,
          run_id,
          approval_id,
          request_id,
          decision,
          source: ApprovalDecisionSource::Human,
          decided_at: *resolved_at,
        });
      }
      return Err(DurableStoreError::ApprovalDecisionConflict);
    }
    if !matches!(request.status, ApprovalRequestStatus::Waiting) {
      return Err(DurableStoreError::ApprovalExpired);
    }
    let delivery = projection
      .notification_deliveries
      .get(&delivery_id)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "Notification capability references an unknown delivery.".to_string(),
        )
      })?;
    if provider_identity.is_some_and(|(expected_delivery_id, expected_provider)| {
      expected_delivery_id != delivery_id || expected_provider != delivery.provider
    }) {
      return Err(DurableStoreError::InvalidApprovalToken);
    }
    if !matches!(
      delivery.status,
      NotificationDeliveryStatus::Succeeded { .. }
    ) {
      return Err(DurableStoreError::Contract(
        "Only a successfully delivered notification can resolve approval.".to_string(),
      ));
    }
    let mut payloads = vec![
      RunEventPayload::NotificationDecisionAccepted(
        crate::event::NotificationDecisionAcceptedData {
          approval_id: approval_id.clone(),
          request_id: request_id.clone(),
          delivery_id: delivery_id.clone(),
          provider: delivery.provider.clone(),
          provider_actor_id: provider_actor_id.to_string(),
          decision,
        },
      ),
      RunEventPayload::ApprovalResolved(ApprovalResolvedData {
        approval_id: approval_id.clone(),
        request_id: request_id.clone(),
        resolution: ApprovalResolution::Decision {
          decision,
          source: ApprovalDecisionSource::Human,
        },
      }),
    ];
    let resolution = match decision {
      ApprovalDecision::Approved => crate::event::NotificationResolution::Approved,
      ApprovalDecision::Rejected => crate::event::NotificationResolution::Rejected,
    };
    for delivered in projection.notification_deliveries.values().filter(|item| {
      item.approval_id == approval_id
        && item.request_id == request_id
        && matches!(item.status, NotificationDeliveryStatus::Succeeded { .. })
    }) {
      payloads.push(RunEventPayload::NotificationMessageUpdateRequested(
        crate::event::NotificationMessageUpdateRequestedData {
          approval_id: approval_id.clone(),
          request_id: request_id.clone(),
          delivery_id: delivered.delivery_id.clone(),
          update_id: format!("nupdate_{}", Uuid::new_v4().simple()),
          resolution,
        },
      ));
    }
    let version = projection
      .event_schema_version
      .unwrap_or(RUN_EVENT_SCHEMA_VERSION_V5);
    for payload in payloads {
      validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
        .map_err(DurableStoreError::Contract)?;
      append_to_history(
        &transaction,
        &mut events,
        &run_id,
        generated_event_id(),
        now,
        version,
        payload,
      )?;
    }
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    transaction.commit()?;
    Ok(ApprovalDecisionOutcome {
      status: ApprovalDecisionOutcomeStatus::Accepted,
      run_id,
      approval_id,
      request_id,
      decision,
      source: ApprovalDecisionSource::Human,
      decided_at: now,
    })
  }

  pub fn begin_notification_update(
    &mut self,
    run_id: &str,
    delivery_id: &str,
    now: DateTime<Utc>,
  ) -> Result<NotificationUpdateWork, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut events = load_events(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    let projection = fold_events(&events)?;
    let update = projection
      .notification_updates
      .get(delivery_id)
      .ok_or_else(|| {
        DurableStoreError::Contract("Unknown notification message update.".to_string())
      })?;
    let attempt = next_notification_update_attempt(&update.status, now)?.ok_or_else(|| {
      DurableStoreError::Contract("Notification message update is not ready.".to_string())
    })?;
    let delivery = projection
      .notification_deliveries
      .get(delivery_id)
      .ok_or_else(|| {
        DurableStoreError::Contract("Notification update has no delivery.".to_string())
      })?;
    let NotificationDeliveryStatus::Succeeded {
      provider_message, ..
    } = &delivery.status
    else {
      return Err(DurableStoreError::Contract(
        "Notification update requires a successful message.".to_string(),
      ));
    };
    let approval = workflow.approval(&update.approval_id).ok_or_else(|| {
      DurableStoreError::Contract("Notification update has no compiled approval.".to_string())
    })?;
    let definition = approval
      .notifications
      .iter()
      .find(|item| item.delivery_id == delivery_id)
      .ok_or_else(|| {
        DurableStoreError::Contract("Notification update has no compiled delivery.".to_string())
      })?;
    let attempt_id = format!("nattempt_{}", Uuid::new_v4().simple());
    let idempotency_key = notification_update_idempotency_key(
      run_id,
      &update.request_id,
      delivery_id,
      &update.update_id,
    );
    let payload = RunEventPayload::NotificationMessageUpdateAttemptStarted(
      NotificationMessageUpdateAttemptStartedData {
        approval_id: update.approval_id.clone(),
        request_id: update.request_id.clone(),
        delivery_id: delivery_id.to_string(),
        update_id: update.update_id.clone(),
        attempt,
        attempt_id: attempt_id.clone(),
      },
    );
    validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
      .map_err(DurableStoreError::Contract)?;
    let version = projection
      .event_schema_version
      .unwrap_or(RUN_EVENT_SCHEMA_VERSION_V5);
    append_to_history(
      &transaction,
      &mut events,
      run_id,
      generated_event_id(),
      now,
      version,
      payload,
    )?;
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let work = NotificationUpdateWork {
      run_id: run_id.to_string(),
      approval_id: update.approval_id.clone(),
      request_id: update.request_id.clone(),
      delivery_id: delivery_id.to_string(),
      provider: definition.provider.clone(),
      credentials: definition.credentials.clone(),
      provider_message: provider_message.clone(),
      resolution: update.resolution,
      update_id: update.update_id.clone(),
      idempotency_key,
      attempt,
      attempt_id,
    };
    transaction.commit()?;
    Ok(work)
  }

  pub fn complete_notification_update(
    &mut self,
    work: &NotificationUpdateWork,
    result: NotificationProviderUpdateResult,
    now: DateTime<Utc>,
  ) -> Result<RunProjection, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut events = load_events(&transaction, &work.run_id)?;
    let workflow = definition_for_run(&transaction, &work.run_id)?;
    let binding = run_binding_in_transaction(&transaction, &work.run_id)?;
    let projection = fold_events(&events)?;
    let update = projection
      .notification_updates
      .get(&work.delivery_id)
      .ok_or_else(|| {
        DurableStoreError::Contract("Unknown active notification update.".to_string())
      })?;
    if !matches!(&update.status, NotificationMessageUpdateStatus::AttemptStarted { attempt, attempt_id } if *attempt == work.attempt && attempt_id == &work.attempt_id)
    {
      return Err(DurableStoreError::Contract(
        "Notification update result does not match its active attempt.".to_string(),
      ));
    }
    let payload = match result {
      NotificationProviderUpdateResult::Succeeded => {
        RunEventPayload::NotificationMessageUpdated(NotificationMessageUpdatedData {
          approval_id: work.approval_id.clone(),
          request_id: work.request_id.clone(),
          delivery_id: work.delivery_id.clone(),
          update_id: work.update_id.clone(),
          attempt: work.attempt,
          attempt_id: work.attempt_id.clone(),
        })
      }
      NotificationProviderUpdateResult::Failed(failure) => {
        let final_ = !failure.retryable || work.attempt >= 3;
        RunEventPayload::NotificationMessageUpdateFailed(NotificationMessageUpdateFailedData {
          approval_id: work.approval_id.clone(),
          request_id: work.request_id.clone(),
          delivery_id: work.delivery_id.clone(),
          update_id: work.update_id.clone(),
          attempt: work.attempt,
          attempt_id: work.attempt_id.clone(),
          final_,
          failure,
        })
      }
    };
    validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
      .map_err(DurableStoreError::Contract)?;
    let version = projection
      .event_schema_version
      .unwrap_or(RUN_EVENT_SCHEMA_VERSION_V5);
    append_to_history(
      &transaction,
      &mut events,
      &work.run_id,
      generated_event_id(),
      now,
      version,
      payload,
    )?;
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = fold_events(&events)?;
    transaction.commit()?;
    Ok(projection)
  }

  pub fn dispatch_ready_notification_updates(
    &mut self,
    run_id: &str,
    now: DateTime<Utc>,
    adapter: &mut dyn NotificationProviderAdapter,
  ) -> Result<NotificationDispatchReport, DurableStoreError> {
    let projection = self.projection(run_id)?;
    let ready = projection
      .notification_updates
      .values()
      .filter(|update| {
        next_notification_update_attempt(&update.status, now)
          .ok()
          .flatten()
          .is_some()
      })
      .map(|update| update.delivery_id.clone())
      .collect::<Vec<_>>();
    let mut report = NotificationDispatchReport::default();
    for delivery_id in ready {
      let work = self.begin_notification_update(run_id, &delivery_id, now)?;
      let result = adapter.update(&work);
      let succeeded = matches!(result, NotificationProviderUpdateResult::Succeeded);
      self.complete_notification_update(&work, result, now)?;
      report.updates_attempted += 1;
      if succeeded {
        report.updates_succeeded += 1;
      } else {
        report.updates_failed += 1;
      }
    }
    Ok(report)
  }

  pub fn issue_approval_token(
    &mut self,
    run_id: &str,
    approval_id: &str,
    request_id: &str,
    issued_at: DateTime<Utc>,
  ) -> Result<IssuedApprovalToken, DurableStoreError> {
    let projection = self.projection(run_id)?;
    let request = projection
      .approval_requests
      .get(approval_id)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "Cannot issue a credential for an unknown approval request.".to_string(),
        )
      })?;
    if request.request_id != request_id
      || !matches!(request.status, ApprovalRequestStatus::Waiting)
      || projection.status != RunStatus::Waiting
    {
      return Err(DurableStoreError::Contract(
        "Approval credentials may be issued only for the matching unresolved request.".to_string(),
      ));
    }
    let credential_expires_at = approval_credential_expiry(issued_at, request.expires_at)?;

    let token_id = random_hex(16)?;
    let secret = random_hex(32)?;
    let secret_hash = Sha256::digest(secret.as_bytes());
    self.connection.execute(
      "INSERT INTO woml_approval_tokens(
         token_id, secret_hash, request_id, run_id, approval_id,
         issued_at, credential_expires_at
       ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
      params![
        token_id,
        secret_hash.as_slice(),
        request_id,
        run_id,
        approval_id,
        issued_at.to_rfc3339(),
        credential_expires_at.to_rfc3339(),
      ],
    )?;
    Ok(IssuedApprovalToken {
      token: format!("apr_{token_id}.{secret}"),
      token_id,
      request_id: request_id.to_string(),
      run_id: run_id.to_string(),
      approval_id: approval_id.to_string(),
      issued_at,
      credential_expires_at,
    })
  }

  pub fn request_approval_atomically(
    &mut self,
    run_id: &str,
    event_id: impl Into<String>,
    occurred_at: DateTime<Utc>,
    request: ApprovalRequestedData,
  ) -> Result<(RunProjection, IssuedApprovalToken), DurableStoreError> {
    let token_id = random_hex(16)?;
    let secret = random_hex(32)?;
    let secret_hash = Sha256::digest(secret.as_bytes());
    let credential_expires_at = approval_credential_expiry(occurred_at, request.expires_at)?;

    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let mut events = load_events(&transaction, run_id)?;
    let event_schema_version = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "An approval cannot be requested before run_started.".to_string(),
        )
      })?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    let notification_definitions = workflow
      .approval(&request.approval_id)
      .map(|approval| approval.notifications)
      .unwrap_or_default();
    let payload = RunEventPayload::ApprovalRequested(request.clone());
    validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
      .map_err(DurableStoreError::Contract)?;
    append_to_history(
      &transaction,
      &mut events,
      run_id,
      event_id.into(),
      occurred_at,
      event_schema_version,
      payload,
    )?;
    for notification in notification_definitions {
      let payload =
        RunEventPayload::NotificationDeliveryRequested(NotificationDeliveryRequestedData {
          approval_id: request.approval_id.clone(),
          request_id: request.request_id.clone(),
          delivery_id: notification.delivery_id,
          provider: notification.provider,
          destination: notification.destination,
        });
      validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
        .map_err(DurableStoreError::Contract)?;
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        occurred_at,
        event_schema_version,
        payload,
      )?;
    }
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = fold_events(&events)?;
    let folded_request = projection
      .approval_requests
      .get(&request.approval_id)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "The approval request did not enter the durable waiting projection.".to_string(),
        )
      })?;
    if folded_request.request_id != request.request_id
      || !matches!(folded_request.status, ApprovalRequestStatus::Waiting)
      || projection.status != RunStatus::Waiting
    {
      return Err(DurableStoreError::Contract(
        "The approval request did not produce the matching waiting state.".to_string(),
      ));
    }

    transaction.execute(
      "INSERT INTO woml_approval_tokens(
         token_id, secret_hash, request_id, run_id, approval_id,
         issued_at, credential_expires_at
       ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
      params![
        token_id,
        secret_hash.as_slice(),
        request.request_id,
        run_id,
        request.approval_id,
        occurred_at.to_rfc3339(),
        credential_expires_at.to_rfc3339(),
      ],
    )?;
    transaction.commit()?;

    Ok((
      projection,
      IssuedApprovalToken {
        token: format!("apr_{token_id}.{secret}"),
        token_id,
        request_id: request.request_id,
        run_id: run_id.to_string(),
        approval_id: request.approval_id,
        issued_at: occurred_at,
        credential_expires_at,
      },
    ))
  }

  pub fn resolve_human_approval(
    &mut self,
    token: &str,
    decision: ApprovalDecision,
    now: DateTime<Utc>,
  ) -> Result<ApprovalDecisionOutcome, DurableStoreError> {
    let (token_id, secret) = parse_approval_token(token)?;
    let candidate_hash = Sha256::digest(secret.as_bytes());
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let row: Option<(Vec<u8>, String, String, String, String)> = transaction
      .query_row(
        "SELECT secret_hash, request_id, run_id, approval_id, credential_expires_at
         FROM woml_approval_tokens WHERE token_id = ?1",
        [token_id],
        |row| {
          Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
          ))
        },
      )
      .optional()?;
    let Some((stored_hash, request_id, run_id, approval_id, credential_expires_at)) = row else {
      return Err(DurableStoreError::InvalidApprovalToken);
    };
    if stored_hash.len() != 32
      || candidate_hash
        .as_slice()
        .ct_eq(stored_hash.as_slice())
        .unwrap_u8()
        != 1
    {
      return Err(DurableStoreError::InvalidApprovalToken);
    }
    let credential_expires_at = parse_stored_timestamp(&credential_expires_at)?;

    let mut events = load_events(&transaction, &run_id)?;
    let workflow = definition_for_run(&transaction, &run_id)?;
    let binding = run_binding_in_transaction(&transaction, &run_id)?;
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = fold_events(&events)?;
    let request = projection
      .approval_requests
      .get(&approval_id)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "Approval credential references an unknown durable request.".to_string(),
        )
      })?;
    if request.request_id != request_id {
      return Err(DurableStoreError::Contract(
        "Approval credential does not match its durable request.".to_string(),
      ));
    }
    match &request.status {
      ApprovalRequestStatus::Resolved {
        resolution:
          ApprovalResolution::Decision {
            decision: existing,
            source: ApprovalDecisionSource::Human,
          },
        resolved_at,
      } if *existing == decision => {
        return Ok(ApprovalDecisionOutcome {
          status: ApprovalDecisionOutcomeStatus::AlreadyResolved,
          run_id,
          approval_id,
          request_id,
          decision,
          source: ApprovalDecisionSource::Human,
          decided_at: *resolved_at,
        });
      }
      ApprovalRequestStatus::Resolved {
        resolution:
          ApprovalResolution::Decision {
            source: ApprovalDecisionSource::Human,
            ..
          },
        ..
      } => return Err(DurableStoreError::ApprovalDecisionConflict),
      ApprovalRequestStatus::Resolved { .. } => {
        return Err(DurableStoreError::ApprovalExpired);
      }
      ApprovalRequestStatus::Waiting => {}
    }
    if request.expires_at.is_some_and(|deadline| now >= deadline) {
      return Err(DurableStoreError::ApprovalExpired);
    }
    if now >= credential_expires_at {
      return Err(DurableStoreError::ExpiredApprovalToken);
    }

    let mut payloads = vec![RunEventPayload::ApprovalResolved(ApprovalResolvedData {
      approval_id: approval_id.clone(),
      request_id: request_id.clone(),
      resolution: ApprovalResolution::Decision {
        decision,
        source: ApprovalDecisionSource::Human,
      },
    })];
    let notification_resolution = match decision {
      ApprovalDecision::Approved => crate::event::NotificationResolution::Approved,
      ApprovalDecision::Rejected => crate::event::NotificationResolution::Rejected,
    };
    for delivery in projection
      .notification_deliveries
      .values()
      .filter(|delivery| {
        delivery.approval_id == approval_id
          && delivery.request_id == request_id
          && matches!(
            delivery.status,
            NotificationDeliveryStatus::Succeeded { .. }
          )
      })
    {
      payloads.push(RunEventPayload::NotificationMessageUpdateRequested(
        crate::event::NotificationMessageUpdateRequestedData {
          approval_id: approval_id.clone(),
          request_id: request_id.clone(),
          delivery_id: delivery.delivery_id.clone(),
          update_id: format!("nupdate_{}", Uuid::new_v4().simple()),
          resolution: notification_resolution,
        },
      ));
    }
    let event_schema_version = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| {
        DurableStoreError::Contract("Approval resolution requires run_started.".to_string())
      })?;
    for payload in payloads {
      validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
        .map_err(DurableStoreError::Contract)?;
      append_to_history(
        &transaction,
        &mut events,
        &run_id,
        generated_event_id(),
        now,
        event_schema_version,
        payload,
      )?;
    }
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    transaction.commit()?;
    Ok(ApprovalDecisionOutcome {
      status: ApprovalDecisionOutcomeStatus::Accepted,
      run_id,
      approval_id,
      request_id,
      decision,
      source: ApprovalDecisionSource::Human,
      decided_at: now,
    })
  }

  pub fn settle_approval_timeout(
    &mut self,
    run_id: &str,
    approval_id: &str,
    now: DateTime<Utc>,
  ) -> Result<ApprovalTimeoutSettlement, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let mut events = load_events(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = fold_events(&events)?;
    let request = projection
      .approval_requests
      .get(approval_id)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "Timeout settlement references an unknown approval request.".to_string(),
        )
      })?;
    if let ApprovalRequestStatus::Resolved {
      resolution,
      resolved_at,
    } = &request.status
    {
      return Ok(ApprovalTimeoutSettlement {
        status: ApprovalTimeoutSettlementStatus::AlreadyResolved,
        run_id: run_id.to_string(),
        approval_id: approval_id.to_string(),
        request_id: request.request_id.clone(),
        resolution: Some(resolution.clone()),
        settled_at: Some(*resolved_at),
      });
    }
    let deadline = request
      .expires_at
      .ok_or(DurableStoreError::ApprovalHasNoDeadline)?;
    if now < deadline {
      return Ok(ApprovalTimeoutSettlement {
        status: ApprovalTimeoutSettlementStatus::NotDue,
        run_id: run_id.to_string(),
        approval_id: approval_id.to_string(),
        request_id: request.request_id.clone(),
        resolution: None,
        settled_at: None,
      });
    }
    let resolution = match request.on_timeout {
      ApprovalTimeoutPolicy::Reject => ApprovalResolution::Decision {
        decision: ApprovalDecision::Rejected,
        source: ApprovalDecisionSource::Timeout,
      },
      ApprovalTimeoutPolicy::Fail => ApprovalResolution::TimeoutFailure,
    };
    let mut payloads = vec![RunEventPayload::ApprovalResolved(ApprovalResolvedData {
      approval_id: approval_id.to_string(),
      request_id: request.request_id.clone(),
      resolution: resolution.clone(),
    })];
    let notification_resolution = match &resolution {
      ApprovalResolution::Decision {
        decision: ApprovalDecision::Rejected,
        ..
      } => crate::event::NotificationResolution::Rejected,
      ApprovalResolution::TimeoutFailure => crate::event::NotificationResolution::TimeoutFailed,
      ApprovalResolution::Decision {
        decision: ApprovalDecision::Approved,
        ..
      } => crate::event::NotificationResolution::Approved,
    };
    for delivery in projection
      .notification_deliveries
      .values()
      .filter(|delivery| {
        delivery.approval_id == approval_id
          && delivery.request_id == request.request_id
          && matches!(
            delivery.status,
            NotificationDeliveryStatus::Succeeded { .. }
          )
      })
    {
      payloads.push(RunEventPayload::NotificationMessageUpdateRequested(
        crate::event::NotificationMessageUpdateRequestedData {
          approval_id: approval_id.to_string(),
          request_id: request.request_id.clone(),
          delivery_id: delivery.delivery_id.clone(),
          update_id: format!("nupdate_{}", Uuid::new_v4().simple()),
          resolution: notification_resolution,
        },
      ));
    }
    if request.on_timeout == ApprovalTimeoutPolicy::Fail {
      payloads.push(RunEventPayload::RunFailed(RunFailedData::V4(
        RunFailedDataV4::Approval {
          approval_id: approval_id.to_string(),
          request_id: request.request_id.clone(),
          failure: ApprovalFailure {
            kind: "approval_timeout".to_string(),
            code: "WOML_APPROVAL_TIMEOUT".to_string(),
            message: format!("Approval {approval_id:?} reached its deadline."),
          },
        },
      )));
    }
    let event_schema_version = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| {
        DurableStoreError::Contract("Approval timeout requires run_started.".to_string())
      })?;
    for payload in payloads {
      validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
        .map_err(DurableStoreError::Contract)?;
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        now,
        event_schema_version,
        payload,
      )?;
    }
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    fold_events(&events)?;
    transaction.commit()?;
    Ok(ApprovalTimeoutSettlement {
      status: ApprovalTimeoutSettlementStatus::Settled,
      run_id: run_id.to_string(),
      approval_id: approval_id.to_string(),
      request_id: request.request_id.clone(),
      resolution: Some(resolution),
      settled_at: Some(now),
    })
  }

  pub fn verify_approval_token(
    &self,
    token: &str,
    now: DateTime<Utc>,
  ) -> Result<ApprovalTokenBinding, DurableStoreError> {
    let (token_id, secret) = parse_approval_token(token)?;
    let row: Option<(Vec<u8>, String, String, String, String, String)> = self
      .connection
      .query_row(
        "SELECT secret_hash, request_id, run_id, approval_id,
                issued_at, credential_expires_at
         FROM woml_approval_tokens WHERE token_id = ?1",
        [token_id],
        |row| {
          Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
          ))
        },
      )
      .optional()?;
    let Some((stored_hash, request_id, run_id, approval_id, issued_at, expires_at)) = row else {
      return Err(DurableStoreError::InvalidApprovalToken);
    };
    let candidate_hash = Sha256::digest(secret.as_bytes());
    if stored_hash.len() != 32
      || candidate_hash
        .as_slice()
        .ct_eq(stored_hash.as_slice())
        .unwrap_u8()
        != 1
    {
      return Err(DurableStoreError::InvalidApprovalToken);
    }
    let issued_at = parse_stored_timestamp(&issued_at)?;
    let credential_expires_at = parse_stored_timestamp(&expires_at)?;
    if now >= credential_expires_at {
      return Err(DurableStoreError::ExpiredApprovalToken);
    }
    Ok(ApprovalTokenBinding {
      token_id: token_id.to_string(),
      request_id,
      run_id,
      approval_id,
      issued_at,
      credential_expires_at,
    })
  }

  pub fn reissue_approval_token(
    &mut self,
    run_id: &str,
    approval_id: &str,
    request_id: &str,
    issued_at: DateTime<Utc>,
  ) -> Result<IssuedApprovalToken, DurableStoreError> {
    self.issue_approval_token(run_id, approval_id, request_id, issued_at)
  }

  pub fn approval_token_count_for_request(
    &self,
    run_id: &str,
    approval_id: &str,
    request_id: &str,
  ) -> Result<usize, DurableStoreError> {
    let count: i64 = self.connection.query_row(
      "SELECT COUNT(*) FROM woml_approval_tokens
       WHERE run_id = ?1 AND approval_id = ?2 AND request_id = ?3",
      params![run_id, approval_id, request_id],
      |row| row.get(0),
    )?;
    usize::try_from(count)
      .map_err(|_| DurableStoreError::Contract("Approval credential count is invalid.".to_string()))
  }

  pub fn recover_interrupted_runs(&mut self) -> Result<RecoveryReport, DurableStoreError> {
    let run_ids = self.run_ids()?;
    let mut report = RecoveryReport {
      inspected_runs: run_ids.len(),
      ..RecoveryReport::default()
    };
    for run_id in run_ids {
      match self.recover_run(&run_id)? {
        RunRecovery::Recovered {
          interrupted_attempts,
        } => {
          report.recovered_runs += 1;
          report.interrupted_attempts += interrupted_attempts;
        }
        RunRecovery::Resumable => {
          report.resumable_runs += 1;
        }
        RunRecovery::Unchanged => {}
      }
    }
    Ok(report)
  }

  fn run_ids(&self) -> Result<Vec<String>, DurableStoreError> {
    let mut statement = self
      .connection
      .prepare("SELECT run_id FROM woml_runs ORDER BY created_at, run_id")?;
    let rows = statement.query_map([], |row| row.get(0))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
  }

  fn recover_run(&mut self, run_id: &str) -> Result<RunRecovery, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let mut events = load_events(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = fold_events(&events)?;
    let event_schema_version = projection.event_schema_version.ok_or_else(|| {
      DurableStoreError::Contract("A stored run has no event schema version.".to_string())
    })?;
    if projection.status == RunStatus::Waiting {
      let started = projection
        .notification_deliveries
        .values()
        .filter_map(|delivery| {
          if let NotificationDeliveryStatus::AttemptStarted {
            attempt,
            attempt_id,
            ..
          } = &delivery.status
          {
            Some((delivery.clone(), *attempt, attempt_id.clone()))
          } else {
            None
          }
        })
        .collect::<Vec<_>>();
      if started.is_empty() {
        return Ok(RunRecovery::Unchanged);
      }
      let now = Utc::now();
      for (delivery, attempt, attempt_id) in &started {
        append_to_history(
          &transaction,
          &mut events,
          run_id,
          generated_event_id(),
          now,
          event_schema_version,
          RunEventPayload::NotificationDeliveryFailed(NotificationDeliveryFailedData {
            approval_id: delivery.approval_id.clone(),
            request_id: delivery.request_id.clone(),
            delivery_id: delivery.delivery_id.clone(),
            attempt: *attempt,
            attempt_id: attempt_id.clone(),
            final_: true,
            failure: NotificationSafeFailure {
              kind: "delivery_ambiguous".to_string(),
              code: "WOML_NOTIFICATION_DELIVERY_AMBIGUOUS".to_string(),
              message: "Recovery found an uncertain notification send and will not replay it."
                .to_string(),
              retryable: false,
              retry_after_ms: None,
            },
          }),
        )?;
      }
      let mut recovered = fold_events(&events)?;
      for (approval_id, request_id) in started
        .iter()
        .map(|(delivery, _, _)| (&delivery.approval_id, &delivery.request_id))
      {
        if recovered.status == RunStatus::Waiting
          && notification_request_all_failed(&workflow, &recovered, approval_id, request_id)
        {
          let ids = workflow
            .approval(approval_id)
            .unwrap()
            .notifications
            .into_iter()
            .map(|item| item.delivery_id)
            .collect();
          append_to_history(
            &transaction,
            &mut events,
            run_id,
            generated_event_id(),
            now,
            event_schema_version,
            RunEventPayload::RunFailed(RunFailedData::V5(RunFailedDataV5::Notification {
              approval_id: approval_id.clone(),
              request_id: request_id.clone(),
              failed_delivery_ids: ids,
              failure: NotificationRunFailure {
                kind: "all_deliveries_failed".to_string(),
                code: "WOML_NOTIFICATION_DELIVERY_FAILED".to_string(),
                message: "Every configured approval notification delivery failed.".to_string(),
              },
            })),
          )?;
          recovered = fold_events(&events)?;
          break;
        }
      }
      validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
        .map_err(DurableStoreError::Contract)?;
      transaction.commit()?;
      let _ = recovered;
      return Ok(RunRecovery::Recovered {
        interrupted_attempts: 0,
      });
    }
    let started_updates = projection
      .notification_updates
      .values()
      .filter_map(|update| {
        if let NotificationMessageUpdateStatus::AttemptStarted {
          attempt,
          attempt_id,
        } = &update.status
        {
          Some((update.clone(), *attempt, attempt_id.clone()))
        } else {
          None
        }
      })
      .collect::<Vec<_>>();
    if !started_updates.is_empty() {
      let now = Utc::now();
      for (update, attempt, attempt_id) in &started_updates {
        append_to_history(
          &transaction,
          &mut events,
          run_id,
          generated_event_id(),
          now,
          event_schema_version,
          RunEventPayload::NotificationMessageUpdateFailed(
            NotificationMessageUpdateFailedData {
              approval_id: update.approval_id.clone(),
              request_id: update.request_id.clone(),
              delivery_id: update.delivery_id.clone(),
              update_id: update.update_id.clone(),
              attempt: *attempt,
              attempt_id: attempt_id.clone(),
              final_: *attempt >= 3,
              failure: NotificationSafeFailure {
                kind: "update_failed".to_string(),
                code: "WOML_NOTIFICATION_UPDATE_INTERRUPTED".to_string(),
                message: "Recovery found an interrupted message update; the durable update remains retryable."
                  .to_string(),
                retryable: *attempt < 3,
                retry_after_ms: None,
              },
            },
          ),
        )?;
      }
      validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
        .map_err(DurableStoreError::Contract)?;
      transaction.commit()?;
      return Ok(RunRecovery::Recovered {
        interrupted_attempts: 0,
      });
    }
    if projection.status != RunStatus::Running {
      return Ok(RunRecovery::Unchanged);
    }
    let active_managed_operations = projection
      .operations
      .values()
      .filter(|operation| {
        operation.execution_mode == crate::event::OperationExecutionMode::Managed
          && operation.status == crate::projection::OperationStatus::Started
      })
      .cloned()
      .collect::<Vec<_>>();
    if !active_managed_operations.is_empty() {
      let now = Utc::now();
      for operation in &active_managed_operations {
        append_to_history(
          &transaction,
          &mut events,
          run_id,
          generated_event_id(),
          now,
          event_schema_version,
          RunEventPayload::OperationFailed(OperationFailedData {
            node_id: operation.node_id.clone(),
            attempt_number: operation.attempt_number,
            invocation_id: operation.identity.invocation_id.clone(),
            call_id: operation.identity.call_id.clone(),
            operation_key: operation.operation_key.clone(),
            capability: operation.capability.clone(),
            operation: operation.operation.clone(),
            execution_mode: operation.execution_mode,
            metadata: operation.metadata.clone(),
            duration_ms: 0.0,
            failure: crate::CapabilityFailure {
              kind: crate::CapabilityFailureKind::Interrupted,
              code: "WOML_CAPABILITY_INTERRUPTED".to_string(),
              message: "Recovery found a managed operation without a durable terminal event; it will not be replayed.".to_string(),
              retryable: false,
              ambiguous: true,
              details: None,
            },
          }),
        )?;
      }
    }
    let started = projection
      .attempts
      .iter()
      .filter(|attempt| attempt.status == AttemptStatus::Started)
      .map(|attempt| attempt.identity.clone())
      .collect::<Vec<_>>();
    if let Some(first) = started.first().cloned() {
      let failure = interrupted_failure();
      for attempt in &started {
        append_to_history(
          &transaction,
          &mut events,
          run_id,
          generated_event_id(),
          Utc::now(),
          event_schema_version,
          RunEventPayload::StepAttemptFailed(StepAttemptFailedData {
            node_id: attempt.node_id.clone(),
            attempt: attempt.attempt,
            invocation_id: attempt.invocation_id.clone(),
            failure: failure.clone(),
          }),
        )?;
      }
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        Utc::now(),
        event_schema_version,
        RunEventPayload::RunFailed(attempt_run_failed_data(
          event_schema_version,
          first.node_id,
          first.attempt,
          first.invocation_id,
          failure,
        )),
      )?;
      validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
        .map_err(DurableStoreError::Contract)?;
      transaction.commit()?;
      return Ok(RunRecovery::Recovered {
        interrupted_attempts: started.len(),
      });
    }

    for group_projection in projection.parallel_groups.values() {
      if group_projection.status != ParallelGroupStatus::Started {
        continue;
      }
      let group = workflow
        .parallel_group(&group_projection.parallel_id)
        .ok_or_else(|| {
          DurableStoreError::Contract(format!(
            "Stored run references unknown parallel group {:?}.",
            group_projection.parallel_id
          ))
        })?;
      let child_statuses = group
        .child_node_ids
        .iter()
        .map(|node_id| {
          (
            node_id,
            projection
              .attempts
              .iter()
              .rev()
              .find(|attempt| attempt.identity.node_id == *node_id)
              .map(|attempt| &attempt.status),
          )
        })
        .collect::<Vec<_>>();
      let every_succeeded = child_statuses
        .iter()
        .all(|(_, status)| matches!(status, Some(AttemptStatus::Succeeded { .. })));
      if every_succeeded {
        append_to_history(
          &transaction,
          &mut events,
          run_id,
          generated_event_id(),
          Utc::now(),
          event_schema_version,
          RunEventPayload::ParallelGroupCompleted(ParallelGroupCompletedData {
            parallel_id: group.parallel_id.clone(),
            outcome: ParallelGroupOutcome::Succeeded,
            failed_node_ids: Vec::new(),
            cancelled_node_ids: Vec::new(),
          }),
        )?;
        validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
          .map_err(DurableStoreError::Contract)?;
        transaction.commit()?;
        return Ok(RunRecovery::Recovered {
          interrupted_attempts: 0,
        });
      }

      let every_terminal = child_statuses.iter().all(|(node_id, status)| {
        matches!(
          status,
          Some(AttemptStatus::Succeeded { .. } | AttemptStatus::Failed { .. })
        ) && !projection.pending_retries.contains_key(*node_id)
      });
      let failed_node_ids = child_statuses
        .iter()
        .filter_map(|(node_id, status)| match status {
          Some(AttemptStatus::Failed { failure })
            if failure.kind != AttemptFailureKind::InvocationCancelled
              && !projection.pending_retries.contains_key(*node_id) =>
          {
            Some((*node_id).clone())
          }
          _ => None,
        })
        .collect::<Vec<_>>();
      if failed_node_ids.is_empty() || (group.on_error == "wait-all" && !every_terminal) {
        continue;
      }
      let cancelled_node_ids = child_statuses
        .iter()
        .filter_map(|(node_id, status)| match status {
          Some(AttemptStatus::Failed { failure })
            if failure.kind == AttemptFailureKind::InvocationCancelled =>
          {
            Some((*node_id).clone())
          }
          _ => None,
        })
        .collect::<Vec<_>>();
      let primary_node_id = events
        .iter()
        .find_map(|event| match &event.payload {
          RunEventPayload::StepAttemptFailed(data) if failed_node_ids.contains(&data.node_id) => {
            Some(data.node_id.clone())
          }
          _ => None,
        })
        .ok_or_else(|| {
          DurableStoreError::Contract(format!(
            "Parallel group {:?} has failed children without a failure event.",
            group.parallel_id
          ))
        })?;
      let policy = if group.on_error == "wait-all" {
        ParallelFailurePolicy::WaitAll
      } else {
        ParallelFailurePolicy::FailFast
      };
      let message = parallel_failure_message(policy, failed_node_ids.len());
      let failure = ParallelFailure {
        kind: "parallel_child_failed".to_string(),
        code: "WOML_PARALLEL_CHILD_FAILED".to_string(),
        message,
      };
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        Utc::now(),
        event_schema_version,
        RunEventPayload::ParallelGroupCompleted(ParallelGroupCompletedData {
          parallel_id: group.parallel_id.clone(),
          outcome: ParallelGroupOutcome::Failed,
          failed_node_ids: failed_node_ids.clone(),
          cancelled_node_ids: cancelled_node_ids.clone(),
        }),
      )?;
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        Utc::now(),
        event_schema_version,
        RunEventPayload::RunFailed(RunFailedData::V3(RunFailedDataV3::Parallel {
          parallel_id: group.parallel_id.clone(),
          policy,
          primary_node_id,
          failed_node_ids,
          cancelled_node_ids,
          failure,
        })),
      )?;
      validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
        .map_err(DurableStoreError::Contract)?;
      transaction.commit()?;
      return Ok(RunRecovery::Recovered {
        interrupted_attempts: 0,
      });
    }

    // A failed attempt followed by a durable retry schedule is unfinished work,
    // not a terminal failure. The schedule is the persistence boundary that
    // makes starting the next attempt safe after a process restart.
    if !projection.pending_retries.is_empty() {
      return Ok(RunRecovery::Resumable);
    }

    // Only the latest attempt for a node can determine its current state. An
    // older failure must never override a later successful retry.
    let failed_attempt = projection.attempts.iter().rev().find_map(|attempt| {
      let is_latest_for_node = projection
        .attempts
        .iter()
        .rev()
        .find(|candidate| candidate.identity.node_id == attempt.identity.node_id)
        .is_some_and(|latest| latest.identity == attempt.identity);
      if is_latest_for_node {
        if let AttemptStatus::Failed { failure } = &attempt.status {
          return Some((attempt.identity.clone(), failure.clone()));
        }
      }
      None
    });
    if let Some((identity, failure)) = failed_attempt {
      if workflow
        .parallel_group_for_child(&identity.node_id)
        .is_some_and(|group| projection.parallel_groups.contains_key(&group.parallel_id))
      {
        return Ok(RunRecovery::Resumable);
      }
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        Utc::now(),
        event_schema_version,
        RunEventPayload::RunFailed(attempt_run_failed_data(
          event_schema_version,
          identity.node_id,
          identity.attempt,
          identity.invocation_id,
          failure,
        )),
      )?;
      transaction.commit()?;
      return Ok(RunRecovery::Recovered {
        interrupted_attempts: 0,
      });
    }

    let terminal_node_id = workflow.terminal_node_id().ok_or_else(|| {
      DurableStoreError::Contract("Stored workflow has no terminal node.".to_string())
    })?;
    if let Some(result) = projection.context.steps.get(terminal_node_id).cloned() {
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        Utc::now(),
        event_schema_version,
        RunEventPayload::RunSucceeded(RunSucceededData {
          terminal_node_id: terminal_node_id.to_string(),
          result,
        }),
      )?;
      transaction.commit()?;
      return Ok(RunRecovery::Recovered {
        interrupted_attempts: 0,
      });
    }

    Ok(RunRecovery::Resumable)
  }
}

fn random_hex(byte_count: usize) -> Result<String, DurableStoreError> {
  let mut bytes = vec![0_u8; byte_count];
  getrandom(&mut bytes).map_err(|_| {
    DurableStoreError::Contract(
      "The operating system could not generate a secure approval credential.".to_string(),
    )
  })?;
  Ok(hex::encode(bytes))
}

fn notification_idempotency_key(run_id: &str, request_id: &str, delivery_id: &str) -> String {
  let mut digest = Sha256::new();
  digest.update(b"woml.notification.delivery.v1\0");
  digest.update(run_id.as_bytes());
  digest.update(b"\0");
  digest.update(request_id.as_bytes());
  digest.update(b"\0");
  digest.update(delivery_id.as_bytes());
  format!("sha256:{}", hex::encode(digest.finalize()))
}

fn notification_update_idempotency_key(
  run_id: &str,
  request_id: &str,
  delivery_id: &str,
  update_id: &str,
) -> String {
  let mut digest = Sha256::new();
  digest.update(b"woml.notification.update.v1\0");
  digest.update(run_id.as_bytes());
  digest.update(b"\0");
  digest.update(request_id.as_bytes());
  digest.update(b"\0");
  digest.update(delivery_id.as_bytes());
  digest.update(b"\0");
  digest.update(update_id.as_bytes());
  format!("sha256:{}", hex::encode(digest.finalize()))
}

fn next_notification_attempt(
  status: &NotificationDeliveryStatus,
  now: DateTime<Utc>,
) -> Result<Option<u32>, DurableStoreError> {
  match status {
    NotificationDeliveryStatus::Requested => Ok(Some(1)),
    NotificationDeliveryStatus::Failed {
      attempt,
      final_: false,
      failure,
      failed_at,
      ..
    } if failure.retryable && *attempt < 3 => {
      let scheduled_delay = if *attempt == 1 { 1_000 } else { 5_000 };
      let delay_ms = scheduled_delay.max(failure.retry_after_ms.unwrap_or(0));
      let delay = i64::try_from(delay_ms)
        .map(chrono::Duration::milliseconds)
        .map_err(|_| {
          DurableStoreError::Contract("Notification retry delay is invalid.".to_string())
        })?;
      Ok((now >= *failed_at + delay).then_some(*attempt + 1))
    }
    _ => Ok(None),
  }
}

fn notification_request_all_failed(
  workflow: &CompiledWorkflowDefinition,
  projection: &RunProjection,
  approval_id: &str,
  request_id: &str,
) -> bool {
  let Some(approval) = workflow.approval(approval_id) else {
    return false;
  };
  !approval.notifications.is_empty()
    && approval.notifications.iter().all(|definition| {
      projection
        .notification_deliveries
        .get(&definition.delivery_id)
        .is_some_and(|delivery| {
          delivery.request_id == request_id
            && matches!(
              delivery.status,
              NotificationDeliveryStatus::Failed { final_: true, .. }
            )
        })
    })
}

fn next_notification_update_attempt(
  status: &NotificationMessageUpdateStatus,
  now: DateTime<Utc>,
) -> Result<Option<u32>, DurableStoreError> {
  match status {
    NotificationMessageUpdateStatus::Requested => Ok(Some(1)),
    NotificationMessageUpdateStatus::Failed {
      attempt,
      final_: false,
      failure,
      failed_at,
      ..
    } if failure.retryable && *attempt < 3 => {
      let scheduled_delay = if *attempt == 1 { 1_000 } else { 5_000 };
      let delay_ms = scheduled_delay.max(failure.retry_after_ms.unwrap_or(0));
      let delay = i64::try_from(delay_ms)
        .map(chrono::Duration::milliseconds)
        .map_err(|_| {
          DurableStoreError::Contract("Notification update retry delay is invalid.".to_string())
        })?;
      Ok((now >= *failed_at + delay).then_some(*attempt + 1))
    }
    _ => Ok(None),
  }
}

fn approval_credential_expiry(
  issued_at: DateTime<Utc>,
  approval_deadline: Option<DateTime<Utc>>,
) -> Result<DateTime<Utc>, DurableStoreError> {
  let default_expiry = issued_at
    .checked_add_signed(chrono::Duration::hours(
      DEFAULT_APPROVAL_CREDENTIAL_LIFETIME_HOURS,
    ))
    .ok_or_else(|| {
      DurableStoreError::Contract("Approval credential expiry exceeds the clock range.".to_string())
    })?;
  let credential_expires_at =
    approval_deadline.map_or(default_expiry, |deadline| deadline.min(default_expiry));
  if credential_expires_at <= issued_at {
    return Err(DurableStoreError::Contract(
      "Cannot issue an approval credential at or after the request deadline.".to_string(),
    ));
  }
  Ok(credential_expires_at)
}

fn parse_approval_token(token: &str) -> Result<(&str, &str), DurableStoreError> {
  let Some(body) = token.strip_prefix("apr_") else {
    return Err(DurableStoreError::InvalidApprovalToken);
  };
  let Some((token_id, secret)) = body.split_once('.') else {
    return Err(DurableStoreError::InvalidApprovalToken);
  };
  if token_id.len() != 32
    || secret.len() != 64
    || token_id.contains('.')
    || secret.contains('.')
    || !token_id.bytes().all(|byte| byte.is_ascii_hexdigit())
    || !secret.bytes().all(|byte| byte.is_ascii_hexdigit())
  {
    return Err(DurableStoreError::InvalidApprovalToken);
  }
  Ok((token_id, secret))
}

fn parse_notification_capability(token: &str) -> Result<(&str, &str), DurableStoreError> {
  let Some(token) = token.strip_prefix("ncap_") else {
    return Err(DurableStoreError::InvalidApprovalToken);
  };
  let Some((id, secret)) = token.split_once('.') else {
    return Err(DurableStoreError::InvalidApprovalToken);
  };
  if id.len() != 32
    || secret.len() != 64
    || !id
      .bytes()
      .chain(secret.bytes())
      .all(|byte| byte.is_ascii_hexdigit())
  {
    return Err(DurableStoreError::InvalidApprovalToken);
  }
  Ok((id, secret))
}

fn parse_stored_timestamp(value: &str) -> Result<DateTime<Utc>, DurableStoreError> {
  DateTime::parse_from_rfc3339(value)
    .map(|timestamp| timestamp.with_timezone(&Utc))
    .map_err(|_| {
      DurableStoreError::Contract("Stored approval credential timestamp is invalid.".to_string())
    })
}

fn ensure_run_exists(connection: &Connection, run_id: &str) -> Result<(), DurableStoreError> {
  let exists: bool = connection.query_row(
    "SELECT EXISTS(SELECT 1 FROM woml_runs WHERE run_id = ?1)",
    [run_id],
    |row| row.get(0),
  )?;
  if !exists {
    return Err(DurableStoreError::RunNotFound(run_id.to_string()));
  }
  Ok(())
}

fn validate_trigger_admission_request(
  request: &TriggerAdmissionRequest,
) -> Result<(), DurableStoreError> {
  if request.source_identity.is_empty() || request.source_identity.len() > 512 {
    return Err(DurableStoreError::Contract(
      "Trigger sourceIdentity must contain between 1 and 512 UTF-8 bytes.".to_string(),
    ));
  }
  if !is_definition_hash(&request.definition_hash) {
    return Err(DurableStoreError::TriggerDefinitionMismatch);
  }
  Ok(())
}

fn admit_trigger_occurrence_in_transaction(
  transaction: &Transaction<'_>,
  request: &TriggerAdmissionRequest,
) -> Result<TriggerAdmissionOutcome, DurableStoreError> {
  let source_identity_hash = sha256_prefixed(request.source_identity.as_bytes());
  let payload_hash = canonical_payload_hash(&request.payload)?;
  let model_json: String = transaction
    .query_row(
      "SELECT model_json FROM woml_definitions WHERE definition_hash = ?1",
      [&request.definition_hash],
      |row| row.get(0),
    )
    .optional()?
    .ok_or_else(|| DurableStoreError::DefinitionNotFound(request.definition_hash.clone()))?;
  let workflow: CompiledWorkflowDefinition = serde_json::from_str(&model_json)?;
  workflow.validate_structure()?;
  if workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V7
    || workflow.workflow_id != request.workflow_id
  {
    return Err(DurableStoreError::TriggerDefinitionMismatch);
  }
  let trigger =
    workflow
      .trigger(&request.trigger_id)
      .ok_or_else(|| DurableStoreError::TriggerNotFound {
        workflow_id: request.workflow_id.clone(),
        trigger_id: request.trigger_id.clone(),
      })?;
  if trigger.handler != request.trigger_handler {
    return Err(DurableStoreError::TriggerHandlerMismatch);
  }

  if let Some(existing) = load_trigger_occurrence_by_identity(
    transaction,
    &request.workflow_id,
    &request.trigger_id,
    &source_identity_hash,
  )? {
    if existing.payload_hash != payload_hash {
      return Err(DurableStoreError::TriggerIdempotencyConflict);
    }
    validate_trigger_occurrence_history(transaction, &existing)?;
    return Ok(TriggerAdmissionOutcome {
      occurrence_id: existing.occurrence_id,
      run_id: existing.run_id,
      duplicate: true,
    });
  }

  let occurrence_id = format!("occ_{}", Uuid::new_v4().simple());
  let run_id = format!("run_{}", Uuid::new_v4().simple());
  transaction.execute(
    "INSERT INTO woml_runs(run_id, workflow_id, definition_hash, created_at)
     VALUES (?1, ?2, ?3, ?4)",
    params![
      run_id,
      request.workflow_id,
      request.definition_hash,
      request.received_at.to_rfc3339(),
    ],
  )?;

  let payload = RunEventPayload::RunStarted(RunStartedData {
    workflow_id: request.workflow_id.clone(),
    definition_hash: request.definition_hash.clone(),
    trigger_id: Some(request.trigger_id.clone()),
    trigger_handler: Some(request.trigger_handler.clone()),
    trigger_occurrence_id: Some(occurrence_id.clone()),
    trigger: request.payload.clone(),
  });
  validate_payload_against_definition(&workflow, &request.definition_hash, &payload)
    .map_err(DurableStoreError::Contract)?;
  let mut events = Vec::new();
  append_to_history(
    transaction,
    &mut events,
    &run_id,
    generated_event_id(),
    request.received_at,
    run_event_schema_version_for_model(workflow.schema_version),
    payload,
  )?;
  validate_event_history_against_definition(&workflow, &request.definition_hash, &events)
    .map_err(DurableStoreError::Contract)?;

  transaction.execute(
    "INSERT INTO woml_trigger_occurrences(
       occurrence_id, workflow_id, trigger_id, trigger_handler,
       definition_hash, source_identity_hash, payload_hash, received_at, run_id
     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    params![
      occurrence_id,
      request.workflow_id,
      request.trigger_id,
      request.trigger_handler,
      request.definition_hash,
      source_identity_hash,
      payload_hash,
      request.received_at.to_rfc3339(),
      run_id,
    ],
  )?;
  Ok(TriggerAdmissionOutcome {
    occurrence_id,
    run_id,
    duplicate: false,
  })
}

fn validate_schedule_cursor_registration(
  registration: &ScheduleCursorRegistration,
) -> Result<(), DurableStoreError> {
  if registration.workflow_id.is_empty()
    || registration.trigger_id.is_empty()
    || registration.cron.is_empty()
    || registration.timezone.is_empty()
    || !matches!(registration.on_missed.as_str(), "skip" | "run-once")
    || !is_definition_hash(&registration.definition_hash)
  {
    return Err(DurableStoreError::Contract(
      "Schedule cursor registration is invalid.".to_string(),
    ));
  }
  Ok(())
}

fn validate_schedule_registration_definition(
  transaction: &Transaction<'_>,
  registration: &ScheduleCursorRegistration,
) -> Result<(), DurableStoreError> {
  let model_json: String = transaction
    .query_row(
      "SELECT model_json FROM woml_definitions WHERE definition_hash = ?1",
      [&registration.definition_hash],
      |row| row.get(0),
    )
    .optional()?
    .ok_or_else(|| DurableStoreError::DefinitionNotFound(registration.definition_hash.clone()))?;
  let workflow: CompiledWorkflowDefinition = serde_json::from_str(&model_json)?;
  if workflow.workflow_id != registration.workflow_id {
    return Err(DurableStoreError::TriggerDefinitionMismatch);
  }
  let trigger = workflow.trigger(&registration.trigger_id).ok_or_else(|| {
    DurableStoreError::TriggerNotFound {
      workflow_id: registration.workflow_id.clone(),
      trigger_id: registration.trigger_id.clone(),
    }
  })?;
  if trigger.handler != "trigger.schedule" {
    return Err(DurableStoreError::TriggerHandlerMismatch);
  }
  let crate::model::ValueExpression::Object { fields } = &trigger.config else {
    return Err(DurableStoreError::TriggerDefinitionMismatch);
  };
  let literal = |name: &str| match fields.get(name) {
    Some(crate::model::ValueExpression::Literal { value }) => value.as_str(),
    _ => None,
  };
  if literal("cron") != Some(registration.cron.as_str())
    || literal("timezone") != Some(registration.timezone.as_str())
    || literal("onMissed") != Some(registration.on_missed.as_str())
  {
    return Err(DurableStoreError::TriggerDefinitionMismatch);
  }
  Ok(())
}

fn load_schedule_cursor(
  connection: &Connection,
  workflow_id: &str,
  trigger_id: &str,
) -> Result<Option<ScheduleCursor>, DurableStoreError> {
  let stored = connection
    .query_row(
      "SELECT workflow_id, trigger_id, definition_hash, cron, timezone, on_missed,
              next_scheduled_at, updated_at
       FROM woml_schedule_cursors
       WHERE workflow_id = ?1 AND trigger_id = ?2",
      params![workflow_id, trigger_id],
      |row| {
        Ok((
          row.get::<_, String>(0)?,
          row.get::<_, String>(1)?,
          row.get::<_, String>(2)?,
          row.get::<_, String>(3)?,
          row.get::<_, String>(4)?,
          row.get::<_, String>(5)?,
          row.get::<_, String>(6)?,
          row.get::<_, String>(7)?,
        ))
      },
    )
    .optional()?;
  stored
    .map(|stored| {
      Ok(ScheduleCursor {
        workflow_id: stored.0,
        trigger_id: stored.1,
        definition_hash: stored.2,
        cron: stored.3,
        timezone: stored.4,
        on_missed: stored.5,
        next_scheduled_at: parse_durable_timestamp(&stored.6, "schedule next instant")?,
        updated_at: parse_durable_timestamp(&stored.7, "schedule update instant")?,
      })
    })
    .transpose()
}

fn update_schedule_cursor(
  transaction: &Transaction<'_>,
  workflow_id: &str,
  trigger_id: &str,
  expected_next_scheduled_at: DateTime<Utc>,
  next_scheduled_at: DateTime<Utc>,
  updated_at: DateTime<Utc>,
) -> Result<(), DurableStoreError> {
  if next_scheduled_at <= expected_next_scheduled_at {
    return Err(DurableStoreError::Contract(
      "A schedule cursor must advance to a later planned instant.".to_string(),
    ));
  }
  let changed = transaction.execute(
    "UPDATE woml_schedule_cursors
     SET next_scheduled_at = ?1, updated_at = ?2
     WHERE workflow_id = ?3 AND trigger_id = ?4 AND next_scheduled_at = ?5",
    params![
      next_scheduled_at.to_rfc3339(),
      updated_at.to_rfc3339(),
      workflow_id,
      trigger_id,
      expected_next_scheduled_at.to_rfc3339(),
    ],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::ScheduleCursorConflict);
  }
  Ok(())
}

fn validate_interval_cursor_registration(
  registration: &IntervalCursorRegistration,
) -> Result<(), DurableStoreError> {
  if registration.workflow_id.is_empty()
    || registration.trigger_id.is_empty()
    || !matches!(registration.on_missed.as_str(), "skip" | "run-once")
    || !is_definition_hash(&registration.definition_hash)
    || crate::interval::WomlInterval::new(registration.every_ms).is_err()
  {
    return Err(DurableStoreError::Contract(
      "Interval cursor registration is invalid.".to_string(),
    ));
  }
  Ok(())
}

fn validate_interval_registration_definition(
  transaction: &Transaction<'_>,
  registration: &IntervalCursorRegistration,
) -> Result<(), DurableStoreError> {
  let model_json: String = transaction
    .query_row(
      "SELECT model_json FROM woml_definitions WHERE definition_hash = ?1",
      [&registration.definition_hash],
      |row| row.get(0),
    )
    .optional()?
    .ok_or_else(|| DurableStoreError::DefinitionNotFound(registration.definition_hash.clone()))?;
  let workflow: CompiledWorkflowDefinition = serde_json::from_str(&model_json)?;
  if workflow.workflow_id != registration.workflow_id {
    return Err(DurableStoreError::TriggerDefinitionMismatch);
  }
  let trigger = workflow.trigger(&registration.trigger_id).ok_or_else(|| {
    DurableStoreError::TriggerNotFound {
      workflow_id: registration.workflow_id.clone(),
      trigger_id: registration.trigger_id.clone(),
    }
  })?;
  if trigger.handler != "trigger.interval" {
    return Err(DurableStoreError::TriggerHandlerMismatch);
  }
  let crate::model::ValueExpression::Object { fields } = &trigger.config else {
    return Err(DurableStoreError::TriggerDefinitionMismatch);
  };
  let every_ms = match fields.get("everyMs") {
    Some(crate::model::ValueExpression::Literal { value }) => value.as_u64(),
    _ => None,
  };
  let on_missed = match fields.get("onMissed") {
    Some(crate::model::ValueExpression::Literal { value }) => value.as_str(),
    _ => None,
  };
  if every_ms != Some(registration.every_ms) || on_missed != Some(registration.on_missed.as_str()) {
    return Err(DurableStoreError::TriggerDefinitionMismatch);
  }
  Ok(())
}

fn load_interval_cursor(
  connection: &Connection,
  workflow_id: &str,
  trigger_id: &str,
) -> Result<Option<IntervalCursor>, DurableStoreError> {
  let stored = connection
    .query_row(
      "SELECT workflow_id, trigger_id, definition_hash, every_ms, on_missed,
              anchor_at, next_sequence, next_scheduled_at, updated_at
       FROM woml_interval_cursors
       WHERE workflow_id = ?1 AND trigger_id = ?2",
      params![workflow_id, trigger_id],
      |row| {
        Ok((
          row.get::<_, String>(0)?,
          row.get::<_, String>(1)?,
          row.get::<_, String>(2)?,
          row.get::<_, i64>(3)?,
          row.get::<_, String>(4)?,
          row.get::<_, String>(5)?,
          row.get::<_, i64>(6)?,
          row.get::<_, String>(7)?,
          row.get::<_, String>(8)?,
        ))
      },
    )
    .optional()?;
  stored
    .map(|stored| {
      Ok(IntervalCursor {
        workflow_id: stored.0,
        trigger_id: stored.1,
        definition_hash: stored.2,
        every_ms: u64::try_from(stored.3).map_err(|_| {
          DurableStoreError::Contract("Stored interval duration is invalid.".to_string())
        })?,
        on_missed: stored.4,
        anchor_at: parse_durable_timestamp(&stored.5, "interval anchor")?,
        next_sequence: u64::try_from(stored.6).map_err(|_| {
          DurableStoreError::Contract("Stored interval sequence is invalid.".to_string())
        })?,
        next_scheduled_at: parse_durable_timestamp(&stored.7, "interval next instant")?,
        updated_at: parse_durable_timestamp(&stored.8, "interval update instant")?,
      })
    })
    .transpose()
}

#[allow(clippy::too_many_arguments)]
fn update_interval_cursor(
  transaction: &Transaction<'_>,
  workflow_id: &str,
  trigger_id: &str,
  expected_sequence: u64,
  expected_scheduled_at: DateTime<Utc>,
  next_sequence: u64,
  next_scheduled_at: DateTime<Utc>,
  updated_at: DateTime<Utc>,
) -> Result<(), DurableStoreError> {
  if next_sequence <= expected_sequence || next_scheduled_at <= expected_scheduled_at {
    return Err(DurableStoreError::Contract(
      "An interval cursor must advance to a later sequence and planned instant.".to_string(),
    ));
  }
  let expected_sequence = i64::try_from(expected_sequence).map_err(|_| {
    DurableStoreError::Contract("Interval sequence exceeds the durable integer range.".to_string())
  })?;
  let next_sequence = i64::try_from(next_sequence).map_err(|_| {
    DurableStoreError::Contract("Interval sequence exceeds the durable integer range.".to_string())
  })?;
  let changed = transaction.execute(
    "UPDATE woml_interval_cursors
     SET next_sequence = ?1, next_scheduled_at = ?2, updated_at = ?3
     WHERE workflow_id = ?4 AND trigger_id = ?5
       AND next_sequence = ?6 AND next_scheduled_at = ?7",
    params![
      next_sequence,
      next_scheduled_at.to_rfc3339(),
      updated_at.to_rfc3339(),
      workflow_id,
      trigger_id,
      expected_sequence,
      expected_scheduled_at.to_rfc3339(),
    ],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::IntervalCursorConflict);
  }
  Ok(())
}

fn parse_durable_timestamp(value: &str, label: &str) -> Result<DateTime<Utc>, DurableStoreError> {
  DateTime::parse_from_rfc3339(value)
    .map(|value| value.with_timezone(&Utc))
    .map_err(|_| DurableStoreError::Contract(format!("Stored {label} is not RFC 3339.")))
}

type StoredTriggerOccurrence = (
  String,
  String,
  String,
  String,
  String,
  String,
  String,
  String,
  String,
);

fn trigger_occurrence_from_stored(
  stored: StoredTriggerOccurrence,
) -> Result<TriggerOccurrence, DurableStoreError> {
  let valid_id = |value: &str| !value.is_empty() && value.chars().count() <= 256;
  let valid_handler = matches!(
    stored.3.as_str(),
    "trigger.manual"
      | "trigger.webhook"
      | "trigger.slack"
      | "trigger.schedule"
      | "trigger.interval"
      | "trigger.event"
  );
  if !valid_id(&stored.0)
    || !valid_id(&stored.1)
    || !valid_id(&stored.2)
    || !valid_handler
    || !is_definition_hash(&stored.4)
    || !is_definition_hash(&stored.5)
    || !is_definition_hash(&stored.6)
    || !valid_id(&stored.8)
  {
    return Err(DurableStoreError::TriggerHistoryInvalid(
      "trigger occurrence contains an invalid identity or hash".to_string(),
    ));
  }
  let received_at = DateTime::parse_from_rfc3339(&stored.7)
    .map_err(|_| {
      DurableStoreError::TriggerHistoryInvalid(
        "trigger occurrence receivedAt is not RFC 3339".to_string(),
      )
    })?
    .with_timezone(&Utc);
  Ok(TriggerOccurrence {
    occurrence_schema_version: 1,
    occurrence_id: stored.0,
    workflow_id: stored.1,
    trigger_id: stored.2,
    trigger_handler: stored.3,
    definition_hash: stored.4,
    source_identity_hash: stored.5,
    payload_hash: stored.6,
    received_at,
    run_id: stored.8,
  })
}

fn load_trigger_occurrence_by_id(
  connection: &Connection,
  occurrence_id: &str,
) -> Result<Option<TriggerOccurrence>, DurableStoreError> {
  let stored: Option<StoredTriggerOccurrence> = connection
    .query_row(
      "SELECT occurrence_id, workflow_id, trigger_id, trigger_handler,
              definition_hash, source_identity_hash, payload_hash, received_at, run_id
       FROM woml_trigger_occurrences WHERE occurrence_id = ?1",
      [occurrence_id],
      |row| {
        Ok((
          row.get(0)?,
          row.get(1)?,
          row.get(2)?,
          row.get(3)?,
          row.get(4)?,
          row.get(5)?,
          row.get(6)?,
          row.get(7)?,
          row.get(8)?,
        ))
      },
    )
    .optional()?;
  stored.map(trigger_occurrence_from_stored).transpose()
}

fn load_trigger_occurrence_by_identity(
  connection: &Connection,
  workflow_id: &str,
  trigger_id: &str,
  source_identity_hash: &str,
) -> Result<Option<TriggerOccurrence>, DurableStoreError> {
  let occurrence_id: Option<String> = connection
    .query_row(
      "SELECT occurrence_id FROM woml_trigger_occurrences
       WHERE workflow_id = ?1 AND trigger_id = ?2 AND source_identity_hash = ?3",
      params![workflow_id, trigger_id, source_identity_hash],
      |row| row.get(0),
    )
    .optional()?;
  occurrence_id
    .map(|occurrence_id| load_trigger_occurrence_by_id(connection, &occurrence_id))
    .transpose()
    .map(Option::flatten)
}

fn sha256_prefixed(bytes: &[u8]) -> String {
  format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

fn canonical_payload_hash(payload: &Map<String, Value>) -> Result<String, DurableStoreError> {
  let canonical = canonical_json(payload)?;
  Ok(sha256_prefixed(&canonical))
}

fn validate_trigger_occurrence_history(
  connection: &Connection,
  occurrence: &TriggerOccurrence,
) -> Result<Map<String, Value>, DurableStoreError> {
  let binding = run_binding_in_transaction(connection, &occurrence.run_id).map_err(|error| {
    DurableStoreError::TriggerHistoryInvalid(format!(
      "occurrence run binding is missing or invalid: {error}"
    ))
  })?;
  if binding.workflow_id != occurrence.workflow_id
    || binding.definition_hash != occurrence.definition_hash
  {
    return Err(DurableStoreError::TriggerHistoryInvalid(
      "occurrence does not match its immutable run binding".to_string(),
    ));
  }
  let workflow = definition_for_run(connection, &occurrence.run_id).map_err(|error| {
    DurableStoreError::TriggerHistoryInvalid(format!(
      "occurrence definition is missing or invalid: {error}"
    ))
  })?;
  let events = load_events(connection, &occurrence.run_id).map_err(|error| {
    DurableStoreError::TriggerHistoryInvalid(format!(
      "occurrence event history cannot be loaded: {error}"
    ))
  })?;
  validate_event_history_against_definition(&workflow, &occurrence.definition_hash, &events)
    .map_err(DurableStoreError::TriggerHistoryInvalid)?;
  let Some(RunEvent {
    event_schema_version,
    occurred_at,
    payload: RunEventPayload::RunStarted(start),
    ..
  }) = events.first()
  else {
    return Err(DurableStoreError::TriggerHistoryInvalid(
      "occurrence run does not begin with run_started v7 or v8".to_string(),
    ));
  };
  if !matches!(
    *event_schema_version,
    RUN_EVENT_SCHEMA_VERSION_V7 | RUN_EVENT_SCHEMA_VERSION_V8
  ) {
    return Err(DurableStoreError::TriggerHistoryInvalid(
      "occurrence run does not begin with run_started v7 or v8".to_string(),
    ));
  }
  if start.workflow_id != occurrence.workflow_id
    || start.definition_hash != occurrence.definition_hash
    || start.trigger_id.as_deref() != Some(occurrence.trigger_id.as_str())
    || start.trigger_handler.as_deref() != Some(occurrence.trigger_handler.as_str())
    || start.trigger_occurrence_id.as_deref() != Some(occurrence.occurrence_id.as_str())
    || occurred_at != &occurrence.received_at
    || canonical_payload_hash(&start.trigger)? != occurrence.payload_hash
  {
    return Err(DurableStoreError::TriggerHistoryInvalid(
      "occurrence fields contradict its durable run_started event".to_string(),
    ));
  }
  Ok(start.trigger.clone())
}

fn run_binding_in_transaction(
  connection: &Connection,
  run_id: &str,
) -> Result<RunDefinitionBinding, DurableStoreError> {
  connection
    .query_row(
      "SELECT workflow_id, definition_hash FROM woml_runs WHERE run_id = ?1",
      [run_id],
      |row| {
        Ok(RunDefinitionBinding {
          run_id: run_id.to_string(),
          workflow_id: row.get(0)?,
          definition_hash: row.get(1)?,
        })
      },
    )
    .optional()?
    .ok_or_else(|| DurableStoreError::RunNotFound(run_id.to_string()))
}

fn definition_for_run(
  connection: &Connection,
  run_id: &str,
) -> Result<CompiledWorkflowDefinition, DurableStoreError> {
  let model_json: String = connection
    .query_row(
      "SELECT definitions.model_json
       FROM woml_runs AS runs
       JOIN woml_definitions AS definitions
         ON definitions.definition_hash = runs.definition_hash
       WHERE runs.run_id = ?1",
      [run_id],
      |row| row.get(0),
    )
    .optional()?
    .ok_or_else(|| DurableStoreError::RunNotFound(run_id.to_string()))?;
  let workflow: CompiledWorkflowDefinition = serde_json::from_str(&model_json)?;
  workflow.validate_structure()?;
  Ok(workflow)
}

fn load_events(connection: &Connection, run_id: &str) -> Result<Vec<RunEvent>, DurableStoreError> {
  let mut statement = connection.prepare(
    "SELECT sequence, event_schema_version, event_json
     FROM woml_run_events WHERE run_id = ?1 ORDER BY sequence",
  )?;
  let mut rows = statement.query([run_id])?;
  let mut events = Vec::new();
  while let Some(row) = rows.next()? {
    let stored_sequence: i64 = row.get(0)?;
    let stored_schema_version: i64 = row.get(1)?;
    let event_json: String = row.get(2)?;
    let event: RunEvent = serde_json::from_str(&event_json)?;
    if i64::try_from(event.sequence).ok() != Some(stored_sequence)
      || i64::from(event.event_schema_version) != stored_schema_version
      || event.run_id != run_id
    {
      return Err(DurableStoreError::InvalidStoredEvent(
        "indexed event columns do not match event_json".to_string(),
      ));
    }
    event
      .validate()
      .map_err(|error| DurableStoreError::InvalidStoredEvent(error.to_string()))?;
    events.push(event);
  }
  Ok(events)
}

fn append_to_history(
  transaction: &Transaction<'_>,
  events: &mut Vec<RunEvent>,
  run_id: &str,
  event_id: String,
  occurred_at: DateTime<Utc>,
  event_schema_version: u32,
  payload: RunEventPayload,
) -> Result<RunEvent, DurableStoreError> {
  let sequence = events.len() as u64 + 1;
  let event = RunEvent {
    event_schema_version,
    event_id,
    run_id: run_id.to_string(),
    sequence,
    occurred_at,
    payload,
  };
  let mut candidate = events.clone();
  candidate.push(event.clone());
  fold_events(&candidate)?;
  let event_json = serde_json::to_string(&event)?;
  let stored_sequence = i64::try_from(sequence).map_err(|_| {
    DurableStoreError::Contract("event sequence exceeds SQLite integer range".to_string())
  })?;
  transaction.execute(
    "INSERT INTO woml_run_events(
       run_id, sequence, event_id, event_schema_version, event_json
     ) VALUES (?1, ?2, ?3, ?4, ?5)",
    params![
      run_id,
      stored_sequence,
      event.event_id,
      i64::from(event.event_schema_version),
      event_json,
    ],
  )?;
  events.push(event.clone());
  Ok(event)
}

fn interrupted_failure() -> AttemptFailure {
  AttemptFailure {
    kind: AttemptFailureKind::Interrupted,
    code: AttemptFailureKind::Interrupted.code().to_string(),
    message: "Recovery found a started attempt without a terminal event.".to_string(),
    details: None,
    ..AttemptFailure::legacy_defaults()
  }
}

fn parallel_failure_message(policy: ParallelFailurePolicy, failed_count: usize) -> String {
  match policy {
    ParallelFailurePolicy::FailFast => {
      "A parallel child failed; active siblings were cancelled.".to_string()
    }
    ParallelFailurePolicy::WaitAll if failed_count == 1 => "One parallel child failed.".to_string(),
    ParallelFailurePolicy::WaitAll => format!("{failed_count} parallel children failed."),
  }
}

fn attempt_run_failed_data(
  event_schema_version: u32,
  node_id: String,
  attempt: u32,
  invocation_id: String,
  failure: AttemptFailure,
) -> RunFailedData {
  match event_schema_version {
    RUN_EVENT_SCHEMA_VERSION_V1 => RunFailedData::V1(RunFailedDataV1 {
      node_id: Some(node_id),
      attempt: Some(attempt),
      invocation_id: Some(invocation_id),
      failure,
    }),
    RUN_EVENT_SCHEMA_VERSION_V2
    | RUN_EVENT_SCHEMA_VERSION_V3
    | RUN_EVENT_SCHEMA_VERSION_V4
    | RUN_EVENT_SCHEMA_VERSION_V5
    | RUN_EVENT_SCHEMA_VERSION_V6
    | RUN_EVENT_SCHEMA_VERSION_V7
    | RUN_EVENT_SCHEMA_VERSION_V8 => RunFailedData::V2(RunFailedDataV2::Attempt {
      node_id,
      attempt,
      invocation_id,
      failure,
    }),
    _ => unreachable!("event versions are validated before recovery"),
  }
}

fn generated_event_id() -> String {
  format!("evt_{}", Uuid::new_v4().simple())
}

#[derive(Debug, Error)]
pub enum DurableEngineError {
  #[error(transparent)]
  Store(#[from] DurableStoreError),
  #[error(transparent)]
  InvalidModel(#[from] ModelValidationError),
  #[error("{0}")]
  Contract(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepFailureDisposition {
  RetryScheduled {
    next_attempt: u32,
    scheduled_at: DateTime<Utc>,
  },
  StepFailed,
  RunFailed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepFailureCommit {
  pub disposition: StepFailureDisposition,
  pub projection: RunProjection,
}

#[derive(Debug)]
pub struct DurableDagEngine {
  workflow: CompiledWorkflowDefinition,
  definition_hash: String,
  store: DurableEventStore,
}

impl DurableDagEngine {
  pub fn new(
    workflow: CompiledWorkflowDefinition,
    definition_hash: impl Into<String>,
    mut store: DurableEventStore,
  ) -> Result<Self, DurableEngineError> {
    workflow.validate_for_durable_execution()?;
    let definition_hash = definition_hash.into();
    store.register_definition(&workflow, &definition_hash)?;
    Ok(Self {
      workflow,
      definition_hash,
      store,
    })
  }

  pub fn new_for_event_history(
    workflow: CompiledWorkflowDefinition,
    definition_hash: impl Into<String>,
    mut store: DurableEventStore,
  ) -> Result<Self, DurableEngineError> {
    workflow.validate_structure()?;
    let definition_hash = definition_hash.into();
    store.register_definition(&workflow, &definition_hash)?;
    Ok(Self {
      workflow,
      definition_hash,
      store,
    })
  }

  pub fn resume(store: DurableEventStore, run_id: &str) -> Result<Self, DurableEngineError> {
    let binding = store.run_binding(run_id)?;
    let workflow = store.definition(&binding.definition_hash)?;
    if workflow.workflow_id != binding.workflow_id {
      return Err(DurableEngineError::Contract(
        "Stored run binding does not match its compiled definition.".to_string(),
      ));
    }
    Ok(Self {
      workflow,
      definition_hash: binding.definition_hash,
      store,
    })
  }

  pub fn workflow(&self) -> &CompiledWorkflowDefinition {
    &self.workflow
  }

  pub fn definition_hash(&self) -> &str {
    &self.definition_hash
  }

  pub fn start_run(
    &mut self,
    event_id: impl Into<String>,
    run_id: impl Into<String>,
    occurred_at: DateTime<Utc>,
    trigger: Map<String, Value>,
  ) -> Result<RunProjection, DurableEngineError> {
    let (_, projection) = self.store.start_run(
      event_id,
      run_id,
      occurred_at,
      self.workflow.workflow_id.clone(),
      self.definition_hash.clone(),
      trigger,
    )?;
    Ok(projection)
  }

  pub fn append_payload(
    &mut self,
    event_id: impl Into<String>,
    run_id: &str,
    occurred_at: DateTime<Utc>,
    payload: RunEventPayload,
  ) -> Result<RunProjection, DurableEngineError> {
    validate_payload_against_definition(&self.workflow, &self.definition_hash, &payload)
      .map_err(DurableEngineError::Contract)?;
    if let RunEventPayload::StepAttemptStarted(data) = &payload {
      let ready = self.ready_node_ids(run_id)?;
      if !ready.iter().any(|node_id| node_id == &data.node_id) {
        return Err(DurableEngineError::Contract(format!(
          "Node {:?} is not ready for execution.",
          data.node_id
        )));
      }
    }
    if self.workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V6
      && matches!(
        payload,
        RunEventPayload::StepAttemptFailed(_) | RunEventPayload::StepRetryScheduled(_)
      )
    {
      return Err(DurableEngineError::Contract(
        "Model v6 step failures and retry schedules must use the atomic failure API.".to_string(),
      ));
    }
    let (_, projection) = self
      .store
      .append_payload(run_id, event_id, occurred_at, payload)?;
    Ok(projection)
  }

  pub fn record_step_attempt_failure(
    &mut self,
    run_id: &str,
    failed_at: DateTime<Utc>,
    failure: StepAttemptFailedData,
  ) -> Result<StepFailureCommit, DurableEngineError> {
    if self.workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V6 {
      return Err(DurableEngineError::Contract(
        "The atomic retry failure API requires compiled model v6.".to_string(),
      ));
    }
    let node = self.workflow.node(&failure.node_id).ok_or_else(|| {
      DurableEngineError::Contract(format!(
        "Attempt failure references unknown node {:?}.",
        failure.node_id
      ))
    })?;
    let parallel_child = self
      .workflow
      .parallel_group_for_child(&failure.node_id)
      .is_some();
    let projection = self.projection(run_id)?;
    if !matches!(
      projection.latest_attempt(&failure.node_id),
      Some(attempt)
        if attempt.identity.attempt == failure.attempt
          && attempt.identity.invocation_id == failure.invocation_id
          && attempt.status == AttemptStatus::Started
    ) {
      return Err(DurableEngineError::Contract(
        "Attempt failure does not close the active compiled step attempt.".to_string(),
      ));
    }

    let retry = node.retry_policy.as_ref().filter(|policy| {
      failure.failure.kind == AttemptFailureKind::ScriptThrew
        && failure.attempt < policy.max_attempts
    });
    let (payloads, disposition) = if let Some(policy) = retry {
      let next_attempt = failure.attempt + 1;
      let delay_ms = policy.delay_before_attempt(next_attempt).ok_or_else(|| {
        DurableEngineError::Contract("Compiled retry delay is invalid.".to_string())
      })?;
      let delay = chrono::Duration::milliseconds(i64::try_from(delay_ms).map_err(|_| {
        DurableEngineError::Contract("Retry delay exceeds the clock range.".to_string())
      })?);
      let scheduled_at = failed_at + delay;
      (
        vec![
          (
            generated_event_id(),
            failed_at,
            RunEventPayload::StepAttemptFailed(failure.clone()),
          ),
          (
            generated_event_id(),
            failed_at,
            RunEventPayload::StepRetryScheduled(StepRetryScheduledData {
              node_id: failure.node_id.clone(),
              failed_attempt: failure.attempt,
              next_attempt,
              scheduled_at,
            }),
          ),
        ],
        StepFailureDisposition::RetryScheduled {
          next_attempt,
          scheduled_at,
        },
      )
    } else if parallel_child {
      (
        vec![(
          generated_event_id(),
          failed_at,
          RunEventPayload::StepAttemptFailed(failure.clone()),
        )],
        StepFailureDisposition::StepFailed,
      )
    } else {
      (
        vec![
          (
            generated_event_id(),
            failed_at,
            RunEventPayload::StepAttemptFailed(failure.clone()),
          ),
          (
            generated_event_id(),
            failed_at,
            RunEventPayload::RunFailed(attempt_run_failed_data(
              run_event_schema_version_for_model(self.workflow.schema_version),
              failure.node_id.clone(),
              failure.attempt,
              failure.invocation_id.clone(),
              failure.failure.clone(),
            )),
          ),
        ],
        StepFailureDisposition::RunFailed,
      )
    };
    let projection = self.store.append_payloads_atomically(run_id, payloads)?;
    Ok(StepFailureCommit {
      disposition,
      projection,
    })
  }

  pub fn start_step_attempt(
    &mut self,
    run_id: &str,
    node_id: &str,
    attempt: u32,
    invocation_id: impl Into<String>,
    occurred_at: DateTime<Utc>,
  ) -> Result<RunProjection, DurableEngineError> {
    let node = self
      .workflow
      .node(node_id)
      .ok_or_else(|| DurableEngineError::Contract(format!("Unknown step node {node_id:?}.")))?;
    let idempotency_key = step_effect_idempotency_key(run_id, &self.definition_hash, node_id);
    self.append_payload(
      generated_event_id(),
      run_id,
      occurred_at,
      RunEventPayload::StepAttemptStarted(crate::event::StepAttemptStartedData {
        node_id: node_id.to_string(),
        attempt,
        invocation_id: invocation_id.into(),
        handler: node.handler.clone(),
        idempotency_key: Some(idempotency_key),
      }),
    )
  }

  pub fn request_approval(
    &mut self,
    run_id: &str,
    occurred_at: DateTime<Utc>,
    request: ApprovalRequestedData,
  ) -> Result<IssuedApprovalToken, DurableEngineError> {
    let ready = self.ready_node_ids(run_id)?;
    if !ready.iter().any(|node_id| node_id == &request.approval_id) {
      return Err(DurableEngineError::Contract(format!(
        "Approval {:?} is not ready to enter waiting state.",
        request.approval_id
      )));
    }
    let (_, token) =
      self
        .store
        .request_approval_atomically(run_id, generated_event_id(), occurred_at, request)?;
    Ok(token)
  }

  pub fn reissue_waiting_approval_token(
    &mut self,
    run_id: &str,
    approval_id: &str,
    request_id: &str,
    issued_at: DateTime<Utc>,
  ) -> Result<IssuedApprovalToken, DurableEngineError> {
    Ok(
      self
        .store
        .reissue_approval_token(run_id, approval_id, request_id, issued_at)?,
    )
  }

  pub fn publish_pure_result(
    &mut self,
    run_id: &str,
    node_id: &str,
    invocation_id: &str,
    output: Value,
  ) -> Result<RunProjection, DurableEngineError> {
    let node = self.workflow.node(node_id).ok_or_else(|| {
      DurableEngineError::Contract(format!("Unknown pure result node {node_id:?}."))
    })?;
    if node.handler != "engine.branch-result" {
      return Err(DurableEngineError::Contract(format!(
        "Node {node_id:?} is not an engine.branch-result operation."
      )));
    }
    let ready = self.ready_node_ids(run_id)?;
    if !ready.iter().any(|ready_id| ready_id == node_id) {
      return Err(DurableEngineError::Contract(format!(
        "Node {node_id:?} is not ready for pure result publication."
      )));
    }
    Ok(self.store.append_payloads_atomically(
      run_id,
      vec![
        (
          generated_event_id(),
          Utc::now(),
          RunEventPayload::StepAttemptStarted(crate::event::StepAttemptStartedData {
            node_id: node_id.to_string(),
            attempt: 1,
            invocation_id: invocation_id.to_string(),
            handler: node.handler.clone(),
            idempotency_key: (self.workflow.schema_version
              >= crate::COMPILED_MODEL_SCHEMA_VERSION_V6)
              .then(|| {
                step_effect_idempotency_key(run_id, &self.definition_hash, node_id)
              }),
          }),
        ),
        (
          generated_event_id(),
          Utc::now(),
          RunEventPayload::StepAttemptSucceeded(crate::event::StepAttemptSucceededData {
            node_id: node_id.to_string(),
            attempt: 1,
            invocation_id: invocation_id.to_string(),
            output,
          }),
        ),
      ],
    )?)
  }

  pub fn projection(&self, run_id: &str) -> Result<RunProjection, DurableEngineError> {
    Ok(self.store.projection(run_id)?)
  }

  pub fn events(&self, run_id: &str) -> Result<Vec<RunEvent>, DurableEngineError> {
    Ok(self.store.events(run_id)?)
  }

  pub fn ready_node_ids(&self, run_id: &str) -> Result<Vec<String>, DurableEngineError> {
    let projection = self.projection(run_id)?;
    ready_node_ids_for_projection(&self.workflow, &self.definition_hash, &projection)
      .map_err(DurableEngineError::Contract)
  }

  pub fn ready_node_ids_at(
    &self,
    run_id: &str,
    now: DateTime<Utc>,
  ) -> Result<Vec<String>, DurableEngineError> {
    let projection = self.projection(run_id)?;
    ready_node_ids_for_projection_at(&self.workflow, &self.definition_hash, &projection, now)
      .map_err(DurableEngineError::Contract)
  }

  pub(crate) fn append_payloads_atomically(
    &mut self,
    run_id: &str,
    payloads: Vec<RunEventPayload>,
  ) -> Result<RunProjection, DurableEngineError> {
    Ok(
      self.store.append_payloads_atomically(
        run_id,
        payloads
          .into_iter()
          .map(|payload| (generated_event_id(), Utc::now(), payload))
          .collect(),
      )?,
    )
  }

  pub fn recover_interrupted_runs(&mut self) -> Result<RecoveryReport, DurableEngineError> {
    Ok(self.store.recover_interrupted_runs()?)
  }

  pub fn into_store(self) -> DurableEventStore {
    self.store
  }
}
