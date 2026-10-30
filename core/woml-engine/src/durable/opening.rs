//! Startup initialization and the deliberately narrower prepared-store opener.
//!
//! Only startup validates/migrates layouts and rebuilds indexes. Request callers
//! establish readiness first; open_ready must not re-enter DDL or migration work.

use crate::durable::{
  migrate_store_v10_to_v11, migrate_store_v11_to_v12, migrate_store_v12_to_v13,
  migrate_store_v13_to_v14, migrate_store_v14_to_v15, migrate_store_v15_to_v16,
  migrate_store_v16_to_v17, migrate_store_v17_to_v18, migrate_store_v18_to_v19,
  migrate_store_v19_to_v20, migrate_store_v1_to_v2, migrate_store_v2_to_v3, migrate_store_v3_to_v4,
  migrate_store_v4_to_v5, migrate_store_v5_to_v6, migrate_store_v6_to_v7, migrate_store_v7_to_v8,
  migrate_store_v8_to_v9, migrate_store_v9_to_v10, rebuild_correction_indexes,
  rebuild_run_summary_index, rebuild_runtime_policy_indexes, runtime_policy_indexes_need_rebuild,
  validate_store_v10_schema, validate_store_v11_schema, validate_store_v12_schema,
  validate_store_v13_schema, validate_store_v14_schema, validate_store_v15_schema,
  validate_store_v16_schema, validate_store_v17_schema, validate_store_v18_schema,
  validate_store_v19_schema, validate_store_v2_schema, validate_store_v3_schema,
  validate_store_v4_schema, validate_store_v5_schema, validate_store_v6_schema,
  validate_store_v7_schema, validate_store_v8_schema, validate_store_v9_schema, DurableEventStore,
  DurableStoreError, CREATE_SCHEMA, STORE_SCHEMA_VERSION_V1, STORE_SCHEMA_VERSION_V10,
  STORE_SCHEMA_VERSION_V11, STORE_SCHEMA_VERSION_V12, STORE_SCHEMA_VERSION_V13,
  STORE_SCHEMA_VERSION_V14, STORE_SCHEMA_VERSION_V15, STORE_SCHEMA_VERSION_V16,
  STORE_SCHEMA_VERSION_V17, STORE_SCHEMA_VERSION_V18, STORE_SCHEMA_VERSION_V19,
  STORE_SCHEMA_VERSION_V2, STORE_SCHEMA_VERSION_V20, STORE_SCHEMA_VERSION_V3,
  STORE_SCHEMA_VERSION_V4, STORE_SCHEMA_VERSION_V5, STORE_SCHEMA_VERSION_V6,
  STORE_SCHEMA_VERSION_V7, STORE_SCHEMA_VERSION_V8, STORE_SCHEMA_VERSION_V9,
};
use crate::remote_dispatch::validate_remote_dispatch_schema;
use chrono::Utc;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::TransactionBehavior;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

impl DurableEventStore {
  pub fn open(path: impl AsRef<Path>) -> Result<Self, DurableStoreError> {
    let mut span = crate::performance::PerformanceSpan::new("sqlite", "sqlite.open_initialize");
    let path = path.as_ref();
    let result = (|| {
      let connection = Connection::open(path)?;
      Self::initialize(connection)
    })();
    if result.is_ok() {
      if let Ok(metadata) = std::fs::metadata(path) {
        span.bytes(
          "database",
          usize::try_from(metadata.len()).unwrap_or(usize::MAX),
        );
      }
      span.succeed();
    }
    result
  }

  /// Opens a store that the runtime has already initialized and validated.
  ///
  /// Request admission must not repeat schema DDL or migration checks: doing
  /// so can turn a short, recoverable SQLite writer conflict into a lock-upgrade
  /// deadlock. Runtime startup remains the sole schema/migration authority.
  pub(crate) fn open_ready(path: impl AsRef<Path>) -> Result<Self, DurableStoreError> {
    let mut span = crate::performance::PerformanceSpan::new("sqlite", "sqlite.open_ready");
    let path = path.as_ref();
    let connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    let version: String = connection.query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )?;
    if version != STORE_SCHEMA_VERSION_V20 {
      return Err(DurableStoreError::UnsupportedStoreVersion(version));
    }
    if let Ok(metadata) = std::fs::metadata(path) {
      span.bytes(
        "database",
        usize::try_from(metadata.len()).unwrap_or(usize::MAX),
      );
    }
    span.succeed();
    Ok(Self {
      connection,
      projection_cache: RefCell::new(HashMap::new()),
    })
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
      Some(STORE_SCHEMA_VERSION_V20) => {
        validate_store_v19_schema(&connection)?;
        validate_remote_dispatch_schema(&connection)?;
      }
      Some(STORE_SCHEMA_VERSION_V19) => validate_store_v19_schema(&connection)?,
      Some(STORE_SCHEMA_VERSION_V18) => validate_store_v18_schema(&connection)?,
      Some(STORE_SCHEMA_VERSION_V17) => validate_store_v17_schema(&connection)?,
      Some(STORE_SCHEMA_VERSION_V16) => validate_store_v16_schema(&connection)?,
      Some(STORE_SCHEMA_VERSION_V15) => validate_store_v15_schema(&connection)?,
      Some(STORE_SCHEMA_VERSION_V14) => validate_store_v14_schema(&connection)?,
      Some(STORE_SCHEMA_VERSION_V13) => validate_store_v13_schema(&connection)?,
      Some(STORE_SCHEMA_VERSION_V12) => validate_store_v12_schema(&connection)?,
      Some(STORE_SCHEMA_VERSION_V11) => validate_store_v11_schema(&connection)?,
      Some(STORE_SCHEMA_VERSION_V10) => validate_store_v10_schema(&connection)?,
      Some(STORE_SCHEMA_VERSION_V9) => validate_store_v9_schema(&connection)?,
      Some(STORE_SCHEMA_VERSION_V8) => {
        validate_store_v8_schema(&connection)?;
        migrate_store_v8_to_v9(&mut connection)?;
      }
      Some(STORE_SCHEMA_VERSION_V7) => {
        validate_store_v7_schema(&connection)?;
        migrate_store_v7_to_v8(&mut connection)?;
        migrate_store_v8_to_v9(&mut connection)?;
      }
      Some(STORE_SCHEMA_VERSION_V6) => {
        validate_store_v6_schema(&connection)?;
        migrate_store_v6_to_v7(&mut connection)?;
        migrate_store_v7_to_v8(&mut connection)?;
        migrate_store_v8_to_v9(&mut connection)?;
      }
      Some(STORE_SCHEMA_VERSION_V5) => {
        validate_store_v5_schema(&connection)?;
        migrate_store_v5_to_v6(&mut connection)?;
        migrate_store_v6_to_v7(&mut connection)?;
        migrate_store_v7_to_v8(&mut connection)?;
        migrate_store_v8_to_v9(&mut connection)?;
      }
      Some(STORE_SCHEMA_VERSION_V4) => {
        validate_store_v4_schema(&connection)?;
        migrate_store_v4_to_v5(&mut connection)?;
        migrate_store_v5_to_v6(&mut connection)?;
        migrate_store_v6_to_v7(&mut connection)?;
        migrate_store_v7_to_v8(&mut connection)?;
        migrate_store_v8_to_v9(&mut connection)?;
      }
      Some(STORE_SCHEMA_VERSION_V3) => {
        validate_store_v3_schema(&connection)?;
        migrate_store_v3_to_v4(&mut connection)?;
        migrate_store_v4_to_v5(&mut connection)?;
        migrate_store_v5_to_v6(&mut connection)?;
        migrate_store_v6_to_v7(&mut connection)?;
        migrate_store_v7_to_v8(&mut connection)?;
        migrate_store_v8_to_v9(&mut connection)?;
      }
      Some(STORE_SCHEMA_VERSION_V2) => {
        validate_store_v2_schema(&connection)?;
        migrate_store_v2_to_v3(&mut connection)?;
        migrate_store_v3_to_v4(&mut connection)?;
        migrate_store_v4_to_v5(&mut connection)?;
        migrate_store_v5_to_v6(&mut connection)?;
        migrate_store_v6_to_v7(&mut connection)?;
        migrate_store_v7_to_v8(&mut connection)?;
        migrate_store_v8_to_v9(&mut connection)?;
      }
      Some(STORE_SCHEMA_VERSION_V1) => {
        migrate_store_v1_to_v2(&mut connection)?;
        validate_store_v2_schema(&connection)?;
        migrate_store_v2_to_v3(&mut connection)?;
        migrate_store_v3_to_v4(&mut connection)?;
        migrate_store_v4_to_v5(&mut connection)?;
        migrate_store_v5_to_v6(&mut connection)?;
        migrate_store_v6_to_v7(&mut connection)?;
        migrate_store_v7_to_v8(&mut connection)?;
        migrate_store_v8_to_v9(&mut connection)?;
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
        migrate_store_v6_to_v7(&mut connection)?;
        migrate_store_v7_to_v8(&mut connection)?;
        migrate_store_v8_to_v9(&mut connection)?;
      }
    }
    let current_version: String = connection.query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )?;
    if current_version == STORE_SCHEMA_VERSION_V9 {
      migrate_store_v9_to_v10(&mut connection)?;
    }
    let current_version: String = connection.query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )?;
    if current_version == STORE_SCHEMA_VERSION_V10 {
      migrate_store_v10_to_v11(&mut connection)?;
    }
    let current_version: String = connection.query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )?;
    if current_version == STORE_SCHEMA_VERSION_V11 {
      migrate_store_v11_to_v12(&mut connection)?;
    }
    let current_version: String = connection.query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )?;
    if current_version == STORE_SCHEMA_VERSION_V12 {
      migrate_store_v12_to_v13(&mut connection)?;
    }
    let current_version: String = connection.query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )?;
    if current_version == STORE_SCHEMA_VERSION_V13 {
      migrate_store_v13_to_v14(&mut connection)?;
    }
    let current_version: String = connection.query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )?;
    if current_version == STORE_SCHEMA_VERSION_V14 {
      migrate_store_v14_to_v15(&mut connection)?;
    }
    let current_version: String = connection.query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )?;
    if current_version == STORE_SCHEMA_VERSION_V15 {
      migrate_store_v15_to_v16(&mut connection)?;
    }
    let current_version: String = connection.query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )?;
    if current_version == STORE_SCHEMA_VERSION_V16 {
      migrate_store_v16_to_v17(&mut connection)?;
    }
    let current_version: String = connection.query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )?;
    if current_version == STORE_SCHEMA_VERSION_V17 {
      migrate_store_v17_to_v18(&mut connection)?;
    }
    let current_version: String = connection.query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )?;
    if current_version == STORE_SCHEMA_VERSION_V18 {
      migrate_store_v18_to_v19(&mut connection)?;
    }
    let current_version: String = connection.query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )?;
    if current_version == STORE_SCHEMA_VERSION_V19 {
      migrate_store_v19_to_v20(&mut connection)?;
    }
    validate_store_v19_schema(&connection)?;
    validate_remote_dispatch_schema(&connection)?;
    {
      let recovery = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
      rebuild_correction_indexes(&recovery)?;
      recovery.commit()?;
    }
    if runtime_policy_indexes_need_rebuild(&connection)? {
      let recovery = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
      rebuild_runtime_policy_indexes(&recovery)?;
      rebuild_run_summary_index(&recovery)?;
      recovery.commit()?;
    }
    connection.execute(
      "DELETE FROM woml_scheduler_claims WHERE expires_at <= ?1",
      [Utc::now().to_rfc3339()],
    )?;
    Ok(Self {
      connection,
      projection_cache: RefCell::new(HashMap::new()),
    })
  }
}
