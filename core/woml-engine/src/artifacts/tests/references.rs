//! Artifact references regression cases; retain the encompassing artifacts::tests filter.

use super::*;

#[tokio::test]
async fn head_and_materialize_verify_same_run_access_and_cleanup() {
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
  let head = put.head_handler();
  let materialize = put.materialize_handler();
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

  let head_result = head
    .execute_request_scoped(
      &reference_call("head", &reference, "run-artifact", "inv-head"),
      Some("workflow".to_string()),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap();
  assert_eq!(head_result["data"]["integrity"], "verified");
  assert_eq!(head_result["data"]["artifact"]["id"], reference.id);

  let invocation_id = "inv-materialize";
  let materialized = materialize
    .execute_request_scoped(
      &reference_call("materialize", &reference, "run-artifact", invocation_id),
      Some("workflow".to_string()),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap();
  let path = PathBuf::from(materialized["data"]["path"].as_str().unwrap());
  assert_eq!(fs::read(&path).unwrap(), b"durable artifact");
  managed.cleanup_invocation(invocation_id);
  assert!(!path.exists());
}

#[tokio::test]
async fn artifact_reads_reject_unauthorized_forged_expired_and_corrupt_references() {
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
  let head = put.head_handler();
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

  let failure = head
    .execute_request_scoped(
      &reference_call("head", &reference, "another-run", "inv-other"),
      Some("workflow".to_string()),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap_err();
  assert_eq!(failure.code, "WOML_ARTIFACT_ACCESS_DENIED");

  let mut forged = reference.clone();
  forged.name = "forged".to_string();
  let failure = head
    .execute_request_scoped(
      &reference_call("head", &forged, "run-artifact", "inv-forged"),
      Some("workflow".to_string()),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap_err();
  assert_eq!(failure.code, "WOML_ARTIFACT_ACCESS_DENIED");

  Connection::open(&state)
    .unwrap()
    .execute(
      "UPDATE woml_artifact_retention SET expires_at = '2000-01-01T00:00:00.000Z'",
      [],
    )
    .unwrap();
  let failure = head
    .execute_request_scoped(
      &reference_call("head", &reference, "run-artifact", "inv-expired"),
      Some("workflow".to_string()),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap_err();
  assert_eq!(failure.code, "WOML_ARTIFACT_EXPIRED");

  Connection::open(&state)
    .unwrap()
    .execute(
      "UPDATE woml_artifact_retention SET expires_at = '2999-01-01T00:00:00.000Z'",
      [],
    )
    .unwrap();
  let blob = backend
    .join("blobs/sha256")
    .join(&reference.sha256[..2])
    .join(&reference.sha256[2..]);
  fs::write(blob, b"corrupt artifact").unwrap();
  let failure = head
    .execute_request_scoped(
      &reference_call("head", &reference, "run-artifact", "inv-corrupt"),
      Some("workflow".to_string()),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap_err();
  assert_eq!(failure.code, "WOML_ARTIFACT_CORRUPT");

  let other = TempDir::new().unwrap();
  let other_store = Arc::new(ManagedArtifactStore::default());
  let other_put = ManagedArtifactPutHandler::new(
    Arc::clone(&other_store),
    policy(other.path(), &other.path().join("artifacts-v1")),
    ArtifactRuntimeMode::Foreground,
    &BTreeMap::new(),
  )
  .unwrap();
  let other_head = other_put.head_handler();
  other_store
    .configure_for_state(&other.path().join("state.sqlite"), other_put.policy())
    .unwrap();
  let failure = other_head
    .execute_request_scoped(
      &reference_call("head", &reference, "run-artifact", "inv-other-store"),
      Some("workflow".to_string()),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap_err();
  assert_eq!(failure.code, "WOML_ARTIFACT_NOT_FOUND");
}

#[tokio::test]
async fn materialization_cleans_partial_files_and_fails_closed_on_cancellation() {
  let temp = TempDir::new().unwrap();
  fs::write(temp.path().join("report.bin"), vec![b'x'; 512 * 1024]).unwrap();
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
  let materialize = put.materialize_handler();
  managed.configure_for_state(&state, put.policy()).unwrap();
  let stored = put
    .execute_request_scoped(
      &call("report.bin", "report"),
      Some("workflow".to_string()),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap();
  let reference: ArtifactReferenceV1 = serde_json::from_value(stored["data"].clone()).unwrap();
  let store = managed.configured().unwrap();

  let corrupt_invocation = "inv-partial";
  let corrupt_directory = store.invocation_materialization_directory(corrupt_invocation);
  ensure_private_directory(&corrupt_directory).unwrap();
  fs::write(corrupt_directory.join(&reference.id), b"partial").unwrap();
  let failure = materialize
    .execute_request_scoped(
      &reference_call(
        "materialize",
        &reference,
        "run-artifact",
        corrupt_invocation,
      ),
      Some("workflow".to_string()),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap_err();
  assert_eq!(failure.code, "WOML_ARTIFACT_CORRUPT");
  managed.cleanup_invocation(corrupt_invocation);

  let unavailable_invocation = "inv-unavailable";
  let unavailable_path = store.invocation_materialization_directory(unavailable_invocation);
  fs::write(&unavailable_path, b"directory collision").unwrap();
  let failure = materialize
    .execute_request_scoped(
      &reference_call(
        "materialize",
        &reference,
        "run-artifact",
        unavailable_invocation,
      ),
      Some("workflow".to_string()),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap_err();
  assert_eq!(failure.code, "WOML_ARTIFACT_MATERIALIZATION_UNAVAILABLE");
  fs::remove_file(unavailable_path).unwrap();

  let cancellation = CapabilityCancellationToken::default();
  cancellation.cancel();
  let failure = materialize
    .execute_request_scoped(
      &reference_call("materialize", &reference, "run-artifact", "inv-cancelled"),
      Some("workflow".to_string()),
      cancellation,
    )
    .await
    .unwrap_err();
  assert_eq!(failure.code, "WOML_ARTIFACT_CANCELLED");

  let startup_partial = store.materializations.join("abandoned-attempt");
  ensure_private_directory(&startup_partial).unwrap();
  fs::write(startup_partial.join("download.part"), b"partial").unwrap();
  drop(store);
  drop(materialize);
  drop(put);
  drop(managed);
  let recovered = Arc::new(ManagedArtifactStore::default());
  let recovered_put = ManagedArtifactPutHandler::new(
    Arc::clone(&recovered),
    policy(temp.path(), &backend),
    ArtifactRuntimeMode::Foreground,
    &BTreeMap::new(),
  )
  .unwrap();
  recovered
    .configure_for_state(&state, recovered_put.policy())
    .unwrap();
  assert!(!startup_partial.exists());
}
