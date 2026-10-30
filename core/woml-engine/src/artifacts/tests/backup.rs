//! Artifact backup regression cases; retain the encompassing artifacts::tests filter.

use super::*;

#[tokio::test]
async fn artifact_backup_round_trips_verified_bytes_and_rejects_corruption() {
  let temp = TempDir::new().unwrap();
  fs::write(temp.path().join("report.txt"), b"durable artifact backup").unwrap();
  let state = temp.path().join("state.sqlite");
  let source_backend = temp.path().join("source-artifacts-v1");
  let source_policy = policy(temp.path(), &source_backend);
  let managed = Arc::new(ManagedArtifactStore::default());
  let put = ManagedArtifactPutHandler::new(
    Arc::clone(&managed),
    source_policy.clone(),
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

  let backup = temp.path().join("backup-artifacts");
  let exported = export_artifact_backup(
    &state,
    &backup,
    source_policy,
    ArtifactRuntimeMode::Foreground,
    &BTreeMap::new(),
  )
  .unwrap();
  assert_eq!(exported.blob_count, 1);
  assert_eq!(exported.total_bytes, reference.size);
  assert_eq!(verify_artifact_backup(&state, &backup).unwrap(), exported);

  let restored_backend = temp.path().join("restored-artifacts-v1");
  let restored = restore_artifact_backup(
    &state,
    &backup,
    policy(temp.path(), &restored_backend),
    ArtifactRuntimeMode::Foreground,
    &BTreeMap::new(),
  )
  .unwrap();
  assert_eq!(restored, exported);
  let restored_blob = restored_backend
    .join("blobs/sha256")
    .join(&reference.sha256[..2])
    .join(&reference.sha256[2..]);
  assert_eq!(
    fs::read(&restored_blob).unwrap(),
    b"durable artifact backup"
  );

  let backup_blob = backup
    .join("blobs/sha256")
    .join(&reference.sha256[..2])
    .join(&reference.sha256[2..]);
  fs::write(backup_blob, b"corrupt").unwrap();
  let failure = verify_artifact_backup(&state, &backup).unwrap_err();
  assert_eq!(failure.code, "WOML_ARTIFACT_CORRUPT");
}
