//! Worker-local RET5/RET6 attempt authority.
//!
//! This store is intentionally separate from the coordinator event store. It
//! records only factual worker observations and a hash-chained journal. It can
//! prove that an attempt was accepted, started, or completed; it cannot decide
//! workflow state.

use std::{
  collections::{BTreeMap, BTreeSet},
  path::Path,
  sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
  },
};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use serde_json_canonicalizer::to_vec as canonical_json;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
  derive_remote_dispatch_id,
  event::is_definition_hash,
  host::{ScriptHostClient, ScriptHostProcessOptions},
  model::ValueExpression,
  protocol::{ExecuteMessage, HostOutcome, RuntimeModuleBinding, ScriptAttempt},
  remote_dispatch::RemoteDispatchRequestV1,
  remote_protocol::{
    RemoteArtifactGrantV1, RemoteArtifactOperationV1, RemoteCoordinatorAuthorityV1,
    RemoteDispatchPackageV1, RemoteEffectiveAuthorityV1, REMOTE_DISPATCH_PACKAGE_PROFILE,
    REMOTE_DISPATCH_PACKAGE_VERSION, REMOTE_TERMINAL_RESULT_MAX_BYTES,
  },
  CapabilityFailure, DurableCapabilityAuthority,
};

pub const WORKER_JOURNAL_SCHEMA_VERSION: u32 = 1;
pub const WORKER_JOURNAL_RECOVERY_LIMIT: usize = 10_000;
pub const REMOTE_CANCELLATION_CONFIRMATION_TIMEOUT_MS: u64 = 2_000;

#[derive(Debug, Clone, Default)]
pub struct RemoteWorkerCancellationTokenV1(Arc<AtomicBool>);

impl RemoteWorkerCancellationTokenV1 {
  pub fn cancel(&self) {
    self.0.store(true, Ordering::Release);
  }

  pub fn is_cancelled(&self) -> bool {
    self.0.load(Ordering::Acquire)
  }
}

const CREATE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS woml_worker_metadata (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS woml_worker_dispatches (
  dispatch_id TEXT PRIMARY KEY,
  package_digest TEXT NOT NULL,
  worker_id TEXT NOT NULL,
  run_id TEXT NOT NULL,
  node_id TEXT NOT NULL,
  iteration_identity TEXT NOT NULL,
  attempt INTEGER NOT NULL,
  invocation_id TEXT NOT NULL UNIQUE,
  target TEXT NOT NULL,
  authority_digest TEXT NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('accepted','started','succeeded','failed','cancelled','ambiguous')),
  terminal_json TEXT,
  terminal_digest TEXT,
  accepted_at TEXT NOT NULL,
  started_at TEXT,
  terminal_at TEXT,
  CHECK ((terminal_json IS NULL) = (terminal_digest IS NULL)),
  CHECK ((state IN ('succeeded','failed','cancelled')) = (terminal_json IS NOT NULL))
);
CREATE TABLE IF NOT EXISTS woml_worker_journal (
  local_sequence INTEGER PRIMARY KEY AUTOINCREMENT,
  dispatch_id TEXT NOT NULL,
  worker_id TEXT NOT NULL,
  session_id TEXT NOT NULL,
  package_digest TEXT NOT NULL,
  attempt_identity_digest TEXT NOT NULL,
  authority_digest TEXT NOT NULL,
  fact_digest TEXT NOT NULL,
  record_type TEXT NOT NULL CHECK (record_type IN ('accepted','started','succeeded','failed','cancelled','ambiguous')),
  recorded_at TEXT NOT NULL,
  previous_record_digest TEXT NOT NULL,
  record_digest TEXT NOT NULL UNIQUE,
  FOREIGN KEY (dispatch_id) REFERENCES woml_worker_dispatches(dispatch_id)
);
CREATE INDEX IF NOT EXISTS woml_worker_dispatch_state
  ON woml_worker_dispatches(state, accepted_at, dispatch_id);
"#;

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
  fn as_str(self) -> &'static str {
    match self {
      Self::Accepted => "accepted",
      Self::Started => "started",
      Self::Succeeded => "succeeded",
      Self::Failed => "failed",
      Self::Cancelled => "cancelled",
      Self::Ambiguous => "ambiguous",
    }
  }

  fn parse(value: &str) -> Result<Self, RemoteWorkerError> {
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
  fn new(
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

  fn verify(&self) -> Result<(), RemoteWorkerError> {
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

#[derive(Debug, Clone, PartialEq, Eq)]
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

pub trait RemoteSecretProvider: Send + Sync {
  fn resolve(&self, name: &str) -> Result<String, RemoteWorkerError>;
}

#[derive(Debug, Default)]
pub struct EnvironmentRemoteSecretProvider;

impl RemoteSecretProvider for EnvironmentRemoteSecretProvider {
  fn resolve(&self, name: &str) -> Result<String, RemoteWorkerError> {
    std::env::var(name).map_err(|_| RemoteWorkerError::SecretUnavailable)
  }
}

pub struct WorkerJournalV1 {
  connection: Connection,
}

impl WorkerJournalV1 {
  pub fn open(path: &Path) -> Result<Self, RemoteWorkerError> {
    let connection = Connection::open(path)?;
    Self::initialize(connection)
  }

  pub fn open_in_memory() -> Result<Self, RemoteWorkerError> {
    Self::initialize(Connection::open_in_memory()?)
  }

  fn initialize(connection: Connection) -> Result<Self, RemoteWorkerError> {
    connection.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
    connection.execute_batch(CREATE_SCHEMA)?;
    let version: Option<String> = connection
      .query_row(
        "SELECT value FROM woml_worker_metadata WHERE key = 'schema_version'",
        [],
        |row| row.get(0),
      )
      .optional()?;
    match version.as_deref() {
      None => {
        connection.execute(
          "INSERT INTO woml_worker_metadata(key,value) VALUES('schema_version',?1)",
          [WORKER_JOURNAL_SCHEMA_VERSION.to_string()],
        )?;
      }
      Some("1") => {}
      Some(_) => return Err(RemoteWorkerError::CorruptJournal),
    }
    let journal = Self { connection };
    journal.verify_chain()?;
    Ok(journal)
  }

  pub fn accept(
    &mut self,
    prepared: &PreparedRemoteDispatchV1,
    worker_id: &str,
    session_id: &str,
    accepted_at: DateTime<Utc>,
  ) -> Result<WorkerAttemptRecordV1, RemoteWorkerError> {
    if prepared.worker_id != worker_id {
      return Err(RemoteWorkerError::Conflict);
    }
    if let Some(existing) = self.load(&prepared.package.dispatch_id)? {
      if existing.package_digest == prepared.package.package_digest
        && existing.worker_id == worker_id
        && existing.authority_digest == prepared.effective_authority.authority_digest
      {
        return Ok(existing);
      }
      return Err(RemoteWorkerError::Conflict);
    }
    let tx = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
      "INSERT INTO woml_worker_dispatches(
         dispatch_id,package_digest,worker_id,run_id,node_id,iteration_identity,attempt,
         invocation_id,target,authority_digest,state,accepted_at
       ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'accepted',?11)",
      params![
        prepared.package.dispatch_id,
        prepared.package.package_digest,
        worker_id,
        prepared.package.run_id,
        prepared.package.node_id,
        prepared.package.iteration_key.as_deref().unwrap_or(""),
        prepared.package.attempt,
        prepared.package.invocation_id,
        prepared.package.target,
        prepared.effective_authority.authority_digest,
        accepted_at.to_rfc3339(),
      ],
    )?;
    append_record(
      &tx,
      &prepared.package,
      worker_id,
      session_id,
      "accepted",
      &prepared.effective_authority.authority_digest,
      &prepared.package.package_digest,
      accepted_at,
    )?;
    tx.commit()?;
    self
      .load(&prepared.package.dispatch_id)?
      .ok_or(RemoteWorkerError::CorruptJournal)
  }

  pub fn start(
    &mut self,
    dispatch_id: &str,
    session_id: &str,
    started_at: DateTime<Utc>,
  ) -> Result<WorkerAttemptRecordV1, RemoteWorkerError> {
    let record = self.load(dispatch_id)?.ok_or(RemoteWorkerError::Conflict)?;
    if record.state != WorkerAttemptStateV1::Accepted {
      return if record.state == WorkerAttemptStateV1::Started {
        Ok(record)
      } else {
        Err(RemoteWorkerError::Conflict)
      };
    }
    let tx = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
      "UPDATE woml_worker_dispatches SET state='started',started_at=?2
       WHERE dispatch_id=?1 AND state='accepted'",
      params![dispatch_id, started_at.to_rfc3339()],
    )?;
    append_record_from_record(
      &tx,
      &record,
      session_id,
      "started",
      &sha256(b"started"),
      started_at,
    )?;
    tx.commit()?;
    self
      .load(dispatch_id)?
      .ok_or(RemoteWorkerError::CorruptJournal)
  }

  pub fn terminal(
    &mut self,
    dispatch_id: &str,
    session_id: &str,
    terminal: &RemoteWorkerTerminalV1,
  ) -> Result<WorkerAttemptRecordV1, RemoteWorkerError> {
    let record = self.load(dispatch_id)?.ok_or(RemoteWorkerError::Conflict)?;
    if let Some(existing) = &record.terminal {
      return if existing == terminal {
        Ok(record)
      } else {
        Err(RemoteWorkerError::Conflict)
      };
    }
    if record.state != WorkerAttemptStateV1::Started
      || !terminal.state.is_terminal()
      || terminal.dispatch_id != record.dispatch_id
      || terminal.invocation_id != record.invocation_id
    {
      return Err(RemoteWorkerError::Conflict);
    }
    let encoded = canonical_json(terminal).map_err(|_| RemoteWorkerError::InvalidResult)?;
    if encoded.len() > REMOTE_TERMINAL_RESULT_MAX_BYTES {
      return Err(RemoteWorkerError::InvalidResult);
    }
    let text = String::from_utf8(encoded).map_err(|_| RemoteWorkerError::InvalidResult)?;
    let tx = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
      "UPDATE woml_worker_dispatches
       SET state=?2,terminal_json=?3,terminal_digest=?4,terminal_at=?5
       WHERE dispatch_id=?1 AND state='started'",
      params![
        dispatch_id,
        terminal.state.as_str(),
        text,
        terminal.terminal_digest,
        terminal.completed_at.to_rfc3339(),
      ],
    )?;
    append_record_from_record(
      &tx,
      &record,
      session_id,
      terminal.state.as_str(),
      &terminal.terminal_digest,
      terminal.completed_at,
    )?;
    tx.commit()?;
    self
      .load(dispatch_id)?
      .ok_or(RemoteWorkerError::CorruptJournal)
  }

  pub fn recover(
    &mut self,
    now: DateTime<Utc>,
  ) -> Result<Vec<WorkerAttemptRecordV1>, RemoteWorkerError> {
    let count: usize = self.connection.query_row(
      "SELECT COUNT(*) FROM woml_worker_dispatches WHERE state IN ('accepted','started')",
      [],
      |row| row.get(0),
    )?;
    if count > WORKER_JOURNAL_RECOVERY_LIMIT {
      return Err(RemoteWorkerError::CorruptJournal);
    }
    let started = {
      let mut statement = self.connection.prepare(
        "SELECT dispatch_id FROM woml_worker_dispatches WHERE state='started' ORDER BY dispatch_id",
      )?;
      let values = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
      values
    };
    for dispatch_id in started {
      let record = self
        .load(&dispatch_id)?
        .ok_or(RemoteWorkerError::CorruptJournal)?;
      let tx = self
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
      tx.execute(
        "UPDATE woml_worker_dispatches SET state='ambiguous' WHERE dispatch_id=?1 AND state='started'",
        [&dispatch_id],
      )?;
      append_record_from_record(
        &tx,
        &record,
        "recovery",
        "ambiguous",
        &sha256(b"ambiguous"),
        now,
      )?;
      tx.commit()?;
    }
    self.pending_or_ambiguous()
  }

  pub fn terminal_for_resend(
    &self,
    dispatch_id: &str,
  ) -> Result<Option<RemoteWorkerTerminalV1>, RemoteWorkerError> {
    Ok(self.load(dispatch_id)?.and_then(|record| record.terminal))
  }

  pub fn pending_or_ambiguous(&self) -> Result<Vec<WorkerAttemptRecordV1>, RemoteWorkerError> {
    let mut statement = self.connection.prepare(
      "SELECT dispatch_id FROM woml_worker_dispatches
       WHERE state IN ('accepted','ambiguous') ORDER BY accepted_at,dispatch_id LIMIT ?1",
    )?;
    let ids = statement
      .query_map([WORKER_JOURNAL_RECOVERY_LIMIT], |row| {
        row.get::<_, String>(0)
      })?
      .collect::<Result<Vec<_>, _>>()?;
    ids
      .into_iter()
      .map(|id| self.load(&id)?.ok_or(RemoteWorkerError::CorruptJournal))
      .collect()
  }

  pub fn load(
    &self,
    dispatch_id: &str,
  ) -> Result<Option<WorkerAttemptRecordV1>, RemoteWorkerError> {
    let record = self
      .connection
      .query_row(
        "SELECT package_digest,worker_id,run_id,node_id,iteration_identity,attempt,
              invocation_id,target,authority_digest,state,terminal_json,terminal_digest
       FROM woml_worker_dispatches WHERE dispatch_id=?1",
        [dispatch_id],
        |row| {
          let state: String = row.get(9)?;
          let terminal_json: Option<String> = row.get(10)?;
          let terminal_digest: Option<String> = row.get(11)?;
          let terminal = terminal_json
            .map(|value| {
              serde_json::from_str::<RemoteWorkerTerminalV1>(&value)
                .map_err(|_| rusqlite::Error::InvalidQuery)
            })
            .transpose()?;
          if terminal
            .as_ref()
            .map(|value| value.terminal_digest.as_str())
            != terminal_digest.as_deref()
            || terminal
              .as_ref()
              .is_some_and(|value| value.verify().is_err())
          {
            return Err(rusqlite::Error::InvalidQuery);
          }
          Ok((
            WorkerAttemptRecordV1 {
              dispatch_id: dispatch_id.to_string(),
              package_digest: row.get(0)?,
              worker_id: row.get(1)?,
              run_id: row.get(2)?,
              node_id: row.get(3)?,
              iteration_identity: row.get(4)?,
              attempt: row.get(5)?,
              invocation_id: row.get(6)?,
              target: row.get(7)?,
              authority_digest: row.get(8)?,
              state: WorkerAttemptStateV1::parse(&state)
                .map_err(|_| rusqlite::Error::InvalidQuery)?,
              terminal,
            },
            state,
          ))
        },
      )
      .optional()
      .map(|value| value.map(|(record, _)| record))
      .map_err(RemoteWorkerError::from)?;
    let Some(record) = record else {
      return Ok(None);
    };
    let accepted: Option<(String, String, String, String)> = self
      .connection
      .query_row(
        "SELECT worker_id,package_digest,attempt_identity_digest,authority_digest
         FROM woml_worker_journal
         WHERE dispatch_id=?1 AND record_type='accepted'
         ORDER BY local_sequence LIMIT 1",
        [dispatch_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
      )
      .optional()?;
    let identity = sha256(
      &canonical_json(&json!({
        "runId": record.run_id,
        "nodeId": record.node_id,
        "iterationIdentity": record.iteration_identity,
        "attempt": record.attempt,
        "invocationId": record.invocation_id,
      }))
      .map_err(|_| RemoteWorkerError::CorruptJournal)?,
    );
    if accepted.as_ref().is_none_or(|accepted| {
      accepted.0 != record.worker_id
        || accepted.1 != record.package_digest
        || accepted.2 != identity
        || accepted.3 != record.authority_digest
    }) {
      return Err(RemoteWorkerError::CorruptJournal);
    }
    Ok(Some(record))
  }

  fn verify_chain(&self) -> Result<(), RemoteWorkerError> {
    let mut statement = self.connection.prepare(
      "SELECT local_sequence,dispatch_id,worker_id,session_id,package_digest,
              attempt_identity_digest,authority_digest,fact_digest,record_type,recorded_at,
              previous_record_digest,record_digest
       FROM woml_worker_journal ORDER BY local_sequence",
    )?;
    let mut rows = statement.query([])?;
    let mut previous =
      "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_string();
    while let Some(row) = rows.next()? {
      let body = json!({
        "localSequence": row.get::<_, u64>(0)?,
        "dispatchId": row.get::<_, String>(1)?,
        "workerId": row.get::<_, String>(2)?,
        "sessionId": row.get::<_, String>(3)?,
        "packageDigest": row.get::<_, String>(4)?,
        "attemptIdentityDigest": row.get::<_, String>(5)?,
        "authorityDigest": row.get::<_, String>(6)?,
        "factDigest": row.get::<_, String>(7)?,
        "recordType": row.get::<_, String>(8)?,
        "recordedAt": row.get::<_, String>(9)?,
        "previousRecordDigest": row.get::<_, String>(10)?,
      });
      if body["previousRecordDigest"] != previous {
        return Err(RemoteWorkerError::CorruptJournal);
      }
      let expected = sha256(&canonical_json(&body).map_err(|_| RemoteWorkerError::CorruptJournal)?);
      let actual: String = row.get(11)?;
      if expected != actual {
        return Err(RemoteWorkerError::CorruptJournal);
      }
      previous = actual;
    }
    Ok(())
  }
}

/// Validates package integrity and computes the authority intersection that is
/// persisted by `WorkerJournalV1::accept` before any script-host process starts.
pub fn prepare_remote_dispatch(
  package: RemoteDispatchPackageV1,
  coordinator: &RemoteCoordinatorAuthorityV1,
  worker_id: &str,
  worker_target: &str,
  worker: &RemoteWorkerAuthorityV1,
  now: DateTime<Utc>,
) -> Result<PreparedRemoteDispatchV1, RemoteWorkerError> {
  let encoded = canonical_json(&package).map_err(|_| RemoteWorkerError::InvalidPackage)?;
  if encoded.len() > crate::WORKER_PROTOCOL_MAX_MESSAGE_BYTES
    || package.profile != REMOTE_DISPATCH_PACKAGE_PROFILE
    || package.version != REMOTE_DISPATCH_PACKAGE_VERSION
    || package.node_id != package.node.id
    || package.node.handler != "runtime.script"
    || package.target != worker_target
    || package.deadline <= now
    || !is_definition_hash(&package.definition_package_root_hash)
    || !is_definition_hash(&package.target_registry_digest)
    || !is_definition_hash(&worker.policy_digest)
    || !is_definition_hash(&coordinator.policy_digest)
    || package
      .infrastructure_policy_digest
      .as_ref()
      .is_some_and(|digest| !is_definition_hash(digest) || digest != &coordinator.policy_digest)
    || package.package_digest
      != package
        .computed_digest()
        .map_err(|_| RemoteWorkerError::InvalidPackage)?
  {
    return Err(if package.deadline <= now {
      RemoteWorkerError::Expired
    } else {
      RemoteWorkerError::InvalidPackage
    });
  }
  let request = RemoteDispatchRequestV1 {
    run_id: package.run_id.clone(),
    node_id: package.node_id.clone(),
    iteration_identity: package.iteration_key.clone().unwrap_or_default(),
    attempt: package.attempt,
    invocation_id: package.invocation_id.clone(),
    target: package.target.clone(),
    target_digest: package.target_registry_digest.clone(),
    package_digest: package.package_digest.clone(),
    requested_at: now,
  };
  if derive_remote_dispatch_id(&request).map_err(|_| RemoteWorkerError::InvalidPackage)?
    != package.dispatch_id
  {
    return Err(RemoteWorkerError::InvalidPackage);
  }
  ScriptAttempt::new(
    package.attempt,
    maximum_attempts(&package),
    &package.package_digest,
  )
  .map_err(|_| RemoteWorkerError::InvalidPackage)?;
  let execution = package
    .node
    .execution
    .as_ref()
    .ok_or(RemoteWorkerError::InvalidPackage)?;
  if execution.profile_version != 1
    || execution.target != package.target
    || execution.required_capabilities != package.required_capabilities
    || package.required_capabilities.first().map(String::as_str) != Some("runtime.script")
  {
    return Err(RemoteWorkerError::InvalidPackage);
  }
  if script_source(&package.node).is_none() {
    return Err(RemoteWorkerError::InvalidPackage);
  }
  let script_runtime = package
    .node
    .script_runtime
    .as_ref()
    .ok_or(RemoteWorkerError::InvalidPackage)?;
  if script_runtime.binding_version != 1
    || script_runtime
      .bindings
      .iter()
      .map(String::as_str)
      .ne(["context", "attempt", "services", "secrets"])
  {
    return Err(RemoteWorkerError::InvalidPackage);
  }
  let requested = package
    .required_capabilities
    .iter()
    .map(|id| {
      capability_version(id)
        .map(|version| (id.clone(), version))
        .ok_or(RemoteWorkerError::CapabilityUnavailable)
    })
    .collect::<Result<BTreeMap<_, _>, _>>()?;
  if requested.len() != package.required_capabilities.len()
    || requested.get("runtime.script") != Some(&1)
  {
    return Err(RemoteWorkerError::CapabilityUnavailable);
  }
  let coordinator_capabilities: BTreeMap<_, _> = coordinator
    .allowed_capabilities
    .iter()
    .map(|capability| (capability.id.clone(), capability.version))
    .collect();
  if coordinator_capabilities.len() != coordinator.allowed_capabilities.len() {
    return Err(RemoteWorkerError::CapabilityUnavailable);
  }
  for (id, version) in &requested {
    if coordinator_capabilities.get(id) != Some(version)
      || worker.allowed_capabilities.get(id) != Some(version)
    {
      return Err(RemoteWorkerError::CapabilityUnavailable);
    }
  }
  let required_secrets: BTreeSet<_> = package.required_secrets.iter().cloned().collect();
  let coordinator_secrets: BTreeSet<_> = coordinator.allowed_secret_names.iter().cloned().collect();
  if required_secrets.len() != package.required_secrets.len()
    || coordinator_secrets.len() != coordinator.allowed_secret_names.len()
    || required_secrets.iter().any(|name| !valid_secret_name(name))
    || !required_secrets.is_subset(&coordinator_secrets)
    || !required_secrets.is_subset(&worker.allowed_secret_names)
    || script_runtime
      .required_secrets
      .iter()
      .cloned()
      .collect::<BTreeSet<_>>()
      != required_secrets
  {
    return Err(RemoteWorkerError::SecretUnavailable);
  }
  validate_artifact_grants(&package.artifact_grants, &package, now)?;
  let mut effective = RemoteEffectiveAuthorityV1 {
    coordinator_policy_digest: coordinator.policy_digest.clone(),
    worker_policy_digest: worker.policy_digest.clone(),
    capabilities: requested
      .into_iter()
      .map(|(id, version)| crate::WorkerCapabilityV1 { id, version })
      .collect(),
    secret_names: package.required_secrets.clone(),
    artifact_grant_ids: package
      .artifact_grants
      .iter()
      .map(artifact_grant_digest)
      .collect::<Result<Vec<_>, _>>()?,
    authority_digest: String::new(),
  };
  effective
    .capabilities
    .sort_by(|left, right| left.id.cmp(&right.id));
  effective.secret_names.sort();
  effective.artifact_grant_ids.sort();
  effective.authority_digest = effective
    .computed_digest()
    .map_err(|_| RemoteWorkerError::InvalidPackage)?;
  Ok(PreparedRemoteDispatchV1 {
    package,
    effective_authority: effective,
    worker_id: worker_id.to_string(),
  })
}

/// Verifies a scoped artifact transfer at the worker boundary. Coordinators
/// perform the same checks before issuing a package; workers repeat them so a
/// compromised or stale transport cannot widen a grant.
#[allow(clippy::too_many_arguments)]
pub fn verify_remote_artifact_transfer(
  prepared: &PreparedRemoteDispatchV1,
  grant: &RemoteArtifactGrantV1,
  worker_id: &str,
  operation: RemoteArtifactOperationV1,
  bytes: &[u8],
  maximum_bytes: u64,
  now: DateTime<Utc>,
) -> Result<String, RemoteWorkerError> {
  let digest = sha256(bytes);
  if prepared.worker_id != worker_id
    || !prepared.package.artifact_grants.contains(grant)
    || grant.operation != operation
    || grant.expires_at <= now
    || bytes.is_empty()
    || maximum_bytes == 0
    || bytes.len() as u64 > maximum_bytes
    || digest != grant.digest
  {
    return Err(RemoteWorkerError::ArtifactIntegrity);
  }
  Ok(digest)
}

pub struct RemoteWorkerAttemptRuntimeV1 {
  journal: WorkerJournalV1,
  worker_id: String,
  session_id: String,
  capacity: usize,
  active: Arc<AtomicUsize>,
  draining: Arc<AtomicBool>,
  host_options: ScriptHostProcessOptions,
  capability_authority: Option<Arc<DurableCapabilityAuthority>>,
  secret_provider: Arc<dyn RemoteSecretProvider>,
}

impl RemoteWorkerAttemptRuntimeV1 {
  #[allow(clippy::too_many_arguments)]
  pub fn new(
    journal: WorkerJournalV1,
    worker_id: impl Into<String>,
    session_id: impl Into<String>,
    capacity: usize,
    host_options: ScriptHostProcessOptions,
    capability_authority: Option<Arc<DurableCapabilityAuthority>>,
    secret_provider: Arc<dyn RemoteSecretProvider>,
  ) -> Result<Self, RemoteWorkerError> {
    if capacity == 0 || capacity > crate::WORKER_PROTOCOL_MAX_CAPACITY as usize {
      return Err(RemoteWorkerError::Capacity);
    }
    Ok(Self {
      journal,
      worker_id: worker_id.into(),
      session_id: session_id.into(),
      capacity,
      active: Arc::new(AtomicUsize::new(0)),
      draining: Arc::new(AtomicBool::new(false)),
      host_options,
      capability_authority,
      secret_provider,
    })
  }

  pub fn begin_drain(&self) {
    self.draining.store(true, Ordering::Release);
  }

  pub fn is_drained(&self) -> bool {
    self.active.load(Ordering::Acquire) == 0
  }

  pub fn journal(&self) -> &WorkerJournalV1 {
    &self.journal
  }

  pub async fn execute(
    &mut self,
    prepared: &PreparedRemoteDispatchV1,
    now: DateTime<Utc>,
  ) -> Result<RemoteWorkerTerminalV1, RemoteWorkerError> {
    self
      .execute_with_cancellation(prepared, now, RemoteWorkerCancellationTokenV1::default())
      .await
  }

  pub async fn execute_with_cancellation(
    &mut self,
    prepared: &PreparedRemoteDispatchV1,
    now: DateTime<Utc>,
    cancellation: RemoteWorkerCancellationTokenV1,
  ) -> Result<RemoteWorkerTerminalV1, RemoteWorkerError> {
    if prepared.worker_id != self.worker_id {
      return Err(RemoteWorkerError::Conflict);
    }
    if self.capability_authority.is_none()
      && prepared
        .effective_authority
        .capabilities
        .iter()
        .any(|capability| capability.id != "runtime.script")
    {
      return Err(RemoteWorkerError::CapabilityUnavailable);
    }
    if let Some(terminal) = self
      .journal
      .terminal_for_resend(&prepared.package.dispatch_id)?
    {
      return Ok(terminal);
    }
    if self.draining.load(Ordering::Acquire) {
      return Err(RemoteWorkerError::Draining);
    }
    let active = self.active.fetch_add(1, Ordering::AcqRel);
    if active >= self.capacity {
      self.active.fetch_sub(1, Ordering::AcqRel);
      return Err(RemoteWorkerError::Capacity);
    }
    let _slot = ActiveSlot(Arc::clone(&self.active));
    let secrets = prepared
      .effective_authority
      .secret_names
      .iter()
      .map(|name| {
        self
          .secret_provider
          .resolve(name)
          .map(|value| (name.clone(), value))
      })
      .collect::<Result<BTreeMap<_, _>, _>>()?;
    let sensitive_values = secrets.values().cloned().collect::<Vec<_>>();
    let source = script_source(&prepared.package.node)
      .ok_or(RemoteWorkerError::InvalidPackage)?
      .to_string();
    let timeout_ms = prepared.package.node.timeout_ms.unwrap_or(30_000).min(
      (prepared.package.deadline - now)
        .num_milliseconds()
        .try_into()
        .unwrap_or(0),
    );
    if timeout_ms == 0 {
      return Err(RemoteWorkerError::Expired);
    }
    let accepted = self
      .journal
      .accept(prepared, &self.worker_id, &self.session_id, now)?;
    if let Some(terminal) = accepted.terminal {
      return Ok(terminal);
    }
    if accepted.state == WorkerAttemptStateV1::Ambiguous
      || accepted.state == WorkerAttemptStateV1::Started
    {
      return Err(RemoteWorkerError::Ambiguous);
    }
    if cancellation.is_cancelled() {
      self
        .journal
        .start(&prepared.package.dispatch_id, &self.session_id, now)?;
      let terminal = RemoteWorkerTerminalV1::new(
        &prepared.package.dispatch_id,
        &prepared.package.invocation_id,
        WorkerAttemptStateV1::Cancelled,
        json!({"code": "WOML_RUN_CANCELLED", "message": "Remote execution was cancelled before script execution."}),
        Utc::now(),
      )?;
      self
        .journal
        .terminal(&prepared.package.dispatch_id, &self.session_id, &terminal)?;
      return Ok(terminal);
    }
    let mut options = self.host_options.clone();
    options.module_artifacts.clear();
    let host = ScriptHostClient::spawn_with_authority(options, self.capability_authority.clone())
      .await
      .map_err(|_| RemoteWorkerError::HostUnavailable)?;
    let module_bindings: Vec<RuntimeModuleBinding> = Vec::new();
    let attempt = ScriptAttempt::new(
      prepared.package.attempt,
      maximum_attempts(&prepared.package),
      &prepared.package.package_digest,
    )
    .map_err(|_| RemoteWorkerError::InvalidPackage)?;
    // Everything above this point is preflight and may be retried safely. The
    // durable start fact is committed immediately before the first script-host
    // execute frame, which is the boundary after which effects are ambiguous.
    if let Err(error) = self
      .journal
      .start(&prepared.package.dispatch_id, &self.session_id, now)
    {
      host.shutdown().await;
      return Err(error);
    }
    let message = ExecuteMessage::runtime_script_with_modules(
      &prepared.package.invocation_id,
      &prepared.package.run_id,
      &prepared.package.node_id,
      attempt,
      timeout_ms,
      &source,
      &prepared.package.context,
      &secrets,
      &module_bindings,
    );
    let completion = {
      let execution = host.execute(&message);
      tokio::pin!(execution);
      loop {
        tokio::select! {
          completion = &mut execution => break completion,
          _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {
            if cancellation.is_cancelled() {
              if host.cancel_run(&prepared.package.invocation_id).await.is_err() {
                break Err(crate::host::ScriptHostClientError::HostCrashed(
                  "remote cancellation could not be delivered".to_string(),
                ));
              }
              break match tokio::time::timeout(
                std::time::Duration::from_millis(REMOTE_CANCELLATION_CONFIRMATION_TIMEOUT_MS),
                &mut execution,
              ).await {
                Ok(completion) => completion,
                Err(_) => Err(crate::host::ScriptHostClientError::HostCrashed(
                  "remote cancellation was not confirmed".to_string(),
                )),
              };
            }
          }
        }
      }
    };
    host.shutdown().await;
    let (state, mut outcome) = match completion {
      Ok(completed) => match completed.outcome {
        HostOutcome::Success { value } => (WorkerAttemptStateV1::Succeeded, value),
        HostOutcome::Failure { error } => (
          if error.kind == crate::protocol::HostReportedFailureKind::InvocationCancelled {
            WorkerAttemptStateV1::Cancelled
          } else {
            WorkerAttemptStateV1::Failed
          },
          json!({"code": error.code, "message": error.message, "kind": error.kind}),
        ),
      },
      Err(_) => return Err(RemoteWorkerError::Ambiguous),
    };
    redact_json_value(&mut outcome, &sensitive_values);
    let terminal = RemoteWorkerTerminalV1::new(
      &prepared.package.dispatch_id,
      &prepared.package.invocation_id,
      state,
      outcome,
      Utc::now(),
    )?;
    self
      .journal
      .terminal(&prepared.package.dispatch_id, &self.session_id, &terminal)?;
    Ok(terminal)
  }
}

struct ActiveSlot(Arc<AtomicUsize>);
impl Drop for ActiveSlot {
  fn drop(&mut self) {
    self.0.fetch_sub(1, Ordering::AcqRel);
  }
}

fn script_source(node: &crate::CompiledWorkflowNode) -> Option<&str> {
  let ValueExpression::Object { fields } = &node.inputs else {
    return None;
  };
  let ValueExpression::Literal { value } = fields.get("source")? else {
    return None;
  };
  value.as_str()
}

fn redact_json_value(value: &mut Value, sensitive_values: &[String]) {
  match value {
    Value::String(text) => {
      let mut ordered = sensitive_values
        .iter()
        .filter(|secret| !secret.is_empty())
        .collect::<Vec<_>>();
      ordered.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
      for secret in ordered {
        *text = text.replace(secret, "[redacted]");
      }
    }
    Value::Array(values) => {
      for value in values {
        redact_json_value(value, sensitive_values);
      }
    }
    Value::Object(fields) => {
      for value in fields.values_mut() {
        redact_json_value(value, sensitive_values);
      }
    }
    Value::Null | Value::Bool(_) | Value::Number(_) => {}
  }
}

fn validate_artifact_grants(
  grants: &[RemoteArtifactGrantV1],
  package: &RemoteDispatchPackageV1,
  now: DateTime<Utc>,
) -> Result<(), RemoteWorkerError> {
  if grants.len() > 128 {
    return Err(RemoteWorkerError::InvalidPackage);
  }
  let mut ids = BTreeSet::new();
  for grant in grants {
    if !ids.insert((grant.artifact_id.as_str(), grant.operation))
      || grant.expires_at <= now
      || grant.expires_at > package.deadline
      || !valid_bound_id(&grant.artifact_id)
      || !is_definition_hash(&grant.digest)
      || matches!(grant.operation, RemoteArtifactOperationV1::Write) && grant.artifact_id == "*"
    {
      return Err(RemoteWorkerError::InvalidPackage);
    }
  }
  Ok(())
}

fn valid_secret_name(value: &str) -> bool {
  !value.is_empty()
    && value.len() <= 128
    && value
      .bytes()
      .next()
      .is_some_and(|byte| byte.is_ascii_uppercase())
    && value
      .bytes()
      .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

fn valid_bound_id(value: &str) -> bool {
  !value.is_empty()
    && value.len() <= 256
    && value
      .bytes()
      .next()
      .is_some_and(|byte| byte.is_ascii_alphanumeric())
    && value
      .bytes()
      .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'-'))
}

fn capability_version(id: &str) -> Option<u32> {
  match id {
    "runtime.script" | "services.artifacts" | "operational.output" => Some(1),
    "services.process" => Some(2),
    _ => None,
  }
}

fn maximum_attempts(package: &RemoteDispatchPackageV1) -> u32 {
  package
    .node
    .retry_policy
    .as_ref()
    .map_or(1, |policy| policy.max_attempts)
}

fn artifact_grant_digest(grant: &RemoteArtifactGrantV1) -> Result<String, RemoteWorkerError> {
  canonical_json(grant)
    .map(|bytes| sha256(&bytes))
    .map_err(|_| RemoteWorkerError::InvalidPackage)
}

fn append_record(
  tx: &rusqlite::Transaction<'_>,
  package: &RemoteDispatchPackageV1,
  worker_id: &str,
  session_id: &str,
  record_type: &str,
  authority_digest: &str,
  fact_digest: &str,
  recorded_at: DateTime<Utc>,
) -> Result<(), RemoteWorkerError> {
  let record = WorkerAttemptRecordV1 {
    dispatch_id: package.dispatch_id.clone(),
    package_digest: package.package_digest.clone(),
    worker_id: worker_id.to_string(),
    run_id: package.run_id.clone(),
    node_id: package.node_id.clone(),
    iteration_identity: package.iteration_key.clone().unwrap_or_default(),
    attempt: package.attempt,
    invocation_id: package.invocation_id.clone(),
    target: package.target.clone(),
    authority_digest: authority_digest.to_string(),
    state: WorkerAttemptStateV1::Accepted,
    terminal: None,
  };
  append_record_from_record(
    tx,
    &record,
    session_id,
    record_type,
    fact_digest,
    recorded_at,
  )
}

fn append_record_from_record(
  tx: &rusqlite::Transaction<'_>,
  record: &WorkerAttemptRecordV1,
  session_id: &str,
  record_type: &str,
  fact_digest: &str,
  recorded_at: DateTime<Utc>,
) -> Result<(), RemoteWorkerError> {
  let previous: String = tx
    .query_row(
      "SELECT record_digest FROM woml_worker_journal ORDER BY local_sequence DESC LIMIT 1",
      [],
      |row| row.get(0),
    )
    .optional()?
    .unwrap_or_else(|| {
      "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_string()
    });
  let next: u64 = tx.query_row(
    "SELECT COALESCE(MAX(local_sequence),0)+1 FROM woml_worker_journal",
    [],
    |row| row.get(0),
  )?;
  let identity = sha256(
    &canonical_json(&json!({
      "runId": record.run_id,
      "nodeId": record.node_id,
      "iterationIdentity": record.iteration_identity,
      "attempt": record.attempt,
      "invocationId": record.invocation_id,
    }))
    .map_err(|_| RemoteWorkerError::CorruptJournal)?,
  );
  let body = json!({
    "localSequence": next,
    "dispatchId": record.dispatch_id,
    "workerId": record.worker_id,
    "sessionId": session_id,
    "packageDigest": record.package_digest,
    "attemptIdentityDigest": identity,
    "authorityDigest": record.authority_digest,
    "factDigest": fact_digest,
    "recordType": record_type,
    "recordedAt": recorded_at.to_rfc3339(),
    "previousRecordDigest": previous,
  });
  let digest = sha256(&canonical_json(&body).map_err(|_| RemoteWorkerError::CorruptJournal)?);
  tx.execute(
    "INSERT INTO woml_worker_journal(
       local_sequence,dispatch_id,worker_id,session_id,package_digest,attempt_identity_digest,
       authority_digest,fact_digest,record_type,recorded_at,previous_record_digest,record_digest
     ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
    params![
      next,
      record.dispatch_id,
      record.worker_id,
      session_id,
      record.package_digest,
      identity,
      record.authority_digest,
      fact_digest,
      record_type,
      recorded_at.to_rfc3339(),
      previous,
      digest,
    ],
  )?;
  Ok(())
}

fn sha256(bytes: &[u8]) -> String {
  format!("sha256:{:x}", Sha256::digest(bytes))
}

impl From<CapabilityFailure> for RemoteWorkerError {
  fn from(_: CapabilityFailure) -> Self {
    Self::CapabilityUnavailable
  }
}
