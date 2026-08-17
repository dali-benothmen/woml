//! Cache v1 capability with a bounded, workflow-scoped local SQLite backend.

use std::{
  fs,
  path::{Path, PathBuf},
  sync::{
    atomic::{AtomicI64, Ordering},
    Arc, Mutex,
  },
  time::Duration,
};

use chrono::{DateTime, SecondsFormat, Utc};
use futures_util::future::BoxFuture;
use rusqlite::{
  params, Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use serde_json_canonicalizer::to_vec as canonical_json;
use sha2::{Digest, Sha256};

use crate::{
  CapabilityCallRequest, CapabilityCancellationToken, CapabilityDescriptor, CapabilityEffect,
  CapabilityFailure, CapabilityFailureKind, CapabilityHandler,
};

pub const CACHE_CONTRACT: &str = "woml.cache";
pub const CACHE_CONTRACT_VERSION: u32 = 1;
pub const DEFAULT_CACHE_TTL_MS: u64 = 300_000;
pub const MAX_CACHE_TTL_MS: u64 = 2_592_000_000;
pub const MAX_CACHE_KEY_BYTES: usize = 256;
pub const MAX_CACHE_VALUE_BYTES: usize = 262_144;
pub const DEFAULT_CACHE_MAX_ENTRIES: u64 = 10_000;
pub const DEFAULT_CACHE_MAX_BYTES: u64 = 67_108_864;
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
const CACHE_OPERATIONS: [&str; 6] = ["get", "set", "delete", "has", "increment", "set_if_absent"];

const CACHE_SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA synchronous = FULL;
CREATE TABLE IF NOT EXISTS cache_metadata (
  singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
  contract TEXT NOT NULL,
  contract_version INTEGER NOT NULL,
  next_access_sequence INTEGER NOT NULL
);
INSERT OR IGNORE INTO cache_metadata(singleton, contract, contract_version, next_access_sequence)
VALUES (1, 'woml.cache-store', 1, 1);
CREATE TABLE IF NOT EXISTS cache_entries (
  workflow_scope TEXT NOT NULL,
  cache_key TEXT NOT NULL,
  value_json TEXT NOT NULL,
  value_bytes INTEGER NOT NULL,
  expires_at_ms INTEGER NOT NULL,
  last_access_sequence INTEGER NOT NULL,
  PRIMARY KEY (workflow_scope, cache_key)
);
CREATE INDEX IF NOT EXISTS cache_entries_expiry
  ON cache_entries(expires_at_ms);
CREATE INDEX IF NOT EXISTS cache_entries_lru
  ON cache_entries(last_access_sequence, workflow_scope, cache_key);
"#;

pub trait CacheClock: Send + Sync + 'static {
  fn now_millis(&self) -> i64;
}

#[derive(Debug)]
pub struct SystemCacheClock;

impl CacheClock for SystemCacheClock {
  fn now_millis(&self) -> i64 {
    Utc::now().timestamp_millis()
  }
}

#[derive(Debug)]
pub struct FixedCacheClock {
  now_millis: AtomicI64,
}

impl FixedCacheClock {
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

impl CacheClock for FixedCacheClock {
  fn now_millis(&self) -> i64 {
    self.now_millis.load(Ordering::SeqCst)
  }
}

#[derive(Debug, Clone, Copy)]
pub struct CacheLimits {
  pub max_entries: u64,
  pub max_bytes: u64,
  pub max_value_bytes: usize,
}

impl Default for CacheLimits {
  fn default() -> Self {
    Self {
      max_entries: DEFAULT_CACHE_MAX_ENTRIES,
      max_bytes: DEFAULT_CACHE_MAX_BYTES,
      max_value_bytes: MAX_CACHE_VALUE_BYTES,
    }
  }
}

#[derive(Clone)]
struct CacheConfiguration {
  path: PathBuf,
  state_scope: String,
}

pub struct ManagedCacheStore {
  configuration: Mutex<Option<CacheConfiguration>>,
  clock: Arc<dyn CacheClock>,
  limits: CacheLimits,
}

impl std::fmt::Debug for ManagedCacheStore {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    formatter
      .debug_struct("ManagedCacheStore")
      .field(
        "configured",
        &self
          .configuration
          .lock()
          .map(|configuration| configuration.is_some())
          .unwrap_or(false),
      )
      .field("limits", &self.limits)
      .finish()
  }
}

impl Default for ManagedCacheStore {
  fn default() -> Self {
    Self::new(Arc::new(SystemCacheClock), CacheLimits::default())
  }
}

impl ManagedCacheStore {
  pub fn new(clock: Arc<dyn CacheClock>, limits: CacheLimits) -> Self {
    Self {
      configuration: Mutex::new(None),
      clock,
      limits,
    }
  }

  pub fn configure_for_state(&self, state_path: &Path) -> Result<(), CapabilityFailure> {
    let parent = state_path.parent().unwrap_or_else(|| Path::new("."));
    let parent = fs::canonicalize(parent).map_err(|_| cache_unavailable())?;
    let state_name = state_path.file_name().ok_or_else(cache_unavailable)?;
    let normalized_state_path = parent.join(state_name);
    let state_scope = cache_state_digest(&normalized_state_path.to_string_lossy());
    let cache_name = if state_name == "state.sqlite" {
      "cache-v1.sqlite".to_string()
    } else {
      format!(
        "cache-v1-{}.sqlite",
        &state_scope.trim_start_matches("sha256:")[..16]
      )
    };
    let cache_path = parent.join(cache_name);
    if normalized_state_path == cache_path {
      return Err(cache_failure(
        CapabilityFailureKind::ServiceRejected,
        "WOML_CACHE_PATH_CONFLICT",
        "The WOML state database cannot use the reserved cache filename.",
        false,
        false,
      ));
    }
    self.configure(cache_path, state_scope)
  }

  pub fn configure_path(&self, path: PathBuf) -> Result<(), CapabilityFailure> {
    let normalized = absolute_lexical(&path).map_err(|_| cache_unavailable())?;
    let state_scope = cache_state_digest(&normalized.to_string_lossy());
    self.configure(path, state_scope)
  }

  fn configure(&self, path: PathBuf, state_scope: String) -> Result<(), CapabilityFailure> {
    if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
      return Err(cache_failure(
        CapabilityFailureKind::ServiceRejected,
        "WOML_CACHE_PATH_UNSAFE",
        "The local cache path is unsafe.",
        false,
        false,
      ));
    }
    let path = absolute_lexical(&path).map_err(|_| cache_unavailable())?;
    let mut configured = self.configuration.lock().expect("cache configuration lock");
    if let Some(existing) = configured.as_ref() {
      return if existing.path == path && existing.state_scope == state_scope {
        Ok(())
      } else {
        Err(cache_failure(
          CapabilityFailureKind::ServiceRejected,
          "WOML_CACHE_PATH_CONFLICT",
          "The runtime cache is already bound to another state location.",
          false,
          false,
        ))
      };
    }
    let connection = open_connection(&path)?;
    initialize(&connection)?;
    *configured = Some(CacheConfiguration { path, state_scope });
    Ok(())
  }

  fn configuration(&self) -> Result<CacheConfiguration, CapabilityFailure> {
    self
      .configuration
      .lock()
      .expect("cache configuration lock")
      .as_ref()
      .cloned()
      .ok_or_else(cache_unavailable)
  }

  fn execute(
    &self,
    workflow_scope: &str,
    request: &CacheRequest,
  ) -> Result<Value, CapabilityFailure> {
    let configuration = self.configuration()?;
    let mut connection = open_connection(&configuration.path)?;
    let transaction = connection
      .transaction_with_behavior(TransactionBehavior::Immediate)
      .map_err(sqlite_failure)?;
    validate_metadata(&transaction)?;
    let now = self.clock.now_millis();
    transaction
      .execute("DELETE FROM cache_entries WHERE expires_at_ms <= ?1", [now])
      .map_err(sqlite_failure)?;
    let scoped_workflow = format!("{}:{workflow_scope}", configuration.state_scope);
    let data = match request.operation.as_str() {
      "get" => self.get(&transaction, &scoped_workflow, &request.input, now)?,
      "set" => self.set(&transaction, &scoped_workflow, &request.input, now)?,
      "delete" => self.delete(&transaction, &scoped_workflow, &request.input)?,
      "has" => self.has(&transaction, &scoped_workflow, &request.input, now)?,
      "increment" => self.increment(&transaction, &scoped_workflow, &request.input, now)?,
      "set_if_absent" => self.set_if_absent(&transaction, &scoped_workflow, &request.input, now)?,
      _ => unreachable!("validated cache operation"),
    };
    transaction.commit().map_err(sqlite_failure)?;
    Ok(json!({
      "contract": CACHE_CONTRACT,
      "contractVersion": CACHE_CONTRACT_VERSION,
      "kind": "result",
      "operation": request.operation,
      "data": data,
    }))
  }

  fn get(
    &self,
    transaction: &Transaction<'_>,
    scope: &str,
    input: &Map<String, Value>,
    _now: i64,
  ) -> Result<Value, CapabilityFailure> {
    let key = key(input)?;
    let Some((value_json, expires_at)) = read_entry(transaction, scope, key)? else {
      return Ok(json!({ "hit": false }));
    };
    touch(transaction, scope, key)?;
    let value = parse_stored_json(&value_json)?;
    Ok(json!({
      "hit": true,
      "value": value,
      "expiresAt": timestamp(expires_at)?,
    }))
  }

  fn set(
    &self,
    transaction: &Transaction<'_>,
    scope: &str,
    input: &Map<String, Value>,
    now: i64,
  ) -> Result<Value, CapabilityFailure> {
    let key = key(input)?;
    let (value_json, value_bytes) = cache_value(input, self.limits.max_value_bytes)?;
    let expires_at = expiry(input, now)?;
    write_entry(
      transaction,
      scope,
      key,
      &value_json,
      value_bytes,
      expires_at,
    )?;
    evict(transaction, self.limits)?;
    Ok(json!({ "stored": true, "expiresAt": timestamp(expires_at)? }))
  }

  fn delete(
    &self,
    transaction: &Transaction<'_>,
    scope: &str,
    input: &Map<String, Value>,
  ) -> Result<Value, CapabilityFailure> {
    let deleted = transaction
      .execute(
        "DELETE FROM cache_entries WHERE workflow_scope = ?1 AND cache_key = ?2",
        params![scope, key(input)?],
      )
      .map_err(sqlite_failure)?
      > 0;
    Ok(json!({ "deleted": deleted }))
  }

  fn has(
    &self,
    transaction: &Transaction<'_>,
    scope: &str,
    input: &Map<String, Value>,
    _now: i64,
  ) -> Result<Value, CapabilityFailure> {
    let key = key(input)?;
    let present = read_entry(transaction, scope, key)?.is_some();
    if present {
      touch(transaction, scope, key)?;
    }
    Ok(json!({ "present": present }))
  }

  fn increment(
    &self,
    transaction: &Transaction<'_>,
    scope: &str,
    input: &Map<String, Value>,
    now: i64,
  ) -> Result<Value, CapabilityFailure> {
    let key = key(input)?;
    let amount = input
      .get("amount")
      .and_then(Value::as_i64)
      .filter(|value| value.abs() <= MAX_SAFE_INTEGER)
      .ok_or_else(|| cache_input("Cache increment amount must be a JSON safe integer."))?;
    let existing = read_entry(transaction, scope, key)?;
    let (current, expires_at) = match existing {
      Some((encoded, expires_at)) => {
        let value = parse_stored_json(&encoded)?;
        let integer = value
          .as_i64()
          .filter(|value| value.abs() <= MAX_SAFE_INTEGER)
          .ok_or_else(|| {
            cache_failure(
              CapabilityFailureKind::ServiceRejected,
              "WOML_CACHE_NOT_INTEGER",
              "The existing cache value is not a JSON safe integer.",
              false,
              false,
            )
          })?;
        (integer, expires_at)
      }
      None => (0, expiry(input, now)?),
    };
    let value = current
      .checked_add(amount)
      .filter(|value| value.abs() <= MAX_SAFE_INTEGER)
      .ok_or_else(|| {
        cache_failure(
          CapabilityFailureKind::ServiceRejected,
          "WOML_CACHE_INTEGER_OVERFLOW",
          "The cache increment would exceed the JSON safe integer range.",
          false,
          false,
        )
      })?;
    let encoded = value.to_string();
    write_entry(transaction, scope, key, &encoded, encoded.len(), expires_at)?;
    evict(transaction, self.limits)?;
    Ok(json!({ "value": value, "expiresAt": timestamp(expires_at)? }))
  }

  fn set_if_absent(
    &self,
    transaction: &Transaction<'_>,
    scope: &str,
    input: &Map<String, Value>,
    now: i64,
  ) -> Result<Value, CapabilityFailure> {
    let key = key(input)?;
    if let Some((encoded, expires_at)) = read_entry(transaction, scope, key)? {
      touch(transaction, scope, key)?;
      return Ok(json!({
        "stored": false,
        "value": parse_stored_json(&encoded)?,
        "expiresAt": timestamp(expires_at)?,
      }));
    }
    let (encoded, value_bytes) = cache_value(input, self.limits.max_value_bytes)?;
    let expires_at = expiry(input, now)?;
    write_entry(transaction, scope, key, &encoded, value_bytes, expires_at)?;
    evict(transaction, self.limits)?;
    Ok(json!({
      "stored": true,
      "value": input.get("value").cloned().unwrap_or(Value::Null),
      "expiresAt": timestamp(expires_at)?,
    }))
  }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CacheRequest {
  contract: String,
  contract_version: u32,
  kind: String,
  operation: String,
  input: Map<String, Value>,
}

#[derive(Debug)]
pub struct ManagedCacheHandler {
  store: Arc<ManagedCacheStore>,
  operation: &'static str,
}

impl ManagedCacheHandler {
  pub fn handlers(store: Arc<ManagedCacheStore>) -> Vec<Arc<Self>> {
    CACHE_OPERATIONS
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

impl CapabilityHandler for ManagedCacheHandler {
  fn descriptor(&self) -> CapabilityDescriptor {
    CapabilityDescriptor {
      capability: "cache".to_string(),
      operation: self.operation.to_string(),
      input_contract_version: 1,
      result_contract_version: 1,
      effect: match self.operation {
        "get" | "has" => CapabilityEffect::Read,
        "increment" => CapabilityEffect::UnsafeWrite,
        _ => CapabilityEffect::IdempotentWrite,
      },
      supports_cancellation: true,
      supports_provider_idempotency: false,
    }
  }

  fn validate_request(&self, request: &CapabilityCallRequest) -> Result<(), CapabilityFailure> {
    parse_request(&request.input, self.operation).map(|_| ())
  }

  fn safe_metadata(&self, input: &Value) -> Map<String, Value> {
    input
      .get("input")
      .and_then(|input| input.get("key"))
      .and_then(Value::as_str)
      .map(|key| {
        Map::from_iter([(
          "keyDigest".to_string(),
          Value::String(cache_key_digest(key)),
        )])
      })
      .unwrap_or_default()
  }

  fn safe_result_metadata(&self, result: &Value) -> Map<String, Value> {
    let Some(data) = result.get("data") else {
      return Map::new();
    };
    for field in ["hit", "stored", "deleted", "present"] {
      if let Some(value) = data.get(field).and_then(Value::as_bool) {
        return Map::from_iter([(field.to_string(), Value::Bool(value))]);
      }
    }
    Map::new()
  }

  fn execute(
    &self,
    _input: Value,
    _cancellation: CapabilityCancellationToken,
  ) -> BoxFuture<'static, Result<Value, CapabilityFailure>> {
    Box::pin(async { Err(cache_scope_unavailable()) })
  }

  fn execute_scoped(
    &self,
    input: Value,
    workflow_scope: Option<String>,
    cancellation: CapabilityCancellationToken,
  ) -> BoxFuture<'static, Result<Value, CapabilityFailure>> {
    let request = match parse_request(&input, self.operation) {
      Ok(request) => request,
      Err(error) => return Box::pin(async move { Err(error) }),
    };
    let Some(workflow_scope) = workflow_scope else {
      return Box::pin(async { Err(cache_scope_unavailable()) });
    };
    let scope = cache_scope_digest(&workflow_scope);
    let store = Arc::clone(&self.store);
    Box::pin(async move {
      tokio::task::spawn_blocking(move || {
        if cancellation.is_cancelled() {
          return Err(cache_cancelled());
        }
        store.execute(&scope, &request)
      })
      .await
      .map_err(|_| cache_unavailable())?
    })
  }
}

fn parse_request(
  input: &Value,
  expected_operation: &str,
) -> Result<CacheRequest, CapabilityFailure> {
  let request: CacheRequest =
    serde_json::from_value(input.clone()).map_err(|_| cache_input("Cache request is invalid."))?;
  if request.contract != CACHE_CONTRACT
    || request.contract_version != CACHE_CONTRACT_VERSION
    || request.kind != "request"
    || request.operation != expected_operation
  {
    return Err(cache_input("Cache request does not match Cache v1."));
  }
  validate_input(&request.operation, &request.input)?;
  Ok(request)
}

fn validate_input(operation: &str, input: &Map<String, Value>) -> Result<(), CapabilityFailure> {
  let allowed: &[&str] = match operation {
    "get" | "delete" | "has" => &["key"],
    "set" | "set_if_absent" => &["key", "value", "ttlMs"],
    "increment" => &["key", "amount", "ttlMs"],
    _ => return Err(cache_input("Cache operation is unsupported.")),
  };
  if input.keys().any(|field| !allowed.contains(&field.as_str()))
    || allowed.iter().any(|field| !input.contains_key(*field))
  {
    return Err(cache_input("Cache operation fields are invalid."));
  }
  key(input)?;
  if matches!(operation, "set" | "set_if_absent") {
    canonical_cache_value(
      input.get("value").expect("required cache value"),
      MAX_CACHE_VALUE_BYTES,
    )?;
    ttl_ms(input)?;
  }
  if operation == "increment" {
    input
      .get("amount")
      .and_then(Value::as_i64)
      .filter(|amount| amount.abs() <= MAX_SAFE_INTEGER)
      .ok_or_else(|| cache_input("Cache increment amount must be a JSON safe integer."))?;
    ttl_ms(input)?;
  }
  Ok(())
}

fn key(input: &Map<String, Value>) -> Result<&str, CapabilityFailure> {
  let key = input
    .get("key")
    .and_then(Value::as_str)
    .ok_or_else(|| cache_input("Cache key must be a string."))?;
  if key.is_empty()
    || key.len() > MAX_CACHE_KEY_BYTES
    || key.chars().any(|character| character.is_control())
  {
    return Err(cache_input(
      "Cache key must be non-empty, at most 256 UTF-8 bytes, and contain no control characters.",
    ));
  }
  Ok(key)
}

fn ttl_ms(input: &Map<String, Value>) -> Result<u64, CapabilityFailure> {
  input
    .get("ttlMs")
    .and_then(Value::as_u64)
    .filter(|ttl| (1..=MAX_CACHE_TTL_MS).contains(ttl))
    .ok_or_else(|| cache_input("Cache ttlMs must be from 1 ms through 30 days."))
}

fn expiry(input: &Map<String, Value>, now: i64) -> Result<i64, CapabilityFailure> {
  now
    .checked_add(i64::try_from(ttl_ms(input)?).map_err(|_| cache_input("Cache TTL is invalid."))?)
    .ok_or_else(|| cache_input("Cache expiry exceeds the supported timestamp range."))
}

fn cache_value(
  input: &Map<String, Value>,
  maximum: usize,
) -> Result<(String, usize), CapabilityFailure> {
  canonical_cache_value(input.get("value").expect("validated cache value"), maximum)
}

fn canonical_cache_value(
  value: &Value,
  maximum: usize,
) -> Result<(String, usize), CapabilityFailure> {
  let encoded = canonical_json(value).map_err(|_| cache_input("Cache value is invalid JSON."))?;
  if encoded.len() > maximum {
    return Err(cache_failure(
      CapabilityFailureKind::InputTooLarge,
      "WOML_CACHE_VALUE_TOO_LARGE",
      "The cache value exceeds 256 KiB.",
      false,
      false,
    ));
  }
  let length = encoded.len();
  String::from_utf8(encoded)
    .map(|encoded| (encoded, length))
    .map_err(|_| cache_input("Cache value is invalid JSON."))
}

fn open_connection(path: &Path) -> Result<Connection, CapabilityFailure> {
  if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
    return Err(cache_failure(
      CapabilityFailureKind::ServiceRejected,
      "WOML_CACHE_PATH_UNSAFE",
      "The local cache path is unsafe.",
      false,
      false,
    ));
  }
  let connection = Connection::open_with_flags(
    path,
    OpenFlags::SQLITE_OPEN_READ_WRITE
      | OpenFlags::SQLITE_OPEN_CREATE
      | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
  )
  .map_err(sqlite_failure)?;
  connection
    .busy_timeout(Duration::from_secs(5))
    .map_err(sqlite_failure)?;
  Ok(connection)
}

fn initialize(connection: &Connection) -> Result<(), CapabilityFailure> {
  connection
    .execute_batch(CACHE_SCHEMA)
    .map_err(sqlite_failure)?;
  let identity: (String, u32) = connection
    .query_row(
      "SELECT contract, contract_version FROM cache_metadata WHERE singleton = 1",
      [],
      |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .map_err(sqlite_failure)?;
  if identity != ("woml.cache-store".to_string(), 1) {
    return Err(cache_corrupt());
  }
  Ok(())
}

fn validate_metadata(transaction: &Transaction<'_>) -> Result<(), CapabilityFailure> {
  let identity: Option<(String, u32)> = transaction
    .query_row(
      "SELECT contract, contract_version FROM cache_metadata WHERE singleton = 1",
      [],
      |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(sqlite_failure)?;
  if identity != Some(("woml.cache-store".to_string(), 1)) {
    return Err(cache_corrupt());
  }
  Ok(())
}

fn next_sequence(transaction: &Transaction<'_>) -> Result<i64, CapabilityFailure> {
  let sequence: i64 = transaction
    .query_row(
      "SELECT next_access_sequence FROM cache_metadata WHERE singleton = 1",
      [],
      |row| row.get(0),
    )
    .map_err(sqlite_failure)?;
  let next = sequence.checked_add(1).ok_or_else(cache_corrupt)?;
  transaction
    .execute(
      "UPDATE cache_metadata SET next_access_sequence = ?1 WHERE singleton = 1",
      [next],
    )
    .map_err(sqlite_failure)?;
  Ok(sequence)
}

fn read_entry(
  transaction: &Transaction<'_>,
  scope: &str,
  key: &str,
) -> Result<Option<(String, i64)>, CapabilityFailure> {
  transaction
    .query_row(
      "SELECT value_json, expires_at_ms FROM cache_entries
       WHERE workflow_scope = ?1 AND cache_key = ?2",
      params![scope, key],
      |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(sqlite_failure)
}

fn touch(transaction: &Transaction<'_>, scope: &str, key: &str) -> Result<(), CapabilityFailure> {
  let sequence = next_sequence(transaction)?;
  transaction
    .execute(
      "UPDATE cache_entries SET last_access_sequence = ?3
       WHERE workflow_scope = ?1 AND cache_key = ?2",
      params![scope, key, sequence],
    )
    .map_err(sqlite_failure)?;
  Ok(())
}

fn write_entry(
  transaction: &Transaction<'_>,
  scope: &str,
  key: &str,
  value_json: &str,
  value_bytes: usize,
  expires_at: i64,
) -> Result<(), CapabilityFailure> {
  let sequence = next_sequence(transaction)?;
  transaction
    .execute(
      "INSERT INTO cache_entries(
         workflow_scope, cache_key, value_json, value_bytes, expires_at_ms, last_access_sequence
       ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
       ON CONFLICT(workflow_scope, cache_key) DO UPDATE SET
         value_json = excluded.value_json,
         value_bytes = excluded.value_bytes,
         expires_at_ms = excluded.expires_at_ms,
         last_access_sequence = excluded.last_access_sequence",
      params![
        scope,
        key,
        value_json,
        value_bytes as i64,
        expires_at,
        sequence
      ],
    )
    .map_err(sqlite_failure)?;
  Ok(())
}

fn evict(transaction: &Transaction<'_>, limits: CacheLimits) -> Result<(), CapabilityFailure> {
  loop {
    let (entries, bytes): (u64, u64) = transaction
      .query_row(
        "SELECT COUNT(*), COALESCE(SUM(value_bytes), 0) FROM cache_entries",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
      )
      .map_err(sqlite_failure)?;
    if entries <= limits.max_entries && bytes <= limits.max_bytes {
      return Ok(());
    }
    let removed = transaction
      .execute(
        "DELETE FROM cache_entries WHERE rowid = (
           SELECT rowid FROM cache_entries
           ORDER BY last_access_sequence ASC, workflow_scope ASC, cache_key ASC LIMIT 1
         )",
        [],
      )
      .map_err(sqlite_failure)?;
    if removed != 1 {
      return Err(cache_corrupt());
    }
  }
}

fn parse_stored_json(value: &str) -> Result<Value, CapabilityFailure> {
  serde_json::from_str(value).map_err(|_| cache_corrupt())
}

fn timestamp(milliseconds: i64) -> Result<String, CapabilityFailure> {
  DateTime::<Utc>::from_timestamp_millis(milliseconds)
    .map(|timestamp| timestamp.to_rfc3339_opts(SecondsFormat::Millis, true))
    .ok_or_else(cache_corrupt)
}

fn cache_scope_digest(workflow_id: &str) -> String {
  digest(b"woml.cache-scope\0v1\0", workflow_id)
}

fn cache_state_digest(state_identity: &str) -> String {
  digest(b"woml.cache-state\0v1\0", state_identity)
}

fn cache_key_digest(key: &str) -> String {
  digest(b"woml.cache-key\0v1\0", key)
}

fn digest(domain: &[u8], value: &str) -> String {
  let mut digest = Sha256::new();
  digest.update(domain);
  digest.update(value.as_bytes());
  format!("sha256:{}", hex::encode(digest.finalize()))
}

fn absolute_lexical(path: &Path) -> std::io::Result<PathBuf> {
  if path.is_absolute() {
    Ok(path.to_path_buf())
  } else {
    Ok(std::env::current_dir()?.join(path))
  }
}

fn sqlite_failure(_error: rusqlite::Error) -> CapabilityFailure {
  cache_unavailable()
}

fn cache_input(message: &str) -> CapabilityFailure {
  cache_failure(
    CapabilityFailureKind::InvalidInput,
    "WOML_CACHE_INPUT_INVALID",
    message,
    false,
    false,
  )
}

fn cache_corrupt() -> CapabilityFailure {
  cache_failure(
    CapabilityFailureKind::InvalidResult,
    "WOML_CACHE_CORRUPT",
    "The local cache failed its integrity checks.",
    false,
    false,
  )
}

fn cache_unavailable() -> CapabilityFailure {
  cache_failure(
    CapabilityFailureKind::TransportFailed,
    "WOML_CACHE_UNAVAILABLE",
    "The local cache is unavailable.",
    true,
    false,
  )
}

fn cache_scope_unavailable() -> CapabilityFailure {
  cache_failure(
    CapabilityFailureKind::ServiceRejected,
    "WOML_CACHE_SCOPE_UNAVAILABLE",
    "Cache access requires a Rust-owned workflow scope.",
    false,
    false,
  )
}

fn cache_cancelled() -> CapabilityFailure {
  cache_failure(
    CapabilityFailureKind::Cancelled,
    "WOML_CACHE_CANCELLED",
    "The cache operation was cancelled.",
    false,
    false,
  )
}

fn cache_failure(
  kind: CapabilityFailureKind,
  code: &str,
  message: &str,
  retryable: bool,
  ambiguous: bool,
) -> CapabilityFailure {
  CapabilityFailure {
    kind,
    code: code.to_string(),
    message: message.to_string(),
    retryable,
    ambiguous,
    details: None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  struct TestCache(PathBuf);

  impl TestCache {
    fn new(label: &str) -> Self {
      let root = std::env::temp_dir().join(format!(
        "woml-sc10-{label}-{}",
        uuid::Uuid::new_v4().simple()
      ));
      fs::create_dir_all(&root).unwrap();
      Self(root.join("cache.sqlite"))
    }
  }

  impl Drop for TestCache {
    fn drop(&mut self) {
      if let Some(parent) = self.0.parent() {
        let _ = fs::remove_dir_all(parent);
      }
    }
  }

  fn request(operation: &str, input: Value) -> CacheRequest {
    parse_request(
      &json!({
        "contract": CACHE_CONTRACT,
        "contractVersion": 1,
        "kind": "request",
        "operation": operation,
        "input": input,
      }),
      operation,
    )
    .unwrap()
  }

  fn data(result: Value) -> Value {
    result.get("data").cloned().unwrap()
  }

  #[test]
  fn miss_null_and_exact_expiry_boundary_are_deterministic() {
    let path = TestCache::new("expiry");
    let clock = Arc::new(FixedCacheClock::new(2_000_000_000_000));
    let store = ManagedCacheStore::new(clock.clone(), CacheLimits::default());
    store.configure_path(path.0.clone()).unwrap();

    let miss = data(
      store
        .execute("workflow-a", &request("get", json!({ "key": "missing" })))
        .unwrap(),
    );
    assert_eq!(miss, json!({ "hit": false }));

    store
      .execute(
        "workflow-a",
        &request(
          "set",
          json!({ "key": "nullable", "value": null, "ttlMs": 10 }),
        ),
      )
      .unwrap();
    let hit = data(
      store
        .execute("workflow-a", &request("get", json!({ "key": "nullable" })))
        .unwrap(),
    );
    assert_eq!(hit.get("hit"), Some(&Value::Bool(true)));
    assert_eq!(hit.get("value"), Some(&Value::Null));

    clock.advance(9);
    assert_eq!(
      data(
        store
          .execute("workflow-a", &request("has", json!({ "key": "nullable" })))
          .unwrap()
      ),
      json!({ "present": true })
    );
    clock.advance(1);
    assert_eq!(
      data(
        store
          .execute("workflow-a", &request("get", json!({ "key": "nullable" })))
          .unwrap()
      ),
      json!({ "hit": false })
    );
  }

  #[test]
  fn restart_and_workflow_definition_scope_behave_as_frozen() {
    let path = TestCache::new("restart");
    let clock = Arc::new(FixedCacheClock::new(2_000_000_000_000));
    {
      let store = ManagedCacheStore::new(clock.clone(), CacheLimits::default());
      store.configure_path(path.0.clone()).unwrap();
      store
        .execute(
          "same-workflow-id",
          &request(
            "set",
            json!({ "key": "warm", "value": { "version": 1 }, "ttlMs": 1000 }),
          ),
        )
        .unwrap();
    }

    let restarted = ManagedCacheStore::new(clock, CacheLimits::default());
    restarted.configure_path(path.0.clone()).unwrap();
    let same_definition_update = data(
      restarted
        .execute(
          "same-workflow-id",
          &request("get", json!({ "key": "warm" })),
        )
        .unwrap(),
    );
    assert_eq!(same_definition_update.get("hit"), Some(&Value::Bool(true)));
    assert_eq!(
      data(
        restarted
          .execute(
            "different-workflow-id",
            &request("get", json!({ "key": "warm" }))
          )
          .unwrap()
      ),
      json!({ "hit": false })
    );
  }

  #[test]
  fn lru_eviction_is_bounded_and_access_ordered() {
    let path = TestCache::new("eviction");
    let store = ManagedCacheStore::new(
      Arc::new(FixedCacheClock::new(2_000_000_000_000)),
      CacheLimits {
        max_entries: 2,
        max_bytes: 1024,
        max_value_bytes: 1024,
      },
    );
    store.configure_path(path.0.clone()).unwrap();
    for key in ["a", "b"] {
      store
        .execute(
          "scope",
          &request("set", json!({ "key": key, "value": key, "ttlMs": 1000 })),
        )
        .unwrap();
    }
    store
      .execute("scope", &request("get", json!({ "key": "a" })))
      .unwrap();
    store
      .execute(
        "scope",
        &request("set", json!({ "key": "c", "value": "c", "ttlMs": 1000 })),
      )
      .unwrap();
    assert_eq!(
      data(
        store
          .execute("scope", &request("get", json!({ "key": "b" })))
          .unwrap()
      ),
      json!({ "hit": false })
    );
    assert_eq!(
      data(
        store
          .execute("scope", &request("has", json!({ "key": "a" })))
          .unwrap()
      ),
      json!({ "present": true })
    );
  }

  #[test]
  fn byte_limit_evicts_and_corrupt_json_fails_closed() {
    let path = TestCache::new("bytes-corrupt");
    let store = ManagedCacheStore::new(
      Arc::new(FixedCacheClock::new(2_000_000_000_000)),
      CacheLimits {
        max_entries: 10,
        max_bytes: 7,
        max_value_bytes: 1024,
      },
    );
    store.configure_path(path.0.clone()).unwrap();
    for key in ["a", "b"] {
      store
        .execute(
          "scope",
          &request("set", json!({ "key": key, "value": "xy", "ttlMs": 1000 })),
        )
        .unwrap();
    }
    assert_eq!(
      data(
        store
          .execute("scope", &request("get", json!({ "key": "a" })))
          .unwrap()
      ),
      json!({ "hit": false })
    );

    let connection = Connection::open(&path.0).unwrap();
    connection
      .execute(
        "UPDATE cache_entries SET value_json = 'not-json' WHERE cache_key = 'b'",
        [],
      )
      .unwrap();
    let failure = store
      .execute("scope", &request("get", json!({ "key": "b" })))
      .unwrap_err();
    assert_eq!(failure.code, "WOML_CACHE_CORRUPT");
  }

  #[test]
  fn increment_and_set_if_absent_are_atomic_across_concurrent_run_connections() {
    let path = TestCache::new("atomic");
    let store = Arc::new(ManagedCacheStore::new(
      Arc::new(FixedCacheClock::new(2_000_000_000_000)),
      CacheLimits::default(),
    ));
    store.configure_path(path.0.clone()).unwrap();
    let mut workers = Vec::new();
    for _ in 0..24 {
      let store = Arc::clone(&store);
      workers.push(std::thread::spawn(move || {
        store
          .execute(
            "scope",
            &request(
              "increment",
              json!({ "key": "counter", "amount": 1, "ttlMs": 1000 }),
            ),
          )
          .unwrap();
        data(
          store
            .execute(
              "scope",
              &request(
                "set_if_absent",
                json!({ "key": "winner", "value": 1, "ttlMs": 1000 }),
              ),
            )
            .unwrap(),
        )
      }));
    }
    let winners = workers
      .into_iter()
      .map(|worker| worker.join().unwrap())
      .filter(|result| result.get("stored") == Some(&Value::Bool(true)))
      .count();
    assert_eq!(winners, 1);
    let counter = data(
      store
        .execute("scope", &request("get", json!({ "key": "counter" })))
        .unwrap(),
    );
    assert_eq!(counter.get("value"), Some(&Value::from(24)));
  }

  #[test]
  fn invalid_limits_and_integer_failures_do_not_modify_entries() {
    let path = TestCache::new("limits");
    let store = ManagedCacheStore::new(
      Arc::new(FixedCacheClock::new(2_000_000_000_000)),
      CacheLimits {
        max_entries: 10,
        max_bytes: 1024,
        max_value_bytes: 8,
      },
    );
    store.configure_path(path.0.clone()).unwrap();
    let oversized = store.execute(
      "scope",
      &request(
        "set",
        json!({ "key": "large", "value": "too-large", "ttlMs": 1000 }),
      ),
    );
    assert_eq!(oversized.unwrap_err().code, "WOML_CACHE_VALUE_TOO_LARGE");

    store
      .execute(
        "scope",
        &request("set", json!({ "key": "text", "value": "x", "ttlMs": 1000 })),
      )
      .unwrap();
    let failure = store
      .execute(
        "scope",
        &request(
          "increment",
          json!({ "key": "text", "amount": 1, "ttlMs": 1000 }),
        ),
      )
      .unwrap_err();
    assert_eq!(failure.code, "WOML_CACHE_NOT_INTEGER");
    let existing = data(
      store
        .execute("scope", &request("get", json!({ "key": "text" })))
        .unwrap(),
    );
    assert_eq!(existing.get("value"), Some(&Value::String("x".to_string())));
  }
}
