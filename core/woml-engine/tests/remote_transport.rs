use std::path::{Path, PathBuf};

use chrono::{Duration, Utc};
use serde_json::{json, Map};
use uuid::Uuid;
use woml_engine::{
  derive_remote_dispatch_id, CompiledWorkflowDefinition, DurableEventStore,
  RemoteCoordinatorAuthorityV1, RemoteDispatchPackageV1, RemoteDispatchRequestV1,
  RemoteDispatchStateV1, RemoteDispatchTransportV1, TriggerAdmissionRequest, WorkerCapabilityV1,
  WorkerDispatchFactV1, WorkerTerminalOutcomeV1, WorkerTerminalV1, WORKER_PROTOCOL_PROFILE,
  WORKER_PROTOCOL_VERSION,
};

const MODEL: &str =
  include_str!("../../../woml/tests/fixtures/remote-execution-targets/model.v21.reviewed.json");
const DEFINITION_HASH: &str =
  "sha256:2121212121212121212121212121212121212121212121212121212121212121";
const TARGET_DIGEST: &str =
  "sha256:d7f8a876583ef92e40aabaca6ed0a78f4189bbeaa374caeca1b5ed4ae7039f93";
const POLICY_DIGEST: &str =
  "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct TemporaryDatabase(PathBuf);

impl TemporaryDatabase {
  fn new(label: &str) -> Self {
    Self(std::env::temp_dir().join(format!(
      "woml-remote-transport-{label}-{}.sqlite",
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

fn setup(label: &str) -> (TemporaryDatabase, String, CompiledWorkflowDefinition) {
  let database = TemporaryDatabase::new(label);
  let workflow: CompiledWorkflowDefinition = serde_json::from_str(MODEL).unwrap();
  let mut store = DurableEventStore::open(database.path()).unwrap();
  store
    .register_definition(&workflow, DEFINITION_HASH)
    .unwrap();
  let run_id = store
    .admit_trigger_occurrence(TriggerAdmissionRequest {
      workflow_id: workflow.workflow_id.clone(),
      definition_hash: DEFINITION_HASH.to_string(),
      trigger_id: "start".to_string(),
      trigger_handler: "trigger.manual".to_string(),
      source_identity: "remote-transport-test".to_string(),
      payload: Map::new(),
      received_at: Utc::now(),
    })
    .unwrap()
    .run_id;
  (database, run_id, workflow)
}

fn package(run_id: &str, workflow: &CompiledWorkflowDefinition) -> RemoteDispatchPackageV1 {
  let now = Utc::now();
  let mut package = RemoteDispatchPackageV1 {
    profile: "woml.dispatch-package/v1".to_string(),
    version: 1,
    dispatch_id: String::new(),
    run_id: run_id.to_string(),
    node_id: "applyRelease".to_string(),
    iteration_key: None,
    attempt: 1,
    invocation_id: format!("inv_{}", Uuid::new_v4().simple()),
    target: "production-private".to_string(),
    target_registry_digest: TARGET_DIGEST.to_string(),
    infrastructure_policy_digest: Some(POLICY_DIGEST.to_string()),
    definition_package_root_hash: DEFINITION_HASH.to_string(),
    node: workflow.node("applyRelease").unwrap().clone(),
    context: woml_engine::WorkflowContext {
      trigger: Map::new(),
      steps: Map::new(),
      lock: None,
    },
    required_capabilities: vec!["runtime.script".to_string()],
    required_secrets: vec![],
    artifact_grants: vec![],
    deadline: now + Duration::minutes(5),
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
    requested_at: now,
  })
  .unwrap();
  package.package_digest = package.computed_digest().unwrap();
  package
}

fn authority() -> RemoteCoordinatorAuthorityV1 {
  RemoteCoordinatorAuthorityV1 {
    policy_digest: POLICY_DIGEST.to_string(),
    allowed_capabilities: vec![WorkerCapabilityV1 {
      id: "runtime.script".to_string(),
      version: 1,
    }],
    allowed_secret_names: vec![],
  }
}

fn fact(kind: &str, dispatch_id: &str, session_id: &str) -> WorkerDispatchFactV1 {
  WorkerDispatchFactV1 {
    profile: WORKER_PROTOCOL_PROFILE.to_string(),
    protocol_version: WORKER_PROTOCOL_VERSION,
    message_id: format!("message_{kind}_{}", Uuid::new_v4().simple()),
    message_type: kind.to_string(),
    worker_id: "production-runner-01".to_string(),
    session_id: session_id.to_string(),
    sent_at: Utc::now(),
    target: "production-private".to_string(),
    dispatch_id: dispatch_id.to_string(),
  }
}

#[test]
fn packaged_transport_commits_offer_accept_start_and_terminal_in_order() {
  let (database, run_id, workflow) = setup("success");
  let transport = RemoteDispatchTransportV1::new(database.path());
  let package = package(&run_id, &workflow);
  let dispatch_id = transport
    .enqueue(package.clone(), authority(), Utc::now())
    .unwrap();
  let offer = transport
    .offer_next(
      "production-runner-01",
      "session_transport_01",
      "production-private",
      Utc::now(),
    )
    .unwrap()
    .unwrap();
  assert_eq!(offer.dispatch.dispatch_id, dispatch_id);
  transport
    .accept(&fact("accepted", &dispatch_id, "session_transport_01"))
    .unwrap();
  transport
    .start(&fact("started", &dispatch_id, "session_transport_01"))
    .unwrap();
  let terminal = WorkerTerminalV1 {
    profile: WORKER_PROTOCOL_PROFILE.to_string(),
    protocol_version: WORKER_PROTOCOL_VERSION,
    message_id: format!("message_terminal_{}", Uuid::new_v4().simple()),
    message_type: "terminal".to_string(),
    worker_id: "production-runner-01".to_string(),
    session_id: "session_transport_01".to_string(),
    sent_at: Utc::now(),
    target: "production-private".to_string(),
    dispatch_id: dispatch_id.clone(),
    outcome: WorkerTerminalOutcomeV1::Succeeded,
    result: Some(json!({"applied": true})),
    failure: None,
    final_output_sequence: 3,
    journal_digest: "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
      .to_string(),
  };
  let receipt = transport.terminal(terminal.clone()).unwrap();
  assert_eq!(receipt.state, RemoteDispatchStateV1::Succeeded);
  assert_eq!(transport.terminal(terminal.clone()).unwrap(), receipt);
  assert_eq!(
    transport.terminal_fact(&dispatch_id).unwrap(),
    Some(terminal)
  );
  assert_eq!(
    DurableEventStore::open(database.path())
      .unwrap()
      .remote_dispatch_record(&dispatch_id)
      .unwrap()
      .state,
    RemoteDispatchStateV1::Succeeded
  );
}

#[test]
fn loss_before_acceptance_requeues_but_loss_after_acceptance_is_ambiguous() {
  let (database, run_id, workflow) = setup("loss");
  let transport = RemoteDispatchTransportV1::new(database.path());
  let first = package(&run_id, &workflow);
  let first_id = transport.enqueue(first, authority(), Utc::now()).unwrap();
  transport
    .offer_next(
      "production-runner-01",
      "session_loss_01",
      "production-private",
      Utc::now(),
    )
    .unwrap()
    .unwrap();
  transport
    .mark_lost(
      Utc::now() + Duration::minutes(1),
      Utc::now() + Duration::minutes(1),
    )
    .unwrap();
  assert_eq!(
    transport
      .offer_next(
        "production-runner-01",
        "session_loss_02",
        "production-private",
        Utc::now(),
      )
      .unwrap()
      .unwrap()
      .dispatch
      .dispatch_id,
    first_id
  );
  transport
    .accept(&fact("accepted", &first_id, "session_loss_02"))
    .unwrap();
  transport
    .mark_lost(
      Utc::now() + Duration::minutes(1),
      Utc::now() + Duration::minutes(1),
    )
    .unwrap();
  assert_eq!(
    DurableEventStore::open(database.path())
      .unwrap()
      .remote_dispatch_record(&first_id)
      .unwrap()
      .state,
    RemoteDispatchStateV1::Ambiguous
  );
  assert!(transport
    .offer_next(
      "production-runner-01",
      "session_loss_03",
      "production-private",
      Utc::now(),
    )
    .unwrap()
    .is_none());
}

#[test]
fn coordinator_restart_rehydrates_only_proven_prestart_work() {
  let (database, run_id, workflow) = setup("restart-safe");
  let now = Utc::now();
  let package = package(&run_id, &workflow);
  let coordinator = authority();
  let first = RemoteDispatchTransportV1::new(database.path());
  first
    .enqueue(package.clone(), coordinator.clone(), now)
    .unwrap();
  first
    .offer_next(
      "production-runner-01",
      "session_before_crash",
      "production-private",
      now,
    )
    .unwrap()
    .unwrap();
  drop(first);

  let restarted = RemoteDispatchTransportV1::new(database.path());
  restarted
    .enqueue(package, coordinator, now + Duration::seconds(1))
    .unwrap();
  assert!(restarted
    .offer_next(
      "production-runner-01",
      "session_after_crash",
      "production-private",
      now + Duration::seconds(1),
    )
    .unwrap()
    .is_none());
  restarted
    .mark_lost(now + Duration::minutes(1), now + Duration::minutes(1))
    .unwrap();
  assert!(restarted
    .offer_next(
      "production-runner-01",
      "session_after_crash",
      "production-private",
      now + Duration::minutes(1),
    )
    .unwrap()
    .is_some());
}

#[test]
fn coordinator_restart_never_requeues_started_work() {
  let (database, run_id, workflow) = setup("restart-ambiguous");
  let now = Utc::now();
  let started_package = package(&run_id, &workflow);
  let coordinator = authority();
  let transport = RemoteDispatchTransportV1::new(database.path());
  let started_id = transport
    .enqueue(started_package.clone(), coordinator.clone(), now)
    .unwrap();
  let offer = transport
    .offer_next(
      "production-runner-01",
      "session_started",
      "production-private",
      now,
    )
    .unwrap()
    .unwrap();
  let accepted = fact("accepted", &started_id, "session_started");
  transport.accept(&accepted).unwrap();
  let started = fact("started", &started_id, "session_started");
  transport.start(&started).unwrap();
  assert_eq!(offer.dispatch.dispatch_id, started_id);
  drop(transport);

  let restarted_again = RemoteDispatchTransportV1::new(database.path());
  restarted_again
    .enqueue(started_package, coordinator, now + Duration::seconds(1))
    .unwrap();
  restarted_again
    .mark_lost(now + Duration::minutes(2), now + Duration::minutes(2))
    .unwrap();
  assert_eq!(
    DurableEventStore::open(database.path())
      .unwrap()
      .remote_dispatch_record(&started_id)
      .unwrap()
      .state,
    RemoteDispatchStateV1::Ambiguous
  );
  assert!(restarted_again
    .offer_next(
      "production-runner-01",
      "session_forbidden_retry",
      "production-private",
      now + Duration::minutes(2),
    )
    .unwrap()
    .is_none());
}
