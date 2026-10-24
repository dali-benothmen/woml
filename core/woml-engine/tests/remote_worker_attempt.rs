use std::{
  collections::BTreeSet,
  path::{Path, PathBuf},
  sync::Arc,
};

use chrono::{Duration, TimeZone, Utc};
use serde_json::{json, Map};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use woml_engine::{
  decode_remote_dispatch_package, decode_worker_output_batch, derive_remote_dispatch_id,
  prepare_remote_dispatch, verify_remote_artifact_transfer, CompiledWorkflowDefinition,
  EnvironmentRemoteSecretProvider, OperationalBodyV1, OperationalSourceV1, RemoteArtifactGrantV1,
  RemoteArtifactOperationV1, RemoteCoordinatorAuthorityV1, RemoteDispatchPackageV1,
  RemoteDispatchRequestV1, RemoteOperationalRecordV1, RemoteSecretProvider,
  RemoteWorkerAttemptRuntimeV1, RemoteWorkerAuthorityV1, RemoteWorkerCancellationTokenV1,
  RemoteWorkerError, ScriptHostProcessOptions, WorkerAttemptStateV1, WorkerCapabilityV1,
  WorkerJournalV1, WorkerOutputBatchV1, WorkflowContext,
};

const MODEL: &str =
  include_str!("../../../woml/tests/fixtures/remote-execution-targets/model.v21.reviewed.json");
const DEFINITION_HASH: &str =
  "sha256:2121212121212121212121212121212121212121212121212121212121212121";
const TARGET_DIGEST: &str =
  "sha256:d7f8a876583ef92e40aabaca6ed0a78f4189bbeaa374caeca1b5ed4ae7039f93";
const POLICY_DIGEST: &str =
  "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
const WORKER_POLICY_DIGEST: &str =
  "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";

struct TemporaryDatabase(PathBuf);

impl TemporaryDatabase {
  fn new() -> Self {
    Self(std::env::temp_dir().join(format!(
      "woml-worker-journal-{}.sqlite",
      Uuid::new_v4().simple()
    )))
  }

  fn path(&self) -> &Path {
    &self.0
  }
}

impl Drop for TemporaryDatabase {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.0);
    let _ = std::fs::remove_file(self.0.with_extension("sqlite-shm"));
    let _ = std::fs::remove_file(self.0.with_extension("sqlite-wal"));
  }
}

fn now() -> chrono::DateTime<Utc> {
  Utc.with_ymd_and_hms(2026, 10, 24, 12, 0, 0).unwrap()
}

fn coordinator() -> RemoteCoordinatorAuthorityV1 {
  RemoteCoordinatorAuthorityV1 {
    policy_digest: POLICY_DIGEST.to_string(),
    allowed_capabilities: vec![WorkerCapabilityV1 {
      id: "runtime.script".to_string(),
      version: 1,
    }],
    allowed_secret_names: vec![],
  }
}

fn authority() -> RemoteWorkerAuthorityV1 {
  RemoteWorkerAuthorityV1 {
    policy_digest: WORKER_POLICY_DIGEST.to_string(),
    allowed_capabilities: [("runtime.script".to_string(), 1)].into_iter().collect(),
    allowed_secret_names: BTreeSet::new(),
  }
}

fn package() -> RemoteDispatchPackageV1 {
  let workflow: CompiledWorkflowDefinition = serde_json::from_str(MODEL).unwrap();
  let node = workflow.node("applyRelease").unwrap().clone();
  let mut steps = Map::new();
  steps.insert("prepare".to_string(), json!({"release": "1.2.3"}));
  let mut package = RemoteDispatchPackageV1 {
    profile: "woml.dispatch-package/v1".to_string(),
    version: 1,
    dispatch_id: String::new(),
    run_id: "run_remote_worker_01".to_string(),
    node_id: "applyRelease".to_string(),
    iteration_key: None,
    attempt: 1,
    invocation_id: "inv_remote_worker_01".to_string(),
    target: "production-private".to_string(),
    target_registry_digest: TARGET_DIGEST.to_string(),
    infrastructure_policy_digest: Some(POLICY_DIGEST.to_string()),
    definition_package_root_hash: DEFINITION_HASH.to_string(),
    node,
    context: WorkflowContext {
      trigger: Map::new(),
      steps,
      lock: None,
    },
    required_capabilities: vec!["runtime.script".to_string()],
    required_secrets: vec![],
    artifact_grants: vec![],
    deadline: now() + Duration::minutes(5),
    package_digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
      .to_string(),
  };
  package.dispatch_id = derive_remote_dispatch_id(&RemoteDispatchRequestV1 {
    run_id: package.run_id.clone(),
    node_id: package.node_id.clone(),
    iteration_identity: String::new(),
    attempt: package.attempt,
    invocation_id: package.invocation_id.clone(),
    target: package.target.clone(),
    target_digest: package.target_registry_digest.clone(),
    package_digest: package.package_digest.clone(),
    requested_at: now(),
  })
  .unwrap();
  package.package_digest = package.computed_digest().unwrap();
  package
}

fn prepare(
  package: RemoteDispatchPackageV1,
) -> Result<woml_engine::PreparedRemoteDispatchV1, RemoteWorkerError> {
  prepare_remote_dispatch(
    package,
    &coordinator(),
    "production-runner-01",
    "production-private",
    &authority(),
    now(),
  )
}

#[test]
fn authority_is_intersected_and_persisted_before_duplicate_acceptance() {
  let prepared = prepare(package()).unwrap();
  assert_eq!(prepared.effective_authority.capabilities.len(), 1);
  assert!(!prepared.effective_authority.authority_digest.is_empty());
  let mut journal = WorkerJournalV1::open_in_memory().unwrap();
  let first = journal
    .accept(&prepared, "production-runner-01", "session_01", now())
    .unwrap();
  assert_eq!(first.state, WorkerAttemptStateV1::Accepted);
  assert_eq!(
    first.authority_digest,
    prepared.effective_authority.authority_digest
  );
  assert_eq!(
    journal
      .accept(&prepared, "production-runner-01", "session_02", now())
      .unwrap(),
    first
  );
  let mut changed = prepared.clone();
  changed.package.package_digest =
    "sha256:abababababababababababababababababababababababababababababababab".to_string();
  assert!(matches!(
    journal.accept(&changed, "production-runner-01", "session_03", now()),
    Err(RemoteWorkerError::Conflict)
  ));
}

#[test]
fn dispatch_package_decoder_is_strict_and_integrity_bound() {
  let package = package();
  let encoded = serde_json::to_vec(&package).unwrap();
  assert_eq!(decode_remote_dispatch_package(&encoded).unwrap(), package);

  let mut changed = package.clone();
  changed.deadline += Duration::seconds(1);
  assert!(decode_remote_dispatch_package(&serde_json::to_vec(&changed).unwrap()).is_err());

  let mut unknown = serde_json::to_value(package).unwrap();
  unknown
    .as_object_mut()
    .unwrap()
    .insert("unexpected".to_string(), json!(true));
  assert!(decode_remote_dispatch_package(&serde_json::to_vec(&unknown).unwrap()).is_err());
}

#[test]
fn output_batch_decoder_requires_integrity_and_strict_sequence() {
  let mut first = RemoteOperationalRecordV1 {
    sequence: 1,
    body: OperationalBodyV1::Output {
      source: OperationalSourceV1::Stdout,
      level: None,
      text: "one\n".to_string(),
      byte_count: 4,
    },
    record_digest: String::new(),
  };
  first.record_digest = first.computed_digest().unwrap();
  let mut second = first.clone();
  second.sequence = 2;
  second.record_digest = second.computed_digest().unwrap();
  let batch = WorkerOutputBatchV1 {
    profile: "woml.worker-protocol/v1".to_string(),
    protocol_version: 1,
    message_id: "output_01".to_string(),
    message_type: "output".to_string(),
    worker_id: "production-runner-01".to_string(),
    session_id: "session_01".to_string(),
    sent_at: now(),
    dispatch_id: format!("dispatch_{}", "a".repeat(64)),
    records: vec![first, second],
  };
  assert_eq!(
    decode_worker_output_batch(&serde_json::to_vec(&batch).unwrap()).unwrap(),
    batch
  );
  let mut reordered = batch;
  reordered.records[1].sequence = 3;
  reordered.records[1].record_digest = reordered.records[1].computed_digest().unwrap();
  assert!(decode_worker_output_batch(&serde_json::to_vec(&reordered).unwrap()).is_err());
}

#[test]
fn restart_preserves_accepted_work_and_marks_started_work_ambiguous() {
  let database = TemporaryDatabase::new();
  let prepared = prepare(package()).unwrap();
  {
    let mut journal = WorkerJournalV1::open(database.path()).unwrap();
    journal
      .accept(&prepared, "production-runner-01", "session_01", now())
      .unwrap();
    assert_eq!(
      journal.recover(now()).unwrap()[0].state,
      WorkerAttemptStateV1::Accepted
    );
    journal
      .start(&prepared.package.dispatch_id, "session_01", now())
      .unwrap();
  }
  let mut reopened = WorkerJournalV1::open(database.path()).unwrap();
  let recovered = reopened.recover(now() + Duration::seconds(1)).unwrap();
  assert_eq!(recovered[0].state, WorkerAttemptStateV1::Ambiguous);
}

#[test]
fn invalid_capabilities_secrets_grants_and_package_substitution_fail_before_acceptance() {
  let base = package();
  let mut unsupported = base.clone();
  unsupported
    .required_capabilities
    .push("services.unknown".to_string());
  unsupported
    .node
    .execution
    .as_mut()
    .unwrap()
    .required_capabilities = unsupported.required_capabilities.clone();
  unsupported.package_digest = unsupported.computed_digest().unwrap();
  assert!(matches!(
    prepare(unsupported),
    Err(RemoteWorkerError::CapabilityUnavailable)
  ));

  let mut secret = base.clone();
  secret.required_secrets = vec!["DEPLOY_TOKEN".to_string()];
  secret
    .node
    .script_runtime
    .as_mut()
    .unwrap()
    .required_secrets = secret.required_secrets.clone();
  secret.package_digest = secret.computed_digest().unwrap();
  let mut coordinator = coordinator();
  coordinator.allowed_secret_names = secret.required_secrets.clone();
  assert!(matches!(
    prepare_remote_dispatch(
      secret,
      &coordinator,
      "production-runner-01",
      "production-private",
      &authority(),
      now(),
    ),
    Err(RemoteWorkerError::SecretUnavailable)
  ));

  let mut expired_grant = base.clone();
  expired_grant.artifact_grants.push(RemoteArtifactGrantV1 {
    artifact_id: "artifact_01".to_string(),
    operation: RemoteArtifactOperationV1::Read,
    expires_at: now() - Duration::seconds(1),
    digest: DEFINITION_HASH.to_string(),
  });
  expired_grant.package_digest = expired_grant.computed_digest().unwrap();
  assert!(matches!(
    prepare(expired_grant),
    Err(RemoteWorkerError::InvalidPackage)
  ));

  let mut substitution = base;
  let woml_engine::ValueExpression::Object { fields } = &mut substitution.node.inputs else {
    panic!("source object");
  };
  fields.insert(
    "source".to_string(),
    woml_engine::ValueExpression::Literal {
      value: json!("return { substituted: true };"),
    },
  );
  assert!(matches!(
    prepare(substitution),
    Err(RemoteWorkerError::InvalidPackage)
  ));
}

#[test]
fn artifact_bytes_are_bound_to_grant_operation_worker_expiry_and_digest() {
  let bytes = b"reviewed artifact bytes";
  let grant = RemoteArtifactGrantV1 {
    artifact_id: "artifact_01".to_string(),
    operation: RemoteArtifactOperationV1::Read,
    expires_at: now() + Duration::minutes(1),
    digest: format!("sha256:{:x}", Sha256::digest(bytes)),
  };
  let mut package = package();
  package.artifact_grants.push(grant.clone());
  package.package_digest = package.computed_digest().unwrap();
  let prepared = prepare(package).unwrap();
  assert_eq!(
    verify_remote_artifact_transfer(
      &prepared,
      &grant,
      "production-runner-01",
      RemoteArtifactOperationV1::Read,
      bytes,
      bytes.len() as u64,
      now(),
    )
    .unwrap(),
    grant.digest
  );
  for (worker, payload, at) in [
    ("other-worker", bytes.as_slice(), now()),
    ("production-runner-01", b"corrupt".as_slice(), now()),
    (
      "production-runner-01",
      bytes.as_slice(),
      now() + Duration::minutes(2),
    ),
  ] {
    assert!(verify_remote_artifact_transfer(
      &prepared,
      &grant,
      worker,
      RemoteArtifactOperationV1::Read,
      payload,
      bytes.len() as u64,
      at,
    )
    .is_err());
  }
  let oversized = verify_remote_artifact_transfer(
    &prepared,
    &grant,
    "production-runner-01",
    RemoteArtifactOperationV1::Read,
    bytes,
    (bytes.len() - 1) as u64,
    now(),
  )
  .unwrap_err();
  assert_eq!(oversized.code(), "WOML_REMOTE_ARTIFACT_INTEGRITY_FAILED");
}

fn host_options() -> Option<ScriptHostProcessOptions> {
  let bun = std::process::Command::new("bun")
    .arg("--version")
    .output()
    .ok()?
    .status
    .success()
    .then_some(PathBuf::from("bun"))?;
  let host = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../woml-cli/src/script-host.ts");
  host
    .exists()
    .then(|| ScriptHostProcessOptions::new(bun, host))
}

#[tokio::test]
async fn graceful_drain_rejects_new_work_without_journaling_acceptance() {
  let database = TemporaryDatabase::new();
  let prepared = prepare(package()).unwrap();
  {
    let runtime_journal = WorkerJournalV1::open(database.path()).unwrap();
    let mut runtime = RemoteWorkerAttemptRuntimeV1::new(
      runtime_journal,
      "production-runner-01",
      "session_01",
      1,
      ScriptHostProcessOptions::new("unreachable-bun", "unreachable-script-host"),
      None,
      Arc::new(EnvironmentRemoteSecretProvider),
    )
    .unwrap();
    runtime.begin_drain();
    assert!(runtime.is_drained());
    assert!(matches!(
      runtime.execute(&prepared, now()).await,
      Err(RemoteWorkerError::Draining)
    ));
  }
  let mut reopened = WorkerJournalV1::open(database.path()).unwrap();
  assert!(reopened.recover(now()).unwrap().is_empty());
}

#[tokio::test]
async fn a_real_remote_step_matches_local_result_shape_and_terminal_redelivery_is_idempotent() {
  let Some(host) = host_options() else {
    return;
  };
  let prepared = prepare(package()).unwrap();
  let mut runtime = RemoteWorkerAttemptRuntimeV1::new(
    WorkerJournalV1::open_in_memory().unwrap(),
    "production-runner-01",
    "session_01",
    1,
    host,
    None,
    Arc::new(EnvironmentRemoteSecretProvider),
  )
  .unwrap();
  let first = runtime.execute(&prepared, now()).await.unwrap();
  assert_eq!(first.state, WorkerAttemptStateV1::Succeeded);
  assert_eq!(first.outcome, json!({"deployed": "1.2.3"}));
  let replay = runtime
    .execute(&prepared, now() + Duration::seconds(1))
    .await
    .unwrap();
  assert_eq!(replay, first);
  assert_eq!(
    runtime
      .journal()
      .terminal_for_resend(&prepared.package.dispatch_id)
      .unwrap(),
    Some(first)
  );
}

#[tokio::test]
async fn confirmed_cancellation_stops_script_work_and_records_a_cancelled_terminal() {
  let Some(host) = host_options() else {
    return;
  };
  let mut package = package();
  let woml_engine::ValueExpression::Object { fields } = &mut package.node.inputs else {
    panic!("source object");
  };
  fields.insert(
    "source".to_string(),
    woml_engine::ValueExpression::Literal {
      value: json!(
        "await new Promise(resolve => setTimeout(resolve, 10_000)); return { late: true };"
      ),
    },
  );
  package.package_digest = package.computed_digest().unwrap();
  let prepared = prepare(package).unwrap();
  let mut runtime = RemoteWorkerAttemptRuntimeV1::new(
    WorkerJournalV1::open_in_memory().unwrap(),
    "production-runner-01",
    "session_01",
    1,
    host,
    None,
    Arc::new(EnvironmentRemoteSecretProvider),
  )
  .unwrap();
  let cancellation = RemoteWorkerCancellationTokenV1::default();
  let request = cancellation.clone();
  let execution = tokio::spawn(async move {
    runtime
      .execute_with_cancellation(&prepared, now(), request)
      .await
  });
  tokio::time::sleep(std::time::Duration::from_millis(100)).await;
  cancellation.cancel();
  let terminal = tokio::time::timeout(std::time::Duration::from_secs(5), execution)
    .await
    .expect("cancellation settles within the confirmation window")
    .unwrap()
    .unwrap();
  assert_eq!(terminal.state, WorkerAttemptStateV1::Cancelled);
  assert_eq!(terminal.outcome["code"], "WOML_SCRIPT_CANCELLED");
}

struct StaticSecretProvider;

impl RemoteSecretProvider for StaticSecretProvider {
  fn resolve(&self, name: &str) -> Result<String, RemoteWorkerError> {
    (name == "DEPLOY_TOKEN")
      .then(|| "super-secret-token".to_string())
      .ok_or(RemoteWorkerError::SecretUnavailable)
  }
}

#[tokio::test]
async fn worker_local_secrets_are_redacted_before_terminal_results_leave_the_worker() {
  let Some(host) = host_options() else {
    return;
  };
  let mut package = package();
  package.required_secrets = vec!["DEPLOY_TOKEN".to_string()];
  package
    .node
    .script_runtime
    .as_mut()
    .unwrap()
    .required_secrets = package.required_secrets.clone();
  let woml_engine::ValueExpression::Object { fields } = &mut package.node.inputs else {
    panic!("source object");
  };
  fields.insert(
    "source".to_string(),
    woml_engine::ValueExpression::Literal {
      value: json!(
        "return { token: secrets.DEPLOY_TOKEN, message: `used:${secrets.DEPLOY_TOKEN}` };"
      ),
    },
  );
  package.package_digest = package.computed_digest().unwrap();
  let mut coordinator = coordinator();
  coordinator.allowed_secret_names = package.required_secrets.clone();
  let mut worker = authority();
  worker
    .allowed_secret_names
    .insert("DEPLOY_TOKEN".to_string());
  let prepared = prepare_remote_dispatch(
    package,
    &coordinator,
    "production-runner-01",
    "production-private",
    &worker,
    now(),
  )
  .unwrap();
  let mut runtime = RemoteWorkerAttemptRuntimeV1::new(
    WorkerJournalV1::open_in_memory().unwrap(),
    "production-runner-01",
    "session_01",
    1,
    host,
    None,
    Arc::new(StaticSecretProvider),
  )
  .unwrap();
  let terminal = runtime.execute(&prepared, now()).await.unwrap();
  assert_eq!(terminal.state, WorkerAttemptStateV1::Failed);
  assert_eq!(terminal.outcome["code"], "WOML_SCRIPT_NON_JSON_RESULT");
  assert_eq!(
    terminal.outcome["message"],
    "Script results must not contain a resolved secret value."
  );
  assert!(!serde_json::to_string(&terminal)
    .unwrap()
    .contains("super-secret-token"));
}
