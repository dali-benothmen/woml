//! Artifact output regression cases; retain the encompassing artifacts::tests filter.

use super::*;

#[test]
fn operational_output_rollover_is_explicit_ordered_redacted_and_atomic() {
  let temp = TempDir::new().unwrap();
  let state = temp.path().join("state.sqlite");
  let backend = temp.path().join("artifacts-v1");
  let managed = Arc::new(ManagedArtifactStore::default());
  let mut authored_policy = policy(temp.path(), &backend);
  authored_policy.operational_output = Some(ArtifactOperationalOutputPolicy {
    enabled: true,
    retention_ms: 86_400_000,
  });
  let secrets = BTreeMap::from([("TOKEN".to_string(), "secret-value".to_string())]);
  let handler = ManagedArtifactPutHandler::new(
    Arc::clone(&managed),
    authored_policy,
    ArtifactRuntimeMode::Foreground,
    &secrets,
  )
  .unwrap();
  managed
    .configure_for_state(&state, handler.policy())
    .unwrap();

  let mut writer = handler
    .begin_operational_output("deploy", "run-deploy", "apply", 2, "inv-apply-2")
    .unwrap()
    .expect("explicit operational-output grant");
  writer.append_redacted("stdout", "plan ready\n").unwrap();
  writer
    .append_redacted("stderr", "token=[redacted]\n")
    .unwrap();
  let artifact = writer.finish(true).unwrap().expect("published artifact");
  let store = managed.configured().unwrap();
  let verified = store
    .backend
    .verify(&artifact.sha256, artifact.size)
    .unwrap();
  let body = fs::read_to_string(&verified.path).unwrap();
  assert_eq!(
    body,
    "{\"source\":\"stdout\",\"text\":\"plan ready\\n\"}\n{\"source\":\"stderr\",\"text\":\"token=[redacted]\\n\"}\n"
  );
  assert!(!body.contains("secret-value"));
  assert_eq!(
    hex::encode(Sha256::digest(body.as_bytes())),
    artifact.sha256
  );
  assert_eq!(artifact.contract, ARTIFACT_REFERENCE_CONTRACT);

  let mut abandoned = handler
    .begin_operational_output("deploy", "run-deploy", "apply", 3, "inv-apply-3")
    .unwrap()
    .unwrap();
  abandoned.append_redacted("stdout", "partial").unwrap();
  drop(abandoned);
  assert_eq!(fs::read_dir(backend.join("staging")).unwrap().count(), 0);
}

#[test]
fn operational_output_rollover_denial_creates_no_fallback_stage() {
  let temp = TempDir::new().unwrap();
  let backend = temp.path().join("artifacts-v1");
  let managed = Arc::new(ManagedArtifactStore::default());
  let handler = ManagedArtifactPutHandler::new(
    managed,
    policy(temp.path(), &backend),
    ArtifactRuntimeMode::Foreground,
    &BTreeMap::new(),
  )
  .unwrap();
  assert!(handler
    .begin_operational_output("deploy", "run-deploy", "apply", 1, "inv-apply-1")
    .unwrap()
    .is_none());
  assert!(!backend.exists());
}
