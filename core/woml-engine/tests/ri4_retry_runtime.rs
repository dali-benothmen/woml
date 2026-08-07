use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{json, Map, Value};
use uuid::Uuid;
use woml_engine::event::{StepAttemptFailedData, StepAttemptSucceededData};
use woml_engine::model::{BackoffPolicy, RetryPolicy, ValueExpression};
use woml_engine::{
  execute_workflow_durable, resume_workflow_durable, AttemptFailure, AttemptFailureKind,
  CompiledWorkflowDefinition, DurableDagEngine, DurableEventStore, ExecutionProgress,
  RunEventPayload, RunStatus, RuntimeExecutionError, RuntimeExecutionOptions,
  ScriptHostProcessOptions, StepFailureDisposition,
};

const RETRY_MODEL: &str = include_str!("../../../woml/tests/fixtures/retry.compiled.v6.json");
const RETRY_HASH: &str = "sha256:27606cefeebc5b6d45c965969b621a2f74ae2ebebe2b94edec80d97bfeb8378c";
const MODIFIED_HASH: &str =
  "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn retry_model() -> CompiledWorkflowDefinition {
  serde_json::from_str(RETRY_MODEL).unwrap()
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

fn options(host: ScriptHostProcessOptions, timeout_ms: u64) -> RuntimeExecutionOptions {
  RuntimeExecutionOptions::new(host, timeout_ms)
}

fn greet_node(
  workflow: &mut CompiledWorkflowDefinition,
) -> &mut woml_engine::model::CompiledWorkflowNode {
  workflow
    .graph
    .nodes
    .iter_mut()
    .find(|node| node.id == "greet")
    .unwrap()
}

fn set_greet_script(workflow: &mut CompiledWorkflowDefinition, source: &str) {
  let ValueExpression::Object { fields } = &mut greet_node(workflow).inputs else {
    panic!("greet inputs must be an object");
  };
  fields.insert(
    "source".to_string(),
    ValueExpression::Literal {
      value: Value::String(source.to_string()),
    },
  );
}

fn use_fast_fixed_retry(workflow: &mut CompiledWorkflowDefinition) {
  greet_node(workflow).retry_policy = Some(RetryPolicy {
    max_attempts: 3,
    backoff: BackoffPolicy::Fixed { delay_ms: 1 },
  });
}

struct TemporaryDatabase {
  path: PathBuf,
}

impl TemporaryDatabase {
  fn new(label: &str) -> Self {
    Self {
      path: std::env::temp_dir().join(format!(
        "woml-ri4-{label}-{}.sqlite",
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
  }
}

fn event_name(payload: &RunEventPayload) -> &'static str {
  match payload {
    RunEventPayload::RunStarted(_) => "run_started",
    RunEventPayload::StepAttemptStarted(_) => "step_attempt_started",
    RunEventPayload::StepAttemptSucceeded(_) => "step_attempt_succeeded",
    RunEventPayload::StepAttemptFailed(_) => "step_attempt_failed",
    RunEventPayload::StepRetryScheduled(_) => "step_retry_scheduled",
    RunEventPayload::RunSucceeded(_) => "run_succeeded",
    RunEventPayload::RunFailed(_) => "run_failed",
    _ => "other",
  }
}

#[tokio::test]
async fn sequential_retry_fails_twice_then_publishes_one_final_output() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("success");
  let progress = Arc::new(Mutex::new(Vec::new()));
  let reported = Arc::clone(&progress);
  let result = execute_workflow_durable(
    retry_model(),
    RETRY_HASH.to_string(),
    Map::new(),
    options(host, 2_000).with_progress_reporter(Arc::new(move |message| {
      reported.lock().unwrap().push(message);
    })),
    database.path().to_path_buf(),
  )
  .await
  .unwrap();

  assert_eq!(result.result, json!({ "message": "Hello World" }));
  assert_eq!(result.context.steps["greet"], result.result);
  assert_eq!(result.execution_order, ["prepare", "greet"]);
  assert_eq!(
    result
      .events
      .iter()
      .map(|event| event_name(&event.payload))
      .collect::<Vec<_>>(),
    [
      "run_started",
      "step_attempt_started",
      "step_attempt_succeeded",
      "step_attempt_started",
      "step_attempt_failed",
      "step_retry_scheduled",
      "step_attempt_started",
      "step_attempt_failed",
      "step_retry_scheduled",
      "step_attempt_started",
      "step_attempt_succeeded",
      "run_succeeded",
    ]
  );

  let greet_starts = result
    .events
    .iter()
    .filter_map(|event| match &event.payload {
      RunEventPayload::StepAttemptStarted(data) if data.node_id == "greet" => {
        Some((event.occurred_at, data))
      }
      _ => None,
    })
    .collect::<Vec<_>>();
  assert_eq!(
    greet_starts
      .iter()
      .map(|(_, data)| data.attempt)
      .collect::<Vec<_>>(),
    [1, 2, 3]
  );
  assert_eq!(
    greet_starts
      .iter()
      .map(|(_, data)| data.idempotency_key.as_deref().unwrap())
      .collect::<std::collections::HashSet<_>>()
      .len(),
    1
  );
  assert_eq!(
    greet_starts
      .iter()
      .map(|(_, data)| data.invocation_id.as_str())
      .collect::<std::collections::HashSet<_>>()
      .len(),
    3
  );

  let schedules = result
    .events
    .iter()
    .filter_map(|event| match &event.payload {
      RunEventPayload::StepRetryScheduled(data) => Some(data),
      _ => None,
    })
    .collect::<Vec<_>>();
  assert_eq!(schedules.len(), 2);
  assert!(greet_starts[1].0 >= schedules[0].scheduled_at);
  assert!(greet_starts[2].0 >= schedules[1].scheduled_at);

  let store = DurableEventStore::open(database.path()).unwrap();
  let projection = store.projection(&result.run_id).unwrap();
  assert_eq!(projection.status, RunStatus::Succeeded);
  assert!(projection.pending_retries.is_empty());
  let progress = progress.lock().unwrap();
  assert_eq!(progress.len(), 5);
  assert!(matches!(
    &progress[0],
    ExecutionProgress::StepAttemptFailed {
      node_id,
      attempt: 1,
      max_attempts: 3,
      ..
    } if node_id == "greet"
  ));
  assert!(matches!(
    &progress[4],
    ExecutionProgress::StepAttemptSucceeded {
      node_id,
      attempt: 3,
      max_attempts: 3,
      ..
    } if node_id == "greet"
  ));
}

#[tokio::test]
async fn retry_exhaustion_never_starts_attempt_four() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("exhausted");
  let mut workflow = retry_model();
  use_fast_fixed_retry(&mut workflow);
  set_greet_script(&mut workflow, "throw new Error('still unavailable');");
  let error = execute_workflow_durable(
    workflow,
    MODIFIED_HASH.to_string(),
    Map::new(),
    options(host, 2_000),
    database.path().to_path_buf(),
  )
  .await
  .unwrap_err();
  let RuntimeExecutionError::RunFailed(details) = error else {
    panic!("retry exhaustion must fail the run");
  };
  assert_eq!(details.code, "WOML_STEP_RETRIES_EXHAUSTED");
  assert_eq!(details.node_id.as_deref(), Some("greet"));
  assert_eq!(details.attempt, Some(3));
  assert_eq!(details.max_attempts, Some(3));
  assert_eq!(
    details.message,
    "attempt 3 of 3 failed [WOML_SCRIPT_THROWN]."
  );
  assert_eq!(details.failure.kind, AttemptFailureKind::ScriptThrew);
  let attempts = details
    .events
    .iter()
    .filter_map(|event| match &event.payload {
      RunEventPayload::StepAttemptStarted(data) if data.node_id == "greet" => Some(data.attempt),
      _ => None,
    })
    .collect::<Vec<_>>();
  assert_eq!(attempts, [1, 2, 3]);
  assert_eq!(
    details
      .events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::StepRetryScheduled(_)))
      .count(),
    2
  );
  assert!(matches!(
    details.events.last().map(|event| &event.payload),
    Some(RunEventPayload::RunFailed(_))
  ));
}

#[tokio::test]
async fn non_retryable_timeout_fails_after_the_first_attempt() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("timeout");
  let mut workflow = retry_model();
  use_fast_fixed_retry(&mut workflow);
  set_greet_script(&mut workflow, "while (true) {}");
  let error = execute_workflow_durable(
    workflow,
    MODIFIED_HASH.to_string(),
    Map::new(),
    options(host, 25),
    database.path().to_path_buf(),
  )
  .await
  .unwrap_err();
  let RuntimeExecutionError::RunFailed(details) = error else {
    panic!("timeout must fail the run");
  };
  assert_eq!(details.failure.kind, AttemptFailureKind::ScriptTimedOut);
  assert_eq!(
    details
      .events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::StepRetryScheduled(_)))
      .count(),
    0
  );
  assert_eq!(
    details
      .events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::StepAttemptStarted(_)))
      .count(),
    2
  );
}

#[tokio::test]
async fn recovery_after_retry_success_dispatches_only_the_downstream_step() {
  let Some(host) = host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("before-downstream");
  let run_id = "run_ri6_before_downstream";
  let now = chrono::Utc::now();
  let mut workflow = retry_model();
  use_fast_fixed_retry(&mut workflow);
  let mut finish = workflow.graph.nodes[0].clone();
  finish.id = "finish".to_string();
  finish.retry_policy = None;
  let ValueExpression::Object { fields } = &mut finish.inputs else {
    unreachable!()
  };
  fields.insert(
    "source".to_string(),
    ValueExpression::Literal {
      value: Value::String("return { final: context.steps.greet.message };".to_string()),
    },
  );
  let mut edge = workflow.graph.edges[0].clone();
  edge.id = "greet-to-finish".to_string();
  edge.from = "greet".to_string();
  edge.to = "finish".to_string();
  workflow.graph.nodes.push(finish);
  workflow.graph.edges.push(edge);
  workflow.validate_for_durable_execution().unwrap();

  {
    let store = DurableEventStore::open(database.path()).unwrap();
    let mut engine = DurableDagEngine::new(workflow, MODIFIED_HASH, store).unwrap();
    engine
      .start_run("evt_start", run_id, now, Map::new())
      .unwrap();
    engine
      .start_step_attempt(run_id, "prepare", 1, "inv_prepare", now)
      .unwrap();
    engine
      .append_payload(
        "evt_prepare_success",
        run_id,
        now,
        RunEventPayload::StepAttemptSucceeded(StepAttemptSucceededData {
          node_id: "prepare".to_string(),
          attempt: 1,
          invocation_id: "inv_prepare".to_string(),
          output: json!({ "name": "World" }),
        }),
      )
      .unwrap();
    engine
      .start_step_attempt(run_id, "greet", 1, "inv_greet_1", now)
      .unwrap();
    let failed = engine
      .record_step_attempt_failure(
        run_id,
        now,
        StepAttemptFailedData {
          node_id: "greet".to_string(),
          attempt: 1,
          invocation_id: "inv_greet_1".to_string(),
          failure: AttemptFailure {
            kind: AttemptFailureKind::ScriptThrew,
            code: "WOML_SCRIPT_THROWN".to_string(),
            message: "temporary".to_string(),
            details: None,
          },
        },
      )
      .unwrap();
    let StepFailureDisposition::RetryScheduled { scheduled_at, .. } = failed.disposition else {
      unreachable!()
    };
    engine
      .start_step_attempt(run_id, "greet", 2, "inv_greet_2", scheduled_at)
      .unwrap();
    engine
      .append_payload(
        "evt_greet_success",
        run_id,
        scheduled_at,
        RunEventPayload::StepAttemptSucceeded(StepAttemptSucceededData {
          node_id: "greet".to_string(),
          attempt: 2,
          invocation_id: "inv_greet_2".to_string(),
          output: json!({ "message": "Hello World" }),
        }),
      )
      .unwrap();
  }

  let mut store = DurableEventStore::open(database.path()).unwrap();
  let recovery = store.recover_interrupted_runs().unwrap();
  assert_eq!(recovery.resumable_runs, 1);
  assert_eq!(store.projection(run_id).unwrap().status, RunStatus::Running);
  drop(store);

  let result = resume_workflow_durable(database.path().to_path_buf(), run_id, options(host, 2_000))
    .await
    .unwrap();
  assert_eq!(result.execution_order, ["prepare", "greet", "finish"]);
  assert_eq!(result.result, json!({ "final": "Hello World" }));
  assert_eq!(
    result.context.steps["greet"],
    json!({ "message": "Hello World" })
  );
  assert_eq!(
    result
      .events
      .iter()
      .filter(|event| matches!(
        &event.payload,
        RunEventPayload::StepAttemptStarted(data) if data.node_id == "greet"
      ))
      .count(),
    2
  );
}
