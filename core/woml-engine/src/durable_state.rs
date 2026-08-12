//! Durable User State v1 transactional authority.
//!
//! The same transactional authority is exposed directly for conformance tests
//! and through the managed capability runtime used by real WOML scripts.

use std::{
  fs,
  path::{Path, PathBuf},
  sync::{
    atomic::{AtomicI64, Ordering},
    Arc, Mutex,
  },
  time::{Duration, Instant},
};

use chrono::{DateTime, SecondsFormat, Utc};
use futures_util::future::BoxFuture;
use rusqlite::{
  params, Connection, ErrorCode, OpenFlags, OptionalExtension, Transaction, TransactionBehavior,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use serde_json_canonicalizer::to_vec as canonical_json;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
  capability::CapabilityIdentityMode, derive_operation_key, CapabilityCallRequest,
  CapabilityCancellationToken, CapabilityDescriptor, CapabilityEffect, CapabilityFailure,
  CapabilityFailureKind, CapabilityHandler, DurableEventStore, DurableStoreError,
};

pub const STATE_CONTRACT: &str = "woml.state";
pub const STATE_CONTRACT_VERSION: u32 = 1;
pub const MAX_STATE_KEY_BYTES: usize = 256;
pub const MAX_STATE_VALUE_BYTES: usize = 262_144;
pub const DEFAULT_STATE_MAX_KEYS: u64 = 10_000;
pub const DEFAULT_STATE_MAX_BYTES: u64 = 67_108_864;
pub const MAX_STATE_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
const STATE_OPERATIONS: [&str; 6] = ["get", "has", "set", "delete", "increment", "set_if_absent"];

pub trait StateClock: Send + Sync + 'static {
  fn now_millis(&self) -> i64;
}

#[derive(Debug)]
pub struct SystemStateClock;

impl StateClock for SystemStateClock {
  fn now_millis(&self) -> i64 {
    Utc::now().timestamp_millis()
  }
}

#[derive(Debug)]
pub struct FixedStateClock {
  now_millis: AtomicI64,
}

impl FixedStateClock {
  pub const fn new(now_millis: i64) -> Self {
    Self {
      now_millis: AtomicI64::new(now_millis),
    }
  }

  pub fn set(&self, now_millis: i64) {
    self.now_millis.store(now_millis, Ordering::SeqCst);
  }

  pub fn advance(&self, milliseconds: i64) {
    self.now_millis.fetch_add(milliseconds, Ordering::SeqCst);
  }
}

impl StateClock for FixedStateClock {
  fn now_millis(&self) -> i64 {
    self.now_millis.load(Ordering::SeqCst)
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DurableStateLimits {
  pub max_keys: u64,
  pub max_bytes: u64,
  pub max_value_bytes: usize,
}

impl Default for DurableStateLimits {
  fn default() -> Self {
    Self {
      max_keys: DEFAULT_STATE_MAX_KEYS,
      max_bytes: DEFAULT_STATE_MAX_BYTES,
      max_value_bytes: MAX_STATE_VALUE_BYTES,
    }
  }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DurableStateExecution {
  pub result: Value,
  pub duplicate: bool,
  pub operation: String,
  pub key_digest: String,
  pub input_digest: String,
  pub result_digest: String,
  pub version: Option<u64>,
  pub value_bytes: Option<u64>,
  pub duration_ms: f64,
}

impl DurableStateExecution {
  /// Redacted fields safe to merge into generic managed-operation metadata.
  pub fn safe_metadata(&self) -> Map<String, Value> {
    let mut metadata = Map::from_iter([
      (
        "profile".to_string(),
        json!("woml.state-operation-metadata/v1"),
      ),
      ("operation".to_string(), json!(self.operation)),
      ("keyDigest".to_string(), json!(self.key_digest)),
      ("inputDigest".to_string(), json!(self.input_digest)),
      ("resultDigest".to_string(), json!(self.result_digest)),
      ("outcome".to_string(), json!("succeeded")),
      ("durationMs".to_string(), json!(self.duration_ms)),
    ]);
    if let Some(version) = self.version {
      metadata.insert("version".to_string(), json!(version));
    }
    if let Some(value_bytes) = self.value_bytes {
      metadata.insert("valueBytes".to_string(), json!(value_bytes));
    }
    metadata
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DurableStateError {
  #[error("Durable User State request is invalid.")]
  InvalidRequest,
  #[error("The state key must be non-empty and at most 256 UTF-8 bytes.")]
  KeyInvalid,
  #[error("The state value is not valid canonical JSON.")]
  ValueInvalid,
  #[error("The state value exceeds its configured byte limit.")]
  ValueTooLarge,
  #[error("The state version does not match.")]
  Conflict,
  #[error("The state mutation requires a stable operation name.")]
  OperationNameInvalid,
  #[error("The state mutation identity was reused with different input.")]
  OperationIdentityConflict,
  #[error("The workflow state quota would be exceeded.")]
  QuotaExceeded,
  #[error("The existing state value is not a JSON safe integer.")]
  IntegerRequired,
  #[error("The state increment would exceed the JSON safe integer range.")]
  IntegerOverflow,
  #[error("The durable state store is unavailable.")]
  StoreUnavailable,
  #[error("The durable state store failed its integrity checks.")]
  StoreCorrupt,
}

impl DurableStateError {
  pub const fn code(&self) -> &'static str {
    match self {
      Self::InvalidRequest => "WOML_STATE_VALUE_INVALID",
      Self::KeyInvalid => "WOML_STATE_KEY_INVALID",
      Self::ValueInvalid => "WOML_STATE_VALUE_INVALID",
      Self::ValueTooLarge => "WOML_STATE_VALUE_TOO_LARGE",
      Self::Conflict => "WOML_STATE_CONFLICT",
      Self::OperationNameInvalid => "WOML_STATE_OPERATION_NAME_INVALID",
      Self::OperationIdentityConflict => "WOML_STATE_OPERATION_IDENTITY_CONFLICT",
      Self::QuotaExceeded => "WOML_STATE_QUOTA_EXCEEDED",
      Self::IntegerRequired => "WOML_STATE_INTEGER_REQUIRED",
      Self::IntegerOverflow => "WOML_STATE_INTEGER_OVERFLOW",
      Self::StoreUnavailable => "WOML_STATE_STORE_UNAVAILABLE",
      Self::StoreCorrupt => "WOML_STATE_STORE_CORRUPT",
    }
  }

  pub const fn retryable(&self) -> bool {
    matches!(self, Self::StoreUnavailable)
  }

  pub const fn ambiguous(&self) -> bool {
    false
  }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StateRequest {
  contract: String,
  contract_version: u32,
  kind: String,
  operation: String,
  input: Map<String, Value>,
}

#[derive(Debug, Clone)]
struct StateEntry {
  value_json: String,
  value_bytes: u64,
  version: u64,
  updated_at: String,
}

#[derive(Debug, Clone)]
struct MutationIdentity {
  operation_key: String,
  operation_name: String,
  input_digest: String,
}

#[derive(Debug, Clone)]
struct StoredMutation {
  scope_digest: String,
  operation_name: String,
  operation: String,
  key_digest: String,
  input_digest: String,
  result_json: String,
  result_digest: String,
  committed_version: Option<u64>,
}

pub struct DurableStateStore {
  path: PathBuf,
  state_location_digest: String,
  clock: Arc<dyn StateClock>,
  limits: DurableStateLimits,
}

/// Runtime-owned handle configured from the CLI's durable state path. Keeping
/// this indirection in Rust prevents scripts from choosing a database or a
/// workflow namespace.
#[derive(Default)]
pub struct ManagedDurableStateStore {
  store: Mutex<Option<Arc<DurableStateStore>>>,
}

impl std::fmt::Debug for ManagedDurableStateStore {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    formatter
      .debug_struct("ManagedDurableStateStore")
      .field(
        "configured",
        &self
          .store
          .lock()
          .map(|store| store.is_some())
          .unwrap_or(false),
      )
      .finish()
  }
}

impl ManagedDurableStateStore {
  pub fn configure_for_state(&self, state_path: &Path) -> Result<(), CapabilityFailure> {
    let store = DurableStateStore::open(state_path).map_err(state_failure)?;
    *self.store.lock().map_err(|_| state_store_unavailable())? = Some(Arc::new(store));
    Ok(())
  }

  fn configured(&self) -> Result<Arc<DurableStateStore>, CapabilityFailure> {
    self
      .store
      .lock()
      .map_err(|_| state_store_unavailable())?
      .clone()
      .ok_or_else(state_store_unavailable)
  }
}

#[derive(Debug)]
pub struct ManagedDurableStateHandler {
  store: Arc<ManagedDurableStateStore>,
  operation: &'static str,
}

impl ManagedDurableStateHandler {
  pub fn handlers(store: Arc<ManagedDurableStateStore>) -> Vec<Arc<Self>> {
    STATE_OPERATIONS
      .iter()
      .map(|operation| {
        Arc::new(Self {
          store: Arc::clone(&store),
          operation,
        })
      })
      .collect()
  }
}

impl CapabilityHandler for ManagedDurableStateHandler {
  fn descriptor(&self) -> CapabilityDescriptor {
    CapabilityDescriptor {
      capability: "state".to_string(),
      operation: self.operation.to_string(),
      input_contract_version: STATE_CONTRACT_VERSION,
      result_contract_version: STATE_CONTRACT_VERSION,
      effect: if matches!(self.operation, "get" | "has") {
        CapabilityEffect::Read
      } else {
        CapabilityEffect::IdempotentWrite
      },
      supports_cancellation: true,
      supports_provider_idempotency: false,
    }
  }

  fn validate_request(&self, request: &CapabilityCallRequest) -> Result<(), CapabilityFailure> {
    parse_request(&request.input, self.operation)
      .map(|_| ())
      .map_err(state_failure)
  }

  fn safe_request_metadata(
    &self,
    request: &CapabilityCallRequest,
  ) -> Result<Map<String, Value>, CapabilityFailure> {
    let parsed = parse_request(&request.input, self.operation).map_err(state_failure)?;
    let key = state_key(&parsed.input).map_err(state_failure)?;
    let input_digest =
      canonical_digest(b"woml.state-input\0v1\0", &request.input).map_err(state_failure)?;
    Ok(Map::from_iter([
      (
        "profile".to_string(),
        json!("woml.state-operation-metadata/v1"),
      ),
      ("operation".to_string(), json!(self.operation)),
      (
        "keyDigest".to_string(),
        json!(digest(b"woml.state-key\0v1\0", key.as_bytes())),
      ),
      ("inputDigest".to_string(), json!(input_digest)),
    ]))
  }

  fn safe_result_metadata(&self, result: &Value) -> Map<String, Value> {
    let mut metadata = Map::from_iter([
      (
        "profile".to_string(),
        json!("woml.state-operation-metadata/v1"),
      ),
      ("operation".to_string(), json!(self.operation)),
      ("outcome".to_string(), json!("succeeded")),
    ]);
    if let Ok(result_digest) = canonical_digest(b"woml.state-result\0v1\0", result) {
      metadata.insert("resultDigest".to_string(), json!(result_digest));
    }
    if let Some(version) = result
      .get("data")
      .and_then(|data| data.get("version"))
      .and_then(Value::as_u64)
    {
      metadata.insert("version".to_string(), json!(version));
    }
    metadata
  }

  fn execute(
    &self,
    _input: Value,
    _cancellation: CapabilityCancellationToken,
  ) -> BoxFuture<'static, Result<Value, CapabilityFailure>> {
    Box::pin(async { Err(state_scope_unavailable()) })
  }

  fn execute_request_scoped(
    &self,
    request: &CapabilityCallRequest,
    workflow_scope: Option<String>,
    cancellation: CapabilityCancellationToken,
  ) -> BoxFuture<'static, Result<Value, CapabilityFailure>> {
    let request = request.clone();
    let Some(workflow_scope) = workflow_scope else {
      return Box::pin(async { Err(state_scope_unavailable()) });
    };
    let store = match self.store.configured() {
      Ok(store) => store,
      Err(error) => return Box::pin(async move { Err(error) }),
    };
    Box::pin(async move {
      tokio::task::spawn_blocking(move || {
        if cancellation.is_cancelled() {
          return Err(state_cancelled());
        }
        store
          .execute(&workflow_scope, &request)
          .map(|execution| execution.result)
          .map_err(state_failure)
      })
      .await
      .map_err(|_| state_handler_crashed())?
    })
  }
}

impl std::fmt::Debug for DurableStateStore {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    formatter
      .debug_struct("DurableStateStore")
      .field("path", &self.path)
      .field("state_location_digest", &self.state_location_digest)
      .field("clock", &"dyn StateClock")
      .field("limits", &self.limits)
      .finish()
  }
}

impl DurableStateStore {
  pub fn open(path: impl AsRef<Path>) -> Result<Self, DurableStateError> {
    Self::open_with(
      path,
      Arc::new(SystemStateClock),
      DurableStateLimits::default(),
    )
  }

  pub fn open_with(
    path: impl AsRef<Path>,
    clock: Arc<dyn StateClock>,
    limits: DurableStateLimits,
  ) -> Result<Self, DurableStateError> {
    if limits.max_keys == 0
      || limits.max_keys > DEFAULT_STATE_MAX_KEYS
      || limits.max_bytes == 0
      || limits.max_bytes > DEFAULT_STATE_MAX_BYTES
      || limits.max_value_bytes == 0
      || limits.max_value_bytes > MAX_STATE_VALUE_BYTES
    {
      return Err(DurableStateError::InvalidRequest);
    }
    let path = absolute_lexical(path.as_ref()).map_err(|_| DurableStateError::StoreUnavailable)?;
    if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
      return Err(DurableStateError::StoreUnavailable);
    }
    DurableEventStore::open(&path)
      .map(drop)
      .map_err(map_durable_store_error)?;
    let canonical = fs::canonicalize(&path).map_err(|_| DurableStateError::StoreUnavailable)?;
    let state_location_digest = digest(
      b"woml.state-location\0v1\0",
      canonical.to_string_lossy().as_bytes(),
    );
    let store = Self {
      path: canonical,
      state_location_digest,
      clock,
      limits,
    };
    let connection = store.connection()?;
    validate_store(&connection)?;
    Ok(store)
  }

  pub fn execute(
    &self,
    workflow_id: &str,
    call: &CapabilityCallRequest,
  ) -> Result<DurableStateExecution, DurableStateError> {
    let started = Instant::now();
    if workflow_id.is_empty() || workflow_id.len() > 256 {
      return Err(DurableStateError::InvalidRequest);
    }
    call
      .validate()
      .map_err(|_| DurableStateError::InvalidRequest)?;
    if call.capability != "state"
      || call.input_contract_version != STATE_CONTRACT_VERSION
      || call.result_contract_version != STATE_CONTRACT_VERSION
    {
      return Err(DurableStateError::InvalidRequest);
    }
    let request = parse_request(&call.input, &call.operation)?;
    let key = state_key(&request.input)?;
    let scope = self.scope_digest(workflow_id);
    let key_digest = digest(b"woml.state-key\0v1\0", key.as_bytes());
    let input_json = canonical_string(&call.input)?;
    let input_digest = digest(b"woml.state-input\0v1\0", input_json.as_bytes());
    let mutation = if is_mutation(&request.operation) {
      Some(validate_mutation_identity(
        call,
        &request.operation,
        &input_digest,
      )?)
    } else {
      None
    };

    let mut connection = self.connection()?;
    let transaction = connection
      .transaction_with_behavior(TransactionBehavior::Immediate)
      .map_err(sqlite_unavailable)?;
    validate_store(&transaction)?;

    if let Some(identity) = &mutation {
      if let Some(stored) = read_mutation(&transaction, &identity.operation_key)? {
        if stored.scope_digest != scope
          || stored.operation_name != identity.operation_name
          || stored.operation != request.operation
          || stored.key_digest != key_digest
          || stored.input_digest != identity.input_digest
        {
          return Err(DurableStateError::OperationIdentityConflict);
        }
        let result: Value =
          serde_json::from_str(&stored.result_json).map_err(|_| DurableStateError::StoreCorrupt)?;
        if canonical_digest(b"woml.state-result\0v1\0", &result)? != stored.result_digest {
          return Err(DurableStateError::StoreCorrupt);
        }
        transaction.commit().map_err(sqlite_unavailable)?;
        return Ok(DurableStateExecution {
          result,
          duplicate: true,
          operation: request.operation,
          key_digest,
          input_digest,
          result_digest: stored.result_digest,
          version: stored.committed_version,
          value_bytes: None,
          duration_ms: started.elapsed().as_secs_f64() * 1_000.0,
        });
      }
    }

    let current = read_entry(&transaction, &scope, key)?;
    let now = state_timestamp(self.clock.now_millis())?;
    let (data, committed_version, value_bytes) = match request.operation.as_str() {
      "get" => {
        let data = match current {
          Some(entry) => json!({
            "found": true,
            "value": parse_stored_value(&entry.value_json)?,
            "version": entry.version,
            "updatedAt": entry.updated_at,
          }),
          None => json!({ "found": false }),
        };
        (data, None, None)
      }
      "has" => {
        let data = match current {
          Some(entry) => json!({ "present": true, "version": entry.version }),
          None => json!({ "present": false }),
        };
        (data, None, None)
      }
      "set" => {
        enforce_version(&request.input, current.as_ref())?;
        let (value_json, bytes) = state_value(&request.input, self.limits.max_value_bytes)?;
        let version = next_version(&transaction, &scope, &key_digest, current.as_ref())?;
        enforce_quota(
          &transaction,
          &scope,
          current.as_ref(),
          Some(bytes),
          self.limits,
        )?;
        write_entry(
          &transaction,
          &scope,
          &key_digest,
          key,
          &value_json,
          bytes,
          version,
          &now,
        )?;
        (
          json!({ "stored": true, "version": version, "updatedAt": now }),
          Some(version),
          Some(bytes),
        )
      }
      "delete" => {
        enforce_version(&request.input, current.as_ref())?;
        let deleted = current.is_some();
        let version = if deleted {
          let version = next_version(&transaction, &scope, &key_digest, current.as_ref())?;
          enforce_quota(&transaction, &scope, current.as_ref(), None, self.limits)?;
          transaction
            .execute(
              "DELETE FROM woml_state_entries WHERE scope_digest = ?1 AND key_text = ?2",
              params![scope, key],
            )
            .map_err(sqlite_unavailable)?;
          Some(version)
        } else {
          None
        };
        (json!({ "deleted": deleted }), version, None)
      }
      "increment" => {
        enforce_version(&request.input, current.as_ref())?;
        let amount = request
          .input
          .get("amount")
          .and_then(Value::as_i64)
          .ok_or(DurableStateError::IntegerOverflow)?;
        if !(-MAX_STATE_SAFE_INTEGER..=MAX_STATE_SAFE_INTEGER).contains(&amount) {
          return Err(DurableStateError::IntegerOverflow);
        }
        let existing = match current.as_ref() {
          Some(entry) => parse_safe_integer(&entry.value_json)?,
          None => 0,
        };
        let value = existing
          .checked_add(amount)
          .filter(|value| (-MAX_STATE_SAFE_INTEGER..=MAX_STATE_SAFE_INTEGER).contains(value))
          .ok_or(DurableStateError::IntegerOverflow)?;
        let value_json = value.to_string();
        let bytes = value_json.len() as u64;
        let version = next_version(&transaction, &scope, &key_digest, current.as_ref())?;
        enforce_quota(
          &transaction,
          &scope,
          current.as_ref(),
          Some(bytes),
          self.limits,
        )?;
        write_entry(
          &transaction,
          &scope,
          &key_digest,
          key,
          &value_json,
          bytes,
          version,
          &now,
        )?;
        (
          json!({ "value": value, "version": version, "updatedAt": now }),
          Some(version),
          Some(bytes),
        )
      }
      "set_if_absent" => match current {
        Some(entry) => (
          json!({
            "stored": false,
            "value": parse_stored_value(&entry.value_json)?,
            "version": entry.version,
            "updatedAt": entry.updated_at,
          }),
          Some(entry.version),
          Some(entry.value_bytes),
        ),
        None => {
          let (value_json, bytes) = state_value(&request.input, self.limits.max_value_bytes)?;
          let version = next_version(&transaction, &scope, &key_digest, None)?;
          enforce_quota(&transaction, &scope, None, Some(bytes), self.limits)?;
          write_entry(
            &transaction,
            &scope,
            &key_digest,
            key,
            &value_json,
            bytes,
            version,
            &now,
          )?;
          (
            json!({
              "stored": true,
              "value": parse_stored_value(&value_json)?,
              "version": version,
              "updatedAt": now,
            }),
            Some(version),
            Some(bytes),
          )
        }
      },
      _ => return Err(DurableStateError::InvalidRequest),
    };
    let result = json!({
      "contract": STATE_CONTRACT,
      "contractVersion": STATE_CONTRACT_VERSION,
      "kind": "result",
      "operation": request.operation,
      "data": data,
    });
    let result_json = canonical_string(&result)?;
    let result_digest = digest(b"woml.state-result\0v1\0", result_json.as_bytes());
    if let Some(identity) = mutation {
      transaction
        .execute(
          "INSERT INTO woml_state_mutations(
             operation_key, scope_digest, operation_name, operation, key_digest,
             input_digest, result_json, result_digest, committed_version, committed_at
           ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
          params![
            identity.operation_key,
            scope,
            identity.operation_name,
            request.operation,
            key_digest,
            identity.input_digest,
            result_json,
            result_digest,
            committed_version,
            now,
          ],
        )
        .map_err(|error| {
          if matches!(
            error,
            rusqlite::Error::SqliteFailure(code, _)
              if code.code == ErrorCode::ConstraintViolation
          ) {
            DurableStateError::OperationIdentityConflict
          } else {
            DurableStateError::StoreUnavailable
          }
        })?;
    }
    transaction.commit().map_err(sqlite_unavailable)?;
    Ok(DurableStateExecution {
      result,
      duplicate: false,
      operation: request.operation,
      key_digest,
      input_digest,
      result_digest,
      version: committed_version,
      value_bytes,
      duration_ms: started.elapsed().as_secs_f64() * 1_000.0,
    })
  }

  pub fn scope_digest(&self, workflow_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"woml.state-scope\0v1\0");
    hasher.update(self.state_location_digest.as_bytes());
    hasher.update(b"\0");
    hasher.update(workflow_id.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
  }

  fn connection(&self) -> Result<Connection, DurableStateError> {
    let connection = Connection::open_with_flags(
      &self.path,
      OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(sqlite_unavailable)?;
    connection
      .busy_timeout(Duration::from_secs(5))
      .map_err(sqlite_unavailable)?;
    connection
      .execute_batch("PRAGMA foreign_keys = ON;")
      .map_err(sqlite_unavailable)?;
    Ok(connection)
  }
}

fn parse_request(
  input: &Value,
  expected_operation: &str,
) -> Result<StateRequest, DurableStateError> {
  let request: StateRequest =
    serde_json::from_value(input.clone()).map_err(|_| DurableStateError::InvalidRequest)?;
  if request.contract != STATE_CONTRACT
    || request.contract_version != STATE_CONTRACT_VERSION
    || request.kind != "request"
    || request.operation != expected_operation
    || !STATE_OPERATIONS.contains(&request.operation.as_str())
  {
    return Err(DurableStateError::InvalidRequest);
  }
  let allowed: &[&str] = match request.operation.as_str() {
    "get" | "has" => &["key"],
    "set" => &["key", "value", "ifVersion"],
    "delete" => &["key", "ifVersion"],
    "increment" => &["key", "amount", "ifVersion"],
    "set_if_absent" => &["key", "value"],
    _ => return Err(DurableStateError::InvalidRequest),
  };
  if request
    .input
    .keys()
    .any(|key| !allowed.contains(&key.as_str()))
    || !request.input.contains_key("key")
    || matches!(request.operation.as_str(), "set" | "set_if_absent")
      && !request.input.contains_key("value")
    || request.operation == "increment" && !request.input.contains_key("amount")
  {
    return Err(DurableStateError::InvalidRequest);
  }
  state_key(&request.input)?;
  if let Some(version) = request.input.get("ifVersion") {
    version
      .as_u64()
      .filter(|version| *version <= MAX_STATE_SAFE_INTEGER as u64)
      .ok_or(DurableStateError::InvalidRequest)?;
  }
  Ok(request)
}

fn state_key(input: &Map<String, Value>) -> Result<&str, DurableStateError> {
  input
    .get("key")
    .and_then(Value::as_str)
    .filter(|key| !key.is_empty() && key.len() <= MAX_STATE_KEY_BYTES)
    .ok_or(DurableStateError::KeyInvalid)
}

fn validate_mutation_identity(
  call: &CapabilityCallRequest,
  operation: &str,
  input_digest: &str,
) -> Result<MutationIdentity, DurableStateError> {
  if call.identity.mode != CapabilityIdentityMode::Named
    || call.identity.operation_key
      != derive_operation_key(
        &call.identity.step_idempotency_key,
        &call.identity.operation_name,
      )
  {
    return Err(DurableStateError::OperationNameInvalid);
  }
  let prefix = format!("state.{operation}.");
  let Some(author_name) = call.identity.operation_name.strip_prefix(&prefix) else {
    return Err(DurableStateError::OperationNameInvalid);
  };
  if !valid_operation_name(author_name) {
    return Err(DurableStateError::OperationNameInvalid);
  }
  Ok(MutationIdentity {
    operation_key: call.identity.operation_key.clone(),
    operation_name: call.identity.operation_name.clone(),
    input_digest: input_digest.to_string(),
  })
}

fn valid_operation_name(name: &str) -> bool {
  let mut bytes = name.bytes();
  bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
    && name.len() <= 128
    && bytes
      .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte))
}

fn is_mutation(operation: &str) -> bool {
  matches!(operation, "set" | "delete" | "increment" | "set_if_absent")
}

fn read_entry(
  transaction: &Transaction<'_>,
  scope: &str,
  key: &str,
) -> Result<Option<StateEntry>, DurableStateError> {
  transaction
    .query_row(
      "SELECT value_json, value_bytes, version, updated_at
       FROM woml_state_entries WHERE scope_digest = ?1 AND key_text = ?2",
      params![scope, key],
      |row| {
        Ok(StateEntry {
          value_json: row.get(0)?,
          value_bytes: row.get(1)?,
          version: row.get(2)?,
          updated_at: row.get(3)?,
        })
      },
    )
    .optional()
    .map_err(sqlite_unavailable)
}

fn read_mutation(
  transaction: &Transaction<'_>,
  operation_key: &str,
) -> Result<Option<StoredMutation>, DurableStateError> {
  transaction
    .query_row(
      "SELECT scope_digest, operation_name, operation, key_digest, input_digest,
              result_json, result_digest, committed_version
       FROM woml_state_mutations WHERE operation_key = ?1",
      [operation_key],
      |row| {
        Ok(StoredMutation {
          scope_digest: row.get(0)?,
          operation_name: row.get(1)?,
          operation: row.get(2)?,
          key_digest: row.get(3)?,
          input_digest: row.get(4)?,
          result_json: row.get(5)?,
          result_digest: row.get(6)?,
          committed_version: row.get(7)?,
        })
      },
    )
    .optional()
    .map_err(sqlite_unavailable)
}

fn write_entry(
  transaction: &Transaction<'_>,
  scope: &str,
  key_digest: &str,
  key: &str,
  value_json: &str,
  value_bytes: u64,
  version: u64,
  updated_at: &str,
) -> Result<(), DurableStateError> {
  transaction
    .execute(
      "INSERT INTO woml_state_entries(
         scope_digest, key_digest, key_text, value_json, value_bytes, version, updated_at
       ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
       ON CONFLICT(scope_digest, key_text) DO UPDATE SET
         key_digest = excluded.key_digest,
         value_json = excluded.value_json,
         value_bytes = excluded.value_bytes,
         version = excluded.version,
         updated_at = excluded.updated_at",
      params![
        scope,
        key_digest,
        key,
        value_json,
        value_bytes,
        version,
        updated_at
      ],
    )
    .map_err(sqlite_unavailable)?;
  Ok(())
}

fn enforce_version(
  input: &Map<String, Value>,
  current: Option<&StateEntry>,
) -> Result<(), DurableStateError> {
  let Some(expected) = input.get("ifVersion").and_then(Value::as_u64) else {
    return Ok(());
  };
  if (expected == 0 && current.is_none()) || current.is_some_and(|entry| entry.version == expected)
  {
    Ok(())
  } else {
    Err(DurableStateError::Conflict)
  }
}

fn next_version(
  transaction: &Transaction<'_>,
  scope: &str,
  key_digest: &str,
  current: Option<&StateEntry>,
) -> Result<u64, DurableStateError> {
  let previous: Option<u64> = transaction
    .query_row(
      "SELECT MAX(committed_version) FROM woml_state_mutations
       WHERE scope_digest = ?1 AND key_digest = ?2",
      params![scope, key_digest],
      |row| row.get(0),
    )
    .map_err(sqlite_unavailable)?;
  if current.is_some_and(|entry| previous != Some(entry.version)) {
    return Err(DurableStateError::StoreCorrupt);
  }
  previous
    .unwrap_or(0)
    .checked_add(1)
    .filter(|version| *version <= MAX_STATE_SAFE_INTEGER as u64)
    .ok_or(DurableStateError::StoreCorrupt)
}

fn enforce_quota(
  transaction: &Transaction<'_>,
  scope: &str,
  current: Option<&StateEntry>,
  replacement_bytes: Option<u64>,
  limits: DurableStateLimits,
) -> Result<(), DurableStateError> {
  let actual: (u64, u64) = transaction
    .query_row(
      "SELECT COUNT(*), COALESCE(SUM(value_bytes), 0)
       FROM woml_state_entries WHERE scope_digest = ?1",
      [scope],
      |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .map_err(sqlite_unavailable)?;
  let stored: Option<(u64, u64)> = transaction
    .query_row(
      "SELECT live_keys, value_bytes FROM woml_state_quotas WHERE scope_digest = ?1",
      [scope],
      |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(sqlite_unavailable)?;
  if stored.is_some_and(|stored| stored != actual) || stored.is_none() && actual != (0, 0) {
    return Err(DurableStateError::StoreCorrupt);
  }
  let next_keys = match (current, replacement_bytes) {
    (None, Some(_)) => actual.0.checked_add(1),
    (Some(_), None) => actual.0.checked_sub(1),
    _ => Some(actual.0),
  }
  .ok_or(DurableStateError::StoreCorrupt)?;
  let next_bytes = actual
    .1
    .checked_sub(current.map_or(0, |entry| entry.value_bytes))
    .and_then(|bytes| bytes.checked_add(replacement_bytes.unwrap_or(0)))
    .ok_or(DurableStateError::StoreCorrupt)?;
  if next_keys > limits.max_keys || next_bytes > limits.max_bytes {
    return Err(DurableStateError::QuotaExceeded);
  }
  transaction
    .execute(
      "INSERT INTO woml_state_quotas(scope_digest, live_keys, value_bytes)
       VALUES (?1, ?2, ?3)
       ON CONFLICT(scope_digest) DO UPDATE SET
         live_keys = excluded.live_keys,
         value_bytes = excluded.value_bytes",
      params![scope, next_keys, next_bytes],
    )
    .map_err(sqlite_unavailable)?;
  Ok(())
}

fn state_value(
  input: &Map<String, Value>,
  maximum: usize,
) -> Result<(String, u64), DurableStateError> {
  let value = input.get("value").ok_or(DurableStateError::ValueInvalid)?;
  let encoded = canonical_json(value).map_err(|_| DurableStateError::ValueInvalid)?;
  if encoded.len() > maximum {
    return Err(DurableStateError::ValueTooLarge);
  }
  let bytes = encoded.len() as u64;
  String::from_utf8(encoded)
    .map(|encoded| (encoded, bytes))
    .map_err(|_| DurableStateError::ValueInvalid)
}

fn parse_safe_integer(encoded: &str) -> Result<i64, DurableStateError> {
  let value: Value = serde_json::from_str(encoded).map_err(|_| DurableStateError::StoreCorrupt)?;
  if let Some(integer) = value.as_i64() {
    if (-MAX_STATE_SAFE_INTEGER..=MAX_STATE_SAFE_INTEGER).contains(&integer) {
      return Ok(integer);
    }
    return Err(DurableStateError::IntegerOverflow);
  }
  if value.is_number() {
    Err(DurableStateError::IntegerOverflow)
  } else {
    Err(DurableStateError::IntegerRequired)
  }
}

fn parse_stored_value(encoded: &str) -> Result<Value, DurableStateError> {
  let value: Value = serde_json::from_str(encoded).map_err(|_| DurableStateError::StoreCorrupt)?;
  if canonical_string(&value)? != encoded {
    return Err(DurableStateError::StoreCorrupt);
  }
  Ok(value)
}

fn canonical_string(value: &Value) -> Result<String, DurableStateError> {
  String::from_utf8(canonical_json(value).map_err(|_| DurableStateError::ValueInvalid)?)
    .map_err(|_| DurableStateError::ValueInvalid)
}

fn canonical_digest(domain: &[u8], value: &Value) -> Result<String, DurableStateError> {
  Ok(digest(domain, canonical_string(value)?.as_bytes()))
}

fn state_timestamp(milliseconds: i64) -> Result<String, DurableStateError> {
  DateTime::<Utc>::from_timestamp_millis(milliseconds)
    .map(|timestamp| timestamp.to_rfc3339_opts(SecondsFormat::Millis, true))
    .ok_or(DurableStateError::StoreCorrupt)
}

fn validate_store(connection: &Connection) -> Result<(), DurableStateError> {
  let version: Option<String> = connection
    .query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )
    .optional()
    .map_err(sqlite_unavailable)?;
  if version.as_deref() != Some("13") {
    return Err(DurableStateError::StoreCorrupt);
  }
  for table in [
    "woml_state_entries",
    "woml_state_mutations",
    "woml_state_quotas",
  ] {
    let exists: bool = connection
      .query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get(0),
      )
      .map_err(sqlite_unavailable)?;
    if !exists {
      return Err(DurableStateError::StoreCorrupt);
    }
  }
  Ok(())
}

fn digest(domain: &[u8], value: &[u8]) -> String {
  let mut hasher = Sha256::new();
  hasher.update(domain);
  hasher.update(value);
  format!("sha256:{}", hex::encode(hasher.finalize()))
}

fn absolute_lexical(path: &Path) -> std::io::Result<PathBuf> {
  if path.is_absolute() {
    Ok(path.to_path_buf())
  } else {
    Ok(std::env::current_dir()?.join(path))
  }
}

fn sqlite_unavailable(_error: rusqlite::Error) -> DurableStateError {
  DurableStateError::StoreUnavailable
}

fn map_durable_store_error(error: DurableStoreError) -> DurableStateError {
  match error {
    DurableStoreError::Contract(_) | DurableStoreError::UnsupportedStoreVersion(_) => {
      DurableStateError::StoreCorrupt
    }
    _ => DurableStateError::StoreUnavailable,
  }
}

fn state_failure(error: DurableStateError) -> CapabilityFailure {
  let kind = match error {
    DurableStateError::InvalidRequest
    | DurableStateError::KeyInvalid
    | DurableStateError::ValueInvalid
    | DurableStateError::OperationNameInvalid => CapabilityFailureKind::InvalidInput,
    DurableStateError::ValueTooLarge => CapabilityFailureKind::InputTooLarge,
    DurableStateError::StoreUnavailable => CapabilityFailureKind::TransportFailed,
    DurableStateError::StoreCorrupt => CapabilityFailureKind::InvalidResult,
    DurableStateError::Conflict
    | DurableStateError::OperationIdentityConflict
    | DurableStateError::QuotaExceeded
    | DurableStateError::IntegerRequired
    | DurableStateError::IntegerOverflow => CapabilityFailureKind::ServiceRejected,
  };
  CapabilityFailure {
    kind,
    code: error.code().to_string(),
    message: error.to_string(),
    retryable: error.retryable(),
    ambiguous: error.ambiguous(),
    details: None,
  }
}

fn state_store_unavailable() -> CapabilityFailure {
  state_failure(DurableStateError::StoreUnavailable)
}

fn state_scope_unavailable() -> CapabilityFailure {
  CapabilityFailure {
    kind: CapabilityFailureKind::ServiceRejected,
    code: "WOML_STATE_SCOPE_UNAVAILABLE".to_string(),
    message: "Durable User State requires an engine-owned workflow scope.".to_string(),
    retryable: false,
    ambiguous: false,
    details: None,
  }
}

fn state_cancelled() -> CapabilityFailure {
  CapabilityFailure {
    kind: CapabilityFailureKind::Cancelled,
    code: "WOML_STATE_CANCELLED".to_string(),
    message: "The Durable User State operation was cancelled.".to_string(),
    retryable: false,
    ambiguous: false,
    details: None,
  }
}

fn state_handler_crashed() -> CapabilityFailure {
  CapabilityFailure {
    kind: CapabilityFailureKind::HandlerCrashed,
    code: "WOML_STATE_HANDLER_CRASHED".to_string(),
    message: "The Durable User State handler stopped unexpectedly.".to_string(),
    retryable: false,
    ambiguous: false,
    details: None,
  }
}
