use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};

use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};
use uuid::Uuid;
use woml_engine::{
  derive_operation_key, derive_workflow_call_key, step_effect_idempotency_key,
  CapabilityCallIdentity, CapabilityCallLimits, CapabilityCallRequest, CapabilityCancellationToken,
  CapabilityHandler, CompiledWorkflowDefinition, DurableDagEngine, DurableEventStore,
  DurableStoreError, ManagedWorkflowCallsHandler, RunIngress, RuntimeExecutionOptions,
  ScriptHostProcessOptions, TriggerAdmissionRequest, WebhookDefinitionRegistration,
  WomlWebhookServer, WomlWebhookServerConfig, WorkflowCallAdmissionRequest, WorkflowCallIndexState,
  WorkflowTargetRegistry, WorkflowTargetRegistryError, COMPILED_MODEL_SCHEMA_VERSION_V11,
  DURABLE_STORE_SCHEMA_VERSION, RUN_EVENT_SCHEMA_VERSION_V10, RUN_EVENT_SCHEMA_VERSION_V9,
};

const PARENT_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/workflow-calls/request-risk.compiled.v8.json");
const CHILD_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/workflow-calls/calculate-risk.compiled.v10.json");
const PARENT_HASH: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const CHILD_HASH: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const TRIGGERED_CHILD_HASH: &str =
  "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

fn parent_model() -> CompiledWorkflowDefinition {
  serde_json::from_str(PARENT_MODEL).unwrap()
}

fn child_model() -> CompiledWorkflowDefinition {
  serde_json::from_str(CHILD_MODEL).unwrap()
}

struct TemporaryDatabase {
  path: PathBuf,
}

impl TemporaryDatabase {
  fn new(label: &str) -> Self {
    Self {
      path: std::env::temp_dir().join(format!(
        "woml-wc2-{label}-{}.sqlite",
        Uuid::new_v4().simple()
      )),
    }
  }

  fn path(&self) -> &Path {
    &self.path
  }
}

impl Drop for TemporaryDatabase {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.path);
    let _ = std::fs::remove_file(format!("{}-wal", self.path.display()));
    let _ = std::fs::remove_file(format!("{}-shm", self.path.display()));
  }
}

fn seed_parent(path: &Path) -> (String, DateTime<Utc>) {
  let now = Utc::now();
  let mut store = DurableEventStore::open(path).unwrap();
  store
    .register_definition(&parent_model(), PARENT_HASH)
    .unwrap();
  store
    .register_definition(&child_model(), CHILD_HASH)
    .unwrap();
  let accepted = store
    .admit_trigger_occurrence(TriggerAdmissionRequest {
      workflow_id: "request-risk".to_string(),
      definition_hash: PARENT_HASH.to_string(),
      trigger_id: "start".to_string(),
      trigger_handler: "trigger.manual".to_string(),
      source_identity: format!("wc2:{}", Uuid::new_v4().simple()),
      payload: Map::new(),
      received_at: now,
    })
    .unwrap();
  let mut engine = DurableDagEngine::resume(store, &accepted.run_id).unwrap();
  engine
    .start_step_attempt(&accepted.run_id, "requestRisk", 1, "inv_wc2_parent", now)
    .unwrap();
  drop(engine.into_store());
  (accepted.run_id, now)
}

fn admission(
  parent_run_id: &str,
  payload: Map<String, Value>,
  at: DateTime<Utc>,
) -> WorkflowCallAdmissionRequest {
  let call_key = derive_workflow_call_key(
    "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    "calculate-risk",
    "call1",
  );
  WorkflowCallAdmissionRequest {
    child_run_id: format!("run_call_{}", call_key.strip_prefix("sha256:").unwrap()),
    call_key,
    parent_run_id: parent_run_id.to_string(),
    parent_node_id: "requestRisk".to_string(),
    parent_attempt: 1,
    target_workflow_id: "calculate-risk".to_string(),
    target_definition_hash: CHILD_HASH.to_string(),
    payload,
    admitted_at: at,
  }
}

#[test]
fn target_registry_is_exact_immutable_and_rejects_duplicate_owners() {
  let registry = WorkflowTargetRegistry::new("runtime_wc2").unwrap();
  let target = registry.register(&child_model(), CHILD_HASH).unwrap();
  assert_eq!(target.workflow_id, "calculate-risk");
  assert_eq!(target.definition_hash, CHILD_HASH);
  assert_eq!(
    registry.register(&child_model(), CHILD_HASH),
    Err(WorkflowTargetRegistryError::DuplicateWorkflowId(
      "calculate-risk".to_string()
    ))
  );
  assert_eq!(
    registry.resolve("calculate-risk"),
    Err(WorkflowTargetRegistryError::RegistryNotSealed)
  );
  registry.seal();
  assert_eq!(registry.resolve("calculate-risk").unwrap(), target);
  assert_eq!(
    registry.register(&parent_model(), PARENT_HASH),
    Err(WorkflowTargetRegistryError::RegistrySealed)
  );
}

#[test]
fn run_call_inspection_is_bounded_and_excludes_internal_identity_data() {
  let database = TemporaryDatabase::new("bounded-inspection");
  let (parent_run_id, now) = seed_parent(database.path());
  let mut store = DurableEventStore::open(database.path()).unwrap();
  let mut first_child_run_id = String::new();
  for index in 1_u64..=51 {
    let call_key = format!("sha256:{index:064x}");
    let child_run_id = format!("run_call_{index:064x}");
    if index == 1 {
      first_child_run_id.clone_from(&child_run_id);
    }
    store
      .admit_workflow_call(WorkflowCallAdmissionRequest {
        call_key,
        child_run_id,
        parent_run_id: parent_run_id.clone(),
        parent_node_id: "requestRisk".to_string(),
        parent_attempt: 1,
        target_workflow_id: "calculate-risk".to_string(),
        target_definition_hash: CHILD_HASH.to_string(),
        payload: Map::from_iter([("index".to_string(), json!(index))]),
        admitted_at: now + chrono::Duration::milliseconds(index as i64),
      })
      .unwrap();
  }

  let parent_relations = store
    .workflow_call_relations_for_run(&parent_run_id)
    .unwrap();
  assert!(parent_relations.parent_call.is_none());
  assert_eq!(parent_relations.child_calls.len(), 50);
  assert!(parent_relations.child_calls_truncated);
  let encoded = serde_json::to_string(&parent_relations).unwrap();
  for forbidden in [
    "callKey",
    "payload",
    "payloadDigest",
    "definitionHash",
    "customer-42",
  ] {
    assert!(!encoded.contains(forbidden));
  }

  let child_relations = store
    .workflow_call_relations_for_run(&first_child_run_id)
    .unwrap();
  assert_eq!(
    child_relations.parent_call.unwrap().parent_run_id,
    parent_run_id
  );
  assert!(child_relations.child_calls.is_empty());
  assert!(!child_relations.child_calls_truncated);
}

#[test]
fn run_call_inspection_is_empty_for_an_ordinary_run() {
  let database = TemporaryDatabase::new("empty-inspection");
  let (run_id, _) = seed_parent(database.path());
  let relations = DurableEventStore::open(database.path())
    .unwrap()
    .workflow_call_relations_for_run(&run_id)
    .unwrap();
  assert!(relations.parent_call.is_none());
  assert!(relations.child_calls.is_empty());
  assert!(!relations.child_calls_truncated);
}

#[test]
fn admission_creates_one_truthfully_bound_child_and_reuses_it() {
  let database = TemporaryDatabase::new("admission");
  let (parent_run_id, at) = seed_parent(database.path());
  let request = admission(
    &parent_run_id,
    Map::from_iter([("customerId".to_string(), json!("customer-42"))]),
    at,
  );
  let mut store = DurableEventStore::open(database.path()).unwrap();
  assert_eq!(DURABLE_STORE_SCHEMA_VERSION, 14);

  let first = store.admit_workflow_call(request.clone()).unwrap();
  assert!(!first.duplicate);
  assert_eq!(first.admission.state, WorkflowCallIndexState::Admitted);
  assert_eq!(first.admission.depth, 1);

  let duplicate = store.admit_workflow_call(request.clone()).unwrap();
  assert!(duplicate.duplicate);
  assert_eq!(
    duplicate.admission.child_run_id,
    first.admission.child_run_id
  );

  let child_events = store.events(&first.admission.child_run_id).unwrap();
  assert_eq!(child_events.len(), 1);
  assert_eq!(
    child_events[0].event_schema_version,
    RUN_EVENT_SCHEMA_VERSION_V9
  );
  let projection = store.projection(&first.admission.child_run_id).unwrap();
  assert_eq!(projection.context.trigger, request.payload);
  assert!(projection.context.steps.is_empty());
  assert_eq!(
    projection.ingress,
    Some(RunIngress::WorkflowCall {
      call_key: request.call_key.clone()
    })
  );
  assert_eq!(
    store.workflow_call(&request.call_key).unwrap().unwrap(),
    first.admission
  );
}

#[test]
fn an_existing_triggered_definition_is_also_callable_without_faking_its_trigger() {
  let database = TemporaryDatabase::new("triggered-target");
  let (parent_run_id, at) = seed_parent(database.path());
  let mut triggered_child = parent_model();
  triggered_child.schema_version = COMPILED_MODEL_SCHEMA_VERSION_V11;
  triggered_child.workflow_id = "triggered-worker".to_string();
  let mut store = DurableEventStore::open(database.path()).unwrap();
  store
    .register_definition(&triggered_child, TRIGGERED_CHILD_HASH)
    .unwrap();

  let call_key = derive_workflow_call_key(
    "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    "triggered-worker",
    "triggered-call",
  );
  let request = WorkflowCallAdmissionRequest {
    child_run_id: format!("run_call_{}", call_key.strip_prefix("sha256:").unwrap()),
    call_key: call_key.clone(),
    parent_run_id,
    parent_node_id: "requestRisk".to_string(),
    parent_attempt: 1,
    target_workflow_id: "triggered-worker".to_string(),
    target_definition_hash: TRIGGERED_CHILD_HASH.to_string(),
    payload: Map::from_iter([("jobId".to_string(), json!("job-42"))]),
    admitted_at: at,
  };
  let outcome = store.admit_workflow_call(request.clone()).unwrap();
  let events = store.events(&outcome.admission.child_run_id).unwrap();
  assert_eq!(events[0].event_schema_version, RUN_EVENT_SCHEMA_VERSION_V10);
  let projection = store.projection(&outcome.admission.child_run_id).unwrap();
  assert_eq!(projection.context.trigger, request.payload);
  assert_eq!(
    projection.ingress,
    Some(RunIngress::WorkflowCall { call_key })
  );
}

#[test]
fn conflicting_payload_or_definition_fails_without_an_extra_child() {
  let database = TemporaryDatabase::new("conflict");
  let (parent_run_id, at) = seed_parent(database.path());
  let request = admission(
    &parent_run_id,
    Map::from_iter([("customerId".to_string(), json!("customer-42"))]),
    at,
  );
  let mut store = DurableEventStore::open(database.path()).unwrap();
  let admitted = store.admit_workflow_call(request.clone()).unwrap();

  let mut conflict = request.clone();
  conflict
    .payload
    .insert("customerId".to_string(), json!("other"));
  assert!(matches!(
    store.admit_workflow_call(conflict),
    Err(DurableStoreError::WorkflowCallIdempotencyConflict)
  ));
  assert_eq!(
    store
      .events(&admitted.admission.child_run_id)
      .unwrap()
      .len(),
    1
  );

  let different_key = derive_workflow_call_key(
    "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    "calculate-risk",
    "call2",
  );
  let mut mismatch = request;
  mismatch.call_key = different_key.clone();
  mismatch.child_run_id = format!(
    "run_call_{}",
    different_key.strip_prefix("sha256:").unwrap()
  );
  mismatch.target_definition_hash = PARENT_HASH.to_string();
  assert!(matches!(
    store.admit_workflow_call(mismatch.clone()),
    Err(DurableStoreError::WorkflowCallDefinitionMismatch)
  ));
  assert!(matches!(
    store.run_binding(&mismatch.child_run_id),
    Err(DurableStoreError::RunNotFound(_))
  ));
}

#[test]
fn concurrent_duplicate_admission_still_creates_exactly_one_child() {
  let database = TemporaryDatabase::new("concurrent");
  let (parent_run_id, at) = seed_parent(database.path());
  let request = admission(
    &parent_run_id,
    Map::from_iter([("customerId".to_string(), json!("customer-42"))]),
    at,
  );
  let barrier = Arc::new(Barrier::new(8));
  let mut workers = Vec::new();
  for _ in 0..8 {
    let barrier = barrier.clone();
    let path = database.path().to_path_buf();
    let request = request.clone();
    workers.push(std::thread::spawn(move || {
      let mut store = DurableEventStore::open(path).unwrap();
      barrier.wait();
      store.admit_workflow_call(request).unwrap()
    }));
  }
  let outcomes = workers
    .into_iter()
    .map(|worker| worker.join().unwrap())
    .collect::<Vec<_>>();
  assert_eq!(
    outcomes.iter().filter(|outcome| !outcome.duplicate).count(),
    1
  );
  let child_ids = outcomes
    .iter()
    .map(|outcome| outcome.admission.child_run_id.as_str())
    .collect::<std::collections::BTreeSet<_>>();
  assert_eq!(child_ids.len(), 1);

  let connection = rusqlite::Connection::open(database.path()).unwrap();
  let child_count: i64 = connection
    .query_row(
      "SELECT COUNT(*) FROM woml_runs WHERE run_id LIKE 'run_call_%'",
      [],
      |row| row.get(0),
    )
    .unwrap();
  assert_eq!(child_count, 1);
}

#[tokio::test]
async fn managed_handler_routes_verified_identity_into_durable_admission() {
  let database = TemporaryDatabase::new("handler");
  let (parent_run_id, _) = seed_parent(database.path());
  let targets = Arc::new(WorkflowTargetRegistry::new("runtime_wc2").unwrap());
  targets.register(&child_model(), CHILD_HASH).unwrap();
  targets.seal();
  let handler = ManagedWorkflowCallsHandler::new(database.path().to_path_buf(), targets);
  let step_key = step_effect_idempotency_key(&parent_run_id, PARENT_HASH, "requestRisk");
  let operation_name = "workflows.call.customer-risk".to_string();
  let request = CapabilityCallRequest {
    contract: "woml.capability-call".to_string(),
    contract_version: 1,
    message_type: "request".to_string(),
    invocation_id: "inv_wc2_parent".to_string(),
    call_id: "call_1".to_string(),
    run_id: parent_run_id,
    node_id: "requestRisk".to_string(),
    attempt_number: 1,
    capability: "workflows".to_string(),
    operation: "call".to_string(),
    input_contract_version: 1,
    result_contract_version: 1,
    identity: CapabilityCallIdentity {
      mode: woml_engine::capability::CapabilityIdentityMode::Named,
      operation_key: derive_operation_key(&step_key, &operation_name),
      step_idempotency_key: step_key,
      operation_name,
      provider_idempotency_key: None,
    },
    limits: CapabilityCallLimits::default(),
    input: json!({
      "contract": "woml.workflow-call",
      "contractVersion": 1,
      "kind": "request",
      "workflowId": "calculate-risk",
      "payload": { "customerId": "customer-42" },
      "options": { "name": "customer-risk", "timeoutMs": 30_000 }
    }),
  };
  request.validate().unwrap();
  handler.validate_request(&request).unwrap();
  let metadata = handler.safe_request_metadata(&request).unwrap();
  assert_eq!(metadata["targetWorkflowId"], "calculate-risk");
  assert_eq!(metadata["targetDefinitionHash"], CHILD_HASH);

  let result = handler
    .execute_request_scoped(&request, None, CapabilityCancellationToken::default())
    .await
    .unwrap();
  assert_eq!(result["contract"], "woml.workflow-call-admission");
  assert_eq!(result["kind"], "admitted");
  assert_eq!(result["data"]["duplicate"], false);
}

#[actix_web::test]
async fn long_lived_runtime_registers_all_targets_before_any_run_starts() {
  let database = TemporaryDatabase::new("runtime-registration");
  let execution = RuntimeExecutionOptions::new(
    ScriptHostProcessOptions::new("bun", "unused-wc2-script-host.ts"),
    2_000,
  );
  let server = WomlWebhookServer::start(WomlWebhookServerConfig {
    bind_address: "127.0.0.1:0".parse().unwrap(),
    database_path: database.path().to_path_buf(),
    registrations: vec![
      WebhookDefinitionRegistration::new(parent_model(), PARENT_HASH),
      WebhookDefinitionRegistration::new(child_model(), CHILD_HASH),
    ],
    startup_manual_triggers: BTreeMap::new(),
    execution,
    progress_reporter: None,
  })
  .await
  .unwrap();
  let store = DurableEventStore::open(database.path()).unwrap();
  assert_eq!(
    store.definition(PARENT_HASH).unwrap().workflow_id,
    "request-risk"
  );
  assert_eq!(
    store.definition(CHILD_HASH).unwrap().workflow_id,
    "calculate-risk"
  );
  assert!(matches!(
    store.run_binding("run_not_started"),
    Err(DurableStoreError::RunNotFound(_))
  ));
  server.stop().await;
}
