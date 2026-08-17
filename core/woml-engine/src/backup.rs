use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use rusqlite::backup::Backup;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{DurableEventStore, DurableStateStore, DURABLE_STORE_SCHEMA_VERSION};

const MAINTENANCE_LEASE_SECONDS: i64 = 300;
const SUPPORTED_BACKUP_STORE_VERSIONS: [u32; 2] = [13, 14];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupStoreInspection {
  pub store_version: u32,
  pub definition_hashes: Vec<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub deployment_id: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub activation_id: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub runtime_instance_id: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub runtime_lease_expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Error)]
pub enum BackupError {
  #[error("the WOML store path is missing or unsafe")]
  StorePathUnsafe,
  #[error("the backup destination already exists or is unsafe")]
  DestinationUnsafe,
  #[error("the WOML store version {0} is not supported for backup or restore")]
  UnsupportedStoreVersion(u32),
  #[error("the WOML store contains no compiled workflow definitions")]
  EmptyDefinitionInventory,
  #[error("another WOML maintenance operation is active")]
  MaintenanceBusy,
  #[error("the restored definition inventory does not match its manifest")]
  DefinitionInventoryMismatch,
  #[error("the WOML backup failed its SQLite or durable-state integrity audit")]
  IntegrityFailed,
  #[error(transparent)]
  Sqlite(#[from] rusqlite::Error),
  #[error("the WOML durable store could not be validated: {0}")]
  Durable(String),
  #[error("the WOML durable state could not be validated: {0}")]
  State(String),
  #[error(transparent)]
  Io(#[from] std::io::Error),
}

fn safe_existing_file(path: &Path) -> Result<PathBuf, BackupError> {
  let metadata = fs::symlink_metadata(path).map_err(|_| BackupError::StorePathUnsafe)?;
  if !metadata.is_file() || metadata.file_type().is_symlink() {
    return Err(BackupError::StorePathUnsafe);
  }
  fs::canonicalize(path).map_err(BackupError::Io)
}

fn open_read_only(path: &Path) -> Result<Connection, BackupError> {
  Ok(Connection::open_with_flags(
    safe_existing_file(path)?,
    OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
  )?)
}

fn store_version(connection: &Connection) -> Result<u32, BackupError> {
  let encoded: String = connection
    .query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )
    .map_err(|_| BackupError::IntegrityFailed)?;
  let version = encoded
    .parse::<u32>()
    .map_err(|_| BackupError::IntegrityFailed)?;
  if !SUPPORTED_BACKUP_STORE_VERSIONS.contains(&version) {
    return Err(BackupError::UnsupportedStoreVersion(version));
  }
  Ok(version)
}

fn definition_hashes(connection: &Connection) -> Result<Vec<String>, BackupError> {
  let mut statement =
    connection.prepare("SELECT definition_hash FROM woml_definitions ORDER BY definition_hash")?;
  let hashes = statement
    .query_map([], |row| row.get::<_, String>(0))?
    .collect::<Result<Vec<_>, _>>()?;
  if hashes.is_empty() {
    return Err(BackupError::EmptyDefinitionInventory);
  }
  if hashes.iter().any(|hash| {
    hash.len() != 71
      || !hash.starts_with("sha256:")
      || !hash[7..].bytes().all(|byte| byte.is_ascii_hexdigit())
  }) {
    return Err(BackupError::IntegrityFailed);
  }
  Ok(hashes)
}

fn quick_integrity(connection: &Connection) -> Result<(), BackupError> {
  let result: String = connection.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
  let foreign_key_failure: bool = connection.query_row(
    "SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_check)",
    [],
    |row| row.get(0),
  )?;
  if result != "ok" || foreign_key_failure {
    return Err(BackupError::IntegrityFailed);
  }
  Ok(())
}

fn audit_definition_inventory(
  store: &DurableEventStore,
  definition_hashes: &[String],
) -> Result<(), BackupError> {
  for definition_hash in definition_hashes {
    store
      .definition(definition_hash)
      .map_err(|error| BackupError::Durable(error.to_string()))?;
    store
      .definition_module_artifacts(definition_hash)
      .map_err(|error| BackupError::Durable(error.to_string()))?;
  }
  Ok(())
}

pub fn inspect_backup_store(path: impl AsRef<Path>) -> Result<BackupStoreInspection, BackupError> {
  let connection = open_read_only(path.as_ref())?;
  quick_integrity(&connection)?;
  let version = store_version(&connection)?;
  let hashes = definition_hashes(&connection)?;
  let owner: Option<(String, String, String, String)> = if version >= 14 {
    connection
      .query_row(
        "SELECT deployment_id, activation_id, runtime_instance_id, lease_expires_at
         FROM woml_runtime_owner WHERE singleton = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
      )
      .optional()?
  } else {
    None
  };
  let (deployment_id, activation_id, runtime_instance_id, runtime_lease_expires_at) =
    if let Some((deployment, activation, runtime, expires)) = owner {
      let expires = DateTime::parse_from_rfc3339(&expires)
        .map_err(|_| BackupError::IntegrityFailed)?
        .with_timezone(&Utc);
      (
        Some(deployment),
        Some(activation),
        Some(runtime),
        Some(expires),
      )
    } else {
      (None, None, None, None)
    };
  Ok(BackupStoreInspection {
    store_version: version,
    definition_hashes: hashes,
    deployment_id,
    activation_id,
    runtime_instance_id,
    runtime_lease_expires_at,
  })
}

fn acquire_maintenance_lease(
  connection: &mut Connection,
  lease_id: &str,
  owner_id: &str,
  operation: &str,
  now: DateTime<Utc>,
) -> Result<(), BackupError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute(
    "DELETE FROM woml_maintenance_lease WHERE singleton = 1 AND expires_at <= ?1",
    [now.to_rfc3339()],
  )?;
  let inserted = transaction.execute(
    "INSERT OR IGNORE INTO woml_maintenance_lease(
       singleton, lease_id, operation, owner_id, expires_at
     ) VALUES (1, ?1, ?2, ?3, ?4)",
    params![
      lease_id,
      operation,
      owner_id,
      (now + chrono::Duration::seconds(MAINTENANCE_LEASE_SECONDS)).to_rfc3339(),
    ],
  )?;
  if inserted != 1 {
    return Err(BackupError::MaintenanceBusy);
  }
  transaction.commit()?;
  Ok(())
}

fn release_maintenance_lease(connection: &Connection, lease_id: &str) {
  let _ = connection.execute(
    "DELETE FROM woml_maintenance_lease WHERE singleton = 1 AND lease_id = ?1",
    [lease_id],
  );
}

fn content_activation_id(definition_hashes: &[String]) -> String {
  let mut hasher = Sha256::new();
  hasher.update(b"woml.backup-activation\0v1\0");
  for hash in definition_hashes {
    hasher.update((hash.len() as u64).to_be_bytes());
    hasher.update(hash.as_bytes());
  }
  format!("sha256:{:x}", hasher.finalize())
}

pub fn create_online_backup(
  source_path: impl AsRef<Path>,
  destination_path: impl AsRef<Path>,
  lease_id: &str,
  owner_id: &str,
  fallback_deployment_id: &str,
) -> Result<BackupStoreInspection, BackupError> {
  if lease_id.is_empty()
    || owner_id.is_empty()
    || fallback_deployment_id.is_empty()
    || lease_id.len() > 320
    || owner_id.len() > 320
    || fallback_deployment_id.len() > 320
  {
    return Err(BackupError::DestinationUnsafe);
  }
  let source_path = safe_existing_file(source_path.as_ref())?;
  let destination_path = destination_path.as_ref();
  if destination_path.exists()
    || fs::symlink_metadata(destination_path)
      .is_ok_and(|metadata| metadata.file_type().is_symlink())
  {
    return Err(BackupError::DestinationUnsafe);
  }

  let store = DurableEventStore::open(&source_path)
    .map_err(|error| BackupError::Durable(error.to_string()))?;
  store
    .audit_integrity()
    .map_err(|error| BackupError::Durable(error.to_string()))?;
  let source_inventory = inspect_backup_store(&source_path)?;
  audit_definition_inventory(&store, &source_inventory.definition_hashes)?;
  drop(store);
  // Persist State v1's original path-derived identity before taking the
  // snapshot. The copied identity lets a verified restore move to another
  // filesystem path without changing workflow state scope.
  DurableStateStore::open(&source_path)
    .map(drop)
    .map_err(|error| BackupError::State(error.to_string()))?;

  let mut source = Connection::open(&source_path)?;
  source.busy_timeout(Duration::from_secs(5))?;
  source.execute_batch("PRAGMA foreign_keys = ON;")?;
  acquire_maintenance_lease(&mut source, lease_id, owner_id, "backup", Utc::now())?;
  let result = (|| {
    let mut inspection = inspect_backup_store(&source_path)?;
    let mut destination = Connection::open(destination_path)?;
    {
      let backup = Backup::new(&source, &mut destination)?;
      backup.run_to_completion(128, Duration::from_millis(5), None)?;
    }
    drop(destination);

    let destination_store = DurableEventStore::open(destination_path)
      .map_err(|error| BackupError::Durable(error.to_string()))?;
    destination_store
      .audit_integrity()
      .map_err(|error| BackupError::Durable(error.to_string()))?;
    audit_definition_inventory(&destination_store, &inspection.definition_hashes)?;
    drop(destination_store);
    DurableStateStore::open(destination_path)
      .map(drop)
      .map_err(|error| BackupError::State(error.to_string()))?;
    let copied = inspect_backup_store(destination_path)?;
    if copied.definition_hashes != inspection.definition_hashes {
      return Err(BackupError::DefinitionInventoryMismatch);
    }
    inspection.deployment_id = inspection
      .deployment_id
      .or_else(|| Some(fallback_deployment_id.to_string()));
    inspection.activation_id = inspection
      .activation_id
      .or_else(|| Some(content_activation_id(&inspection.definition_hashes)));
    Ok(inspection)
  })();
  release_maintenance_lease(&source, lease_id);
  if result.is_err() {
    let _ = fs::remove_file(destination_path);
  }
  result
}

pub fn record_verified_backup(
  source_path: impl AsRef<Path>,
  backup_id: &str,
  completed_at: DateTime<Utc>,
) -> Result<(), BackupError> {
  if backup_id.is_empty() || backup_id.len() > 320 {
    return Err(BackupError::IntegrityFailed);
  }
  let path = safe_existing_file(source_path.as_ref())?;
  let connection = Connection::open(path)?;
  connection.execute(
    "INSERT INTO woml_last_verified_backup(singleton, backup_id, completed_at, verified)
     VALUES (1, ?1, ?2, 1)
     ON CONFLICT(singleton) DO UPDATE SET
       backup_id = excluded.backup_id,
       completed_at = excluded.completed_at,
       verified = 1",
    params![backup_id, completed_at.to_rfc3339()],
  )?;
  Ok(())
}

pub fn prepare_restored_store(
  path: impl AsRef<Path>,
  expected_definition_hashes: &[String],
  backup_id: &str,
  restored_at: DateTime<Utc>,
) -> Result<BackupStoreInspection, BackupError> {
  let path = safe_existing_file(path.as_ref())?;
  let before = inspect_backup_store(&path)?;
  if before.definition_hashes != expected_definition_hashes {
    return Err(BackupError::DefinitionInventoryMismatch);
  }
  let store =
    DurableEventStore::open(&path).map_err(|error| BackupError::Durable(error.to_string()))?;
  store
    .audit_integrity()
    .map_err(|error| BackupError::Durable(error.to_string()))?;
  audit_definition_inventory(&store, expected_definition_hashes)?;
  drop(store);
  let mut connection = Connection::open(&path)?;
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute("DELETE FROM woml_runtime_owner", [])?;
  transaction.execute("DELETE FROM woml_maintenance_lease", [])?;
  transaction.execute("DELETE FROM woml_scheduler_claims", [])?;
  transaction.execute("DELETE FROM woml_workflow_runtime_routes", [])?;
  transaction.execute(
    "INSERT INTO woml_last_verified_backup(singleton, backup_id, completed_at, verified)
     VALUES (1, ?1, ?2, 1)
     ON CONFLICT(singleton) DO UPDATE SET
       backup_id = excluded.backup_id,
       completed_at = excluded.completed_at,
       verified = 1",
    params![backup_id, restored_at.to_rfc3339()],
  )?;
  transaction.commit()?;
  connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
  drop(connection);
  DurableStateStore::open(&path)
    .map(drop)
    .map_err(|error| BackupError::State(error.to_string()))?;
  let after = inspect_backup_store(&path)?;
  if after.store_version != DURABLE_STORE_SCHEMA_VERSION
    || after.definition_hashes != expected_definition_hashes
  {
    return Err(BackupError::IntegrityFailed);
  }
  let store =
    DurableEventStore::open(&path).map_err(|error| BackupError::Durable(error.to_string()))?;
  audit_definition_inventory(&store, expected_definition_hashes)?;
  Ok(after)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn content_activation_is_stable() {
    let hashes = vec![
      format!("sha256:{}", "a".repeat(64)),
      format!("sha256:{}", "b".repeat(64)),
    ];
    assert_eq!(
      content_activation_id(&hashes),
      content_activation_id(&hashes)
    );
    assert_ne!(
      content_activation_id(&hashes),
      content_activation_id(&[hashes[0].clone()])
    );
  }
}
