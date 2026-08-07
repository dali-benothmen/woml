use std::collections::HashSet;
use std::future::{poll_fn, Future};
use std::path::PathBuf;
use std::pin::Pin;
use std::task::Poll;

use serde::Serialize;
use serde_json::{Map, Value};
use thiserror::Error;
use uuid::Uuid;

use crate::engine::{
  resolve_context_reference, selected_branch_arm, BranchEvaluationError, BranchEvaluationErrorKind,
};
use crate::event::{
  ApprovalFailure, BranchSelectedData, ParallelFailure, ParallelFailurePolicy,
  ParallelGroupCompletedData, ParallelGroupOutcome, ParallelGroupStartedData, RunFailedData,
  RunFailedDataV1, RunFailedDataV2, RunFailedDataV3, RunSucceededData, StepAttemptFailedData,
  StepAttemptStartedData, StepAttemptSucceededData,
};
use crate::model::{ParallelGroupDefinition, ValueExpression};
use crate::projection::AttemptStatus;
use crate::protocol::{ExecuteMessage, HostOutcome};
use crate::{
  run_event_schema_version_for_model, AttemptFailure, AttemptFailureKind, BranchFailure,
  CompiledWorkflowDefinition, DurableDagEngine, DurableEngineError, DurableEventStore,
  DurableStoreError, EngineError, InMemoryDagEngine, RecoveryReport, RunEvent, RunEventPayload,
  RunFailure, RunProjection, RunStatus, ScriptHostClient, ScriptHostClientError,
  ScriptHostProcessOptions, WorkflowContext, RUN_EVENT_SCHEMA_VERSION_V1,
  RUN_EVENT_SCHEMA_VERSION_V2,
};

#[derive(Debug, Clone)]
pub struct RuntimeExecutionOptions {
  pub script_host: ScriptHostProcessOptions,
  pub script_timeout_ms: u64,
  pub max_context_bytes: Option<usize>,
}

impl RuntimeExecutionOptions {
  pub fn new(script_host: ScriptHostProcessOptions, script_timeout_ms: u64) -> Self {
    Self {
      script_host,
      script_timeout_ms,
      max_context_bytes: None,
    }
  }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowExecutionResult {
  pub workflow_id: String,
  pub run_id: String,
  pub terminal_node_id: String,
  pub result: Value,
  pub context: WorkflowContext,
  pub execution_order: Vec<String>,
  pub events: Vec<RunEvent>,
}

#[derive(Debug, Error)]
pub enum RuntimeExecutionError {
  #[error(transparent)]
  Engine(#[from] EngineError),
  #[error(transparent)]
  DurableEngine(#[from] DurableEngineError),
  #[error(transparent)]
  DurableStore(#[from] DurableStoreError),
  #[error(transparent)]
  Host(#[from] ScriptHostClientError),
  #[error(transparent)]
  RunFailed(Box<FailedRunDetails>),
  #[error(transparent)]
  BranchFailed(Box<FailedBranchDetails>),
  #[error(transparent)]
  ParallelFailed(Box<FailedParallelDetails>),
  #[error(transparent)]
  ApprovalFailed(Box<FailedApprovalDetails>),
  #[error("workflow execution stalled: {0}")]
  Stalled(String),
  #[error("runtime configuration is invalid: {0}")]
  InvalidConfiguration(String),
}

#[derive(Debug, Error)]
#[error("workflow execution failed [{code}]: {message}")]
pub struct FailedRunDetails {
  pub code: String,
  pub message: String,
  pub failure: AttemptFailure,
  pub events: Vec<RunEvent>,
}

#[derive(Debug, Error)]
#[error("workflow branch failed [{code}]: {message}")]
pub struct FailedBranchDetails {
  pub code: String,
  pub message: String,
  pub branch_id: String,
  pub arm_id: Option<String>,
  pub path: Option<Vec<String>>,
  pub site: BranchFailureSite,
  pub failure: BranchFailure,
  pub events: Vec<RunEvent>,
}

#[derive(Debug, Error)]
#[error("workflow parallel group failed [{code}]: {message}")]
pub struct FailedParallelDetails {
  pub code: String,
  pub message: String,
  pub parallel_id: String,
  pub policy: ParallelFailurePolicy,
  pub primary_node_id: String,
  pub failed_node_ids: Vec<String>,
  pub cancelled_node_ids: Vec<String>,
  pub failure: ParallelFailure,
  pub events: Vec<RunEvent>,
}

#[derive(Debug, Error)]
#[error("workflow approval failed [{code}]: {message}")]
pub struct FailedApprovalDetails {
  pub code: String,
  pub message: String,
  pub approval_id: String,
  pub request_id: String,
  pub failure: ApprovalFailure,
  pub events: Vec<RunEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchFailureSite {
  Test,
  Result,
  Selection,
}

impl BranchFailureSite {
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::Test => "test",
      Self::Result => "result",
      Self::Selection => "selection",
    }
  }
}

pub async fn execute_workflow(
  workflow: CompiledWorkflowDefinition,
  definition_hash: String,
  trigger: Map<String, Value>,
  options: RuntimeExecutionOptions,
) -> Result<WorkflowExecutionResult, RuntimeExecutionError> {
  let engine = InMemoryDagEngine::new(workflow, definition_hash)?;
  execute_with_engine(engine, trigger, options).await
}

pub async fn execute_workflow_durable(
  workflow: CompiledWorkflowDefinition,
  definition_hash: String,
  trigger: Map<String, Value>,
  options: RuntimeExecutionOptions,
  database_path: PathBuf,
) -> Result<WorkflowExecutionResult, RuntimeExecutionError> {
  let mut store = DurableEventStore::open(database_path)?;
  store.recover_interrupted_runs()?;
  let engine = DurableDagEngine::new(workflow, definition_hash, store)?;
  execute_with_engine(engine, trigger, options).await
}

pub async fn resume_workflow_durable(
  database_path: PathBuf,
  run_id: &str,
  options: RuntimeExecutionOptions,
) -> Result<WorkflowExecutionResult, RuntimeExecutionError> {
  let mut store = DurableEventStore::open(database_path)?;
  store.recover_interrupted_runs()?;
  let engine = DurableDagEngine::resume(store, run_id)?;
  resume_with_engine(engine, run_id, options).await
}

pub fn recover_durable_runs(
  database_path: PathBuf,
) -> Result<RecoveryReport, RuntimeExecutionError> {
  let mut store = DurableEventStore::open(database_path)?;
  Ok(store.recover_interrupted_runs()?)
}

async fn execute_with_engine<E: RuntimeDagEngine>(
  mut engine: E,
  trigger: Map<String, Value>,
  options: RuntimeExecutionOptions,
) -> Result<WorkflowExecutionResult, RuntimeExecutionError> {
  if options.script_timeout_ms == 0 {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "script_timeout_ms must be greater than zero".to_string(),
    ));
  }

  let host = ScriptHostClient::spawn(options.script_host.clone()).await?;
  let execution = execute_with_host(&mut engine, trigger, &options, &host).await;
  host.shutdown().await;
  execution
}

async fn resume_with_engine<E: RuntimeDagEngine>(
  mut engine: E,
  run_id: &str,
  options: RuntimeExecutionOptions,
) -> Result<WorkflowExecutionResult, RuntimeExecutionError> {
  if options.script_timeout_ms == 0 {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "script_timeout_ms must be greater than zero".to_string(),
    ));
  }
  let projection = engine.projection(run_id)?;
  let execution_order = recorded_execution_order(&engine.events(run_id)?);
  match projection.status {
    RunStatus::Succeeded => return final_result(&engine, run_id, execution_order),
    RunStatus::Failed => return Err(resumed_failure(&engine, run_id, projection)?),
    RunStatus::Running => {}
    RunStatus::Waiting => {
      return Err(RuntimeExecutionError::Stalled(
        "stored run is waiting for approval; runtime continuation begins in A4".to_string(),
      ));
    }
    RunStatus::NotStarted => {
      return Err(RuntimeExecutionError::Stalled(
        "stored run has no run_started event".to_string(),
      ));
    }
  }
  let terminal_node_id = engine
    .workflow()
    .terminal_node_id()
    .ok_or_else(|| RuntimeExecutionError::Stalled("no terminal node exists".to_string()))?
    .to_string();
  let host = ScriptHostClient::spawn(options.script_host.clone()).await?;
  let execution = continue_with_host(
    &mut engine,
    run_id,
    terminal_node_id,
    execution_order,
    &options,
    &host,
  )
  .await;
  host.shutdown().await;
  execution
}

async fn execute_with_host<E: RuntimeDagEngine>(
  engine: &mut E,
  trigger: Map<String, Value>,
  options: &RuntimeExecutionOptions,
  host: &ScriptHostClient,
) -> Result<WorkflowExecutionResult, RuntimeExecutionError> {
  let terminal_node_id = engine
    .workflow()
    .terminal_node_id()
    .ok_or_else(|| RuntimeExecutionError::Stalled("no terminal node exists".to_string()))?
    .to_string();
  let run_id = generated_id("run");
  engine.start_run(&run_id, trigger)?;
  continue_with_host(engine, &run_id, terminal_node_id, Vec::new(), options, host).await
}

async fn continue_with_host<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  terminal_node_id: String,
  mut execution_order: Vec<String>,
  options: &RuntimeExecutionOptions,
  host: &ScriptHostClient,
) -> Result<WorkflowExecutionResult, RuntimeExecutionError> {
  loop {
    let ready = engine.ready_node_ids(run_id)?;
    let Some(node_id) = ready.first() else {
      let projection = engine.projection(run_id)?;
      if projection.status == RunStatus::Succeeded {
        return final_result(engine, run_id, execution_order);
      }
      return Err(RuntimeExecutionError::Stalled(
        "no node is ready before the run reached a terminal state".to_string(),
      ));
    };
    if let Some(group) = engine.workflow().parallel_group_for_child(node_id) {
      if ready.iter().any(|ready_id| {
        engine
          .workflow()
          .parallel_group_for_child(ready_id)
          .is_none_or(|owner| owner.parallel_id != group.parallel_id)
      }) {
        return Err(RuntimeExecutionError::Stalled(format!(
          "parallel group {:?} became ready alongside an unrelated node",
          group.parallel_id
        )));
      }
      let completed = execute_parallel_group(engine, run_id, group, options, host).await?;
      execution_order.extend(completed);
      continue;
    }
    if ready.len() != 1 {
      return Err(RuntimeExecutionError::Stalled(
        "the current runtime received more than one ready node".to_string(),
      ));
    }

    let (handler, inputs, source) = {
      let node = engine
        .workflow()
        .node(node_id)
        .ok_or_else(|| RuntimeExecutionError::Stalled("ready node disappeared".to_string()))?;
      (
        node.handler.clone(),
        node.inputs.clone(),
        node.script_source().map(str::to_string),
      )
    };

    match handler.as_str() {
      "engine.parallel-start" => {
        let parallel_id = node_id
          .strip_prefix("__woml_parallel__")
          .and_then(|id| id.strip_suffix("__start"))
          .ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "parallel start node {node_id:?} has no canonical group identity"
            ))
          })?
          .to_string();
        engine.append_payload(
          run_id,
          RunEventPayload::ParallelGroupStarted(ParallelGroupStartedData { parallel_id }),
        )?;
      }
      "engine.branch-select" => {
        let context = engine.projection(run_id)?.context;
        match selected_branch_arm(engine.workflow(), node_id, &context) {
          Ok(arm_id) => {
            let branch_id = crate::engine::selector_branch_id(node_id)
              .ok_or_else(|| {
                RuntimeExecutionError::Stalled(format!(
                  "selector node {node_id:?} has no canonical branch identity"
                ))
              })?
              .to_string();
            engine.append_payload(
              run_id,
              RunEventPayload::BranchSelected(BranchSelectedData { branch_id, arm_id }),
            )?;
          }
          Err(error) => {
            let site = if matches!(error.kind, BranchEvaluationErrorKind::SelectionInvalid) {
              BranchFailureSite::Selection
            } else {
              BranchFailureSite::Test
            };
            return fail_branch(engine, run_id, error, site);
          }
        }
      }
      "engine.branch-result" => {
        let projection = engine.projection(run_id)?;
        let arm_id = projection
          .branch_selections
          .get(node_id)
          .cloned()
          .ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "branch result {node_id:?} became ready without a recorded selection"
            ))
          })?;
        let ValueExpression::Object { fields } = &inputs else {
          return Err(RuntimeExecutionError::Stalled(format!(
            "branch result {node_id:?} has invalid compiled inputs"
          )));
        };
        let expression = fields.get(&arm_id).ok_or_else(|| {
          RuntimeExecutionError::Stalled(format!(
            "branch result {node_id:?} has no value for selected arm {arm_id:?}"
          ))
        })?;
        let output = match resolve_context_reference(expression, &projection.context) {
          Ok(output) => output,
          Err(error) => {
            return fail_branch(
              engine,
              run_id,
              BranchEvaluationError {
                branch_id: node_id.clone(),
                arm_id: Some(arm_id),
                path: Some(error.path),
                kind: BranchEvaluationErrorKind::ReferenceNotAvailable,
              },
              BranchFailureSite::Result,
            );
          }
        };
        engine.publish_pure_result(run_id, node_id, output.clone())?;
        execution_order.push(node_id.clone());
        if node_id == &terminal_node_id {
          engine.append_payload(
            run_id,
            RunEventPayload::RunSucceeded(RunSucceededData {
              terminal_node_id: terminal_node_id.clone(),
              result: output,
            }),
          )?;
          return final_result(engine, run_id, execution_order);
        }
      }
      "runtime.script" => {
        let source = source.ok_or_else(|| {
          RuntimeExecutionError::Stalled(format!("node {node_id:?} has no script source"))
        })?;
        let output = execute_script_node(engine, run_id, node_id, &source, options, host).await?;
        execution_order.push(node_id.clone());
        if node_id == &terminal_node_id {
          engine.append_payload(
            run_id,
            RunEventPayload::RunSucceeded(RunSucceededData {
              terminal_node_id: terminal_node_id.clone(),
              result: output,
            }),
          )?;
          return final_result(engine, run_id, execution_order);
        }
      }
      _ => {
        return Err(RuntimeExecutionError::Stalled(format!(
          "ready node {node_id:?} uses unknown handler {handler:?}"
        )));
      }
    }
  }
}

#[derive(Debug)]
struct ParallelInvocationCompletion {
  node_id: String,
  invocation_id: String,
  outcome: Result<Value, AttemptFailure>,
}

type ParallelInvocationFuture<'a> =
  Pin<Box<dyn Future<Output = ParallelInvocationCompletion> + Send + 'a>>;

struct ActiveParallelInvocation<'a> {
  invocation_id: String,
  future: ParallelInvocationFuture<'a>,
}

struct ParallelInvocationRequest {
  run_id: String,
  node_id: String,
  invocation_id: String,
  source: String,
  context: WorkflowContext,
  timeout_ms: u64,
  max_context_bytes: Option<usize>,
}

async fn next_parallel_completion(
  active: &mut Vec<ActiveParallelInvocation<'_>>,
) -> Option<ParallelInvocationCompletion> {
  poll_fn(|context| {
    for index in 0..active.len() {
      if let Poll::Ready(completion) = active[index].future.as_mut().poll(context) {
        drop(active.swap_remove(index));
        return Poll::Ready(Some(completion));
      }
    }
    if active.is_empty() {
      Poll::Ready(None)
    } else {
      Poll::Pending
    }
  })
  .await
}

async fn invoke_parallel_child(
  host: &ScriptHostClient,
  request: ParallelInvocationRequest,
) -> ParallelInvocationCompletion {
  let ParallelInvocationRequest {
    run_id,
    node_id,
    invocation_id,
    source,
    context,
    timeout_ms,
    max_context_bytes,
  } = request;
  let outcome = if let Some(limit) = max_context_bytes {
    match serde_json::to_vec(&context) {
      Ok(encoded) if encoded.len() > limit => Err(AttemptFailure {
        kind: AttemptFailureKind::ContextTooLarge,
        code: AttemptFailureKind::ContextTooLarge.code().to_string(),
        message: "Invocation context exceeds the configured byte limit.".to_string(),
        details: Some(crate::FailureSizeDetails {
          actual_bytes: Some(encoded.len() as u64),
          limit_bytes: Some(limit as u64),
        }),
      }),
      Err(error) => Err(AttemptFailure {
        kind: AttemptFailureKind::InvalidScriptResult,
        code: AttemptFailureKind::InvalidScriptResult.code().to_string(),
        message: format!("Invocation context could not be encoded: {error}"),
        details: None,
      }),
      _ => {
        execute_parallel_request(
          host,
          &run_id,
          &node_id,
          &invocation_id,
          &source,
          &context,
          timeout_ms,
        )
        .await
      }
    }
  } else {
    execute_parallel_request(
      host,
      &run_id,
      &node_id,
      &invocation_id,
      &source,
      &context,
      timeout_ms,
    )
    .await
  };
  ParallelInvocationCompletion {
    node_id,
    invocation_id,
    outcome,
  }
}

async fn execute_parallel_request(
  host: &ScriptHostClient,
  run_id: &str,
  node_id: &str,
  invocation_id: &str,
  source: &str,
  context: &WorkflowContext,
  timeout_ms: u64,
) -> Result<Value, AttemptFailure> {
  let request =
    ExecuteMessage::runtime_script(invocation_id, run_id, node_id, timeout_ms, source, context);
  match host.execute(&request).await {
    Ok(completed) => match completed.outcome {
      HostOutcome::Success { value } => Ok(value),
      HostOutcome::Failure { error } => Err(error.into_attempt_failure()),
    },
    Err(error) => Err(AttemptFailure {
      kind: AttemptFailureKind::HostCrashed,
      code: AttemptFailureKind::HostCrashed.code().to_string(),
      message: error.to_string(),
      details: None,
    }),
  }
}

async fn execute_parallel_group<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  group: ParallelGroupDefinition,
  options: &RuntimeExecutionOptions,
  host: &ScriptHostClient,
) -> Result<Vec<String>, RuntimeExecutionError> {
  let policy = match group.on_error.as_str() {
    "fail-fast" => ParallelFailurePolicy::FailFast,
    "wait-all" => ParallelFailurePolicy::WaitAll,
    _ => {
      return Err(RuntimeExecutionError::Stalled(format!(
        "parallel group {:?} has an unknown failure policy",
        group.parallel_id
      )));
    }
  };
  let projection = engine.projection(run_id)?;
  let fork_context = projection
    .parallel_groups
    .get(&group.parallel_id)
    .ok_or_else(|| {
      RuntimeExecutionError::Stalled(format!(
        "parallel group {:?} has no durable fork event",
        group.parallel_id
      ))
    })?
    .fork_context
    .clone();
  let attempted = projection.attempted_node_ids();
  let child_ids = group
    .child_node_ids
    .iter()
    .filter(|node_id| !attempted.contains(node_id.as_str()))
    .cloned()
    .collect::<Vec<_>>();
  drop(attempted);

  let mut next_child = 0;
  let mut failed_nodes = HashSet::new();
  let mut cancelled_nodes = HashSet::new();
  for attempt in &projection.attempts {
    if !group.child_node_ids.contains(&attempt.identity.node_id) {
      continue;
    }
    if let AttemptStatus::Failed { failure } = &attempt.status {
      if failure.kind == AttemptFailureKind::InvocationCancelled {
        cancelled_nodes.insert(attempt.identity.node_id.clone());
      } else {
        failed_nodes.insert(attempt.identity.node_id.clone());
      }
    }
  }
  let mut primary_failure = engine.events(run_id)?.iter().find_map(|event| {
    if let RunEventPayload::StepAttemptFailed(data) = &event.payload {
      failed_nodes
        .contains(&data.node_id)
        .then(|| data.node_id.clone())
    } else {
      None
    }
  });
  let mut completion_order = Vec::new();
  let mut active: Vec<ActiveParallelInvocation<'_>> = Vec::new();

  loop {
    let may_schedule = policy == ParallelFailurePolicy::WaitAll || primary_failure.is_none();
    while may_schedule && active.len() < group.concurrency && next_child < child_ids.len() {
      let node_id = child_ids[next_child].clone();
      next_child += 1;
      let source = engine
        .workflow()
        .node(&node_id)
        .and_then(|node| node.script_source())
        .ok_or_else(|| {
          RuntimeExecutionError::Stalled(format!("parallel child {node_id:?} has no script source"))
        })?
        .to_string();
      let invocation_id = generated_id("inv");
      engine.append_payload(
        run_id,
        RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
          node_id: node_id.clone(),
          attempt: 1,
          invocation_id: invocation_id.clone(),
          handler: "runtime.script".to_string(),
        }),
      )?;
      active.push(ActiveParallelInvocation {
        invocation_id: invocation_id.clone(),
        future: Box::pin(invoke_parallel_child(
          host,
          ParallelInvocationRequest {
            run_id: run_id.to_string(),
            node_id,
            invocation_id,
            source,
            context: fork_context.clone(),
            timeout_ms: options.script_timeout_ms,
            max_context_bytes: options.max_context_bytes,
          },
        )),
      });
    }

    let Some(completion) = next_parallel_completion(&mut active).await else {
      break;
    };
    completion_order.push(completion.node_id.clone());
    match completion.outcome {
      Ok(output) => engine.append_payload(
        run_id,
        RunEventPayload::StepAttemptSucceeded(StepAttemptSucceededData {
          node_id: completion.node_id,
          attempt: 1,
          invocation_id: completion.invocation_id,
          output,
        }),
      )?,
      Err(failure) => {
        let node_id = completion.node_id.clone();
        let cancellation = failure.kind == AttemptFailureKind::InvocationCancelled;
        engine.append_payload(
          run_id,
          RunEventPayload::StepAttemptFailed(StepAttemptFailedData {
            node_id: completion.node_id,
            attempt: 1,
            invocation_id: completion.invocation_id,
            failure,
          }),
        )?;
        if cancellation {
          cancelled_nodes.insert(node_id);
        } else {
          failed_nodes.insert(node_id.clone());
          if primary_failure.is_none() {
            primary_failure = Some(node_id);
            if policy == ParallelFailurePolicy::FailFast {
              let active_invocation_ids = active
                .iter()
                .map(|invocation| invocation.invocation_id.clone())
                .collect::<Vec<_>>();
              for invocation_id in active_invocation_ids {
                let _ = host.cancel(&invocation_id).await;
              }
            }
          }
        }
      }
    }
  }

  if let Some(primary_node_id) = primary_failure {
    let failed_node_ids = group
      .child_node_ids
      .iter()
      .filter(|node_id| failed_nodes.contains(*node_id))
      .cloned()
      .collect::<Vec<_>>();
    let cancelled_node_ids = group
      .child_node_ids
      .iter()
      .filter(|node_id| cancelled_nodes.contains(*node_id))
      .cloned()
      .collect::<Vec<_>>();
    let message = match policy {
      ParallelFailurePolicy::FailFast => {
        "A parallel child failed; active siblings were cancelled.".to_string()
      }
      ParallelFailurePolicy::WaitAll if failed_node_ids.len() == 1 => {
        "One parallel child failed.".to_string()
      }
      ParallelFailurePolicy::WaitAll => {
        format!("{} parallel children failed.", failed_node_ids.len())
      }
    };
    let failure = ParallelFailure {
      kind: "parallel_child_failed".to_string(),
      code: "WOML_PARALLEL_CHILD_FAILED".to_string(),
      message: message.clone(),
    };
    engine.append_payloads(
      run_id,
      vec![
        RunEventPayload::ParallelGroupCompleted(ParallelGroupCompletedData {
          parallel_id: group.parallel_id.clone(),
          outcome: ParallelGroupOutcome::Failed,
          failed_node_ids: failed_node_ids.clone(),
          cancelled_node_ids: cancelled_node_ids.clone(),
        }),
        RunEventPayload::RunFailed(RunFailedData::V3(RunFailedDataV3::Parallel {
          parallel_id: group.parallel_id.clone(),
          policy,
          primary_node_id: primary_node_id.clone(),
          failed_node_ids: failed_node_ids.clone(),
          cancelled_node_ids: cancelled_node_ids.clone(),
          failure: failure.clone(),
        })),
      ],
    )?;
    return Err(RuntimeExecutionError::ParallelFailed(Box::new(
      FailedParallelDetails {
        code: failure.code.clone(),
        message,
        parallel_id: group.parallel_id,
        policy,
        primary_node_id,
        failed_node_ids,
        cancelled_node_ids,
        failure,
        events: engine.events(run_id)?,
      },
    )));
  }
  if next_child != child_ids.len() {
    return Err(RuntimeExecutionError::Stalled(format!(
      "parallel group {:?} stopped before every child was scheduled",
      group.parallel_id
    )));
  }
  engine.append_payload(
    run_id,
    RunEventPayload::ParallelGroupCompleted(ParallelGroupCompletedData {
      parallel_id: group.parallel_id,
      outcome: ParallelGroupOutcome::Succeeded,
      failed_node_ids: Vec::new(),
      cancelled_node_ids: Vec::new(),
    }),
  )?;
  Ok(completion_order)
}

async fn execute_script_node<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  node_id: &str,
  source: &str,
  options: &RuntimeExecutionOptions,
  host: &ScriptHostClient,
) -> Result<Value, RuntimeExecutionError> {
  let invocation_id = generated_id("inv");
  engine.append_payload(
    run_id,
    RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
      node_id: node_id.to_string(),
      attempt: 1,
      invocation_id: invocation_id.clone(),
      handler: "runtime.script".to_string(),
    }),
  )?;

  let context = engine.projection(run_id)?.context;
  if let Some(limit) = options.max_context_bytes {
    let actual = serde_json::to_vec(&context)
      .map_err(|error| RuntimeExecutionError::Stalled(error.to_string()))?
      .len();
    if actual > limit {
      let failure = AttemptFailure {
        kind: AttemptFailureKind::ContextTooLarge,
        code: AttemptFailureKind::ContextTooLarge.code().to_string(),
        message: "Invocation context exceeds the configured byte limit.".to_string(),
        details: Some(crate::FailureSizeDetails {
          actual_bytes: Some(actual as u64),
          limit_bytes: Some(limit as u64),
        }),
      };
      return fail_attempt(engine, run_id, node_id, &invocation_id, failure);
    }
  }

  let request = ExecuteMessage::runtime_script(
    &invocation_id,
    run_id,
    node_id,
    options.script_timeout_ms,
    source,
    &context,
  );
  let outcome = match host.execute(&request).await {
    Ok(completed) => completed.outcome,
    Err(error) => {
      let failure = AttemptFailure {
        kind: AttemptFailureKind::HostCrashed,
        code: AttemptFailureKind::HostCrashed.code().to_string(),
        message: error.to_string(),
        details: None,
      };
      return fail_attempt(engine, run_id, node_id, &invocation_id, failure);
    }
  };

  match outcome {
    HostOutcome::Success { value } => {
      engine.append_payload(
        run_id,
        RunEventPayload::StepAttemptSucceeded(StepAttemptSucceededData {
          node_id: node_id.to_string(),
          attempt: 1,
          invocation_id,
          output: value.clone(),
        }),
      )?;
      Ok(value)
    }
    HostOutcome::Failure { error } => fail_attempt(
      engine,
      run_id,
      node_id,
      &invocation_id,
      error.into_attempt_failure(),
    ),
  }
}

fn recorded_execution_order(events: &[RunEvent]) -> Vec<String> {
  events
    .iter()
    .filter_map(|event| match &event.payload {
      RunEventPayload::StepAttemptSucceeded(data) => Some(data.node_id.clone()),
      _ => None,
    })
    .collect()
}

fn resumed_failure<E: RuntimeDagEngine>(
  engine: &E,
  run_id: &str,
  projection: RunProjection,
) -> Result<RuntimeExecutionError, RuntimeExecutionError> {
  let events = engine.events(run_id)?;
  let failure = projection.failure.ok_or_else(|| {
    RuntimeExecutionError::Stalled("failed run has no folded failure".to_string())
  })?;
  Ok(match failure {
    RunFailure::Attempt(failure) => RuntimeExecutionError::RunFailed(Box::new(FailedRunDetails {
      code: failure.code.clone(),
      message: failure.message.clone(),
      failure,
      events,
    })),
    RunFailure::Parallel {
      parallel_id,
      policy,
      primary_node_id,
      failed_node_ids,
      cancelled_node_ids,
      failure,
    } => RuntimeExecutionError::ParallelFailed(Box::new(FailedParallelDetails {
      code: failure.code.clone(),
      message: failure.message.clone(),
      parallel_id,
      policy,
      primary_node_id,
      failed_node_ids,
      cancelled_node_ids,
      failure,
      events,
    })),
    RunFailure::Branch(failure) => {
      let (branch_id, arm_id, path) = events
        .iter()
        .rev()
        .find_map(|event| match &event.payload {
          RunEventPayload::RunFailed(RunFailedData::V2(RunFailedDataV2::Branch {
            branch_id,
            arm_id,
            path,
            ..
          })) => Some((branch_id.clone(), arm_id.clone(), path.clone())),
          _ => None,
        })
        .ok_or_else(|| {
          RuntimeExecutionError::Stalled(
            "failed branch run has no durable branch identity".to_string(),
          )
        })?;
      let site = match &failure {
        BranchFailure::BranchTestNotBoolean { .. } => BranchFailureSite::Test,
        BranchFailure::ReferenceNotAvailable { .. }
          if projection.branch_selections.contains_key(&branch_id) =>
        {
          BranchFailureSite::Result
        }
        BranchFailure::ReferenceNotAvailable { .. } => BranchFailureSite::Test,
        BranchFailure::BranchSelectionInvalid { .. } => BranchFailureSite::Selection,
      };
      let message = match &failure {
        BranchFailure::BranchTestNotBoolean { message, .. }
        | BranchFailure::ReferenceNotAvailable { message, .. }
        | BranchFailure::BranchSelectionInvalid { message, .. } => message.clone(),
      };
      RuntimeExecutionError::BranchFailed(Box::new(FailedBranchDetails {
        code: failure.code().to_string(),
        message,
        branch_id,
        arm_id,
        path,
        site,
        failure,
        events,
      }))
    }
    RunFailure::Approval {
      approval_id,
      request_id,
      failure,
    } => RuntimeExecutionError::ApprovalFailed(Box::new(FailedApprovalDetails {
      code: failure.code.clone(),
      message: failure.message.clone(),
      approval_id,
      request_id,
      failure,
      events,
    })),
  })
}

fn final_result<E: RuntimeDagEngine>(
  engine: &E,
  run_id: &str,
  execution_order: Vec<String>,
) -> Result<WorkflowExecutionResult, RuntimeExecutionError> {
  let projection = engine.projection(run_id)?;
  let terminal_node_id = projection.terminal_node_id.ok_or_else(|| {
    RuntimeExecutionError::Stalled("successful run has no terminal node".to_string())
  })?;
  let result = projection
    .result
    .ok_or_else(|| RuntimeExecutionError::Stalled("successful run has no result".to_string()))?;
  Ok(WorkflowExecutionResult {
    workflow_id: engine.workflow().workflow_id.clone(),
    run_id: run_id.to_string(),
    terminal_node_id,
    result,
    context: projection.context,
    execution_order,
    events: engine.events(run_id)?,
  })
}

fn fail_attempt<T, E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  node_id: &str,
  invocation_id: &str,
  failure: AttemptFailure,
) -> Result<T, RuntimeExecutionError> {
  engine.append_payload(
    run_id,
    RunEventPayload::StepAttemptFailed(StepAttemptFailedData {
      node_id: node_id.to_string(),
      attempt: 1,
      invocation_id: invocation_id.to_string(),
      failure: failure.clone(),
    }),
  )?;
  engine.append_payload(
    run_id,
    RunEventPayload::RunFailed(match engine.event_schema_version() {
      RUN_EVENT_SCHEMA_VERSION_V1 => RunFailedData::V1(RunFailedDataV1 {
        node_id: Some(node_id.to_string()),
        attempt: Some(1),
        invocation_id: Some(invocation_id.to_string()),
        failure: failure.clone(),
      }),
      RUN_EVENT_SCHEMA_VERSION_V2 | crate::RUN_EVENT_SCHEMA_VERSION_V3 => {
        RunFailedData::V2(RunFailedDataV2::Attempt {
          node_id: node_id.to_string(),
          attempt: 1,
          invocation_id: invocation_id.to_string(),
          failure: failure.clone(),
        })
      }
      _ => unreachable!("compiled models select a supported run-event version"),
    }),
  )?;
  Err(RuntimeExecutionError::RunFailed(Box::new(
    FailedRunDetails {
      code: failure.code.clone(),
      message: failure.message.clone(),
      failure,
      events: engine.events(run_id)?,
    },
  )))
}

fn fail_branch<T, E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  error: BranchEvaluationError,
  site: BranchFailureSite,
) -> Result<T, RuntimeExecutionError> {
  let BranchEvaluationError {
    branch_id,
    arm_id,
    path,
    kind,
  } = error;
  let reference = path
    .as_ref()
    .map(|path| format!("context.{}", path.join(".")));
  let failure = match kind {
    BranchEvaluationErrorKind::NotBoolean(actual_type) => BranchFailure::BranchTestNotBoolean {
      code: "WOML_BRANCH_TEST_NOT_BOOLEAN".to_string(),
      message: format!(
        "Branch test for {} must resolve to a JSON boolean.",
        arm_id.as_deref().unwrap_or(&branch_id)
      ),
      actual_type,
    },
    BranchEvaluationErrorKind::ReferenceNotAvailable => BranchFailure::ReferenceNotAvailable {
      code: "WOML_REFERENCE_NOT_AVAILABLE".to_string(),
      message: format!(
        "Reference {} is not available.",
        reference.as_deref().unwrap_or("<unknown>")
      ),
    },
    BranchEvaluationErrorKind::SelectionInvalid => BranchFailure::BranchSelectionInvalid {
      code: "WOML_BRANCH_SELECTION_INVALID".to_string(),
      message: format!("Branch {branch_id:?} has no valid selectable arm."),
    },
  };
  let code = failure.code().to_string();
  let message = match &failure {
    BranchFailure::BranchTestNotBoolean { message, .. }
    | BranchFailure::ReferenceNotAvailable { message, .. }
    | BranchFailure::BranchSelectionInvalid { message, .. } => message.clone(),
  };
  let details_branch_id = branch_id.clone();
  let details_arm_id = arm_id.clone();
  let details_path = path.clone();
  engine.append_payload(
    run_id,
    RunEventPayload::RunFailed(RunFailedData::V2(RunFailedDataV2::Branch {
      branch_id,
      arm_id,
      path,
      failure: failure.clone(),
    })),
  )?;
  Err(RuntimeExecutionError::BranchFailed(Box::new(
    FailedBranchDetails {
      code,
      message,
      branch_id: details_branch_id,
      arm_id: details_arm_id,
      path: details_path,
      site,
      failure,
      events: engine.events(run_id)?,
    },
  )))
}

trait RuntimeDagEngine {
  fn workflow(&self) -> &CompiledWorkflowDefinition;
  fn start_run(
    &mut self,
    run_id: &str,
    trigger: Map<String, Value>,
  ) -> Result<(), RuntimeExecutionError>;
  fn append_payload(
    &mut self,
    run_id: &str,
    payload: RunEventPayload,
  ) -> Result<(), RuntimeExecutionError>;
  fn projection(&self, run_id: &str) -> Result<RunProjection, RuntimeExecutionError>;
  fn events(&self, run_id: &str) -> Result<Vec<RunEvent>, RuntimeExecutionError>;
  fn ready_node_ids(&self, run_id: &str) -> Result<Vec<String>, RuntimeExecutionError>;
  fn append_payloads(
    &mut self,
    run_id: &str,
    payloads: Vec<RunEventPayload>,
  ) -> Result<(), RuntimeExecutionError> {
    for payload in payloads {
      self.append_payload(run_id, payload)?;
    }
    Ok(())
  }
  fn event_schema_version(&self) -> u32 {
    run_event_schema_version_for_model(self.workflow().schema_version)
  }
  fn publish_pure_result(
    &mut self,
    run_id: &str,
    node_id: &str,
    output: Value,
  ) -> Result<(), RuntimeExecutionError> {
    let invocation_id = generated_id("inv");
    self.append_payload(
      run_id,
      RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
        node_id: node_id.to_string(),
        attempt: 1,
        invocation_id: invocation_id.clone(),
        handler: "engine.branch-result".to_string(),
      }),
    )?;
    self.append_payload(
      run_id,
      RunEventPayload::StepAttemptSucceeded(StepAttemptSucceededData {
        node_id: node_id.to_string(),
        attempt: 1,
        invocation_id,
        output,
      }),
    )?;
    Ok(())
  }
}

impl RuntimeDagEngine for InMemoryDagEngine {
  fn workflow(&self) -> &CompiledWorkflowDefinition {
    self.workflow()
  }

  fn start_run(
    &mut self,
    run_id: &str,
    trigger: Map<String, Value>,
  ) -> Result<(), RuntimeExecutionError> {
    self.start_run(generated_id("evt"), run_id, chrono::Utc::now(), trigger)?;
    Ok(())
  }

  fn append_payload(
    &mut self,
    run_id: &str,
    payload: RunEventPayload,
  ) -> Result<(), RuntimeExecutionError> {
    let sequence = self.events(run_id).len() as u64 + 1;
    self.append_event(RunEvent {
      event_schema_version: self.event_schema_version(),
      event_id: generated_id("evt"),
      run_id: run_id.to_string(),
      sequence,
      occurred_at: chrono::Utc::now(),
      payload,
    })?;
    Ok(())
  }

  fn projection(&self, run_id: &str) -> Result<RunProjection, RuntimeExecutionError> {
    Ok(self.projection(run_id)?)
  }

  fn events(&self, run_id: &str) -> Result<Vec<RunEvent>, RuntimeExecutionError> {
    Ok(self.events(run_id).to_vec())
  }

  fn ready_node_ids(&self, run_id: &str) -> Result<Vec<String>, RuntimeExecutionError> {
    Ok(self.ready_node_ids(run_id)?)
  }
}

impl RuntimeDagEngine for DurableDagEngine {
  fn workflow(&self) -> &CompiledWorkflowDefinition {
    self.workflow()
  }

  fn start_run(
    &mut self,
    run_id: &str,
    trigger: Map<String, Value>,
  ) -> Result<(), RuntimeExecutionError> {
    self.start_run(generated_id("evt"), run_id, chrono::Utc::now(), trigger)?;
    Ok(())
  }

  fn append_payload(
    &mut self,
    run_id: &str,
    payload: RunEventPayload,
  ) -> Result<(), RuntimeExecutionError> {
    self.append_payload(generated_id("evt"), run_id, chrono::Utc::now(), payload)?;
    Ok(())
  }

  fn projection(&self, run_id: &str) -> Result<RunProjection, RuntimeExecutionError> {
    Ok(self.projection(run_id)?)
  }

  fn events(&self, run_id: &str) -> Result<Vec<RunEvent>, RuntimeExecutionError> {
    Ok(self.events(run_id)?)
  }

  fn ready_node_ids(&self, run_id: &str) -> Result<Vec<String>, RuntimeExecutionError> {
    Ok(self.ready_node_ids(run_id)?)
  }

  fn append_payloads(
    &mut self,
    run_id: &str,
    payloads: Vec<RunEventPayload>,
  ) -> Result<(), RuntimeExecutionError> {
    self.append_payloads_atomically(run_id, payloads)?;
    Ok(())
  }

  fn publish_pure_result(
    &mut self,
    run_id: &str,
    node_id: &str,
    output: Value,
  ) -> Result<(), RuntimeExecutionError> {
    let invocation_id = generated_id("inv");
    self.publish_pure_result(run_id, node_id, &invocation_id, output)?;
    Ok(())
  }
}

fn generated_id(prefix: &str) -> String {
  format!("{prefix}_{}", Uuid::new_v4().simple())
}
