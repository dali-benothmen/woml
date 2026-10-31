//! Artifact publication regression cases; retain the encompassing artifacts::tests filter.

use super::*;

#[tokio::test]
async fn put_is_durable_deduplicated_and_identity_safe() {
  let temp = TempDir::new().unwrap();
  fs::write(temp.path().join("report.txt"), b"durable artifact").unwrap();
  let state = temp.path().join("state.sqlite");
  let backend = temp.path().join("artifacts-v1");
  fs::create_dir_all(backend.join("staging")).unwrap();
  fs::write(backend.join("staging/orphan.part"), b"partial").unwrap();
  let managed = Arc::new(ManagedArtifactStore::default());
  let handler = ManagedArtifactPutHandler::new(
    Arc::clone(&managed),
    policy(temp.path(), &backend),
    ArtifactRuntimeMode::Foreground,
    &BTreeMap::new(),
  )
  .unwrap();
  managed
    .configure_for_state(&state, handler.policy())
    .unwrap();
  assert!(!backend.join("staging/orphan.part").exists());
  let request = call("report.txt", "report");
  let first = handler
    .execute_request_scoped(
      &request,
      Some("workflow".to_string()),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap();
  let second = handler
    .execute_request_scoped(
      &request,
      Some("workflow".to_string()),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap();
  assert_eq!(first, second);
  assert_eq!(first["contract"], "woml.artifacts");
  assert_eq!(first["data"]["contract"], "woml.artifact-reference");
  assert_eq!(first["data"]["name"], "report");
  assert_eq!(first["data"]["size"], 16);
  let digest = first["data"]["sha256"].as_str().unwrap();
  assert!(backend
    .join("blobs/sha256")
    .join(&digest[..2])
    .join(&digest[2..])
    .is_file());
  let connection = Connection::open(&state).unwrap();
  for table in [
    "woml_artifacts",
    "woml_artifact_blobs",
    "woml_artifact_provenance",
    "woml_artifact_access",
    "woml_artifact_transfers",
    "woml_artifact_retention",
  ] {
    let count: u64 = connection
      .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
      })
      .unwrap();
    assert_eq!(count, 1, "{table}");
  }
  fs::write(temp.path().join("report.txt"), b"changed bytes").unwrap();
  let failure = handler
    .execute_request_scoped(
      &request,
      Some("workflow".to_string()),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap_err();
  assert_eq!(failure.code, "WOML_ARTIFACT_OPERATION_IDENTITY_CONFLICT");
}

#[tokio::test]
async fn put_enforces_byte_limits_and_known_secret_scanning() {
  let temp = TempDir::new().unwrap();
  fs::write(
    temp.path().join("secret.txt"),
    b"prefix super-secret-value suffix",
  )
  .unwrap();
  let state = temp.path().join("state.sqlite");
  let backend = temp.path().join("artifacts-v1");
  let managed = Arc::new(ManagedArtifactStore::default());
  let mut secrets = BTreeMap::new();
  secrets.insert("TOKEN".to_string(), "super-secret-value".to_string());
  let handler = ManagedArtifactPutHandler::new(
    Arc::clone(&managed),
    policy(temp.path(), &backend),
    ArtifactRuntimeMode::Foreground,
    &secrets,
  )
  .unwrap();
  managed
    .configure_for_state(&state, handler.policy())
    .unwrap();
  let request = call("secret.txt", "secret-report");
  let failure = handler
    .execute_request_scoped(
      &request,
      Some("workflow".to_string()),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap_err();
  assert_eq!(failure.code, "WOML_ARTIFACT_QUARANTINED");
  let connection = Connection::open(&state).unwrap();
  let settlement: (String, String, i64) = connection
    .query_row(
      "SELECT status, error_code, ambiguous FROM woml_artifact_transfers",
      [],
      |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .unwrap();
  assert_eq!(
    settlement,
    (
      "failed".to_string(),
      "WOML_ARTIFACT_QUARANTINED".to_string(),
      0
    )
  );

  let second_root = TempDir::new().unwrap();
  fs::write(second_root.path().join("large.txt"), b"12345").unwrap();
  let mut limited = policy(second_root.path(), &second_root.path().join("artifacts-v1"));
  limited.limits.max_artifact_bytes = 4;
  limited.limits.max_run_bytes = 4;
  limited.limits.max_store_bytes = 4;
  let limited_store = Arc::new(ManagedArtifactStore::default());
  let limited_handler = ManagedArtifactPutHandler::new(
    Arc::clone(&limited_store),
    limited,
    ArtifactRuntimeMode::Foreground,
    &BTreeMap::new(),
  )
  .unwrap();
  limited_store
    .configure_for_state(
      &second_root.path().join("state.sqlite"),
      limited_handler.policy(),
    )
    .unwrap();
  let failure = limited_handler
    .execute_request_scoped(
      &call("large.txt", "large-report"),
      Some("workflow".to_string()),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap_err();
  assert_eq!(failure.code, "WOML_ARTIFACT_TOO_LARGE");
}

#[tokio::test]
async fn retries_reconnect_and_startup_recovers_uncommitted_files_fail_closed() {
  let temp = TempDir::new().unwrap();
  fs::write(temp.path().join("report.txt"), b"durable artifact").unwrap();
  let state = temp.path().join("state.sqlite");
  let backend = temp.path().join("artifacts-v1");
  let managed = Arc::new(ManagedArtifactStore::default());
  let put = ManagedArtifactPutHandler::new(
    Arc::clone(&managed),
    policy(temp.path(), &backend),
    ArtifactRuntimeMode::Foreground,
    &BTreeMap::new(),
  )
  .unwrap();
  managed.configure_for_state(&state, put.policy()).unwrap();
  let original = call("report.txt", "report");
  let first = put
    .execute_request_scoped(
      &original,
      Some("workflow".to_string()),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap();
  let mut retry = original.clone();
  retry.attempt_number = 2;
  retry.invocation_id = "invocation-artifact-retry".to_string();
  retry.call_id = "call-artifact-retry".to_string();
  let replayed = put
    .execute_request_scoped(
      &retry,
      Some("workflow".to_string()),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap();
  assert_eq!(first, replayed);
  let connection = Connection::open(&state).unwrap();
  let artifacts: u64 = connection
    .query_row("SELECT COUNT(*) FROM woml_artifacts", [], |row| row.get(0))
    .unwrap();
  assert_eq!(artifacts, 1);

  let crash_root = TempDir::new().unwrap();
  let crash_state = crash_root.path().join("state.sqlite");
  let crash_backend = crash_root.path().join("artifacts-v1");
  let initial_store = Arc::new(ManagedArtifactStore::default());
  let initial_put = ManagedArtifactPutHandler::new(
    Arc::clone(&initial_store),
    policy(crash_root.path(), &crash_backend),
    ArtifactRuntimeMode::Foreground,
    &BTreeMap::new(),
  )
  .unwrap();
  initial_store
    .configure_for_state(&crash_state, initial_put.policy())
    .unwrap();
  drop(initial_put);
  drop(initial_store);
  let connection = Connection::open(&crash_state).unwrap();
  let interrupted = call("report.txt", "interrupted");
  connection
    .execute(
      "INSERT INTO woml_artifact_transfers(
         operation_key, run_id, node_id, attempt_number, operation,
         input_digest, status, started_at
       ) VALUES (?1, ?2, ?3, 1, 'put', ?4, 'started', ?5)",
      params![
        interrupted.identity.operation_key,
        interrupted.run_id,
        interrupted.node_id,
        canonical_digest(b"woml.artifact-input\0v1\0", &interrupted.input).unwrap(),
        timestamp(),
      ],
    )
    .unwrap();
  fs::create_dir_all(crash_backend.join("staging")).unwrap();
  fs::write(
    crash_backend.join("staging/before-publish.part"),
    b"partial",
  )
  .unwrap();
  let orphan_digest = hex::encode(Sha256::digest(b"published without metadata"));
  let orphan = crash_backend
    .join("blobs/sha256")
    .join(&orphan_digest[..2])
    .join(&orphan_digest[2..]);
  fs::create_dir_all(orphan.parent().unwrap()).unwrap();
  fs::write(&orphan, b"published without metadata").unwrap();
  drop(connection);

  let recovered = Arc::new(ManagedArtifactStore::default());
  let recovered_put = ManagedArtifactPutHandler::new(
    Arc::clone(&recovered),
    policy(crash_root.path(), &crash_backend),
    ArtifactRuntimeMode::Foreground,
    &BTreeMap::new(),
  )
  .unwrap();
  recovered
    .configure_for_state(&crash_state, recovered_put.policy())
    .unwrap();
  assert!(!crash_backend.join("staging/before-publish.part").exists());
  assert!(!orphan.exists());
  let failure = recovered_put
    .execute_request_scoped(
      &interrupted,
      Some("workflow".to_string()),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap_err();
  assert_eq!(failure.code, "WOML_ARTIFACT_INTERRUPTED");
}
