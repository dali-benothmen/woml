//! Artifact inspection regression cases; retain the encompassing artifacts::tests filter.

use super::*;

#[tokio::test]
async fn operator_inspection_export_and_observation_are_bounded_and_audited() {
  let temp = TempDir::new().unwrap();
  fs::write(
    temp.path().join("plan.bin"),
    b"reviewed infrastructure plan",
  )
  .unwrap();
  let state = temp.path().join("state.sqlite");
  let backend = temp.path().join("artifacts-v1");
  let authored_policy = policy(temp.path(), &backend);
  let managed = Arc::new(ManagedArtifactStore::default());
  let handler = ManagedArtifactPutHandler::new(
    Arc::clone(&managed),
    authored_policy.clone(),
    ArtifactRuntimeMode::Foreground,
    &BTreeMap::new(),
  )
  .unwrap();
  managed
    .configure_for_state(&state, handler.policy())
    .unwrap();
  let created = handler
    .execute_request_scoped(
      &call("plan.bin", "terraform-plan"),
      Some("infrastructure-review".to_string()),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap();
  let artifact_id = created["data"]["id"].as_str().unwrap();

  let inspection = inspect_artifact(&state, artifact_id).unwrap();
  assert_eq!(inspection.artifact.name, "terraform-plan");
  assert_eq!(inspection.digest_prefix.len(), 12);
  assert_eq!(inspection.provenance.run_id, "run-artifact");
  assert_eq!(inspection.provenance.node_id, "create");
  let run = inspect_run_artifacts(&state, "run-artifact").unwrap();
  assert_eq!(run.artifacts, vec![inspection.clone()]);
  assert!(!run.truncated);

  let export_path = temp.path().join("reviewed-plan.bin");
  let exported = export_artifact(
    &state,
    &export_path,
    artifact_id,
    authored_policy.clone(),
    ArtifactRuntimeMode::Foreground,
    &BTreeMap::new(),
  )
  .unwrap();
  assert!(exported.verified);
  assert_eq!(
    fs::read(&export_path).unwrap(),
    b"reviewed infrastructure plan"
  );
  assert!(!format!("{inspection:?}{exported:?}").contains(export_path.to_string_lossy().as_ref()));

  let observation = observe_artifacts(
    &state,
    authored_policy,
    ArtifactRuntimeMode::Foreground,
    &BTreeMap::new(),
  )
  .unwrap();
  assert!(observation.backend_healthy);
  assert_eq!(observation.backend, "local");
  assert_eq!(observation.transfers, 1);
  assert_eq!(observation.transferred_bytes, 28);
  assert_eq!(observation.failed_transfers, 0);
  assert_eq!(observation.corrupt_artifacts, 0);
  assert_eq!(observation.orphan_blobs, 0);

  let connection = Connection::open(&state).unwrap();
  let audits: u64 = connection
    .query_row(
      "SELECT COUNT(*) FROM woml_artifact_operator_audit",
      [],
      |row| row.get(0),
    )
    .unwrap();
  assert_eq!(audits, 3);
}

#[tokio::test]
async fn streaming_startup_and_metadata_stay_within_publication_budgets() {
  const FIXTURE_BYTES: usize = 8 * 1024 * 1024;
  const MINIMUM_BYTES_PER_SECOND: f64 = 1024.0 * 1024.0;
  const STARTUP_BUDGET: Duration = Duration::from_secs(5);
  const INSPECTION_BUDGET: Duration = Duration::from_secs(1);
  assert!(STREAM_BUFFER_BYTES <= 64 * 1024);

  let temp = TempDir::new().unwrap();
  let source = temp.path().join("large-report.bin");
  fs::write(&source, vec![0x5a; FIXTURE_BYTES]).unwrap();
  let state = temp.path().join("state.sqlite");
  let backend = temp.path().join("artifacts-v1");
  let mut authored_policy = policy(temp.path(), &backend);
  authored_policy.limits.max_artifact_bytes = 16 * 1024 * 1024;
  authored_policy.limits.max_run_bytes = 32 * 1024 * 1024;
  authored_policy.limits.max_store_bytes = 64 * 1024 * 1024;
  let managed = Arc::new(ManagedArtifactStore::default());
  let handler = ManagedArtifactPutHandler::new(
    Arc::clone(&managed),
    authored_policy.clone(),
    ArtifactRuntimeMode::Foreground,
    &BTreeMap::new(),
  )
  .unwrap();
  managed
    .configure_for_state(&state, handler.policy())
    .unwrap();
  let started = std::time::Instant::now();
  let created = handler
    .execute_request_scoped(
      &call("large-report.bin", "large-report"),
      Some("performance-workflow".to_string()),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap();
  let elapsed = started.elapsed();
  let throughput = FIXTURE_BYTES as f64 / elapsed.as_secs_f64().max(0.001);
  assert!(throughput >= MINIMUM_BYTES_PER_SECOND, "{throughput} B/s");

  drop(handler);
  drop(managed);
  let startup = std::time::Instant::now();
  let recovered = Arc::new(ManagedArtifactStore::default());
  let recovered_handler = ManagedArtifactPutHandler::new(
    Arc::clone(&recovered),
    authored_policy,
    ArtifactRuntimeMode::Foreground,
    &BTreeMap::new(),
  )
  .unwrap();
  recovered
    .configure_for_state(&state, recovered_handler.policy())
    .unwrap();
  assert!(startup.elapsed() <= STARTUP_BUDGET);

  let artifact_id = created["data"]["id"].as_str().unwrap();
  let inspection = std::time::Instant::now();
  let artifact = inspect_artifact(&state, artifact_id).unwrap();
  let run = inspect_run_artifacts(&state, "run-artifact").unwrap();
  assert!(inspection.elapsed() <= INSPECTION_BUDGET);
  assert_eq!(artifact.artifact.size, FIXTURE_BYTES as u64);
  assert_eq!(run.artifacts.len(), 1);
}
