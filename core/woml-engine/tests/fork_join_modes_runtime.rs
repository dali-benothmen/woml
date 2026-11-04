use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;
use uuid::Uuid;
use woml_engine::model::{CompiledWorkflowEdge, EdgeCondition, ValueExpression};
use woml_engine::{
  execute_workflow_durable, CapabilityCancellationToken, CapabilityDescriptor, CapabilityEffect,
  CapabilityFailure, CapabilityFailureKind, CapabilityHandler, CapabilityRegistry,
  CompiledWorkflowDefinition, DurableEventStore, RunEventPayload, RuntimeExecutionOptions,
  ScriptHostProcessOptions,
};

const MODEL: &str =
  include_str!("../../../woml/tests/fixtures/fork-branch/join-all.compiled.v13.json");

fn model() -> CompiledWorkflowDefinition {
  serde_json::from_str(MODEL).unwrap()
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

struct TemporaryDatabase(PathBuf);

impl TemporaryDatabase {
  fn new() -> Self {
    Self(std::env::temp_dir().join(format!("woml-fork-modes-{}.sqlite", Uuid::new_v4())))
  }

  fn path(&self) -> &Path {
    &self.0
  }
}

impl Drop for TemporaryDatabase {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.0);
  }
}

fn set_script(workflow: &mut CompiledWorkflowDefinition, node_id: &str, source: String) {
  let node = workflow
    .graph
    .nodes
    .iter_mut()
    .find(|node| node.id == node_id)
    .unwrap();
  let ValueExpression::Object { fields } = &mut node.inputs else {
    panic!("script inputs must be an object");
  };
  fields.insert(
    "source".to_string(),
    ValueExpression::Literal {
      value: Value::String(source),
    },
  );
}

fn retain_finish_visibility(workflow: &mut CompiledWorkflowDefinition, step_ids: &[&str]) {
  let visibility = workflow
    .graph
    .context_visibility
    .as_mut()
    .unwrap()
    .iter_mut()
    .find(|visibility| visibility.node_id == "finish")
    .unwrap();
  visibility.step_ids = step_ids.iter().map(|id| (*id).to_string()).collect();
}

fn event_position(
  events: &[woml_engine::RunEvent],
  predicate: impl Fn(&RunEventPayload) -> bool,
) -> usize {
  events
    .iter()
    .position(|event| predicate(&event.payload))
    .unwrap()
}

// Test-owned synchronization, exercised through the normal capability authority
// and durable operation path. A permit survives release-before-arrival, unlike
// a timing assumption about when another isolated Worker will start.
struct BranchGate(Arc<Semaphore>);

impl CapabilityHandler for BranchGate {
  fn descriptor(&self) -> CapabilityDescriptor {
    CapabilityDescriptor {
      capability: "test".into(),
      operation: "control".into(),
      input_contract_version: 1,
      result_contract_version: 1,
      effect: CapabilityEffect::Read,
      supports_cancellation: true,
      supports_provider_idempotency: false,
    }
  }

  fn execute(
    &self,
    _input: Value,
    cancellation: CapabilityCancellationToken,
  ) -> BoxFuture<'static, Result<Value, CapabilityFailure>> {
    let gate = Arc::clone(&self.0);
    Box::pin(async move {
      tokio::select! {
        permit = gate.acquire_owned() => {
          permit.expect("test gate remains open").forget();
          Ok(Value::Null)
        }
        _ = cancellation.cancelled() => Err(CapabilityFailure {
          kind: CapabilityFailureKind::Cancelled,
          code: "WOML_TEST_GATE_CANCELLED".into(),
          message: "The test branch gate was cancelled.".into(),
          retryable: false,
          ambiguous: false,
          details: None,
        }),
      }
    })
  }
}

#[tokio::test]
async fn branch_gate_requires_a_permit_per_call_and_honors_cancellation() {
  let permits = Arc::new(Semaphore::new(0));
  let gate = BranchGate(Arc::clone(&permits));
  let mut waiting = gate.execute(Value::Null, CapabilityCancellationToken::default());
  assert!(futures_util::poll!(waiting.as_mut()).is_pending());
  permits.add_permits(1);
  assert_eq!(waiting.await.unwrap(), Value::Null);
  assert_eq!(permits.available_permits(), 0);

  // The observer may reach its checkpoint before a cold Worker calls the gate.
  permits.add_permits(1);
  assert_eq!(
    gate
      .execute(Value::Null, CapabilityCancellationToken::default())
      .await
      .unwrap(),
    Value::Null
  );
  assert_eq!(permits.available_permits(), 0);

  let cancellation = CapabilityCancellationToken::default();
  let mut waiting = gate.execute(Value::Null, cancellation.clone());
  assert!(futures_util::poll!(waiting.as_mut()).is_pending());
  cancellation.cancel();
  assert_eq!(
    waiting.await.unwrap_err().kind,
    CapabilityFailureKind::Cancelled
  );
}

async fn execute_gated(
  workflow: CompiledWorkflowDefinition,
  release_after: &str,
  blocked_branches: usize,
) -> woml_engine::WorkflowExecutionResult {
  let host = host_options().expect("Bun script host is available");
  let database = TemporaryDatabase::new();
  let control = DurableEventStore::open(database.path()).unwrap();
  let gate = Arc::new(Semaphore::new(0));
  let registry = Arc::new(CapabilityRegistry::default());
  registry
    .register(Arc::new(BranchGate(Arc::clone(&gate))))
    .unwrap();
  workflow.validate_for_durable_execution().unwrap();
  let hash = format!(
    "sha256:{:x}",
    Sha256::digest(serde_json::to_vec(&workflow).unwrap())
  );
  let execution = Box::pin(execute_workflow_durable(
    workflow,
    hash,
    Map::new(),
    RuntimeExecutionOptions::new(host, 3_000).with_capability_registry(registry),
    database.path().to_path_buf(),
  ));
  let release = async {
    let mut released = 0;
    loop {
      if let Some(run) = control.list_runs(1).unwrap().runs.first() {
        let events = control.events(&run.run_id).unwrap();
        let checkpoint = events.iter().any(|event| {
          matches!(&event.payload, RunEventPayload::StepAttemptSucceeded(data)
            if data.node_id == release_after)
        });
        assert!(
          !events
            .iter()
            .any(|event| matches!(event.payload, RunEventPayload::RunOutcomeDecided(_))),
          "the run must not settle while an owned branch is gated: {events:#?}"
        );
        if checkpoint {
          let joined = events
            .iter()
            .any(|event| matches!(event.payload, RunEventPayload::ForkJoinSettled(_)));
          assert_eq!(
            joined,
            release_after == "finish",
            "join release must follow the selected branches, not Worker speed: {events:#?}"
          );
          let settled_branches = events
            .iter()
            .filter(|event| matches!(event.payload, RunEventPayload::ForkBranchSettled(_)))
            .count();
          if released == 0 || settled_branches >= released {
            // Non-blocking mode releases one branch at a time. Before the
            // second permit, the observer must see the first branch settled
            // but no run outcome: every owned branch is a finalization barrier.
            gate.add_permits(1);
            released += 1;
            if released == blocked_branches {
              break;
            }
          }
        }
      }
      // Poll durable facts, not elapsed time, to decide when to open the gate.
      tokio::time::sleep(Duration::from_millis(5)).await;
    }
  };
  // This watchdog only bounds a broken test/runtime. No timing sample chooses
  // branch order, and the existing per-invocation deadline stays unchanged.
  let (execution, ()) = tokio::time::timeout(Duration::from_secs(15), async {
    tokio::join!(execution, release)
  })
  .await
  .unwrap_or_else(|_| {
    let histories = control
      .list_runs(1)
      .unwrap()
      .runs
      .iter()
      .map(|run| control.events(&run.run_id).unwrap())
      .collect::<Vec<_>>();
    panic!("fork join gate did not reach {release_after}: {histories:#?}")
  });
  execution.unwrap()
}

async fn execute_selected(unjoined_finishes_first: bool) -> woml_engine::WorkflowExecutionResult {
  let mut workflow = model();
  workflow.graph.forks.as_mut().unwrap()[0].joined_branch_ids = vec!["instagram".to_string()];
  workflow
    .graph
    .edges
    .retain(|edge| edge.id != "distribution:join:facebook");
  retain_finish_visibility(&mut workflow, &["prepare", "publishInstagram"]);
  set_script(
    &mut workflow,
    "publishInstagram",
    format!(
      "{} return {{ platform: 'instagram', sawFacebook: context.steps.publishFacebook !== undefined }};",
      if unjoined_finishes_first { "await services.test.control(null);" } else { "" }
    ),
  );
  set_script(
    &mut workflow,
    "publishFacebook",
    format!(
      "{} return {{ platform: 'facebook', sawInstagram: context.steps.publishInstagram !== undefined }};",
      if unjoined_finishes_first { "" } else { "await services.test.control(null);" }
    ),
  );
  set_script(
    &mut workflow,
    "finish",
    "return { instagram: context.steps.publishInstagram, sawFacebook: context.steps.publishFacebook !== undefined };".to_string(),
  );
  execute_gated(
    workflow,
    if unjoined_finishes_first {
      "publishFacebook"
    } else {
      "finish"
    },
    1,
  )
  .await
}

#[tokio::test]
async fn selected_join_releases_for_named_branches_and_never_leaks_unjoined_output() {
  if host_options().is_none() {
    return;
  }
  let slow_unjoined = execute_selected(false).await;
  let fast_unjoined = execute_selected(true).await;
  let expected = json!({
    "instagram": { "platform": "instagram", "sawFacebook": false },
    "sawFacebook": false
  });
  assert_eq!(slow_unjoined.result, expected);
  assert_eq!(fast_unjoined.result, expected);

  let slow_join = event_position(&slow_unjoined.events, |payload| {
    matches!(payload, RunEventPayload::ForkJoinSettled(_))
  });
  let slow_finish = event_position(
    &slow_unjoined.events,
    |payload| matches!(payload, RunEventPayload::StepAttemptSucceeded(data) if data.node_id == "finish"),
  );
  let slow_facebook = event_position(
    &slow_unjoined.events,
    |payload| matches!(payload, RunEventPayload::StepAttemptSucceeded(data) if data.node_id == "publishFacebook"),
  );
  assert!(slow_join < slow_finish);
  assert!(slow_finish < slow_facebook);

  let fast_facebook = event_position(
    &fast_unjoined.events,
    |payload| matches!(payload, RunEventPayload::StepAttemptSucceeded(data) if data.node_id == "publishFacebook"),
  );
  let fast_join = event_position(&fast_unjoined.events, |payload| {
    matches!(payload, RunEventPayload::ForkJoinSettled(_))
  });
  assert!(fast_facebook < fast_join);

  for execution in [&slow_unjoined, &fast_unjoined] {
    let instagram = event_position(&execution.events, |payload| {
      matches!(payload, RunEventPayload::StepAttemptSucceeded(data)
        if data.node_id == "publishInstagram")
    });
    let instagram_branch = event_position(&execution.events, |payload| {
      matches!(payload, RunEventPayload::ForkBranchSettled(data)
        if data.branch_id == "instagram")
    });
    let join = event_position(&execution.events, |payload| {
      matches!(payload, RunEventPayload::ForkJoinSettled(_))
    });
    assert!(instagram < instagram_branch);
    assert!(instagram_branch < join);
    let last_branch = execution
      .events
      .iter()
      .rposition(|event| matches!(event.payload, RunEventPayload::ForkBranchSettled(_)))
      .unwrap();
    let outcome = event_position(&execution.events, |payload| {
      matches!(payload, RunEventPayload::RunOutcomeDecided(_))
    });
    assert!(last_branch < outcome);
  }
}

#[tokio::test]
async fn non_blocking_join_releases_immediately_but_final_success_waits_for_owned_branches() {
  let Some(_) = host_options() else {
    return;
  };
  let mut workflow = model();
  workflow.graph.forks.as_mut().unwrap()[0]
    .joined_branch_ids
    .clear();
  workflow.graph.edges.retain(|edge| {
    edge.id != "distribution:join:instagram" && edge.id != "distribution:join:facebook"
  });
  workflow.graph.edges.push(CompiledWorkflowEdge {
    id: "distribution:join:none".to_string(),
    from: "__woml_fork__distribution__open".to_string(),
    to: "__woml_fork__distribution__join".to_string(),
    condition: EdgeCondition::Always,
    branch_id: None,
    parallel_id: None,
    approval_id: None,
  });
  retain_finish_visibility(&mut workflow, &["prepare"]);
  set_script(
    &mut workflow,
    "publishInstagram",
    "await services.test.control(null); return { platform: 'instagram', sawFacebook: context.steps.publishFacebook !== undefined };".to_string(),
  );
  set_script(
    &mut workflow,
    "publishFacebook",
    "await services.test.control(null); return { platform: 'facebook', sawInstagram: context.steps.publishInstagram !== undefined };".to_string(),
  );
  set_script(
    &mut workflow,
    "finish",
    "return { accepted: context.steps.prepare.enabled, sawInstagram: context.steps.publishInstagram !== undefined, sawFacebook: context.steps.publishFacebook !== undefined };".to_string(),
  );
  let execution = execute_gated(workflow, "finish", 2).await;
  assert_eq!(
    execution.result,
    json!({ "accepted": true, "sawInstagram": false, "sawFacebook": false })
  );

  let join = event_position(&execution.events, |payload| {
    matches!(payload, RunEventPayload::ForkJoinSettled(_))
  });
  let first_attempt = event_position(
    &execution.events,
    |payload| matches!(payload, RunEventPayload::StepAttemptStarted(data) if data.node_id == "publishInstagram" || data.node_id == "publishFacebook" || data.node_id == "finish"),
  );
  let finish = event_position(
    &execution.events,
    |payload| matches!(payload, RunEventPayload::StepAttemptSucceeded(data) if data.node_id == "finish"),
  );
  let instagram = event_position(
    &execution.events,
    |payload| matches!(payload, RunEventPayload::StepAttemptSucceeded(data) if data.node_id == "publishInstagram"),
  );
  let facebook = event_position(
    &execution.events,
    |payload| matches!(payload, RunEventPayload::StepAttemptSucceeded(data) if data.node_id == "publishFacebook"),
  );
  let outcome = event_position(&execution.events, |payload| {
    matches!(payload, RunEventPayload::RunOutcomeDecided(_))
  });
  assert!(join < first_attempt);
  assert!(finish < instagram);
  assert!(finish < facebook);
  assert!(instagram < outcome);
  assert!(facebook < outcome);
}
