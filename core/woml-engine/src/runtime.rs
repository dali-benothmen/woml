use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::future::{poll_fn, Future};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::task::Poll;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use crate::engine::{
  node_is_complete, resolve_context_reference, selected_branch_arm, selected_choice_arm,
  step_effect_idempotency_key, BranchEvaluationError, BranchEvaluationErrorKind,
};
use crate::event::{
  ApprovalFailure, ApprovalRequestedData, ApprovalTimeoutPolicy, BranchSelectedData,
  ChoiceSelectedData, FinalLifecycleStatus, ForkBranchOutcome, ForkBranchSettledData,
  ForkJoinOutcome, ForkJoinSettledData, ForkOpenedData, LifecycleActionFailedData,
  LifecycleActionIdentityData, LifecycleFailure, LifecycleFailureKind, LifecycleHookCompletedData,
  LifecycleHookCompletionStatus, LifecycleHookRequestedData, LifecycleSubject,
  LifecycleSubjectKind, OperationExecutionMode, OperationFailedData, OperationStartedData,
  OperationSucceededData, ParallelFailure, ParallelFailurePolicy, ParallelGroupCompletedData,
  ParallelGroupOutcome, ParallelGroupStartedData, RunFailedData, RunFailedDataV1, RunFailedDataV2,
  RunFailedDataV3, RunFinalizedData, RunOutcomeDecidedData, RunSucceededData,
  StepAttemptFailedData, StepAttemptStartedData, StepAttemptSucceededData,
};
use crate::interval::{IntervalProgress, IntervalProgressReporter};
use crate::model::{
  ApprovalDefinition, CompiledLifecycleAction, LifecycleEventName, ParallelGroupDefinition,
  TemplatePart, ValueExpression,
};
use crate::projection::{
  ApprovalRequestStatus, AttemptStatus, LifecycleActionStatus, LifecycleHookProjection,
  LifecycleHookStatus, ParallelGroupStatus,
};
use crate::protocol::{
  ExecuteMessage, HostOutcome, LifecycleBindingV1, LifecycleFailureBindingV1,
  LifecycleStepBindingV1, LifecycleWorkflowBindingV1, RuntimeModuleBinding, ScriptAttempt,
};
use crate::schedule::{
  ScheduleClock, ScheduleProgress, ScheduleProgressReporter, SystemScheduleClock,
};
use crate::workflow_calls::WorkflowCallProgressReporter;
use crate::{
  run_event_schema_version_for_model, ApprovalDecisionOutcome, ApprovalTimeoutSettlement,
  AttemptFailure, AttemptFailureKind, BranchFailure, CapabilityFailure, CapabilityFailureKind,
  CapabilityRegistry, CompiledWorkflowDefinition, DurableCapabilityAuthority, DurableDagEngine,
  DurableEngineError, DurableEventStore, DurableStoreError, EngineError, InMemoryDagEngine,
  InformationalNotificationDeliverMessage, IssuedApprovalToken, NotificationCredentials,
  NotificationHostClient, NotificationHostClientError, NotificationHostOutcome,
  NotificationHostProcessOptions, OperationStatus, PolicyClaimWaitReason,
  PolicyExecutionClaimResult, PolicyWaitingFor, RecoveryReport, RunEvent, RunEventPayload,
  RunFailure, RunProjection, RunStatus, RunTimeoutSettlement, SchedulerClaimV1, ScriptHostClient,
  ScriptHostClientError, ScriptHostModuleArtifact, ScriptHostProcessOptions,
  StepFailureDisposition, TriggerAdmissionRequest, WorkflowContext,
  INFORMATIONAL_NOTIFICATION_PROVIDER_PROTOCOL_VERSION, NOTIFICATION_PROVIDER_PROTOCOL,
  RUN_EVENT_SCHEMA_VERSION_V1, RUN_EVENT_SCHEMA_VERSION_V2,
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

pub const LIFECYCLE_PROGRESS_PROFILE: &str = "woml.lifecycle-progress/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleProgressPhase {
  HookRequested,
  ActionStarted,
  ActionSucceeded,
  ActionFailed,
  HookCompleted,
  RunFinalizing,
  RunFinalized,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleProgress {
  pub profile: &'static str,
  pub run_id: String,
  pub workflow_id: String,
  pub phase: LifecycleProgressPhase,
  pub hook_id: String,
  pub action_id: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub step_id: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub code: Option<String>,
}

pub type LifecycleProgressReporter = Arc<dyn Fn(LifecycleProgress) + Send + Sync>;

pub const RUNTIME_POLICY_PROGRESS_PROFILE: &str = "woml.runtime-policy-progress/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimePolicyProgressPhase {
  Queued,
  Eligible,
  Started,
  TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimePolicyProgress {
  pub profile: &'static str,
  pub run_id: String,
  pub workflow_id: String,
  pub phase: RuntimePolicyProgressPhase,
  pub queue: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub waiting_for: Option<PolicyWaitingFor>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub eligible_at: Option<chrono::DateTime<chrono::Utc>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub code: Option<String>,
}

pub type RuntimePolicyProgressReporter = Arc<dyn Fn(RuntimePolicyProgress) + Send + Sync>;
pub(crate) type PolicyExecutionRegistry =
  Arc<tokio::sync::RwLock<HashMap<String, std::sync::Weak<PolicyExecutionCoordinator>>>>;

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
  pub notification_host: Option<NotificationHostProcessOptions>,
  pub script_timeout_ms: u64,
  pub max_context_bytes: Option<usize>,
  pub clock: Arc<dyn EngineClock>,
  pub progress_reporter: Option<ExecutionProgressReporter>,
  pub lifecycle_progress_reporter: Option<LifecycleProgressReporter>,
  pub schedule_clock: Arc<dyn ScheduleClock>,
  pub schedule_progress_reporter: Option<ScheduleProgressReporter>,
  pub interval_progress_reporter: Option<IntervalProgressReporter>,
  pub workflow_call_progress_reporter: Option<WorkflowCallProgressReporter>,
  pub runtime_policy_progress_reporter: Option<RuntimePolicyProgressReporter>,
  pub resolved_secrets: Arc<BTreeMap<String, String>>,
  pub capability_registry: Arc<CapabilityRegistry>,
  pub runtime_modules: Arc<Vec<RuntimeModuleArtifact>>,
  capability_authority: Option<Arc<DurableCapabilityAuthority>>,
  managed_database_pool: Option<Arc<crate::ManagedDatabasePool>>,
  managed_storage_store: Option<Arc<crate::ManagedStorageStore>>,
  managed_cache_store: Option<Arc<crate::ManagedCacheStore>>,
  managed_durable_state_store: Option<Arc<crate::ManagedDurableStateStore>>,
  policy_execution: Option<Arc<PolicyExecutionCoordinator>>,
  policy_execution_registry: PolicyExecutionRegistry,
}

impl std::fmt::Debug for RuntimeExecutionOptions {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    formatter
      .debug_struct("RuntimeExecutionOptions")
      .field("script_host", &self.script_host)
      .field("script_timeout_ms", &self.script_timeout_ms)
      .field("notification_host", &self.notification_host)
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
        "lifecycle_progress_reporter",
        &self
          .lifecycle_progress_reporter
          .as_ref()
          .map(|_| "configured"),
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
      .field(
        "workflow_call_progress_reporter",
        &self
          .workflow_call_progress_reporter
          .as_ref()
          .map(|_| "configured"),
      )
      .finish()
  }
}

impl RuntimeExecutionOptions {
  pub fn new(script_host: ScriptHostProcessOptions, script_timeout_ms: u64) -> Self {
    let notification_host_path = script_host.host_script_path.parent().map(|parent| {
      let extension = script_host
        .host_script_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("js");
      parent.join(format!("notification-provider-host.{extension}"))
    });
    let notification_host = notification_host_path.map(|path| {
      NotificationHostProcessOptions::new(script_host.bun_executable.clone(), path)
        .with_protocol_version(INFORMATIONAL_NOTIFICATION_PROVIDER_PROTOCOL_VERSION)
    });
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
    let managed_durable_state_store = Arc::new(crate::ManagedDurableStateStore::default());
    for handler in
      crate::ManagedDurableStateHandler::handlers(Arc::clone(&managed_durable_state_store))
    {
      capability_registry
        .register(handler)
        .expect("each production durable state operation is registered exactly once");
    }
    Self {
      script_host,
      notification_host,
      script_timeout_ms,
      max_context_bytes: None,
      clock: Arc::new(SystemEngineClock),
      progress_reporter: None,
      lifecycle_progress_reporter: None,
      schedule_clock: Arc::new(SystemScheduleClock),
      schedule_progress_reporter: None,
      interval_progress_reporter: None,
      workflow_call_progress_reporter: None,
      runtime_policy_progress_reporter: None,
      resolved_secrets: Arc::new(BTreeMap::new()),
      capability_registry,
      runtime_modules: Arc::new(Vec::new()),
      capability_authority: None,
      managed_database_pool: Some(managed_database_pool),
      managed_storage_store: Some(managed_storage_store),
      managed_cache_store: Some(managed_cache_store),
      managed_durable_state_store: Some(managed_durable_state_store),
      policy_execution: None,
      policy_execution_registry: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
    }
  }

  pub fn with_clock(mut self, clock: Arc<dyn EngineClock>) -> Self {
    self.clock = clock;
    self
  }

  pub fn with_notification_host(mut self, host: NotificationHostProcessOptions) -> Self {
    self.notification_host =
      Some(host.with_protocol_version(INFORMATIONAL_NOTIFICATION_PROVIDER_PROTOCOL_VERSION));
    self
  }

  pub fn with_progress_reporter(mut self, reporter: ExecutionProgressReporter) -> Self {
    self.progress_reporter = Some(reporter);
    self
  }

  pub fn with_lifecycle_progress_reporter(mut self, reporter: LifecycleProgressReporter) -> Self {
    self.lifecycle_progress_reporter = Some(reporter);
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
    self.managed_durable_state_store = None;
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

  pub fn with_workflow_call_progress_reporter(
    mut self,
    reporter: WorkflowCallProgressReporter,
  ) -> Self {
    self.workflow_call_progress_reporter = Some(reporter);
    self
  }

  pub fn with_runtime_policy_progress_reporter(
    mut self,
    reporter: RuntimePolicyProgressReporter,
  ) -> Self {
    self.runtime_policy_progress_reporter = Some(reporter);
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

  fn report_runtime_policy(&self, progress: RuntimePolicyProgress) {
    if let Some(reporter) = &self.runtime_policy_progress_reporter {
      reporter(progress);
    }
  }

  fn report(&self, progress: ExecutionProgress) {
    if let Some(reporter) = &self.progress_reporter {
      reporter(progress);
    }
  }

  fn report_lifecycle(&self, progress: LifecycleProgress) {
    if let Some(reporter) = &self.lifecycle_progress_reporter {
      reporter(progress);
    }
  }

  async fn release_policy_execution_slot(&self) -> Result<(), RuntimeExecutionError> {
    let Some(coordinator) = &self.policy_execution else {
      return Ok(());
    };
    let lease = coordinator.lease.lock().await.take();
    if let Some(lease) = lease {
      lease.release().await?;
    }
    Ok(())
  }

  pub(crate) async fn suspend_policy_execution_slot(&self) -> Result<(), RuntimeExecutionError> {
    let Some(coordinator) = &self.policy_execution else {
      return Ok(());
    };
    coordinator.suspend().await
  }

  pub(crate) fn policy_execution_registry(&self) -> PolicyExecutionRegistry {
    Arc::clone(&self.policy_execution_registry)
  }

  async fn ensure_policy_execution_slot(&self) -> Result<(), RuntimeExecutionError> {
    let Some(coordinator) = &self.policy_execution else {
      return Ok(());
    };
    let mut current = coordinator.lease.lock().await;
    if let Some(lease) = current.as_mut() {
      lease.resume().await?;
      return Ok(());
    }
    match acquire_policy_execution_lease(&coordinator.database_path, &coordinator.run_id, self)
      .await?
    {
      PolicyClaimAcquisition::Claimed(lease) => {
        *current = Some(lease);
        Ok(())
      }
      PolicyClaimAcquisition::Recovered => Err(RuntimeExecutionError::Stalled(format!(
        "run {:?} required fail-closed recovery while reacquiring a policy slot",
        coordinator.run_id
      ))),
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
  #[error(transparent)]
  RunCancelled(Box<CancelledRunDetails>),
  #[error("workflow execution stalled: {0}")]
  Stalled(String),
  #[error("runtime configuration is invalid: {0}")]
  InvalidConfiguration(String),
}

#[derive(Debug, Error)]
#[error("workflow run {run_id:?} was cancelled [{code}]")]
pub struct CancelledRunDetails {
  pub code: String,
  pub run_id: String,
  pub cancellation_request_id: String,
  pub events: Vec<RunEvent>,
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

const POLICY_CLAIM_LEASE: Duration = Duration::from_secs(15);
const POLICY_CLAIM_HEARTBEAT: Duration = Duration::from_secs(5);
const POLICY_QUEUE_RECHECK: Duration = Duration::from_millis(250);

fn policy_wakeup(database_path: &std::path::Path) -> Arc<tokio::sync::Notify> {
  static WAKEUPS: OnceLock<
    std::sync::Mutex<HashMap<PathBuf, std::sync::Weak<tokio::sync::Notify>>>,
  > = OnceLock::new();
  let wakeups = WAKEUPS.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
  let mut wakeups = wakeups
    .lock()
    .unwrap_or_else(|poisoned| poisoned.into_inner());
  if let Some(wakeup) = wakeups
    .get(database_path)
    .and_then(std::sync::Weak::upgrade)
  {
    return wakeup;
  }
  let wakeup = Arc::new(tokio::sync::Notify::new());
  wakeups.insert(database_path.to_path_buf(), Arc::downgrade(&wakeup));
  wakeup
}

async fn wait_for_policy_wakeup(database_path: &std::path::Path) {
  let wakeup = policy_wakeup(database_path);
  tokio::select! {
    _ = wakeup.notified() => {}
    _ = tokio::time::sleep(POLICY_QUEUE_RECHECK) => {}
  }
}

async fn wait_for_policy_rate_eligibility(
  database_path: &std::path::Path,
  eligible_at: chrono::DateTime<chrono::Utc>,
  policy_now: chrono::DateTime<chrono::Utc>,
) {
  let wakeup = policy_wakeup(database_path);
  let wait = (eligible_at - policy_now)
    .to_std()
    .unwrap_or(Duration::ZERO);
  tokio::select! {
    _ = wakeup.notified() => {}
    _ = tokio::time::sleep(wait) => {}
  }
}

struct PolicyExecutionLease {
  database_path: PathBuf,
  claim: SchedulerClaimV1,
  stop: Option<tokio::sync::oneshot::Sender<()>>,
  heartbeat: tokio::task::JoinHandle<()>,
  lost: Arc<AtomicBool>,
  suspended: bool,
}

pub(crate) struct PolicyExecutionCoordinator {
  database_path: PathBuf,
  run_id: String,
  lease: tokio::sync::Mutex<Option<PolicyExecutionLease>>,
}

impl PolicyExecutionCoordinator {
  pub(crate) async fn suspend(&self) -> Result<(), RuntimeExecutionError> {
    let mut lease = self.lease.lock().await;
    if let Some(lease) = lease.as_mut() {
      lease.suspend()?;
    }
    Ok(())
  }

  pub(crate) async fn resume(&self) -> Result<(), RuntimeExecutionError> {
    let mut lease = self.lease.lock().await;
    if let Some(lease) = lease.as_mut() {
      lease.resume().await?;
    }
    Ok(())
  }
}

impl PolicyExecutionLease {
  fn start(database_path: PathBuf, claim: SchedulerClaimV1) -> Self {
    let (stop, mut stopped) = tokio::sync::oneshot::channel();
    let heartbeat_path = database_path.clone();
    let heartbeat_claim = claim.clone();
    let lost = Arc::new(AtomicBool::new(false));
    let heartbeat_lost = Arc::clone(&lost);
    let heartbeat = tokio::spawn(async move {
      loop {
        tokio::select! {
          _ = &mut stopped => break,
          _ = tokio::time::sleep(POLICY_CLAIM_HEARTBEAT) => {
            let renewed = DurableEventStore::open(heartbeat_path.clone()).and_then(|mut store| {
              store.renew_policy_claim(
                &heartbeat_claim.run_id,
                &heartbeat_claim.owner_id,
                &heartbeat_claim.claim_id,
                chrono::Utc::now(),
                POLICY_CLAIM_LEASE,
              )
            });
            if renewed.is_err() {
              heartbeat_lost.store(true, Ordering::Release);
              break;
            }
          }
        }
      }
    });
    Self {
      database_path,
      claim,
      stop: Some(stop),
      heartbeat,
      lost,
      suspended: false,
    }
  }

  fn suspend(&mut self) -> Result<(), RuntimeExecutionError> {
    if self.suspended {
      return Ok(());
    }
    let mut store = DurableEventStore::open(self.database_path.clone())?;
    if !store.suspend_policy_claim(
      &self.claim.run_id,
      &self.claim.owner_id,
      &self.claim.claim_id,
    )? {
      return Err(RuntimeExecutionError::Stalled(format!(
        "scheduler ownership was lost for run {:?}",
        self.claim.run_id
      )));
    }
    self.suspended = true;
    policy_wakeup(&self.database_path).notify_waiters();
    Ok(())
  }

  async fn resume(&mut self) -> Result<(), RuntimeExecutionError> {
    while self.suspended {
      if self.lost.load(Ordering::Acquire) {
        return Err(RuntimeExecutionError::Stalled(format!(
          "scheduler ownership was lost for run {:?}",
          self.claim.run_id
        )));
      }
      let resumed = DurableEventStore::open(self.database_path.clone())?.resume_policy_claim(
        &self.claim.run_id,
        &self.claim.owner_id,
        &self.claim.claim_id,
        chrono::Utc::now(),
      )?;
      if resumed {
        self.suspended = false;
        break;
      }
      wait_for_policy_wakeup(&self.database_path).await;
    }
    Ok(())
  }

  async fn release(mut self) -> Result<(), RuntimeExecutionError> {
    if let Some(stop) = self.stop.take() {
      let _ = stop.send(());
    }
    let _ = self.heartbeat.await;
    let mut store = DurableEventStore::open(self.database_path.clone())?;
    let released = store.release_policy_claim(
      &self.claim.run_id,
      &self.claim.owner_id,
      &self.claim.claim_id,
    )?;
    if !released || self.lost.load(Ordering::Acquire) {
      return Err(RuntimeExecutionError::Stalled(format!(
        "scheduler ownership was lost for run {:?}",
        self.claim.run_id
      )));
    }
    policy_wakeup(&self.database_path).notify_waiters();
    Ok(())
  }
}

enum PolicyClaimAcquisition {
  Claimed(PolicyExecutionLease),
  Recovered,
}

async fn acquire_policy_execution_lease(
  database_path: &std::path::Path,
  run_id: &str,
  options: &RuntimeExecutionOptions,
) -> Result<PolicyClaimAcquisition, RuntimeExecutionError> {
  let owner_id = format!("scheduler_{}", Uuid::new_v4().simple());
  let mut reported_wait = false;
  loop {
    let lease_now = chrono::Utc::now();
    let policy_now = options.clock.now();
    let mut store = DurableEventStore::open(database_path.to_path_buf())?;
    let rate_eligible_at;
    match store.try_claim_policy_run_at(
      run_id,
      &owner_id,
      lease_now,
      policy_now,
      POLICY_CLAIM_LEASE,
    ) {
      Ok(PolicyExecutionClaimResult::Claimed { claim, .. }) => {
        let binding = store.run_binding(run_id)?;
        let queue = store
          .definition(&binding.definition_hash)?
          .runtime_policy_queue_name()
          .ok_or_else(|| {
            RuntimeExecutionError::InvalidConfiguration(
              "Model v12 has no runtime policy queue".to_string(),
            )
          })?;
        options.report_runtime_policy(RuntimePolicyProgress {
          profile: RUNTIME_POLICY_PROGRESS_PROFILE,
          run_id: run_id.to_string(),
          workflow_id: claim.workflow_id.clone(),
          phase: RuntimePolicyProgressPhase::Eligible,
          queue: queue.clone(),
          waiting_for: None,
          eligible_at: None,
          code: None,
        });
        options.report_runtime_policy(RuntimePolicyProgress {
          profile: RUNTIME_POLICY_PROGRESS_PROFILE,
          run_id: run_id.to_string(),
          workflow_id: claim.workflow_id.clone(),
          phase: RuntimePolicyProgressPhase::Started,
          queue,
          waiting_for: None,
          eligible_at: None,
          code: None,
        });
        return Ok(PolicyClaimAcquisition::Claimed(
          PolicyExecutionLease::start(database_path.to_path_buf(), claim),
        ));
      }
      Ok(PolicyExecutionClaimResult::Waiting {
        workflow_id,
        queue,
        reason,
        eligible_at,
        ..
      }) => {
        let waiting_for = match reason {
          PolicyClaimWaitReason::Concurrency => PolicyWaitingFor::Concurrency,
          PolicyClaimWaitReason::RateLimit => PolicyWaitingFor::RateLimit,
        };
        if !reported_wait {
          options.report_runtime_policy(RuntimePolicyProgress {
            profile: RUNTIME_POLICY_PROGRESS_PROFILE,
            run_id: run_id.to_string(),
            workflow_id,
            phase: RuntimePolicyProgressPhase::Queued,
            queue,
            waiting_for: Some(waiting_for),
            eligible_at,
            code: None,
          });
          reported_wait = true;
        }
        rate_eligible_at = eligible_at;
      }
      Err(DurableStoreError::SchedulerRecoveryRequired(_)) => {
        store.recover_policy_run_after_lease_loss(run_id, lease_now)?;
        return Ok(PolicyClaimAcquisition::Recovered);
      }
      Err(error) => return Err(error.into()),
    }
    if let Some(eligible_at) = rate_eligible_at {
      wait_for_policy_rate_eligibility(database_path, eligible_at, policy_now).await;
    } else {
      wait_for_policy_wakeup(database_path).await;
    }
  }
}

fn schedule_workflow_timeout(
  database_path: PathBuf,
  run_id: String,
  workflow_id: String,
  queue: String,
  deadline_at: chrono::DateTime<chrono::Utc>,
  options: RuntimeExecutionOptions,
) {
  tokio::spawn(async move {
    let mut deadline = deadline_at;
    loop {
      let wait = (deadline - options.clock.now())
        .to_std()
        .unwrap_or(Duration::ZERO);
      tokio::time::sleep(wait).await;
      let settlement = DurableEventStore::open(database_path.clone()).and_then(|mut store| {
        let settlement = store.settle_run_timeout(&run_id, options.clock.now())?;
        let needs_lifecycle = matches!(
          &settlement,
          RunTimeoutSettlement::TimedOut { projection }
            if !projection.lifecycle_hooks.is_empty()
              && !store.policy_run_has_live_claim(&run_id, chrono::Utc::now())?
        );
        if matches!(
          &settlement,
          RunTimeoutSettlement::TimedOut { projection }
            if projection.lifecycle_hooks.is_empty()
        ) {
          let _ = store.finalize_run_v11(&run_id, options.clock.now());
        }
        Ok((settlement, needs_lifecycle))
      });
      match settlement {
        Ok((RunTimeoutSettlement::NotDue { deadline_at }, _)) => deadline = deadline_at,
        Ok((RunTimeoutSettlement::TimedOut { .. }, needs_lifecycle)) => {
          if let Some(reporter) = &options.runtime_policy_progress_reporter {
            reporter(RuntimePolicyProgress {
              profile: RUNTIME_POLICY_PROGRESS_PROFILE,
              run_id: run_id.clone(),
              workflow_id: workflow_id.clone(),
              phase: RuntimePolicyProgressPhase::TimedOut,
              queue: queue.clone(),
              waiting_for: None,
              eligible_at: None,
              code: Some("WOML_WORKFLOW_TIMED_OUT".to_string()),
            });
          }
          policy_wakeup(&database_path).notify_waiters();
          if needs_lifecycle {
            let _ =
              finish_timed_out_policy_lifecycle(database_path.clone(), &run_id, options.clone())
                .await;
          }
          break;
        }
        Ok(_) => {
          policy_wakeup(&database_path).notify_waiters();
          break;
        }
        Err(_) => tokio::time::sleep(CANCELLATION_POLL_INTERVAL).await,
      }
    }
  });
}

async fn finish_timed_out_policy_lifecycle(
  database_path: PathBuf,
  run_id: &str,
  options: RuntimeExecutionOptions,
) -> Result<(), RuntimeExecutionError> {
  let acquisition = acquire_policy_execution_lease(&database_path, run_id, &options).await?;
  let store = DurableEventStore::open(database_path.clone())?;
  let binding = store.run_binding(run_id)?;
  let mut options = runtime_modules_from_store(options, &store, &binding.definition_hash)?;
  let engine = DurableDagEngine::resume(store, run_id)?;
  match acquisition {
    PolicyClaimAcquisition::Recovered => {
      let _ = resume_with_engine(engine, run_id, options).await;
    }
    PolicyClaimAcquisition::Claimed(lease) => {
      let coordinator = Arc::new(PolicyExecutionCoordinator {
        database_path: database_path.clone(),
        run_id: run_id.to_string(),
        lease: tokio::sync::Mutex::new(Some(lease)),
      });
      options.policy_execution = Some(Arc::clone(&coordinator));
      options
        .policy_execution_registry
        .write()
        .await
        .insert(run_id.to_string(), Arc::downgrade(&coordinator));
      let _ = resume_with_engine(engine, run_id, options.clone()).await;
      let released = options.release_policy_execution_slot().await;
      options
        .policy_execution_registry
        .write()
        .await
        .remove(run_id);
      released?;
    }
  }
  Ok(())
}

async fn execute_policy_run_durable(
  database_path: PathBuf,
  run_id: &str,
  options: RuntimeExecutionOptions,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  {
    let store = DurableEventStore::open(database_path.clone())?;
    let binding = store.run_binding(run_id)?;
    let workflow = store.definition(&binding.definition_hash)?;
    let policy = workflow.runtime_policy.as_ref().ok_or_else(|| {
      RuntimeExecutionError::InvalidConfiguration(
        "Model v12 execution requires a runtime policy".to_string(),
      )
    })?;
    let projection = store.projection(run_id)?;
    if let Some(deadline_at) = projection.timeout_at {
      schedule_workflow_timeout(
        database_path.clone(),
        run_id.to_string(),
        workflow.workflow_id.clone(),
        policy
          .queue
          .as_ref()
          .map_or_else(|| workflow.workflow_id.clone(), |queue| queue.name.clone()),
        deadline_at,
        options.clone(),
      );
    }
    if matches!(
      projection.status,
      RunStatus::Waiting | RunStatus::Succeeded | RunStatus::Failed | RunStatus::Cancelled
    ) {
      let binding = store.run_binding(run_id)?;
      let options = runtime_modules_from_store(options, &store, &binding.definition_hash)?;
      let engine = DurableDagEngine::resume(store, run_id)?;
      return resume_with_engine(engine, run_id, options).await;
    }
  }
  let acquisition = acquire_policy_execution_lease(&database_path, run_id, &options).await?;
  let store = DurableEventStore::open(database_path.clone())?;
  let binding = store.run_binding(run_id)?;
  let mut options = runtime_modules_from_store(options, &store, &binding.definition_hash)?;
  let workflow = store.definition(&binding.definition_hash)?;
  if let Some(deadline_at) = store.projection(run_id)?.timeout_at {
    let queue = workflow.runtime_policy_queue_name().ok_or_else(|| {
      RuntimeExecutionError::InvalidConfiguration(
        "Model v12 has no runtime policy queue".to_string(),
      )
    })?;
    schedule_workflow_timeout(
      database_path.clone(),
      run_id.to_string(),
      workflow.workflow_id,
      queue,
      deadline_at,
      options.clone(),
    );
  }
  let engine = DurableDagEngine::resume(store, run_id)?;
  match acquisition {
    PolicyClaimAcquisition::Recovered => resume_with_engine(engine, run_id, options).await,
    PolicyClaimAcquisition::Claimed(lease) => {
      let coordinator = Arc::new(PolicyExecutionCoordinator {
        database_path: database_path.clone(),
        run_id: run_id.to_string(),
        lease: tokio::sync::Mutex::new(Some(lease)),
      });
      options.policy_execution = Some(Arc::clone(&coordinator));
      options
        .policy_execution_registry
        .write()
        .await
        .insert(run_id.to_string(), Arc::downgrade(&coordinator));
      let outcome = resume_with_engine(engine, run_id, options.clone()).await;
      let released = options.release_policy_execution_slot().await;
      options
        .policy_execution_registry
        .write()
        .await
        .remove(run_id);
      match (outcome, released) {
        (Ok(outcome), Ok(())) => Ok(outcome),
        (Err(error), Ok(())) => Err(error),
        (_, Err(error)) => Err(error),
      }
    }
  }
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
  let mut store = DurableEventStore::open(database_path.clone())?;
  if workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V12 {
    store.register_definition_module_artifacts(
      &workflow,
      &definition_hash,
      options.runtime_modules.as_ref(),
    )?;
    let trigger_definition = workflow
      .triggers
      .iter()
      .find(|candidate| candidate.handler == "trigger.manual")
      .ok_or_else(|| {
        RuntimeExecutionError::InvalidConfiguration(
          "a direct Model v12+ run requires a manual trigger".to_string(),
        )
      })?;
    let admission = store.admit_trigger_occurrence(TriggerAdmissionRequest {
      workflow_id: workflow.workflow_id.clone(),
      definition_hash,
      trigger_id: trigger_definition.id.clone(),
      trigger_handler: trigger_definition.handler.clone(),
      source_identity: format!("manual:{}", Uuid::new_v4().simple()),
      payload: trigger,
      received_at: options.clock.now(),
    })?;
    drop(store);
    return execute_policy_run_durable(database_path, &admission.run_id, options).await;
  }
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
  let store = DurableEventStore::open(database_path.clone())?;
  let binding = store.run_binding(run_id)?;
  let workflow = store.definition(&binding.definition_hash)?;
  if workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V12 {
    drop(store);
    return execute_policy_run_durable(database_path, run_id, options).await;
  }
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
  let mut store = DurableEventStore::open(database_path.clone())?;
  let binding = store.run_binding(run_id)?;
  let workflow = store.definition(&binding.definition_hash)?;
  let has_approval = workflow_has_approval(&workflow);
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
  if workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V12 {
    drop(store);
    return execute_policy_run_durable(database_path, run_id, options).await;
  }
  store.recover_interrupted_runs()?;
  let options = runtime_modules_from_store(options, &store, &binding.definition_hash)?;
  let engine = DurableDagEngine::resume(store, run_id)?;
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
  if let Some(state) = &options.managed_durable_state_store {
    state
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
  let now = clock.now();
  let projection = store.projection(run_id)?;
  if projection.timeout_reached_at.is_some() {
    let request = projection
      .approval_requests
      .get(approval_id)
      .ok_or_else(|| {
        RuntimeExecutionError::Stalled(
          "workflow timeout references an unknown approval request".to_string(),
        )
      })?;
    return Ok(ApprovalTimeoutSettlement {
      status: crate::ApprovalTimeoutSettlementStatus::Settled,
      run_id: run_id.to_string(),
      approval_id: approval_id.to_string(),
      request_id: request.request_id.clone(),
      resolution: None,
      settled_at: projection.timeout_reached_at,
    });
  }
  if projection
    .timeout_at
    .is_some_and(|deadline_at| now >= deadline_at)
    && projection.business_outcome.is_none()
    && projection.cancellation_request_id.is_none()
  {
    let settlement = store.settle_run_timeout(run_id, now)?;
    let timed_out = match settlement {
      RunTimeoutSettlement::TimedOut { projection }
      | RunTimeoutSettlement::LostRace { projection } => projection
        .timeout_reached_at
        .is_some()
        .then_some(projection),
      RunTimeoutSettlement::NotConfigured | RunTimeoutSettlement::NotDue { .. } => None,
    };
    if let Some(projection) = timed_out {
      let request = projection
        .approval_requests
        .get(approval_id)
        .ok_or_else(|| {
          RuntimeExecutionError::Stalled(
            "workflow timeout references an unknown approval request".to_string(),
          )
        })?;
      return Ok(ApprovalTimeoutSettlement {
        status: crate::ApprovalTimeoutSettlementStatus::Settled,
        run_id: run_id.to_string(),
        approval_id: approval_id.to_string(),
        request_id: request.request_id.clone(),
        resolution: None,
        settled_at: projection.timeout_reached_at,
      });
    }
  }
  Ok(store.settle_approval_timeout(run_id, approval_id, now)?)
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
    RunStatus::Cancelled => {
      return Err(cancelled_run_error(&engine, run_id)?);
    }
    RunStatus::Finalizing => {
      let mut host = None;
      drive_workflow_lifecycle(&mut engine, run_id, &options, &mut host).await?;
      if let Some(host) = host {
        host.shutdown().await;
      }
      let projection = engine.projection(run_id)?;
      return match projection.status {
        RunStatus::Succeeded => Ok(WorkflowRuntimeOutcome::succeeded(final_result(
          &engine,
          run_id,
          execution_order,
        )?)),
        RunStatus::Failed => Err(resumed_failure(&engine, run_id, projection)?),
        RunStatus::Cancelled => Err(cancelled_run_error(&engine, run_id)?),
        _ => Err(RuntimeExecutionError::Stalled(
          "stored run did not finish lifecycle continuation".to_string(),
        )),
      };
    }
    RunStatus::Cancelling | RunStatus::Running => {}
    RunStatus::Waiting => {
      return engine.reissue_waiting_outcome(run_id, options.clock.now());
    }
    RunStatus::NotStarted | RunStatus::Queued => {
      return Err(RuntimeExecutionError::Stalled(
        "stored run has not entered execution".to_string(),
      ));
    }
  }
  let terminal_node_id = runtime_result_node_id(engine.workflow())
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
  let terminal_node_id = runtime_result_node_id(engine.workflow())
    .ok_or_else(|| RuntimeExecutionError::Stalled("no terminal node exists".to_string()))?
    .to_string();
  let run_id = generated_id("run");
  engine.start_run(&run_id, trigger)?;
  continue_runtime(engine, &run_id, terminal_node_id, Vec::new(), options).await
}

fn runtime_result_node_id(workflow: &CompiledWorkflowDefinition) -> Option<&str> {
  workflow
    .graph
    .settlement
    .as_ref()
    .map(|settlement| settlement.main_result_node_id.as_str())
    .or_else(|| workflow.terminal_node_id())
}

async fn continue_runtime<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  terminal_node_id: String,
  mut execution_order: Vec<String>,
  options: &RuntimeExecutionOptions,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  let mut host = None;
  drive_workflow_lifecycle(engine, run_id, options, &mut host).await?;
  let mut execution = continue_runtime_loop(
    engine,
    run_id,
    terminal_node_id,
    &mut execution_order,
    options,
    &mut host,
  )
  .await;
  if execution.is_err() && engine.projection(run_id)?.status == RunStatus::Cancelling {
    settle_cancelling_run(engine, run_id, options.clock.now())?;
    execution = Err(cancelled_run_error(engine, run_id)?);
  }
  let lifecycle = drive_workflow_lifecycle(engine, run_id, options, &mut host).await;
  if let Some(host) = host {
    host.shutdown().await;
  }
  lifecycle?;
  match execution {
    Ok(WorkflowRuntimeOutcome::Succeeded { execution, .. }) => Ok(
      WorkflowRuntimeOutcome::succeeded(final_result(engine, run_id, execution.execution_order)?),
    ),
    Ok(waiting @ WorkflowRuntimeOutcome::Waiting { .. }) => Ok(waiting),
    Err(RuntimeExecutionError::RunCancelled(_)) => Err(cancelled_run_error(engine, run_id)?),
    Err(error) => Err(error),
  }
}

fn settle_workflow_timeout_if_due<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  now: chrono::DateTime<chrono::Utc>,
  options: &RuntimeExecutionOptions,
) -> Result<bool, RuntimeExecutionError> {
  let projection = engine.projection(run_id)?;
  if projection.business_outcome.is_some() || projection.cancellation_request_id.is_some() {
    return Ok(false);
  }
  if projection
    .timeout_at
    .is_none_or(|deadline_at| now < deadline_at)
  {
    return Ok(false);
  }
  let settled = engine.settle_workflow_timeout(run_id, now)?;
  if settled {
    options.report_runtime_policy(RuntimePolicyProgress {
      profile: RUNTIME_POLICY_PROGRESS_PROFILE,
      run_id: run_id.to_string(),
      workflow_id: engine.workflow().workflow_id.clone(),
      phase: RuntimePolicyProgressPhase::TimedOut,
      queue: engine
        .workflow()
        .runtime_policy_queue_name()
        .unwrap_or_else(|| engine.workflow().workflow_id.clone()),
      waiting_for: None,
      eligible_at: None,
      code: Some("WOML_WORKFLOW_TIMED_OUT".to_string()),
    });
  }
  Ok(settled)
}

fn lifecycle_action_source(action: &CompiledLifecycleAction) -> Option<&str> {
  let ValueExpression::Object { fields } = &action.inputs else {
    return None;
  };
  let ValueExpression::Literal { value } = fields.get("source")? else {
    return None;
  };
  value.as_str()
}

fn resolved_lifecycle_secrets(
  action: &CompiledLifecycleAction,
  options: &RuntimeExecutionOptions,
) -> Result<BTreeMap<String, String>, RuntimeExecutionError> {
  let Some(runtime) = &action.script_runtime else {
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
            "lifecycle action {:?} requires unresolved secret {name:?}",
            action.action_id
          ))
        })
    })
    .collect()
}

#[derive(Debug)]
struct LifecycleNotificationDelivery<'a> {
  delivery_id: &'a str,
  provider: &'a str,
  destination: &'a str,
  credentials: BTreeMap<String, String>,
  message: &'a ValueExpression,
}

fn lifecycle_notification_deliveries(
  action: &CompiledLifecycleAction,
) -> Result<Vec<LifecycleNotificationDelivery<'_>>, RuntimeExecutionError> {
  let ValueExpression::Object { fields } = &action.inputs else {
    return Err(RuntimeExecutionError::InvalidConfiguration(format!(
      "lifecycle notification action {:?} has invalid inputs",
      action.action_id
    )));
  };
  let Some(ValueExpression::Array { items }) = fields.get("deliveries") else {
    return Err(RuntimeExecutionError::InvalidConfiguration(format!(
      "lifecycle notification action {:?} has no deliveries",
      action.action_id
    )));
  };
  items
    .iter()
    .map(|item| {
      let ValueExpression::Object { fields } = item else {
        return Err(RuntimeExecutionError::InvalidConfiguration(
          "lifecycle notification delivery is not an object".to_string(),
        ));
      };
      let literal = |name: &str| match fields.get(name) {
        Some(ValueExpression::Literal { value }) => value.as_str(),
        _ => None,
      };
      let ValueExpression::Object {
        fields: credential_fields,
      } = fields.get("credentials").ok_or_else(|| {
        RuntimeExecutionError::InvalidConfiguration(
          "lifecycle notification credentials are unavailable".to_string(),
        )
      })?
      else {
        return Err(RuntimeExecutionError::InvalidConfiguration(
          "lifecycle notification credentials are invalid".to_string(),
        ));
      };
      let credentials = credential_fields
        .iter()
        .map(|(name, expression)| match expression {
          ValueExpression::SecretReference { name: secret } => Ok((name.clone(), secret.clone())),
          _ => Err(RuntimeExecutionError::InvalidConfiguration(
            "lifecycle notification credentials must be symbolic secret references".to_string(),
          )),
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
      Ok(LifecycleNotificationDelivery {
        delivery_id: literal("deliveryId").ok_or_else(|| {
          RuntimeExecutionError::InvalidConfiguration(
            "lifecycle notification deliveryId is unavailable".to_string(),
          )
        })?,
        provider: literal("provider").ok_or_else(|| {
          RuntimeExecutionError::InvalidConfiguration(
            "lifecycle notification provider is unavailable".to_string(),
          )
        })?,
        destination: literal("destination").ok_or_else(|| {
          RuntimeExecutionError::InvalidConfiguration(
            "lifecycle notification destination is unavailable".to_string(),
          )
        })?,
        credentials,
        message: fields.get("message").ok_or_else(|| {
          RuntimeExecutionError::InvalidConfiguration(
            "lifecycle notification message is unavailable".to_string(),
          )
        })?,
      })
    })
    .collect()
}

fn resolve_json_path(root: &Value, path: &[String]) -> Option<Value> {
  let mut value = root;
  for segment in path {
    value = value.as_object()?.get(segment)?;
  }
  Some(value.clone())
}

fn render_template_scalar(value: Value) -> Option<String> {
  match value {
    Value::Null => Some("null".to_string()),
    Value::Bool(value) => Some(value.to_string()),
    Value::Number(value) => Some(value.to_string()),
    Value::String(value) => Some(value),
    Value::Array(_) | Value::Object(_) => None,
  }
}

fn render_lifecycle_notification_message(
  expression: &ValueExpression,
  context: &WorkflowContext,
  lifecycle: &LifecycleBindingV1,
) -> Result<String, LifecycleFailure> {
  let ValueExpression::Template { parts } = expression else {
    return Err(LifecycleFailure {
      kind: LifecycleFailureKind::ProviderFailed,
      code: "WOML_LIFECYCLE_TEMPLATE_INVALID".to_string(),
      message: "Lifecycle notification message is not a WOML Template v1 value.".to_string(),
    });
  };
  let lifecycle_json = serde_json::to_value(lifecycle).map_err(|_| LifecycleFailure {
    kind: LifecycleFailureKind::ProviderFailed,
    code: "WOML_LIFECYCLE_TEMPLATE_INVALID".to_string(),
    message: "Lifecycle notification data could not be rendered safely.".to_string(),
  })?;
  let mut rendered = String::new();
  for part in parts {
    match part {
      TemplatePart::Text { text } => rendered.push_str(text),
      TemplatePart::ContextReference { path } => {
        let value = resolve_context_reference(
          &ValueExpression::ContextReference { path: path.clone() },
          context,
        )
        .ok()
        .and_then(render_template_scalar)
        .ok_or_else(|| LifecycleFailure {
          kind: LifecycleFailureKind::ProviderFailed,
          code: "WOML_LIFECYCLE_TEMPLATE_REFERENCE_INVALID".to_string(),
          message: format!(
            "Lifecycle notification reference context.{} is unavailable or is not a scalar.",
            path.join(".")
          ),
        })?;
        rendered.push_str(&value);
      }
      TemplatePart::LifecycleReference { path } => {
        let value = resolve_json_path(&lifecycle_json, path)
          .and_then(render_template_scalar)
          .ok_or_else(|| LifecycleFailure {
            kind: LifecycleFailureKind::ProviderFailed,
            code: "WOML_LIFECYCLE_TEMPLATE_REFERENCE_INVALID".to_string(),
            message: format!(
              "Lifecycle notification reference lifecycle.{} is unavailable or is not a scalar.",
              path.join(".")
            ),
          })?;
        rendered.push_str(&value);
      }
    }
    if rendered.chars().count() > 4_096 {
      return Err(LifecycleFailure {
        kind: LifecycleFailureKind::SizeLimitExceeded,
        code: "WOML_LIFECYCLE_TEMPLATE_TOO_LARGE".to_string(),
        message: "Lifecycle notification message exceeds 4096 characters.".to_string(),
      });
    }
  }
  if rendered.is_empty() {
    return Err(LifecycleFailure {
      kind: LifecycleFailureKind::ProviderFailed,
      code: "WOML_LIFECYCLE_TEMPLATE_EMPTY".to_string(),
      message: "Lifecycle notification message rendered to an empty value.".to_string(),
    });
  }
  Ok(rendered)
}

fn notification_capability_failure(
  kind: CapabilityFailureKind,
  code: String,
  message: String,
  retryable: bool,
  ambiguous: bool,
) -> CapabilityFailure {
  CapabilityFailure {
    kind,
    code,
    message,
    retryable,
    ambiguous,
    details: None,
  }
}

fn lifecycle_failure_from_attempt(failure: AttemptFailure) -> LifecycleFailure {
  let kind = match failure.kind {
    AttemptFailureKind::ScriptThrew => LifecycleFailureKind::ScriptThrew,
    AttemptFailureKind::ServiceFailed => LifecycleFailureKind::ProviderFailed,
    AttemptFailureKind::ScriptTimedOut => LifecycleFailureKind::TimedOut,
    AttemptFailureKind::InvalidScriptResult => LifecycleFailureKind::NonJson,
    AttemptFailureKind::WorkerCrashed => LifecycleFailureKind::WorkerCrashed,
    AttemptFailureKind::HostCrashed => LifecycleFailureKind::HostCrashed,
    AttemptFailureKind::Interrupted => LifecycleFailureKind::Interrupted,
    AttemptFailureKind::ContextTooLarge | AttemptFailureKind::ResultTooLarge => {
      LifecycleFailureKind::SizeLimitExceeded
    }
    AttemptFailureKind::InvocationCancelled => LifecycleFailureKind::Cancelled,
  };
  LifecycleFailure {
    kind,
    code: failure.code,
    message: failure.message,
  }
}

fn workflow_lifecycle_request(
  workflow: &CompiledWorkflowDefinition,
  run_id: &str,
  event: LifecycleEventName,
) -> Option<RunEventPayload> {
  let hook = workflow.lifecycle_hook_for_event(event)?;
  let subject = LifecycleSubject {
    kind: LifecycleSubjectKind::Workflow,
    id: run_id.to_string(),
  };
  Some(RunEventPayload::LifecycleHookRequested(
    LifecycleHookRequestedData {
      hook_invocation_id: crate::derive_lifecycle_hook_invocation_id(
        run_id,
        &hook.hook_id,
        subject.kind,
        &subject.id,
      ),
      hook_id: hook.hook_id.clone(),
      event: hook.event,
      subject,
    },
  ))
}

fn step_lifecycle_request(
  workflow: &CompiledWorkflowDefinition,
  run_id: &str,
  event: LifecycleEventName,
  step_id: &str,
) -> Option<RunEventPayload> {
  let hook = workflow.lifecycle_hook_for_step_event(event, step_id)?;
  let subject = LifecycleSubject {
    kind: LifecycleSubjectKind::Step,
    id: step_id.to_string(),
  };
  Some(RunEventPayload::LifecycleHookRequested(
    LifecycleHookRequestedData {
      hook_invocation_id: crate::derive_lifecycle_hook_invocation_id(
        run_id,
        &hook.hook_id,
        subject.kind,
        &subject.id,
      ),
      hook_id: hook.hook_id.clone(),
      event: hook.event,
      subject,
    },
  ))
}

fn report_lifecycle(
  options: &RuntimeExecutionOptions,
  workflow: &CompiledWorkflowDefinition,
  run_id: &str,
  phase: LifecycleProgressPhase,
  hook_id: &str,
  action_id: &str,
  step_id: Option<&str>,
  code: Option<String>,
) {
  options.report_lifecycle(LifecycleProgress {
    profile: LIFECYCLE_PROGRESS_PROFILE,
    run_id: run_id.to_string(),
    workflow_id: workflow.workflow_id.clone(),
    phase,
    hook_id: hook_id.to_string(),
    action_id: action_id.to_string(),
    step_id: step_id.map(str::to_string),
    code,
  });
}

fn pending_lifecycle_hook(
  projection: &RunProjection,
  events: &[RunEvent],
) -> Option<LifecycleHookProjection> {
  let pending = |hook: &LifecycleHookProjection| {
    matches!(
      hook.status,
      LifecycleHookStatus::Requested | LifecycleHookStatus::Running
    )
  };
  for subject_kind in [LifecycleSubjectKind::Step, LifecycleSubjectKind::Workflow] {
    if let Some(hook) = events.iter().find_map(|event| {
      let RunEventPayload::LifecycleHookRequested(request) = &event.payload else {
        return None;
      };
      let hook = projection
        .lifecycle_hooks
        .get(&request.hook_invocation_id)?;
      (hook.subject.kind == subject_kind && pending(hook)).then(|| hook.clone())
    }) {
      return Some(hook);
    }
  }
  None
}

fn lifecycle_request_snapshot(
  events: &[RunEvent],
  hook_invocation_id: &str,
) -> Result<RunProjection, RuntimeExecutionError> {
  let request_index = events
    .iter()
    .position(|event| {
      matches!(
        &event.payload,
        RunEventPayload::LifecycleHookRequested(request)
          if request.hook_invocation_id == hook_invocation_id
      )
    })
    .ok_or_else(|| {
      RuntimeExecutionError::Stalled("lifecycle request event disappeared".to_string())
    })?;
  crate::fold_events(&events[..=request_index])
    .map_err(|error| RuntimeExecutionError::Stalled(error.to_string()))
}

fn step_lifecycle_outcome(
  event: LifecycleEventName,
  projection: &RunProjection,
  step_id: &str,
) -> Option<crate::BusinessOutcome> {
  match event {
    LifecycleEventName::StepStart => None,
    LifecycleEventName::StepSuccess => Some(crate::BusinessOutcome::Succeeded),
    LifecycleEventName::StepFailure => Some(crate::BusinessOutcome::Failed),
    LifecycleEventName::StepComplete => {
      if projection.context.steps.contains_key(step_id) {
        Some(crate::BusinessOutcome::Succeeded)
      } else {
        projection.latest_attempt(step_id).and_then(|attempt| {
          let AttemptStatus::Failed { failure } = &attempt.status else {
            return None;
          };
          Some(if failure.kind == AttemptFailureKind::InvocationCancelled {
            crate::BusinessOutcome::Cancelled
          } else {
            crate::BusinessOutcome::Failed
          })
        })
      }
    }
    _ => None,
  }
}

async fn execute_lifecycle_notification<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  action: &CompiledLifecycleAction,
  hook_invocation_id: &str,
  context: &WorkflowContext,
  lifecycle: &LifecycleBindingV1,
  options: &RuntimeExecutionOptions,
) -> Result<(), LifecycleFailure> {
  let deliveries = lifecycle_notification_deliveries(action).map_err(|error| LifecycleFailure {
    kind: LifecycleFailureKind::ProviderFailed,
    code: "WOML_NOTIFICATION_REQUEST_INVALID".to_string(),
    message: error.to_string(),
  })?;
  let host_options = options
    .notification_host
    .clone()
    .ok_or_else(|| LifecycleFailure {
      kind: LifecycleFailureKind::HostCrashed,
      code: "WOML_NOTIFICATION_HOST_UNAVAILABLE".to_string(),
      message: "The informational notification provider host is not configured.".to_string(),
    })?;
  let host = NotificationHostClient::spawn(host_options)
    .await
    .map_err(|error| LifecycleFailure {
      kind: LifecycleFailureKind::HostCrashed,
      code: "WOML_NOTIFICATION_HOST_CRASHED".to_string(),
      message: error.to_string(),
    })?;
  let action_key = step_effect_idempotency_key(run_id, engine.definition_hash(), &action.action_id);
  let mut failures = Vec::new();

  for delivery in deliveries {
    let message = match render_lifecycle_notification_message(delivery.message, context, lifecycle)
    {
      Ok(message) => message,
      Err(failure) => {
        failures.push(failure);
        continue;
      }
    };
    let credentials = match NotificationCredentials::from_symbolic(&delivery.credentials) {
      Ok(credentials) => credentials,
      Err(message) => {
        failures.push(LifecycleFailure {
          kind: LifecycleFailureKind::ProviderFailed,
          code: "WOML_NOTIFICATION_REQUEST_INVALID".to_string(),
          message,
        });
        continue;
      }
    };
    let operation_key = crate::derive_operation_key(&action_key, delivery.delivery_id);
    let mut delivered = false;
    for provider_attempt in 1..=3_u32 {
      let invocation_id = generated_id("ninv");
      let mut started_metadata = Map::new();
      started_metadata.insert(
        "provider".to_string(),
        Value::String(delivery.provider.to_string()),
      );
      started_metadata.insert(
        "destination".to_string(),
        Value::String(delivery.destination.to_string()),
      );
      started_metadata.insert("providerAttempt".to_string(), Value::from(provider_attempt));
      engine
        .append_payload(
          run_id,
          RunEventPayload::OperationStarted(OperationStartedData {
            node_id: action.action_id.clone(),
            attempt_number: 1,
            invocation_id: invocation_id.clone(),
            call_id: delivery.delivery_id.to_string(),
            operation_key: operation_key.clone(),
            capability: "notifications".to_string(),
            operation: "deliver".to_string(),
            execution_mode: OperationExecutionMode::Managed,
            metadata: started_metadata,
          }),
        )
        .map_err(|error| LifecycleFailure {
          kind: LifecycleFailureKind::ProviderFailed,
          code: "WOML_NOTIFICATION_EVENT_FAILED".to_string(),
          message: error.to_string(),
        })?;
      let request = InformationalNotificationDeliverMessage {
        protocol: NOTIFICATION_PROVIDER_PROTOCOL,
        protocol_version: INFORMATIONAL_NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
        message_type: "deliver",
        mode: "informational",
        invocation_id: invocation_id.clone(),
        run_id: run_id.to_string(),
        hook_invocation_id: hook_invocation_id.to_string(),
        action_id: action.action_id.clone(),
        delivery_id: delivery.delivery_id.to_string(),
        provider: delivery.provider.to_string(),
        destination: delivery.destination.to_string(),
        idempotency_key: operation_key.clone(),
        credentials: credentials.clone(),
        message: message.clone(),
      };
      match host.invoke(&invocation_id, &request).await {
        Ok(completed) => match completed.outcome {
          NotificationHostOutcome::DeliverySuccess { provider_message } => {
            let encoded = serde_json::to_vec(&provider_message).unwrap_or_default();
            let mut metadata = Map::new();
            metadata.insert(
              "provider".to_string(),
              Value::String(delivery.provider.to_string()),
            );
            metadata.insert(
              "destination".to_string(),
              Value::String(delivery.destination.to_string()),
            );
            metadata.insert("providerAttempt".to_string(), Value::from(provider_attempt));
            metadata.insert(
              "workspaceId".to_string(),
              Value::String(provider_message.workspace_id),
            );
            metadata.insert(
              "channelId".to_string(),
              Value::String(provider_message.channel_id),
            );
            metadata.insert(
              "providerMessageId".to_string(),
              Value::String(provider_message.message_id),
            );
            engine
              .append_payload(
                run_id,
                RunEventPayload::OperationSucceeded(OperationSucceededData {
                  node_id: action.action_id.clone(),
                  attempt_number: 1,
                  invocation_id,
                  call_id: delivery.delivery_id.to_string(),
                  operation_key: operation_key.clone(),
                  capability: "notifications".to_string(),
                  operation: "deliver".to_string(),
                  execution_mode: OperationExecutionMode::Managed,
                  metadata,
                  duration_ms: completed.duration_ms,
                  result_bytes: encoded.len() as u64,
                  result_digest: format!("sha256:{}", hex::encode(Sha256::digest(&encoded))),
                }),
              )
              .map_err(|error| LifecycleFailure {
                kind: LifecycleFailureKind::ProviderFailed,
                code: "WOML_NOTIFICATION_EVENT_FAILED".to_string(),
                message: error.to_string(),
              })?;
            delivered = true;
            break;
          }
          NotificationHostOutcome::Failure { error } => {
            let retry = error.retryable && provider_attempt < 3;
            let capability_failure = notification_capability_failure(
              if error.kind == "delivery_ambiguous" {
                CapabilityFailureKind::Ambiguous
              } else {
                CapabilityFailureKind::TransportFailed
              },
              error.code.clone(),
              error.message.clone(),
              error.retryable,
              error.kind == "delivery_ambiguous",
            );
            engine
              .append_payload(
                run_id,
                RunEventPayload::OperationFailed(OperationFailedData {
                  node_id: action.action_id.clone(),
                  attempt_number: 1,
                  invocation_id,
                  call_id: delivery.delivery_id.to_string(),
                  operation_key: operation_key.clone(),
                  capability: "notifications".to_string(),
                  operation: "deliver".to_string(),
                  execution_mode: OperationExecutionMode::Managed,
                  metadata: Map::new(),
                  duration_ms: completed.duration_ms,
                  failure: capability_failure,
                }),
              )
              .map_err(|event_error| LifecycleFailure {
                kind: LifecycleFailureKind::ProviderFailed,
                code: "WOML_NOTIFICATION_EVENT_FAILED".to_string(),
                message: event_error.to_string(),
              })?;
            if retry {
              if let Some(delay) = error.retry_after_ms {
                tokio::time::sleep(Duration::from_millis(delay)).await;
              }
              continue;
            }
            failures.push(LifecycleFailure {
              kind: LifecycleFailureKind::ProviderFailed,
              code: error.code,
              message: error.message,
            });
            break;
          }
          NotificationHostOutcome::UpdateSuccess => {
            failures.push(LifecycleFailure {
              kind: LifecycleFailureKind::ProviderFailed,
              code: "WOML_NOTIFICATION_PROTOCOL_INVALID".to_string(),
              message: "Informational notification delivery returned an update result.".to_string(),
            });
            break;
          }
        },
        Err(error) => {
          let capability_failure = notification_capability_failure(
            match error {
              NotificationHostClientError::HostCrashed(_)
              | NotificationHostClientError::Startup(_) => CapabilityFailureKind::HostCrashed,
              NotificationHostClientError::Protocol(_)
              | NotificationHostClientError::InteractionTimedOut => {
                CapabilityFailureKind::TransportFailed
              }
            },
            "WOML_NOTIFICATION_HOST_CRASHED".to_string(),
            error.to_string(),
            false,
            true,
          );
          engine
            .append_payload(
              run_id,
              RunEventPayload::OperationFailed(OperationFailedData {
                node_id: action.action_id.clone(),
                attempt_number: 1,
                invocation_id,
                call_id: delivery.delivery_id.to_string(),
                operation_key: operation_key.clone(),
                capability: "notifications".to_string(),
                operation: "deliver".to_string(),
                execution_mode: OperationExecutionMode::Managed,
                metadata: Map::new(),
                duration_ms: 0.0,
                failure: capability_failure,
              }),
            )
            .map_err(|event_error| LifecycleFailure {
              kind: LifecycleFailureKind::ProviderFailed,
              code: "WOML_NOTIFICATION_EVENT_FAILED".to_string(),
              message: event_error.to_string(),
            })?;
          failures.push(LifecycleFailure {
            kind: LifecycleFailureKind::HostCrashed,
            code: "WOML_NOTIFICATION_HOST_CRASHED".to_string(),
            message: error.to_string(),
          });
          break;
        }
      }
    }
    if !delivered && failures.is_empty() {
      failures.push(LifecycleFailure {
        kind: LifecycleFailureKind::ProviderFailed,
        code: "WOML_NOTIFICATION_DELIVERY_FAILED".to_string(),
        message: "Slack lifecycle notification delivery failed.".to_string(),
      });
    }
  }
  host.shutdown().await;
  if failures.is_empty() {
    Ok(())
  } else {
    let failed = failures.len();
    let first = failures.remove(0);
    Err(LifecycleFailure {
      kind: first.kind,
      code: first.code,
      message: if failed == 1 {
        first.message
      } else {
        format!(
          "{failed} Slack lifecycle deliveries failed. First failure: {}",
          first.message
        )
      },
    })
  }
}

async fn drive_workflow_lifecycle<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  options: &RuntimeExecutionOptions,
  host: &mut Option<ScriptHostClient>,
) -> Result<(), RuntimeExecutionError> {
  if !matches!(
    engine.workflow().schema_version,
    crate::COMPILED_MODEL_SCHEMA_VERSION_V11
      | crate::COMPILED_MODEL_SCHEMA_VERSION_V12
      | crate::COMPILED_MODEL_SCHEMA_VERSION_V13
  ) {
    return Ok(());
  }
  loop {
    let projection = engine.projection(run_id)?;
    let events = engine.events(run_id)?;
    let pending = pending_lifecycle_hook(&projection, &events);
    let Some(hook_projection) = pending else {
      if projection.status == RunStatus::Finalizing {
        let outcome = projection.business_outcome.ok_or_else(|| {
          RuntimeExecutionError::Stalled(
            "finalizing workflow has no durable business outcome".to_string(),
          )
        })?;
        let warnings = crate::durable::lifecycle_warnings_from_projection(&projection);
        let final_hook = projection
          .lifecycle_hooks
          .values()
          .find(|hook| hook.event == LifecycleEventName::RunComplete)
          .or_else(|| {
            projection.lifecycle_hooks.values().find(|hook| {
              matches!(
                hook.event,
                LifecycleEventName::RunSuccess
                  | LifecycleEventName::RunFailure
                  | LifecycleEventName::RunCancel
              )
            })
          });
        let hook_id = final_hook.map_or("lifecycle:run_complete", |hook| hook.hook_id.as_str());
        let action_id = final_hook
          .and_then(|hook| hook.actions.keys().next())
          .map_or("lifecycle:run_complete:action:0", String::as_str);
        report_lifecycle(
          options,
          engine.workflow(),
          run_id,
          LifecycleProgressPhase::RunFinalizing,
          hook_id,
          action_id,
          None,
          None,
        );
        engine.append_payload(
          run_id,
          RunEventPayload::RunFinalized(RunFinalizedData {
            outcome,
            lifecycle_status: if warnings.is_empty() {
              FinalLifecycleStatus::Completed
            } else {
              FinalLifecycleStatus::CompletedWithWarnings
            },
            warnings,
          }),
        )?;
        report_lifecycle(
          options,
          engine.workflow(),
          run_id,
          LifecycleProgressPhase::RunFinalized,
          hook_id,
          action_id,
          None,
          None,
        );
      }
      return Ok(());
    };
    if hook_projection.event == LifecycleEventName::RunCancel
      && projection.business_outcome.is_none()
    {
      // The cancellation request admits this hook durably, but it must not run
      // until all pre-outcome work has settled and cancellation is the durable
      // business outcome.
      return Ok(());
    }
    let hook = engine
      .workflow()
      .lifecycle_hook(&hook_projection.hook_id)
      .cloned()
      .ok_or_else(|| {
        RuntimeExecutionError::Stalled(
          "durable lifecycle hook is not in the definition".to_string(),
        )
      })?;
    let progress_action_id = hook.actions.first().map_or_else(
      || format!("{}:action:0", hook.hook_id),
      |action| action.action_id.clone(),
    );
    if hook_projection.status == LifecycleHookStatus::Requested {
      report_lifecycle(
        options,
        engine.workflow(),
        run_id,
        LifecycleProgressPhase::HookRequested,
        &hook.hook_id,
        &progress_action_id,
        matches!(hook_projection.subject.kind, LifecycleSubjectKind::Step)
          .then_some(hook_projection.subject.id.as_str()),
        None,
      );
    }
    if hook_projection
      .actions
      .values()
      .any(|action| action.status == LifecycleActionStatus::Started)
    {
      return Err(RuntimeExecutionError::Stalled(
        "an interrupted lifecycle action must be recovered before resume".to_string(),
      ));
    }
    let next_action = hook
      .actions
      .iter()
      .find(|action| !hook_projection.actions.contains_key(&action.action_id))
      .cloned();
    let Some(action) = next_action else {
      let failed_actions = hook_projection.failed_actions;
      let mut payloads = vec![RunEventPayload::LifecycleHookCompleted(
        LifecycleHookCompletedData {
          hook_invocation_id: hook_projection.hook_invocation_id.clone(),
          status: if failed_actions == 0 {
            LifecycleHookCompletionStatus::Completed
          } else {
            LifecycleHookCompletionStatus::CompletedWithWarnings
          },
          failed_actions,
        },
      )];
      if matches!(
        hook.event,
        LifecycleEventName::RunSuccess
          | LifecycleEventName::RunFailure
          | LifecycleEventName::RunCancel
      ) {
        if let Some(request) =
          workflow_lifecycle_request(engine.workflow(), run_id, LifecycleEventName::RunComplete)
        {
          payloads.push(request);
        }
      } else if matches!(
        hook.event,
        LifecycleEventName::StepSuccess | LifecycleEventName::StepFailure
      ) {
        if let Some(request) = step_lifecycle_request(
          engine.workflow(),
          run_id,
          LifecycleEventName::StepComplete,
          &hook_projection.subject.id,
        ) {
          payloads.push(request);
        }
      }
      engine.append_payloads(run_id, payloads)?;
      report_lifecycle(
        options,
        engine.workflow(),
        run_id,
        LifecycleProgressPhase::HookCompleted,
        &hook.hook_id,
        &progress_action_id,
        matches!(hook_projection.subject.kind, LifecycleSubjectKind::Step)
          .then_some(hook_projection.subject.id.as_str()),
        None,
      );
      continue;
    };
    if projection.cancellation_request_id.is_some()
      && matches!(
        hook.event,
        LifecycleEventName::RunStart | LifecycleEventName::StepStart
      )
    {
      let attempt_number = 1;
      engine.append_payloads(
        run_id,
        vec![
          RunEventPayload::LifecycleActionAttemptStarted(LifecycleActionIdentityData {
            hook_invocation_id: hook_projection.hook_invocation_id.clone(),
            action_id: action.action_id.clone(),
            attempt: attempt_number,
          }),
          RunEventPayload::LifecycleActionFailed(LifecycleActionFailedData {
            hook_invocation_id: hook_projection.hook_invocation_id.clone(),
            action_id: action.action_id.clone(),
            attempt: attempt_number,
            failure: LifecycleFailure {
              kind: LifecycleFailureKind::Cancelled,
              code: "WOML_LIFECYCLE_ACTION_CANCELLED".to_string(),
              message: "The pending pre-outcome lifecycle action was cancelled without execution."
                .to_string(),
            },
          }),
        ],
      )?;
      report_lifecycle(
        options,
        engine.workflow(),
        run_id,
        LifecycleProgressPhase::ActionFailed,
        &hook.hook_id,
        &action.action_id,
        matches!(hook_projection.subject.kind, LifecycleSubjectKind::Step)
          .then_some(hook_projection.subject.id.as_str()),
        Some("WOML_LIFECYCLE_ACTION_CANCELLED".to_string()),
      );
      continue;
    }
    if !matches!(
      action.handler.as_str(),
      "runtime.lifecycle-script" | "notification.informational"
    ) {
      return Err(RuntimeExecutionError::InvalidConfiguration(format!(
        "lifecycle action {:?} uses an unsupported runtime handler",
        action.action_id
      )));
    }
    let attempt_number = 1;
    let invocation_id = generated_id("lcinv");
    let idempotency_key =
      step_effect_idempotency_key(run_id, engine.definition_hash(), &action.action_id);
    engine.append_payload(
      run_id,
      RunEventPayload::LifecycleActionAttemptStarted(LifecycleActionIdentityData {
        hook_invocation_id: hook_projection.hook_invocation_id.clone(),
        action_id: action.action_id.clone(),
        attempt: attempt_number,
      }),
    )?;
    report_lifecycle(
      options,
      engine.workflow(),
      run_id,
      LifecycleProgressPhase::ActionStarted,
      &hook.hook_id,
      &action.action_id,
      matches!(hook_projection.subject.kind, LifecycleSubjectKind::Step)
        .then_some(hook_projection.subject.id.as_str()),
      None,
    );
    let snapshot = lifecycle_request_snapshot(&events, &hook_projection.hook_invocation_id)?;
    let context = snapshot.context.clone();
    let step = matches!(hook_projection.subject.kind, LifecycleSubjectKind::Step).then(|| {
      let step_id = hook_projection.subject.id.clone();
      LifecycleStepBindingV1 {
        id: step_id.clone(),
        outcome: step_lifecycle_outcome(hook.event, &snapshot, &step_id),
        attempts: snapshot
          .latest_attempt(&step_id)
          .map_or(0, |attempt| attempt.identity.attempt),
      }
    });
    let step_failure = step.as_ref().and_then(|step| {
      snapshot.latest_attempt(&step.id).and_then(|attempt| {
        let AttemptStatus::Failed { failure } = &attempt.status else {
          return None;
        };
        Some(LifecycleFailureBindingV1 {
          code: failure.code.clone(),
          message: failure.message.clone(),
        })
      })
    });
    let binding = LifecycleBindingV1 {
      event: hook.event,
      workflow: LifecycleWorkflowBindingV1 {
        id: engine.workflow().workflow_id.clone(),
        outcome: snapshot.business_outcome,
      },
      step,
      failure: step_failure.or_else(|| {
        snapshot
          .lifecycle_failure
          .as_ref()
          .map(|failure| LifecycleFailureBindingV1 {
            code: failure.code.clone(),
            message: failure.message.clone(),
          })
      }),
    };
    if action.handler == "notification.informational" {
      let result = execute_lifecycle_notification(
        engine,
        run_id,
        &action,
        &hook_projection.hook_invocation_id,
        &context,
        &binding,
        options,
      )
      .await;
      let (payload, phase, failure_code) = match result {
        Ok(()) => (
          RunEventPayload::LifecycleActionSucceeded(LifecycleActionIdentityData {
            hook_invocation_id: hook_projection.hook_invocation_id.clone(),
            action_id: action.action_id.clone(),
            attempt: attempt_number,
          }),
          LifecycleProgressPhase::ActionSucceeded,
          None,
        ),
        Err(failure) => {
          let code = failure.code.clone();
          (
            RunEventPayload::LifecycleActionFailed(LifecycleActionFailedData {
              hook_invocation_id: hook_projection.hook_invocation_id.clone(),
              action_id: action.action_id.clone(),
              attempt: attempt_number,
              failure,
            }),
            LifecycleProgressPhase::ActionFailed,
            Some(code),
          )
        }
      };
      engine.append_payload(run_id, payload)?;
      report_lifecycle(
        options,
        engine.workflow(),
        run_id,
        phase,
        &hook.hook_id,
        &action.action_id,
        matches!(hook_projection.subject.kind, LifecycleSubjectKind::Step)
          .then_some(hook_projection.subject.id.as_str()),
        failure_code,
      );
      continue;
    }
    let source = lifecycle_action_source(&action).ok_or_else(|| {
      RuntimeExecutionError::InvalidConfiguration(format!(
        "lifecycle action {:?} has no script source",
        action.action_id
      ))
    })?;
    let secrets = resolved_lifecycle_secrets(&action, options)?;
    if host.is_none() {
      match ScriptHostClient::spawn_with_authority(
        options.script_host.clone(),
        options.capability_authority.clone(),
      )
      .await
      {
        Ok(client) => *host = Some(client),
        Err(error) => {
          engine.append_payload(
            run_id,
            RunEventPayload::LifecycleActionFailed(LifecycleActionFailedData {
              hook_invocation_id: hook_projection.hook_invocation_id.clone(),
              action_id: action.action_id.clone(),
              attempt: attempt_number,
              failure: LifecycleFailure {
                kind: LifecycleFailureKind::HostCrashed,
                code: "WOML_SCRIPT_HOST_CRASHED".to_string(),
                message: error.to_string(),
              },
            }),
          )?;
          report_lifecycle(
            options,
            engine.workflow(),
            run_id,
            LifecycleProgressPhase::ActionFailed,
            &hook.hook_id,
            &progress_action_id,
            matches!(hook_projection.subject.kind, LifecycleSubjectKind::Step)
              .then_some(hook_projection.subject.id.as_str()),
            Some("WOML_SCRIPT_HOST_CRASHED".to_string()),
          );
          continue;
        }
      }
    }
    let modules = options
      .runtime_modules
      .iter()
      .map(|module| RuntimeModuleBinding {
        name: module.name.clone(),
        bundle_digest: module.bundle_digest.clone(),
        exports: module.exports.clone(),
      })
      .collect::<Vec<_>>();
    let request = ExecuteMessage::lifecycle_script_with_modules(
      &invocation_id,
      run_id,
      &action.action_id,
      ScriptAttempt::new(attempt_number, 1, &idempotency_key)
        .map_err(RuntimeExecutionError::Stalled)?,
      options.script_timeout_ms,
      source,
      &context,
      &binding,
      &secrets,
      &modules,
    );
    let host_client = host.as_ref().expect("script host was initialized");
    let mut execution = Box::pin(host_client.execute(&request));
    let mut cancellation_sent = false;
    let result = loop {
      tokio::select! {
        result = &mut execution => break result,
        _ = tokio::time::sleep(CANCELLATION_POLL_INTERVAL), if !cancellation_sent => {
          let current = engine.projection(run_id)?;
          let pre_outcome_action = current.business_outcome.is_none()
            && hook.event != LifecycleEventName::RunCancel;
          if current.status == RunStatus::Cancelling && pre_outcome_action {
            cancellation_sent = true;
            if let Err(error) = host_client.cancel_run(&invocation_id).await {
              break Err(error);
            }
          }
        }
      }
    };
    drop(execution);
    let (payload, phase, failure_code) = match result {
      _ if cancellation_sent => (
        RunEventPayload::LifecycleActionFailed(LifecycleActionFailedData {
          hook_invocation_id: hook_projection.hook_invocation_id.clone(),
          action_id: action.action_id.clone(),
          attempt: attempt_number,
          failure: LifecycleFailure {
            kind: LifecycleFailureKind::Cancelled,
            code: "WOML_LIFECYCLE_ACTION_CANCELLED".to_string(),
            message: "The lifecycle action was cancelled with its workflow run.".to_string(),
          },
        }),
        LifecycleProgressPhase::ActionFailed,
        Some("WOML_LIFECYCLE_ACTION_CANCELLED".to_string()),
      ),
      Ok(completed) => match completed.outcome {
        HostOutcome::Success { .. } => (
          RunEventPayload::LifecycleActionSucceeded(LifecycleActionIdentityData {
            hook_invocation_id: hook_projection.hook_invocation_id.clone(),
            action_id: action.action_id.clone(),
            attempt: attempt_number,
          }),
          LifecycleProgressPhase::ActionSucceeded,
          None,
        ),
        HostOutcome::Failure { error } => {
          let failure = lifecycle_failure_from_attempt(error.into_attempt_failure());
          let code = failure.code.clone();
          (
            RunEventPayload::LifecycleActionFailed(LifecycleActionFailedData {
              hook_invocation_id: hook_projection.hook_invocation_id.clone(),
              action_id: action.action_id.clone(),
              attempt: attempt_number,
              failure,
            }),
            LifecycleProgressPhase::ActionFailed,
            Some(code),
          )
        }
      },
      Err(error) => {
        *host = None;
        (
          RunEventPayload::LifecycleActionFailed(LifecycleActionFailedData {
            hook_invocation_id: hook_projection.hook_invocation_id.clone(),
            action_id: action.action_id.clone(),
            attempt: attempt_number,
            failure: LifecycleFailure {
              kind: LifecycleFailureKind::HostCrashed,
              code: "WOML_SCRIPT_HOST_CRASHED".to_string(),
              message: error.to_string(),
            },
          }),
          LifecycleProgressPhase::ActionFailed,
          Some("WOML_SCRIPT_HOST_CRASHED".to_string()),
        )
      }
    };
    engine.append_payload(run_id, payload)?;
    report_lifecycle(
      options,
      engine.workflow(),
      run_id,
      phase,
      &hook.hook_id,
      &progress_action_id,
      matches!(hook_projection.subject.kind, LifecycleSubjectKind::Step)
        .then_some(hook_projection.subject.id.as_str()),
      failure_code,
    );
  }
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
    settle_workflow_timeout_if_due(engine, run_id, options.clock.now(), options)?;
    let current = engine.projection(run_id)?;
    if current.timeout_reached_at.is_some() {
      return Err(resumed_failure(engine, run_id, current)?);
    }
    if current.status == RunStatus::Cancelling {
      options.ensure_policy_execution_slot().await?;
      settle_cancelling_run(engine, run_id, options.clock.now())?;
      return Err(cancelled_run_error(engine, run_id)?);
    }
    if current.status == RunStatus::Cancelled {
      return Err(cancelled_run_error(engine, run_id)?);
    }
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
        options.suspend_policy_execution_slot().await?;
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
          tokio::time::sleep(wait.min(CANCELLATION_POLL_INTERVAL)).await;
        }
        // The retry may become due between the ready-node query above and this
        // clock read. Recompute readiness instead of reporting a false stall.
        continue;
      }
      return Err(RuntimeExecutionError::Stalled(
        "no node is ready before the run reached a terminal state".to_string(),
      ));
    };
    options.ensure_policy_execution_slot().await?;
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
    if let Some(fork) = ready_fork(engine.workflow(), &current, &ready) {
      if host.is_none() {
        *host = Some(
          ScriptHostClient::spawn_with_authority(
            options.script_host.clone(),
            options.capability_authority.clone(),
          )
          .await?,
        );
      }
      let fork_progress = execute_fork(
        engine,
        run_id,
        fork,
        options,
        host.as_ref().expect("script host was initialized"),
      )
      .await?;
      match fork_progress {
        ForkRuntimeProgress::Continued(completed) => execution_order.extend(completed),
        ForkRuntimeProgress::Waiting { completed, outcome } => {
          execution_order.extend(completed);
          return Ok(outcome);
        }
      }
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
      "engine.fork-open" => {
        let fork = engine
          .workflow()
          .graph
          .forks
          .as_deref()
          .unwrap_or_default()
          .iter()
          .find(|fork| fork.open_node_id == *node_id)
          .ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "fork open node {node_id:?} has no Model v13 descriptor"
            ))
          })?;
        engine.append_payload(
          run_id,
          RunEventPayload::ForkOpened(ForkOpenedData {
            fork_id: fork.fork_id.clone(),
          }),
        )?;
      }
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
      "engine.choice-select" => {
        let context = engine.projection(run_id)?.context;
        let (choice_id, arm_id) = selected_choice_arm(engine.workflow(), node_id, &context)
          .map_err(|error| {
            RuntimeExecutionError::Stalled(format!(
              "choice {:?} could not select an arm: {:?}",
              error.branch_id, error.kind
            ))
          })?;
        engine.append_payload(
          run_id,
          RunEventPayload::ChoiceSelected(ChoiceSelectedData { choice_id, arm_id }),
        )?;
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
        // Model v13 workflows must pass through the explicit settlement node.
        // The last value-producing main-route step is the public result source,
        // but it is not permission to publish success while owned fork work may
        // still have failed or remain unsettled.
        if node_id == &terminal_node_id && engine.workflow().graph.settlement.is_none() {
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
        return request_approval_wait(engine, run_id, node_id, options);
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
        // In Model v13 this is only the main route's value source. The
        // workflow-settlement node decides whether all owned fork work permits
        // that value to become a public success result.
        if node_id == &terminal_node_id && engine.workflow().graph.settlement.is_none() {
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
      "engine.workflow-settlement" => {
        let projection = engine.projection(run_id)?;
        if let Some(failure) = final_workflow_attempt_failure(&projection) {
          engine.append_payload(
            run_id,
            RunEventPayload::RunFailed(attempt_run_failed_data(
              engine.event_schema_version(),
              &failure,
            )),
          )?;
          return Err(resumed_failure(engine, run_id, engine.projection(run_id)?)?);
        }
        let settlement = engine.workflow().graph.settlement.as_ref().ok_or_else(|| {
          RuntimeExecutionError::Stalled(
            "workflow settlement node has no Model v13 descriptor".to_string(),
          )
        })?;
        if settlement.node_id != *node_id {
          return Err(RuntimeExecutionError::Stalled(format!(
            "workflow settlement node {node_id:?} does not match Model v13"
          )));
        }
        let result = engine
          .projection(run_id)?
          .context
          .steps
          .get(&settlement.main_result_node_id)
          .cloned()
          .ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "workflow settlement has no public main result from {:?}",
              settlement.main_result_node_id
            ))
          })?;
        engine.append_payload(
          run_id,
          RunEventPayload::RunSucceeded(RunSucceededData {
            terminal_node_id: settlement.main_result_node_id.clone(),
            result,
          }),
        )?;
        return Ok(WorkflowRuntimeOutcome::succeeded(final_result(
          engine,
          run_id,
          execution_order.clone(),
        )?));
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

fn request_approval_wait<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  node_id: &str,
  options: &RuntimeExecutionOptions,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
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
  let workflow_deadline = engine.projection(run_id)?.timeout_at;
  let waiting_expires_at = match (expires_at, workflow_deadline) {
    (Some(approval), Some(workflow)) => Some(approval.min(workflow)),
    (Some(approval), None) => Some(approval),
    (None, Some(workflow)) => Some(workflow),
    (None, None) => None,
  };
  let waiting_on_timeout = if workflow_deadline
    .is_some_and(|workflow| expires_at.is_none_or(|approval| workflow <= approval))
  {
    ApprovalTimeoutPolicy::Fail
  } else {
    on_timeout
  };
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
  Ok(waiting_outcome(
    engine.workflow().workflow_id.clone(),
    run_id.to_string(),
    approval,
    request_id,
    waiting_expires_at,
    waiting_on_timeout,
    token,
  ))
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

fn fork_branch_routes(
  workflow: &CompiledWorkflowDefinition,
  fork: &crate::model::CompiledFork,
) -> BTreeMap<String, HashSet<String>> {
  fork
    .branches
    .iter()
    .map(|branch| {
      let mut route = HashSet::new();
      let mut pending = VecDeque::from([branch.entry_node_id.as_str()]);
      while let Some(node_id) = pending.pop_front() {
        if !route.insert(node_id.to_string()) || node_id == branch.terminal_node_id {
          continue;
        }
        pending.extend(
          workflow
            .graph
            .edges
            .iter()
            .filter(|edge| edge.from == node_id)
            .map(|edge| edge.to.as_str()),
        );
      }
      (branch.branch_id.clone(), route)
    })
    .collect()
}

fn ready_fork(
  workflow: &CompiledWorkflowDefinition,
  projection: &RunProjection,
  ready: &[String],
) -> Option<crate::model::CompiledFork> {
  workflow
    .graph
    .forks
    .as_deref()
    .unwrap_or_default()
    .iter()
    .find(|fork| {
      projection
        .forks
        .get(&fork.fork_id)
        .is_some_and(|state| state.join_status == crate::ForkJoinStatus::Pending)
        && {
          let routes = fork_branch_routes(workflow, fork);
          ready.iter().any(|node_id| {
            routes.values().any(|route| route.contains(node_id)) || node_id == &fork.join_node_id
          })
        }
    })
    .cloned()
}

fn runtime_context_for_node(
  workflow: &CompiledWorkflowDefinition,
  projection: &RunProjection,
  node_id: &str,
) -> WorkflowContext {
  let mut context = projection.context.clone();
  if let Some(visibility) = workflow
    .graph
    .context_visibility
    .as_deref()
    .unwrap_or_default()
    .iter()
    .find(|visibility| visibility.node_id == node_id)
  {
    context
      .steps
      .retain(|step_id, _| visibility.step_ids.contains(step_id));
  }
  context
}

fn final_attempt_failure_in_nodes(
  projection: &RunProjection,
  node_ids: &HashSet<String>,
) -> Option<StepAttemptFailedData> {
  projection.attempts.iter().rev().find_map(|attempt| {
    if !node_ids.contains(&attempt.identity.node_id)
      || projection
        .context
        .steps
        .contains_key(&attempt.identity.node_id)
      || projection
        .pending_retries
        .contains_key(&attempt.identity.node_id)
    {
      return None;
    }
    // Only the last attempt for a node can be a final failure. Looking at the
    // latest projection directly also avoids treating an earlier failed retry
    // as fatal after a later attempt succeeded.
    let latest = projection.latest_attempt(&attempt.identity.node_id)?;
    if latest.identity != attempt.identity {
      return None;
    }
    let AttemptStatus::Failed { failure } = &attempt.status else {
      return None;
    };
    Some(StepAttemptFailedData {
      node_id: attempt.identity.node_id.clone(),
      attempt: attempt.identity.attempt,
      invocation_id: attempt.identity.invocation_id.clone(),
      failure: failure.clone(),
    })
  })
}

fn final_workflow_attempt_failure(projection: &RunProjection) -> Option<StepAttemptFailedData> {
  let node_ids = projection
    .attempts
    .iter()
    .map(|attempt| attempt.identity.node_id.clone())
    .collect::<HashSet<_>>();
  final_attempt_failure_in_nodes(projection, &node_ids)
}

fn settle_open_fork_cancellation<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  fork: &crate::model::CompiledFork,
) -> Result<(), RuntimeExecutionError> {
  let mut projection = engine.projection(run_id)?;
  for branch in &fork.branches {
    if projection
      .forks
      .get(&fork.fork_id)
      .is_some_and(|state| state.branches.contains_key(&branch.branch_id))
    {
      continue;
    }
    engine.append_payload(
      run_id,
      RunEventPayload::ForkBranchSettled(ForkBranchSettledData {
        fork_id: fork.fork_id.clone(),
        branch_id: branch.branch_id.clone(),
        terminal_node_id: branch.terminal_node_id.clone(),
        outcome: ForkBranchOutcome::Cancelled,
      }),
    )?;
    projection = engine.projection(run_id)?;
  }

  if projection
    .forks
    .get(&fork.fork_id)
    .is_some_and(|state| state.join_status == crate::ForkJoinStatus::Pending)
  {
    let join = if let Some(blocking_branch_id) = fork.joined_branch_ids.first() {
      ForkJoinSettledData {
        fork_id: fork.fork_id.clone(),
        outcome: ForkJoinOutcome::Cancelled,
        blocking_branch_id: Some(blocking_branch_id.clone()),
      }
    } else {
      ForkJoinSettledData {
        fork_id: fork.fork_id.clone(),
        outcome: ForkJoinOutcome::Succeeded,
        blocking_branch_id: None,
      }
    };
    engine.append_payload(run_id, RunEventPayload::ForkJoinSettled(join))?;
  }
  Ok(())
}

enum ForkRuntimeProgress {
  Continued(Vec<String>),
  Waiting {
    completed: Vec<String>,
    outcome: WorkflowRuntimeOutcome,
  },
}

async fn execute_fork<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  fork: crate::model::CompiledFork,
  options: &RuntimeExecutionOptions,
  host: &ScriptHostClient,
) -> Result<ForkRuntimeProgress, RuntimeExecutionError> {
  let routes = fork_branch_routes(engine.workflow(), &fork);
  let mut active: Vec<ActiveParallelInvocation<'_>> = Vec::new();
  let mut completion_order = Vec::new();
  let mut cancellation_sent = false;

  loop {
    settle_workflow_timeout_if_due(engine, run_id, options.clock.now(), options)?;
    let mut projection = engine.projection(run_id)?;
    if projection.timeout_reached_at.is_some() {
      let invocation_ids = active
        .iter()
        .map(|invocation| invocation.invocation_id.clone())
        .collect::<Vec<_>>();
      for invocation_id in invocation_ids {
        let _ = host.cancel_run(&invocation_id).await;
      }
      return Err(resumed_failure(engine, run_id, projection)?);
    }
    if projection.status == RunStatus::Cancelling && active.is_empty() {
      settle_open_fork_cancellation(engine, run_id, &fork)?;
      return Ok(ForkRuntimeProgress::Continued(completion_order));
    }
    if projection.status == RunStatus::Cancelling && !cancellation_sent {
      cancellation_sent = true;
      let invocation_ids = active
        .iter()
        .map(|invocation| invocation.invocation_id.clone())
        .collect::<Vec<_>>();
      for invocation_id in invocation_ids {
        let _ = host.cancel_run(&invocation_id).await;
      }
    }
    let ready = engine.ready_node_ids(run_id)?;

    for branch in &fork.branches {
      if ready.contains(&branch.terminal_node_id)
        && projection
          .forks
          .get(&fork.fork_id)
          .is_some_and(|state| !state.branches.contains_key(&branch.branch_id))
      {
        engine.append_payload(
          run_id,
          RunEventPayload::ForkBranchSettled(ForkBranchSettledData {
            fork_id: fork.fork_id.clone(),
            branch_id: branch.branch_id.clone(),
            terminal_node_id: branch.terminal_node_id.clone(),
            outcome: ForkBranchOutcome::Succeeded,
          }),
        )?;
      }
    }

    projection = engine.projection(run_id)?;
    for branch in &fork.branches {
      let branch_is_settled = projection
        .forks
        .get(&fork.fork_id)
        .is_some_and(|state| state.branches.contains_key(&branch.branch_id));
      if branch_is_settled {
        continue;
      }
      let route = routes.get(&branch.branch_id).ok_or_else(|| {
        RuntimeExecutionError::Stalled(format!(
          "fork {:?} lost branch route {:?}",
          fork.fork_id, branch.branch_id
        ))
      })?;
      let Some(failure) = final_attempt_failure_in_nodes(&projection, route) else {
        continue;
      };
      let outcome = if failure.failure.kind == AttemptFailureKind::InvocationCancelled {
        ForkBranchOutcome::Cancelled
      } else {
        ForkBranchOutcome::Failed
      };
      engine.append_payload(
        run_id,
        RunEventPayload::ForkBranchSettled(ForkBranchSettledData {
          fork_id: fork.fork_id.clone(),
          branch_id: branch.branch_id.clone(),
          terminal_node_id: branch.terminal_node_id.clone(),
          outcome,
        }),
      )?;
    }

    projection = engine.projection(run_id)?;
    let join_is_pending = projection
      .forks
      .get(&fork.fork_id)
      .is_some_and(|state| state.join_status == crate::ForkJoinStatus::Pending);
    let joined_blocker = fork.joined_branch_ids.iter().find_map(|branch_id| {
      projection
        .forks
        .get(&fork.fork_id)
        .and_then(|state| state.branches.get(branch_id))
        .and_then(|branch| branch.outcome)
        .filter(|outcome| *outcome != ForkBranchOutcome::Succeeded)
        .map(|outcome| (branch_id.clone(), outcome))
    });
    let joined_branches_succeeded = fork.joined_branch_ids.iter().all(|branch_id| {
      projection
        .forks
        .get(&fork.fork_id)
        .and_then(|state| state.branches.get(branch_id))
        .and_then(|branch| branch.outcome)
        == Some(ForkBranchOutcome::Succeeded)
    });
    if join_is_pending {
      let join = if let Some((branch_id, outcome)) = joined_blocker {
        Some(ForkJoinSettledData {
          fork_id: fork.fork_id.clone(),
          outcome: if outcome == ForkBranchOutcome::Cancelled {
            ForkJoinOutcome::Cancelled
          } else {
            ForkJoinOutcome::Failed
          },
          blocking_branch_id: Some(branch_id),
        })
      } else if joined_branches_succeeded {
        Some(ForkJoinSettledData {
          fork_id: fork.fork_id.clone(),
          outcome: ForkJoinOutcome::Succeeded,
          blocking_branch_id: None,
        })
      } else {
        None
      };
      if let Some(join) = join {
        engine.append_payload(run_id, RunEventPayload::ForkJoinSettled(join))?;
      }
    }

    projection = engine.projection(run_id)?;
    let every_branch_settled = fork.branches.iter().all(|branch| {
      projection
        .forks
        .get(&fork.fork_id)
        .is_some_and(|state| state.branches.contains_key(&branch.branch_id))
    });
    if every_branch_settled && active.is_empty() {
      if projection.status == RunStatus::Cancelling {
        return Ok(ForkRuntimeProgress::Continued(completion_order));
      }
      if let Some(failure) = final_workflow_attempt_failure(&projection) {
        let join_failed = projection
          .forks
          .get(&fork.fork_id)
          .is_some_and(|state| state.join_status != crate::ForkJoinStatus::Succeeded);
        let main_result_recorded =
          engine
            .workflow()
            .graph
            .settlement
            .as_ref()
            .is_some_and(|settlement| {
              projection
                .context
                .steps
                .contains_key(&settlement.main_result_node_id)
            });
        if join_failed || main_result_recorded {
          engine.append_payload(
            run_id,
            RunEventPayload::RunFailed(attempt_run_failed_data(
              engine.event_schema_version(),
              &failure,
            )),
          )?;
          return Err(resumed_failure(engine, run_id, engine.projection(run_id)?)?);
        }
      }
      return Ok(ForkRuntimeProgress::Continued(completion_order));
    }

    let ready = engine.ready_node_ids(run_id)?;
    let mut pure_node_completed = false;
    for node_id in &ready {
      if !routes.values().any(|route| route.contains(node_id)) {
        continue;
      }
      let handler = engine
        .workflow()
        .node(node_id)
        .map(|node| node.handler.as_str())
        .ok_or_else(|| RuntimeExecutionError::Stalled("ready node disappeared".to_string()))?;
      match handler {
        "engine.approval-wait" if active.is_empty() => {
          let outcome = request_approval_wait(engine, run_id, node_id, options)?;
          return Ok(ForkRuntimeProgress::Waiting {
            completed: completion_order,
            outcome,
          });
        }
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
          pure_node_completed = true;
          break;
        }
        "engine.choice-select" => {
          let context = runtime_context_for_node(engine.workflow(), &projection, node_id);
          let (choice_id, arm_id) = selected_choice_arm(engine.workflow(), node_id, &context)
            .map_err(|error| {
              RuntimeExecutionError::Stalled(format!(
                "fork branch choice {:?} could not select an arm: {:?}",
                error.branch_id, error.kind
              ))
            })?;
          engine.append_payload(
            run_id,
            RunEventPayload::ChoiceSelected(ChoiceSelectedData { choice_id, arm_id }),
          )?;
          pure_node_completed = true;
          break;
        }
        "engine.branch-select" => {
          let arm_id = selected_branch_arm(engine.workflow(), node_id, &projection.context)
            .map_err(|error| {
              RuntimeExecutionError::Stalled(format!(
                "fork branch choice {:?} could not select an arm: {:?}",
                error.branch_id, error.kind
              ))
            })?;
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
          pure_node_completed = true;
          break;
        }
        "engine.branch-result" => {
          let arm_id = projection
            .branch_selections
            .get(node_id)
            .cloned()
            .ok_or_else(|| {
              RuntimeExecutionError::Stalled(format!(
                "branch result {node_id:?} became ready without a recorded selection"
              ))
            })?;
          let node = engine
            .workflow()
            .node(node_id)
            .ok_or_else(|| RuntimeExecutionError::Stalled("ready node disappeared".to_string()))?;
          let ValueExpression::Object { fields } = &node.inputs else {
            return Err(RuntimeExecutionError::Stalled(format!(
              "branch result {node_id:?} has invalid compiled inputs"
            )));
          };
          let expression = fields.get(&arm_id).ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "branch result {node_id:?} has no value for selected arm {arm_id:?}"
            ))
          })?;
          let output =
            resolve_context_reference(expression, &projection.context).map_err(|error| {
              RuntimeExecutionError::Stalled(format!(
                "branch result {node_id:?} cannot read {:?}",
                error.path
              ))
            })?;
          engine.publish_pure_result(run_id, node_id, output)?;
          completion_order.push(node_id.clone());
          pure_node_completed = true;
          break;
        }
        _ => {}
      }
    }
    if pure_node_completed {
      continue;
    }

    if active.is_empty() {
      if let Some(group) = ready.iter().find_map(|node_id| {
        let group = engine.workflow().parallel_group_for_child(node_id)?;
        routes
          .values()
          .any(|route| {
            group
              .child_node_ids
              .iter()
              .all(|child| route.contains(child))
          })
          .then_some(group)
      }) {
        let completed = execute_parallel_group(engine, run_id, group, options, host).await?;
        completion_order.extend(completed);
        continue;
      }
    }

    options.ensure_policy_execution_slot().await?;
    for node_id in &ready {
      if !engine
        .workflow()
        .node(node_id)
        .is_some_and(|node| node.handler == "runtime.script")
      {
        continue;
      }
      let branch_owner = engine
        .workflow()
        .fork_branch_owner(node_id)
        .map(|(owner_fork, branch)| (owner_fork.fork_id.clone(), branch.branch_id.clone()));
      match branch_owner.as_ref() {
        Some((fork_id, branch_id)) if fork_id == &fork.fork_id => {
          let route = routes.get(branch_id).ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "fork {:?} lost branch route {:?}",
              fork.fork_id, branch_id
            ))
          })?;
          let branch_is_settled = projection
            .forks
            .get(&fork.fork_id)
            .is_some_and(|state| state.branches.contains_key(branch_id));
          let branch_is_active = active
            .iter()
            .any(|invocation| route.contains(&invocation.node_id));
          if branch_is_settled || branch_is_active {
            continue;
          }
        }
        Some(_) => continue,
        None => {
          let main_is_active = active.iter().any(|invocation| {
            engine
              .workflow()
              .fork_branch_owner(&invocation.node_id)
              .is_none()
          });
          if main_is_active {
            continue;
          }
        }
      }
      let source = engine
        .workflow()
        .node(node_id)
        .and_then(|node| node.script_source())
        .ok_or_else(|| {
          RuntimeExecutionError::Stalled(format!("fork branch script {node_id:?} has no source"))
        })?
        .to_string();
      let secrets = resolved_script_secrets(engine.workflow(), node_id, options)?;
      let attempt_number = projection
        .pending_retries
        .get(node_id)
        .map_or(1, |retry| retry.next_attempt);
      let max_attempts = engine
        .workflow()
        .node(node_id)
        .and_then(|node| node.retry_policy.as_ref())
        .map_or(1, |policy| policy.max_attempts);
      let invocation_id = generated_id("inv");
      let idempotency_key = step_effect_idempotency_key(run_id, engine.definition_hash(), node_id);
      let context = runtime_context_for_node(engine.workflow(), &projection, node_id);
      engine.append_payload(
        run_id,
        RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
          node_id: node_id.clone(),
          attempt: attempt_number,
          invocation_id: invocation_id.clone(),
          handler: "runtime.script".to_string(),
          idempotency_key: Some(idempotency_key.clone()),
        }),
      )?;
      active.push(ActiveParallelInvocation {
        node_id: node_id.clone(),
        invocation_id: invocation_id.clone(),
        future: Box::pin(invoke_parallel_child(
          host,
          ParallelInvocationRequest {
            run_id: run_id.to_string(),
            node_id: node_id.clone(),
            invocation_id,
            source,
            context,
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

    if active.is_empty() {
      let projection = engine.projection(run_id)?;
      if let Some(scheduled_at) = projection
        .pending_retries
        .values()
        .map(|retry| retry.scheduled_at)
        .min()
      {
        options.suspend_policy_execution_slot().await?;
        let now = chrono::Utc::now();
        if scheduled_at > now {
          let wait = (scheduled_at - now).to_std().map_err(|_| {
            RuntimeExecutionError::Stalled(
              "fork retry schedule exceeds the runtime clock".to_string(),
            )
          })?;
          tokio::time::sleep(wait.min(CANCELLATION_POLL_INTERVAL)).await;
        }
        continue;
      }
    }

    let completion = loop {
      tokio::select! {
        completion = next_parallel_completion(&mut active) => break completion,
        _ = tokio::time::sleep(CANCELLATION_POLL_INTERVAL), if !cancellation_sent => {
          settle_workflow_timeout_if_due(engine, run_id, options.clock.now(), options)?;
          let current = engine.projection(run_id)?;
          if current.timeout_reached_at.is_some() {
            let invocation_ids = active
              .iter()
              .map(|invocation| invocation.invocation_id.clone())
              .collect::<Vec<_>>();
            for invocation_id in invocation_ids {
              let _ = host.cancel_run(&invocation_id).await;
            }
            return Err(resumed_failure(engine, run_id, current)?);
          }
          if current.status == RunStatus::Cancelling {
            cancellation_sent = true;
            let invocation_ids = active
              .iter()
              .map(|invocation| invocation.invocation_id.clone())
              .collect::<Vec<_>>();
            for invocation_id in invocation_ids {
              let _ = host.cancel_run(&invocation_id).await;
            }
          }
        }
      }
    };
    let Some(mut completion) = completion else {
      return Err(RuntimeExecutionError::Stalled(format!(
        "fork {:?} has unfinished owned work but no safe executable operation",
        fork.fork_id
      )));
    };
    if cancellation_sent || engine.projection(run_id)?.status == RunStatus::Cancelling {
      completion.outcome = Err(AttemptFailure {
        kind: AttemptFailureKind::InvocationCancelled,
        code: AttemptFailureKind::InvocationCancelled.code().to_string(),
        message: "The fork branch invocation was cancelled with its workflow run.".to_string(),
        details: None,
        ..AttemptFailure::legacy_defaults()
      });
    }
    match completion.outcome {
      Ok(output) => {
        completion_order.push(completion.node_id.clone());
        engine.append_payload(
          run_id,
          RunEventPayload::StepAttemptSucceeded(StepAttemptSucceededData {
            node_id: completion.node_id,
            attempt: completion.attempt_number,
            invocation_id: completion.invocation_id,
            output,
          }),
        )?;
      }
      Err(failure) => {
        let code = failure.code.clone();
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
          code,
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
          StepFailureDisposition::StepFailed => {}
          StepFailureDisposition::RunFailed => {
            return Err(RuntimeExecutionError::Stalled(
              "fork-coordinated work failed the run before all owned branches settled".to_string(),
            ));
          }
        }
      }
    }
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
  node_id: String,
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
  let mut run_cancellation_sent = false;
  let mut workflow_timeout_sent = false;

  loop {
    let current = engine.projection(run_id)?;
    let cancelling = current.status == RunStatus::Cancelling;
    let timed_out = current.timeout_reached_at.is_some();
    if timed_out && !workflow_timeout_sent {
      workflow_timeout_sent = true;
      let active_invocation_ids = active
        .iter()
        .map(|invocation| invocation.invocation_id.clone())
        .collect::<Vec<_>>();
      for invocation_id in active_invocation_ids {
        let _ = host.cancel_run(&invocation_id).await;
      }
    }
    if cancelling && !run_cancellation_sent {
      run_cancellation_sent = true;
      let active_invocation_ids = active
        .iter()
        .map(|invocation| invocation.invocation_id.clone())
        .collect::<Vec<_>>();
      for invocation_id in active_invocation_ids {
        let _ = host.cancel_run(&invocation_id).await;
      }
    }
    while !fail_fast_closed && !cancelling && !timed_out && active.len() < group.concurrency {
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
      let invocation_context =
        if engine.workflow().schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V13 {
          runtime_context_for_node(engine.workflow(), &projection, &node_id)
        } else {
          fork_context.clone()
        };
      active.push(ActiveParallelInvocation {
        node_id: node_id.clone(),
        invocation_id: invocation_id.clone(),
        future: Box::pin(invoke_parallel_child(
          host,
          ParallelInvocationRequest {
            run_id: run_id.to_string(),
            node_id,
            invocation_id,
            source,
            context: invocation_context,
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

    let completion = loop {
      tokio::select! {
        completion = next_parallel_completion(&mut active) => break completion,
        _ = tokio::time::sleep(CANCELLATION_POLL_INTERVAL), if !run_cancellation_sent && !workflow_timeout_sent => {
          settle_workflow_timeout_if_due(engine, run_id, options.clock.now(), options)?;
          let current = engine.projection(run_id)?;
          if current.timeout_reached_at.is_some() {
            workflow_timeout_sent = true;
            let active_invocation_ids = active
              .iter()
              .map(|invocation| invocation.invocation_id.clone())
              .collect::<Vec<_>>();
            for invocation_id in active_invocation_ids {
              let _ = host.cancel_run(&invocation_id).await;
            }
          } else if current.status == RunStatus::Cancelling {
            run_cancellation_sent = true;
            let active_invocation_ids = active
              .iter()
              .map(|invocation| invocation.invocation_id.clone())
              .collect::<Vec<_>>();
            for invocation_id in active_invocation_ids {
              let _ = host.cancel_run(&invocation_id).await;
            }
          }
        }
      }
    };
    settle_workflow_timeout_if_due(engine, run_id, options.clock.now(), options)?;
    if workflow_timeout_sent || engine.projection(run_id)?.timeout_reached_at.is_some() {
      drop(active);
      let projection = engine.projection(run_id)?;
      return Err(resumed_failure(engine, run_id, projection)?);
    }
    let Some(mut completion) = completion else {
      break;
    };
    if run_cancellation_sent || engine.projection(run_id)?.status == RunStatus::Cancelling {
      completion.outcome = Err(AttemptFailure {
        kind: AttemptFailureKind::InvocationCancelled,
        code: AttemptFailureKind::InvocationCancelled.code().to_string(),
        message: "The parallel step invocation was cancelled with its workflow run.".to_string(),
        details: None,
        ..AttemptFailure::legacy_defaults()
      });
    }
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
  if projection.status == RunStatus::Cancelling {
    return Ok(completion_order);
  }
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

  let projection = engine.projection(run_id)?;
  let context = runtime_context_for_node(engine.workflow(), &projection, node_id);
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
  let host_client = host.as_ref().expect("script host was initialized");
  let mut execution = Box::pin(host_client.execute(&request));
  let mut cancellation_sent = false;
  let mut timeout_sent = false;
  let completed = loop {
    tokio::select! {
      result = &mut execution => break result,
      _ = tokio::time::sleep(CANCELLATION_POLL_INTERVAL), if !cancellation_sent && !timeout_sent => {
        settle_workflow_timeout_if_due(engine, run_id, options.clock.now(), options)?;
        let current = engine.projection(run_id)?;
        if current.timeout_reached_at.is_some() {
          timeout_sent = true;
          if let Err(error) = host_client.cancel_run(&invocation_id).await {
            break Err(error);
          }
        } else if current.status == RunStatus::Cancelling {
          cancellation_sent = true;
          if let Err(error) = host_client.cancel_run(&invocation_id).await {
            break Err(error);
          }
        }
      }
    }
  };
  drop(execution);
  settle_workflow_timeout_if_due(engine, run_id, options.clock.now(), options)?;
  if timeout_sent || engine.projection(run_id)?.timeout_reached_at.is_some() {
    let projection = engine.projection(run_id)?;
    return Err(resumed_failure(engine, run_id, projection)?);
  }
  let outcome = match completed {
    _ if cancellation_sent || engine.projection(run_id)?.status == RunStatus::Cancelling => {
      return settle_script_attempt_failure(
        engine,
        options,
        run_id,
        node_id,
        attempt_number,
        &invocation_id,
        AttemptFailure {
          kind: AttemptFailureKind::InvocationCancelled,
          code: AttemptFailureKind::InvocationCancelled.code().to_string(),
          message: "The step invocation was cancelled with its workflow run.".to_string(),
          details: None,
          ..AttemptFailure::legacy_defaults()
        },
      );
    }
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
  Cancelled,
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
  if let Some(lifecycle_failure) = projection.lifecycle_failure.clone() {
    let kind = match lifecycle_failure.kind {
      crate::LifecycleFailureKind::TimedOut => AttemptFailureKind::ScriptTimedOut,
      crate::LifecycleFailureKind::NonJson => AttemptFailureKind::InvalidScriptResult,
      crate::LifecycleFailureKind::WorkerCrashed => AttemptFailureKind::WorkerCrashed,
      crate::LifecycleFailureKind::HostCrashed => AttemptFailureKind::HostCrashed,
      crate::LifecycleFailureKind::Interrupted => AttemptFailureKind::Interrupted,
      crate::LifecycleFailureKind::SizeLimitExceeded => AttemptFailureKind::ResultTooLarge,
      crate::LifecycleFailureKind::Cancelled => AttemptFailureKind::InvocationCancelled,
      crate::LifecycleFailureKind::ProviderFailed => AttemptFailureKind::ServiceFailed,
      crate::LifecycleFailureKind::ScriptThrew => AttemptFailureKind::ScriptThrew,
    };
    let failure = AttemptFailure {
      kind,
      code: lifecycle_failure.code.clone(),
      message: lifecycle_failure.message.clone(),
      details: None,
      ..AttemptFailure::legacy_defaults()
    };
    let identity = events.iter().rev().find_map(|event| match &event.payload {
      RunEventPayload::StepAttemptFailed(data)
        if data.failure.code == lifecycle_failure.code
          && data.failure.message == lifecycle_failure.message =>
      {
        Some((data.node_id.clone(), data.attempt))
      }
      _ => None,
    });
    let (node_id, attempt, max_attempts) = identity
      .map(|(node_id, attempt)| {
        let maximum = retry_max_attempts(engine, &node_id);
        (Some(node_id), Some(attempt), Some(maximum))
      })
      .unwrap_or((None, None, None));
    return Ok(RuntimeExecutionError::RunFailed(Box::new(
      FailedRunDetails {
        code: lifecycle_failure.code,
        message: lifecycle_failure.message,
        node_id,
        attempt,
        max_attempts,
        failure,
        events,
      },
    )));
  }
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
  let terminal_node_id = projection
    .terminal_node_id
    .or_else(|| engine.workflow().terminal_node_id().map(str::to_string))
    .ok_or_else(|| {
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

const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(25);

fn cancelled_run_error<E: RuntimeDagEngine>(
  engine: &E,
  run_id: &str,
) -> Result<RuntimeExecutionError, RuntimeExecutionError> {
  let projection = engine.projection(run_id)?;
  let cancellation_request_id = projection.cancellation_request_id.ok_or_else(|| {
    RuntimeExecutionError::Stalled("cancelled run has no durable cancellation request".to_string())
  })?;
  Ok(RuntimeExecutionError::RunCancelled(Box::new(
    CancelledRunDetails {
      code: "WOML_RUN_CANCELLED".to_string(),
      run_id: run_id.to_string(),
      cancellation_request_id,
      events: engine.events(run_id)?,
    },
  )))
}

fn settle_cancelling_run<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  occurred_at: chrono::DateTime<chrono::Utc>,
) -> Result<(), RuntimeExecutionError> {
  let projection = engine.projection(run_id)?;
  if projection.status != RunStatus::Cancelling {
    return Ok(());
  }

  let active_operations = projection
    .operations
    .values()
    .filter(|operation| operation.status == OperationStatus::Started)
    .cloned()
    .collect::<Vec<_>>();
  for operation in active_operations {
    engine.append_payload(
      run_id,
      RunEventPayload::OperationFailed(OperationFailedData {
        node_id: operation.node_id,
        attempt_number: operation.attempt_number,
        invocation_id: operation.identity.invocation_id,
        call_id: operation.identity.call_id,
        operation_key: operation.operation_key,
        capability: operation.capability,
        operation: operation.operation,
        execution_mode: operation.execution_mode,
        metadata: operation.metadata,
        duration_ms: 0.0,
        failure: CapabilityFailure {
          kind: CapabilityFailureKind::Ambiguous,
          code: "WOML_CAPABILITY_CANCELLATION_AMBIGUOUS".to_string(),
          message: "Cancellation found an operation without a durable terminal event; its external outcome is ambiguous and it will not be replayed.".to_string(),
          retryable: false,
          ambiguous: true,
          details: None,
        },
      }),
    )?;
  }

  let projection = engine.projection(run_id)?;
  let active_attempts = projection
    .attempts
    .iter()
    .filter(|attempt| attempt.status == AttemptStatus::Started)
    .map(|attempt| attempt.identity.clone())
    .collect::<Vec<_>>();
  for attempt in active_attempts {
    let disposition = engine.record_step_attempt_failure(
      run_id,
      occurred_at,
      StepAttemptFailedData {
        node_id: attempt.node_id,
        attempt: attempt.attempt,
        invocation_id: attempt.invocation_id,
        failure: AttemptFailure {
          kind: AttemptFailureKind::InvocationCancelled,
          code: AttemptFailureKind::InvocationCancelled.code().to_string(),
          message: "The step invocation was cancelled with its workflow run.".to_string(),
          details: None,
          ..AttemptFailure::legacy_defaults()
        },
      },
    )?;
    if disposition != StepFailureDisposition::StepFailed {
      return Err(RuntimeExecutionError::Stalled(
        "cancellation attempted to retry or fail the workflow".to_string(),
      ));
    }
  }

  let projection = engine.projection(run_id)?;
  let started_parallel_ids = projection
    .parallel_groups
    .values()
    .filter(|group| group.status == ParallelGroupStatus::Started)
    .map(|group| group.parallel_id.clone())
    .collect::<Vec<_>>();
  for parallel_id in started_parallel_ids {
    let group = engine
      .workflow()
      .parallel_group(&parallel_id)
      .ok_or_else(|| {
        RuntimeExecutionError::Stalled(
          "cancelling run references an unknown parallel group".to_string(),
        )
      })?;
    let projection = engine.projection(run_id)?;
    let mut failed_node_ids = Vec::new();
    let mut cancelled_node_ids = Vec::new();
    for node_id in &group.child_node_ids {
      let Some(attempt) = projection.latest_attempt(node_id) else {
        continue;
      };
      let AttemptStatus::Failed { failure } = &attempt.status else {
        continue;
      };
      if failure.kind == AttemptFailureKind::InvocationCancelled {
        cancelled_node_ids.push(node_id.clone());
      } else {
        failed_node_ids.push(node_id.clone());
      }
    }
    if failed_node_ids.is_empty() && cancelled_node_ids.is_empty() {
      continue;
    }
    engine.append_payload(
      run_id,
      RunEventPayload::ParallelGroupCompleted(ParallelGroupCompletedData {
        parallel_id,
        outcome: ParallelGroupOutcome::Failed,
        failed_node_ids,
        cancelled_node_ids,
      }),
    )?;
  }

  // A run may be cancelled while a fork is waiting on an approval, a retry,
  // or queued branch work. Close every opened ownership boundary before the
  // terminal cancellation event so replay and retention never see an orphaned
  // branch.
  let opened_forks = engine
    .workflow()
    .graph
    .forks
    .as_deref()
    .unwrap_or_default()
    .iter()
    .filter(|fork| {
      engine
        .projection(run_id)
        .ok()
        .is_some_and(|projection| projection.forks.contains_key(&fork.fork_id))
    })
    .cloned()
    .collect::<Vec<_>>();
  for fork in opened_forks {
    settle_open_fork_cancellation(engine, run_id, &fork)?;
  }

  let projection = engine.projection(run_id)?;
  let cancellation_request_id = projection.cancellation_request_id.ok_or_else(|| {
    RuntimeExecutionError::Stalled("cancelling run has no durable request identity".to_string())
  })?;
  engine.decide_run_cancelled(run_id, cancellation_request_id, occurred_at)
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
      if failure.kind == AttemptFailureKind::InvocationCancelled
        && engine.projection(run_id)?.status == RunStatus::Cancelling
      {
        return Ok(ScriptNodeOutcome::Cancelled);
      }
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
    | crate::RUN_EVENT_SCHEMA_VERSION_V9
    | crate::RUN_EVENT_SCHEMA_VERSION_V10
    | crate::RUN_EVENT_SCHEMA_VERSION_V11
    | crate::RUN_EVENT_SCHEMA_VERSION_V12 => RunFailedData::V2(RunFailedDataV2::Attempt {
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
  fn settle_workflow_timeout(
    &mut self,
    _run_id: &str,
    _occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<bool, RuntimeExecutionError> {
    Ok(false)
  }
  fn decide_run_cancelled(
    &mut self,
    run_id: &str,
    cancellation_request_id: String,
    _occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    self.append_payload(
      run_id,
      RunEventPayload::RunOutcomeDecided(RunOutcomeDecidedData::Cancelled {
        cancellation_request_id,
      }),
    )
  }
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
      let started = RunEventPayload::RunStarted(crate::RunStartedData {
        workflow_id: self.workflow().workflow_id.clone(),
        definition_hash: self.definition_hash().to_string(),
        trigger_id: Some(trigger_definition.id.clone()),
        trigger_handler: Some(trigger_definition.handler.clone()),
        trigger_occurrence_id: Some(generated_id("occ")),
        ingress: None,
        trigger,
      });
      let payloads = crate::durable::expand_model_v11_payload(self.workflow(), run_id, started)?;
      for payload in payloads {
        self.append_event(RunEvent {
          event_schema_version: self.event_schema_version(),
          event_id: generated_id("evt"),
          run_id: run_id.to_string(),
          sequence: self.events(run_id).len() as u64 + 1,
          occurred_at: chrono::Utc::now(),
          payload,
        })?;
      }
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
    let event_schema_version = self.event_schema_version();
    let payloads = if matches!(
      event_schema_version,
      crate::RUN_EVENT_SCHEMA_VERSION_V10
        | crate::RUN_EVENT_SCHEMA_VERSION_V11
        | crate::RUN_EVENT_SCHEMA_VERSION_V12
    ) {
      crate::durable::expand_model_v11_payload(self.workflow(), run_id, payload)?
    } else {
      vec![payload]
    };
    let occurred_at = chrono::Utc::now();
    for payload in payloads {
      let sequence = self.events(run_id).len() as u64 + 1;
      self.append_event(RunEvent {
        event_schema_version,
        event_id: generated_id("evt"),
        run_id: run_id.to_string(),
        sequence,
        occurred_at,
        payload,
      })?;
    }
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

  fn settle_workflow_timeout(
    &mut self,
    run_id: &str,
    occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<bool, RuntimeExecutionError> {
    Ok(matches!(
      DurableDagEngine::settle_run_timeout(self, run_id, occurred_at)?,
      RunTimeoutSettlement::TimedOut { .. }
    ))
  }

  fn decide_run_cancelled(
    &mut self,
    run_id: &str,
    cancellation_request_id: String,
    occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::decide_run_outcome(
      self,
      run_id,
      RunOutcomeDecidedData::Cancelled {
        cancellation_request_id,
      },
      occurred_at,
    )?;
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
