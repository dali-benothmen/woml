//! Artifact backends regression cases; retain the encompassing artifacts::tests filter.

use super::*;

fn assert_backend_conformance(backend: &dyn ArtifactBackend, store_id: &str) {
  let connection = Connection::open_in_memory().unwrap();
  initialize_schema(&connection).unwrap();
  backend.bind_store(&connection, store_id).unwrap();
  let bytes = b"portable artifact backend";
  let digest = hex::encode(Sha256::digest(bytes));

  let mut stage = backend.begin_stage().unwrap();
  stage.file.as_mut().unwrap().write_all(bytes).unwrap();
  backend
    .publish(
      stage,
      &digest,
      bytes.len() as u64,
      &CapabilityCancellationToken::default(),
    )
    .unwrap();
  let verified = backend.verify(&digest, bytes.len() as u64).unwrap();
  assert_eq!(fs::read(&verified.path).unwrap(), bytes);
  drop(verified);
  assert_eq!(backend.inventory(10).unwrap(), vec![digest.clone()]);

  let mut duplicate = backend.begin_stage().unwrap();
  duplicate.file.as_mut().unwrap().write_all(bytes).unwrap();
  backend
    .publish(
      duplicate,
      &digest,
      bytes.len() as u64,
      &CapabilityCancellationToken::default(),
    )
    .unwrap();
  assert_eq!(backend.inventory(10).unwrap(), vec![digest.clone()]);

  let mut cancelled = backend.begin_stage().unwrap();
  cancelled
    .file
    .as_mut()
    .unwrap()
    .write_all(b"cancelled")
    .unwrap();
  let cancellation = CapabilityCancellationToken::default();
  cancellation.cancel();
  let failure = backend
    .publish(
      cancelled,
      &hex::encode(Sha256::digest(b"cancelled")),
      9,
      &cancellation,
    )
    .unwrap_err();
  assert_eq!(failure.code, "WOML_ARTIFACT_CANCELLED");

  backend.delete(&digest).unwrap();
  backend.delete(&digest).unwrap();
  assert!(backend.inventory(10).unwrap().is_empty());
}

#[test]
fn local_and_s3_backends_share_the_frozen_conformance_suite() {
  let local_root = TempDir::new().unwrap();
  let local = LocalArtifactBackend::open(&local_root.path().join("local")).unwrap();
  assert_backend_conformance(&local, "store-local");

  let s3_root = TempDir::new().unwrap();
  let server = MockS3Server::start();
  let s3_config = |access_key: &str| PreparedS3Backend {
    bucket: "woml-test".to_string(),
    region: "eu-central-1".to_string(),
    prefix: "tenant-a/".to_string(),
    endpoint: Url::parse(&server.endpoint).unwrap(),
    force_path_style: true,
    credentials: S3Credentials {
      access_key: access_key.to_string(),
      secret_key: "test-secret-key".to_string(),
      session_token: Some("test-session-token".to_string()),
    },
    server_side_encryption: S3ServerSideEncryption::Aes256,
  };
  let s3 = S3ArtifactBackend::open(s3_config("ACCESS_ONE"), s3_root.path()).unwrap();
  assert_backend_conformance(&s3, "store-s3");

  // A new backend instance resolves credentials again, which permits
  // operator-controlled rotation without changing durable metadata.
  let rotated = S3ArtifactBackend::open(s3_config("ACCESS_TWO"), s3_root.path()).unwrap();
  let connection = Connection::open_in_memory().unwrap();
  initialize_schema(&connection).unwrap();
  rotated.bind_store(&connection, "store-s3").unwrap();

  let requests = server.requests.lock().unwrap().clone();
  let blob_puts = requests
    .iter()
    .filter(|request| request.method == "PUT" && request.target.contains("blobs/sha256"))
    .count();
  let blob_gets = requests
    .iter()
    .filter(|request| request.method == "GET" && request.target.contains("blobs/sha256"))
    .count();
  assert!(
    blob_puts >= 3,
    "transient upload and duplicate publication must retry safely"
  );
  assert!(
    blob_gets >= 3,
    "eventual visibility must recover before verification succeeds"
  );
  assert!(requests.iter().any(|request| {
    request
      .headers
      .get("authorization")
      .is_some_and(|value| value.contains("Credential=ACCESS_ONE/"))
      && request
        .headers
        .get("x-amz-server-side-encryption")
        .map(String::as_str)
        == Some("AES256")
  }));
  assert!(requests.iter().any(|request| {
    request
      .headers
      .get("authorization")
      .is_some_and(|value| value.contains("Credential=ACCESS_TWO/"))
  }));
  let wire_log = format!("{requests:?}");
  assert!(!wire_log.contains("test-secret-key"));
}

#[test]
fn rust_resolves_s3_credentials_symbolically_and_redacts_policy_debugging() {
  let temp = TempDir::new().unwrap();
  let mut authored = policy(temp.path(), &temp.path().join("unused-local"));
  authored.backend = ArtifactBackendPolicy::S3 {
    contract: "woml.artifact-backend".to_string(),
    contract_version: 1,
    bucket: "woml-production-artifacts".to_string(),
    region: "eu-central-1".to_string(),
    prefix: "runtime-a/".to_string(),
    endpoint: Some("https://objects.example.com".to_string()),
    force_path_style: true,
    authentication: Box::new(json!({
      "mode": "secrets",
      "accessKeySecret": "S3_ACCESS_KEY",
      "secretKeySecret": "S3_SECRET_KEY",
      "sessionTokenSecret": "S3_SESSION_TOKEN"
    })),
    server_side_encryption: Box::new(json!({
      "mode": "aws-kms",
      "keyId": "alias/woml-artifacts"
    })),
  };
  let missing = PreparedArtifactPolicy::prepare(
    authored.clone(),
    ArtifactRuntimeMode::Foreground,
    &BTreeMap::new(),
  )
  .unwrap_err();
  assert!(missing.to_string().contains("S3_ACCESS_KEY"));

  let secrets = BTreeMap::from([
    ("S3_ACCESS_KEY".to_string(), "access-value".to_string()),
    ("S3_SECRET_KEY".to_string(), "secret-value".to_string()),
    ("S3_SESSION_TOKEN".to_string(), "session-value".to_string()),
  ]);
  let prepared =
    PreparedArtifactPolicy::prepare(authored.clone(), ArtifactRuntimeMode::Foreground, &secrets)
      .unwrap();
  let debug = format!("{prepared:?}");
  for value in secrets.values() {
    assert!(!debug.contains(value));
  }

  if let ArtifactBackendPolicy::S3 { endpoint, .. } = &mut authored.backend {
    *endpoint = Some("http://objects.example.com".to_string());
  }
  assert!(
    PreparedArtifactPolicy::prepare(authored, ArtifactRuntimeMode::Foreground, &secrets,).is_err()
  );
}
