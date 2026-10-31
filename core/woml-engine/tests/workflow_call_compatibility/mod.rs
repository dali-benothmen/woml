//! Reattachment must validate the child contract actually admitted by the store.

use chrono::Utc;
use serde_json::{json, Map};
use woml_engine::{
  derive_workflow_call_key, run_event_schema_version_for_model, DurableEventStore,
  WorkflowCallAdmissionRequest,
};

#[test]
fn current_policy_children_reattach_after_reopen_without_new_admission_or_identity_changes() {
  for version in [12, 13, 14, 18, 21] {
    let database = super::TemporaryDatabase::new("policy-child-reattach");
    let (parent_run_id, _) = super::seed_parent(database.path());
    let mut child = if version == 12 {
      super::parent_model()
    } else {
      serde_json::from_str(include_str!(
        "../../../../woml/tests/fixtures/fork-branch/join-all.compiled.v13.json"
      ))
      .unwrap()
    };
    child.schema_version = version;
    child.workflow_id = "policy-child".into();
    if version >= 17 {
      child.graph.waits = Some(vec![]);
    }
    if version >= 18 {
      child.graph.locks = Some(vec![]);
    }
    if version >= 19 {
      child.graph.sagas = Some(vec![]);
    }
    if version == 21 {
      child.graph.nodes.iter_mut().find(|node| node.handler == "runtime.script").unwrap().execution =
        Some(serde_json::from_value(json!({ "profileVersion": 1, "target": "local", "requiredCapabilities": ["runtime.script"] })).unwrap());
    }
    child.runtime_policy = Some(
      serde_json::from_value(json!({
        "profileVersion": 1, "concurrency": 1
      }))
      .unwrap(),
    );
    let call_key = derive_workflow_call_key(
      "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
      &child.workflow_id,
      "one-logical-call",
    );
    let request = WorkflowCallAdmissionRequest {
      child_run_id: format!("run_call_{}", call_key.strip_prefix("sha256:").unwrap()),
      call_key,
      parent_run_id,
      parent_node_id: "requestRisk".into(),
      parent_attempt: 1,
      target_workflow_id: child.workflow_id.clone(),
      target_definition_hash: super::TRIGGERED_CHILD_HASH.into(),
      payload: Map::from_iter([("privateInput".into(), json!("only-child-payload"))]),
      admitted_at: Utc::now(),
    };
    let mut store = DurableEventStore::open(database.path()).unwrap();
    store
      .register_definition(&child, super::TRIGGERED_CHILD_HASH)
      .unwrap();
    let first = store.admit_workflow_call(request.clone()).unwrap();
    let events = store.events(&first.admission.child_run_id).unwrap();
    assert_eq!(
      events[0].event_schema_version,
      run_event_schema_version_for_model(version)
    );
    drop(store);

    let mut reopened = DurableEventStore::open(database.path()).unwrap();
    let repeated = reopened
      .admit_workflow_call(request.clone())
      .unwrap_or_else(|error| panic!("Model v{version} child could not reattach: {error:?}"));
    assert!(repeated.duplicate);
    assert_eq!(repeated.admission, first.admission);
    assert_eq!(
      reopened.events(&first.admission.child_run_id).unwrap(),
      events
    );
    assert_eq!(
      reopened
        .workflow_call_relations_for_run(&request.parent_run_id)
        .unwrap()
        .child_calls
        .len(),
      1
    );
    let mut conflicting = request.clone();
    conflicting
      .payload
      .insert("privateInput".into(), json!("different-input"));
    assert!(matches!(
      reopened.admit_workflow_call(conflicting),
      Err(woml_engine::DurableStoreError::WorkflowCallIdempotencyConflict)
    ));
    assert_eq!(
      reopened.events(&first.admission.child_run_id).unwrap(),
      events
    );
  }
}
