use std::collections::{BTreeMap, HashMap};
use std::future::{poll_fn, Future};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use crate::engine::{
  node_is_complete, resolve_context_reference, selected_branch_arm, step_effect_idempotency_key,
  BranchEvaluationError, BranchEvaluationErrorKind,
};
use crate::event::{
  ApprovalFailure, ApprovalRequestedData, ApprovalTimeoutPolicy, BranchSelectedData,
  ParallelFailure, ParallelFailurePolicy, ParallelGroupCompletedData, ParallelGroupOutcome,
  ParallelGroupStartedData, RunFailedData, RunFailedDataV1, RunFailedDataV2, RunFailedDataV3,
  RunSucceededData, StepAttemptFailedData, StepAttemptStartedData, StepAttemptSucceededData,
};
use crate::interval::{IntervalProgress, IntervalProgressReporter};
use crate::model::{ApprovalDefinition, ParallelGroupDefinition, ValueExpression};
use crate::projection::{ApprovalRequestStatus, AttemptStatus};
use crate::protocol::{ExecuteMessage, HostOutcome, RuntimeModuleBinding, ScriptAttempt};
use crate::schedule::{
  ScheduleClock, ScheduleProgress, ScheduleProgressReporter, SystemScheduleClock,
};
use crate::{
  run_event_schema_version_for_model, ApprovalDecisionOutcome, ApprovalTimeoutSettlement,
  AttemptFailure, AttemptFailureKind, BranchFailure, CapabilityRegistry,
  CompiledWorkflowDefinition, DurableCapabilityAuthority, DurableDagEngine, DurableEngineError,
  DurableEventStore, DurableStoreError, EngineError, InMemoryDagEngine, IssuedApprovalToken,
  RecoveryReport, RunEvent, RunEventPayload, RunFailure, RunProjection, RunStatus,
  ScriptHostClient, ScriptHostClientError, ScriptHostModuleArtifact, ScriptHostProcessOptions,
  StepFailureDisposition, WorkflowContext, RUN_EVENT_SCHEMA_VERSION_V1,
  RUN_EVENT_SCHEMA_VERSION_V2,
};

pub trait EngineClock: Send + Sync {
  fn now(&self) -> chrono::DateTime<chrono::Utc>;
}

pub const EXECUTION_PROGRESS_CONTRACT: &str = "woml.execution-progress";
pub const EXECUTION_PROGRESS_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExecutionProgress {
  StepAttemptFailed {
    contract: &'static str,
    version: u32,
    #[serde(rename = "runId")]
    run_id: String,
    #[serde(rename = "nodeId")]
    node_id: String,
    attempt: u32,
    #[serde(rename = "maxAttempts")]
    max_attempts: u32,
    #[serde(rename = "failureCode")]
    failure_code: String,
  },
  StepRetryScheduled {
    contract: &'static str,
    version: u32,
    #[serde(rename = "runId")]
    run_id: String,
    #[serde(rename = "nodeId")]
    node_id: String,
    #[serde(rename = "nextAttempt")]
    next_attempt: u32,
    #[serde(rename = "maxAttempts")]
    max_attempts: u32,
    #[serde(rename = "scheduledAt")]
    scheduled_at: chrono::DateTime<chrono::Utc>,
  },
  StepAttemptSucceeded {
    contract: &'static str,
    version: u32,
    #[serde(rename = "runId")]
    run_id: String,
    #[serde(rename = "nodeId")]
    node_id: String,
    attempt: u32,
    #[serde(rename = "maxAttempts")]
    max_attempts: u32,
  },
}

pub type ExecutionProgressReporter = Arc<dyn Fn(ExecutionProgress) + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeModuleArtifact {
  pub name: String,
  pub bundle_digest: String,
  pub source_map_digest: String,
  pub exports: Vec<String>,
  pub bundle: String,
  pub source_map: String,
}

#[derive(Debug, Default)]
pub struct SystemEngineClock;

impl EngineClock for SystemEngineClock {
  fn now(&self) -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now()
  }
}

#[derive(Debug, Clone)]
pub struct FixedEngineClock {
  now: chrono::DateTime<chrono::Utc>,
}

impl FixedEngineClock {
  pub const fn new(now: chrono::DateTime<chrono::Utc>) -> Self {
    Self { now }
  }
}

impl EngineClock for FixedEngineClock {
  fn now(&self) -> chrono::DateTime<chrono::Utc> {
    self.now
  }
}

#[derive(Clone)]
pub struct RuntimeExecutionOptions {
  pub script_host: ScriptHostProcessOptions,
  pub script_timeout_ms: u64,
  pub max_context_bytes: Option<usize>,
  pub clock: Arc<dyn EngineClock>,
  pub progress_reporter: Option<ExecutionProgressReporter>,
  pub schedule_clock: Arc<dyn ScheduleClock>,
  pub schedule_progress_reporter: Option<ScheduleProgressReporter>,
  pub interval_progress_reporter: Option<IntervalProgressReporter>,
  pub resolved_secrets: Arc<BTreeMap<String, String>>,
  pub capability_registry: Arc<CapabilityRegistry>,
  pub runtime_modules: Arc<Vec<RuntimeModuleArtifact>>,
  capability_authority: Option<Arc<DurableCapabilityAuthority>>,
  managed_database_pool: Option<Arc<crate::ManagedDatabasePool>>,
  managed_storage_store: Option<Arc<crate::ManagedStorageStore>>,
  managed_cache_store: Option<Arc<crate::ManagedCacheStore>>,
}

impl std::fmt::Debug for RuntimeExecutionOptions {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    formatter
      .debug_struct("RuntimeExecutionOptions")
      .field("script_host", &self.script_host)
      .field("script_timeout_ms", &self.script_timeout_ms)
      .field("max_context_bytes", &self.max_context_bytes)
      .field("clock", &"dyn EngineClock")
      .field("schedule_clock", &"dyn ScheduleClock")
      .field("resolved_secret_count", &self.resolved_secrets.len())
      .field("capability_registry", &"CapabilityRegistry")
      .field("runtime_module_count", &self.runtime_modules.len())
      .field(
        "progress_reporter",
        &self.progress_reporter.as_ref().map(|_| "configured"),
      )
      .field(
        "schedule_progress_reporter",
        &self
          .schedule_progress_reporter
          .as_ref()
          .map(|_| "configured"),
      )
      .field(
        "interval_progress_reporter",
        &self
          .interval_progress_reporter
          .as_ref()
          .map(|_| "configured"),
      )
      .finish()
  }
}

impl RuntimeExecutionOptions {
  pub fn new(script_host: ScriptHostProcessOptions, script_timeout_ms: u64) -> Self {
    let capability_registry = Arc::new(CapabilityRegistry::default());
    let managed_storage_store = Arc::new(crate::ManagedStorageStore::default());
    capability_registry
      .register(Arc::new(crate::ManagedHttpHandler::with_storage(
        Arc::clone(&managed_storage_store),
      )))
      .expect("the production HTTP capability is registered exactly once");
    let managed_database_pool = Arc::new(crate::ManagedDatabasePool::default());
    for handler in crate::ManagedDatabaseHandler::handlers(Arc::clone(&managed_database_pool)) {
      capability_registry
        .register(handler)
        .expect("each production database operation is registered exactly once");
    }
    for handler in crate::ManagedStorageHandler::handlers(Arc::clone(&managed_storage_store)) {
      capability_registry
        .register(handler)
        .expect("each production storage operation is registered exactly once");
    }
    let managed_cache_store = Arc::new(crate::ManagedCacheStore::default());
    for handler in crate::ManagedCacheHandler::handlers(Arc::clone(&managed_cache_store)) {
      capability_registry
        .register(handler)
        .expect("each production cache operation is registered exactly once");
    }
    Self {
      script_host,
      script_timeout_ms,
      max_context_bytes: None,
      clock: Arc::new(SystemEngineClock),
      progress_reporter: None,
      schedule_clock: Arc::new(SystemScheduleClock),
      schedule_progress_reporter: None,
      interval_progress_reporter: None,
      resolved_secrets: Arc::new(BTreeMap::new()),
      capability_registry,
      runtime_modules: Arc::new(Vec::new()),
      capability_authority: None,
      managed_database_pool: Some(managed_database_pool),
      managed_storage_store: Some(managed_storage_store),
      managed_cache_store: Some(managed_cache_store),
    }
  }

  pub fn with_clock(mut self, clock: Arc<dyn EngineClock>) -> Self {
    self.clock = clock;
    self
  }

  pub fn with_progress_reporter(mut self, reporter: ExecutionProgressReporter) -> Self {
    self.progress_reporter = Some(reporter);
    self
  }

  pub fn with_schedule_clock(mut self, clock: Arc<dyn ScheduleClock>) -> Self {
    self.schedule_clock = clock;
    self
  }

  pub fn with_resolved_secrets(mut self, secrets: BTreeMap<String, String>) -> Self {
    self.resolved_secrets = Arc::new(secrets);
    self
  }

  pub fn with_runtime_modules(mut self, modules: Vec<RuntimeModuleArtifact>) -> Self {
    self.script_host.module_artifacts = modules
      .iter()
      .map(|module| ScriptHostModuleArtifact {
        bundle_digest: module.bundle_digest.clone(),
        bundle: module.bundle.clone(),
        source_map_digest: module.source_map_digest.clone(),
        source_map: module.source_map.clone(),
      })
      .collect();
    self.runtime_modules = Arc::new(modules);
    self
  }

  pub fn with_capability_registry(mut self, registry: Arc<CapabilityRegistry>) -> Self {
    self.capability_registry = registry;
    self.managed_database_pool = None;
    self.managed_storage_store = None;
    self.managed_cache_store = None;
    self
  }

  pub fn with_schedule_progress_reporter(mut self, reporter: ScheduleProgressReporter) -> Self {
    self.schedule_progress_reporter = Some(reporter);
    self
  }

  pub fn with_interval_progress_reporter(mut self, reporter: IntervalProgressReporter) -> Self {
    self.interval_progress_reporter = Some(reporter);
    self
  }

  pub(crate) fn report_schedule(&self, progress: ScheduleProgress) {
    if let Some(reporter) = &self.schedule_progress_reporter {
      reporter(progress);
    }
  }

  pub(crate) fn report_interval(&self, progress: IntervalProgress) {
    if let Some(reporter) = &self.interval_progress_reporter {
      reporter(progress);
    }
  }

  fn report(&self, progress: ExecutionProgress) {
    if let Some(reporter) = &self.progress_reporter {
      reporter(progress);
    }
  }
}

fn retry_max_attempts<E: RuntimeDagEngine>(engine: &E, node_id: &str) -> u32 {
  engine
    .workflow()
    .node(node_id)
    .and_then(|node| node.retry_policy.as_ref())
    .map_or(1, |policy| policy.max_attempts)
}

fn report_attempt_failed<E: RuntimeDagEngine>(
  engine: &E,
  options: &RuntimeExecutionOptions,
  run_id: &str,
  node_id: &str,
  attempt: u32,
  failure_code: String,
) {
  let max_attempts = retry_max_attempts(engine, node_id);
  if max_attempts > 1 {
    options.report(ExecutionProgress::StepAttemptFailed {
      contract: EXECUTION_PROGRESS_CONTRACT,
      version: EXECUTION_PROGRESS_VERSION,
      run_id: run_id.to_string(),
      node_id: node_id.to_string(),
      attempt,
      max_attempts,
      failure_code,
    });
  }
}

fn report_retry_scheduled<E: RuntimeDagEngine>(
  engine: &E,
  options: &RuntimeExecutionOptions,
  run_id: &str,
  node_id: &str,
  next_attempt: u32,
  scheduled_at: chrono::DateTime<chrono::Utc>,
) {
  options.report(ExecutionProgress::StepRetryScheduled {
    contract: EXECUTION_PROGRESS_CONTRACT,
    version: EXECUTION_PROGRESS_VERSION,
    run_id: run_id.to_string(),
    node_id: node_id.to_string(),
    next_attempt,
    max_attempts: retry_max_attempts(engine, node_id),
    scheduled_at,
  });
}

fn report_attempt_succeeded<E: RuntimeDagEngine>(
  engine: &E,
  options: &RuntimeExecutionOptions,
  run_id: &str,
  node_id: &str,
  attempt: u32,
) {
  let max_attempts = retry_max_attempts(engine, node_id);
  if max_attempts > 1 && attempt > 1 {
    options.report(ExecutionProgress::StepAttemptSucceeded {
      contract: EXECUTION_PROGRESS_CONTRACT,
      version: EXECUTION_PROGRESS_VERSION,
      run_id: run_id.to_string(),
      node_id: node_id.to_string(),
      attempt,
      max_attempts,
    });
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

pub const RUNTIME_OUTCOME_CONTRACT: &str = "woml.runtime-outcome";
pub const RUNTIME_OUTCOME_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum WorkflowRuntimeOutcome {
  Succeeded {
    contract: &'static str,
    version: u32,
    execution: WorkflowExecutionResult,
  },
  Waiting {
    contract: &'static str,
    version: u32,
    #[serde(rename = "workflowId")]
    workflow_id: String,
    #[serde(rename = "runId")]
    run_id: String,
    approval: WaitingWorkflowApproval,
  },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitingWorkflowApproval {
  pub approval_id: String,
  pub request_id: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub name: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub description: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
  pub on_timeout: ApprovalTimeoutPolicy,
  pub token: String,
  pub credential_expires_at: chrono::DateTime<chrono::Utc>,
}

impl WorkflowRuntimeOutcome {
  fn succeeded(execution: WorkflowExecutionResult) -> Self {
    Self::Succeeded {
      contract: RUNTIME_OUTCOME_CONTRACT,
      version: RUNTIME_OUTCOME_VERSION,
      execution,
    }
  }
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
  #[error(transparent)]
  NotificationFailed(Box<FailedNotificationDetails>),
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
  pub node_id: Option<String>,
  pub attempt: Option<u32>,
  pub max_attempts: Option<u32>,
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

#[derive(Debug, Error)]
#[error("workflow notification failed [{code}]: {message}")]
pub struct FailedNotificationDetails {
  pub code: String,
  pub message: String,
  pub approval_id: String,
  pub request_id: String,
  pub failed_delivery_ids: Vec<String>,
  pub failure: crate::event::NotificationRunFailure,
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
  succeeded_execution(execute_with_engine(engine, trigger, options).await?)
}

pub async fn execute_workflow_durable(
  workflow: CompiledWorkflowDefinition,
  definition_hash: String,
  trigger: Map<String, Value>,
  options: RuntimeExecutionOptions,
  database_path: PathBuf,
) -> Result<WorkflowExecutionResult, RuntimeExecutionError> {
  if workflow_has_approval(&workflow) {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "durable approval workflows require execute_workflow_durable_outcome".to_string(),
    ));
  }
  succeeded_execution(
    execute_workflow_durable_internal(workflow, definition_hash, trigger, options, database_path)
      .await?,
  )
}

pub async fn execute_workflow_durable_outcome(
  workflow: CompiledWorkflowDefinition,
  definition_hash: String,
  trigger: Map<String, Value>,
  options: RuntimeExecutionOptions,
  database_path: PathBuf,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  if !workflow_has_approval(&workflow) {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "the approval runtime outcome API requires a model-v4 approval workflow".to_string(),
    ));
  }
  execute_workflow_durable_internal(workflow, definition_hash, trigger, options, database_path)
    .await
}

async fn execute_workflow_durable_internal(
  workflow: CompiledWorkflowDefinition,
  definition_hash: String,
  trigger: Map<String, Value>,
  options: RuntimeExecutionOptions,
  database_path: PathBuf,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  let options = attach_durable_capability_authority(options, &database_path)?;
  let mut store = DurableEventStore::open(database_path)?;
  store.recover_interrupted_runs()?;
  store.register_definition_module_artifacts(
    &workflow,
    &definition_hash,
    options.runtime_modules.as_ref(),
  )?;
  let engine = DurableDagEngine::new(workflow, definition_hash, store)?;
  execute_with_engine(engine, trigger, options).await
}

pub async fn resume_workflow_durable(
  database_path: PathBuf,
  run_id: &str,
  options: RuntimeExecutionOptions,
) -> Result<WorkflowExecutionResult, RuntimeExecutionError> {
  succeeded_execution(
    resume_workflow_durable_internal(database_path, run_id, options, Some(false)).await?,
  )
}

pub async fn resume_workflow_durable_outcome(
  database_path: PathBuf,
  run_id: &str,
  options: RuntimeExecutionOptions,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  resume_workflow_durable_internal(database_path, run_id, options, Some(true)).await
}

pub async fn resume_workflow_durable_any_outcome(
  database_path: PathBuf,
  run_id: &str,
  options: RuntimeExecutionOptions,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  resume_workflow_durable_internal(database_path, run_id, options, None).await
}

/// Continues one run that was already atomically admitted by a production
/// trigger. Unlike the one-shot CLI resume APIs, this does not run global crash
/// recovery before dispatch: a long-lived trigger server may have other runs
/// executing concurrently, and their active attempts must not be mistaken for
/// leftovers from a dead process.
pub async fn execute_admitted_trigger_run_durable(
  database_path: PathBuf,
  run_id: &str,
  options: RuntimeExecutionOptions,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  let options = attach_durable_capability_authority(options, &database_path)?;
  let store = DurableEventStore::open(database_path)?;
  let binding = store.run_binding(run_id)?;
  let options = runtime_modules_from_store(options, &store, &binding.definition_hash)?;
  let engine = DurableDagEngine::resume(store, run_id)?;
  resume_with_engine(engine, run_id, options).await
}

async fn resume_workflow_durable_internal(
  database_path: PathBuf,
  run_id: &str,
  options: RuntimeExecutionOptions,
  approval_outcome_api: Option<bool>,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  let options = attach_durable_capability_authority(options, &database_path)?;
  let mut store = DurableEventStore::open(database_path)?;
  store.recover_interrupted_runs()?;
  let binding = store.run_binding(run_id)?;
  let options = runtime_modules_from_store(options, &store, &binding.definition_hash)?;
  let engine = DurableDagEngine::resume(store, run_id)?;
  let has_approval = workflow_has_approval(engine.workflow());
  if approval_outcome_api == Some(true) && !has_approval {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "the approval runtime outcome API requires a model-v4 approval workflow".to_string(),
    ));
  }
  if approval_outcome_api == Some(false) && has_approval {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "durable approval workflows require resume_workflow_durable_outcome".to_string(),
    ));
  }
  resume_with_engine(engine, run_id, options).await
}

fn attach_durable_capability_authority(
  mut options: RuntimeExecutionOptions,
  database_path: &std::path::Path,
) -> Result<RuntimeExecutionOptions, RuntimeExecutionError> {
  if let Some(storage) = &options.managed_storage_store {
    storage
      .configure_for_state(database_path)
      .map_err(|failure| RuntimeExecutionError::InvalidConfiguration(failure.message))?;
  }
  if let Some(cache) = &options.managed_cache_store {
    cache
      .configure_for_state(database_path)
      .map_err(|failure| RuntimeExecutionError::InvalidConfiguration(failure.message))?;
  }
  if let Some(pool) = &options.managed_database_pool {
    pool
      .protect_path(database_path)
      .map_err(|failure| RuntimeExecutionError::InvalidConfiguration(failure.message))?;
  }
  let store = DurableEventStore::open(database_path.to_path_buf())?;
  if let Some(pool) = &options.managed_database_pool {
    // Protect the post-open canonical identity as well as the pre-open lexical
    // path so a state path created during this call cannot gain an alias.
    pool
      .protect_path(database_path)
      .map_err(|failure| RuntimeExecutionError::InvalidConfiguration(failure.message))?;
  }
  options.capability_authority = Some(Arc::new(DurableCapabilityAuthority::new(
    Arc::clone(&options.capability_registry),
    Arc::new(tokio::sync::Mutex::new(store)),
  )));
  Ok(options)
}

fn runtime_modules_from_store(
  options: RuntimeExecutionOptions,
  store: &DurableEventStore,
  definition_hash: &str,
) -> Result<RuntimeExecutionOptions, RuntimeExecutionError> {
  let stored = store.definition_module_artifacts(definition_hash)?;
  if options.runtime_modules.is_empty() {
    return Ok(options.with_runtime_modules(stored));
  }
  if options.runtime_modules.as_ref() != &stored {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "supplied module artifacts do not match the durable definition artifacts".to_string(),
    ));
  }
  Ok(options)
}

fn workflow_has_approval(workflow: &CompiledWorkflowDefinition) -> bool {
  workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V4
    && workflow
      .graph
      .nodes
      .iter()
      .any(|node| node.handler == "engine.approval-wait")
}

fn resolved_script_secrets(
  workflow: &CompiledWorkflowDefinition,
  node_id: &str,
  options: &RuntimeExecutionOptions,
) -> Result<BTreeMap<String, String>, RuntimeExecutionError> {
  let Some(runtime) = workflow
    .node(node_id)
    .and_then(|node| node.script_runtime.as_ref())
  else {
    return Ok(BTreeMap::new());
  };
  runtime
    .required_secrets
    .iter()
    .map(|name| {
      options
        .resolved_secrets
        .get(name)
        .filter(|value| !value.is_empty())
        .cloned()
        .map(|value| (name.clone(), value))
        .ok_or_else(|| {
          RuntimeExecutionError::InvalidConfiguration(format!(
            "script node {node_id:?} requires unresolved secret {name:?}"
          ))
        })
    })
    .collect()
}

fn succeeded_execution(
  outcome: WorkflowRuntimeOutcome,
) -> Result<WorkflowExecutionResult, RuntimeExecutionError> {
  match outcome {
    WorkflowRuntimeOutcome::Succeeded { execution, .. } => Ok(execution),
    WorkflowRuntimeOutcome::Waiting { .. } => Err(RuntimeExecutionError::Stalled(
      "workflow is durably waiting for approval; use the runtime outcome API".to_string(),
    )),
  }
}

pub fn recover_durable_runs(
  database_path: PathBuf,
) -> Result<RecoveryReport, RuntimeExecutionError> {
  let mut store = DurableEventStore::open(database_path)?;
  Ok(store.recover_interrupted_runs()?)
}

pub fn resolve_human_approval_durable(
  database_path: PathBuf,
  token: &str,
  decision: crate::ApprovalDecision,
  clock: &dyn EngineClock,
) -> Result<ApprovalDecisionOutcome, RuntimeExecutionError> {
  let mut store = DurableEventStore::open(database_path)?;
  Ok(store.resolve_human_approval(token, decision, clock.now())?)
}

pub fn settle_approval_timeout_durable(
  database_path: PathBuf,
  run_id: &str,
  approval_id: &str,
  clock: &dyn EngineClock,
) -> Result<ApprovalTimeoutSettlement, RuntimeExecutionError> {
  let mut store = DurableEventStore::open(database_path)?;
  Ok(store.settle_approval_timeout(run_id, approval_id, clock.now())?)
}

async fn execute_with_engine<E: RuntimeDagEngine>(
  mut engine: E,
  trigger: Map<String, Value>,
  options: RuntimeExecutionOptions,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  if options.script_timeout_ms == 0 {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "script_timeout_ms must be greater than zero".to_string(),
    ));
  }
  validate_runtime_modules(engine.workflow(), &options)?;

  execute_runtime(&mut engine, trigger, &options).await
}

fn sha256_identity(content: &str) -> String {
  format!("sha256:{:x}", Sha256::digest(content.as_bytes()))
}

fn validate_runtime_modules(
  workflow: &CompiledWorkflowDefinition,
  options: &RuntimeExecutionOptions,
) -> Result<(), RuntimeExecutionError> {
  let Some(runtime) = &workflow.module_runtime else {
    if options.runtime_modules.is_empty() {
      return Ok(());
    }
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "module artifacts were supplied for a workflow without moduleRuntime".to_string(),
    ));
  };
  if runtime.modules.len() != options.runtime_modules.len() {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "the runtime module artifacts do not match Model v9".to_string(),
    ));
  }
  let mut total_bytes = 0usize;
  for (binding, artifact) in runtime.modules.iter().zip(options.runtime_modules.iter()) {
    total_bytes = total_bytes
      .checked_add(artifact.bundle.len())
      .and_then(|value| value.checked_add(artifact.source_map.len()))
      .ok_or_else(|| {
        RuntimeExecutionError::InvalidConfiguration(
          "runtime module artifact byte count overflowed".to_string(),
        )
      })?;
    let matches = binding.name == artifact.name
      && binding.bundle_digest == artifact.bundle_digest
      && binding.source_map_digest == artifact.source_map_digest
      && binding.exports == artifact.exports
      && sha256_identity(&artifact.bundle) == artifact.bundle_digest
      && sha256_identity(&artifact.source_map) == artifact.source_map_digest
      && artifact.bundle.len() <= crate::durable::MAX_MODULE_ARTIFACT_BYTES
      && artifact.source_map.len() <= crate::durable::MAX_MODULE_ARTIFACT_BYTES;
    if !matches {
      return Err(RuntimeExecutionError::InvalidConfiguration(format!(
        "runtime module artifact {:?} failed its Model v9 identity or size check",
        binding.name
      )));
    }
    if options.resolved_secrets.values().any(|secret| {
      !secret.is_empty()
        && (artifact.bundle.contains(secret) || artifact.source_map.contains(secret))
    }) {
      return Err(RuntimeExecutionError::InvalidConfiguration(format!(
        "runtime module artifact {:?} contains a resolved secret value",
        binding.name
      )));
    }
  }
  if total_bytes > crate::durable::MAX_MODULE_ARTIFACT_SET_BYTES {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "runtime module artifacts exceed the per-definition cache limit".to_string(),
    ));
  }
  Ok(())
}

async fn resume_with_engine<E: RuntimeDagEngine>(
  mut engine: E,
  run_id: &str,
  options: RuntimeExecutionOptions,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  if options.script_timeout_ms == 0 {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "script_timeout_ms must be greater than zero".to_string(),
    ));
  }
  validate_runtime_modules(engine.workflow(), &options)?;
  let projection = engine.projection(run_id)?;
  let execution_order = recorded_execution_order(&engine.events(run_id)?);
  match projection.status {
    RunStatus::Succeeded => {
      return Ok(WorkflowRuntimeOutcome::succeeded(final_result(
        &engine,
        run_id,
        execution_order,
      )?));
    }
    RunStatus::Failed => return Err(resumed_failure(&engine, run_id, projection)?),
    RunStatus::Running => {}
    RunStatus::Waiting => {
      return engine.reissue_waiting_outcome(run_id, options.clock.now());
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
  continue_runtime(
    &mut engine,
    run_id,
    terminal_node_id,
    execution_order,
    &options,
  )
  .await
}

async fn execute_runtime<E: RuntimeDagEngine>(
  engine: &mut E,
  trigger: Map<String, Value>,
  options: &RuntimeExecutionOptions,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  let terminal_node_id = engine
    .workflow()
    .terminal_node_id()
    .ok_or_else(|| RuntimeExecutionError::Stalled("no terminal node exists".to_string()))?
    .to_string();
  let run_id = generated_id("run");
  engine.start_run(&run_id, trigger)?;
  continue_runtime(engine, &run_id, terminal_node_id, Vec::new(), options).await
}

async fn continue_runtime<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  terminal_node_id: String,
  mut execution_order: Vec<String>,
  options: &RuntimeExecutionOptions,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  let mut host = None;
  let execution = continue_runtime_loop(
    engine,
    run_id,
    terminal_node_id,
    &mut execution_order,
    options,
    &mut host,
  )
  .await;
  if let Some(host) = host {
    host.shutdown().await;
  }
  execution
}

async fn continue_runtime_loop<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  terminal_node_id: String,
  execution_order: &mut Vec<String>,
  options: &RuntimeExecutionOptions,
  host: &mut Option<ScriptHostClient>,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  loop {
    let ready = engine.ready_node_ids(run_id)?;
    let Some(node_id) = ready.first() else {
      let projection = engine.projection(run_id)?;
      if projection.status == RunStatus::Succeeded {
        return Ok(WorkflowRuntimeOutcome::succeeded(final_result(
          engine,
          run_id,
          execution_order.clone(),
        )?));
      }
      if let Some(result) =
        completed_terminal_approval_result(engine.workflow(), &terminal_node_id, &projection)
      {
        engine.append_payload(
          run_id,
          RunEventPayload::RunSucceeded(RunSucceededData {
            terminal_node_id: terminal_node_id.clone(),
            result,
          }),
        )?;
        return Ok(WorkflowRuntimeOutcome::succeeded(final_result(
          engine,
          run_id,
          execution_order.clone(),
        )?));
      }
      if projection.status == RunStatus::Running && !projection.pending_retries.is_empty() {
        let scheduled_at = projection
          .pending_retries
          .values()
          .map(|retry| retry.scheduled_at)
          .min()
          .ok_or_else(|| {
            RuntimeExecutionError::Stalled("pending retry state has no next schedule".to_string())
          })?;
        let now = chrono::Utc::now();
        if scheduled_at > now {
          let wait = (scheduled_at - now).to_std().map_err(|_| {
            RuntimeExecutionError::Stalled("retry schedule exceeds the runtime clock".to_string())
          })?;
          tokio::time::sleep(wait).await;
          continue;
        }
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
      if host.is_none() {
        *host = Some(
          ScriptHostClient::spawn_with_authority(
            options.script_host.clone(),
            options.capability_authority.clone(),
          )
          .await?,
        );
      }
      let completed = execute_parallel_group(
        engine,
        run_id,
        group,
        options,
        host.as_ref().expect("script host was initialized"),
      )
      .await?;
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
          return Ok(WorkflowRuntimeOutcome::succeeded(final_result(
            engine,
            run_id,
            execution_order.clone(),
          )?));
        }
      }
      "engine.approval-wait" => {
        let approval = engine.workflow().approval(node_id).ok_or_else(|| {
          RuntimeExecutionError::Stalled(format!(
            "approval node {node_id:?} has invalid compiled inputs"
          ))
        })?;
        let occurred_at = options.clock.now();
        let expires_at = approval
          .timeout_ms
          .map(|milliseconds| {
            i64::try_from(milliseconds)
              .map(chrono::Duration::milliseconds)
              .map(|duration| occurred_at + duration)
          })
          .transpose()
          .map_err(|_| {
            RuntimeExecutionError::Stalled(format!(
              "approval {:?} timeout exceeds the clock range",
              approval.approval_id
            ))
          })?;
        let on_timeout = match approval.on_timeout.as_str() {
          "reject" => ApprovalTimeoutPolicy::Reject,
          "fail" => ApprovalTimeoutPolicy::Fail,
          _ => {
            return Err(RuntimeExecutionError::Stalled(format!(
              "approval {:?} has an unknown timeout policy",
              approval.approval_id
            )));
          }
        };
        let request_id = generated_id("aprreq");
        let token = engine.request_approval(
          run_id,
          occurred_at,
          ApprovalRequestedData {
            approval_id: approval.approval_id.clone(),
            request_id: request_id.clone(),
            expires_at,
            on_timeout,
          },
        )?;
        return Ok(waiting_outcome(
          engine.workflow().workflow_id.clone(),
          run_id.to_string(),
          approval,
          request_id,
          expires_at,
          on_timeout,
          token,
        ));
      }
      "runtime.script" => {
        let source = source.ok_or_else(|| {
          RuntimeExecutionError::Stalled(format!("node {node_id:?} has no script source"))
        })?;
        let outcome = execute_script_node(engine, run_id, node_id, &source, options, host).await?;
        let ScriptNodeOutcome::Succeeded(output) = outcome else {
          continue;
        };
        execution_order.push(node_id.clone());
        if node_id == &terminal_node_id {
          engine.append_payload(
            run_id,
            RunEventPayload::RunSucceeded(RunSucceededData {
              terminal_node_id: terminal_node_id.clone(),
              result: output,
            }),
          )?;
          return Ok(WorkflowRuntimeOutcome::succeeded(final_result(
            engine,
            run_id,
            execution_order.clone(),
          )?));
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

fn completed_terminal_approval_result(
  workflow: &CompiledWorkflowDefinition,
  terminal_node_id: &str,
  projection: &RunProjection,
) -> Option<Value> {
  let terminal = workflow.node(terminal_node_id)?;
  if terminal.handler != "engine.approval-join" || !node_is_complete(workflow, terminal, projection)
  {
    return None;
  }
  let approval_id = terminal_node_id
    .strip_prefix("__woml_approval__")?
    .strip_suffix("__join")?;
  projection.context.steps.get(approval_id).cloned()
}

fn waiting_outcome(
  workflow_id: String,
  run_id: String,
  approval: ApprovalDefinition,
  request_id: String,
  expires_at: Option<chrono::DateTime<chrono::Utc>>,
  on_timeout: ApprovalTimeoutPolicy,
  token: IssuedApprovalToken,
) -> WorkflowRuntimeOutcome {
  WorkflowRuntimeOutcome::Waiting {
    contract: RUNTIME_OUTCOME_CONTRACT,
    version: RUNTIME_OUTCOME_VERSION,
    workflow_id,
    run_id,
    approval: WaitingWorkflowApproval {
      approval_id: approval.approval_id,
      request_id,
      name: approval.name,
      description: approval.description,
      expires_at,
      on_timeout,
      token: token.token,
      credential_expires_at: token.credential_expires_at,
    },
  }
}

#[derive(Debug)]
struct ParallelInvocationCompletion {
  node_id: String,
  invocation_id: String,
  attempt_number: u32,
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
  attempt_number: u32,
  max_attempts: u32,
  idempotency_key: String,
  secrets: BTreeMap<String, String>,
  module_bindings: Vec<RuntimeModuleBinding>,
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
  let outcome = if let Some(limit) = request.max_context_bytes {
    match serde_json::to_vec(&request.context) {
      Ok(encoded) if encoded.len() > limit => Err(AttemptFailure {
        kind: AttemptFailureKind::ContextTooLarge,
        code: AttemptFailureKind::ContextTooLarge.code().to_string(),
        message: "Invocation context exceeds the configured byte limit.".to_string(),
        details: Some(crate::FailureSizeDetails {
          actual_bytes: Some(encoded.len() as u64),
          limit_bytes: Some(limit as u64),
        }),
        ..AttemptFailure::legacy_defaults()
      }),
      Err(error) => Err(AttemptFailure {
        kind: AttemptFailureKind::InvalidScriptResult,
        code: AttemptFailureKind::InvalidScriptResult.code().to_string(),
        message: format!("Invocation context could not be encoded: {error}"),
        details: None,
        ..AttemptFailure::legacy_defaults()
      }),
      _ => execute_parallel_request(host, &request).await,
    }
  } else {
    execute_parallel_request(host, &request).await
  };
  ParallelInvocationCompletion {
    node_id: request.node_id,
    invocation_id: request.invocation_id,
    attempt_number: request.attempt_number,
    outcome,
  }
}

async fn execute_parallel_request(
  host: &ScriptHostClient,
  invocation: &ParallelInvocationRequest,
) -> Result<Value, AttemptFailure> {
  let attempt = ScriptAttempt::new(
    invocation.attempt_number,
    invocation.max_attempts,
    &invocation.idempotency_key,
  )
  .map_err(|message| AttemptFailure {
    kind: AttemptFailureKind::InvalidScriptResult,
    code: AttemptFailureKind::InvalidScriptResult.code().to_string(),
    message,
    details: None,
    ..AttemptFailure::legacy_defaults()
  })?;
  let request = ExecuteMessage::runtime_script_with_modules(
    &invocation.invocation_id,
    &invocation.run_id,
    &invocation.node_id,
    attempt,
    invocation.timeout_ms,
    &invocation.source,
    &invocation.context,
    &invocation.secrets,
    &invocation.module_bindings,
  );
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
      ..AttemptFailure::legacy_defaults()
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
  let mut completion_order = Vec::new();
  let mut active: Vec<ActiveParallelInvocation<'_>> = Vec::new();
  let mut fail_fast_closed = false;

  loop {
    while !fail_fast_closed && active.len() < group.concurrency {
      let projection = engine.projection(run_id)?;
      let now = chrono::Utc::now();
      let Some((node_id, attempt_number)) = group.child_node_ids.iter().find_map(|node_id| {
        if projection.context.steps.contains_key(node_id) {
          return None;
        }
        match projection.latest_attempt(node_id) {
          None => Some((node_id.clone(), 1)),
          Some(attempt) if matches!(attempt.status, AttemptStatus::Failed { .. }) => projection
            .pending_retries
            .get(node_id)
            .filter(|schedule| schedule.scheduled_at <= now)
            .map(|schedule| (node_id.clone(), schedule.next_attempt)),
          Some(_) => None,
        }
      }) else {
        break;
      };
      let source = engine
        .workflow()
        .node(&node_id)
        .and_then(|node| node.script_source())
        .ok_or_else(|| {
          RuntimeExecutionError::Stalled(format!("parallel child {node_id:?} has no script source"))
        })?
        .to_string();
      let secrets = resolved_script_secrets(engine.workflow(), &node_id, options)?;
      let invocation_id = generated_id("inv");
      let max_attempts = engine
        .workflow()
        .node(&node_id)
        .and_then(|node| node.retry_policy.as_ref())
        .map_or(1, |policy| policy.max_attempts);
      let idempotency_key = step_effect_idempotency_key(run_id, engine.definition_hash(), &node_id);
      engine.append_payload(
        run_id,
        RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
          node_id: node_id.clone(),
          attempt: attempt_number,
          invocation_id: invocation_id.clone(),
          handler: "runtime.script".to_string(),
          idempotency_key: (engine.event_schema_version() >= crate::RUN_EVENT_SCHEMA_VERSION_V6)
            .then(|| idempotency_key.clone()),
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
            attempt_number,
            max_attempts,
            idempotency_key,
            secrets,
            module_bindings: options
              .runtime_modules
              .iter()
              .map(|module| RuntimeModuleBinding {
                name: module.name.clone(),
                bundle_digest: module.bundle_digest.clone(),
                exports: module.exports.clone(),
              })
              .collect(),
          },
        )),
      });
    }

    let Some(completion) = next_parallel_completion(&mut active).await else {
      break;
    };
    match completion.outcome {
      Ok(output) => {
        completion_order.push(completion.node_id.clone());
        let completed_node_id = completion.node_id.clone();
        let completed_attempt = completion.attempt_number;
        engine.append_payload(
          run_id,
          RunEventPayload::StepAttemptSucceeded(StepAttemptSucceededData {
            node_id: completion.node_id,
            attempt: completion.attempt_number,
            invocation_id: completion.invocation_id,
            output,
          }),
        )?;
        report_attempt_succeeded(
          engine,
          options,
          run_id,
          &completed_node_id,
          completed_attempt,
        );
      }
      Err(failure) => {
        let cancellation = failure.kind == AttemptFailureKind::InvocationCancelled;
        let failure_code = failure.code.clone();
        let disposition = engine.record_step_attempt_failure(
          run_id,
          options.clock.now(),
          StepAttemptFailedData {
            node_id: completion.node_id.clone(),
            attempt: completion.attempt_number,
            invocation_id: completion.invocation_id,
            failure,
          },
        )?;
        report_attempt_failed(
          engine,
          options,
          run_id,
          &completion.node_id,
          completion.attempt_number,
          failure_code,
        );
        match disposition {
          StepFailureDisposition::RetryScheduled {
            next_attempt,
            scheduled_at,
          } => report_retry_scheduled(
            engine,
            options,
            run_id,
            &completion.node_id,
            next_attempt,
            scheduled_at,
          ),
          StepFailureDisposition::StepFailed => {
            if !cancellation && policy == ParallelFailurePolicy::FailFast {
              fail_fast_closed = true;
              let active_invocation_ids = active
                .iter()
                .map(|invocation| invocation.invocation_id.clone())
                .collect::<Vec<_>>();
              for invocation_id in active_invocation_ids {
                let _ = host.cancel(&invocation_id).await;
              }
            }
          }
          StepFailureDisposition::RunFailed => {
            return Err(RuntimeExecutionError::Stalled(
              "a parallel child failure terminated the run before its group settled".to_string(),
            ));
          }
        }
      }
    }
  }

  let projection = engine.projection(run_id)?;
  let mut failed_node_ids = Vec::new();
  let mut cancelled_node_ids = Vec::new();
  let mut final_attempts = HashMap::new();
  let mut every_child_settled = true;
  let mut every_child_succeeded = true;
  for node_id in &group.child_node_ids {
    if projection.context.steps.contains_key(node_id) {
      continue;
    }
    every_child_succeeded = false;
    let Some(attempt) = projection.latest_attempt(node_id) else {
      every_child_settled = false;
      continue;
    };
    let AttemptStatus::Failed { failure } = &attempt.status else {
      every_child_settled = false;
      continue;
    };
    if projection.pending_retries.contains_key(node_id) {
      every_child_settled = false;
      continue;
    }
    final_attempts.insert(node_id.clone(), attempt.identity.attempt);
    if failure.kind == AttemptFailureKind::InvocationCancelled {
      cancelled_node_ids.push(node_id.clone());
    } else {
      failed_node_ids.push(node_id.clone());
    }
  }

  if every_child_succeeded {
    engine.append_payload(
      run_id,
      RunEventPayload::ParallelGroupCompleted(ParallelGroupCompletedData {
        parallel_id: group.parallel_id,
        outcome: ParallelGroupOutcome::Succeeded,
        failed_node_ids: Vec::new(),
        cancelled_node_ids: Vec::new(),
      }),
    )?;
    return Ok(completion_order);
  }

  let group_has_final_failure = !failed_node_ids.is_empty();
  let group_must_fail = match policy {
    ParallelFailurePolicy::FailFast => group_has_final_failure,
    ParallelFailurePolicy::WaitAll => group_has_final_failure && every_child_settled,
  };
  if group_must_fail {
    let primary_node_id = engine
      .events(run_id)?
      .iter()
      .find_map(|event| match &event.payload {
        RunEventPayload::StepAttemptFailed(data)
          if failed_node_ids.contains(&data.node_id)
            && final_attempts.get(&data.node_id) == Some(&data.attempt) =>
        {
          Some(data.node_id.clone())
        }
        _ => None,
      })
      .ok_or_else(|| {
        RuntimeExecutionError::Stalled(format!(
          "parallel group {:?} has no durable primary final failure",
          group.parallel_id
        ))
      })?;
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

  if projection.pending_retries.is_empty() {
    return Err(RuntimeExecutionError::Stalled(format!(
      "parallel group {:?} has unfinished children without a retry schedule",
      group.parallel_id
    )));
  }
  Ok(completion_order)
}

async fn execute_script_node<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  node_id: &str,
  source: &str,
  options: &RuntimeExecutionOptions,
  host: &mut Option<ScriptHostClient>,
) -> Result<ScriptNodeOutcome, RuntimeExecutionError> {
  let secrets = resolved_script_secrets(engine.workflow(), node_id, options)?;
  let invocation_id = generated_id("inv");
  let projection = engine.projection(run_id)?;
  let attempt_number = match projection.latest_attempt(node_id) {
    None => 1,
    Some(_) => projection
      .pending_retries
      .get(node_id)
      .map(|retry| retry.next_attempt)
      .ok_or_else(|| {
        RuntimeExecutionError::Stalled(format!(
          "node {node_id:?} has prior attempts but no pending retry"
        ))
      })?,
  };
  let max_attempts = engine
    .workflow()
    .node(node_id)
    .and_then(|node| node.retry_policy.as_ref())
    .map_or(1, |policy| policy.max_attempts);
  let idempotency_key = step_effect_idempotency_key(run_id, engine.definition_hash(), node_id);
  engine.append_payload(
    run_id,
    RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
      node_id: node_id.to_string(),
      attempt: attempt_number,
      invocation_id: invocation_id.clone(),
      handler: "runtime.script".to_string(),
      idempotency_key: (engine.event_schema_version() >= crate::RUN_EVENT_SCHEMA_VERSION_V6)
        .then(|| idempotency_key.clone()),
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
        ..AttemptFailure::legacy_defaults()
      };
      return settle_script_attempt_failure(
        engine,
        options,
        run_id,
        node_id,
        attempt_number,
        &invocation_id,
        failure,
      );
    }
  }

  if host.is_none() {
    match ScriptHostClient::spawn_with_authority(
      options.script_host.clone(),
      options.capability_authority.clone(),
    )
    .await
    {
      Ok(client) => *host = Some(client),
      Err(error) => {
        return settle_script_attempt_failure(
          engine,
          options,
          run_id,
          node_id,
          attempt_number,
          &invocation_id,
          AttemptFailure {
            kind: AttemptFailureKind::HostCrashed,
            code: AttemptFailureKind::HostCrashed.code().to_string(),
            message: error.to_string(),
            details: None,
            ..AttemptFailure::legacy_defaults()
          },
        );
      }
    }
  }

  let module_bindings = options
    .runtime_modules
    .iter()
    .map(|module| RuntimeModuleBinding {
      name: module.name.clone(),
      bundle_digest: module.bundle_digest.clone(),
      exports: module.exports.clone(),
    })
    .collect::<Vec<_>>();
  let request = ExecuteMessage::runtime_script_with_modules(
    &invocation_id,
    run_id,
    node_id,
    ScriptAttempt::new(attempt_number, max_attempts, &idempotency_key)
      .map_err(RuntimeExecutionError::Stalled)?,
    options.script_timeout_ms,
    source,
    &context,
    &secrets,
    &module_bindings,
  );
  let outcome = match host
    .as_ref()
    .expect("script host was initialized")
    .execute(&request)
    .await
  {
    Ok(completed) => completed.outcome,
    Err(error) => {
      let failure = AttemptFailure {
        kind: AttemptFailureKind::HostCrashed,
        code: AttemptFailureKind::HostCrashed.code().to_string(),
        message: error.to_string(),
        details: None,
        ..AttemptFailure::legacy_defaults()
      };
      return settle_script_attempt_failure(
        engine,
        options,
        run_id,
        node_id,
        attempt_number,
        &invocation_id,
        failure,
      );
    }
  };

  match outcome {
    HostOutcome::Success { value } => {
      engine.append_payload(
        run_id,
        RunEventPayload::StepAttemptSucceeded(StepAttemptSucceededData {
          node_id: node_id.to_string(),
          attempt: attempt_number,
          invocation_id,
          output: value.clone(),
        }),
      )?;
      report_attempt_succeeded(engine, options, run_id, node_id, attempt_number);
      Ok(ScriptNodeOutcome::Succeeded(value))
    }
    HostOutcome::Failure { error } => settle_script_attempt_failure(
      engine,
      options,
      run_id,
      node_id,
      attempt_number,
      &invocation_id,
      error.into_attempt_failure(),
    ),
  }
}

enum ScriptNodeOutcome {
  Succeeded(Value),
  RetryScheduled,
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
    RunFailure::Attempt(failure) => {
      let identity = events.iter().rev().find_map(|event| match &event.payload {
        RunEventPayload::RunFailed(RunFailedData::V1(data)) => data
          .node_id
          .as_ref()
          .zip(data.attempt)
          .map(|(node_id, attempt)| (node_id.clone(), attempt)),
        RunEventPayload::RunFailed(RunFailedData::V2(RunFailedDataV2::Attempt {
          node_id,
          attempt,
          ..
        })) => Some((node_id.clone(), *attempt)),
        _ => None,
      });
      let (node_id, attempt, max_attempts) = identity
        .map(|(node_id, attempt)| {
          let max_attempts = retry_max_attempts(engine, &node_id);
          (Some(node_id), Some(attempt), Some(max_attempts))
        })
        .unwrap_or((None, None, None));
      let exhausted = attempt
        .zip(max_attempts)
        .is_some_and(|(attempt, maximum)| maximum > 1 && attempt >= maximum)
        && failure.kind == AttemptFailureKind::ScriptThrew;
      let code = if exhausted {
        "WOML_STEP_RETRIES_EXHAUSTED".to_string()
      } else {
        failure.code.clone()
      };
      let message = if exhausted {
        format!(
          "attempt {} of {} failed [{}].",
          attempt.unwrap(),
          max_attempts.unwrap(),
          failure.code
        )
      } else {
        failure.message.clone()
      };
      RuntimeExecutionError::RunFailed(Box::new(FailedRunDetails {
        code,
        message,
        node_id,
        attempt,
        max_attempts,
        failure,
        events,
      }))
    }
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
    RunFailure::Notification {
      approval_id,
      request_id,
      failed_delivery_ids,
      failure,
    } => RuntimeExecutionError::NotificationFailed(Box::new(FailedNotificationDetails {
      code: failure.code.clone(),
      message: failure.message.clone(),
      approval_id,
      request_id,
      failed_delivery_ids,
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

fn settle_script_attempt_failure<E: RuntimeDagEngine>(
  engine: &mut E,
  options: &RuntimeExecutionOptions,
  run_id: &str,
  node_id: &str,
  attempt: u32,
  invocation_id: &str,
  failure: AttemptFailure,
) -> Result<ScriptNodeOutcome, RuntimeExecutionError> {
  let disposition = engine.record_step_attempt_failure(
    run_id,
    options.clock.now(),
    StepAttemptFailedData {
      node_id: node_id.to_string(),
      attempt,
      invocation_id: invocation_id.to_string(),
      failure: failure.clone(),
    },
  )?;
  report_attempt_failed(
    engine,
    options,
    run_id,
    node_id,
    attempt,
    failure.code.clone(),
  );
  match disposition {
    StepFailureDisposition::RetryScheduled {
      next_attempt,
      scheduled_at,
    } => {
      report_retry_scheduled(engine, options, run_id, node_id, next_attempt, scheduled_at);
      return Ok(ScriptNodeOutcome::RetryScheduled);
    }
    StepFailureDisposition::StepFailed => {
      return Err(RuntimeExecutionError::Stalled(format!(
        "non-parallel node {node_id:?} returned a group-owned failure"
      )));
    }
    StepFailureDisposition::RunFailed => {}
  }
  Err(RuntimeExecutionError::RunFailed(Box::new(
    FailedRunDetails {
      code: if retry_max_attempts(engine, node_id) > 1
        && attempt >= retry_max_attempts(engine, node_id)
        && failure.kind == AttemptFailureKind::ScriptThrew
      {
        "WOML_STEP_RETRIES_EXHAUSTED".to_string()
      } else {
        failure.code.clone()
      },
      message: if retry_max_attempts(engine, node_id) > 1
        && attempt >= retry_max_attempts(engine, node_id)
        && failure.kind == AttemptFailureKind::ScriptThrew
      {
        format!(
          "attempt {} of {} failed [{}].",
          attempt,
          retry_max_attempts(engine, node_id),
          failure.code
        )
      } else {
        failure.message.clone()
      },
      node_id: Some(node_id.to_string()),
      attempt: Some(attempt),
      max_attempts: Some(retry_max_attempts(engine, node_id)),
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

fn attempt_run_failed_data(
  event_schema_version: u32,
  failure: &StepAttemptFailedData,
) -> RunFailedData {
  match event_schema_version {
    RUN_EVENT_SCHEMA_VERSION_V1 => RunFailedData::V1(RunFailedDataV1 {
      node_id: Some(failure.node_id.clone()),
      attempt: Some(failure.attempt),
      invocation_id: Some(failure.invocation_id.clone()),
      failure: failure.failure.clone(),
    }),
    RUN_EVENT_SCHEMA_VERSION_V2
    | crate::RUN_EVENT_SCHEMA_VERSION_V3
    | crate::RUN_EVENT_SCHEMA_VERSION_V4
    | crate::RUN_EVENT_SCHEMA_VERSION_V5
    | crate::RUN_EVENT_SCHEMA_VERSION_V6
    | crate::RUN_EVENT_SCHEMA_VERSION_V7
    | crate::RUN_EVENT_SCHEMA_VERSION_V8
    | crate::RUN_EVENT_SCHEMA_VERSION_V9 => RunFailedData::V2(RunFailedDataV2::Attempt {
      node_id: failure.node_id.clone(),
      attempt: failure.attempt,
      invocation_id: failure.invocation_id.clone(),
      failure: failure.failure.clone(),
    }),
    _ => unreachable!("compiled models select a supported run-event version"),
  }
}

trait RuntimeDagEngine {
  fn workflow(&self) -> &CompiledWorkflowDefinition;
  fn definition_hash(&self) -> &str;
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
  fn record_step_attempt_failure(
    &mut self,
    run_id: &str,
    _failed_at: chrono::DateTime<chrono::Utc>,
    failure: StepAttemptFailedData,
  ) -> Result<StepFailureDisposition, RuntimeExecutionError> {
    if self
      .workflow()
      .parallel_group_for_child(&failure.node_id)
      .is_some()
    {
      self.append_payload(run_id, RunEventPayload::StepAttemptFailed(failure))?;
      return Ok(StepFailureDisposition::StepFailed);
    }
    if self.event_schema_version() >= crate::RUN_EVENT_SCHEMA_VERSION_V6 {
      return Err(RuntimeExecutionError::Stalled(
        "Model v6 retry failures require the durable runtime".to_string(),
      ));
    }
    let run_failed = attempt_run_failed_data(self.event_schema_version(), &failure);
    self.append_payloads(
      run_id,
      vec![
        RunEventPayload::StepAttemptFailed(failure),
        RunEventPayload::RunFailed(run_failed),
      ],
    )?;
    Ok(StepFailureDisposition::RunFailed)
  }
  fn projection(&self, run_id: &str) -> Result<RunProjection, RuntimeExecutionError>;
  fn events(&self, run_id: &str) -> Result<Vec<RunEvent>, RuntimeExecutionError>;
  fn ready_node_ids(&self, run_id: &str) -> Result<Vec<String>, RuntimeExecutionError>;
  fn request_approval(
    &mut self,
    _run_id: &str,
    _occurred_at: chrono::DateTime<chrono::Utc>,
    _request: ApprovalRequestedData,
  ) -> Result<IssuedApprovalToken, RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "human approval requires the durable runtime".to_string(),
    ))
  }
  fn reissue_waiting_outcome(
    &mut self,
    _run_id: &str,
    _now: chrono::DateTime<chrono::Utc>,
  ) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "human approval requires the durable runtime".to_string(),
    ))
  }
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
        idempotency_key: None,
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

  fn definition_hash(&self) -> &str {
    self.definition_hash()
  }

  fn start_run(
    &mut self,
    run_id: &str,
    trigger: Map<String, Value>,
  ) -> Result<(), RuntimeExecutionError> {
    if self.event_schema_version() >= crate::RUN_EVENT_SCHEMA_VERSION_V7 {
      let trigger_definition = self
        .workflow()
        .triggers
        .iter()
        .find(|candidate| candidate.handler == "trigger.manual")
        .ok_or_else(|| {
          RuntimeExecutionError::InvalidConfiguration(
            "a direct Model v7+ run requires a manual trigger".to_string(),
          )
        })?;
      self.append_event(RunEvent {
        event_schema_version: self.event_schema_version(),
        event_id: generated_id("evt"),
        run_id: run_id.to_string(),
        sequence: 1,
        occurred_at: chrono::Utc::now(),
        payload: RunEventPayload::RunStarted(crate::RunStartedData {
          workflow_id: self.workflow().workflow_id.clone(),
          definition_hash: self.definition_hash().to_string(),
          trigger_id: Some(trigger_definition.id.clone()),
          trigger_handler: Some(trigger_definition.handler.clone()),
          trigger_occurrence_id: Some(generated_id("occ")),
          ingress: None,
          trigger,
        }),
      })?;
      return Ok(());
    }
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

  fn definition_hash(&self) -> &str {
    self.definition_hash()
  }

  fn start_run(
    &mut self,
    run_id: &str,
    trigger: Map<String, Value>,
  ) -> Result<(), RuntimeExecutionError> {
    if self.event_schema_version() >= crate::RUN_EVENT_SCHEMA_VERSION_V7 {
      let trigger_definition = self
        .workflow()
        .triggers
        .iter()
        .find(|candidate| candidate.handler == "trigger.manual")
        .ok_or_else(|| {
          RuntimeExecutionError::InvalidConfiguration(
            "a direct Model v7+ run requires a manual trigger".to_string(),
          )
        })?;
      DurableDagEngine::append_payload(
        self,
        generated_id("evt"),
        run_id,
        chrono::Utc::now(),
        RunEventPayload::RunStarted(crate::RunStartedData {
          workflow_id: self.workflow().workflow_id.clone(),
          definition_hash: self.definition_hash().to_string(),
          trigger_id: Some(trigger_definition.id.clone()),
          trigger_handler: Some(trigger_definition.handler.clone()),
          trigger_occurrence_id: Some(generated_id("occ")),
          ingress: None,
          trigger,
        }),
      )?;
      return Ok(());
    }
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

  fn record_step_attempt_failure(
    &mut self,
    run_id: &str,
    failed_at: chrono::DateTime<chrono::Utc>,
    failure: StepAttemptFailedData,
  ) -> Result<StepFailureDisposition, RuntimeExecutionError> {
    if self.event_schema_version() >= crate::RUN_EVENT_SCHEMA_VERSION_V6 {
      return Ok(
        DurableDagEngine::record_step_attempt_failure(self, run_id, failed_at, failure)?
          .disposition,
      );
    }
    if self
      .workflow()
      .parallel_group_for_child(&failure.node_id)
      .is_some()
    {
      DurableDagEngine::append_payload(
        self,
        generated_id("evt"),
        run_id,
        failed_at,
        RunEventPayload::StepAttemptFailed(failure),
      )?;
      return Ok(StepFailureDisposition::StepFailed);
    }
    let run_failed = attempt_run_failed_data(self.event_schema_version(), &failure);
    self.append_payloads(
      run_id,
      vec![
        RunEventPayload::StepAttemptFailed(failure),
        RunEventPayload::RunFailed(run_failed),
      ],
    )?;
    Ok(StepFailureDisposition::RunFailed)
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

  fn request_approval(
    &mut self,
    run_id: &str,
    occurred_at: chrono::DateTime<chrono::Utc>,
    request: ApprovalRequestedData,
  ) -> Result<IssuedApprovalToken, RuntimeExecutionError> {
    Ok(DurableDagEngine::request_approval(
      self,
      run_id,
      occurred_at,
      request,
    )?)
  }

  fn reissue_waiting_outcome(
    &mut self,
    run_id: &str,
    now: chrono::DateTime<chrono::Utc>,
  ) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
    let projection = self.projection(run_id)?;
    let request = projection
      .approval_requests
      .values()
      .find(|request| matches!(request.status, ApprovalRequestStatus::Waiting))
      .cloned()
      .ok_or_else(|| {
        RuntimeExecutionError::Stalled("waiting run has no unresolved approval request".to_string())
      })?;
    let approval = self
      .workflow()
      .approval(&request.approval_id)
      .ok_or_else(|| {
        RuntimeExecutionError::Stalled(
          "waiting run references an unknown approval definition".to_string(),
        )
      })?;
    let token = DurableDagEngine::reissue_waiting_approval_token(
      self,
      run_id,
      &request.approval_id,
      &request.request_id,
      now,
    )?;
    Ok(waiting_outcome(
      self.workflow().workflow_id.clone(),
      run_id.to_string(),
      approval,
      request.request_id,
      request.expires_at,
      request.on_timeout,
      token,
    ))
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
