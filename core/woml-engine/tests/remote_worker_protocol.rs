use chrono::{Duration, TimeZone, Utc};
use std::{fs, path::PathBuf};
use uuid::Uuid;
use woml_engine::{
  decode_worker_hello, parse_target_registry, parse_worker_configuration,
  prepare_worker_configuration, RemoteCoordinatorError, RemoteSessionAuthorityV1,
  RemoteWorkerHealthStatusV1, TrustedWorkerIdentityV1, WorkerPollV1, WorkerShutdownV1,
  WORKER_PROTOCOL_PROFILE,
};

const REGISTRY: &str =
  include_str!("../../../woml/tests/fixtures/remote-execution-targets/target-registry.v1.json");
const MESSAGES: &str =
  include_str!("../../../woml/tests/fixtures/remote-execution-targets/worker-protocol.v1.json");
const CONFIGURATION: &str = include_str!(
  "../../../woml/tests/fixtures/remote-execution-targets/worker-configuration.v1.json"
);

fn hello() -> woml_engine::WorkerHelloV1 {
  let messages: serde_json::Value = serde_json::from_str(MESSAGES).unwrap();
  decode_worker_hello(serde_json::to_vec(&messages[0]).unwrap().as_slice()).unwrap()
}

fn authority() -> RemoteSessionAuthorityV1 {
  RemoteSessionAuthorityV1::new(
    parse_target_registry(REGISTRY.as_bytes()).unwrap(),
    [TrustedWorkerIdentityV1 {
      worker_id: "production-runner-01".to_string(),
      certificate_digest: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        .to_string(),
    }],
    [
      ("runtime.script".to_string(), 1),
      ("services.process".to_string(), 2),
      ("services.artifacts".to_string(), 1),
    ],
  )
  .unwrap()
}

#[test]
fn authenticated_session_is_bound_idle_renewable_and_closable() {
  let now = Utc.with_ymd_and_hms(2026, 10, 24, 12, 0, 0).unwrap();
  let mut authority = authority();
  let session = authority
    .establish(
      hello(),
      "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      now,
    )
    .unwrap();
  assert!(!session.dispatch_enabled);
  assert_eq!(session.worker_id, "production-runner-01");
  assert_eq!(
    authority.inspect(&session.session_id).unwrap().platform,
    hello().platform
  );
  let result = authority
    .poll(
      WorkerPollV1 {
        profile: WORKER_PROTOCOL_PROFILE.to_string(),
        protocol_version: 1,
        message_id: "poll_01".to_string(),
        message_type: "poll".to_string(),
        worker_id: session.worker_id.clone(),
        session_id: session.session_id.clone(),
        sent_at: now + Duration::seconds(1),
        target: "production-private".to_string(),
        dispatch_id: "none".to_string(),
      },
      "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      now + Duration::seconds(1),
    )
    .unwrap();
  assert_eq!(result.status, "idle");
  assert!(result.dispatch.is_none());
  assert_eq!(
    authority
      .poll(
        WorkerPollV1 {
          profile: WORKER_PROTOCOL_PROFILE.to_string(),
          protocol_version: 1,
          message_id: "poll_reordered_01".to_string(),
          message_type: "poll".to_string(),
          worker_id: session.worker_id.clone(),
          session_id: session.session_id.clone(),
          sent_at: now,
          target: "production-private".to_string(),
          dispatch_id: "none".to_string(),
        },
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        now + Duration::seconds(1),
      )
      .unwrap_err(),
    RemoteCoordinatorError::ReplayOrConflict
  );
  authority
    .shutdown(
      WorkerShutdownV1 {
        profile: WORKER_PROTOCOL_PROFILE.to_string(),
        protocol_version: 1,
        message_id: "shutdown_01".to_string(),
        message_type: "shutdown".to_string(),
        worker_id: session.worker_id.clone(),
        session_id: session.session_id.clone(),
        sent_at: now + Duration::seconds(2),
      },
      "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      now + Duration::seconds(2),
    )
    .unwrap();
  assert!(authority.inspect(&session.session_id).is_none());
}

#[test]
fn drain_rejects_new_admission_without_abandoning_active_dispatches() {
  let now = Utc.with_ymd_and_hms(2026, 10, 24, 12, 0, 0).unwrap();
  let mut authority = authority();
  let session = authority
    .establish(
      hello(),
      "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      now,
    )
    .unwrap();
  authority
    .reserve_dispatch(&session.session_id, "production-private")
    .unwrap();
  authority.begin_drain("production-private").unwrap();
  assert_eq!(
    authority
      .reserve_dispatch(&session.session_id, "production-private")
      .unwrap_err(),
    RemoteCoordinatorError::Unavailable
  );
  let health = authority.target_health("production-private").unwrap();
  let worker = health
    .iter()
    .find(|worker| worker.worker_id == "production-runner-01")
    .unwrap();
  assert_eq!(worker.status, RemoteWorkerHealthStatusV1::Draining);
  assert_eq!(worker.active_dispatches, 1);
  authority.release_dispatch(&session.session_id).unwrap();
  assert_eq!(
    authority
      .inspect(&session.session_id)
      .unwrap()
      .active_dispatches,
    0
  );
  let poll = authority
    .poll(
      WorkerPollV1 {
        profile: WORKER_PROTOCOL_PROFILE.to_string(),
        protocol_version: 1,
        message_id: "poll_draining_01".to_string(),
        message_type: "poll".to_string(),
        worker_id: session.worker_id,
        session_id: session.session_id,
        sent_at: now + Duration::seconds(1),
        target: "production-private".to_string(),
        dispatch_id: "none".to_string(),
      },
      "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      now + Duration::seconds(1),
    )
    .unwrap();
  assert_eq!(poll.status, "draining");
}

#[test]
fn identity_replay_target_version_and_capability_forgery_fail_closed() {
  let now = Utc.with_ymd_and_hms(2026, 10, 24, 12, 0, 0).unwrap();
  let mut identity_authority = authority();
  assert_eq!(
    identity_authority
      .establish(
        hello(),
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        now
      )
      .unwrap_err(),
    RemoteCoordinatorError::Unauthorized
  );

  let mut replay_authority = authority();
  replay_authority
    .establish(
      hello(),
      "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      now,
    )
    .unwrap();
  assert_eq!(
    replay_authority
      .establish(
        hello(),
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        now,
      )
      .unwrap_err(),
    RemoteCoordinatorError::ReplayOrConflict
  );

  for mutate in ["target", "version", "capability"] {
    let mut forged = hello();
    forged.message_id = format!("hello_{mutate}");
    match mutate {
      "target" => forged.targets = vec!["maintenance-runner".to_string()],
      "version" => forged.worker_version = "0.9.0".to_string(),
      _ => forged.capabilities[0].version = 65_535,
    }
    assert!(identity_authority
      .establish(
        forged,
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        now,
      )
      .is_err());
  }
}

#[test]
fn worker_configuration_is_strict_and_operator_material_stays_file_referenced() {
  assert_eq!(
    parse_worker_configuration(CONFIGURATION.as_bytes())
      .unwrap()
      .worker_id,
    "production-runner-01"
  );
  let mut invalid_url: serde_json::Value = serde_json::from_str(CONFIGURATION).unwrap();
  invalid_url["coordinatorUrl"] = serde_json::json!("https://coordinator.internal:999999");
  assert!(parse_worker_configuration(&serde_json::to_vec(&invalid_url).unwrap()).is_err());
  let root = std::env::temp_dir().join(format!("woml-worker-{}", Uuid::new_v4().simple()));
  fs::create_dir_all(&root).unwrap();
  let config_path = root.join("worker.json");
  let cert = root.join("worker.crt");
  let key = root.join("worker.key");
  let ca = root.join("ca.crt");
  let policy = root.join("process-policy.json");
  for path in [&cert, &key, &ca, &policy] {
    fs::write(path, b"operator-owned").unwrap();
  }
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    for path in [&cert, &key, &ca, &policy] {
      fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
  }
  let source = CONFIGURATION
    .replace("/etc/woml/worker.crt", "worker.crt")
    .replace("/etc/woml/worker.key", "worker.key")
    .replace("/etc/woml/coordinator-ca.crt", "ca.crt")
    .replace("./process-policy.json", "process-policy.json");
  fs::write(&config_path, &source).unwrap();
  let prepared = prepare_worker_configuration(source.as_bytes(), &config_path).unwrap();
  assert_eq!(prepared.private_key_path, key.canonicalize().unwrap());
  assert!(!serde_json::to_string(&prepared.configuration)
    .unwrap()
    .contains("operator-owned"));
  let _ = fs::remove_dir_all(root);
}

#[allow(dead_code)]
fn _path(_: PathBuf) {}
