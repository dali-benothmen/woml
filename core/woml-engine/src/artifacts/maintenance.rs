//! Bounded retention/scrub/orphan collection protecting active runs and unreleased review pins. Logical removal precedes unreachable-byte deletion.

use crate::artifacts::artifact_failure;
use crate::artifacts::backend_unavailable;
use crate::artifacts::timestamp;
use crate::artifacts::validate_schema;
use crate::artifacts::ArtifactMaintenanceV1;
use crate::artifacts::ArtifactPolicyV1;
use crate::artifacts::ArtifactRuntimeMode;
use crate::artifacts::ArtifactStore;
use crate::artifacts::ManagedArtifactPutHandler;
use crate::artifacts::ManagedArtifactStore;
use crate::CapabilityFailure;
use crate::CapabilityFailureKind;
use chrono::SecondsFormat;
use chrono::Utc;
use rusqlite::params;
use rusqlite::Connection;
use rusqlite::TransactionBehavior;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

const ARTIFACT_MAINTENANCE_BATCH: usize = 250;

const ARTIFACT_SCRUB_BATCH: usize = 64;

pub(in crate::artifacts) const ARTIFACT_ORPHAN_SCAN_BATCH: usize = 1_000;

pub fn maintain_artifacts(
  state_path: impl AsRef<Path>,
  policy: ArtifactPolicyV1,
  mode: ArtifactRuntimeMode,
  resolved_secrets: &BTreeMap<String, String>,
  now: chrono::DateTime<Utc>,
  dry_run: bool,
) -> Result<ArtifactMaintenanceV1, CapabilityFailure> {
  let managed = Arc::new(ManagedArtifactStore::default());
  let handler =
    ManagedArtifactPutHandler::new(Arc::clone(&managed), policy, mode, resolved_secrets).map_err(
      |error| {
        artifact_failure(
          CapabilityFailureKind::InvalidInput,
          "WOML_ARTIFACT_POLICY_INVALID",
          &error.to_string(),
          false,
          false,
        )
      },
    )?;
  managed.configure_for_state(state_path.as_ref(), handler.policy())?;
  managed.configured()?.maintenance(now, dry_run)
}

fn artifact_retention_candidates(
  connection: &Connection,
  now: &str,
  limit: usize,
) -> Result<Vec<(String, String, u64)>, CapabilityFailure> {
  let mut statement = connection
    .prepare(
      "SELECT a.artifact_id, a.sha256, a.size
       FROM woml_artifacts AS a
       JOIN woml_artifact_retention AS retention
         ON retention.artifact_id = a.artifact_id
       WHERE retention.expires_at <= ?1
         AND a.status IN ('available','quarantined','expired','corrupt')
         AND NOT EXISTS(
           SELECT 1 FROM woml_approval_artifact_pins AS pins
           WHERE pins.artifact_id = a.artifact_id AND pins.released_at IS NULL
         )
         AND NOT EXISTS(
           SELECT 1
           FROM woml_artifact_access AS access
           JOIN woml_runs AS runs ON runs.run_id = access.run_id
           LEFT JOIN woml_run_summaries AS summaries ON summaries.run_id = runs.run_id
           WHERE access.artifact_id = a.artifact_id
             AND access.revoked_at IS NULL
             AND (
               summaries.run_id IS NULL
               OR summaries.status NOT IN ('succeeded','failed','cancelled')
             )
         )
       ORDER BY retention.expires_at, a.artifact_id
       LIMIT ?2",
    )
    .map_err(|_| backend_unavailable())?;
  let candidates = statement
    .query_map(params![now, limit as u64], |row| {
      Ok((
        row.get::<_, String>(0)?,
        row.get::<_, String>(1)?,
        row.get::<_, u64>(2)?,
      ))
    })
    .map_err(|_| backend_unavailable())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|_| backend_unavailable())?;
  Ok(candidates)
}

impl ArtifactStore {
  pub(in crate::artifacts) fn maintenance(
    &self,
    now: chrono::DateTime<Utc>,
    dry_run: bool,
  ) -> Result<ArtifactMaintenanceV1, CapabilityFailure> {
    let mut connection = self.connection()?;
    validate_schema(&connection)?;
    let candidates = artifact_retention_candidates(
      &connection,
      &now.to_rfc3339_opts(SecondsFormat::Millis, true),
      ARTIFACT_MAINTENANCE_BATCH,
    )?;
    let mut report = ArtifactMaintenanceV1 {
      profile: "woml.artifact-maintenance/v1".to_string(),
      kind: if dry_run { "plan" } else { "result" }.to_string(),
      completed_at: timestamp(),
      eligible_artifacts: candidates.len() as u64,
      eligible_blobs: candidates
        .iter()
        .map(|(_, digest, _)| digest)
        .collect::<std::collections::BTreeSet<_>>()
        .len() as u64,
      estimated_bytes: candidates.iter().map(|(_, _, size)| *size).sum(),
      deleted_artifacts: 0,
      deleted_blobs: 0,
      deleted_bytes: 0,
      scrubbed_blobs: 0,
      corrupt_blobs: 0,
      collected_orphans: 0,
      live_artifacts: 0,
      live_blobs: 0,
      live_bytes: 0,
    };
    if !dry_run && !candidates.is_empty() {
      let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| backend_unavailable())?;
      let current = artifact_retention_candidates(
        &transaction,
        &now.to_rfc3339_opts(SecondsFormat::Millis, true),
        ARTIFACT_MAINTENANCE_BATCH,
      )?;
      let artifact_ids = current
        .iter()
        .map(|(artifact_id, _, _)| artifact_id.clone())
        .collect::<Vec<_>>();
      let digests = current
        .iter()
        .map(|(_, digest, size)| (digest.clone(), *size))
        .collect::<std::collections::BTreeMap<_, _>>();
      for artifact_id in &artifact_ids {
        transaction
          .execute(
            "DELETE FROM woml_artifact_transfers WHERE artifact_id = ?1",
            [artifact_id],
          )
          .map_err(|_| backend_unavailable())?;
        for table in [
          "woml_artifact_access",
          "woml_artifact_retention",
          "woml_artifact_provenance",
        ] {
          transaction
            .execute(
              &format!("DELETE FROM {table} WHERE artifact_id = ?1"),
              [artifact_id],
            )
            .map_err(|_| backend_unavailable())?;
        }
        transaction
          .execute(
            "DELETE FROM woml_artifacts WHERE artifact_id = ?1",
            [artifact_id],
          )
          .map_err(|_| backend_unavailable())?;
      }
      let mut unreachable = Vec::new();
      for (digest, size) in digests {
        let referenced: bool = transaction
          .query_row(
            "SELECT EXISTS(SELECT 1 FROM woml_artifacts WHERE sha256 = ?1)",
            [&digest],
            |row| row.get(0),
          )
          .map_err(|_| backend_unavailable())?;
        if !referenced {
          transaction
            .execute(
              "DELETE FROM woml_artifact_blobs WHERE sha256 = ?1",
              [&digest],
            )
            .map_err(|_| backend_unavailable())?;
          unreachable.push((digest, size));
        }
      }
      transaction.commit().map_err(|_| backend_unavailable())?;
      report.deleted_artifacts = artifact_ids.len() as u64;
      for (digest, size) in unreachable {
        self.backend.delete(&digest)?;
        report.deleted_blobs += 1;
        report.deleted_bytes = report.deleted_bytes.saturating_add(size);
      }
    }
    if !dry_run {
      let mut statement = connection
        .prepare("SELECT sha256, size FROM woml_artifact_blobs ORDER BY sha256 LIMIT ?1")
        .map_err(|_| backend_unavailable())?;
      let live = statement
        .query_map([ARTIFACT_SCRUB_BATCH as u64], |row| {
          Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        })
        .map_err(|_| backend_unavailable())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| backend_unavailable())?;
      drop(statement);
      for (digest, size) in live {
        report.scrubbed_blobs += 1;
        if self.backend.verify(&digest, size).is_err() {
          connection
            .execute(
              "UPDATE woml_artifacts SET status = 'corrupt'
               WHERE sha256 = ?1 AND status != 'deleted'",
              [&digest],
            )
            .map_err(|_| backend_unavailable())?;
          report.corrupt_blobs += 1;
        }
      }
      for digest in self.backend.inventory(ARTIFACT_ORPHAN_SCAN_BATCH)? {
        let reachable: bool = connection
          .query_row(
            "SELECT EXISTS(SELECT 1 FROM woml_artifact_blobs WHERE sha256 = ?1)",
            [&digest],
            |row| row.get(0),
          )
          .map_err(|_| backend_unavailable())?;
        if !reachable {
          self.backend.delete(&digest)?;
          report.collected_orphans += 1;
        }
      }
    }
    let (live_artifacts, live_blobs, live_bytes): (u64, u64, u64) = connection
      .query_row(
        "SELECT
           (SELECT COUNT(*) FROM woml_artifacts WHERE status != 'deleted'),
           (SELECT COUNT(*) FROM woml_artifact_blobs),
           (SELECT COALESCE(SUM(size), 0) FROM woml_artifact_blobs)",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
      )
      .map_err(|_| backend_unavailable())?;
    report.live_artifacts = live_artifacts;
    report.live_blobs = live_blobs;
    report.live_bytes = live_bytes;
    report.completed_at = timestamp();
    if !dry_run {
      let encoded = serde_json::to_string(&report).map_err(|_| backend_unavailable())?;
      connection
        .execute(
          "INSERT INTO woml_artifact_store_metadata(key, value)
           VALUES ('last_maintenance_v1', ?1)
           ON CONFLICT(key) DO UPDATE SET value = excluded.value",
          [encoded],
        )
        .map_err(|_| backend_unavailable())?;
    }
    Ok(report)
  }
}
