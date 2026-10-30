//! Artifact lifecycle regression cases; retain the encompassing artifacts::tests filter.

use super::*;

#[test]
fn artifact_backend_is_bound_to_one_durable_store() {
  let temp = TempDir::new().unwrap();
  let backend = temp.path().join("artifacts-v1");
  let first_store = Arc::new(ManagedArtifactStore::default());
  let first_put = ManagedArtifactPutHandler::new(
    Arc::clone(&first_store),
    policy(temp.path(), &backend),
    ArtifactRuntimeMode::Foreground,
    &BTreeMap::new(),
  )
  .unwrap();
  first_store
    .configure_for_state(&temp.path().join("first.sqlite"), first_put.policy())
    .unwrap();
  drop(first_put);
  drop(first_store);

  let second_store = Arc::new(ManagedArtifactStore::default());
  let second_put = ManagedArtifactPutHandler::new(
    Arc::clone(&second_store),
    policy(temp.path(), &backend),
    ArtifactRuntimeMode::Foreground,
    &BTreeMap::new(),
  )
  .unwrap();
  let failure = second_store
    .configure_for_state(&temp.path().join("second.sqlite"), second_put.policy())
    .unwrap_err();
  assert_eq!(failure.code, "WOML_ARTIFACT_BACKEND_UNAVAILABLE");
}

#[tokio::test]
async fn maintenance_protects_active_runs_then_collects_expired_artifacts_and_orphans() {
  let temp = TempDir::new().unwrap();
  fs::write(temp.path().join("report.txt"), b"durable artifact").unwrap();
  let state = temp.path().join("state.sqlite");
  let backend = temp.path().join("artifacts-v1");
  let artifact_policy = policy(temp.path(), &backend);
  let managed = Arc::new(ManagedArtifactStore::default());
  let put = ManagedArtifactPutHandler::new(
    Arc::clone(&managed),
    artifact_policy.clone(),
    ArtifactRuntimeMode::Foreground,
    &BTreeMap::new(),
  )
  .unwrap();
  managed.configure_for_state(&state, put.policy()).unwrap();
  let stored = put
    .execute_request_scoped(
      &call("report.txt", "report"),
      Some("workflow".to_string()),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap();
  let reference: ArtifactReferenceV1 = serde_json::from_value(stored["data"].clone()).unwrap();
  drop(put);
  drop(managed);

  let connection = Connection::open(&state).unwrap();
  connection
    .execute(
      "UPDATE woml_artifact_retention SET expires_at = '2000-01-01T00:00:00.000Z'",
      [],
    )
    .unwrap();
  let definition_hash = format!("sha256:{}", "a".repeat(64));
  connection
    .execute(
      "INSERT INTO woml_definitions(
         definition_hash, workflow_id, schema_version, model_json, created_at
       ) VALUES (?1, 'workflow', 1, '{}', ?2)",
      params![definition_hash, timestamp()],
    )
    .unwrap();
  connection
    .execute(
      "INSERT INTO woml_runs(run_id, workflow_id, definition_hash, created_at)
       VALUES ('run-artifact', 'workflow', ?1, ?2)",
      params![definition_hash, timestamp()],
    )
    .unwrap();
  connection
    .execute(
      "INSERT INTO woml_run_summaries(
         run_id, workflow_id, status, admitted_at, updated_at
       ) VALUES ('run-artifact', 'workflow', 'waiting', ?1, ?1)",
      [timestamp()],
    )
    .unwrap();
  drop(connection);

  let protected = maintain_artifacts(
    &state,
    artifact_policy.clone(),
    ArtifactRuntimeMode::Foreground,
    &BTreeMap::new(),
    Utc::now(),
    true,
  )
  .unwrap();
  assert_eq!(protected.eligible_artifacts, 0);

  Connection::open(&state)
    .unwrap()
    .execute(
      "UPDATE woml_run_summaries SET status = 'succeeded', updated_at = ?1
       WHERE run_id = 'run-artifact'",
      [timestamp()],
    )
    .unwrap();
  let orphan_digest = hex::encode(Sha256::digest(b"orphan"));
  let orphan = backend
    .join("blobs/sha256")
    .join(&orphan_digest[..2])
    .join(&orphan_digest[2..]);
  fs::create_dir_all(orphan.parent().unwrap()).unwrap();
  fs::write(&orphan, b"orphan").unwrap();
  let collected = maintain_artifacts(
    &state,
    artifact_policy,
    ArtifactRuntimeMode::Foreground,
    &BTreeMap::new(),
    Utc::now(),
    false,
  )
  .unwrap();
  assert_eq!(collected.deleted_artifacts, 1);
  assert_eq!(collected.deleted_blobs, 1);
  assert_eq!(collected.deleted_bytes, reference.size);
  // Startup recovery is allowed to collect the orphan before the bounded
  // maintenance scan accounts for it.
  assert!(collected.collected_orphans <= 1);
  assert_eq!(collected.live_artifacts, 0);
  assert_eq!(collected.live_blobs, 0);
  assert!(!orphan.exists());
}

#[test]
fn future_artifact_schema_is_rejected() {
  let temp = TempDir::new().unwrap();
  let state = temp.path().join("state.sqlite");
  DurableEventStore::open(&state).unwrap();
  let connection = Connection::open(&state).unwrap();
  initialize_schema(&connection).unwrap();
  connection
    .execute(
      "UPDATE woml_artifact_store_metadata SET value = '2' WHERE key = 'schema_version'",
      [],
    )
    .unwrap();
  assert_eq!(
    validate_schema(&connection).unwrap_err().kind,
    CapabilityFailureKind::UnsupportedVersion
  );
}
