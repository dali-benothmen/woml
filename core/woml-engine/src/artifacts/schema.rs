//! Original artifact metadata DDL, schema validation, and store identity. Extraction does not bump or rewrite the schema.

use crate::artifacts::artifact_failure;
use crate::artifacts::backend_unavailable;
use crate::CapabilityFailure;
use crate::CapabilityFailureKind;
use rusqlite::Connection;
use rusqlite::OptionalExtension;

const ARTIFACT_STORE_SCHEMA_VERSION: u32 = 1;

pub(in crate::artifacts) fn initialize_schema(
  connection: &Connection,
) -> Result<(), CapabilityFailure> {
  connection
    .execute_batch(
      "CREATE TABLE IF NOT EXISTS woml_artifact_store_metadata(
         key TEXT PRIMARY KEY NOT NULL,
         value TEXT NOT NULL
       );
       INSERT OR IGNORE INTO woml_artifact_store_metadata(key, value)
       VALUES ('schema_version', '1');
       CREATE TABLE IF NOT EXISTS woml_artifact_blobs(
         sha256 TEXT PRIMARY KEY NOT NULL,
         size INTEGER NOT NULL CHECK(size >= 0),
         created_at TEXT NOT NULL
       );
       CREATE TABLE IF NOT EXISTS woml_artifacts(
         artifact_id TEXT PRIMARY KEY NOT NULL,
         name TEXT NOT NULL,
         size INTEGER NOT NULL CHECK(size >= 0),
         sha256 TEXT NOT NULL REFERENCES woml_artifact_blobs(sha256),
         content_type TEXT NOT NULL,
         status TEXT NOT NULL CHECK(status IN ('available','quarantined','expired','corrupt','deleted')),
         created_at TEXT NOT NULL
       );
       CREATE INDEX IF NOT EXISTS woml_artifacts_sha256 ON woml_artifacts(sha256);
       CREATE TABLE IF NOT EXISTS woml_artifact_provenance(
         artifact_id TEXT PRIMARY KEY NOT NULL REFERENCES woml_artifacts(artifact_id),
         workflow_id TEXT NOT NULL,
         run_id TEXT NOT NULL,
         node_id TEXT NOT NULL,
         attempt_number INTEGER NOT NULL,
         operation_key TEXT NOT NULL UNIQUE
       );
       CREATE INDEX IF NOT EXISTS woml_artifact_provenance_run ON woml_artifact_provenance(run_id);
       CREATE TABLE IF NOT EXISTS woml_artifact_access(
         artifact_id TEXT NOT NULL REFERENCES woml_artifacts(artifact_id),
         run_id TEXT NOT NULL,
         granted_at TEXT NOT NULL,
         revoked_at TEXT,
         PRIMARY KEY(artifact_id, run_id)
       );
       CREATE TABLE IF NOT EXISTS woml_artifact_transfers(
         operation_key TEXT PRIMARY KEY NOT NULL,
         run_id TEXT NOT NULL,
         node_id TEXT NOT NULL,
         attempt_number INTEGER NOT NULL,
         operation TEXT NOT NULL CHECK(operation IN ('put','head','materialize')),
         input_digest TEXT NOT NULL,
         status TEXT NOT NULL CHECK(status IN ('started','succeeded','failed','cancelled','interrupted')),
         artifact_id TEXT REFERENCES woml_artifacts(artifact_id),
         content_digest TEXT,
         size INTEGER,
         result_digest TEXT,
         error_code TEXT,
         ambiguous INTEGER,
         started_at TEXT NOT NULL,
         settled_at TEXT
       );
       CREATE INDEX IF NOT EXISTS woml_artifact_transfers_run ON woml_artifact_transfers(run_id);
       CREATE TABLE IF NOT EXISTS woml_artifact_retention(
         artifact_id TEXT PRIMARY KEY NOT NULL REFERENCES woml_artifacts(artifact_id),
         requested_ms INTEGER NOT NULL,
         effective_ms INTEGER NOT NULL,
         expires_at TEXT NOT NULL
       );
       CREATE TABLE IF NOT EXISTS woml_artifact_operator_audit(
         audit_id TEXT PRIMARY KEY NOT NULL,
         operation TEXT NOT NULL CHECK(operation IN ('inspect','inspect_run','export')),
         subject_id TEXT NOT NULL,
         occurred_at TEXT NOT NULL
       );",
    )
    .map_err(|_| backend_unavailable())?;
  Ok(())
}

pub(in crate::artifacts) fn ensure_store_identity(
  connection: &Connection,
) -> Result<String, CapabilityFailure> {
  let existing: Option<String> = connection
    .query_row(
      "SELECT value FROM woml_artifact_store_metadata WHERE key = 'store_id'",
      [],
      |row| row.get(0),
    )
    .optional()
    .map_err(|_| backend_unavailable())?;
  if let Some(store_id) = existing {
    if store_id.starts_with("artifact-store-") && store_id.len() == 47 {
      return Ok(store_id);
    }
    return Err(backend_unavailable());
  }
  let store_id = format!("artifact-store-{}", uuid::Uuid::new_v4().simple());
  connection
    .execute(
      "INSERT INTO woml_artifact_store_metadata(key, value) VALUES ('store_id', ?1)",
      [&store_id],
    )
    .map_err(|_| backend_unavailable())?;
  Ok(store_id)
}

pub(in crate::artifacts) fn validate_schema(
  connection: &Connection,
) -> Result<(), CapabilityFailure> {
  let version: Option<String> = connection
    .query_row(
      "SELECT value FROM woml_artifact_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )
    .optional()
    .map_err(|_| backend_unavailable())?;
  if version.as_deref() != Some(&ARTIFACT_STORE_SCHEMA_VERSION.to_string()) {
    return Err(artifact_failure(
      CapabilityFailureKind::UnsupportedVersion,
      "WOML_ARTIFACT_BACKEND_UNAVAILABLE",
      "The artifact metadata store uses an unsupported schema version.",
      false,
      false,
    ));
  }
  Ok(())
}
