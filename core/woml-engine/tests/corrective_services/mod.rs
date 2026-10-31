//! Exercise managed calls through the real corrective Script Host path.

use super::{
  compile_woml_fixture, correction_retry_model, model, runtime_options, two_effect_failure_model,
  HASH,
};
use serde_json::{json, Map};
use sha2::{Digest, Sha256};
use woml_engine::{
  execute_workflow_durable, resume_workflow_durable_any_outcome, CompiledWorkflowDefinition,
  DurableEventStore, OperationStatus, RunEventPayload, RuntimeExecutionError,
};

struct TestDirectory(std::path::PathBuf);
impl TestDirectory {
  fn new() -> Self {
    let root = std::env::temp_dir().join(format!(
      "woml-corrective-services-{}",
      uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir(&root).unwrap();
    Self(root)
  }
}
impl Drop for TestDirectory {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir_all(&self.0);
  }
}

fn with_managed_calls(
  mut workflow: CompiledWorkflowDefinition,
  retry: bool,
) -> CompiledWorkflowDefinition {
  for saga in workflow.graph.sagas.as_mut().unwrap() {
    for action in saga.compensations.iter_mut().chain(&mut saga.cleanup) {
      let woml_engine::model::ValueExpression::Object { fields } = &mut action.node.inputs else {
        unreachable!()
      };
      let prefix = if retry {
        "if (attempt.number === 1) throw new Error('retry before corrective effect');"
      } else {
        ""
      };
      fields.insert("source".into(), woml_engine::model::ValueExpression::Literal {
        value: json!(format!("{prefix} await services.cache.set('corrective-marker', {{ ready: true }}); const value = await services.cache.get('corrective-marker'); if (!value.hit || !value.value.ready) throw new Error('corrective managed effect missing'); return {{ corrected: true }};")),
      });
    }
  }
  workflow.validate_structure().unwrap();
  workflow
}

#[tokio::test]
async fn real_compensation_and_cleanup_execute_managed_services_and_resume_without_replay() {
  let mut current = model();
  current.schema_version = 21;
  current.graph.nodes[0].execution = Some(
    serde_json::from_value(json!({
      "profileVersion": 1, "target": "local", "requiredCapabilities": ["runtime.script"]
    }))
    .unwrap(),
  );
  for (workflow, failure, retry) in [
    (model(), false, false),
    (current, false, false),
    (two_effect_failure_model(), true, false),
    (correction_retry_model(), true, true),
  ] {
    let Some(options) = runtime_options() else {
      return;
    };
    let directory = TestDirectory::new();
    let database = directory.0.join("state.sqlite");
    let outcome = execute_workflow_durable(
      with_managed_calls(workflow, retry),
      HASH.into(),
      Map::new(),
      options.clone(),
      database.clone(),
    )
    .await;
    let run = if failure {
      let error = outcome.expect_err("the original business failure remains primary");
      let RuntimeExecutionError::RunFailed(details) = error else {
        panic!("unexpected runtime outcome: {error:?}")
      };
      assert_eq!(details.node_id.as_deref(), Some("verifyDeployment"));
      details.events[0].run_id.clone()
    } else {
      outcome
        .expect("successful forward work still executes managed cleanup")
        .run_id
    };
    let store = DurableEventStore::open(&database).unwrap();
    let events = store.events(&run).unwrap();
    let projection = store.projection(&run).unwrap();
    let completed = events
      .iter()
      .filter_map(|event| match &event.payload {
        RunEventPayload::CorrectionAttemptSucceeded(data) => Some((event.sequence, data)),
        _ => None,
      })
      .collect::<Vec<_>>();
    assert_eq!(completed.len(), if failure { 3 } else { 1 });
    for (sequence, action) in completed {
      assert_eq!(action.attempt, if retry { 2 } else { 1 });
      let invocation = woml_engine::durable::correction_invocation_id(
        &run,
        &action.saga_id,
        &action.action_id,
        action.attempt,
      );
      let key = format!("sha256:{:x}", Sha256::digest(invocation.as_bytes()));
      let operations = projection
        .operations
        .values()
        .filter(|operation| operation.identity.invocation_id == invocation)
        .collect::<Vec<_>>();
      assert_eq!(
        operations.len(),
        2,
        "each corrective action calls cache.set and cache.get"
      );
      assert!(operations
        .iter()
        .all(|operation| matches!(operation.status, OperationStatus::Succeeded { .. })));
      for event in &events {
        if let RunEventPayload::OperationStarted(operation) = &event.payload {
          if operation.invocation_id == invocation {
            assert!(event.sequence < sequence);
            assert_eq!(operation.node_id, action.action_id);
            assert_eq!(operation.attempt_number, action.attempt);
            // Automatic names are frozen by the Script Host, and the key must
            // be derived from this corrective attempt rather than a DAG step.
            assert!(
              ["cache.set", "cache.get"].iter().any(
                |name| operation.operation_key == woml_engine::derive_operation_key(&key, name)
              )
            );
          }
        }
      }
    }
    assert!(events
      .iter()
      .any(|event| matches!(event.payload, RunEventPayload::SagaCorrectionCompleted(_))));
    drop(store);
    let resumed =
      resume_workflow_durable_any_outcome(database.clone(), &run, options.clone()).await;
    if !failure {
      resumed.unwrap();
    }
    assert_eq!(
      DurableEventStore::open(&database)
        .unwrap()
        .events(&run)
        .unwrap(),
      events,
      "settled corrective effects must not be replayed on resume"
    );
  }
}

#[tokio::test]
async fn iteration_compensations_keep_distinct_managed_operation_identities() {
  let Some(options) = runtime_options() else {
    return;
  };
  let Some(workflow) = compile_woml_fixture("for-each.reviewed.woml") else {
    return;
  };
  let directory = TestDirectory::new();
  let database = directory.0.join("state.sqlite");
  let error = execute_workflow_durable(
    with_managed_calls(workflow, false),
    HASH.into(),
    Map::new(),
    options,
    database.clone(),
  )
  .await
  .unwrap_err();
  let RuntimeExecutionError::RunFailed(details) = error else {
    panic!("expected original item publication failure")
  };
  assert_eq!(details.node_id.as_deref(), Some("failItems"));
  let store = DurableEventStore::open(&database).unwrap();
  let inspection = store
    .inspect_correction_v1(&details.events[0].run_id)
    .unwrap();
  assert_eq!(inspection.sagas[0].completed_compensations, 3);
  assert_eq!(inspection.sagas[0].completed_cleanup, 1);
  let projection = store.projection(&details.events[0].run_id).unwrap();
  let operations = projection.operations.values().collect::<Vec<_>>();
  assert_eq!(operations.len(), 8);
  assert!(operations
    .iter()
    .all(|operation| matches!(operation.status, OperationStatus::Succeeded { .. })));
  assert_eq!(
    operations
      .iter()
      .map(|operation| &operation.operation_key)
      .collect::<std::collections::BTreeSet<_>>()
      .len(),
    8
  );
}

#[tokio::test]
async fn cleanup_can_use_a_callback_scoped_lease_without_borrowing_a_forward_attempt() {
  let Some(options) = runtime_options() else {
    return;
  };
  let directory = TestDirectory::new();
  let database = directory.0.join("state.sqlite");
  let mut workflow = model();
  let cleanup = &mut workflow.graph.sagas.as_mut().unwrap()[0].cleanup[0];
  let woml_engine::model::ValueExpression::Object { fields } = &mut cleanup.node.inputs else {
    unreachable!()
  };
  fields.insert("source".into(), woml_engine::model::ValueExpression::Literal {
    value: json!("return await services.locks.withLease('f02/cleanup-resource', { name: 'cleanup-resource', acquireTimeout: '1s', lease: '5s' }, async lease => { await lease.assertCurrent(); await services.cache.set('lease-protected-cleanup', { ready: true }); return { cleaned: true }; });"),
  });
  let result =
    execute_workflow_durable(workflow, HASH.into(), Map::new(), options, database.clone())
      .await
      .unwrap();
  assert!(result.events.iter().any(|event| matches!(&event.payload,
    RunEventPayload::CorrectionAttemptSucceeded(data) if data.action_kind == woml_engine::CorrectiveActionKind::Cleanup)));
  let projection = DurableEventStore::open(&database)
    .unwrap()
    .projection(&result.run_id)
    .unwrap();
  assert!(projection
    .operations
    .values()
    .any(|operation| operation.capability == "locks" && operation.operation == "acquire"));
  assert!(projection
    .operations
    .values()
    .any(|operation| operation.capability == "cache"));
  assert!(projection
    .operations
    .values()
    .all(|operation| matches!(operation.status, OperationStatus::Succeeded { .. })));
}

#[tokio::test]
async fn compensation_and_cleanup_state_calls_obey_the_admitted_infrastructure_policy() {
  use woml_engine::{
    prepare_infrastructure_policy, InfrastructurePolicyExecutionModeV1, InfrastructurePolicyTrustV1,
  };
  for decision in ["allow", "deny"] {
    let Some(options) = runtime_options() else {
      return;
    };
    let directory = TestDirectory::new();
    let database = directory.0.join("state.sqlite");
    let mut workflow = two_effect_failure_model();
    for saga in workflow.graph.sagas.as_mut().unwrap() {
      for action in saga.compensations.iter_mut().chain(&mut saga.cleanup) {
        let woml_engine::model::ValueExpression::Object { fields } = &mut action.node.inputs else {
          unreachable!()
        };
        fields.insert("source".into(), woml_engine::model::ValueExpression::Literal {
          value: json!("await services.state.increment('private-correction-count', 1, { name: 'corrective-count' }); return { corrected: true };"),
        });
      }
    }
    let policy = prepare_infrastructure_policy(serde_json::from_value(json!({
      "contract": "woml.infrastructure-policy", "contractVersion": 1,
      "id": "corrective-state", "version": "1", "environment": "test",
      "defaults": { "read": decision, "idempotent-write": decision, "unsafe-write": decision }, "rules": []
    })).unwrap(), &InfrastructurePolicyTrustV1::default()).unwrap();
    let error = execute_workflow_durable(
      workflow,
      HASH.into(),
      Map::new(),
      options.with_infrastructure_policy(policy),
      database.clone(),
    )
    .await
    .unwrap_err();
    let RuntimeExecutionError::RunFailed(details) = error else {
      panic!("expected original deployment failure")
    };
    assert_eq!(details.node_id.as_deref(), Some("verifyDeployment"));
    let projection = DurableEventStore::open(&database)
      .unwrap()
      .projection(&details.events[0].run_id)
      .unwrap();
    assert_eq!(projection.infrastructure_policy_decisions.len(), 3);
    assert_eq!(
      projection
        .infrastructure_policy_decisions
        .iter()
        .filter(|record| record.decision.operation.execution_mode
          == InfrastructurePolicyExecutionModeV1::Compensation)
        .count(),
      2
    );
    assert_eq!(
      projection
        .infrastructure_policy_decisions
        .iter()
        .filter(|record| record.decision.operation.execution_mode
          == InfrastructurePolicyExecutionModeV1::Cleanup)
        .count(),
      1
    );
    assert_eq!(
      projection.operations.len(),
      if decision == "allow" { 3 } else { 0 }
    );
    assert!(projection
      .operations
      .values()
      .all(|operation| operation.capability == "state"
        && matches!(operation.status, OperationStatus::Succeeded { .. })));
    assert!(!serde_json::to_string(&details.events)
      .unwrap()
      .contains("private-correction-count"));
  }
}
