use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use chrono::{DateTime, Utc};
use rusqlite::{
  params, Connection, ErrorCode, OpenFlags, OptionalExtension, Transaction, TransactionBehavior,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const RETENTION_BATCH_RUNS: usize = 250;
const MAINTENANCE_LEASE_SECONDS: i64 = 300;
pub const TRIGGER_DEDUPLICATION_SAFETY_DAYS: i64 = 30;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetentionPolicyV1 {
  pub policy_id: String,
  pub succeeded_before: DateTime<Utc>,
  pub failed_before: DateTime<Utc>,
  pub cancelled_before: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetentionPlanV1 {
  pub profile: &'static str,
  pub kind: &'static str,
  pub policy_id: String,
  pub succeeded_before: DateTime<Utc>,
  pub failed_before: DateTime<Utc>,
  pub cancelled_before: DateTime<Utc>,
  pub eligible_runs: u64,
  pub estimated_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetentionResultV1 {
  pub profile: String,
  pub kind: String,
  pub policy_id: String,
  pub completed_at: DateTime<Utc>,
  pub deleted_runs: u64,
  pub deleted_bytes: u64,
  pub state_entries_deleted: u64,
}

impl RetentionResultV1 {
  fn new(policy_id: String, completed_at: DateTime<Utc>) -> Self {
    Self {
      profile: "woml.retention/v1".to_string(),
      kind: "result".to_string(),
      policy_id,
      completed_at,
      deleted_runs: 0,
      deleted_bytes: 0,
      state_entries_deleted: 0,
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetentionExecutionV1 {
  pub result: RetentionResultV1,
  pub batches: u64,
  pub checkpoint_busy: u64,
  pub checkpoint_log_frames: u64,
  pub checkpointed_frames: u64,
  pub compacted: bool,
}

#[derive(Debug, Error)]
pub enum RetentionError {
  #[error("the WOML retention store path is missing or unsafe")]
  StorePathUnsafe,
  #[error("Retention Policy v1 is invalid")]
  InvalidPolicy,
  #[error("another WOML maintenance operation is active")]
  MaintenanceBusy,
  #[error("the WOML store failed its integrity check before retention")]
  IntegrityFailed,
  #[error("SQLite has insufficient disk space for this maintenance operation")]
  DiskFull,
  #[error("the WOML retention operation could not acquire the SQLite writer")]
  StoreBusy,
  #[error(transparent)]
  Sqlite(#[from] rusqlite::Error),
  #[error(transparent)]
  Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
struct RunFact {
  status: String,
  updated_at: DateTime<Utc>,
}

#[derive(Debug, Default)]
struct RetentionInventory {
  facts: BTreeMap<String, RunFact>,
  adjacency: HashMap<String, BTreeSet<String>>,
  protected: HashSet<String>,
}

fn safe_existing_file(path: &Path) -> Result<PathBuf, RetentionError> {
  let metadata = fs::symlink_metadata(path).map_err(|_| RetentionError::StorePathUnsafe)?;
  if !metadata.is_file() || metadata.file_type().is_symlink() {
    return Err(RetentionError::StorePathUnsafe);
  }
  fs::canonicalize(path).map_err(RetentionError::Io)
}

fn map_sqlite(error: rusqlite::Error) -> RetentionError {
  match &error {
    rusqlite::Error::SqliteFailure(code, _) if code.code == ErrorCode::DiskFull => {
      RetentionError::DiskFull
    }
    rusqlite::Error::SqliteFailure(code, _)
      if matches!(
        code.code,
        ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
      ) =>
    {
      RetentionError::StoreBusy
    }
    _ => RetentionError::Sqlite(error),
  }
}

fn open(path: &Path) -> Result<Connection, RetentionError> {
  let connection = Connection::open_with_flags(
    safe_existing_file(path)?,
    OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
  )
  .map_err(map_sqlite)?;
  connection
    .busy_timeout(Duration::from_secs(5))
    .map_err(map_sqlite)?;
  connection
    .execute_batch("PRAGMA foreign_keys = ON;")
    .map_err(map_sqlite)?;
  Ok(connection)
}

fn valid_policy(policy: &RetentionPolicyV1, now: DateTime<Utc>) -> bool {
  !policy.policy_id.is_empty()
    && policy.policy_id.len() <= 320
    && !policy.policy_id.contains('\0')
    && policy.succeeded_before <= now
    && policy.failed_before <= now
    && policy.cancelled_before <= now
}

fn parse_time(value: String) -> Result<DateTime<Utc>, RetentionError> {
  DateTime::parse_from_rfc3339(&value)
    .map(|value| value.with_timezone(&Utc))
    .map_err(|_| RetentionError::IntegrityFailed)
}

fn quick_integrity(connection: &Connection) -> Result<(), RetentionError> {
  let check: String = connection
    .query_row("PRAGMA quick_check", [], |row| row.get(0))
    .map_err(map_sqlite)?;
  let foreign_key_failure: bool = connection
    .query_row(
      "SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_check)",
      [],
      |row| row.get(0),
    )
    .map_err(map_sqlite)?;
  if check != "ok" || foreign_key_failure {
    return Err(RetentionError::IntegrityFailed);
  }
  Ok(())
}

fn connect(inventory: &mut RetentionInventory, left: String, right: String) {
  inventory
    .adjacency
    .entry(left.clone())
    .or_default()
    .insert(right.clone());
  inventory.adjacency.entry(right).or_default().insert(left);
}

fn load_inventory(
  connection: &Connection,
  deduplication_cutoff: DateTime<Utc>,
) -> Result<RetentionInventory, RetentionError> {
  let mut inventory = RetentionInventory::default();
  let mut statement = connection
    .prepare(
      "SELECT runs.run_id, summaries.status, summaries.updated_at
       FROM woml_runs AS runs
       LEFT JOIN woml_run_summaries AS summaries ON summaries.run_id = runs.run_id
       ORDER BY runs.run_id",
    )
    .map_err(map_sqlite)?;
  let rows = statement
    .query_map([], |row| {
      Ok((
        row.get::<_, String>(0)?,
        row.get::<_, Option<String>>(1)?,
        row.get::<_, Option<String>>(2)?,
      ))
    })
    .map_err(map_sqlite)?
    .collect::<Result<Vec<_>, _>>()
    .map_err(map_sqlite)?;
  for (run_id, status, updated_at) in rows {
    let (Some(status), Some(updated_at)) = (status, updated_at) else {
      inventory.protected.insert(run_id);
      continue;
    };
    inventory.facts.insert(
      run_id,
      RunFact {
        status,
        updated_at: parse_time(updated_at)?,
      },
    );
  }

  let mut statement = connection
    .prepare("SELECT parent_run_id, child_run_id FROM woml_workflow_calls")
    .map_err(map_sqlite)?;
  for edge in statement
    .query_map([], |row| {
      Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })
    .map_err(map_sqlite)?
  {
    let (parent, child) = edge.map_err(map_sqlite)?;
    connect(&mut inventory, parent, child);
  }

  let mut statement = connection
    .prepare(
      "SELECT publications.parent_run_id, deliveries.run_id, publications.emitted_at
       FROM woml_internal_event_publications AS publications
       LEFT JOIN woml_internal_event_deliveries AS deliveries
         ON deliveries.publication_id = publications.publication_id",
    )
    .map_err(map_sqlite)?;
  for edge in statement
    .query_map([], |row| {
      Ok((
        row.get::<_, String>(0)?,
        row.get::<_, Option<String>>(1)?,
        row.get::<_, String>(2)?,
      ))
    })
    .map_err(map_sqlite)?
  {
    let (parent, child, emitted_at) = edge.map_err(map_sqlite)?;
    if parse_time(emitted_at)? > deduplication_cutoff {
      inventory.protected.insert(parent.clone());
      if let Some(child) = &child {
        inventory.protected.insert(child.clone());
      }
    }
    if let Some(child) = child {
      connect(&mut inventory, parent, child);
    }
  }

  for table in ["woml_scheduler_claims", "woml_runtime_policy_queue"] {
    let mut statement = connection
      .prepare(&format!("SELECT run_id FROM {table}"))
      .map_err(map_sqlite)?;
    for run_id in statement
      .query_map([], |row| row.get::<_, String>(0))
      .map_err(map_sqlite)?
    {
      inventory.protected.insert(run_id.map_err(map_sqlite)?);
    }
  }

  let mut statement = connection
    .prepare("SELECT run_id, received_at FROM woml_trigger_occurrences")
    .map_err(map_sqlite)?;
  for occurrence in statement
    .query_map([], |row| {
      Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })
    .map_err(map_sqlite)?
  {
    let (run_id, received_at) = occurrence.map_err(map_sqlite)?;
    if parse_time(received_at)? > deduplication_cutoff {
      inventory.protected.insert(run_id);
    }
  }
  Ok(inventory)
}

fn cutoff_for<'a>(policy: &'a RetentionPolicyV1, status: &str) -> Option<&'a DateTime<Utc>> {
  match status {
    "succeeded" => Some(&policy.succeeded_before),
    "failed" => Some(&policy.failed_before),
    "cancelled" => Some(&policy.cancelled_before),
    _ => None,
  }
}

fn eligible_components(
  inventory: &RetentionInventory,
  policy: &RetentionPolicyV1,
) -> Vec<Vec<String>> {
  let all_ids = inventory
    .facts
    .keys()
    .chain(inventory.protected.iter())
    .cloned()
    .collect::<BTreeSet<_>>();
  let mut visited = HashSet::new();
  let mut groups: Vec<(DateTime<Utc>, String, Vec<String>)> = Vec::new();
  for root in all_ids {
    if !visited.insert(root.clone()) {
      continue;
    }
    let mut pending = VecDeque::from([root.clone()]);
    let mut component = Vec::new();
    while let Some(run_id) = pending.pop_front() {
      component.push(run_id.clone());
      for neighbor in inventory.adjacency.get(&run_id).into_iter().flatten() {
        if visited.insert(neighbor.clone()) {
          pending.push_back(neighbor.clone());
        }
      }
    }
    component.sort();
    if component.len() > RETENTION_BATCH_RUNS
      || component
        .iter()
        .any(|run_id| inventory.protected.contains(run_id))
    {
      continue;
    }
    let mut newest = DateTime::<Utc>::MIN_UTC;
    let mut eligible = true;
    for run_id in &component {
      let Some(fact) = inventory.facts.get(run_id) else {
        eligible = false;
        break;
      };
      let Some(cutoff) = cutoff_for(policy, &fact.status) else {
        eligible = false;
        break;
      };
      if fact.updated_at >= *cutoff {
        eligible = false;
        break;
      }
      newest = newest.max(fact.updated_at);
    }
    if eligible {
      groups.push((newest, component[0].clone(), component));
    }
  }
  groups.sort_by(|left, right| (left.0, &left.1).cmp(&(right.0, &right.1)));
  groups
    .into_iter()
    .map(|(_, _, component)| component)
    .collect()
}

fn install_candidates(connection: &Connection, run_ids: &[String]) -> Result<(), RetentionError> {
  connection
    .execute_batch(
      "CREATE TEMP TABLE IF NOT EXISTS woml_retention_candidates(
         run_id TEXT PRIMARY KEY
       ) WITHOUT ROWID;
       DELETE FROM woml_retention_candidates;",
    )
    .map_err(map_sqlite)?;
  let mut statement = connection
    .prepare("INSERT INTO woml_retention_candidates(run_id) VALUES (?1)")
    .map_err(map_sqlite)?;
  for run_id in run_ids {
    statement.execute([run_id]).map_err(map_sqlite)?;
  }
  Ok(())
}

fn logical_bytes(connection: &Connection, run_ids: &[String]) -> Result<u64, RetentionError> {
  if run_ids.is_empty() {
    return Ok(0);
  }
  install_candidates(connection, run_ids)?;
  let bytes: i64 = connection
    .query_row(
      "SELECT COALESCE(SUM(bytes), 0) FROM (
         SELECT length(runs.run_id) + length(runs.workflow_id) + length(runs.definition_hash)
              + length(runs.created_at) + 32 AS bytes
         FROM woml_runs AS runs JOIN woml_retention_candidates USING(run_id)
         UNION ALL
         SELECT length(events.run_id) + length(events.event_id) + length(events.event_json) + 32
         FROM woml_run_events AS events JOIN woml_retention_candidates USING(run_id)
         UNION ALL
         SELECT length(summaries.run_id) + length(summaries.workflow_id) + length(summaries.status)
              + length(summaries.admitted_at) + COALESCE(length(summaries.started_at), 0)
              + length(summaries.updated_at) + COALESCE(length(summaries.queue_name), 0)
              + COALESCE(length(summaries.waiting_for), 0) + COALESCE(length(summaries.eligible_at), 0) + 48
         FROM woml_run_summaries AS summaries JOIN woml_retention_candidates USING(run_id)
         UNION ALL
         SELECT length(tokens.token_id) + length(tokens.request_id) + length(tokens.run_id)
              + length(tokens.approval_id) + length(tokens.secret_hash) + 64
         FROM woml_approval_tokens AS tokens JOIN woml_retention_candidates USING(run_id)
         UNION ALL
         SELECT length(capabilities.capability_id) + length(capabilities.attempt_id)
              + length(capabilities.request_id) + length(capabilities.run_id)
              + length(capabilities.delivery_id) + length(capabilities.secret_hash) + 96
         FROM woml_notification_capabilities AS capabilities
         JOIN woml_retention_candidates USING(run_id)
         UNION ALL
         SELECT length(occurrences.occurrence_id) + length(occurrences.workflow_id)
              + length(occurrences.trigger_id) + length(occurrences.definition_hash)
              + length(occurrences.source_identity_hash) + length(occurrences.payload_hash) + 96
         FROM woml_trigger_occurrences AS occurrences JOIN woml_retention_candidates USING(run_id)
         UNION ALL
         SELECT length(calls.call_key) + length(calls.parent_run_id) + length(calls.child_run_id)
              + length(calls.target_definition_hash) + length(calls.payload_digest) + 96
         FROM woml_workflow_calls AS calls
         WHERE calls.parent_run_id IN (SELECT run_id FROM woml_retention_candidates)
            OR calls.child_run_id IN (SELECT run_id FROM woml_retention_candidates)
         UNION ALL
         SELECT length(publications.publication_id) + length(publications.parent_run_id)
              + length(publications.event_name) + length(publications.payload_hash) + 64
         FROM woml_internal_event_publications AS publications
         WHERE publications.parent_run_id IN (SELECT run_id FROM woml_retention_candidates)
         UNION ALL
         SELECT length(deliveries.publication_id) + length(deliveries.workflow_id)
              + length(deliveries.trigger_id) + length(deliveries.run_id) + 48
         FROM woml_internal_event_deliveries AS deliveries
         WHERE deliveries.run_id IN (SELECT run_id FROM woml_retention_candidates)
       )",
      [],
      |row| row.get(0),
    )
    .map_err(map_sqlite)?;
  u64::try_from(bytes).map_err(|_| RetentionError::IntegrityFailed)
}

fn candidate_ids(
  connection: &Connection,
  policy: &RetentionPolicyV1,
  now: DateTime<Utc>,
) -> Result<Vec<String>, RetentionError> {
  let deduplication_cutoff = now - chrono::Duration::days(TRIGGER_DEDUPLICATION_SAFETY_DAYS);
  Ok(
    eligible_components(&load_inventory(connection, deduplication_cutoff)?, policy)
      .into_iter()
      .filter(|component| component.len() <= RETENTION_BATCH_RUNS)
      .flatten()
      .collect(),
  )
}

fn candidate_batch_ids(
  connection: &Connection,
  policy: &RetentionPolicyV1,
  now: DateTime<Utc>,
) -> Result<Vec<String>, RetentionError> {
  let deduplication_cutoff = now - chrono::Duration::days(TRIGGER_DEDUPLICATION_SAFETY_DAYS);
  let components = eligible_components(&load_inventory(connection, deduplication_cutoff)?, policy);
  let mut batch = Vec::new();
  for component in components {
    // Dependency-connected runs must be deleted atomically. A component larger
    // than the v1 batch bound remains protected for a future explicit policy.
    if component.len() > RETENTION_BATCH_RUNS {
      continue;
    }
    if !batch.is_empty() && batch.len() + component.len() > RETENTION_BATCH_RUNS {
      break;
    }
    batch.extend(component);
    if batch.len() == RETENTION_BATCH_RUNS {
      break;
    }
  }
  Ok(batch)
}

pub fn plan_retention(
  path: impl AsRef<Path>,
  policy: &RetentionPolicyV1,
  now: DateTime<Utc>,
) -> Result<RetentionPlanV1, RetentionError> {
  if !valid_policy(policy, now) {
    return Err(RetentionError::InvalidPolicy);
  }
  let connection = open(path.as_ref())?;
  quick_integrity(&connection)?;
  let eligible = candidate_ids(&connection, policy, now)?;
  let estimated_bytes = logical_bytes(&connection, &eligible)?;
  Ok(RetentionPlanV1 {
    profile: "woml.retention/v1",
    kind: "plan",
    policy_id: policy.policy_id.clone(),
    succeeded_before: policy.succeeded_before,
    failed_before: policy.failed_before,
    cancelled_before: policy.cancelled_before,
    eligible_runs: eligible.len() as u64,
    estimated_bytes,
  })
}

fn acquire_lease(
  connection: &mut Connection,
  lease_id: &str,
  owner_id: &str,
  operation: &str,
  now: DateTime<Utc>,
) -> Result<(), RetentionError> {
  let transaction = connection
    .transaction_with_behavior(TransactionBehavior::Immediate)
    .map_err(map_sqlite)?;
  transaction
    .execute(
      "DELETE FROM woml_maintenance_lease WHERE singleton = 1 AND expires_at <= ?1",
      [now.to_rfc3339()],
    )
    .map_err(map_sqlite)?;
  let inserted = transaction
    .execute(
      "INSERT OR IGNORE INTO woml_maintenance_lease(
         singleton, lease_id, operation, owner_id, expires_at
       ) VALUES (1, ?1, ?2, ?3, ?4)",
      params![
        lease_id,
        operation,
        owner_id,
        (now + chrono::Duration::seconds(MAINTENANCE_LEASE_SECONDS)).to_rfc3339(),
      ],
    )
    .map_err(map_sqlite)?;
  if inserted != 1 {
    return Err(RetentionError::MaintenanceBusy);
  }
  transaction.commit().map_err(map_sqlite)?;
  Ok(())
}

fn renew_lease(
  connection: &Connection,
  lease_id: &str,
  now: DateTime<Utc>,
) -> Result<(), RetentionError> {
  let updated = connection
    .execute(
      "UPDATE woml_maintenance_lease SET expires_at = ?1
       WHERE singleton = 1 AND lease_id = ?2 AND operation = 'retention'",
      params![
        (now + chrono::Duration::seconds(MAINTENANCE_LEASE_SECONDS)).to_rfc3339(),
        lease_id,
      ],
    )
    .map_err(map_sqlite)?;
  if updated != 1 {
    return Err(RetentionError::MaintenanceBusy);
  }
  Ok(())
}

fn release_lease(connection: &Connection, lease_id: &str) {
  let _ = connection.execute(
    "DELETE FROM woml_maintenance_lease WHERE singleton = 1 AND lease_id = ?1",
    [lease_id],
  );
}

const DROP_RETENTION_GUARDS: &str = r#"
DROP TRIGGER woml_runs_no_delete;
DROP TRIGGER woml_run_events_no_delete;
DROP TRIGGER woml_approval_tokens_no_delete;
DROP TRIGGER woml_notification_capabilities_no_delete;
DROP TRIGGER woml_trigger_occurrences_no_delete;
DROP TRIGGER woml_internal_event_publications_no_delete;
DROP TRIGGER woml_internal_event_deliveries_no_delete;
DROP TRIGGER woml_workflow_calls_no_delete;
"#;

const RESTORE_RETENTION_GUARDS: &str = r#"
CREATE TRIGGER woml_runs_no_delete BEFORE DELETE ON woml_runs BEGIN
  SELECT RAISE(ABORT, 'WOML run bindings are immutable');
END;
CREATE TRIGGER woml_run_events_no_delete BEFORE DELETE ON woml_run_events BEGIN
  SELECT RAISE(ABORT, 'WOML run events are append-only');
END;
CREATE TRIGGER woml_approval_tokens_no_delete BEFORE DELETE ON woml_approval_tokens BEGIN
  SELECT RAISE(ABORT, 'WOML approval credentials are append-only');
END;
CREATE TRIGGER woml_notification_capabilities_no_delete BEFORE DELETE ON woml_notification_capabilities BEGIN
  SELECT RAISE(ABORT, 'WOML notification capabilities are append-only');
END;
CREATE TRIGGER woml_trigger_occurrences_no_delete BEFORE DELETE ON woml_trigger_occurrences BEGIN
  SELECT RAISE(ABORT, 'WOML trigger occurrences are immutable');
END;
CREATE TRIGGER woml_internal_event_publications_no_delete BEFORE DELETE ON woml_internal_event_publications BEGIN
  SELECT RAISE(ABORT, 'WOML internal event publications are immutable');
END;
CREATE TRIGGER woml_internal_event_deliveries_no_delete BEFORE DELETE ON woml_internal_event_deliveries BEGIN
  SELECT RAISE(ABORT, 'WOML internal event deliveries are immutable');
END;
CREATE TRIGGER woml_workflow_calls_no_delete BEFORE DELETE ON woml_workflow_calls BEGIN
  SELECT RAISE(ABORT, 'WOML workflow calls are durable');
END;
"#;

fn delete_batch(
  transaction: &Transaction<'_>,
  run_ids: &[String],
  policy: &RetentionPolicyV1,
  now: DateTime<Utc>,
) -> Result<(u64, u64), RetentionError> {
  if run_ids.is_empty() {
    return Ok((0, 0));
  }
  install_candidates(transaction, run_ids)?;
  let current = candidate_ids(transaction, policy, now)?;
  let current = current.into_iter().collect::<HashSet<_>>();
  if run_ids.iter().any(|run_id| !current.contains(run_id)) {
    return Ok((0, 0));
  }
  let bytes = logical_bytes(transaction, run_ids)?;
  transaction
    .execute_batch(DROP_RETENTION_GUARDS)
    .map_err(map_sqlite)?;
  for statement in [
    "DELETE FROM woml_scheduler_claims WHERE run_id IN (SELECT run_id FROM woml_retention_candidates)",
    "DELETE FROM woml_runtime_policy_queue WHERE run_id IN (SELECT run_id FROM woml_retention_candidates)",
    "DELETE FROM woml_runtime_policy_starts WHERE run_id IN (SELECT run_id FROM woml_retention_candidates)",
    "DELETE FROM woml_runtime_policy_bindings WHERE run_id IN (SELECT run_id FROM woml_retention_candidates)",
    "DELETE FROM woml_notification_capabilities WHERE run_id IN (SELECT run_id FROM woml_retention_candidates)",
    "DELETE FROM woml_approval_tokens WHERE run_id IN (SELECT run_id FROM woml_retention_candidates)",
    "DELETE FROM woml_internal_event_deliveries WHERE run_id IN (SELECT run_id FROM woml_retention_candidates) OR publication_id IN (SELECT publication_id FROM woml_internal_event_publications WHERE parent_run_id IN (SELECT run_id FROM woml_retention_candidates))",
    "DELETE FROM woml_internal_event_publications WHERE parent_run_id IN (SELECT run_id FROM woml_retention_candidates)",
    "DELETE FROM woml_workflow_calls WHERE parent_run_id IN (SELECT run_id FROM woml_retention_candidates) OR child_run_id IN (SELECT run_id FROM woml_retention_candidates)",
    "DELETE FROM woml_trigger_occurrences WHERE run_id IN (SELECT run_id FROM woml_retention_candidates)",
    "DELETE FROM woml_run_summaries WHERE run_id IN (SELECT run_id FROM woml_retention_candidates)",
    "DELETE FROM woml_run_events WHERE run_id IN (SELECT run_id FROM woml_retention_candidates)",
  ] {
    transaction.execute(statement, []).map_err(map_sqlite)?;
  }
  let deleted = transaction
    .execute(
      "DELETE FROM woml_runs WHERE run_id IN (SELECT run_id FROM woml_retention_candidates)",
      [],
    )
    .map_err(map_sqlite)?;
  transaction
    .execute_batch(RESTORE_RETENTION_GUARDS)
    .map_err(map_sqlite)?;
  Ok((deleted as u64, bytes))
}

pub fn execute_retention(
  path: impl AsRef<Path>,
  policy: &RetentionPolicyV1,
  lease_id: &str,
  owner_id: &str,
  compact: bool,
  now: DateTime<Utc>,
) -> Result<RetentionExecutionV1, RetentionError> {
  if !valid_policy(policy, now)
    || lease_id.is_empty()
    || lease_id.len() > 320
    || owner_id.is_empty()
    || owner_id.len() > 320
  {
    return Err(RetentionError::InvalidPolicy);
  }
  let mut connection = open(path.as_ref())?;
  quick_integrity(&connection)?;
  acquire_lease(&mut connection, lease_id, owner_id, "retention", now)?;
  let operation = (|| {
    let mut result = RetentionResultV1::new(policy.policy_id.clone(), now);
    let mut batches = 0_u64;
    loop {
      renew_lease(&connection, lease_id, Utc::now())?;
      let batch = candidate_batch_ids(&connection, policy, now)?;
      if batch.is_empty() {
        break;
      }
      let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(map_sqlite)?;
      let (deleted, bytes) = delete_batch(&transaction, &batch, policy, now)?;
      if deleted == 0 {
        transaction.rollback().map_err(map_sqlite)?;
        break;
      }
      result.deleted_runs += deleted;
      result.deleted_bytes += bytes;
      result.completed_at = Utc::now();
      let encoded = serde_json::to_string(&result).map_err(|_| RetentionError::IntegrityFailed)?;
      transaction
        .execute(
          "INSERT INTO woml_store_metadata(key, value)
           VALUES ('last_retention_result_v1', ?1)
           ON CONFLICT(key) DO UPDATE SET value = excluded.value",
          [encoded],
        )
        .map_err(map_sqlite)?;
      transaction.commit().map_err(map_sqlite)?;
      batches += 1;
      thread::yield_now();
    }
    result.completed_at = Utc::now();
    let encoded = serde_json::to_string(&result).map_err(|_| RetentionError::IntegrityFailed)?;
    connection
      .execute(
        "INSERT INTO woml_store_metadata(key, value)
         VALUES ('last_retention_result_v1', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [encoded],
      )
      .map_err(map_sqlite)?;
    let (checkpoint_busy, checkpoint_log_frames, checkpointed_frames): (i64, i64, i64) = connection
      .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
      })
      .map_err(map_sqlite)?;
    if compact {
      renew_lease(&connection, lease_id, Utc::now())?;
      connection.execute_batch("VACUUM").map_err(map_sqlite)?;
    }
    quick_integrity(&connection)?;
    Ok(RetentionExecutionV1 {
      result,
      batches,
      checkpoint_busy: checkpoint_busy.max(0) as u64,
      checkpoint_log_frames: checkpoint_log_frames.max(0) as u64,
      checkpointed_frames: checkpointed_frames.max(0) as u64,
      compacted: compact,
    })
  })();
  release_lease(&connection, lease_id);
  operation
}

pub fn last_retention_result(
  path: impl AsRef<Path>,
) -> Result<Option<RetentionResultV1>, RetentionError> {
  let connection = open(path.as_ref())?;
  let encoded = connection
    .query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'last_retention_result_v1'",
      [],
      |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(map_sqlite)?;
  encoded
    .map(|value| serde_json::from_str(&value).map_err(|_| RetentionError::IntegrityFailed))
    .transpose()
}
