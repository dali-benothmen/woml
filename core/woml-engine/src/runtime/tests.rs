//! Characterization of the foundation boundaries extracted in RCM3.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::{CapabilityRegistry, ScriptHostProcessOptions};
use serde_json::json;

use super::{
  ensure_local_script_execution, runtime_context_for_node, CompiledWorkflowDefinition, EngineClock,
  FixedEngineClock, RunProjection, RuntimeExecutionOptions,
};

fn options() -> RuntimeExecutionOptions {
  RuntimeExecutionOptions::new(
    ScriptHostProcessOptions::new("bun", "unused-script-host.ts"),
    500,
  )
}

fn workflow() -> CompiledWorkflowDefinition {
  CompiledWorkflowDefinition::from_json(include_str!(
    "../../../../woml/tests/fixtures/hello.compiled.v1.json"
  ))
  .unwrap()
}

#[test]
fn optional_runtime_resources_remain_unactivated_by_construction() {
  let options = options();
  assert!(options
    .capability_registry
    .descriptor("http", "request")
    .is_some());
  assert!(options
    .capability_registry
    .descriptor("process", "run")
    .is_none());
  assert!(options.shared_script_hosts.is_none());
  assert!(options.capability_authority.is_none());
  assert!(options.managed_process_handler.is_none());
  assert!(options.managed_artifact_store.is_none());
  assert!(options.operational_output_store.is_none());
  assert!(options.policy_execution.is_none());
  assert!(options.reviewer_authority.is_none());
}

#[test]
fn cloned_options_share_owners_and_a_custom_registry_does_not_keep_default_authority() {
  let original = options().with_shared_script_hosts();
  let cloned = original.clone();
  assert!(Arc::ptr_eq(
    &original.capability_registry,
    &cloned.capability_registry
  ));
  assert!(Arc::ptr_eq(&original.lock_renewals, &cloned.lock_renewals));
  assert!(Arc::ptr_eq(
    original.shared_script_hosts.as_ref().unwrap(),
    cloned.shared_script_hosts.as_ref().unwrap(),
  ));

  let registry = Arc::new(CapabilityRegistry::default());
  let custom = original.with_capability_registry(Arc::clone(&registry));
  assert!(Arc::ptr_eq(&custom.capability_registry, &registry));
  assert!(custom
    .capability_registry
    .descriptor("http", "request")
    .is_none());
  assert!(custom.managed_database_pool.is_none());
  assert!(custom.managed_storage_store.is_none());
  assert!(custom.managed_cache_store.is_none());
  assert!(custom.managed_durable_state_store.is_none());
  assert!(custom.managed_locks_service.is_none());
}

#[test]
fn node_context_is_a_visibility_filtered_snapshot_not_mutable_run_state() {
  let mut workflow = workflow();
  workflow.graph.context_visibility = Some(vec![crate::model::CompiledContextVisibility {
    node_id: "b".to_string(),
    step_ids: vec!["a".to_string()],
  }]);
  let mut projection = RunProjection::default();
  projection
    .context
    .trigger
    .insert("orderId".to_string(), json!(42));
  projection
    .context
    .steps
    .insert("a".to_string(), json!({"ready": true}));
  projection
    .context
    .steps
    .insert("unrelated".to_string(), json!("already finished"));
  let before = projection.clone();
  let mut context = runtime_context_for_node(&workflow, &projection, "b");
  assert_eq!(context.trigger, projection.context.trigger);
  assert_eq!(context.steps.keys().collect::<Vec<_>>(), ["a"]);
  context
    .steps
    .insert("a".to_string(), json!("local mutation"));
  assert_eq!(projection, before);
}

#[test]
fn remote_placement_still_fails_closed_before_local_script_invocation() {
  let mut workflow = workflow();
  let node = &mut workflow.graph.nodes[0];
  assert!(ensure_local_script_execution(node).is_ok());
  node.execution = Some(crate::model::CompiledExecutionPlacement {
    profile_version: 1,
    target: "production-private".to_string(),
    required_capabilities: Vec::new(),
  });
  let error = ensure_local_script_execution(node).unwrap_err();
  assert!(error
    .to_string()
    .contains("requires an activated remote dispatch coordinator"));
}

#[test]
fn fixed_clocks_and_secret_safe_option_debugging_keep_their_contract() {
  let now = chrono::DateTime::parse_from_rfc3339("2026-10-30T10:00:00Z")
    .unwrap()
    .with_timezone(&chrono::Utc);
  let options = options()
    .with_clock(Arc::new(FixedEngineClock::new(now)))
    .with_resolved_secrets(BTreeMap::from([(
      "TOKEN".to_string(),
      "not-for-debug".to_string(),
    )]));
  assert_eq!(options.clock.now(), now);
  assert_eq!(FixedEngineClock::new(now).now(), now);
  let debug = format!("{options:?}");
  assert!(!debug.contains("not-for-debug"));
  assert!(debug.contains("resolved_secret_count: 1"));
}
