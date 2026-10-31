//! Atomic first creation, distinct from upgrades of an existing durable store.
//!
//! An empty database has no history or indexes to migrate/rebuild. Reuse the
//! frozen historical DDL in its original order, but publish the complete current
//! layout and marker with one durable commit instead of nineteen migrations and
//! autocommitted base-schema statements. Existing stores still use migrations.rs.

use super::schema::{
  CREATE_APPROVAL_SCHEMA_V2, CREATE_CORRECTION_SCHEMA_V17, CREATE_DURABLE_STATE_SCHEMA_V13,
  CREATE_DURABLE_WAIT_SCHEMA_V15, CREATE_INFRASTRUCTURE_POLICY_SCHEMA_V19,
  CREATE_INTERNAL_EVENT_SCHEMA_V7, CREATE_INTERVAL_SCHEMA_V6, CREATE_LOCK_SCHEMA_V16,
  CREATE_MODULE_ARTIFACT_SCHEMA_V8, CREATE_NOTIFICATION_SCHEMA_V3,
  CREATE_PRODUCTION_RUNTIME_SCHEMA_V14, CREATE_RICH_APPROVAL_SCHEMA_V18,
  CREATE_RUN_SUMMARY_SCHEMA_V11, CREATE_SCHEDULE_SCHEMA_V5, CREATE_SCHEMA,
  CREATE_TRIGGER_SCHEMA_V4, CREATE_WORKFLOW_CALL_SCHEMA_V9, CREATE_WORKFLOW_RUNTIME_SCHEMA_V10,
  MIGRATE_RUNTIME_POLICY_SCHEMA_V12, STORE_SCHEMA_VERSION_V20,
};
use super::{validate_store_v19_schema, DurableStoreError};
use crate::remote_dispatch::{validate_remote_dispatch_schema, CREATE_REMOTE_DISPATCH_SCHEMA_V20};
use rusqlite::{Connection, TransactionBehavior};

fn is_empty(connection: &Connection) -> rusqlite::Result<bool> {
  connection.query_row(
    "SELECT NOT EXISTS(SELECT 1 FROM sqlite_master)",
    [],
    |row| row.get(0),
  )
}

pub(super) fn initialize_empty_store(
  connection: &mut Connection,
) -> Result<bool, DurableStoreError> {
  // Do not acquire a writer lock on every ordinary open. Absence of a version
  // marker alone is insufficient: a partial or foreign schema is not empty.
  if !is_empty(connection)? {
    return Ok(false);
  }
  let mut span = crate::performance::PerformanceSpan::new("sqlite", "sqlite.bootstrap_empty");
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  // Another opener may have initialized it while we waited for the writer lock.
  if !is_empty(&transaction)? {
    return Ok(false);
  }
  for ddl in [
    CREATE_SCHEMA,
    CREATE_APPROVAL_SCHEMA_V2,
    CREATE_NOTIFICATION_SCHEMA_V3,
    CREATE_TRIGGER_SCHEMA_V4,
    CREATE_SCHEDULE_SCHEMA_V5,
    CREATE_INTERVAL_SCHEMA_V6,
    CREATE_INTERNAL_EVENT_SCHEMA_V7,
    CREATE_MODULE_ARTIFACT_SCHEMA_V8,
    CREATE_WORKFLOW_CALL_SCHEMA_V9,
    CREATE_WORKFLOW_RUNTIME_SCHEMA_V10,
    CREATE_RUN_SUMMARY_SCHEMA_V11,
    MIGRATE_RUNTIME_POLICY_SCHEMA_V12,
    CREATE_DURABLE_STATE_SCHEMA_V13,
    CREATE_PRODUCTION_RUNTIME_SCHEMA_V14,
    CREATE_DURABLE_WAIT_SCHEMA_V15,
    CREATE_LOCK_SCHEMA_V16,
    CREATE_CORRECTION_SCHEMA_V17,
    CREATE_RICH_APPROVAL_SCHEMA_V18,
    CREATE_INFRASTRUCTURE_POLICY_SCHEMA_V19,
    CREATE_REMOTE_DISPATCH_SCHEMA_V20,
  ] {
    transaction.execute_batch(ddl)?;
  }
  validate_store_v19_schema(&transaction)?;
  validate_remote_dispatch_schema(&transaction)?;
  transaction.execute(
    "INSERT INTO woml_store_metadata(key, value) VALUES ('schema_version', ?1)",
    [STORE_SCHEMA_VERSION_V20],
  )?;
  transaction.commit()?;
  span.succeed();
  Ok(true)
}
