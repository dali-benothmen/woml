use std::sync::{
  atomic::{AtomicUsize, Ordering},
  Arc,
};

mod state_policy;

use chrono::Utc;
use futures_util::future::BoxFuture;
use serde_json::{json, Map, Value};
use tokio::sync::Mutex;
use woml_engine::event::StepAttemptStartedData;
use woml_engine::{
  create_online_backup, derive_operation_key, prepare_emergency_deny_overlay,
  prepare_infrastructure_policy, prepare_restored_store, step_effect_idempotency_key,
  CapabilityCallIdentity, CapabilityCallLimits, CapabilityCallRequest, CapabilityCallResult,
  CapabilityCancellationToken, CapabilityDescriptor, CapabilityEffect, CapabilityFailure,
  CapabilityFailureKind, CapabilityHandler, CapabilityRegistry, CompiledWorkflowDefinition,
  DurableCapabilityAuthority, DurableEventStore, InfrastructurePolicyBundleV1,
  InfrastructurePolicyTrustV1, ManagedLocksService, NativeFetchInvocationIdentity,
  NativeFetchObservation, RunEventPayload, RunStartedData,
};

const MODEL: &str = include_str!("../../../woml/tests/fixtures/services-bindings.compiled.v8.json");
const DEFINITION_HASH: &str =
  "sha256:1111111111111111111111111111111111111111111111111111111111111111";

#[derive(Debug)]
struct CountingProcessHandler {
  calls: Arc<AtomicUsize>,
  fail_after_dispatch: bool,
}

impl CapabilityHandler for CountingProcessHandler {
  fn descriptor(&self) -> CapabilityDescriptor {
    CapabilityDescriptor {
      capability: "process".to_string(),
      operation: "run".to_string(),
      input_contract_version: 1,
      result_contract_version: 1,
      effect: CapabilityEffect::UnsafeWrite,
      supports_cancellation: true,
      supports_provider_idempotency: false,
    }
  }

  fn execute(
    &self,
    _input: Value,
    _cancellation: CapabilityCancellationToken,
  ) -> BoxFuture<'static, Result<Value, CapabilityFailure>> {
    self.calls.fetch_add(1, Ordering::SeqCst);
    let fail = self.fail_after_dispatch;
    Box::pin(async move {
      if fail {
        Err(CapabilityFailure {
          kind: CapabilityFailureKind::ServiceRejected,
          code: "WOML_PROCESS_POLICY_DENIED".to_string(),
          message: "The lower-level process policy denied execution.".to_string(),
          retryable: false,
          ambiguous: false,
          details: None,
        })
      } else {
        Ok(json!({"exitCode": 0}))
      }
    })
  }
}

fn model() -> CompiledWorkflowDefinition {
  let mut model: CompiledWorkflowDefinition = serde_json::from_str(MODEL).unwrap();
  model.graph.nodes.retain(|node| node.id == "loadCustomer");
  model.graph.edges.clear();
  model.validate_structure().unwrap();
  model
}

fn active_store(run_id: &str, invocation_id: &str) -> DurableEventStore {
  initialize_active_store(
    DurableEventStore::open_in_memory().unwrap(),
    run_id,
    invocation_id,
  )
}

fn initialize_active_store(
  mut store: DurableEventStore,
  run_id: &str,
  invocation_id: &str,
) -> DurableEventStore {
  let model = model();
  store.register_definition(&model, DEFINITION_HASH).unwrap();
  store
    .append_payload(
      run_id,
      format!("event-{run_id}-start"),
      Utc::now(),
      RunEventPayload::RunStarted(RunStartedData {
        workflow_id: model.workflow_id,
        definition_hash: DEFINITION_HASH.to_string(),
        trigger_id: Some("start".to_string()),
        trigger_handler: Some("trigger.manual".to_string()),
        trigger_occurrence_id: Some(format!("occurrence-{run_id}")),
        ingress: None,
        trigger: Map::new(),
      }),
    )
    .unwrap();
  store
    .append_payload(
      run_id,
      format!("event-{run_id}-attempt"),
      Utc::now(),
      RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
        node_id: "loadCustomer".to_string(),
        attempt: 1,
        invocation_id: invocation_id.to_string(),
        handler: "runtime.script".to_string(),
        idempotency_key: Some(step_effect_idempotency_key(
          run_id,
          DEFINITION_HASH,
          "loadCustomer",
        )),
      }),
    )
    .unwrap();
  store
}

fn policy(default: &str, version: &str) -> woml_engine::PreparedInfrastructurePolicyV1 {
  let bundle: InfrastructurePolicyBundleV1 = serde_json::from_value(json!({
    "contract": "woml.infrastructure-policy",
    "contractVersion": 1,
    "id": "production",
    "version": version,
    "environment": "production",
    "defaults": {
      "read": default,
      "idempotent-write": default,
      "unsafe-write": default
    },
    "rules": []
  }))
  .unwrap();
  prepare_infrastructure_policy(bundle, &InfrastructurePolicyTrustV1::default()).unwrap()
}

fn policy_requiring_verified_artifact() -> woml_engine::PreparedInfrastructurePolicyV1 {
  let bundle: InfrastructurePolicyBundleV1 = serde_json::from_value(json!({
    "contract": "woml.infrastructure-policy",
    "contractVersion": 1,
    "id": "reviewed-production",
    "version": "1",
    "environment": "production",
    "defaults": {
      "read": "deny",
      "idempotent-write": "deny",
      "unsafe-write": "deny"
    },
    "rules": [{
      "id": "reviewed-terraform",
      "match": {
        "capabilities": ["process"],
        "operations": ["run"],
        "effects": ["unsafe-write"],
        "executables": ["terraform"]
      },
      "decision": "allow",
      "requires": {
        "artifacts": {
          "minimum": 1,
          "verifiedIntegrity": true,
          "bindReviewDigest": false
        }
      }
    }]
  }))
  .unwrap();
  prepare_infrastructure_policy(bundle, &InfrastructurePolicyTrustV1::default()).unwrap()
}

fn emergency_deny() -> woml_engine::PreparedInfrastructurePolicyV1 {
  let bundle: InfrastructurePolicyBundleV1 = serde_json::from_value(json!({
    "contract": "woml.infrastructure-policy",
    "contractVersion": 1,
    "id": "incident-stop",
    "version": "1",
    "environment": "production",
    "defaults": {
      "read": "allow",
      "idempotent-write": "allow",
      "unsafe-write": "allow"
    },
    "rules": [{
      "id": "stop-process",
      "match": { "capabilities": ["process"], "operations": ["run"] },
      "decision": "deny"
    }]
  }))
  .unwrap();
  prepare_emergency_deny_overlay(bundle, &InfrastructurePolicyTrustV1::default()).unwrap()
}

fn request(run_id: &str, invocation_id: &str, call_id: &str) -> CapabilityCallRequest {
  let step_key = step_effect_idempotency_key(run_id, DEFINITION_HASH, "loadCustomer");
  let operation_name = format!("process.run.{call_id}");
  CapabilityCallRequest {
    contract: "woml.capability-call".to_string(),
    contract_version: 1,
    message_type: "request".to_string(),
    invocation_id: invocation_id.to_string(),
    call_id: call_id.to_string(),
    run_id: run_id.to_string(),
    node_id: "loadCustomer".to_string(),
    attempt_number: 1,
    capability: "process".to_string(),
    operation: "run".to_string(),
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
    input: json!({"executable": "terraform", "args": ["apply"], "cwd": "."}),
  }
}

async fn authority(
  store: DurableEventStore,
  handler: CountingProcessHandler,
) -> (Arc<Mutex<DurableEventStore>>, DurableCapabilityAuthority) {
  let store = Arc::new(Mutex::new(store));
  let registry = Arc::new(CapabilityRegistry::default());
  registry.register(Arc::new(handler)).unwrap();
  let authority = DurableCapabilityAuthority::new_with_locks_and_infrastructure_policy(
    registry,
    Arc::clone(&store),
    Arc::new(ManagedLocksService::default()),
    true,
  );
  (store, authority)
}

async fn authority_with_emergency_deny(
  store: DurableEventStore,
  handler: CountingProcessHandler,
) -> (Arc<Mutex<DurableEventStore>>, DurableCapabilityAuthority) {
  let store = Arc::new(Mutex::new(store));
  let registry = Arc::new(CapabilityRegistry::default());
  registry.register(Arc::new(handler)).unwrap();
  let authority = DurableCapabilityAuthority::new_with_infrastructure_policy_authorities(
    registry,
    Arc::clone(&store),
    Arc::new(ManagedLocksService::default()),
    true,
    None,
    Some(Arc::new(emergency_deny())),
  );
  (store, authority)
}

#[test]
fn an_emergency_overlay_is_structurally_unable_to_allow() {
  let mut bundle = emergency_deny().bundle;
  bundle.rules[0].decision = woml_engine::InfrastructurePolicyDecisionV1::Allow;
  assert!(
    prepare_emergency_deny_overlay(bundle, &InfrastructurePolicyTrustV1::default())
      .unwrap_err()
      .to_string()
      .contains("deny rules")
  );
}

#[tokio::test(flavor = "current_thread")]
async fn emergency_deny_stops_a_new_effect_without_mutating_the_admitted_policy() {
  let calls = Arc::new(AtomicUsize::new(0));
  let mut store = active_store("run-policy-emergency", "inv-policy-emergency");
  let admitted = policy("allow", "1");
  store
    .bind_infrastructure_policy("run-policy-emergency", &admitted, Utc::now())
    .unwrap();
  let (store, authority) = authority_with_emergency_deny(
    store,
    CountingProcessHandler {
      calls: Arc::clone(&calls),
      fail_after_dispatch: false,
    },
  )
  .await;
  let result = authority
    .execute(
      request(
        "run-policy-emergency",
        "inv-policy-emergency",
        "call-policy-emergency",
      ),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap();
  assert!(matches!(
    result,
    CapabilityCallResult::Failed(ref failed) if failed.error.code == "WOML_POLICY_DENIED"
  ));
  assert_eq!(calls.load(Ordering::SeqCst), 0);
  let store = store.lock().await;
  assert_eq!(
    store
      .infrastructure_policy_binding("run-policy-emergency")
      .unwrap()
      .unwrap()
      .policy_digest,
    admitted.digest
  );
  let events = store.events("run-policy-emergency").unwrap();
  let decision = events.iter().find_map(|event| match &event.payload {
    RunEventPayload::InfrastructurePolicyDecisionRecorded(data) => Some(&data.decision),
    _ => None,
  });
  let overlay = decision
    .expect("durable policy decision")
    .emergency_deny
    .as_ref()
    .unwrap();
  assert_eq!(overlay.policy_id, "incident-stop");
  assert_eq!(overlay.matched_rule_ids, vec!["stop-process"]);
}

#[tokio::test(flavor = "current_thread")]
async fn denial_is_durable_and_never_reaches_the_handler_or_operation_start() {
  let calls = Arc::new(AtomicUsize::new(0));
  let mut store = active_store("run-policy-deny", "inv-policy-deny");
  store
    .bind_infrastructure_policy("run-policy-deny", &policy("deny", "1"), Utc::now())
    .unwrap();
  let (store, authority) = authority(
    store,
    CountingProcessHandler {
      calls: Arc::clone(&calls),
      fail_after_dispatch: false,
    },
  )
  .await;
  let result = authority
    .execute(
      request("run-policy-deny", "inv-policy-deny", "call-policy-deny"),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap();
  assert!(matches!(
    result,
    CapabilityCallResult::Failed(ref failed) if failed.error.code == "WOML_POLICY_DENIED"
  ));
  assert_eq!(calls.load(Ordering::SeqCst), 0);
  let events = store.lock().await.events("run-policy-deny").unwrap();
  assert!(events.iter().any(|event| matches!(
    event.payload,
    RunEventPayload::InfrastructurePolicyDecisionRecorded(_)
  )));
  assert!(!events
    .iter()
    .any(|event| matches!(event.payload, RunEventPayload::OperationStarted(_))));
}

#[tokio::test(flavor = "current_thread")]
async fn unsatisfied_requirements_are_durable_and_never_reach_the_handler() {
  let calls = Arc::new(AtomicUsize::new(0));
  let mut store = active_store("run-policy-requirements", "inv-policy-requirements");
  store
    .bind_infrastructure_policy(
      "run-policy-requirements",
      &policy_requiring_verified_artifact(),
      Utc::now(),
    )
    .unwrap();
  let (store, authority) = authority(
    store,
    CountingProcessHandler {
      calls: Arc::clone(&calls),
      fail_after_dispatch: false,
    },
  )
  .await;
  let result = authority
    .execute(
      request(
        "run-policy-requirements",
        "inv-policy-requirements",
        "call-policy-requirements",
      ),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap();
  assert!(matches!(
    result,
    CapabilityCallResult::Failed(ref failed)
      if failed.error.code == "WOML_POLICY_REQUIREMENTS_UNSATISFIED"
  ));
  assert_eq!(calls.load(Ordering::SeqCst), 0);
  let events = store
    .lock()
    .await
    .events("run-policy-requirements")
    .unwrap();
  assert!(events.iter().any(|event| matches!(
    event.payload,
    RunEventPayload::InfrastructurePolicyDecisionRecorded(_)
  )));
  assert!(!events
    .iter()
    .any(|event| matches!(event.payload, RunEventPayload::OperationStarted(_))));
}

#[tokio::test(flavor = "current_thread")]
async fn infrastructure_allow_does_not_override_the_lower_level_handler_policy() {
  let calls = Arc::new(AtomicUsize::new(0));
  let mut store = active_store("run-policy-allow", "inv-policy-allow");
  store
    .bind_infrastructure_policy("run-policy-allow", &policy("allow", "1"), Utc::now())
    .unwrap();
  let (store, authority) = authority(
    store,
    CountingProcessHandler {
      calls: Arc::clone(&calls),
      fail_after_dispatch: true,
    },
  )
  .await;
  let result = authority
    .execute(
      request("run-policy-allow", "inv-policy-allow", "call-policy-allow"),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap();
  assert!(matches!(
    result,
    CapabilityCallResult::Failed(ref failed)
      if failed.error.code == "WOML_PROCESS_POLICY_DENIED"
  ));
  assert_eq!(calls.load(Ordering::SeqCst), 1);
  let events = store.lock().await.events("run-policy-allow").unwrap();
  let decision = events.iter().position(|event| {
    matches!(
      event.payload,
      RunEventPayload::InfrastructurePolicyDecisionRecorded(_)
    )
  });
  let started = events
    .iter()
    .position(|event| matches!(event.payload, RunEventPayload::OperationStarted(_)));
  assert!(decision < started);
}

#[tokio::test(flavor = "current_thread")]
async fn native_fetch_is_rejected_before_network_io_when_a_policy_is_admitted() {
  let calls = Arc::new(AtomicUsize::new(0));
  let mut store = active_store("run-policy-fetch", "inv-policy-fetch");
  store
    .bind_infrastructure_policy("run-policy-fetch", &policy("allow", "1"), Utc::now())
    .unwrap();
  let (store, authority) = authority(
    store,
    CountingProcessHandler {
      calls,
      fail_after_dispatch: false,
    },
  )
  .await;
  let error = authority
    .observe_native_fetch(
      &NativeFetchInvocationIdentity {
        run_id: "run-policy-fetch".to_string(),
        node_id: "loadCustomer".to_string(),
        attempt_number: 1,
        invocation_id: "inv-policy-fetch".to_string(),
        step_idempotency_key: step_effect_idempotency_key(
          "run-policy-fetch",
          DEFINITION_HASH,
          "loadCustomer",
        ),
      },
      NativeFetchObservation::Started {
        contract: "woml.native-fetch-observation".to_string(),
        contract_version: 1,
        invocation_id: "inv-policy-fetch".to_string(),
        request_id: "fetch_policy_1234567890".to_string(),
        method: "GET".to_string(),
        origin: "https://example.com".to_string(),
        path: "/health".to_string(),
        request_body_bytes: None,
        started_at: Utc::now(),
      },
    )
    .await
    .unwrap_err();
  assert!(error.to_string().contains("WOML_POLICY_INPUT_UNSUPPORTED"));
  let events = store.lock().await.events("run-policy-fetch").unwrap();
  assert!(events.iter().any(|event| matches!(
    event.payload,
    RunEventPayload::InfrastructurePolicyDecisionRecorded(_)
  )));
  assert!(!events
    .iter()
    .any(|event| matches!(event.payload, RunEventPayload::OperationStarted(_))));
}

#[test]
fn durable_binding_reloads_exact_bytes_and_rejects_policy_replacement() {
  let mut store = active_store("run-policy-binding", "inv-policy-binding");
  let first = policy("deny", "1");
  store
    .bind_infrastructure_policy("run-policy-binding", &first, Utc::now())
    .unwrap();
  let restored = store
    .infrastructure_policy_for_run("run-policy-binding")
    .unwrap()
    .unwrap();
  assert_eq!(restored.digest, first.digest);
  assert_eq!(restored.canonical_payload, first.canonical_payload);
  let replacement = policy("allow", "2");
  assert!(store
    .bind_infrastructure_policy("run-policy-binding", &replacement, Utc::now())
    .unwrap_err()
    .to_string()
    .contains("different infrastructure policy"));
}

#[test]
fn backup_and_restore_preserve_exact_policy_identity_and_canonical_bytes() {
  let suffix = uuid::Uuid::new_v4().simple().to_string();
  let source = std::env::temp_dir().join(format!("woml-policy-source-{suffix}.sqlite"));
  let backup = std::env::temp_dir().join(format!("woml-policy-backup-{suffix}.sqlite"));
  let first = policy("deny", "1");
  let mut store = initialize_active_store(
    DurableEventStore::open(&source).unwrap(),
    "run-policy-backup",
    "inv-policy-backup",
  );
  store
    .bind_infrastructure_policy("run-policy-backup", &first, Utc::now())
    .unwrap();
  drop(store);

  create_online_backup(
    &source,
    &backup,
    "policy-backup-lease",
    "policy-backup-test",
    "policy-backup-deployment",
  )
  .unwrap();
  prepare_restored_store(
    &backup,
    &[DEFINITION_HASH.to_string()],
    "policy-backup-id",
    Utc::now(),
  )
  .unwrap();
  let restored = DurableEventStore::open(&backup)
    .unwrap()
    .infrastructure_policy_for_run("run-policy-backup")
    .unwrap()
    .unwrap();
  assert_eq!(restored.digest, first.digest);
  assert_eq!(restored.canonical_payload, first.canonical_payload);

  for path in [&source, &backup] {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
  }
}
