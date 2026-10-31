//! Private remote dispatch schema.

use crate::DurableStoreError;
use rusqlite::Connection;

pub(crate) const CREATE_REMOTE_DISPATCH_SCHEMA_V20: &str = r#"
CREATE TABLE IF NOT EXISTS woml_remote_dispatches (
  dispatch_id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL,
  node_id TEXT NOT NULL,
  iteration_identity TEXT NOT NULL DEFAULT '',
  attempt INTEGER NOT NULL CHECK (attempt BETWEEN 1 AND 10),
  invocation_id TEXT NOT NULL,
  target TEXT NOT NULL,
  target_digest TEXT NOT NULL,
  package_digest TEXT NOT NULL,
  state TEXT NOT NULL CHECK (state IN (
    'requested','offered','accepted','started','cancel_requested',
    'succeeded','failed','cancelled','ambiguous','abandoned'
  )),
  worker_id TEXT,
  session_id TEXT,
  offer_id TEXT,
  offer_expires_at TEXT,
  accepted_at TEXT,
  started_at TEXT,
  last_heartbeat_at TEXT,
  last_heartbeat_sequence INTEGER,
  last_heartbeat_digest TEXT,
  terminal_digest TEXT,
  ambiguity_reason TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE (run_id, node_id, iteration_identity, attempt),
  UNIQUE (invocation_id),
  FOREIGN KEY (run_id) REFERENCES woml_runs(run_id),
  CHECK (
    (state = 'requested' AND worker_id IS NULL AND session_id IS NULL AND offer_id IS NULL)
    OR
    (state != 'requested' AND worker_id IS NOT NULL AND session_id IS NOT NULL AND offer_id IS NOT NULL)
  ),
  CHECK (
    (last_heartbeat_at IS NULL AND last_heartbeat_sequence IS NULL AND last_heartbeat_digest IS NULL)
    OR
    (last_heartbeat_at IS NOT NULL AND last_heartbeat_sequence IS NOT NULL AND last_heartbeat_digest IS NOT NULL)
  ),
  CHECK (
    (state IN ('succeeded','failed','cancelled') AND terminal_digest IS NOT NULL)
    OR
    (state NOT IN ('succeeded','failed','cancelled') AND terminal_digest IS NULL)
  )
);

CREATE TABLE IF NOT EXISTS woml_remote_dispatch_audit (
  audit_id TEXT PRIMARY KEY,
  dispatch_id TEXT NOT NULL,
  action TEXT NOT NULL CHECK (action IN (
    'requested','offered','accepted','started','heartbeat','cancel_requested',
    'succeeded','failed','cancelled','offer_expired','lost',
    'authorized_possible_duplicate_retry','reconciled_failed'
  )),
  actor TEXT NOT NULL,
  request_digest TEXT NOT NULL,
  details_json TEXT NOT NULL,
  recorded_at TEXT NOT NULL,
  FOREIGN KEY (dispatch_id) REFERENCES woml_remote_dispatches(dispatch_id)
);

CREATE INDEX IF NOT EXISTS woml_remote_dispatches_recovery
  ON woml_remote_dispatches(state, updated_at, dispatch_id);
CREATE INDEX IF NOT EXISTS woml_remote_dispatches_offer_expiry
  ON woml_remote_dispatches(state, offer_expires_at, dispatch_id);
CREATE INDEX IF NOT EXISTS woml_remote_dispatch_audit_dispatch
  ON woml_remote_dispatch_audit(dispatch_id, recorded_at, audit_id);
"#;

pub(crate) fn validate_remote_dispatch_schema(
  connection: &Connection,
) -> Result<(), DurableStoreError> {
  for (name, expected_columns) in [
    (
      "woml_remote_dispatches",
      &[
        "dispatch_id",
        "run_id",
        "node_id",
        "iteration_identity",
        "attempt",
        "invocation_id",
        "target",
        "target_digest",
        "package_digest",
        "state",
        "worker_id",
        "session_id",
        "offer_id",
        "offer_expires_at",
        "accepted_at",
        "started_at",
        "last_heartbeat_at",
        "last_heartbeat_sequence",
        "last_heartbeat_digest",
        "terminal_digest",
        "ambiguity_reason",
        "created_at",
        "updated_at",
      ][..],
    ),
    (
      "woml_remote_dispatch_audit",
      &[
        "audit_id",
        "dispatch_id",
        "action",
        "actor",
        "request_digest",
        "details_json",
        "recorded_at",
      ][..],
    ),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
      [name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v20 is missing required table {name:?}."
      )));
    }
    let mut statement =
      connection.prepare("SELECT name FROM pragma_table_info(?1) ORDER BY cid")?;
    let columns = statement
      .query_map([name], |row| row.get::<_, String>(0))?
      .collect::<Result<Vec<_>, _>>()?;
    if columns != expected_columns {
      return Err(DurableStoreError::Contract(format!(
        "Store v20 {name} columns do not match the frozen schema."
      )));
    }
  }
  for name in [
    "woml_remote_dispatches_recovery",
    "woml_remote_dispatches_offer_expiry",
    "woml_remote_dispatch_audit_dispatch",
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name=?1)",
      [name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v20 is missing required index {name:?}."
      )));
    }
  }
  Ok(())
}
