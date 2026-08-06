use std::path::PathBuf;

use serde::Serialize;
use serde_json::{Map, Value};
use thiserror::Error;
use uuid::Uuid;

use crate::event::{
  RunFailedData, RunFailedDataV1, RunSucceededData, StepAttemptFailedData, StepAttemptStartedData,
  StepAttemptSucceededData,
};
use crate::protocol::{ExecuteMessage, HostOutcome};
use crate::{
  AttemptFailure, AttemptFailureKind, CompiledWorkflowDefinition, DurableDagEngine,
  DurableEngineError, DurableEventStore, DurableStoreError, EngineError, InMemoryDagEngine,
  RecoveryReport, RunEvent, RunEventPayload, RunProjection, RunStatus, ScriptHostClient,
  ScriptHostClientError, ScriptHostProcessOptions, WorkflowContext, RUN_EVENT_SCHEMA_VERSION_V1,
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
  let mut execution_order = Vec::new();

  loop {
    let ready = engine.ready_node_ids(&run_id)?;
    let Some(node_id) = ready.first() else {
      let projection = engine.projection(&run_id)?;
      if projection.status == RunStatus::Succeeded {
        return final_result(engine, &run_id, execution_order);
      }
      return Err(RuntimeExecutionError::Stalled(
        "no node is ready before the run reached a terminal state".to_string(),
      ));
    };
    if ready.len() != 1 {
      return Err(RuntimeExecutionError::Stalled(
        "the R3 runtime received more than one ready node".to_string(),
      ));
    }

    let node = engine
      .workflow()
      .node(node_id)
      .ok_or_else(|| RuntimeExecutionError::Stalled("ready node disappeared".to_string()))?;
    let source = node
      .script_source()
      .ok_or_else(|| {
        RuntimeExecutionError::Stalled(format!("node {node_id:?} has no script source"))
      })?
      .to_string();
    let invocation_id = generated_id("inv");
    engine.append_payload(
      &run_id,
      RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
        node_id: node_id.clone(),
        attempt: 1,
        invocation_id: invocation_id.clone(),
        handler: "runtime.script".to_string(),
      }),
    )?;

    let context = engine.projection(&run_id)?.context;
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
        return fail_attempt(engine, &run_id, node_id, &invocation_id, failure);
      }
    }

    let request = ExecuteMessage::runtime_script(
      &invocation_id,
      &run_id,
      node_id,
      options.script_timeout_ms,
      &source,
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
        return fail_attempt(engine, &run_id, node_id, &invocation_id, failure);
      }
    };

    match outcome {
      HostOutcome::Success { value } => {
        engine.append_payload(
          &run_id,
          RunEventPayload::StepAttemptSucceeded(StepAttemptSucceededData {
            node_id: node_id.clone(),
            attempt: 1,
            invocation_id,
            output: value.clone(),
          }),
        )?;
        execution_order.push(node_id.clone());
        if node_id == &terminal_node_id {
          engine.append_payload(
            &run_id,
            RunEventPayload::RunSucceeded(RunSucceededData {
              terminal_node_id: terminal_node_id.clone(),
              result: value,
            }),
          )?;
          return final_result(engine, &run_id, execution_order);
        }
      }
      HostOutcome::Failure { error } => {
        return fail_attempt(
          engine,
          &run_id,
          node_id,
          &invocation_id,
          error.into_attempt_failure(),
        );
      }
    }
  }
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
    RunEventPayload::RunFailed(RunFailedData::V1(RunFailedDataV1 {
      node_id: Some(node_id.to_string()),
      attempt: Some(1),
      invocation_id: Some(invocation_id.to_string()),
      failure: failure.clone(),
    })),
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
      event_schema_version: RUN_EVENT_SCHEMA_VERSION_V1,
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
}

fn generated_id(prefix: &str) -> String {
  format!("{prefix}_{}", Uuid::new_v4().simple())
}
