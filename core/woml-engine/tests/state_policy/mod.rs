//! StateService policy tests use the real managed handler and durable store.

use chrono::Utc;
use serde_json::{json, Value};
use std::{
  path::{Path, PathBuf},
  sync::Arc,
};
use tokio::sync::Mutex;
use woml_engine::{
  capability::CapabilityIdentityMode,
  derive_operation_key,
  infrastructure_policy_runtime::{
    infrastructure_policy_descriptor_coverage, InfrastructurePolicyCoverageV1,
  },
  prepare_emergency_deny_overlay, prepare_infrastructure_policy, CapabilityCallRequest,
  CapabilityCallResult, CapabilityCancellationToken, CapabilityRegistry,
  DurableCapabilityAuthority, DurableEventStore, DurableStateStore,
  InfrastructurePolicyDecisionOutcomeV1, InfrastructurePolicyDescriptorContextV1,
  InfrastructurePolicyEffectV1, InfrastructurePolicyExecutionModeV1, InfrastructurePolicyTrustV1,
  ManagedDurableStateHandler, ManagedDurableStateStore, ManagedLocksService, OperationStatus,
  PreparedInfrastructurePolicyV1, RunEventPayload,
};

struct TestDirectory(PathBuf);
impl TestDirectory {
  fn new() -> Self {
    let path = std::env::temp_dir().join(format!(
      "woml-state-policy-{}",
      uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir(&path).unwrap();
    Self(path)
  }
  fn database(&self) -> PathBuf {
    self.0.join("state.sqlite")
  }
}
impl Drop for TestDirectory {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir_all(&self.0);
  }
}

fn registry(path: Option<&Path>) -> Arc<CapabilityRegistry> {
  let state = Arc::new(ManagedDurableStateStore::default());
  if let Some(path) = path {
    state.configure_for_state(path).unwrap();
  }
  let registry = Arc::new(CapabilityRegistry::default());
  for handler in ManagedDurableStateHandler::handlers(state) {
    registry.register(handler).unwrap();
  }
  registry
}

fn call(operation: &str, input: Value, name: &str) -> CapabilityCallRequest {
  let mut call = super::request("run-state-policy", "inv-state-policy", name);
  call.capability = "state".into();
  call.operation = operation.into();
  call.identity.operation_name = format!("state.{operation}.{name}");
  call.identity.operation_key = derive_operation_key(
    &call.identity.step_idempotency_key,
    &call.identity.operation_name,
  );
  call.input = json!({"contract": "woml.state", "contractVersion": 1, "kind": "request", "operation": operation, "input": input});
  call
}

struct Harness {
  authority: DurableCapabilityAuthority,
  store: Arc<Mutex<DurableEventStore>>,
  directory: TestDirectory,
}

impl Harness {
  fn new(
    policy: Option<PreparedInfrastructurePolicyV1>,
    overlay: Option<PreparedInfrastructurePolicyV1>,
  ) -> Self {
    let directory = TestDirectory::new();
    let path = directory.database();
    let mut store = super::initialize_active_store(
      DurableEventStore::open(&path).unwrap(),
      "run-state-policy",
      "inv-state-policy",
    );
    if let Some(policy) = policy {
      store
        .bind_infrastructure_policy("run-state-policy", &policy, Utc::now())
        .unwrap();
    }
    let store = Arc::new(Mutex::new(store));
    let authority = DurableCapabilityAuthority::new_with_infrastructure_policy_authorities(
      registry(Some(&path)),
      store.clone(),
      Arc::new(ManagedLocksService::default()),
      true,
      None,
      overlay.map(Arc::new),
    );
    Self {
      authority,
      store,
      directory,
    }
  }
  async fn execute(&self, operation: &str, input: Value, name: &str) -> CapabilityCallResult {
    self
      .authority
      .execute(
        call(operation, input, name),
        CapabilityCancellationToken::default(),
      )
      .await
      .unwrap()
  }
}

fn context() -> InfrastructurePolicyDescriptorContextV1 {
  InfrastructurePolicyDescriptorContextV1 {
    workflow_id: "customer-services".into(),
    definition_hash: super::DEFINITION_HASH.into(),
    environment: "production".into(),
    effective_target: None,
    execution_mode: InfrastructurePolicyExecutionModeV1::Normal,
    review_digest: None,
  }
}

fn operations(key: &str) -> Vec<(&'static str, Value)> {
  vec![
    ("get", json!({"key": key})),
    ("has", json!({"key": key})),
    ("set", json!({"key": key, "value": "private-state-value"})),
    ("delete", json!({"key": key})),
    ("increment", json!({"key": key, "amount": 1})),
    (
      "set_if_absent",
      json!({"key": key, "value": "private-state-value"}),
    ),
  ]
}

fn success_data(result: CapabilityCallResult) -> Value {
  let CapabilityCallResult::Succeeded(success) = result else {
    panic!("state call failed: {result:?}")
  };
  success.result["data"].clone()
}

#[test]
fn all_six_state_adapters_are_bounded_pure_and_do_not_serialize_keys_or_values() {
  let registry = registry(None); // Descriptor construction must not require a configured database.
  let key = "private/customer:42/雪";
  for (operation, input) in operations(key) {
    assert_eq!(
      infrastructure_policy_descriptor_coverage("state", operation),
      InfrastructurePolicyCoverageV1::Supported
    );
    let request = call(operation, input, "metadata");
    let descriptor = registry
      .infrastructure_policy_descriptor(&request, context())
      .unwrap();
    assert_eq!(
      descriptor.effect,
      if matches!(operation, "get" | "has") {
        InfrastructurePolicyEffectV1::Read
      } else {
        InfrastructurePolicyEffectV1::IdempotentWrite
      }
    );
    assert_eq!(descriptor.facts.len(), 3);
    assert_eq!(descriptor.facts["action"], operation);
    assert_eq!(descriptor.facts["scopeClass"], "workflow");
    assert_eq!(descriptor.facts["keyDigest"].len(), 71);
    assert!(descriptor.target.as_deref().unwrap().starts_with("state-"));
    assert_eq!(descriptor.target.as_deref().unwrap().len(), 70);
    let encoded = serde_json::to_string(&descriptor).unwrap();
    assert!(!encoded.contains(key));
    assert!(!encoded.contains("private-state-value"));
    assert_eq!(descriptor.argument_count, 0);
    assert!(descriptor.argument_digest.is_none());
    assert!(descriptor.executable.is_none());
  }
  let first = call(
    "set",
    json!({"key": key, "value": "first-secret"}),
    "same-operation",
  );
  let mut changed_value = first.clone();
  changed_value.input["input"]["value"] = json!("second-secret");
  assert_eq!(
    registry
      .infrastructure_policy_descriptor(&first, context())
      .unwrap(),
    registry
      .infrastructure_policy_descriptor(&changed_value, context())
      .unwrap(),
    "policy facts must not depend on state values"
  );
  changed_value.input["input"]["key"] = json!("other-private-key");
  assert_ne!(
    registry
      .infrastructure_policy_descriptor(&first, context())
      .unwrap()
      .target,
    registry
      .infrastructure_policy_descriptor(&changed_value, context())
      .unwrap()
      .target
  );
}

#[test]
fn malformed_and_unsupported_state_policy_inputs_fail_closed() {
  let registry = registry(None);
  let valid = call("set", json!({"key": "private-key", "value": 1}), "validate");
  for mutation in 0..12 {
    let mut request = valid.clone();
    match mutation {
      0 => request.input["contractVersion"] = json!(2),
      1 => request.input["contract"] = json!("other.state"),
      2 => request.input["kind"] = json!("result"),
      3 => request.input["operation"] = json!("get"),
      4 => request.input["input"]["key"] = json!(""),
      5 => request.input["input"]["key"] = json!("x".repeat(257)),
      6 => request.input["input"]["key"] = json!(false),
      7 => {
        request.input["input"]
          .as_object_mut()
          .unwrap()
          .remove("value");
      }
      8 => request.input["input"]["ifVersion"] = json!(-1),
      9 => request.input["input"]["scope"] = json!("other-workflow"),
      10 => request.input["policyFacts"] = json!({"action": "get"}),
      _ => request.input_contract_version = 2,
    }
    assert!(
      registry
        .infrastructure_policy_descriptor(&request, context())
        .is_err(),
      "mutation {mutation}"
    );
  }
  for operation in ["clear", "scan", "setIfAbsent", "unknown"] {
    assert_eq!(
      infrastructure_policy_descriptor_coverage("state", operation),
      InfrastructurePolicyCoverageV1::Unsupported
    );
    assert!(registry
      .infrastructure_policy_descriptor(
        &call(operation, json!({"key": "private-key"}), "unsupported"),
        context()
      )
      .is_err());
  }
}

#[tokio::test]
async fn every_supported_state_operation_can_be_allowed_or_denied_without_policy_exemptions() {
  for (operation, input) in operations("private/customer:42") {
    for decision in ["allow", "deny"] {
      let harness = Harness::new(Some(super::policy(decision, "1")), None);
      let result = harness.execute(operation, input.clone(), "operation").await;
      let projection = harness
        .store
        .lock()
        .await
        .projection("run-state-policy")
        .unwrap();
      assert_eq!(projection.infrastructure_policy_decisions.len(), 1);
      let recorded = &projection.infrastructure_policy_decisions[0].decision;
      assert_eq!(recorded.operation.capability, "state");
      assert_eq!(recorded.operation.operation, operation);
      if decision == "allow" {
        assert!(
          matches!(result, CapabilityCallResult::Succeeded(_)),
          "{operation}: {result:?}"
        );
        assert_eq!(
          recorded.outcome,
          InfrastructurePolicyDecisionOutcomeV1::Allow
        );
        assert_eq!(projection.operations.len(), 1);
      } else {
        assert!(
          matches!(result, CapabilityCallResult::Failed(failed) if failed.error.code == "WOML_POLICY_DENIED")
        );
        assert_eq!(
          recorded.outcome,
          InfrastructurePolicyDecisionOutcomeV1::Deny
        );
        assert!(projection.operations.is_empty());
        let state = DurableStateStore::open(harness.directory.database()).unwrap();
        let workflow = harness
          .store
          .lock()
          .await
          .run_binding("run-state-policy")
          .unwrap()
          .workflow_id;
        assert_eq!(
          state
            .execute(
              &workflow,
              &call(
                "get",
                json!({"key": "private/customer:42"}),
                "verify-denied"
              )
            )
            .unwrap()
            .result["data"]["found"],
          false
        );
      }
      let history = serde_json::to_string(
        &harness
          .store
          .lock()
          .await
          .events("run-state-policy")
          .unwrap(),
      )
      .unwrap();
      assert!(!history.contains("private/customer:42"));
      assert!(!history.contains("private-state-value"));
    }
  }
}

#[tokio::test]
async fn state_policy_reads_and_writes_keep_their_effect_defaults() {
  let mut policy = super::policy("deny", "1").bundle;
  policy.defaults.read = woml_engine::InfrastructurePolicyDefaultV1::Allow;
  let policy =
    prepare_infrastructure_policy(policy, &InfrastructurePolicyTrustV1::default()).unwrap();
  let harness = Harness::new(Some(policy), None);
  for (operation, input) in operations("private-key") {
    let result = harness.execute(operation, input, operation).await;
    if matches!(operation, "get" | "has") {
      assert!(matches!(result, CapabilityCallResult::Succeeded(_)));
    } else {
      assert!(
        matches!(result, CapabilityCallResult::Failed(failed) if failed.error.code == "WOML_POLICY_DENIED")
      );
    }
  }
}

#[tokio::test]
async fn policy_allow_preserves_state_scoping_versions_and_mutation_deduplication() {
  let harness = Harness::new(Some(super::policy("allow", "1")), None);
  assert_eq!(
    success_data(
      harness
        .execute(
          "set",
          json!({"key": "count", "value": 10, "ifVersion": 0}),
          "create"
        )
        .await
    )["version"],
    1
  );
  let increment = call(
    "increment",
    json!({"key": "count", "amount": 2, "ifVersion": 1}),
    "increment-once",
  );
  let first = success_data(
    harness
      .authority
      .execute(increment.clone(), CapabilityCancellationToken::default())
      .await
      .unwrap(),
  );
  let mut redelivery = increment.clone();
  redelivery.call_id = "new-transport-id".into();
  assert!(harness
    .authority
    .execute(redelivery, CapabilityCancellationToken::default())
    .await
    .is_err());
  let connection = harness.store.lock().await;
  let workflow_id = connection
    .run_binding("run-state-policy")
    .unwrap()
    .workflow_id;
  drop(connection);
  let state = DurableStateStore::open(harness.directory.database()).unwrap();
  let receipt = state.execute(&workflow_id, &increment).unwrap();
  assert!(receipt.duplicate);
  assert_eq!(receipt.result["data"], first);
  let mut changed = increment.clone();
  changed.input["input"]["amount"] = json!(99);
  assert!(state.execute(&workflow_id, &changed).is_err());
  assert_eq!(
    success_data(
      harness
        .execute("get", json!({"key": "count"}), "read-count")
        .await
    )["value"],
    12
  );
  let other = state
    .execute(
      "other-workflow",
      &call("get", json!({"key": "count"}), "other-scope"),
    )
    .unwrap();
  assert_eq!(other.result["data"]["found"], false);
  let conflict = harness
    .execute(
      "set",
      json!({"key": "count", "value": 50, "ifVersion": 1}),
      "stale-version",
    )
    .await;
  assert!(
    matches!(conflict, CapabilityCallResult::Failed(failed) if failed.error.code == "WOML_STATE_CONFLICT")
  );
  assert_eq!(
    success_data(
      harness
        .execute(
          "set_if_absent",
          json!({"key": "count", "value": 99}),
          "existing"
        )
        .await
    )["stored"],
    false
  );
  assert_eq!(
    success_data(
      harness
        .execute(
          "delete",
          json!({"key": "count", "ifVersion": 2}),
          "delete-count"
        )
        .await
    )["deleted"],
    true
  );
  assert_eq!(
    success_data(
      harness
        .execute("has", json!({"key": "count"}), "after-delete")
        .await
    )["present"],
    false
  );
}

#[tokio::test]
async fn an_emergency_deny_overrides_state_allow_before_any_mutation() {
  let mut overlay = super::policy("allow", "stop-state").bundle;
  overlay.rules = serde_json::from_value(json!([{
    "id": "deny-state-writes", "match": {"capabilities": ["state"], "effects": ["idempotent-write"]}, "decision": "deny"
  }])).unwrap();
  let overlay =
    prepare_emergency_deny_overlay(overlay, &InfrastructurePolicyTrustV1::default()).unwrap();
  let harness = Harness::new(Some(super::policy("allow", "1")), Some(overlay));
  assert!(
    matches!(harness.execute("set", json!({"key": "count", "value": 1}), "blocked").await,
    CapabilityCallResult::Failed(failed) if failed.error.code == "WOML_POLICY_DENIED")
  );
  assert_eq!(
    success_data(
      harness
        .execute("get", json!({"key": "count"}), "read")
        .await
    )["found"],
    false
  );
  let projection = harness
    .store
    .lock()
    .await
    .projection("run-state-policy")
    .unwrap();
  assert!(projection.infrastructure_policy_decisions[0]
    .decision
    .emergency_deny
    .is_some());
  assert!(projection
    .operations
    .values()
    .all(|operation| operation.operation == "get"));
}

#[tokio::test]
async fn invalid_or_unconfigured_calls_do_not_gain_authority_from_the_adapter() {
  let harness = Harness::new(None, None);
  assert!(
    matches!(harness.execute("set", json!({"key": "count", "value": 1}), "missing-policy").await,
    CapabilityCallResult::Failed(failed) if failed.error.code == "WOML_POLICY_REQUIRED")
  );
  assert!(harness
    .store
    .lock()
    .await
    .projection("run-state-policy")
    .unwrap()
    .operations
    .is_empty());
  let harness = Harness::new(Some(super::policy("allow", "1")), None);
  let before = harness
    .store
    .lock()
    .await
    .events("run-state-policy")
    .unwrap();
  let bad = harness
    .execute(
      "set",
      json!({"key": "private-key", "value": 1, "scope": "other-workflow"}),
      "malformed",
    )
    .await;
  assert!(matches!(bad, CapabilityCallResult::Failed(_)));
  assert_eq!(
    harness
      .store
      .lock()
      .await
      .events("run-state-policy")
      .unwrap(),
    before
  );
  let mut unnamed = call("increment", json!({"key": "count", "amount": 1}), "unnamed");
  unnamed.identity.mode = CapabilityIdentityMode::Automatic;
  assert!(
    matches!(harness.authority.execute(unnamed, CapabilityCancellationToken::default()).await.unwrap(),
    CapabilityCallResult::Failed(failed) if failed.error.code == "WOML_STATE_OPERATION_NAME_INVALID")
  );
  assert_eq!(
    success_data(
      harness
        .execute("get", json!({"key": "count"}), "verify-unchanged")
        .await
    )["found"],
    false
  );
}

#[tokio::test]
async fn an_allowed_state_mutation_passes_policy_and_commits_through_the_real_handler() {
  let directory = TestDirectory::new();
  let path = directory.database();
  let mut store = super::initialize_active_store(
    DurableEventStore::open(&path).unwrap(),
    "run-state-policy",
    "inv-state-policy",
  );
  store
    .bind_infrastructure_policy("run-state-policy", &super::policy("allow", "1"), Utc::now())
    .unwrap();
  let store = Arc::new(Mutex::new(store));
  let authority = DurableCapabilityAuthority::new_with_locks_and_infrastructure_policy(
    registry(Some(&path)),
    store.clone(),
    Arc::new(ManagedLocksService::default()),
    true,
  );
  let result = authority
    .execute(
      call(
        "set",
        json!({"key": "private/customer:42", "value": "private-state-value"}),
        "write",
      ),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap();
  assert!(
    matches!(result, CapabilityCallResult::Succeeded(_)),
    "{result:?}"
  );
  let result = authority
    .execute(
      call("get", json!({"key": "private/customer:42"}), "read"),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap();
  let CapabilityCallResult::Succeeded(result) = result else {
    panic!("allowed read failed")
  };
  assert_eq!(result.result["data"]["value"], "private-state-value");
}

#[tokio::test]
async fn opaque_state_targets_and_evidence_requirements_are_enforced() {
  let request = call(
    "set",
    json!({"key": "private/customer:42", "value": 1}),
    "target",
  );
  let target = registry(None)
    .infrastructure_policy_descriptor(&request, context())
    .unwrap()
    .target
    .unwrap();
  let mut bundle = super::policy("deny", "targeted").bundle;
  bundle.rules = serde_json::from_value(json!([{
    "id": "one-state-key", "match": {"capabilities": ["state"], "operations": ["set"], "targets": [target]}, "decision": "allow"
  }])).unwrap();
  let policy =
    prepare_infrastructure_policy(bundle, &InfrastructurePolicyTrustV1::default()).unwrap();
  let harness = Harness::new(Some(policy), None);
  assert!(matches!(
    harness
      .authority
      .execute(request, CapabilityCancellationToken::default())
      .await
      .unwrap(),
    CapabilityCallResult::Succeeded(_)
  ));
  assert!(
    matches!(harness.execute("set", json!({"key": "other-private-key", "value": 1}), "other-target").await,
    CapabilityCallResult::Failed(failed) if failed.error.code == "WOML_POLICY_DENIED")
  );

  let mut bundle = super::policy_requiring_verified_artifact().bundle;
  bundle.rules[0].matcher =
    serde_json::from_value(json!({"capabilities": ["state"], "operations": ["set"]})).unwrap();
  let policy =
    prepare_infrastructure_policy(bundle, &InfrastructurePolicyTrustV1::default()).unwrap();
  let harness = Harness::new(Some(policy), None);
  assert!(
    matches!(harness.execute("set", json!({"key": "private-key", "value": 1}), "missing-evidence").await,
    CapabilityCallResult::Failed(failed) if failed.error.code == "WOML_POLICY_REQUIREMENTS_UNSATISFIED")
  );
  assert!(harness
    .store
    .lock()
    .await
    .projection("run-state-policy")
    .unwrap()
    .operations
    .is_empty());
}

#[tokio::test]
async fn state_validation_and_size_limits_still_reject_invalid_mutations_after_policy_allow() {
  let harness = Harness::new(Some(super::policy("allow", "1")), None);
  assert!(
    matches!(harness.execute("increment", json!({"key": "count", "amount": "not-an-integer"}), "invalid-amount").await,
    CapabilityCallResult::Failed(failed) if failed.error.code == "WOML_STATE_INTEGER_OVERFLOW")
  );
  assert!(
    matches!(harness.execute("set", json!({"key": "count", "value": "x".repeat(woml_engine::MAX_STATE_VALUE_BYTES + 1)}), "oversized-value").await,
    CapabilityCallResult::Failed(failed) if failed.error.code == "WOML_STATE_VALUE_TOO_LARGE")
  );
  assert_eq!(
    success_data(
      harness
        .execute("has", json!({"key": "count"}), "verify-no-write")
        .await
    )["present"],
    false
  );
  let projection = harness
    .store
    .lock()
    .await
    .projection("run-state-policy")
    .unwrap();
  assert_eq!(
    projection
      .operations
      .values()
      .filter(|operation| matches!(operation.status, OperationStatus::Failed { .. }))
      .count(),
    2
  );
}

#[tokio::test]
async fn real_state_scripts_run_under_policy_without_putting_state_values_in_history() {
  if !std::process::Command::new("bun")
    .arg("--version")
    .output()
    .is_ok_and(|output| output.status.success())
  {
    return;
  }
  let directory = TestDirectory::new();
  let mut workflow: woml_engine::CompiledWorkflowDefinition = serde_json::from_str(include_str!(
    "../../../../woml/tests/fixtures/runtime-policies/runtime-policy.compiled.v12.json"
  ))
  .unwrap();
  let node = &mut workflow.graph.nodes[0];
  node
    .script_runtime
    .as_mut()
    .unwrap()
    .required_secrets
    .clear();
  let woml_engine::model::ValueExpression::Object { fields } = &mut node.inputs else {
    unreachable!()
  };
  fields.insert("source".into(), woml_engine::model::ValueExpression::Literal { value: json!(r#"
    const key = 'private/customer:42';
    await services.state.setIfAbsent(key, { token: 'private-state-value' }, { name: 'initialize' });
    const first = await services.state.get(key);
    const exists = await services.state.has(key);
    if (!exists.present || first.value.token !== 'private-state-value') throw new Error('state round trip failed');
    const saved = await services.state.set(key, 10, { name: 'replace', ifVersion: first.version });
    const counted = await services.state.increment(key, 2, { name: 'add', ifVersion: saved.version });
    await services.state.delete(key, { name: 'remove', ifVersion: counted.version });
    return { done: true };
  "#) });
  let host = woml_engine::ScriptHostProcessOptions::new(
    PathBuf::from("bun"),
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../woml-cli/src/script-host.ts"),
  );
  let options = woml_engine::RuntimeExecutionOptions::new(host, 10_000)
    .with_infrastructure_policy(super::policy("allow", "1"));
  let result = woml_engine::execute_workflow_durable(
    workflow,
    super::DEFINITION_HASH.into(),
    serde_json::Map::new(),
    options,
    directory.database(),
  )
  .await
  .unwrap();
  assert_eq!(result.result, json!({"done": true}));
  assert_eq!(
    result
      .events
      .iter()
      .filter(|event| matches!(
        event.payload,
        RunEventPayload::InfrastructurePolicyDecisionRecorded(_)
      ))
      .count(),
    6
  );
  assert_eq!(
    result
      .events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::OperationSucceeded(_)))
      .count(),
    6
  );
  let encoded = serde_json::to_string(&result.events).unwrap();
  assert!(!encoded.contains("private/customer:42"));
  assert!(!encoded.contains("private-state-value"));
  let reopened = DurableEventStore::open(directory.database()).unwrap();
  assert_eq!(reopened.events(&result.run_id).unwrap(), result.events);
  assert_eq!(
    reopened
      .infrastructure_policy_for_run(&result.run_id)
      .unwrap()
      .unwrap()
      .digest,
    super::policy("allow", "1").digest
  );
}
