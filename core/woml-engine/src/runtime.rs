use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::future::{poll_fn, Future};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::task::Poll;
use std::time::Duration;

use futures_util::stream::FuturesUnordered;
use futures_util::StreamExt;
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
  ChoiceSelectedData, CompensationRegisteredData, CorrectionAttemptFailedData,
  CorrectionAttemptIdentityData, CorrectionInterruptedData, CorrectionRetryScheduledData,
  CorrectiveActionKind, FinalLifecycleStatus, ForEachIterationFailedData,
  ForEachIterationFailureKind, ForEachIterationIdentityData, ForEachIterationScope,
  ForEachIterationSkippedData, ForEachIterationSucceededData, ForEachOpenedData,
  ForEachSettledData, ForEachSkipReason, ForkBranchOutcome, ForkBranchSettledData, ForkJoinOutcome,
  ForkJoinSettledData, ForkOpenedData, LifecycleActionFailedData, LifecycleActionIdentityData,
  LifecycleFailure, LifecycleFailureKind, LifecycleHookCompletedData,
  LifecycleHookCompletionStatus, LifecycleHookRequestedData, LifecycleSubject,
  LifecycleSubjectKind, ManualRecoveryRequiredData, OperationExecutionMode, OperationFailedData,
  OperationStartedData, OperationSucceededData, ParallelFailure, ParallelFailurePolicy,
  ParallelGroupCompletedData, ParallelGroupOutcome, ParallelGroupStartedData, RunFailedData,
  RunFailedDataV1, RunFailedDataV2, RunFailedDataV3, RunFinalizedData, RunOutcomeDecidedData,
  RunSucceededData, SagaCorrectionReason, SagaCorrectionStartedData, SagaIdentityData,
  StepAttemptFailedData, StepAttemptStartedData, StepAttemptSucceededData, StepRetryScheduledData,
};
use crate::interval::{IntervalProgress, IntervalProgressReporter};
use crate::model::{
  ApprovalDefinition, CompiledCorrectionIdentity, CompiledCorrectiveAction,
  CompiledLifecycleAction, CompiledReusableInvocation, CompiledSaga, LifecycleEventName,
  ParallelGroupDefinition, TemplatePart, ValueExpression,
};
use crate::projection::{
  ApprovalRequestStatus, AttemptStatus, ForEachIterationStatus, ForEachStatus,
  LifecycleActionStatus, LifecycleHookProjection, LifecycleHookStatus, ParallelGroupStatus,
  ReusableLifecycleStatus,
};
use crate::protocol::{
  CleanupBindingV1, CleanupReasonV1, CompensationBindingV1, CompensationStatusV1,
  CorrectionBindingRef, CorrectionReasonV1, ExecuteMessage, HostOutcome, LifecycleBindingV1,
  LifecycleFailureBindingV1, LifecycleStepBindingV1, LifecycleWorkflowBindingV1,
  ReusableDefinitionBindingV3, ReusableDefinitionKind, ReusableInvocationOutcome,
  ReusableLifecycleBindingV1, ReusableLifecycleErrorV1,
  ReusableLifecycleHook as ProtocolReusableLifecycleHook, ReusableScriptBindingV3,
  RuntimeModuleBinding, ScriptAttempt,
};
use crate::schedule::{
  ScheduleClock, ScheduleProgress, ScheduleProgressReporter, SystemScheduleClock,
};
use crate::workflow_calls::WorkflowCallProgressReporter;
use crate::{
  run_event_schema_version_for_model, ApprovalDecisionOutcome, ApprovalTimeoutSettlement,
  AttemptFailure, AttemptFailureKind, BoundRichApprovalReview, BranchFailure, CapabilityFailure,
  CapabilityFailureKind, CapabilityRegistry, CompiledWorkflowDefinition,
  CustomNotificationHostClient, CustomNotificationHostClientError,
  CustomNotificationHostProcessOptions, CustomNotificationKind, CustomNotificationRequest,
  CustomProviderAttempt, CustomProviderExecuteMessage, CustomProviderLimits, CustomProviderOutcome,
  CustomProviderScriptArtifact, DurableCapabilityAuthority, DurableDagEngine, DurableEngineError,
  DurableEventStore, DurableStoreError, EngineError, InMemoryDagEngine,
  InformationalNotificationDeliverMessage, IssuedApprovalToken, NotificationCredentials,
  NotificationHostClient, NotificationHostClientError, NotificationHostOutcome,
  NotificationHostProcessOptions, OperationStatus, PolicyClaimWaitReason,
  PolicyExecutionClaimResult, PolicyWaitingFor, RecoveryReport, ReviewPayloadV1,
  ReviewerDirectoryAuthorityV1, RichApprovalArtifactVerifier, RichApprovalBindingInput,
  RichApprovalError, RichApprovalResolutionReason, RunEvent, RunEventPayload, RunFailure,
  RunProjection, RunStatus, RunTimeoutSettlement, SchedulerClaimV1, ScriptHostClient,
  ScriptHostClientError, ScriptHostModuleArtifact, ScriptHostProcessOptions,
  StepFailureDisposition, TriggerAdmissionRequest, WorkflowContext,
  CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL, CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
  INFORMATIONAL_NOTIFICATION_PROVIDER_PROTOCOL_VERSION, NOTIFICATION_PROVIDER_PROTOCOL,
  RUN_EVENT_SCHEMA_VERSION_V1, RUN_EVENT_SCHEMA_VERSION_V2,
};

pub trait EngineClock: Send + Sync {
  fn now(&self) -> chrono::DateTime<chrono::Utc>;
}

pub const EXECUTION_PROGRESS_CONTRACT: &str = "woml.execution-progress";
pub const EXECUTION_PROGRESS_VERSION: u32 = 1;

static REVIEWER_AUTHORITIES: OnceLock<RwLock<HashMap<PathBuf, Arc<ReviewerDirectoryAuthorityV1>>>> =
  OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExecutionProgress {
  ForEachProgress {
    contract: &'static str,
    version: u32,
    #[serde(rename = "runId")]
    run_id: String,
    #[serde(rename = "forEachId")]
    for_each_id: String,
    status: &'static str,
    total: u32,
    succeeded: u32,
    failed: u32,
    skipped: u32,
    active: u32,
    pending: u32,
    concurrency: u32,
  },
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
  pub custom_notification_host_path: Option<PathBuf>,
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
  pub operational_output_reporter: Option<crate::OperationalOutputReporter>,
  pub telemetry_reporter: Option<crate::TelemetrySignalReporter>,
  pub telemetry_context: Option<crate::TelemetrySignalContext>,
  pub telemetry_admission_link_reporter: Option<crate::TelemetryAdmissionLinkReporter>,
  pub telemetry_propagation: Arc<AtomicBool>,
  pub telemetry_sampling_ratio: Arc<RwLock<f64>>,
  pub resolved_secrets: Arc<BTreeMap<String, String>>,
  pub(crate) signal_ingress_token: Option<String>,
  pub capability_registry: Arc<CapabilityRegistry>,
  pub runtime_modules: Arc<Vec<RuntimeModuleArtifact>>,
  shared_script_hosts: Option<Arc<SharedScriptHostPool>>,
  capability_authority: Option<Arc<DurableCapabilityAuthority>>,
  managed_database_pool: Option<Arc<crate::ManagedDatabasePool>>,
  managed_storage_store: Option<Arc<crate::ManagedStorageStore>>,
  managed_cache_store: Option<Arc<crate::ManagedCacheStore>>,
  managed_durable_state_store: Option<Arc<crate::ManagedDurableStateStore>>,
  managed_locks_service: Option<Arc<crate::ManagedLocksService>>,
  managed_artifact_store: Option<Arc<crate::ManagedArtifactStore>>,
  managed_artifact_put_handler: Option<Arc<crate::ManagedArtifactPutHandler>>,
  managed_process_handler: Option<Arc<crate::ManagedProcessHandler>>,
  operational_output_store: Option<Arc<crate::LazyOperationalOutputStore>>,
  artifact_policy: Option<Arc<crate::artifacts::PreparedArtifactPolicy>>,
  infrastructure_policy: Option<Arc<crate::PreparedInfrastructurePolicyV1>>,
  emergency_deny_overlay: Option<Arc<crate::PreparedInfrastructurePolicyV1>>,
  infrastructure_policy_required: bool,
  reviewer_authority: Option<Arc<ReviewerDirectoryAuthorityV1>>,
  policy_execution: Option<Arc<PolicyExecutionCoordinator>>,
  policy_execution_registry: PolicyExecutionRegistry,
  lock_renewals: Arc<std::sync::Mutex<HashMap<String, Arc<AtomicBool>>>>,
}

struct ArtifactInvocationCleanup {
  store: Option<Arc<crate::ManagedArtifactStore>>,
  invocation_id: String,
}

impl ArtifactInvocationCleanup {
  fn new(options: &RuntimeExecutionOptions, invocation_id: &str) -> Self {
    Self {
      store: options.managed_artifact_store.clone(),
      invocation_id: invocation_id.to_string(),
    }
  }

  fn from_store(store: Option<Arc<crate::ManagedArtifactStore>>, invocation_id: &str) -> Self {
    Self {
      store,
      invocation_id: invocation_id.to_string(),
    }
  }
}

impl Drop for ArtifactInvocationCleanup {
  fn drop(&mut self) {
    if let Some(store) = &self.store {
      store.cleanup_invocation(&self.invocation_id);
    }
  }
}

#[derive(Debug)]
struct IdleScriptHost {
  options: ScriptHostProcessOptions,
  client: ScriptHostClient,
}

#[derive(Debug, Default)]
struct SharedScriptHostPool {
  idle: tokio::sync::Mutex<Vec<IdleScriptHost>>,
}

const MAX_IDLE_SCRIPT_HOSTS: usize = 4;

impl std::fmt::Debug for RuntimeExecutionOptions {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    formatter
      .debug_struct("RuntimeExecutionOptions")
      .field("script_host", &self.script_host)
      .field("script_timeout_ms", &self.script_timeout_ms)
      .field("notification_host", &self.notification_host)
      .field(
        "custom_notification_host_path",
        &self.custom_notification_host_path,
      )
      .field("max_context_bytes", &self.max_context_bytes)
      .field("clock", &"dyn EngineClock")
      .field("schedule_clock", &"dyn ScheduleClock")
      .field("resolved_secret_count", &self.resolved_secrets.len())
      .field("capability_registry", &"CapabilityRegistry")
      .field("runtime_module_count", &self.runtime_modules.len())
      .field(
        "infrastructure_policy_digest",
        &self
          .infrastructure_policy
          .as_ref()
          .map(|policy| &policy.digest),
      )
      .field(
        "infrastructure_policy_required",
        &self.infrastructure_policy_required,
      )
      .field(
        "emergency_deny_overlay_digest",
        &self
          .emergency_deny_overlay
          .as_ref()
          .map(|policy| &policy.digest),
      )
      .field(
        "shared_script_hosts",
        &self.shared_script_hosts.as_ref().map(|_| "enabled"),
      )
      .field(
        "lock_renewal_count",
        &self
          .lock_renewals
          .lock()
          .map(|items| items.len())
          .unwrap_or(0),
      )
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
      .field(
        "telemetry_reporter",
        &self.telemetry_reporter.as_ref().map(|_| "configured"),
      )
      .field(
        "telemetry_context",
        &self.telemetry_context.as_ref().map(|_| "configured"),
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
    let custom_notification_host_path = script_host.host_script_path.parent().map(|parent| {
      let extension = script_host
        .host_script_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("js");
      parent.join(format!("custom-notification-provider-host.{extension}"))
    });
    let capability_registry = Arc::new(CapabilityRegistry::default());
    let managed_storage_store = Arc::new(crate::ManagedStorageStore::default());
    let telemetry_propagation = Arc::new(AtomicBool::new(false));
    let telemetry_sampling_ratio = Arc::new(RwLock::new(0.0));
    capability_registry
      .register(Arc::new(
        crate::ManagedHttpHandler::with_storage_and_propagation(
          Arc::clone(&managed_storage_store),
          Arc::clone(&telemetry_propagation),
          Arc::clone(&telemetry_sampling_ratio),
        ),
      ))
      .expect("the production HTTP capability is registered exactly once");
    capability_registry
      .register(Arc::new(crate::ManagedTelegramHandler::default()))
      .expect("the production Telegram capability is registered exactly once");
    capability_registry
      .register(Arc::new(crate::ManagedDiscordHandler::default()))
      .expect("the production Discord capability is registered exactly once");
    capability_registry
      .register(Arc::new(crate::ManagedWhatsAppHandler::default()))
      .expect("the production WhatsApp capability is registered exactly once");
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
    let managed_locks_service = Arc::new(crate::ManagedLocksService::default());
    for handler in crate::ManagedLocksService::handlers(Arc::clone(&managed_locks_service)) {
      capability_registry
        .register(handler)
        .expect("each production lock operation is registered exactly once");
    }
    Self {
      script_host,
      notification_host,
      custom_notification_host_path,
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
      operational_output_reporter: None,
      telemetry_reporter: None,
      telemetry_context: None,
      telemetry_admission_link_reporter: None,
      telemetry_propagation,
      telemetry_sampling_ratio,
      resolved_secrets: Arc::new(BTreeMap::new()),
      signal_ingress_token: None,
      capability_registry,
      runtime_modules: Arc::new(Vec::new()),
      shared_script_hosts: None,
      capability_authority: None,
      managed_database_pool: Some(managed_database_pool),
      managed_storage_store: Some(managed_storage_store),
      managed_cache_store: Some(managed_cache_store),
      managed_durable_state_store: Some(managed_durable_state_store),
      managed_locks_service: Some(managed_locks_service),
      managed_artifact_store: None,
      managed_artifact_put_handler: None,
      managed_process_handler: None,
      operational_output_store: None,
      artifact_policy: None,
      infrastructure_policy: None,
      emergency_deny_overlay: None,
      infrastructure_policy_required: false,
      reviewer_authority: None,
      policy_execution: None,
      policy_execution_registry: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
      lock_renewals: Arc::new(std::sync::Mutex::new(HashMap::new())),
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

  pub fn with_telemetry_reporter(
    mut self,
    context: crate::TelemetrySignalContext,
    reporter: crate::TelemetrySignalReporter,
  ) -> Self {
    self.telemetry_context = Some(context);
    self.telemetry_reporter = Some(reporter);
    self
  }

  pub fn with_telemetry_propagation(
    mut self,
    enabled: bool,
    sampling_ratio: f64,
    reporter: Option<crate::TelemetryAdmissionLinkReporter>,
  ) -> Self {
    self.telemetry_propagation.store(enabled, Ordering::Release);
    *self
      .telemetry_sampling_ratio
      .write()
      .expect("telemetry sampling ratio lock") = sampling_ratio.clamp(0.0, 1.0);
    self.telemetry_admission_link_reporter = reporter;
    self
  }

  pub fn report_telemetry_admission_context(
    &self,
    run_id: &str,
    context: crate::InboundTraceContext,
    occurred_at: chrono::DateTime<chrono::Utc>,
  ) {
    if !self.telemetry_propagation.load(Ordering::Acquire) {
      return;
    }
    let (Some(reporter), Some(signal_context)) = (
      self.telemetry_admission_link_reporter.as_ref(),
      self.telemetry_context.as_ref(),
    ) else {
      return;
    };
    let (status, trace_id, span_id, sampled) = match context {
      crate::InboundTraceContext::Absent => return,
      crate::InboundTraceContext::Invalid => (
        crate::TelemetryAdmissionLinkStatus::Invalid,
        None,
        None,
        None,
      ),
      crate::InboundTraceContext::Valid(context) => (
        crate::TelemetryAdmissionLinkStatus::Linked,
        Some(context.trace_id),
        Some(context.span_id),
        Some(context.sampled),
      ),
    };
    reporter(crate::TelemetryAdmissionLinkV1 {
      profile: crate::TELEMETRY_ADMISSION_LINK_PROFILE,
      occurred_at,
      runtime_instance_id: signal_context.runtime_instance_id.clone(),
      deployment_id: signal_context.deployment_id.clone(),
      run_id: run_id.to_string(),
      status,
      trace_id,
      span_id,
      sampled,
    });
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

  pub fn with_signal_ingress_token(mut self, token: Option<String>) -> Self {
    self.signal_ingress_token = token.filter(|token| !token.is_empty());
    self
  }

  pub fn with_infrastructure_policy(
    mut self,
    policy: crate::PreparedInfrastructurePolicyV1,
  ) -> Self {
    self.infrastructure_policy = Some(Arc::new(policy));
    self.infrastructure_policy_required = true;
    self
  }

  pub fn require_infrastructure_policy(mut self) -> Self {
    self.infrastructure_policy_required = true;
    self
  }

  pub fn with_emergency_deny_overlay(
    mut self,
    policy: crate::PreparedInfrastructurePolicyV1,
  ) -> Self {
    self.emergency_deny_overlay = Some(Arc::new(policy));
    self.infrastructure_policy_required = true;
    self
  }

  pub fn with_runtime_modules(mut self, modules: Vec<RuntimeModuleArtifact>) -> Self {
    self.script_host.module_artifacts = modules
      .iter()
      .filter(|module| !module.name.starts_with("__woml_"))
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

  pub fn with_shared_script_hosts(mut self) -> Self {
    self.shared_script_hosts = Some(Arc::new(SharedScriptHostPool::default()));
    self
  }

  async fn acquire_script_host(&self) -> Result<ScriptHostClient, ScriptHostClientError> {
    let Some(pool) = &self.shared_script_hosts else {
      return ScriptHostClient::spawn_with_authority(
        self.script_host.clone(),
        self.capability_authority.clone(),
      )
      .await;
    };
    loop {
      let candidate = {
        let mut idle = pool.idle.lock().await;
        idle
          .iter()
          .position(|host| host.options == self.script_host)
          .map(|index| idle.swap_remove(index).client)
      };
      let Some(mut client) = candidate else {
        return ScriptHostClient::spawn_with_authority(
          self.script_host.clone(),
          self.capability_authority.clone(),
        )
        .await;
      };
      if client.is_healthy().await {
        client.set_default_authority(self.capability_authority.clone());
        return Ok(client);
      }
      client.shutdown().await;
    }
  }

  async fn release_script_host(&self, mut client: ScriptHostClient) {
    let Some(pool) = &self.shared_script_hosts else {
      client.shutdown().await;
      return;
    };
    if !client.is_healthy().await {
      client.shutdown().await;
      return;
    }
    client.set_default_authority(None);
    let mut idle = pool.idle.lock().await;
    if idle.len() < MAX_IDLE_SCRIPT_HOSTS {
      idle.push(IdleScriptHost {
        options: self.script_host.clone(),
        client,
      });
      return;
    }
    drop(idle);
    client.shutdown().await;
  }

  pub async fn shutdown_shared_script_hosts(&self) {
    let Some(pool) = &self.shared_script_hosts else {
      return;
    };
    let clients = {
      let mut idle = pool.idle.lock().await;
      std::mem::take(&mut *idle)
    };
    for host in clients {
      host.client.shutdown().await;
    }
  }

  pub fn with_capability_registry(mut self, registry: Arc<CapabilityRegistry>) -> Self {
    self.capability_registry = registry;
    self.managed_database_pool = None;
    self.managed_storage_store = None;
    self.managed_cache_store = None;
    self.managed_durable_state_store = None;
    self.managed_locks_service = None;
    self.managed_artifact_store = None;
    self.managed_artifact_put_handler = None;
    self.managed_process_handler = None;
    self.operational_output_store = None;
    self.artifact_policy = None;
    self
  }

  /// Binds worker-side capability calls to a durable authority owned by the
  /// worker journal database. Remote attempt runtimes use this narrow accessor
  /// so they share the exact same policy, idempotency, and cancellation path as
  /// local script execution without gaining workflow scheduling authority.
  pub fn prepare_remote_worker_capabilities(
    self,
    database_path: &Path,
  ) -> Result<Self, RuntimeExecutionError> {
    attach_durable_capability_authority(self, database_path)
  }

  pub fn remote_worker_capability_authority(&self) -> Option<Arc<DurableCapabilityAuthority>> {
    self.capability_authority.clone()
  }

  /// Enables the managed process capability for this runtime only after the
  /// runtime-owned Process Policy v1 has been validated by the Rust core.
  pub fn with_process_policy(
    mut self,
    policy: crate::ProcessPolicyV1,
    mode: crate::ProcessRuntimeMode,
  ) -> Result<Self, crate::ProcessPolicyError> {
    let handler = Arc::new(crate::ManagedProcessHandler::new_with_secrets(
      policy,
      mode,
      Arc::clone(&self.resolved_secrets),
    )?);
    if let Some(artifacts) = &self.managed_artifact_put_handler {
      handler.configure_output_artifacts(Arc::clone(artifacts))?;
    }
    self
      .capability_registry
      .register(handler.clone())
      .map_err(|error| crate::ProcessPolicyError::Invalid(error.to_string()))?;
    self.managed_process_handler = Some(handler);
    Ok(self)
  }

  /// Enables durable artifacts only after Rust validates Artifact Policy v1.
  pub fn with_artifact_policy(
    mut self,
    policy: crate::ArtifactPolicyV1,
    mode: crate::ArtifactRuntimeMode,
  ) -> Result<Self, crate::ArtifactPolicyError> {
    let store = Arc::new(crate::ManagedArtifactStore::default());
    let handler = Arc::new(crate::ManagedArtifactPutHandler::new(
      Arc::clone(&store),
      policy,
      mode,
      &self.resolved_secrets,
    )?);
    let head_handler = handler.head_handler();
    let materialize_handler = handler.materialize_handler();
    self.artifact_policy = Some(handler.policy());
    if let Some(process) = &self.managed_process_handler {
      process
        .configure_output_artifacts(Arc::clone(&handler))
        .map_err(|error| crate::ArtifactPolicyError::Invalid(error.to_string()))?;
    }
    self
      .capability_registry
      .register(handler.clone())
      .map_err(|error| crate::ArtifactPolicyError::Invalid(error.to_string()))?;
    self
      .capability_registry
      .register(Arc::new(head_handler))
      .map_err(|error| crate::ArtifactPolicyError::Invalid(error.to_string()))?;
    self
      .capability_registry
      .register(Arc::new(materialize_handler))
      .map_err(|error| crate::ArtifactPolicyError::Invalid(error.to_string()))?;
    self.managed_artifact_store = Some(store);
    self.managed_artifact_put_handler = Some(handler);
    Ok(self)
  }

  pub fn with_reviewer_directory(mut self, authority: Arc<ReviewerDirectoryAuthorityV1>) -> Self {
    self.reviewer_authority = Some(authority);
    self
  }

  pub fn reviewer_authority(&self) -> Option<Arc<ReviewerDirectoryAuthorityV1>> {
    self.reviewer_authority.clone()
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

  pub fn with_operational_output_reporter(
    mut self,
    reporter: crate::OperationalOutputReporter,
  ) -> Self {
    self.operational_output_reporter = Some(reporter);
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

  async fn release_policy_execution_slot(
    &self,
    store: &mut DurableEventStore,
  ) -> Result<(), RuntimeExecutionError> {
    let Some(coordinator) = &self.policy_execution else {
      return Ok(());
    };
    let lease = coordinator.lease.lock().await.take();
    if let Some(lease) = lease {
      lease.release(store).await?;
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

  fn ensure_lock_renewal(&self, run_id: &str, lock_id: &str, lease_ms: u64) {
    let Some(authority) = self.capability_authority.clone() else {
      return;
    };
    let identity = format!("{run_id}\0{lock_id}");
    let cancelled = Arc::new(AtomicBool::new(false));
    {
      let Ok(mut renewals) = self.lock_renewals.lock() else {
        return;
      };
      if renewals.contains_key(&identity) {
        return;
      }
      renewals.insert(identity.clone(), Arc::clone(&cancelled));
    }
    let renewals = Arc::clone(&self.lock_renewals);
    let run_id = run_id.to_string();
    let lock_id = lock_id.to_string();
    let interval = Duration::from_millis((lease_ms / 3).max(1));
    tokio::spawn(async move {
      loop {
        tokio::time::sleep(interval).await;
        if cancelled.load(Ordering::SeqCst) {
          break;
        }
        let outcome = authority.store().lock().await.renew_lock_lease_atomically(
          &run_id,
          &lock_id,
          chrono::Utc::now(),
        );
        if !matches!(outcome, Ok(crate::LockLeaseMaintenance::Renewed)) {
          break;
        }
      }
      if let Ok(mut active) = renewals.lock() {
        active.remove(&identity);
      }
    });
  }

  fn stop_lock_renewal(&self, run_id: &str, lock_id: &str) {
    let identity = format!("{run_id}\0{lock_id}");
    if let Ok(mut renewals) = self.lock_renewals.lock() {
      if let Some(cancelled) = renewals.remove(&identity) {
        cancelled.store(true, Ordering::SeqCst);
      }
    }
  }

  fn ensure_iteration_lock_renewal(
    &self,
    run_id: &str,
    lock_id: &str,
    scope: ForEachIterationScope,
    lease_ms: u64,
  ) {
    let Some(authority) = self.capability_authority.clone() else {
      return;
    };
    let identity = format!(
      "{run_id}\0{}\0{}\0{lock_id}",
      scope.for_each_id, scope.index
    );
    let cancelled = Arc::new(AtomicBool::new(false));
    {
      let Ok(mut renewals) = self.lock_renewals.lock() else {
        return;
      };
      if renewals.contains_key(&identity) {
        return;
      }
      renewals.insert(identity.clone(), Arc::clone(&cancelled));
    }
    let renewals = Arc::clone(&self.lock_renewals);
    let run_id = run_id.to_string();
    let lock_id = lock_id.to_string();
    let interval = Duration::from_millis((lease_ms / 3).max(1));
    tokio::spawn(async move {
      loop {
        tokio::time::sleep(interval).await;
        if cancelled.load(Ordering::SeqCst) {
          break;
        }
        let outcome = authority
          .store()
          .lock()
          .await
          .renew_lock_lease_atomically_scoped(
            &run_id,
            &lock_id,
            Some(scope.clone()),
            chrono::Utc::now(),
          );
        if !matches!(outcome, Ok(crate::LockLeaseMaintenance::Renewed)) {
          break;
        }
      }
      if let Ok(mut active) = renewals.lock() {
        active.remove(&identity);
      }
    });
  }

  fn stop_iteration_lock_renewal(
    &self,
    run_id: &str,
    lock_id: &str,
    scope: &ForEachIterationScope,
  ) {
    let identity = format!(
      "{run_id}\0{}\0{}\0{lock_id}",
      scope.for_each_id, scope.index
    );
    if let Ok(mut renewals) = self.lock_renewals.lock() {
      if let Some(cancelled) = renewals.remove(&identity) {
        cancelled.store(true, Ordering::SeqCst);
      }
    }
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

fn report_for_each_progress<E: RuntimeDagEngine>(
  engine: &E,
  options: &RuntimeExecutionOptions,
  run_id: &str,
  for_each_id: &str,
) -> Result<(), RuntimeExecutionError> {
  let projection = engine.projection(run_id)?;
  let Some(state) = projection.for_each.get(for_each_id) else {
    return Ok(());
  };
  let succeeded = state
    .iterations
    .values()
    .filter(|status| matches!(status, ForEachIterationStatus::Succeeded { .. }))
    .count() as u32;
  let failed = state
    .iterations
    .values()
    .filter(|status| matches!(status, ForEachIterationStatus::Failed { .. }))
    .count() as u32;
  let skipped = state
    .iterations
    .values()
    .filter(|status| matches!(status, ForEachIterationStatus::Skipped))
    .count() as u32;
  let active = state
    .iterations
    .values()
    .filter(|status| matches!(status, ForEachIterationStatus::Started))
    .count() as u32;
  let pending = state
    .total
    .saturating_sub(succeeded + failed + skipped + active);
  let status = match state.status {
    ForEachStatus::Open => "running",
    ForEachStatus::Succeeded => "succeeded",
    ForEachStatus::Failed => "failed",
    ForEachStatus::Cancelled => "cancelled",
  };
  options.report(ExecutionProgress::ForEachProgress {
    contract: EXECUTION_PROGRESS_CONTRACT,
    version: EXECUTION_PROGRESS_VERSION,
    run_id: run_id.to_string(),
    for_each_id: for_each_id.to_string(),
    status,
    total: state.total,
    succeeded,
    failed,
    skipped,
    active,
    pending,
    concurrency: state.concurrency,
  });
  Ok(())
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
  #[serde(rename = "waiting_time")]
  WaitingTime {
    contract: &'static str,
    version: u32,
    #[serde(rename = "workflowId")]
    workflow_id: String,
    #[serde(rename = "runId")]
    run_id: String,
    wait: WaitingWorkflowTimer,
  },
  #[serde(rename = "waiting_signal")]
  WaitingSignal {
    contract: &'static str,
    version: u32,
    #[serde(rename = "workflowId")]
    workflow_id: String,
    #[serde(rename = "runId")]
    run_id: String,
    wait: WaitingWorkflowSignal,
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
  #[serde(skip_serializing_if = "Option::is_none")]
  pub token: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub credential_expires_at: Option<chrono::DateTime<chrono::Utc>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub review_digest: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub review: Option<ReviewPayloadV1>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub authority: Option<WaitingRichApprovalAuthority>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitingRichApprovalAuthority {
  pub required: u32,
  pub received: u32,
  pub required_roles: Vec<String>,
  pub represented_roles: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitingWorkflowTimer {
  pub wait_id: String,
  pub node_id: String,
  pub kind: crate::DurableWaitKind,
  pub registered_at: chrono::DateTime<chrono::Utc>,
  pub deadline: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitingWorkflowSignal {
  pub wait_id: String,
  pub node_id: String,
  pub registered_at: chrono::DateTime<chrono::Utc>,
  pub signals: Vec<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub deadline: Option<chrono::DateTime<chrono::Utc>>,
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
  let mut engine = InMemoryDagEngine::new(workflow, definition_hash)?;
  succeeded_execution(execute_with_engine(&mut engine, trigger, options).await?)
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

  async fn release(mut self, store: &mut DurableEventStore) -> Result<(), RuntimeExecutionError> {
    if let Some(stop) = self.stop.take() {
      let _ = stop.send(());
    }
    let _ = self.heartbeat.await;
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
  let mut store = DurableEventStore::open(database_path.to_path_buf())?;
  acquire_policy_execution_lease_with_store(database_path, run_id, options, &mut store).await
}

async fn acquire_policy_execution_lease_with_store(
  database_path: &std::path::Path,
  run_id: &str,
  options: &RuntimeExecutionOptions,
  store: &mut DurableEventStore,
) -> Result<PolicyClaimAcquisition, RuntimeExecutionError> {
  let mut span =
    crate::performance::PerformanceSpan::new("runtime", "runtime.acquire_policy_lease");
  span.run_id(run_id.to_string());
  let result =
    acquire_policy_execution_lease_unprofiled(database_path, run_id, options, store).await;
  if result.is_ok() {
    span.succeed();
  }
  result
}

async fn acquire_policy_execution_lease_unprofiled(
  database_path: &std::path::Path,
  run_id: &str,
  options: &RuntimeExecutionOptions,
  store: &mut DurableEventStore,
) -> Result<PolicyClaimAcquisition, RuntimeExecutionError> {
  let owner_id = format!("scheduler_{}", Uuid::new_v4().simple());
  let mut reported_wait = false;
  loop {
    let lease_now = chrono::Utc::now();
    let policy_now = options.clock.now();
    let rate_eligible_at = match store.try_claim_policy_run_at(
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
        eligible_at
      }
      Err(DurableStoreError::SchedulerRecoveryRequired(_)) => {
        store.recover_policy_run_after_lease_loss(run_id, lease_now)?;
        return Ok(PolicyClaimAcquisition::Recovered);
      }
      Err(error) => return Err(error.into()),
    };
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
  let mut store = DurableEventStore::open(database_path.clone())?;
  let acquisition =
    acquire_policy_execution_lease_with_store(&database_path, run_id, &options, &mut store).await?;
  let binding = store.run_binding(run_id)?;
  let mut options = runtime_modules_from_store(options, &store, &binding.definition_hash)?;
  let mut engine = DurableDagEngine::resume(store, run_id)?;
  match acquisition {
    PolicyClaimAcquisition::Recovered => {
      let _ = resume_with_engine(&mut engine, run_id, options).await;
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
      let _ = resume_with_engine(&mut engine, run_id, options.clone()).await;
      let mut store = engine.into_store();
      let released = options.release_policy_execution_slot(&mut store).await;
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
  store: DurableEventStore,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  let mut span = crate::performance::PerformanceSpan::new("runtime", "runtime.execute_policy_run");
  span.run_id(run_id.to_string());
  let result = execute_policy_run_durable_unprofiled(database_path, run_id, options, store).await;
  if result.is_ok() {
    span.succeed();
  }
  result
}

async fn execute_policy_run_durable_unprofiled(
  database_path: PathBuf,
  run_id: &str,
  options: RuntimeExecutionOptions,
  mut store: DurableEventStore,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  {
    let binding = store.run_binding(run_id)?;
    let workflow = store.definition(&binding.definition_hash)?;
    validate_rich_approval_authority(&workflow, &options)?;
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
      let mut engine = DurableDagEngine::resume(store, run_id)?;
      return resume_with_engine(&mut engine, run_id, options).await;
    }
  }
  let acquisition =
    acquire_policy_execution_lease_with_store(&database_path, run_id, &options, &mut store).await?;
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
  let mut engine = DurableDagEngine::resume(store, run_id)?;
  match acquisition {
    PolicyClaimAcquisition::Recovered => {
      recover_operational_run(&options, run_id)?;
      resume_with_engine(&mut engine, run_id, options).await
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
      let outcome = resume_with_engine(&mut engine, run_id, options.clone()).await;
      let mut store = engine.into_store();
      let released = options.release_policy_execution_slot(&mut store).await;
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
  if workflow_has_approval(&workflow)
    || workflow_has_durable_wait(&workflow)
    || workflow_has_durable_lock(&workflow)
  {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "durably waiting workflows require execute_workflow_durable_outcome".to_string(),
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
  if !workflow_has_approval(&workflow)
    && !workflow_has_durable_wait(&workflow)
    && !workflow_has_durable_lock(&workflow)
  {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "the runtime outcome API requires an approval, durable-wait, or lock workflow".to_string(),
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
  let mut span = crate::performance::PerformanceSpan::new("runtime", "runtime.execute_durable");
  span.count("nodes", workflow.graph.nodes.len());
  span.count("modules", options.runtime_modules.len());
  let result = execute_workflow_durable_internal_unprofiled(
    workflow,
    definition_hash,
    trigger,
    options,
    database_path,
  )
  .await;
  if let Ok(outcome) = &result {
    span.run_id(runtime_outcome_run_id(outcome).to_string());
    span.succeed();
  }
  result
}

async fn execute_workflow_durable_internal_unprofiled(
  workflow: CompiledWorkflowDefinition,
  definition_hash: String,
  trigger: Map<String, Value>,
  options: RuntimeExecutionOptions,
  database_path: PathBuf,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  validate_rich_approval_authority(&workflow, &options)?;
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
    bind_run_infrastructure_policy(&mut store, &admission.run_id, &options)?;
    return execute_policy_run_durable(database_path, &admission.run_id, options, store).await;
  }
  recover_operational_output(&options)?;
  store.recover_interrupted_runs()?;
  store.register_definition_module_artifacts(
    &workflow,
    &definition_hash,
    options.runtime_modules.as_ref(),
  )?;
  let mut engine = DurableDagEngine::new(workflow, definition_hash, store)?;
  execute_with_engine(&mut engine, trigger, options).await
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
  let mut span =
    crate::performance::PerformanceSpan::new("runtime", "runtime.execute_admitted_run");
  span.run_id(run_id.to_string());
  let result =
    execute_admitted_trigger_run_durable_unprofiled(database_path, run_id, options).await;
  if result.is_ok() {
    span.succeed();
  }
  result
}

async fn execute_admitted_trigger_run_durable_unprofiled(
  database_path: PathBuf,
  run_id: &str,
  options: RuntimeExecutionOptions,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  let options = attach_durable_capability_authority(options, &database_path)?;
  let mut store = DurableEventStore::open(database_path.clone())?;
  bind_run_infrastructure_policy(&mut store, run_id, &options)?;
  let binding = store.run_binding(run_id)?;
  let workflow = store.definition(&binding.definition_hash)?;
  validate_rich_approval_authority(&workflow, &options)?;
  if workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V12 {
    return execute_policy_run_durable(database_path, run_id, options, store).await;
  }
  let options = runtime_modules_from_store(options, &store, &binding.definition_hash)?;
  let mut engine = DurableDagEngine::resume(store, run_id)?;
  resume_with_engine(&mut engine, run_id, options).await
}

async fn resume_workflow_durable_internal(
  database_path: PathBuf,
  run_id: &str,
  options: RuntimeExecutionOptions,
  approval_outcome_api: Option<bool>,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  let options = attach_durable_capability_authority(options, &database_path)?;
  let mut store = DurableEventStore::open(database_path.clone())?;
  bind_run_infrastructure_policy(&mut store, run_id, &options)?;
  let binding = store.run_binding(run_id)?;
  let workflow = store.definition(&binding.definition_hash)?;
  validate_rich_approval_authority(&workflow, &options)?;
  let has_approval = workflow_has_approval(&workflow);
  let has_durable_wait = workflow_has_durable_wait(&workflow);
  let has_durable_lock = workflow_has_durable_lock(&workflow);
  if approval_outcome_api == Some(true) && !has_approval && !has_durable_wait && !has_durable_lock {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "the runtime outcome API requires an approval, durable-wait, or lock workflow".to_string(),
    ));
  }
  if approval_outcome_api == Some(false) && (has_approval || has_durable_wait || has_durable_lock) {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "durably waiting workflows require resume_workflow_durable_outcome".to_string(),
    ));
  }
  if workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V12 {
    return execute_policy_run_durable(database_path, run_id, options, store).await;
  }
  recover_operational_output(&options)?;
  store.recover_interrupted_runs()?;
  let options = runtime_modules_from_store(options, &store, &binding.definition_hash)?;
  let mut engine = DurableDagEngine::resume(store, run_id)?;
  resume_with_engine(&mut engine, run_id, options).await
}

fn attach_durable_capability_authority(
  mut options: RuntimeExecutionOptions,
  database_path: &std::path::Path,
) -> Result<RuntimeExecutionOptions, RuntimeExecutionError> {
  let operational_output = Arc::new(
    crate::operational_output_store_for_state(database_path)
      .map(|store| store.with_reporter(options.operational_output_reporter.clone()))
      .map_err(|error| {
        RuntimeExecutionError::InvalidConfiguration(format!(
          "{}: the operational-output path could not be derived",
          error.code()
        ))
      })?,
  );
  if let Some(process) = &options.managed_process_handler {
    process
      .configure_output_store(Arc::clone(&operational_output))
      .map_err(|error| RuntimeExecutionError::InvalidConfiguration(error.to_string()))?;
  }
  options.operational_output_store = Some(Arc::clone(&operational_output));
  if let Some(authority) = options.reviewer_authority() {
    REVIEWER_AUTHORITIES
      .get_or_init(|| RwLock::new(HashMap::new()))
      .write()
      .map_err(|_| {
        RuntimeExecutionError::InvalidConfiguration(
          "WOML_APPROVAL_AUTHORITY_UNAVAILABLE: reviewer authority registry is unavailable"
            .to_string(),
        )
      })?
      .insert(database_path.to_path_buf(), authority);
  }
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
  if let Some(locks) = &options.managed_locks_service {
    locks
      .configure_for_state(database_path)
      .map_err(|failure| RuntimeExecutionError::InvalidConfiguration(failure.message))?;
  }
  if let (Some(artifacts), Some(policy)) =
    (&options.managed_artifact_store, &options.artifact_policy)
  {
    artifacts
      .configure_for_state(database_path, Arc::clone(policy))
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
  let locks = options
    .managed_locks_service
    .clone()
    .unwrap_or_else(|| Arc::new(crate::ManagedLocksService::default()));
  options.capability_authority = Some(Arc::new(
    DurableCapabilityAuthority::new_with_infrastructure_policy_authorities(
      Arc::clone(&options.capability_registry),
      Arc::new(tokio::sync::Mutex::new(store)),
      locks,
      options.infrastructure_policy_required || options.infrastructure_policy.is_some(),
      options.managed_artifact_store.clone(),
      options.emergency_deny_overlay.clone(),
    )
    .with_operational_output(operational_output, Arc::clone(&options.resolved_secrets)),
  ));
  Ok(options)
}

fn recover_operational_output(
  options: &RuntimeExecutionOptions,
) -> Result<(), RuntimeExecutionError> {
  let Some(lazy) = options.operational_output_store.as_ref() else {
    return Ok(());
  };
  let Some(store) = lazy.activate_if_exists().map_err(|error| {
    RuntimeExecutionError::InvalidConfiguration(format!("{}: {error}", error.code()))
  })?
  else {
    return Ok(());
  };
  store.recover_interrupted_streams().map_err(|error| {
    RuntimeExecutionError::InvalidConfiguration(format!("{}: {error}", error.code()))
  })?;
  Ok(())
}

fn recover_operational_run(
  options: &RuntimeExecutionOptions,
  run_id: &str,
) -> Result<(), RuntimeExecutionError> {
  let Some(lazy) = options.operational_output_store.as_ref() else {
    return Ok(());
  };
  let Some(store) = lazy.activate_if_exists().map_err(|error| {
    RuntimeExecutionError::InvalidConfiguration(format!("{}: {error}", error.code()))
  })?
  else {
    return Ok(());
  };
  store.recover_interrupted_run(run_id).map_err(|error| {
    RuntimeExecutionError::InvalidConfiguration(format!("{}: {error}", error.code()))
  })?;
  Ok(())
}

fn bind_run_infrastructure_policy(
  store: &mut DurableEventStore,
  run_id: &str,
  options: &RuntimeExecutionOptions,
) -> Result<(), RuntimeExecutionError> {
  if let Some(policy) = options.infrastructure_policy.as_ref() {
    store.bind_infrastructure_policy(run_id, policy, options.clock.now())?;
    return Ok(());
  }
  if options.infrastructure_policy_required
    && store.infrastructure_policy_binding(run_id)?.is_none()
  {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "WOML_POLICY_REQUIRED: this runtime requires an infrastructure policy, but the run has no admitted policy bundle."
        .to_string(),
    ));
  }
  Ok(())
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

/// Finalizes one custom notification provider's definition-owned lifecycle
/// after its durable delivery outcome is known. Notification journeys use this
/// adapter so provider hooks share the same Event v13 authority, immutable
/// artifacts, managed services, and recovery rules as custom-step hooks.
pub async fn execute_reusable_provider_lifecycle_durable(
  database_path: PathBuf,
  run_id: &str,
  provider_id: &str,
  outcome: ReusableInvocationOutcome,
  result: Option<Value>,
  error: Option<ReusableLifecycleErrorV1>,
  options: RuntimeExecutionOptions,
) -> Result<(), RuntimeExecutionError> {
  let options = attach_durable_capability_authority(options, &database_path)?;
  let store = DurableEventStore::open(database_path)?;
  let binding = store.run_binding(run_id)?;
  let options = runtime_modules_from_store(options, &store, &binding.definition_hash)?;
  let mut engine = DurableDagEngine::resume(store, run_id)?;
  validate_runtime_modules(engine.workflow(), &options)?;
  let descriptor_has_lifecycle = reusable_provider_descriptor(engine.workflow(), provider_id)
    .is_some_and(|descriptor| {
      matches!(
        descriptor,
        CompiledReusableInvocation::NotificationProvider {
          lifecycle: Some(_),
          ..
        }
      )
    });
  if !descriptor_has_lifecycle {
    return Ok(());
  }
  let host = options.acquire_script_host().await?;
  let lifecycle = drive_reusable_provider_lifecycle(
    &mut engine,
    run_id,
    provider_id,
    outcome,
    result,
    error,
    &options,
    &host,
  )
  .await;
  options.release_script_host(host).await;
  lifecycle
}

fn workflow_has_approval(workflow: &CompiledWorkflowDefinition) -> bool {
  workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V4
    && workflow
      .graph
      .nodes
      .iter()
      .any(|node| node.handler == "engine.approval-wait")
}

fn validate_rich_approval_authority(
  workflow: &CompiledWorkflowDefinition,
  options: &RuntimeExecutionOptions,
) -> Result<(), RuntimeExecutionError> {
  if workflow.schema_version != crate::COMPILED_MODEL_SCHEMA_VERSION_V20 {
    return Ok(());
  }
  let authority = options.reviewer_authority.as_ref().ok_or_else(|| {
    RuntimeExecutionError::InvalidConfiguration(
      "WOML_APPROVAL_AUTHORITY_UNAVAILABLE: Model v20 requires an operator-owned Reviewer Directory v1".to_string(),
    )
  })?;
  let version = authority.active_version().map_err(|error| {
    RuntimeExecutionError::InvalidConfiguration(format!("{}: {error}", error.code()))
  })?;
  for node in workflow
    .graph
    .nodes
    .iter()
    .filter(|node| node.handler == "engine.approval-wait")
  {
    let approval = workflow.approval(&node.id).ok_or_else(|| {
      RuntimeExecutionError::InvalidConfiguration(
        "Model v20 contains an invalid approval descriptor".to_string(),
      )
    })?;
    let Some(required) = approval.required_approvals else {
      continue;
    };
    authority
      .validate_requirements(&version, required, &approval.required_roles)
      .map_err(|error| {
        RuntimeExecutionError::InvalidConfiguration(format!("{}: {error}", error.code()))
      })?;
  }
  Ok(())
}

fn workflow_has_durable_wait(workflow: &CompiledWorkflowDefinition) -> bool {
  workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V17
    && workflow
      .graph
      .nodes
      .iter()
      .any(|node| node.handler == "engine.durable-wait")
}

fn workflow_has_durable_lock(workflow: &CompiledWorkflowDefinition) -> bool {
  workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V18
    && workflow
      .graph
      .locks
      .as_deref()
      .is_some_and(|locks| !locks.is_empty())
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
    WorkflowRuntimeOutcome::WaitingTime { .. } => Err(RuntimeExecutionError::Stalled(
      "workflow is durably waiting for time; use the runtime outcome API".to_string(),
    )),
    WorkflowRuntimeOutcome::WaitingSignal { .. } => Err(RuntimeExecutionError::Stalled(
      "workflow is durably waiting for an external signal; use the runtime outcome API".to_string(),
    )),
  }
}

pub fn recover_durable_runs(
  database_path: PathBuf,
) -> Result<RecoveryReport, RuntimeExecutionError> {
  let mut store = DurableEventStore::open(database_path)?;
  Ok(store.recover_interrupted_runs()?)
}

/// Atomically resolves every due timer wait in the bounded batch and resumes
/// only the runs whose wait this caller won. The caller supplies the clock via
/// RuntimeExecutionOptions, so restart recovery and tests do not depend on an
/// in-memory timer.
pub async fn resume_due_timer_waits_durable(
  database_path: PathBuf,
  options: RuntimeExecutionOptions,
  limit: usize,
) -> Result<Vec<WorkflowRuntimeOutcome>, RuntimeExecutionError> {
  resume_due_timer_waits_for_workflows_durable(database_path, options, limit, None).await
}

/// Resumes due timer waits only for workflow IDs owned by the calling runtime.
/// A shared durable store can serve multiple runtime processes, so the
/// production scheduler must never claim a wait merely because it can see it.
pub(crate) async fn resume_due_timer_waits_for_workflows_durable(
  database_path: PathBuf,
  options: RuntimeExecutionOptions,
  limit: usize,
  workflow_ids: Option<&HashSet<String>>,
) -> Result<Vec<WorkflowRuntimeOutcome>, RuntimeExecutionError> {
  let now = options.clock.now();
  let due = match workflow_ids {
    Some(workflow_ids) => DurableEventStore::open(database_path.clone())?
      .due_timer_waits_for_workflows(now, limit, workflow_ids)?,
    None => DurableEventStore::open(database_path.clone())?.due_timer_waits(now, limit)?,
  };
  let mut outcomes = Vec::with_capacity(due.len());
  for wait in due {
    let mut store = DurableEventStore::open(database_path.clone())?;
    let settlement = store.settle_timer_wait(&wait.run_id, &wait.wait_id, now)?;
    if !matches!(settlement, crate::TimerWaitSettlement::Resolved { .. }) {
      continue;
    }
    outcomes.push(
      execute_policy_run_durable(database_path.clone(), &wait.run_id, options.clone(), store)
        .await?,
    );
  }
  Ok(outcomes)
}

pub(crate) struct CorrectionRecoveryResume {
  pub run_id: String,
  pub outcome: Result<WorkflowRuntimeOutcome, RuntimeExecutionError>,
}

/// Resumes only bounded correction work owned by the calling production
/// runtime. The durable policy claim remains the single execution authority;
/// this scan is discovery, not a second scheduler or an in-memory claim.
pub(crate) async fn resume_corrections_for_workflows_durable(
  database_path: PathBuf,
  options: RuntimeExecutionOptions,
  limit: usize,
  workflow_ids: &HashSet<String>,
) -> Result<Vec<CorrectionRecoveryResume>, RuntimeExecutionError> {
  let candidates = DurableEventStore::open(database_path.clone())?
    .correction_recovery_batch_for_workflows_v1(limit, workflow_ids)?;
  let mut run_ids = candidates
    .into_iter()
    .map(|candidate| candidate.run_id)
    .collect::<Vec<_>>();
  run_ids.sort();
  run_ids.dedup();

  let mut resumed = Vec::with_capacity(run_ids.len());
  for run_id in run_ids {
    let mut store = DurableEventStore::open(database_path.clone())?;
    if store.policy_run_has_live_claim(&run_id, chrono::Utc::now())? {
      continue;
    }
    resumed.push(CorrectionRecoveryResume {
      run_id: run_id.clone(),
      outcome: execute_admitted_trigger_run_durable(
        database_path.clone(),
        &run_id,
        options.clone(),
      )
      .await,
    });
  }
  Ok(resumed)
}

pub struct ExternalSignalRuntimeOutcome {
  pub admission: crate::ExternalSignalAdmissionOutcome,
  pub continuation: Option<Result<WorkflowRuntimeOutcome, RuntimeExecutionError>>,
}

pub async fn admit_external_signal_durable(
  database_path: PathBuf,
  request: crate::ExternalSignalAdmissionRequest,
  options: RuntimeExecutionOptions,
) -> Result<ExternalSignalRuntimeOutcome, RuntimeExecutionError> {
  let mut known_secrets = options
    .resolved_secrets
    .values()
    .cloned()
    .collect::<Vec<_>>();
  if let Some(token) = options.signal_ingress_token.as_ref() {
    known_secrets.push(token.clone());
  }
  let mut store = DurableEventStore::open(database_path.clone())?;
  let admission = store.admit_external_signal_atomically(request, &known_secrets)?;
  let continuation = if admission.resumes_run() {
    Some(execute_admitted_trigger_run_durable(database_path, &admission.run_id, options).await)
  } else {
    None
  };
  Ok(ExternalSignalRuntimeOutcome {
    admission,
    continuation,
  })
}

pub(crate) struct DueSignalWaitResume {
  pub run_id: String,
  pub outcome: Result<WorkflowRuntimeOutcome, RuntimeExecutionError>,
}

pub(crate) async fn resume_due_signal_waits_for_workflows_durable(
  database_path: PathBuf,
  options: RuntimeExecutionOptions,
  limit: usize,
  workflow_ids: &HashSet<String>,
) -> Result<Vec<DueSignalWaitResume>, RuntimeExecutionError> {
  let now = options.clock.now();
  let due = DurableEventStore::open(database_path.clone())?.due_signal_waits_for_workflows(
    now,
    limit,
    workflow_ids,
  )?;
  let mut resumed = Vec::with_capacity(due.len());
  for wait in due {
    let mut store = DurableEventStore::open(database_path.clone())?;
    let settlement = store.settle_signal_wait_timeout(&wait.run_id, &wait.wait_id, now)?;
    if !matches!(
      settlement,
      crate::SignalWaitTimeoutSettlement::Continued { .. }
        | crate::SignalWaitTimeoutSettlement::Failed { .. }
    ) {
      continue;
    }
    resumed.push(DueSignalWaitResume {
      run_id: wait.run_id.clone(),
      outcome: execute_policy_run_durable(
        database_path.clone(),
        &wait.run_id,
        options.clone(),
        store,
      )
      .await,
    });
  }
  Ok(resumed)
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

pub fn resolve_rich_approval_durable(
  database_path: PathBuf,
  decision: crate::RichApprovalIdentityDecision,
) -> Result<crate::RichApprovalDecisionOutcome, RuntimeExecutionError> {
  let authority = REVIEWER_AUTHORITIES
    .get_or_init(|| RwLock::new(HashMap::new()))
    .read()
    .map_err(|_| {
      RuntimeExecutionError::InvalidConfiguration(
        "WOML_APPROVAL_AUTHORITY_UNAVAILABLE: reviewer authority is unavailable".to_string(),
      )
    })?
    .get(&database_path)
    .cloned()
    .ok_or_else(|| {
      RuntimeExecutionError::InvalidConfiguration(
        "WOML_APPROVAL_AUTHORITY_UNAVAILABLE: no reviewer authority is active for this durable store".to_string(),
      )
    })?;
  let mut store = DurableEventStore::open(database_path)?;
  Ok(store.record_rich_approval_identity_decision(&authority, decision)?)
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
  if let Some(request) = projection.rich_approval_requests.get(approval_id) {
    let binding = store.run_binding(run_id)?;
    let workflow = store.definition(&binding.definition_hash)?;
    let approval = workflow.approval(approval_id).ok_or_else(|| {
      RuntimeExecutionError::Stalled(
        "rich approval timeout references an unknown approval definition".to_string(),
      )
    })?;
    let deadline = approval
      .timeout_ms
      .ok_or(DurableStoreError::ApprovalHasNoDeadline)
      .and_then(|milliseconds| {
        i64::try_from(milliseconds)
          .map(chrono::Duration::milliseconds)
          .map(|duration| request.requested_at + duration)
          .map_err(|_| {
            DurableStoreError::Contract(
              "Rich approval timeout exceeds the durable clock range.".to_string(),
            )
          })
      })?;
    if request.status != crate::RichApprovalRequestStatus::Waiting {
      return Ok(ApprovalTimeoutSettlement {
        status: crate::ApprovalTimeoutSettlementStatus::AlreadyResolved,
        run_id: run_id.to_string(),
        approval_id: approval_id.to_string(),
        request_id: request.request_id.clone(),
        resolution: None,
        settled_at: request.resolved_at,
      });
    }
    if now < deadline {
      return Ok(ApprovalTimeoutSettlement {
        status: crate::ApprovalTimeoutSettlementStatus::NotDue,
        run_id: run_id.to_string(),
        approval_id: approval_id.to_string(),
        request_id: request.request_id.clone(),
        resolution: None,
        settled_at: None,
      });
    }
    let outcome = store.settle_rich_approval_without_vote(
      run_id,
      &request.request_id,
      &request.review_digest,
      RichApprovalResolutionReason::Timeout,
      now,
    )?;
    return Ok(ApprovalTimeoutSettlement {
      status: if outcome.status == crate::RichApprovalDecisionStatus::AlreadyResolved {
        crate::ApprovalTimeoutSettlementStatus::AlreadyResolved
      } else {
        crate::ApprovalTimeoutSettlementStatus::Settled
      },
      run_id: run_id.to_string(),
      approval_id: approval_id.to_string(),
      request_id: request.request_id.clone(),
      resolution: None,
      settled_at: Some(now),
    });
  }
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
  engine: &mut E,
  trigger: Map<String, Value>,
  options: RuntimeExecutionOptions,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  let mut span = crate::performance::PerformanceSpan::new("runtime", "runtime.run_engine");
  span.count("nodes", engine.workflow().graph.nodes.len());
  let result = execute_with_engine_unprofiled(engine, trigger, options.clone()).await;
  match &result {
    Ok(outcome) => {
      let run_id = runtime_outcome_run_id(outcome);
      report_telemetry_history(engine, run_id, &options);
      span.run_id(run_id.to_string());
      span.succeed();
    }
    Err(error) => {
      if let Some(events) = runtime_error_events(error) {
        report_telemetry_events(&engine.workflow().workflow_id, events, &options);
      }
    }
  }
  result
}

async fn execute_with_engine_unprofiled<E: RuntimeDagEngine>(
  engine: &mut E,
  trigger: Map<String, Value>,
  options: RuntimeExecutionOptions,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  if options.script_timeout_ms == 0 {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "script_timeout_ms must be greater than zero".to_string(),
    ));
  }
  validate_runtime_modules(engine.workflow(), &options)?;

  execute_runtime(engine, trigger, &options).await
}

fn sha256_identity(content: &str) -> String {
  format!("sha256:{:x}", Sha256::digest(content.as_bytes()))
}

fn reusable_lifecycle_artifact_name(action_id: &str) -> String {
  format!(
    "__woml_lifecycle__{}",
    sha256_identity(action_id).trim_start_matches("sha256:")
  )
}

fn validate_runtime_modules(
  workflow: &CompiledWorkflowDefinition,
  options: &RuntimeExecutionOptions,
) -> Result<(), RuntimeExecutionError> {
  let module_artifacts = options
    .runtime_modules
    .iter()
    .filter(|artifact| !artifact.name.starts_with("__woml_"))
    .collect::<Vec<_>>();
  let provider_artifacts = options
    .runtime_modules
    .iter()
    .filter(|artifact| artifact.name.starts_with("__woml_provider__"))
    .collect::<Vec<_>>();
  let reusable_step_artifacts = options
    .runtime_modules
    .iter()
    .filter(|artifact| artifact.name.starts_with("__woml_reusable__"))
    .collect::<Vec<_>>();
  let reusable_lifecycle_artifacts = options
    .runtime_modules
    .iter()
    .filter(|artifact| artifact.name.starts_with("__woml_lifecycle__"))
    .collect::<Vec<_>>();
  let runtime_bindings = workflow
    .module_runtime
    .as_ref()
    .map_or(&[][..], |runtime| runtime.modules.as_slice());
  if runtime_bindings.len() != module_artifacts.len() {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "the runtime module artifacts do not match Model v9".to_string(),
    ));
  }
  let mut total_bytes = 0usize;
  for (binding, artifact) in runtime_bindings.iter().zip(module_artifacts) {
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
  let provider_descriptors = workflow
    .reusable_definitions
    .iter()
    .flatten()
    .filter_map(|definition| match definition {
      crate::model::CompiledReusableInvocation::NotificationProvider {
        script_artifact_id, ..
      } => Some(script_artifact_id.as_str()),
      _ => None,
    })
    .collect::<HashSet<_>>();
  let mut provider_ids = HashSet::new();
  for artifact in provider_artifacts {
    let artifact_id = artifact.name.trim_start_matches("__woml_provider__");
    total_bytes = total_bytes
      .checked_add(artifact.bundle.len())
      .and_then(|value| value.checked_add(artifact.source_map.len()))
      .ok_or_else(|| {
        RuntimeExecutionError::InvalidConfiguration(
          "custom-provider artifact byte count overflowed".to_string(),
        )
      })?;
    if !provider_descriptors.contains(artifact_id)
      || !provider_ids.insert(artifact_id)
      || !artifact.exports.is_empty()
      || !artifact.source_map.is_empty()
      || sha256_identity(&artifact.bundle) != artifact.bundle_digest
      || sha256_identity("") != artifact.source_map_digest
      || artifact.bundle.len() > crate::durable::MAX_MODULE_ARTIFACT_BYTES
    {
      return Err(RuntimeExecutionError::InvalidConfiguration(format!(
        "custom-provider artifact {artifact_id:?} failed its immutable identity or size check"
      )));
    }
  }
  if provider_ids.len() != provider_descriptors.len() {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "the custom-provider runtime artifacts do not match Model v14".to_string(),
    ));
  }
  let reusable_step_descriptors = workflow
    .reusable_definitions
    .iter()
    .flatten()
    .filter_map(|definition| match definition {
      crate::model::CompiledReusableInvocation::Step {
        script_artifact_id, ..
      } => Some(script_artifact_id.as_str()),
      _ => None,
    })
    .collect::<HashSet<_>>();
  let mut reusable_step_ids = HashSet::new();
  for artifact in reusable_step_artifacts {
    let artifact_id = artifact.name.trim_start_matches("__woml_reusable__");
    total_bytes = total_bytes
      .checked_add(artifact.bundle.len())
      .and_then(|value| value.checked_add(artifact.source_map.len()))
      .ok_or_else(|| {
        RuntimeExecutionError::InvalidConfiguration(
          "custom-step artifact byte count overflowed".to_string(),
        )
      })?;
    if !reusable_step_descriptors.contains(artifact_id)
      || !reusable_step_ids.insert(artifact_id)
      || !artifact.exports.is_empty()
      || !artifact.source_map.is_empty()
      || sha256_identity(&artifact.bundle) != artifact.bundle_digest
      || sha256_identity("") != artifact.source_map_digest
      || artifact.bundle.len() > crate::durable::MAX_MODULE_ARTIFACT_BYTES
    {
      return Err(RuntimeExecutionError::InvalidConfiguration(format!(
        "custom-step artifact {artifact_id:?} failed its immutable identity or size check"
      )));
    }
  }
  if reusable_step_ids.len() != reusable_step_descriptors.len() {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "the custom-step runtime artifacts do not match Model v14".to_string(),
    ));
  }
  let reusable_lifecycle_names = workflow
    .reusable_definitions
    .iter()
    .flatten()
    .flat_map(|definition| match definition {
      crate::model::CompiledReusableInvocation::Step { lifecycle, .. }
      | crate::model::CompiledReusableInvocation::NotificationProvider { lifecycle, .. } => {
        lifecycle
          .iter()
          .flat_map(|lifecycle| {
            lifecycle
              .on_success
              .iter()
              .chain(&lifecycle.on_error)
              .chain(&lifecycle.on_complete)
              .flat_map(|actions| actions.iter())
              .map(|action_id| reusable_lifecycle_artifact_name(action_id))
              .collect::<Vec<_>>()
          })
          .collect::<Vec<_>>()
      }
    })
    .collect::<HashSet<_>>();
  let mut found_lifecycle_names = HashSet::new();
  for artifact in reusable_lifecycle_artifacts {
    total_bytes = total_bytes
      .checked_add(artifact.bundle.len())
      .and_then(|value| value.checked_add(artifact.source_map.len()))
      .ok_or_else(|| {
        RuntimeExecutionError::InvalidConfiguration(
          "reusable lifecycle artifact byte count overflowed".to_string(),
        )
      })?;
    if !reusable_lifecycle_names.contains(&artifact.name)
      || !found_lifecycle_names.insert(artifact.name.as_str())
      || !artifact.exports.is_empty()
      || !artifact.source_map.is_empty()
      || sha256_identity(&artifact.bundle) != artifact.bundle_digest
      || sha256_identity("") != artifact.source_map_digest
      || artifact.bundle.len() > crate::durable::MAX_MODULE_ARTIFACT_BYTES
    {
      return Err(RuntimeExecutionError::InvalidConfiguration(format!(
        "reusable lifecycle artifact {:?} failed its immutable identity or size check",
        artifact.name
      )));
    }
  }
  if found_lifecycle_names.len() != reusable_lifecycle_names.len() {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "the reusable lifecycle runtime artifacts do not match Model v14".to_string(),
    ));
  }
  if total_bytes > crate::durable::MAX_MODULE_ARTIFACT_SET_BYTES {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "runtime module artifacts exceed the per-definition cache limit".to_string(),
    ));
  }
  Ok(())
}

async fn resume_with_engine<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  options: RuntimeExecutionOptions,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  let mut span = crate::performance::PerformanceSpan::new("runtime", "runtime.resume_engine");
  span.run_id(run_id.to_string());
  span.count("nodes", engine.workflow().graph.nodes.len());
  let result = resume_with_engine_unprofiled(engine, run_id, options.clone()).await;
  report_telemetry_history(engine, run_id, &options);
  settle_operational_run(engine, run_id, &options);
  if result.is_ok() {
    span.succeed();
  }
  result
}

fn report_telemetry_history<E: RuntimeDagEngine>(
  engine: &E,
  run_id: &str,
  options: &RuntimeExecutionOptions,
) {
  if options.telemetry_reporter.is_none() || options.telemetry_context.is_none() {
    return;
  }
  let Ok(events) = engine.events(run_id) else {
    return;
  };
  report_telemetry_events(&engine.workflow().workflow_id, &events, options);
}

fn report_telemetry_events(
  workflow_id: &str,
  events: &[RunEvent],
  options: &RuntimeExecutionOptions,
) {
  let (Some(reporter), Some(context)) = (
    options.telemetry_reporter.as_ref(),
    options.telemetry_context.as_ref(),
  ) else {
    return;
  };
  for signal in crate::project_telemetry_signals(events, workflow_id, context) {
    reporter(signal);
  }
}

fn runtime_error_events(error: &RuntimeExecutionError) -> Option<&[RunEvent]> {
  match error {
    RuntimeExecutionError::RunFailed(details) => Some(&details.events),
    RuntimeExecutionError::BranchFailed(details) => Some(&details.events),
    RuntimeExecutionError::ParallelFailed(details) => Some(&details.events),
    RuntimeExecutionError::ApprovalFailed(details) => Some(&details.events),
    RuntimeExecutionError::NotificationFailed(details) => Some(&details.events),
    RuntimeExecutionError::RunCancelled(details) => Some(&details.events),
    _ => None,
  }
}

fn settle_operational_run<E: RuntimeDagEngine>(
  engine: &E,
  run_id: &str,
  options: &RuntimeExecutionOptions,
) {
  let Some(store) = options
    .operational_output_store
    .as_ref()
    .and_then(|store| store.active_store())
  else {
    return;
  };
  let Ok(projection) = engine.projection(run_id) else {
    return;
  };
  let status = match projection.status {
    RunStatus::Succeeded => crate::OperationalCloseStatusV1::Succeeded,
    RunStatus::Failed => crate::OperationalCloseStatusV1::Failed,
    RunStatus::Cancelled => crate::OperationalCloseStatusV1::Cancelled,
    _ => return,
  };
  let _ = store.append(crate::OperationalRecordDraftV1 {
    identity: crate::OperationalIdentityV1 {
      workflow_id: engine.workflow().workflow_id.clone(),
      run_id: run_id.to_string(),
      node_id: None,
      attempt: None,
      invocation_id: None,
      phase: crate::OperationalExecutionPhaseV1::Forward,
    },
    body: crate::OperationalBodyV1::Closed {
      status,
      reason_code: None,
    },
  });
}

fn runtime_outcome_run_id(outcome: &WorkflowRuntimeOutcome) -> &str {
  match outcome {
    WorkflowRuntimeOutcome::Succeeded { execution, .. } => &execution.run_id,
    WorkflowRuntimeOutcome::Waiting { run_id, .. } => run_id,
    WorkflowRuntimeOutcome::WaitingTime { run_id, .. } => run_id,
    WorkflowRuntimeOutcome::WaitingSignal { run_id, .. } => run_id,
  }
}

async fn resume_with_engine_unprofiled<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  options: RuntimeExecutionOptions,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  if options.script_timeout_ms == 0 {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "script_timeout_ms must be greater than zero".to_string(),
    ));
  }
  validate_runtime_modules(engine.workflow(), &options)?;
  ensure_held_locks_current(engine, run_id, &options)?;
  let projection = engine.projection(run_id)?;
  let execution_order = recorded_execution_order(&engine.events(run_id)?);
  match projection.status {
    RunStatus::Succeeded => {
      return Ok(WorkflowRuntimeOutcome::succeeded(final_result(
        engine,
        run_id,
        execution_order,
      )?));
    }
    RunStatus::Failed => return Err(resumed_failure(engine, run_id, projection)?),
    RunStatus::Cancelled => {
      return Err(cancelled_run_error(engine, run_id)?);
    }
    RunStatus::Finalizing => {
      let mut host = None;
      drive_pending_reusable_step_lifecycles(engine, run_id, &options, &mut host).await?;
      drive_workflow_lifecycle(engine, run_id, &options, &mut host).await?;
      if let Some(host) = host {
        options.release_script_host(host).await;
      }
      let projection = engine.projection(run_id)?;
      return match projection.status {
        RunStatus::Succeeded => Ok(WorkflowRuntimeOutcome::succeeded(final_result(
          engine,
          run_id,
          execution_order,
        )?)),
        RunStatus::Failed => Err(resumed_failure(engine, run_id, projection)?),
        RunStatus::Cancelled => Err(cancelled_run_error(engine, run_id)?),
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
  continue_runtime(engine, run_id, terminal_node_id, execution_order, &options).await
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
  // The runtime loop intentionally supports every control-flow and correction
  // primitive. Keep its generated future off the comparatively small test and
  // worker thread stacks; adding a new primitive must not increase the stack
  // required by workflows that never use it.
  let mut execution = Box::pin(continue_runtime_loop(
    engine,
    run_id,
    terminal_node_id,
    &mut execution_order,
    options,
    &mut host,
  ))
  .await;
  if execution.is_err() && engine.projection(run_id)?.status == RunStatus::Cancelling {
    settle_cancelling_run(engine, run_id, options.clock.now(), options)?;
    execution = Err(cancelled_run_error(engine, run_id)?);
  }
  let lifecycle = drive_workflow_lifecycle(engine, run_id, options, &mut host).await;
  if let Some(host) = host {
    options.release_script_host(host).await;
  }
  lifecycle?;
  match execution {
    Ok(WorkflowRuntimeOutcome::Succeeded { execution, .. }) => Ok(
      WorkflowRuntimeOutcome::succeeded(final_result(engine, run_id, execution.execution_order)?),
    ),
    Ok(
      waiting @ (WorkflowRuntimeOutcome::Waiting { .. }
      | WorkflowRuntimeOutcome::WaitingTime { .. }
      | WorkflowRuntimeOutcome::WaitingSignal { .. }),
    ) => Ok(waiting),
    Err(RuntimeExecutionError::RunCancelled(_)) => Err(cancelled_run_error(engine, run_id)?),
    Err(error) => Err(error),
  }
}

enum DurableWaitNodeProgress {
  Resolved,
  Waiting(WorkflowRuntimeOutcome),
}

async fn register_durable_wait_node<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  node_id: &str,
  options: &RuntimeExecutionOptions,
) -> Result<DurableWaitNodeProgress, RuntimeExecutionError> {
  let descriptor = engine
    .workflow()
    .durable_wait(node_id)
    .cloned()
    .ok_or_else(|| {
      RuntimeExecutionError::Stalled(format!(
        "durable wait node {node_id:?} has no Model v17 descriptor"
      ))
    })?;
  let registered_at = options.clock.now();
  let (kind, deadline, signals, correlation_digest) = match &descriptor {
    crate::CompiledWait::Duration { duration_ms, .. } => {
      let duration = chrono::Duration::milliseconds(i64::try_from(*duration_ms).map_err(|_| {
        RuntimeExecutionError::Stalled(
          "durable wait duration exceeds the runtime clock".to_string(),
        )
      })?);
      (
        crate::DurableWaitKind::Duration,
        Some(registered_at + duration),
        None,
        None,
      )
    }
    crate::CompiledWait::Until { at, .. } => {
      let value = match at {
        ValueExpression::Literal { value } => value.clone(),
        ValueExpression::ContextReference { .. } => {
          resolve_context_reference(at, &engine.projection(run_id)?.context).map_err(|error| {
            RuntimeExecutionError::Stalled(format!(
              "durable wait {node_id:?} cannot resolve context.{}",
              error.path.join(".")
            ))
          })?
        }
        _ => {
          return Err(RuntimeExecutionError::Stalled(format!(
            "durable wait {node_id:?} has an invalid until expression"
          )))
        }
      };
      let value = value.as_str().ok_or_else(|| {
        RuntimeExecutionError::Stalled(format!(
          "durable wait {node_id:?} resolved at to a non-string value"
        ))
      })?;
      let deadline = chrono::DateTime::parse_from_rfc3339(value)
        .map_err(|_| {
          RuntimeExecutionError::Stalled(format!(
            "durable wait {node_id:?} resolved at to an invalid RFC 3339 instant"
          ))
        })?
        .with_timezone(&chrono::Utc);
      (crate::DurableWaitKind::Until, Some(deadline), None, None)
    }
    crate::CompiledWait::Signal {
      signals,
      correlation,
      timeout_ms,
      ..
    } => {
      let correlation = match correlation {
        ValueExpression::Literal { value } => value.clone(),
        ValueExpression::ContextReference { .. } => {
          resolve_context_reference(correlation, &engine.projection(run_id)?.context).map_err(
            |error| {
              RuntimeExecutionError::Stalled(format!(
                "signal wait {node_id:?} cannot resolve context.{}",
                error.path.join(".")
              ))
            },
          )?
        }
        _ => {
          return Err(RuntimeExecutionError::Stalled(format!(
            "signal wait {node_id:?} has an invalid correlation expression"
          )))
        }
      };
      let correlation = correlation.as_str().ok_or_else(|| {
        RuntimeExecutionError::Stalled(format!(
          "signal wait {node_id:?} resolved correlation to a non-string value"
        ))
      })?;
      let deadline = timeout_ms
        .map(|timeout_ms| {
          i64::try_from(timeout_ms)
            .map(chrono::Duration::milliseconds)
            .map(|duration| registered_at + duration)
        })
        .transpose()
        .map_err(|_| {
          RuntimeExecutionError::Stalled(
            "signal wait timeout exceeds the runtime clock".to_string(),
          )
        })?;
      (
        crate::DurableWaitKind::Signal,
        deadline,
        Some(signals.iter().map(|signal| signal.signal.clone()).collect()),
        Some(format!(
          "sha256:{}",
          hex::encode(Sha256::digest(correlation.as_bytes()))
        )),
      )
    }
  };
  engine.register_durable_wait(
    run_id,
    registered_at,
    crate::WaitRegisteredData {
      wait_id: descriptor.wait_id().to_string(),
      node_id: descriptor.node_id().to_string(),
      kind,
      registered_at,
      deadline,
      signals,
      correlation_digest,
    },
  )?;
  let projection = engine.projection(run_id)?;
  if projection.status == RunStatus::Running
    && projection.context.steps.contains_key(descriptor.wait_id())
  {
    return Ok(DurableWaitNodeProgress::Resolved);
  }
  if deadline.is_some_and(|deadline| deadline <= registered_at)
    && kind != crate::DurableWaitKind::Signal
    && matches!(
      engine.settle_timer_wait(run_id, descriptor.wait_id(), registered_at)?,
      crate::TimerWaitSettlement::Resolved { .. }
        | crate::TimerWaitSettlement::AlreadyResolved { .. }
    )
  {
    return Ok(DurableWaitNodeProgress::Resolved);
  }
  options.suspend_policy_execution_slot().await?;
  if kind == crate::DurableWaitKind::Signal {
    return Ok(DurableWaitNodeProgress::Waiting(
      WorkflowRuntimeOutcome::WaitingSignal {
        contract: RUNTIME_OUTCOME_CONTRACT,
        version: RUNTIME_OUTCOME_VERSION,
        workflow_id: engine.workflow().workflow_id.clone(),
        run_id: run_id.to_string(),
        wait: WaitingWorkflowSignal {
          wait_id: descriptor.wait_id().to_string(),
          node_id: descriptor.node_id().to_string(),
          registered_at,
          signals: match descriptor {
            crate::CompiledWait::Signal { signals, .. } => {
              signals.iter().map(|signal| signal.signal.clone()).collect()
            }
            _ => Vec::new(),
          },
          deadline,
        },
      },
    ));
  }
  Ok(DurableWaitNodeProgress::Waiting(
    WorkflowRuntimeOutcome::WaitingTime {
      contract: RUNTIME_OUTCOME_CONTRACT,
      version: RUNTIME_OUTCOME_VERSION,
      workflow_id: engine.workflow().workflow_id.clone(),
      run_id: run_id.to_string(),
      wait: WaitingWorkflowTimer {
        wait_id: descriptor.wait_id().to_string(),
        node_id: descriptor.node_id().to_string(),
        kind,
        registered_at,
        deadline: deadline.expect("timer waits have a deadline"),
      },
    },
  ))
}

fn ensure_held_locks_current<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  options: &RuntimeExecutionOptions,
) -> Result<(), RuntimeExecutionError> {
  let projection = engine.projection(run_id)?;
  let mut lost = projection
    .locks
    .values()
    .filter(|lock| lock.status == crate::LockStatus::LeaseLost)
    .map(|lock| lock.lock_id.clone())
    .collect::<Vec<_>>();
  lost.extend(projection.for_each.values().flat_map(|loop_state| {
    loop_state.executions.values().flat_map(|execution| {
      execution
        .locks
        .values()
        .filter(|lock| lock.status == crate::LockStatus::LeaseLost)
        .map(|lock| lock.lock_id.clone())
    })
  }));
  if let Some(lock_id) = lost.first() {
    if projection.business_outcome.is_some() || projection.status == RunStatus::Cancelling {
      return Ok(());
    }
    engine.decide_run_failed(
      run_id,
      LifecycleFailure {
        kind: LifecycleFailureKind::Interrupted,
        code: "WOML_LOCK_LEASE_LOST".to_string(),
        message: format!(
          "Lock {lock_id:?} expired before its protected continuation could finish."
        ),
      },
      options.clock.now(),
    )?;
    return Err(resumed_failure(engine, run_id, engine.projection(run_id)?)?);
  }
  let held = projection
    .locks
    .values()
    .filter(|lock| lock.status == crate::LockStatus::Held)
    .map(|lock| lock.lock_id.clone())
    .collect::<Vec<_>>();
  for lock_id in held {
    let descriptor = engine
      .workflow()
      .durable_lock(&lock_id)
      .cloned()
      .ok_or_else(|| {
        RuntimeExecutionError::Stalled("A held lease has no compiled lock descriptor.".to_string())
      })?;
    match engine.renew_lock_lease(run_id, &lock_id, options.clock.now())? {
      crate::LockLeaseMaintenance::Renewed => {
        options.ensure_lock_renewal(run_id, &lock_id, descriptor.lease_ms);
      }
      crate::LockLeaseMaintenance::Lost | crate::LockLeaseMaintenance::NotHeld => {
        options.stop_lock_renewal(run_id, &lock_id);
        let projection = engine.projection(run_id)?;
        if projection.business_outcome.is_none() {
          engine.decide_run_failed(
            run_id,
            LifecycleFailure {
              kind: LifecycleFailureKind::Interrupted,
              code: "WOML_LOCK_LEASE_LOST".to_string(),
              message: format!(
                "Lock {lock_id:?} expired before WOML could renew its durable lease."
              ),
            },
            options.clock.now(),
          )?;
        }
        return Err(resumed_failure(engine, run_id, engine.projection(run_id)?)?);
      }
    }
  }
  Ok(())
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
  provider_id: Option<&'a str>,
  credentials: BTreeMap<String, String>,
  message: &'a ValueExpression,
  template_name: Option<&'a str>,
  language: Option<&'a str>,
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
      let credentials_expression = fields.get("credentials").ok_or_else(|| {
        RuntimeExecutionError::InvalidConfiguration(
          "lifecycle notification credentials are unavailable".to_string(),
        )
      })?;
      let ValueExpression::Object {
        fields: credential_fields,
      } = credentials_expression
      else {
        return Err(RuntimeExecutionError::InvalidConfiguration(
          "lifecycle notification credentials are invalid".to_string(),
        ));
      };
      let provider = literal("provider").ok_or_else(|| {
        RuntimeExecutionError::InvalidConfiguration(
          "lifecycle notification provider is unavailable".to_string(),
        )
      })?;
      let credentials = credential_fields
        .iter()
        .map(|(name, expression)| match expression {
          ValueExpression::SecretReference { name: secret } => Ok((name.clone(), secret.clone())),
          ValueExpression::Literal { value }
            if provider == "whatsapp" && name == "phoneNumberId" =>
          {
            value
              .as_str()
              .map(|value| (name.clone(), value.to_string()))
              .ok_or_else(|| {
                RuntimeExecutionError::InvalidConfiguration(
                  "WhatsApp phoneNumberId must be a literal string".to_string(),
                )
              })
          }
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
        provider,
        destination: literal("destination").ok_or_else(|| {
          RuntimeExecutionError::InvalidConfiguration(
            "lifecycle notification destination is unavailable".to_string(),
          )
        })?,
        provider_id: literal("providerId"),
        credentials,
        message: fields.get("message").ok_or_else(|| {
          RuntimeExecutionError::InvalidConfiguration(
            "lifecycle notification message is unavailable".to_string(),
          )
        })?,
        template_name: literal("templateName"),
        language: literal("language"),
      })
    })
    .collect()
}

fn reusable_provider_descriptor<'a>(
  workflow: &'a CompiledWorkflowDefinition,
  provider_id: &str,
) -> Option<&'a CompiledReusableInvocation> {
  workflow
    .reusable_definitions
    .iter()
    .flatten()
    .find(|definition| {
      matches!(definition, CompiledReusableInvocation::NotificationProvider { provider_id: value, .. } if value == provider_id)
    })
}

fn reusable_step_descriptor<'a>(
  workflow: &'a CompiledWorkflowDefinition,
  node_id: &str,
) -> Option<&'a CompiledReusableInvocation> {
  workflow
    .reusable_definitions
    .iter()
    .flatten()
    .find(|definition| {
      matches!(definition, CompiledReusableInvocation::Step { node_id: value, .. } if value == node_id)
    })
}

fn resolved_reusable_props(
  descriptor: &CompiledReusableInvocation,
  context: &WorkflowContext,
  secrets: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, Value>, RuntimeExecutionError> {
  resolved_reusable_props_with_iteration(descriptor, context, secrets, None)
}

fn resolved_reusable_props_with_iteration(
  descriptor: &CompiledReusableInvocation,
  context: &WorkflowContext,
  secrets: &BTreeMap<String, String>,
  iteration: Option<(&Value, u32, u32)>,
) -> Result<BTreeMap<String, Value>, RuntimeExecutionError> {
  let props = match descriptor {
    CompiledReusableInvocation::Step { props, .. }
    | CompiledReusableInvocation::NotificationProvider { props, .. } => props,
  };
  props
    .iter()
    .map(|prop| {
      let value = match &prop.expression {
        crate::model::CompiledReusablePropExpression::Literal { value } => {
          Value::String(value.clone())
        }
        crate::model::CompiledReusablePropExpression::Context { path } => {
          if let Some((item, index, total)) = iteration {
            if path == "item" || path.starts_with("item.") {
              let mut value = item;
              for segment in path.split('.').skip(1) {
                value = value.get(segment).ok_or_else(|| {
                  RuntimeExecutionError::InvalidConfiguration(format!(
                    "Reusable prop {:?} is unavailable on context.item.",
                    prop.name
                  ))
                })?;
              }
              return Ok((prop.binding_name.clone(), value.clone()));
            }
            if path == "iteration.index" || path == "iteration.total" {
              return Ok((
                prop.binding_name.clone(),
                Value::from(if path.ends_with("index") {
                  index
                } else {
                  total
                }),
              ));
            }
          }
          let path = path
            .split('.')
            .map(|segment| {
              if segment == "payload" {
                "trigger".to_string()
              } else {
                segment.to_string()
              }
            })
            .collect::<Vec<_>>();
          resolve_context_reference(&ValueExpression::ContextReference { path }, context).map_err(
            |_| {
              RuntimeExecutionError::InvalidConfiguration(format!(
                "Reusable prop {:?} is unavailable at this invocation.",
                prop.name
              ))
            },
          )?
        }
        crate::model::CompiledReusablePropExpression::Secret { name } => Value::String(
          secrets
            .get(name)
            .filter(|value| !value.is_empty())
            .cloned()
            .ok_or_else(|| {
              RuntimeExecutionError::InvalidConfiguration(format!(
                "Required reusable secret {name:?} is unavailable."
              ))
            })?,
        ),
      };
      Ok((prop.binding_name.clone(), value))
    })
    .collect()
}

fn reusable_step_binding_for_iteration(
  workflow: &CompiledWorkflowDefinition,
  node_id: &str,
  context: &WorkflowContext,
  item: &Value,
  index: u32,
  total: u32,
  secrets: &BTreeMap<String, String>,
) -> Result<Option<ReusableScriptBindingV3>, RuntimeExecutionError> {
  let Some(descriptor) = reusable_step_descriptor(workflow, node_id) else {
    return Ok(None);
  };
  let CompiledReusableInvocation::Step {
    invocation_id,
    alias,
    definition_digest,
    source,
    ..
  } = descriptor
  else {
    unreachable!()
  };
  Ok(Some(ReusableScriptBindingV3 {
    profile: "woml.reusable-script-binding/v3".to_string(),
    invocation_id: invocation_id.clone(),
    definition: ReusableDefinitionBindingV3 {
      kind: ReusableDefinitionKind::Step,
      alias: alias.clone(),
      digest: definition_digest.clone(),
      source: source.clone(),
    },
    props: resolved_reusable_props_with_iteration(
      descriptor,
      context,
      secrets,
      Some((item, index, total)),
    )?,
  }))
}

fn reusable_step_binding(
  workflow: &CompiledWorkflowDefinition,
  node_id: &str,
  context: &WorkflowContext,
  secrets: &BTreeMap<String, String>,
) -> Result<Option<ReusableScriptBindingV3>, RuntimeExecutionError> {
  let Some(descriptor) = reusable_step_descriptor(workflow, node_id) else {
    return Ok(None);
  };
  let CompiledReusableInvocation::Step {
    invocation_id,
    alias,
    definition_digest,
    source,
    ..
  } = descriptor
  else {
    unreachable!()
  };
  Ok(Some(ReusableScriptBindingV3 {
    profile: "woml.reusable-script-binding/v3".to_string(),
    invocation_id: invocation_id.clone(),
    definition: ReusableDefinitionBindingV3 {
      kind: ReusableDefinitionKind::Step,
      alias: alias.clone(),
      digest: definition_digest.clone(),
      source: source.clone(),
    },
    props: resolved_reusable_props(descriptor, context, secrets)?,
  }))
}

fn reusable_definition_binding(
  descriptor: &CompiledReusableInvocation,
  context: &WorkflowContext,
  secrets: &BTreeMap<String, String>,
) -> Result<ReusableScriptBindingV3, RuntimeExecutionError> {
  let (invocation_id, kind, alias, definition_digest, source) = match descriptor {
    CompiledReusableInvocation::Step {
      invocation_id,
      alias,
      definition_digest,
      source,
      ..
    } => (
      invocation_id,
      ReusableDefinitionKind::Step,
      alias,
      definition_digest,
      source,
    ),
    CompiledReusableInvocation::NotificationProvider {
      provider_id,
      alias,
      definition_digest,
      source,
      ..
    } => (
      provider_id,
      ReusableDefinitionKind::NotificationProvider,
      alias,
      definition_digest,
      source,
    ),
  };
  Ok(ReusableScriptBindingV3 {
    profile: "woml.reusable-script-binding/v3".to_string(),
    invocation_id: invocation_id.clone(),
    definition: ReusableDefinitionBindingV3 {
      kind,
      alias: alias.clone(),
      digest: definition_digest.clone(),
      source: source.clone(),
    },
    props: resolved_reusable_props(descriptor, context, secrets)?,
  })
}

fn reusable_definition_binding_for_iteration(
  descriptor: &CompiledReusableInvocation,
  context: &WorkflowContext,
  item: &Value,
  index: u32,
  total: u32,
  secrets: &BTreeMap<String, String>,
) -> Result<ReusableScriptBindingV3, RuntimeExecutionError> {
  let (invocation_id, kind, alias, definition_digest, source) = match descriptor {
    CompiledReusableInvocation::Step {
      invocation_id,
      alias,
      definition_digest,
      source,
      ..
    } => (
      invocation_id,
      ReusableDefinitionKind::Step,
      alias,
      definition_digest,
      source,
    ),
    CompiledReusableInvocation::NotificationProvider {
      provider_id,
      alias,
      definition_digest,
      source,
      ..
    } => (
      provider_id,
      ReusableDefinitionKind::NotificationProvider,
      alias,
      definition_digest,
      source,
    ),
  };
  Ok(ReusableScriptBindingV3 {
    profile: "woml.reusable-script-binding/v3".to_string(),
    invocation_id: invocation_id.clone(),
    definition: ReusableDefinitionBindingV3 {
      kind,
      alias: alias.clone(),
      digest: definition_digest.clone(),
      source: source.clone(),
    },
    props: resolved_reusable_props_with_iteration(
      descriptor,
      context,
      secrets,
      Some((item, index, total)),
    )?,
  })
}

fn resolved_reusable_descriptor_secrets(
  descriptor: &CompiledReusableInvocation,
  options: &RuntimeExecutionOptions,
) -> Result<BTreeMap<String, String>, RuntimeExecutionError> {
  let props = match descriptor {
    CompiledReusableInvocation::Step { props, .. }
    | CompiledReusableInvocation::NotificationProvider { props, .. } => props,
  };
  props
    .iter()
    .filter_map(|prop| match &prop.expression {
      crate::model::CompiledReusablePropExpression::Secret { name } => Some(name),
      _ => None,
    })
    .map(|name| {
      options
        .resolved_secrets
        .get(name)
        .filter(|value| !value.is_empty())
        .cloned()
        .map(|value| (name.clone(), value))
        .ok_or_else(|| {
          RuntimeExecutionError::InvalidConfiguration(format!(
            "reusable lifecycle requires unresolved secret {name:?}"
          ))
        })
    })
    .collect()
}

async fn drive_reusable_lifecycle<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  descriptor: &CompiledReusableInvocation,
  outcome: ReusableInvocationOutcome,
  result: Option<Value>,
  error: Option<ReusableLifecycleErrorV1>,
  options: &RuntimeExecutionOptions,
  host: &ScriptHostClient,
) -> Result<(), RuntimeExecutionError> {
  let (invocation_id, definition_digest, lifecycle) = match descriptor {
    CompiledReusableInvocation::Step {
      invocation_id,
      definition_digest,
      lifecycle,
      ..
    } => (invocation_id, definition_digest, lifecycle),
    CompiledReusableInvocation::NotificationProvider {
      provider_id,
      definition_digest,
      lifecycle,
      ..
    } => (provider_id, definition_digest, lifecycle),
  };
  let Some(lifecycle) = lifecycle else {
    return Ok(());
  };

  let hooks = match outcome {
    ReusableInvocationOutcome::Succeeded => vec![
      (
        ProtocolReusableLifecycleHook::OnSuccess,
        crate::ReusableLifecycleHook::OnSuccess,
        lifecycle.on_success.clone().unwrap_or_default(),
      ),
      (
        ProtocolReusableLifecycleHook::OnComplete,
        crate::ReusableLifecycleHook::OnComplete,
        lifecycle.on_complete.clone().unwrap_or_default(),
      ),
    ],
    ReusableInvocationOutcome::Failed => vec![
      (
        ProtocolReusableLifecycleHook::OnError,
        crate::ReusableLifecycleHook::OnError,
        lifecycle.on_error.clone().unwrap_or_default(),
      ),
      (
        ProtocolReusableLifecycleHook::OnComplete,
        crate::ReusableLifecycleHook::OnComplete,
        lifecycle.on_complete.clone().unwrap_or_default(),
      ),
    ],
    ReusableInvocationOutcome::Cancelled => vec![(
      ProtocolReusableLifecycleHook::OnComplete,
      crate::ReusableLifecycleHook::OnComplete,
      lifecycle.on_complete.clone().unwrap_or_default(),
    )],
  };
  let context = engine.projection(run_id)?.context;
  let secrets = resolved_reusable_descriptor_secrets(descriptor, options)?;
  let reusable = reusable_definition_binding(descriptor, &context, &secrets)?;
  let modules = options
    .runtime_modules
    .iter()
    .filter(|module| !module.name.starts_with("__woml_"))
    .map(|module| RuntimeModuleBinding {
      name: module.name.clone(),
      bundle_digest: module.bundle_digest.clone(),
      exports: module.exports.clone(),
    })
    .collect::<Vec<_>>();

  for (protocol_hook, event_hook, actions) in hooks {
    if actions.is_empty() {
      continue;
    }
    let key = format!("{}:{:?}", invocation_id, event_hook);
    let projected = engine
      .projection(run_id)?
      .reusable_lifecycle_hooks
      .get(&key)
      .cloned();
    if projected.as_ref().is_some_and(|hook| {
      matches!(
        hook.status,
        ReusableLifecycleStatus::Completed | ReusableLifecycleStatus::CompletedWithWarnings
      )
    }) {
      continue;
    }
    if projected.is_none() {
      engine.append_payload(
        run_id,
        RunEventPayload::ReusableLifecycleRequested(crate::ReusableLifecycleRequestedData {
          invocation_id: invocation_id.clone(),
          definition_digest: definition_digest.clone(),
          hook: event_hook,
        }),
      )?;
    }
    let binding = ReusableLifecycleBindingV1 {
      hook: protocol_hook,
      outcome,
      result: result.clone(),
      error: error.clone(),
    };
    for action_id in actions {
      let projected = engine
        .projection(run_id)?
        .reusable_lifecycle_hooks
        .get(&key)
        .cloned()
        .ok_or_else(|| {
          RuntimeExecutionError::Stalled("reusable lifecycle request was not projected".to_string())
        })?;
      if projected.completed_action_ids.contains(&action_id) {
        continue;
      }
      if projected.active_action_id.as_deref() == Some(action_id.as_str()) {
        engine.append_payload(
          run_id,
          RunEventPayload::ReusableLifecycleActionFailed(
            crate::ReusableLifecycleActionFailedData {
              invocation_id: invocation_id.clone(),
              definition_digest: definition_digest.clone(),
              hook: event_hook,
              action_id: action_id.clone(),
              outcome: crate::ReusableLifecycleOutcome::Failed,
              warning_code: "WOML_REUSABLE_LIFECYCLE_INTERRUPTED".to_string(),
            },
          ),
        )?;
        continue;
      }
      let artifact_name = reusable_lifecycle_artifact_name(&action_id);
      let source = options
        .runtime_modules
        .iter()
        .find(|artifact| artifact.name == artifact_name)
        .map(|artifact| artifact.bundle.clone())
        .ok_or_else(|| {
          RuntimeExecutionError::InvalidConfiguration(format!(
            "reusable lifecycle artifact for {action_id:?} is unavailable"
          ))
        })?;
      engine.append_payload(
        run_id,
        RunEventPayload::ReusableLifecycleActionStarted(
          crate::ReusableLifecycleActionStartedData {
            invocation_id: invocation_id.clone(),
            definition_digest: definition_digest.clone(),
            hook: event_hook,
            action_id: action_id.clone(),
          },
        ),
      )?;
      let host_invocation_id = generated_id("rlcinv");
      let idempotency_key =
        step_effect_idempotency_key(run_id, engine.definition_hash(), &action_id);
      let request = ExecuteMessage::reusable_lifecycle_script_with_modules(
        &host_invocation_id,
        run_id,
        &action_id,
        ScriptAttempt::new(1, 1, &idempotency_key).map_err(RuntimeExecutionError::Stalled)?,
        options.script_timeout_ms,
        &source,
        &context,
        &secrets,
        &modules,
        &reusable,
        &binding,
      );
      let _artifact_cleanup = ArtifactInvocationCleanup::new(options, &host_invocation_id);
      let succeeded = host
        .execute(&request)
        .await
        .is_ok_and(|completed| matches!(completed.outcome, HostOutcome::Success { .. }));
      engine.append_payload(
        run_id,
        if succeeded {
          RunEventPayload::ReusableLifecycleActionSucceeded(
            crate::ReusableLifecycleActionSucceededData {
              invocation_id: invocation_id.clone(),
              definition_digest: definition_digest.clone(),
              hook: event_hook,
              action_id,
              outcome: crate::ReusableLifecycleOutcome::Succeeded,
            },
          )
        } else {
          RunEventPayload::ReusableLifecycleActionFailed(crate::ReusableLifecycleActionFailedData {
            invocation_id: invocation_id.clone(),
            definition_digest: definition_digest.clone(),
            hook: event_hook,
            action_id,
            outcome: crate::ReusableLifecycleOutcome::Failed,
            warning_code: "WOML_REUSABLE_LIFECYCLE_ACTION_FAILED".to_string(),
          })
        },
      )?;
    }
    let warnings = engine
      .projection(run_id)?
      .reusable_lifecycle_hooks
      .get(&key)
      .map_or(0, |hook| hook.warning_codes.len());
    engine.append_payload(
      run_id,
      RunEventPayload::ReusableLifecycleCompleted(crate::ReusableLifecycleCompletedData {
        invocation_id: invocation_id.clone(),
        definition_digest: definition_digest.clone(),
        hook: event_hook,
        outcome: if warnings == 0 {
          crate::ReusableLifecycleOutcome::Succeeded
        } else {
          crate::ReusableLifecycleOutcome::CompletedWithWarnings
        },
      }),
    )?;
  }
  Ok(())
}

async fn drive_reusable_step_lifecycle<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  node_id: &str,
  outcome: ReusableInvocationOutcome,
  result: Option<Value>,
  error: Option<ReusableLifecycleErrorV1>,
  options: &RuntimeExecutionOptions,
  host: &ScriptHostClient,
) -> Result<(), RuntimeExecutionError> {
  let Some(descriptor) = reusable_step_descriptor(engine.workflow(), node_id).cloned() else {
    return Ok(());
  };
  drive_reusable_lifecycle(
    engine,
    run_id,
    &descriptor,
    outcome,
    result,
    error,
    options,
    host,
  )
  .await
}

async fn drive_reusable_provider_lifecycle<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  provider_id: &str,
  outcome: ReusableInvocationOutcome,
  result: Option<Value>,
  error: Option<ReusableLifecycleErrorV1>,
  options: &RuntimeExecutionOptions,
  host: &ScriptHostClient,
) -> Result<(), RuntimeExecutionError> {
  let Some(descriptor) = reusable_provider_descriptor(engine.workflow(), provider_id).cloned()
  else {
    return Ok(());
  };
  drive_reusable_lifecycle(
    engine,
    run_id,
    &descriptor,
    outcome,
    result,
    error,
    options,
    host,
  )
  .await
}

async fn drive_pending_reusable_step_lifecycles<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  options: &RuntimeExecutionOptions,
  host: &mut Option<ScriptHostClient>,
) -> Result<(), RuntimeExecutionError> {
  let descriptors = engine
    .workflow()
    .reusable_definitions
    .clone()
    .unwrap_or_default();
  for descriptor in descriptors {
    let CompiledReusableInvocation::Step {
      node_id,
      lifecycle: Some(_),
      ..
    } = descriptor
    else {
      continue;
    };
    let projection = engine.projection(run_id)?;
    let Some(attempt) = projection.latest_attempt(&node_id) else {
      continue;
    };
    let (outcome, result, error) = match &attempt.status {
      AttemptStatus::Started => continue,
      AttemptStatus::Succeeded { output } => (
        ReusableInvocationOutcome::Succeeded,
        Some(output.clone()),
        None,
      ),
      AttemptStatus::Failed { .. } if projection.pending_retries.contains_key(&node_id) => continue,
      AttemptStatus::Failed { failure }
        if failure.kind == AttemptFailureKind::InvocationCancelled =>
      {
        (ReusableInvocationOutcome::Cancelled, None, None)
      }
      AttemptStatus::Failed { failure } => (
        ReusableInvocationOutcome::Failed,
        None,
        Some(ReusableLifecycleErrorV1 {
          code: failure.code.clone(),
          message: failure.message.clone(),
        }),
      ),
    };
    if host.is_none() {
      *host = Some(options.acquire_script_host().await?);
    }
    drive_reusable_step_lifecycle(
      engine,
      run_id,
      &node_id,
      outcome,
      result,
      error,
      options,
      host
        .as_ref()
        .expect("reusable lifecycle host was initialized"),
    )
    .await?;
  }
  Ok(())
}

fn custom_provider_runtime_artifacts(
  workflow: &CompiledWorkflowDefinition,
  options: &RuntimeExecutionOptions,
) -> Result<Vec<CustomProviderScriptArtifact>, LifecycleFailure> {
  let mut seen = HashSet::new();
  let mut artifacts = Vec::new();
  for (definition_digest, script_artifact_id) in workflow
    .reusable_definitions
    .iter()
    .flatten()
    .filter_map(|definition| match definition {
      CompiledReusableInvocation::NotificationProvider {
        definition_digest,
        script_artifact_id,
        ..
      } => Some((definition_digest, script_artifact_id)),
      _ => None,
    })
  {
    if !seen.insert(script_artifact_id.as_str()) {
      continue;
    }
    let name = format!("__woml_provider__{script_artifact_id}");
    let artifact = options
      .runtime_modules
      .iter()
      .find(|artifact| artifact.name == name)
      .ok_or_else(|| LifecycleFailure {
        kind: LifecycleFailureKind::ProviderFailed,
        code: "WOML_CUSTOM_PROVIDER_ARTIFACT_MISSING".to_string(),
        message: "The immutable custom provider artifact is unavailable.".to_string(),
      })?;
    artifacts.push(CustomProviderScriptArtifact {
      script_artifact_id: script_artifact_id.clone(),
      definition_digest: definition_digest.clone(),
      source: artifact.bundle.clone(),
    });
  }
  Ok(artifacts)
}

fn custom_lifecycle_props(
  descriptor: &CompiledReusableInvocation,
  context: &WorkflowContext,
  secrets: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, Value>, LifecycleFailure> {
  let CompiledReusableInvocation::NotificationProvider { props, .. } = descriptor else {
    return Err(LifecycleFailure {
      kind: LifecycleFailureKind::ProviderFailed,
      code: "WOML_CUSTOM_PROVIDER_DESCRIPTOR_INVALID".to_string(),
      message: "The lifecycle delivery does not reference a custom provider.".to_string(),
    });
  };
  props
    .iter()
    .map(|prop| {
      let value = match &prop.expression {
        crate::model::CompiledReusablePropExpression::Literal { value } => {
          Value::String(value.clone())
        }
        crate::model::CompiledReusablePropExpression::Context { path } => {
          let path = path
            .split('.')
            .map(|segment| {
              if segment == "payload" {
                "trigger".to_string()
              } else {
                segment.to_string()
              }
            })
            .collect::<Vec<_>>();
          resolve_context_reference(&ValueExpression::ContextReference { path }, context).map_err(
            |_| LifecycleFailure {
              kind: LifecycleFailureKind::ProviderFailed,
              code: "WOML_CUSTOM_PROVIDER_PROP_UNAVAILABLE".to_string(),
              message: format!("Custom provider prop {:?} is unavailable.", prop.name),
            },
          )?
        }
        crate::model::CompiledReusablePropExpression::Secret { name } => Value::String(
          secrets
            .get(name)
            .filter(|value| !value.is_empty())
            .cloned()
            .ok_or_else(|| LifecycleFailure {
              kind: LifecycleFailureKind::ProviderFailed,
              code: "WOML_CUSTOM_PROVIDER_SECRET_UNAVAILABLE".to_string(),
              message: format!("Required custom provider secret {name:?} is unavailable."),
            })?,
        ),
      };
      Ok((prop.binding_name.clone(), value))
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

enum LifecycleProviderExecution {
  Succeeded {
    duration_ms: f64,
    receipt: Vec<u8>,
    metadata: Map<String, Value>,
  },
  Failed {
    duration_ms: f64,
    failure: CapabilityFailure,
    lifecycle: LifecycleFailure,
    retryable: bool,
  },
}

fn custom_provider_failure_kind(kind: crate::CustomProviderFailureKind) -> CapabilityFailureKind {
  match kind {
    crate::CustomProviderFailureKind::DeliveryAmbiguous => CapabilityFailureKind::Ambiguous,
    crate::CustomProviderFailureKind::HostCrashed
    | crate::CustomProviderFailureKind::WorkerCrashed => CapabilityFailureKind::HostCrashed,
    _ => CapabilityFailureKind::TransportFailed,
  }
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
  if options.infrastructure_policy_required || options.infrastructure_policy.is_some() {
    return Err(LifecycleFailure {
      kind: LifecycleFailureKind::ProviderFailed,
      code: "WOML_POLICY_INPUT_UNSUPPORTED".to_string(),
      message: "Lifecycle notification delivery is blocked while Infrastructure Policy enforcement is active because this direct provider path has no reviewed pre-effect descriptor adapter."
        .to_string(),
    });
  }
  let deliveries = lifecycle_notification_deliveries(action).map_err(|error| LifecycleFailure {
    kind: LifecycleFailureKind::ProviderFailed,
    code: "WOML_NOTIFICATION_REQUEST_INVALID".to_string(),
    message: error.to_string(),
  })?;
  let has_builtin = deliveries.iter().any(|delivery| {
    matches!(
      delivery.provider,
      "slack" | "telegram" | "discord" | "whatsapp"
    )
  });
  let has_custom = deliveries
    .iter()
    .any(|delivery| delivery.provider == "custom");
  let host = if has_builtin {
    let host_options = options
      .notification_host
      .clone()
      .ok_or_else(|| LifecycleFailure {
        kind: LifecycleFailureKind::HostCrashed,
        code: "WOML_NOTIFICATION_HOST_UNAVAILABLE".to_string(),
        message: "The informational notification provider host is not configured.".to_string(),
      })?;
    Some(
      NotificationHostClient::spawn(host_options)
        .await
        .map_err(|error| LifecycleFailure {
          kind: LifecycleFailureKind::HostCrashed,
          code: "WOML_NOTIFICATION_HOST_CRASHED".to_string(),
          message: error.to_string(),
        })?,
    )
  } else {
    None
  };
  let custom_host = if has_custom {
    let path = options
      .custom_notification_host_path
      .clone()
      .ok_or_else(|| LifecycleFailure {
        kind: LifecycleFailureKind::HostCrashed,
        code: "WOML_CUSTOM_PROVIDER_HOST_UNAVAILABLE".to_string(),
        message: "The custom notification provider host is not configured.".to_string(),
      })?;
    let artifacts = custom_provider_runtime_artifacts(engine.workflow(), options)?;
    Some(
      CustomNotificationHostClient::spawn(
        CustomNotificationHostProcessOptions::new(options.script_host.bun_executable.clone(), path)
          .with_artifacts(artifacts),
      )
      .await
      .map_err(|error| LifecycleFailure {
        kind: LifecycleFailureKind::HostCrashed,
        code: "WOML_CUSTOM_PROVIDER_HOST_CRASHED".to_string(),
        message: error.to_string(),
      })?,
    )
  } else {
    None
  };
  let action_key = step_effect_idempotency_key(run_id, engine.definition_hash(), &action.action_id);
  let mut failures = Vec::new();
  let mut reusable_lifecycle_host: Option<ScriptHostClient> = None;

  for delivery in deliveries {
    let message = match render_lifecycle_notification_message(delivery.message, context, lifecycle)
    {
      Ok(message) => message,
      Err(failure) => {
        failures.push(failure);
        continue;
      }
    };
    let credentials = if matches!(
      delivery.provider,
      "slack" | "telegram" | "discord" | "whatsapp"
    ) {
      match NotificationCredentials::from_symbolic(&delivery.credentials) {
        Ok(credentials) => Some(credentials),
        Err(message) => {
          failures.push(LifecycleFailure {
            kind: LifecycleFailureKind::ProviderFailed,
            code: "WOML_NOTIFICATION_REQUEST_INVALID".to_string(),
            message,
          });
          continue;
        }
      }
    } else {
      None
    };
    let custom_descriptor = if delivery.provider == "custom" {
      let Some(provider_id) = delivery.provider_id else {
        failures.push(LifecycleFailure {
          kind: LifecycleFailureKind::ProviderFailed,
          code: "WOML_CUSTOM_PROVIDER_DESCRIPTOR_INVALID".to_string(),
          message: "The lifecycle delivery has no custom provider identity.".to_string(),
        });
        continue;
      };
      let Some(descriptor) = reusable_provider_descriptor(engine.workflow(), provider_id).cloned()
      else {
        failures.push(LifecycleFailure {
          kind: LifecycleFailureKind::ProviderFailed,
          code: "WOML_CUSTOM_PROVIDER_DESCRIPTOR_INVALID".to_string(),
          message: "The custom lifecycle provider descriptor is unavailable.".to_string(),
        });
        continue;
      };
      Some(descriptor)
    } else {
      None
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
      let execution = if let Some(descriptor) = custom_descriptor.as_ref() {
        let CompiledReusableInvocation::NotificationProvider {
          definition_digest,
          script_artifact_id,
          ..
        } = descriptor
        else {
          unreachable!()
        };
        let request = CustomProviderExecuteMessage {
          protocol: CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL.to_string(),
          protocol_version: CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
          message_type: "execute".to_string(),
          invocation_id: invocation_id.clone(),
          definition_digest: definition_digest.clone(),
          script_artifact_id: script_artifact_id.clone(),
          props: custom_lifecycle_props(descriptor, context, &options.resolved_secrets)?,
          notification: CustomNotificationRequest {
            kind: CustomNotificationKind::Informational,
            message: message.clone(),
            delivery_id: delivery.delivery_id.to_string(),
            idempotency_key: operation_key.clone(),
            actions: None,
          },
          attempt: CustomProviderAttempt {
            number: provider_attempt,
            max: 3,
          },
          limits: CustomProviderLimits {
            timeout_ms: 30_000,
            max_result_bytes: 16_384,
          },
        };
        match custom_host
          .as_ref()
          .expect("custom host")
          .invoke(&request)
          .await
        {
          Ok(completed) => match completed.outcome {
            CustomProviderOutcome::Succeeded { receipt } => {
              let encoded = serde_json::to_vec(&receipt).unwrap_or_default();
              let mut metadata = Map::new();
              if let Some(message_id) = receipt.message_id {
                metadata.insert("providerMessageId".to_string(), Value::String(message_id));
              }
              LifecycleProviderExecution::Succeeded {
                duration_ms: completed.duration_ms,
                receipt: encoded,
                metadata,
              }
            }
            CustomProviderOutcome::Failed { error } => {
              let ambiguous = matches!(
                error.kind,
                crate::CustomProviderFailureKind::DeliveryAmbiguous
                  | crate::CustomProviderFailureKind::HostCrashed
                  | crate::CustomProviderFailureKind::WorkerCrashed
              );
              let retryable = error.retryable && !ambiguous;
              LifecycleProviderExecution::Failed {
                duration_ms: completed.duration_ms,
                failure: notification_capability_failure(
                  custom_provider_failure_kind(error.kind),
                  error.code.clone(),
                  error.message.clone(),
                  retryable,
                  ambiguous,
                ),
                lifecycle: LifecycleFailure {
                  kind: LifecycleFailureKind::ProviderFailed,
                  code: error.code,
                  message: error.message,
                },
                retryable,
              }
            }
          },
          Err(error) => LifecycleProviderExecution::Failed {
            duration_ms: 0.0,
            failure: notification_capability_failure(
              CapabilityFailureKind::HostCrashed,
              "WOML_CUSTOM_PROVIDER_HOST_CRASHED".to_string(),
              "The custom provider host stopped during an uncertain delivery.".to_string(),
              false,
              true,
            ),
            lifecycle: LifecycleFailure {
              kind: LifecycleFailureKind::HostCrashed,
              code: "WOML_CUSTOM_PROVIDER_HOST_CRASHED".to_string(),
              message: match error {
                CustomNotificationHostClientError::Protocol(_) => {
                  "The custom provider returned an invalid response.".to_string()
                }
                _ => "The custom provider host stopped during delivery.".to_string(),
              },
            },
            retryable: false,
          },
        }
      } else {
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
          credentials: credentials.clone().expect("built-in provider credentials"),
          message: message.clone(),
          template_name: delivery.template_name.map(str::to_string),
          language: delivery.language.map(str::to_string),
        };
        match host
          .as_ref()
          .expect("built-in notification host")
          .invoke(&invocation_id, &request)
          .await
        {
          Ok(completed) => match completed.outcome {
            NotificationHostOutcome::DeliverySuccess { provider_message } => {
              let encoded = serde_json::to_vec(&provider_message).unwrap_or_default();
              let mut metadata = Map::new();
              match provider_message {
                crate::ProviderMessageIdentity::Slack(message) => {
                  metadata.insert(
                    "workspaceId".to_string(),
                    Value::String(message.workspace_id),
                  );
                  metadata.insert("channelId".to_string(), Value::String(message.channel_id));
                  metadata.insert(
                    "providerMessageId".to_string(),
                    Value::String(message.message_id),
                  );
                }
                crate::ProviderMessageIdentity::Communication(message) => {
                  metadata.insert("accountId".to_string(), Value::String(message.account_id));
                  metadata.insert(
                    "conversationId".to_string(),
                    Value::String(message.conversation_id),
                  );
                  metadata.insert(
                    "providerMessageId".to_string(),
                    Value::String(message.message_id),
                  );
                }
              }
              LifecycleProviderExecution::Succeeded {
                duration_ms: completed.duration_ms,
                receipt: encoded,
                metadata,
              }
            }
            NotificationHostOutcome::Failure { error } => {
              let ambiguous = error.kind == "delivery_ambiguous";
              LifecycleProviderExecution::Failed {
                duration_ms: completed.duration_ms,
                failure: notification_capability_failure(
                  if ambiguous {
                    CapabilityFailureKind::Ambiguous
                  } else {
                    CapabilityFailureKind::TransportFailed
                  },
                  error.code.clone(),
                  error.message.clone(),
                  error.retryable,
                  ambiguous,
                ),
                lifecycle: LifecycleFailure {
                  kind: LifecycleFailureKind::ProviderFailed,
                  code: error.code,
                  message: error.message,
                },
                retryable: error.retryable && !ambiguous,
              }
            }
            NotificationHostOutcome::UpdateSuccess => LifecycleProviderExecution::Failed {
              duration_ms: completed.duration_ms,
              failure: notification_capability_failure(
                CapabilityFailureKind::TransportFailed,
                "WOML_NOTIFICATION_PROTOCOL_INVALID".to_string(),
                "Informational delivery returned an update result.".to_string(),
                false,
                false,
              ),
              lifecycle: LifecycleFailure {
                kind: LifecycleFailureKind::ProviderFailed,
                code: "WOML_NOTIFICATION_PROTOCOL_INVALID".to_string(),
                message: "Informational notification delivery returned an update result."
                  .to_string(),
              },
              retryable: false,
            },
          },
          Err(error) => LifecycleProviderExecution::Failed {
            duration_ms: 0.0,
            failure: notification_capability_failure(
              match error {
                NotificationHostClientError::HostCrashed(_)
                | NotificationHostClientError::Startup(_) => CapabilityFailureKind::HostCrashed,
                _ => CapabilityFailureKind::TransportFailed,
              },
              "WOML_NOTIFICATION_HOST_CRASHED".to_string(),
              error.to_string(),
              false,
              true,
            ),
            lifecycle: LifecycleFailure {
              kind: LifecycleFailureKind::HostCrashed,
              code: "WOML_NOTIFICATION_HOST_CRASHED".to_string(),
              message: error.to_string(),
            },
            retryable: false,
          },
        }
      };
      match execution {
        LifecycleProviderExecution::Succeeded {
          duration_ms,
          receipt,
          mut metadata,
        } => {
          let lifecycle_result = serde_json::from_slice::<Value>(&receipt).ok();
          metadata.insert(
            "provider".to_string(),
            Value::String(delivery.provider.to_string()),
          );
          metadata.insert(
            "destination".to_string(),
            Value::String(delivery.destination.to_string()),
          );
          metadata.insert("providerAttempt".to_string(), Value::from(provider_attempt));
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
                duration_ms,
                result_bytes: receipt.len() as u64,
                result_digest: format!("sha256:{}", hex::encode(Sha256::digest(&receipt))),
              }),
            )
            .map_err(|error| LifecycleFailure {
              kind: LifecycleFailureKind::ProviderFailed,
              code: "WOML_NOTIFICATION_EVENT_FAILED".to_string(),
              message: error.to_string(),
            })?;
          if let Some(descriptor) = custom_descriptor.as_ref() {
            if reusable_lifecycle_host.is_none() {
              reusable_lifecycle_host = Some(options.acquire_script_host().await.map_err(
                |error| LifecycleFailure {
                  kind: LifecycleFailureKind::HostCrashed,
                  code: "WOML_REUSABLE_LIFECYCLE_HOST_CRASHED".to_string(),
                  message: error.to_string(),
                },
              )?);
            }
            drive_reusable_lifecycle(
              engine,
              run_id,
              descriptor,
              ReusableInvocationOutcome::Succeeded,
              lifecycle_result,
              None,
              options,
              reusable_lifecycle_host
                .as_ref()
                .expect("reusable provider lifecycle host"),
            )
            .await
            .map_err(|error| LifecycleFailure {
              kind: LifecycleFailureKind::ProviderFailed,
              code: "WOML_REUSABLE_LIFECYCLE_FAILED".to_string(),
              message: error.to_string(),
            })?;
          }
          delivered = true;
          break;
        }
        LifecycleProviderExecution::Failed {
          duration_ms,
          failure,
          lifecycle,
          retryable,
        } => {
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
                duration_ms,
                failure,
              }),
            )
            .map_err(|error| LifecycleFailure {
              kind: LifecycleFailureKind::ProviderFailed,
              code: "WOML_NOTIFICATION_EVENT_FAILED".to_string(),
              message: error.to_string(),
            })?;
          if retryable && provider_attempt < 3 {
            tokio::time::sleep(Duration::from_secs(if provider_attempt == 1 {
              1
            } else {
              5
            }))
            .await;
            continue;
          }
          if let Some(descriptor) = custom_descriptor.as_ref() {
            if reusable_lifecycle_host.is_none() {
              reusable_lifecycle_host = Some(options.acquire_script_host().await.map_err(
                |error| LifecycleFailure {
                  kind: LifecycleFailureKind::HostCrashed,
                  code: "WOML_REUSABLE_LIFECYCLE_HOST_CRASHED".to_string(),
                  message: error.to_string(),
                },
              )?);
            }
            drive_reusable_lifecycle(
              engine,
              run_id,
              descriptor,
              ReusableInvocationOutcome::Failed,
              None,
              Some(ReusableLifecycleErrorV1 {
                code: lifecycle.code.clone(),
                message: lifecycle.message.clone(),
              }),
              options,
              reusable_lifecycle_host
                .as_ref()
                .expect("reusable provider lifecycle host"),
            )
            .await
            .map_err(|error| LifecycleFailure {
              kind: LifecycleFailureKind::ProviderFailed,
              code: "WOML_REUSABLE_LIFECYCLE_FAILED".to_string(),
              message: error.to_string(),
            })?;
          }
          failures.push(lifecycle);
          break;
        }
      }
    }
    if !delivered && failures.is_empty() {
      failures.push(LifecycleFailure {
        kind: LifecycleFailureKind::ProviderFailed,
        code: "WOML_NOTIFICATION_DELIVERY_FAILED".to_string(),
        message: "Lifecycle notification delivery failed.".to_string(),
      });
    }
  }
  if let Some(host) = host {
    host.shutdown().await;
  }
  if let Some(host) = custom_host {
    host.shutdown().await;
  }
  if let Some(host) = reusable_lifecycle_host {
    options.release_script_host(host).await;
  }
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
          "{failed} lifecycle deliveries failed. First failure: {}",
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
      | crate::COMPILED_MODEL_SCHEMA_VERSION_V14
      | crate::COMPILED_MODEL_SCHEMA_VERSION_V15
      | crate::COMPILED_MODEL_SCHEMA_VERSION_V16
      | crate::COMPILED_MODEL_SCHEMA_VERSION_V17
      | crate::COMPILED_MODEL_SCHEMA_VERSION_V18
      | crate::COMPILED_MODEL_SCHEMA_VERSION_V19
      | crate::COMPILED_MODEL_SCHEMA_VERSION_V20
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
      match options.acquire_script_host().await {
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
      .filter(|module| !module.name.starts_with("__woml_"))
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
    let artifact_cleanup = ArtifactInvocationCleanup::new(options, &invocation_id);
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
    drop(artifact_cleanup);
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
    ensure_held_locks_current(engine, run_id, options)?;
    drive_pending_reusable_step_lifecycles(engine, run_id, options, host).await?;
    let current = engine.projection(run_id)?;
    if current.timeout_reached_at.is_some() {
      return Err(resumed_failure(engine, run_id, current)?);
    }
    if current.status == RunStatus::Cancelling {
      let active_saga = engine
        .workflow()
        .graph
        .sagas
        .as_deref()
        .unwrap_or_default()
        .iter()
        .find_map(|saga| {
          current.sagas.get(&saga.saga_id).and_then(|state| {
            (state.phase != crate::SagaPhase::CorrectionCompleted)
              .then_some((saga.clone(), state.reason))
          })
        });
      if let Some((saga, existing_reason)) = active_saga {
        let completed = execute_saga_correction(
          engine,
          run_id,
          &saga,
          existing_reason.unwrap_or(SagaCorrectionReason::Cancel),
          options,
          host,
        )
        .await?;
        if !completed
          && engine
            .projection(run_id)?
            .sagas
            .get(&saga.saga_id)
            .is_some_and(|state| state.phase == crate::SagaPhase::ManualRecovery)
        {
          return Err(RuntimeExecutionError::Stalled(
            "saga cancellation requires manual recovery before the run can settle".to_string(),
          ));
        }
      }
      options.ensure_policy_execution_slot().await?;
      settle_cancelling_run(engine, run_id, options.clock.now(), options)?;
      drive_workflow_lifecycle(engine, run_id, options, host).await?;
      return Err(cancelled_run_error(engine, run_id)?);
    }
    if current.status == RunStatus::Cancelled {
      return Err(cancelled_run_error(engine, run_id)?);
    }
    if let Some((saga, reason)) = engine
      .workflow()
      .graph
      .sagas
      .as_deref()
      .unwrap_or_default()
      .iter()
      .find_map(|saga| {
        current.sagas.get(&saga.saga_id).and_then(|state| {
          matches!(
            state.phase,
            crate::SagaPhase::Correcting | crate::SagaPhase::ManualRecovery
          )
          .then_some((saga.clone(), state.reason))
        })
      })
    {
      let reason = reason.ok_or_else(|| {
        RuntimeExecutionError::Stalled("active saga correction has no durable reason".to_string())
      })?;
      let completed = execute_saga_correction(engine, run_id, &saga, reason, options, host).await?;
      if !completed
        && engine
          .projection(run_id)?
          .sagas
          .get(&saga.saga_id)
          .is_some_and(|state| state.phase == crate::SagaPhase::ManualRecovery)
      {
        return Err(RuntimeExecutionError::Stalled(format!(
          "saga {:?} requires manual recovery",
          saga.saga_id
        )));
      }
      if reason == SagaCorrectionReason::Failure {
        let failure = engine
          .events(run_id)?
          .into_iter()
          .rev()
          .find_map(|event| match event.payload {
            RunEventPayload::StepAttemptFailed(data)
              if engine
                .workflow()
                .saga_for_body_node(&data.node_id)
                .is_some_and(|owner| owner.saga_id == saga.saga_id) =>
            {
              Some(data)
            }
            _ => None,
          })
          .ok_or_else(|| {
            RuntimeExecutionError::Stalled(
              "failed saga correction has no durable forward failure".to_string(),
            )
          })?;
        engine.append_payload(
          run_id,
          RunEventPayload::RunFailed(attempt_run_failed_data(
            engine.event_schema_version(),
            &failure,
          )),
        )?;
        return Err(resumed_failure(engine, run_id, engine.projection(run_id)?)?);
      }
      continue;
    }
    let ready = engine.ready_node_ids(run_id)?;
    if ready.is_empty() {
      if let Some(fork) = ready_fork(engine.workflow(), &current, &ready) {
        if host.is_none() {
          *host = Some(options.acquire_script_host().await?);
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
    }
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
    if let Some(lock) = engine
      .workflow()
      .graph
      .locks
      .as_deref()
      .unwrap_or_default()
      .iter()
      .find(|lock| lock.acquire_node_id == *node_id)
      .cloned()
    {
      let projection = engine.projection(run_id)?;
      if !projection.locks.contains_key(&lock.lock_id) {
        let key = resolve_lock_key(&lock.key, &projection.context)?;
        let now = options.clock.now();
        let deadline = now
          + chrono::Duration::milliseconds(i64::try_from(lock.acquire_timeout_ms).map_err(
            |_| {
              RuntimeExecutionError::InvalidConfiguration(
                "Lock acquisition timeout exceeds the runtime clock.".to_string(),
              )
            },
          )?);
        engine.register_lock_waiter(run_id, &lock.lock_id, &key, now, deadline)?;
      }
      options.suspend_policy_execution_slot().await?;
      loop {
        let projection = engine.projection(run_id)?;
        let state = projection.locks.get(&lock.lock_id).ok_or_else(|| {
          RuntimeExecutionError::Stalled(
            "Registered lock disappeared from durable projection.".to_string(),
          )
        })?;
        match state.status {
          crate::LockStatus::Held => {
            options.ensure_lock_renewal(run_id, &lock.lock_id, lock.lease_ms);
            break;
          }
          crate::LockStatus::Released => break,
          crate::LockStatus::Waiting => {
            if options.clock.now() >= state.deadline {
              let resolved_at = options.clock.now();
              engine.timeout_lock_waiter(run_id, &lock.lock_id, resolved_at)?;
              engine.decide_run_failed(
                run_id,
                LifecycleFailure {
                  kind: LifecycleFailureKind::TimedOut,
                  code: "WOML_LOCK_ACQUIRE_TIMED_OUT".to_string(),
                  message: format!(
                    "Lock {:?} was not acquired before its deadline.",
                    lock.lock_id
                  ),
                },
                resolved_at,
              )?;
              return Err(resumed_failure(engine, run_id, engine.projection(run_id)?)?);
            }
            tokio::time::sleep(CANCELLATION_POLL_INTERVAL).await;
          }
          _ => {
            return Err(RuntimeExecutionError::Stalled(format!(
              "Lock {:?} acquisition did not succeed.",
              lock.lock_id
            )))
          }
        }
      }
      execution_order.push(lock.acquire_node_id);
      continue;
    }
    if let Some(lock) = engine
      .workflow()
      .graph
      .locks
      .as_deref()
      .unwrap_or_default()
      .iter()
      .find(|lock| lock.release_node_id == *node_id)
      .cloned()
    {
      let projection = engine.projection(run_id)?;
      let held = projection.locks.get(&lock.lock_id).ok_or_else(|| {
        RuntimeExecutionError::Stalled("Lock release has no projected acquisition.".to_string())
      })?;
      let acquired_at = held.acquired_at.ok_or_else(|| {
        RuntimeExecutionError::Stalled("Lock release has no acquisition timestamp.".to_string())
      })?;
      let released_at = options.clock.now();
      let waited_ms = (acquired_at - held.registered_at).num_milliseconds().max(0);
      let result = serde_json::json!({
        "status": "released", "key": held.key, "fencingToken": held.fencing_token,
        "acquiredAt": acquired_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "releasedAt": released_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "waitedMs": waited_ms,
      });
      engine.release_lock(run_id, &lock.lock_id, result, released_at)?;
      options.stop_lock_renewal(run_id, &lock.lock_id);
      execution_order.push(lock.release_node_id);
      continue;
    }
    options.ensure_policy_execution_slot().await?;
    if let Some(mut group) = engine.workflow().parallel_group_for_child(node_id) {
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
      let wait_node_ids = group
        .child_node_ids
        .iter()
        .filter(|child_id| {
          ready.contains(child_id)
            && engine
              .workflow()
              .node(child_id)
              .is_some_and(|node| node.handler == "engine.durable-wait")
        })
        .cloned()
        .collect::<Vec<_>>();
      let mut pending_wait = None;
      for wait_node_id in wait_node_ids {
        match register_durable_wait_node(engine, run_id, &wait_node_id, options).await? {
          DurableWaitNodeProgress::Resolved => execution_order.push(wait_node_id),
          DurableWaitNodeProgress::Waiting(outcome) => pending_wait = Some(outcome),
        }
      }
      group.child_node_ids.retain(|child_id| {
        ready.contains(child_id)
          && engine
            .workflow()
            .node(child_id)
            .is_some_and(|node| node.handler == "runtime.script")
      });
      if !group.child_node_ids.is_empty() {
        if host.is_none() {
          *host = Some(options.acquire_script_host().await?);
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
      }
      if let Some(outcome) = pending_wait {
        return Ok(outcome);
      }
      continue;
    }
    if let Some(wait_node_id) = ready.iter().find(|ready_id| {
      engine
        .workflow()
        .node(ready_id)
        .is_some_and(|node| node.handler == "engine.durable-wait")
    }) {
      match register_durable_wait_node(engine, run_id, wait_node_id, options).await? {
        DurableWaitNodeProgress::Resolved => {
          execution_order.push(wait_node_id.clone());
          continue;
        }
        DurableWaitNodeProgress::Waiting(outcome) => return Ok(outcome),
      }
    }
    if let Some(fork) = ready_fork(engine.workflow(), &current, &ready) {
      if host.is_none() {
        *host = Some(options.acquire_script_host().await?);
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
      return Err(RuntimeExecutionError::Stalled(format!(
        "the current runtime received more than one ready node: {ready:?}"
      )));
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
      "engine.saga-open" => {
        let saga = engine
          .workflow()
          .saga_for_control_node(node_id)
          .ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "saga open node {node_id:?} has no Model v19 descriptor"
            ))
          })?;
        engine.append_payload(
          run_id,
          RunEventPayload::SagaStarted(SagaIdentityData {
            saga_id: saga.saga_id.clone(),
          }),
        )?;
      }
      "engine.saga-commit" => {
        let saga = engine
          .workflow()
          .saga_for_control_node(node_id)
          .cloned()
          .ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "saga commit node {node_id:?} has no Model v19 descriptor"
            ))
          })?;
        engine.append_payload(
          run_id,
          RunEventPayload::SagaForwardSucceeded(SagaIdentityData {
            saga_id: saga.saga_id.clone(),
          }),
        )?;
        let cleanup_succeeded = execute_saga_correction(
          engine,
          run_id,
          &saga,
          SagaCorrectionReason::Success,
          options,
          host,
        )
        .await?;
        if !cleanup_succeeded {
          let projection = engine.projection(run_id)?;
          if projection
            .sagas
            .get(&saga.saga_id)
            .is_some_and(|state| state.phase == crate::SagaPhase::ManualRecovery)
          {
            return Err(RuntimeExecutionError::Stalled(format!(
              "saga {:?} requires manual cleanup recovery",
              saga.saga_id
            )));
          }
          engine.decide_run_failed(
            run_id,
            LifecycleFailure {
              kind: LifecycleFailureKind::ScriptThrew,
              code: "WOML_CLEANUP_FAILED".to_string(),
              message: format!(
                "Saga {:?} completed its forward work but one or more cleanup actions failed.",
                saga.saga_id
              ),
            },
            options.clock.now(),
          )?;
          return Err(resumed_failure(engine, run_id, engine.projection(run_id)?)?);
        }
      }
      "engine.saga-result" => {
        let saga = engine
          .workflow()
          .saga_for_control_node(node_id)
          .ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "saga result node {node_id:?} has no Model v19 descriptor"
            ))
          })?;
        let output = engine
          .projection(run_id)?
          .context
          .steps
          .get(&saga.body_result_node_id)
          .cloned()
          .ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "saga {:?} has no result from {:?}",
              saga.saga_id, saga.body_result_node_id
            ))
          })?;
        engine.publish_pure_result(run_id, node_id, output)?;
        execution_order.push(node_id.clone());
      }
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
      "engine.for-each-open" => {
        let descriptor = engine
          .workflow()
          .graph
          .for_each
          .as_deref()
          .unwrap_or_default()
          .iter()
          .find(|descriptor| descriptor.open_node_id == *node_id)
          .cloned()
          .ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "for-each open node {node_id:?} has no Model v16 descriptor"
            ))
          })?;
        if host.is_none() {
          *host = Some(options.acquire_script_host().await?);
        }
        execute_bounded_for_each(
          engine,
          run_id,
          &descriptor,
          options,
          host.as_ref().expect("for-each script host was initialized"),
        )
        .await?;
        execution_order.push(descriptor.for_each_id);
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
        let (choice_id, arm_id) = match selected_choice_arm(engine.workflow(), node_id, &context) {
          Ok(selection) => selection,
          Err(error) => {
            return fail_choice(engine, run_id, error);
          }
        };
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
      "engine.choice-result" => {
        let projection = engine.projection(run_id)?;
        let choice = engine
          .workflow()
          .graph
          .choices
          .as_deref()
          .unwrap_or_default()
          .iter()
          .find(|choice| choice.result_node_id.as_deref() == Some(node_id))
          .ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "choice result {node_id:?} has no compiled choice descriptor"
            ))
          })?;
        let arm_id = projection
          .choice_selections
          .get(&choice.choice_id)
          .cloned()
          .ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "choice result {node_id:?} became ready without a recorded selection"
            ))
          })?;
        let ValueExpression::Object { fields } = &inputs else {
          return Err(RuntimeExecutionError::Stalled(format!(
            "choice result {node_id:?} has invalid compiled inputs"
          )));
        };
        let expression = fields.get(&arm_id).ok_or_else(|| {
          RuntimeExecutionError::Stalled(format!(
            "choice result {node_id:?} has no value for selected arm {arm_id:?}"
          ))
        })?;
        let output = match resolve_context_reference(expression, &projection.context) {
          Ok(output) => output,
          Err(error) => {
            return fail_choice(
              engine,
              run_id,
              BranchEvaluationError {
                branch_id: choice.choice_id.clone(),
                arm_id: Some(arm_id),
                path: Some(error.path),
                kind: BranchEvaluationErrorKind::ReferenceNotAvailable,
              },
            );
          }
        };
        engine.publish_pure_result(run_id, node_id, output.clone())?;
        execution_order.push(node_id.clone());
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
      "engine.durable-wait" => {
        match register_durable_wait_node(engine, run_id, node_id, options).await? {
          DurableWaitNodeProgress::Resolved => {
            execution_order.push(node_id.clone());
            continue;
          }
          DurableWaitNodeProgress::Waiting(outcome) => return Ok(outcome),
        }
      }
      "runtime.script" => {
        let source = source.ok_or_else(|| {
          RuntimeExecutionError::Stalled(format!("node {node_id:?} has no script source"))
        })?;
        let publish_terminal_success = node_id == &terminal_node_id
          && engine.workflow().graph.settlement.is_none()
          && reusable_step_descriptor(engine.workflow(), node_id).is_none();
        let outcome = execute_script_node(
          engine,
          run_id,
          node_id,
          &source,
          publish_terminal_success,
          options,
          host,
        )
        .await?;
        let output = match outcome {
          ScriptNodeOutcome::Succeeded(output) => output,
          ScriptNodeOutcome::WorkflowSucceeded => {
            execution_order.push(node_id.clone());
            return Ok(WorkflowRuntimeOutcome::succeeded(final_result(
              engine,
              run_id,
              execution_order.clone(),
            )?));
          }
          ScriptNodeOutcome::RetryScheduled | ScriptNodeOutcome::Cancelled => continue,
          ScriptNodeOutcome::SagaFailed(failure) => {
            let saga = engine
              .workflow()
              .saga_for_body_node(node_id)
              .cloned()
              .ok_or_else(|| {
                RuntimeExecutionError::Stalled(
                  "failed saga step has no owning Model v19 descriptor".to_string(),
                )
              })?;
            let correction_completed = execute_saga_correction(
              engine,
              run_id,
              &saga,
              SagaCorrectionReason::Failure,
              options,
              host,
            )
            .await?;
            if !correction_completed
              && engine
                .projection(run_id)?
                .sagas
                .get(&saga.saga_id)
                .is_some_and(|state| state.phase == crate::SagaPhase::ManualRecovery)
            {
              return Err(RuntimeExecutionError::Stalled(format!(
                "saga {:?} requires manual recovery",
                saga.saga_id
              )));
            }
            engine.append_payload(
              run_id,
              RunEventPayload::RunFailed(attempt_run_failed_data(
                engine.event_schema_version(),
                &failure,
              )),
            )?;
            return Err(resumed_failure(engine, run_id, engine.projection(run_id)?)?);
          }
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
  if let Some(review_reference) = approval.review.clone() {
    let authority = options.reviewer_authority.as_ref().ok_or_else(|| {
      RuntimeExecutionError::InvalidConfiguration(
        "WOML_APPROVAL_AUTHORITY_UNAVAILABLE: rich approval requires an operator-owned Reviewer Directory v1".to_string(),
      )
    })?;
    let required = approval.required_approvals.ok_or_else(|| {
      RuntimeExecutionError::Stalled(
        "a rich approval has no compiled required approval count".to_string(),
      )
    })?;
    let directory_version = authority.active_version().map_err(|error| {
      RuntimeExecutionError::InvalidConfiguration(format!("{}: {error}", error.code()))
    })?;
    authority
      .validate_requirements(&directory_version, required, &approval.required_roles)
      .map_err(|error| {
        RuntimeExecutionError::InvalidConfiguration(format!("{}: {error}", error.code()))
      })?;
    let projection = engine.projection(run_id)?;
    let review = crate::resolve_rich_approval_review(&review_reference, &projection.context)
      .map_err(rich_approval_runtime_error)?;
    let workflow_version = engine
      .workflow()
      .metadata
      .as_ref()
      .and_then(|metadata| metadata.version.as_deref())
      .unwrap_or("0.0.0")
      .to_string();
    let reject_artifacts = RejectRichApprovalArtifacts;
    let artifact_verifier: &dyn RichApprovalArtifactVerifier = options
      .managed_artifact_store
      .as_deref()
      .map(|store| store as &dyn RichApprovalArtifactVerifier)
      .unwrap_or(&reject_artifacts);
    let known_secrets = options
      .resolved_secrets
      .values()
      .cloned()
      .collect::<Vec<_>>();
    let binding = crate::bind_rich_approval_review(
      RichApprovalBindingInput {
        definition_hash: engine.definition_hash().to_string(),
        workflow_id: engine.workflow().workflow_id.clone(),
        workflow_version,
        run_id: run_id.to_string(),
        approval_id: approval.approval_id.clone(),
        request_id: request_id.clone(),
        review,
        required_approvals: required,
        required_roles: approval.required_roles.clone(),
        directory_version,
      },
      artifact_verifier,
      &known_secrets,
    )
    .map_err(rich_approval_runtime_error)?;
    engine.request_rich_approval(run_id, occurred_at, &binding)?;
    return Ok(rich_waiting_outcome(
      engine.workflow().workflow_id.clone(),
      run_id.to_string(),
      approval,
      waiting_expires_at,
      waiting_on_timeout,
      binding,
      0,
      Vec::new(),
    ));
  }
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
      token: Some(token.token),
      credential_expires_at: Some(token.credential_expires_at),
      review_digest: None,
      review: None,
      authority: None,
    },
  }
}

fn rich_waiting_outcome(
  workflow_id: String,
  run_id: String,
  approval: ApprovalDefinition,
  expires_at: Option<chrono::DateTime<chrono::Utc>>,
  on_timeout: ApprovalTimeoutPolicy,
  binding: BoundRichApprovalReview,
  received: u32,
  represented_roles: Vec<String>,
) -> WorkflowRuntimeOutcome {
  WorkflowRuntimeOutcome::Waiting {
    contract: RUNTIME_OUTCOME_CONTRACT,
    version: RUNTIME_OUTCOME_VERSION,
    workflow_id,
    run_id,
    approval: WaitingWorkflowApproval {
      approval_id: approval.approval_id,
      request_id: binding.request_id,
      name: approval.name,
      description: approval.description,
      expires_at,
      on_timeout,
      token: None,
      credential_expires_at: None,
      review_digest: Some(binding.review_digest),
      review: Some(binding.review),
      authority: Some(WaitingRichApprovalAuthority {
        required: binding.required_approvals,
        received,
        required_roles: binding.required_roles,
        represented_roles,
      }),
    },
  }
}

struct RejectRichApprovalArtifacts;

impl RichApprovalArtifactVerifier for RejectRichApprovalArtifacts {
  fn verify_review_artifact(
    &self,
    _run_id: &str,
    _artifact: &crate::ArtifactReferenceV1,
  ) -> Result<crate::ArtifactReferenceV1, RichApprovalError> {
    Err(RichApprovalError::ArtifactInvalid)
  }
}

fn rich_approval_runtime_error(error: RichApprovalError) -> RuntimeExecutionError {
  RuntimeExecutionError::InvalidConfiguration(format!("{}: {error}", error.code()))
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
      let Some(state) = projection.forks.get(&fork.fork_id) else {
        return false;
      };
      if state.join_status != crate::ForkJoinStatus::Pending {
        return false;
      }
      let routes = fork_branch_routes(workflow, fork);
      let has_ready_work = ready.iter().any(|node_id| {
        routes.values().any(|route| route.contains(node_id)) || node_id == &fork.join_node_id
      });
      let has_recovered_failure = fork.branches.iter().any(|branch| {
        !state.branches.contains_key(&branch.branch_id)
          && routes
            .get(&branch.branch_id)
            .is_some_and(|route| final_attempt_failure_in_nodes(projection, route).is_some())
      });
      let needs_join_settlement = fork
        .branches
        .iter()
        .all(|branch| state.branches.contains_key(&branch.branch_id));
      has_ready_work || has_recovered_failure || needs_join_settlement
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
  if let Some(descriptor) = lock_descriptor_for_body_node(workflow, node_id) {
    if let Some(lock) = projection
      .locks
      .get(&descriptor.lock_id)
      .filter(|lock| lock.status == crate::LockStatus::Held)
    {
      context.lock = Some(crate::LockContext {
        id: lock.lock_id.clone(),
        key: lock.key.clone(),
        fencing_token: lock.fencing_token.clone().unwrap_or_default(),
        acquired_at: lock
          .acquired_at
          .map(|value| value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
          .unwrap_or_default(),
      });
    }
  }
  context
}

fn lock_descriptor_for_body_node<'a>(
  workflow: &'a CompiledWorkflowDefinition,
  node_id: &str,
) -> Option<&'a crate::CompiledLock> {
  fn contains_node(
    lock: &crate::CompiledLock,
    node_id: &str,
    edges: &[crate::model::CompiledWorkflowEdge],
  ) -> bool {
    let mut pending = vec![lock.body_entry_node_id.as_str()];
    let mut visited = HashSet::new();
    while let Some(candidate) = pending.pop() {
      if !visited.insert(candidate) {
        continue;
      }
      if candidate == node_id {
        return true;
      }
      if candidate == lock.body_terminal_node_id {
        continue;
      }
      pending.extend(
        edges
          .iter()
          .filter(|edge| edge.from == candidate)
          .map(|edge| edge.to.as_str()),
      );
    }
    false
  }
  workflow
    .graph
    .locks
    .as_deref()
    .unwrap_or_default()
    .iter()
    .find(|lock| {
      contains_node(lock, node_id, &workflow.graph.edges)
        || workflow
          .graph
          .for_each
          .as_deref()
          .unwrap_or_default()
          .iter()
          .any(|loop_| contains_node(lock, node_id, &loop_.body.edges))
    })
}

fn resolve_lock_key(
  expression: &ValueExpression,
  context: &WorkflowContext,
) -> Result<String, RuntimeExecutionError> {
  match expression {
    ValueExpression::Literal { value } => value.as_str().map(str::to_string),
    ValueExpression::ContextReference { .. } => resolve_context_reference(expression, context)
      .ok()
      .and_then(render_template_scalar),
    ValueExpression::Template { parts } => {
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
            .ok_or_else(|| {
              RuntimeExecutionError::InvalidConfiguration(
                "A lock-key reference is unavailable or is not scalar.".to_string(),
              )
            })?;
            rendered.push_str(&value);
          }
          TemplatePart::LifecycleReference { .. } => {
            return Err(RuntimeExecutionError::InvalidConfiguration(
              "Lock keys cannot use lifecycle references.".to_string(),
            ))
          }
        }
      }
      Some(rendered)
    }
    _ => None,
  }
  .filter(|key| {
    !key.is_empty()
      && key.len() <= 256
      && key.trim() == key
      && !key.chars().any(char::is_control)
      && !key.split('/').any(str::is_empty)
  })
  .ok_or_else(|| {
    RuntimeExecutionError::InvalidConfiguration(
      "The lock key did not resolve to a valid scalar key.".to_string(),
    )
  })
}

fn resolve_for_each_lock_key(
  expression: &ValueExpression,
  context: &WorkflowContext,
  item: &Value,
  index: u32,
  total: u32,
) -> Result<String, RuntimeExecutionError> {
  let rendered = match expression {
    ValueExpression::Literal { value } => value.as_str().map(str::to_string),
    ValueExpression::ContextReference { .. } => render_template_scalar(resolve_for_each_reference(
      expression, context, item, index, total,
    )?),
    ValueExpression::Template { parts } => {
      let mut output = String::new();
      for part in parts {
        match part {
          TemplatePart::Text { text } => output.push_str(text),
          TemplatePart::ContextReference { path } => {
            let value = resolve_for_each_reference(
              &ValueExpression::ContextReference { path: path.clone() },
              context,
              item,
              index,
              total,
            )?;
            output.push_str(&render_template_scalar(value).ok_or_else(|| {
              RuntimeExecutionError::Stalled(
                "Iteration lock key contains a non-scalar context value.".to_string(),
              )
            })?);
          }
          TemplatePart::LifecycleReference { .. } => {
            return Err(RuntimeExecutionError::Stalled(
              "Lock keys cannot contain lifecycle references.".to_string(),
            ));
          }
        }
      }
      Some(output)
    }
    _ => None,
  }
  .ok_or_else(|| {
    RuntimeExecutionError::Stalled("Iteration lock key did not resolve to a string.".to_string())
  })?;
  if rendered.is_empty()
    || rendered.len() > 256
    || rendered.trim() != rendered
    || rendered.chars().any(char::is_control)
    || rendered.split('/').any(str::is_empty)
  {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "The iteration lock key did not resolve to a valid scalar key.".to_string(),
    ));
  }
  Ok(rendered)
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
  let host_client = host;
  let routes = fork_branch_routes(engine.workflow(), &fork);
  let mut active: Vec<ActiveParallelInvocation<'_>> = Vec::new();
  let mut completion_order = Vec::new();
  let mut cancellation_sent = false;
  let mut suspended_outcome = None;

  loop {
    settle_workflow_timeout_if_due(engine, run_id, options.clock.now(), options)?;
    let mut projection = engine.projection(run_id)?;
    if projection.timeout_reached_at.is_some() {
      let invocation_ids = active
        .iter()
        .map(|invocation| invocation.invocation_id.clone())
        .collect::<Vec<_>>();
      for invocation_id in invocation_ids {
        let _ = host_client.cancel_run(&invocation_id).await;
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
        let _ = host_client.cancel_run(&invocation_id).await;
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
          correct_saga_failure_for_node(engine, run_id, &failure.node_id, options).await?;
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
        "engine.lock-acquire" => {
          let lock = engine
            .workflow()
            .graph
            .locks
            .as_deref()
            .unwrap_or_default()
            .iter()
            .find(|lock| lock.acquire_node_id == *node_id)
            .cloned()
            .ok_or_else(|| {
              RuntimeExecutionError::Stalled(
                "Fork lock acquire node has no compiled descriptor.".to_string(),
              )
            })?;
          if !projection.locks.contains_key(&lock.lock_id) {
            let key = resolve_lock_key(&lock.key, &projection.context)?;
            let now = options.clock.now();
            let deadline = now
              + chrono::Duration::milliseconds(i64::try_from(lock.acquire_timeout_ms).map_err(
                |_| {
                  RuntimeExecutionError::InvalidConfiguration(
                    "Fork lock acquisition timeout exceeds the runtime clock.".to_string(),
                  )
                },
              )?);
            engine.register_lock_waiter(run_id, &lock.lock_id, &key, now, deadline)?;
          }
          let state = engine
            .projection(run_id)?
            .locks
            .get(&lock.lock_id)
            .cloned()
            .ok_or_else(|| {
              RuntimeExecutionError::Stalled(
                "Fork lock waiter disappeared from durable projection.".to_string(),
              )
            })?;
          match state.status {
            crate::LockStatus::Held => {
              options.ensure_lock_renewal(run_id, &lock.lock_id, lock.lease_ms);
              completion_order.push(lock.acquire_node_id);
              pure_node_completed = true;
              break;
            }
            crate::LockStatus::Waiting if options.clock.now() < state.deadline => {}
            crate::LockStatus::Waiting => {
              engine.timeout_lock_waiter(run_id, &lock.lock_id, options.clock.now())?;
              return Err(RuntimeExecutionError::InvalidConfiguration(format!(
                "Fork lock {:?} was not acquired before its deadline.",
                lock.lock_id
              )));
            }
            _ => {
              return Err(RuntimeExecutionError::InvalidConfiguration(format!(
                "Fork lock {:?} did not acquire safely.",
                lock.lock_id
              )));
            }
          }
        }
        "engine.lock-release" => {
          let lock = engine
            .workflow()
            .graph
            .locks
            .as_deref()
            .unwrap_or_default()
            .iter()
            .find(|lock| lock.release_node_id == *node_id)
            .cloned()
            .ok_or_else(|| {
              RuntimeExecutionError::Stalled(
                "Fork lock release node has no compiled descriptor.".to_string(),
              )
            })?;
          let held = projection
            .locks
            .get(&lock.lock_id)
            .cloned()
            .ok_or_else(|| {
              RuntimeExecutionError::Stalled("Fork lock release has no lease.".to_string())
            })?;
          let acquired_at = held.acquired_at.ok_or_else(|| {
            RuntimeExecutionError::Stalled(
              "Fork lock release has no acquisition timestamp.".to_string(),
            )
          })?;
          let released_at = options.clock.now();
          let result = serde_json::json!({
            "status": "released", "key": held.key, "fencingToken": held.fencing_token,
            "acquiredAt": acquired_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "releasedAt": released_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "waitedMs": (acquired_at - held.registered_at).num_milliseconds().max(0),
          });
          engine.release_lock(run_id, &lock.lock_id, result, released_at)?;
          options.stop_lock_renewal(run_id, &lock.lock_id);
          completion_order.push(lock.release_node_id);
          pure_node_completed = true;
          break;
        }
        "engine.approval-wait" if active.is_empty() => {
          let outcome = request_approval_wait(engine, run_id, node_id, options)?;
          return Ok(ForkRuntimeProgress::Waiting {
            completed: completion_order,
            outcome,
          });
        }
        "engine.durable-wait" => {
          match register_durable_wait_node(engine, run_id, node_id, options).await? {
            DurableWaitNodeProgress::Resolved => {
              completion_order.push(node_id.clone());
              pure_node_completed = true;
              break;
            }
            DurableWaitNodeProgress::Waiting(outcome) => {
              suspended_outcome.get_or_insert(outcome);
            }
          }
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
          let (choice_id, arm_id) = match selected_choice_arm(engine.workflow(), node_id, &context)
          {
            Ok(selection) => selection,
            Err(error) => {
              return fail_choice(engine, run_id, error);
            }
          };
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
        "engine.choice-result" => {
          let choice = engine
            .workflow()
            .graph
            .choices
            .as_deref()
            .unwrap_or_default()
            .iter()
            .find(|choice| choice.result_node_id.as_deref() == Some(node_id))
            .ok_or_else(|| {
              RuntimeExecutionError::Stalled(format!(
                "choice result {node_id:?} has no compiled choice descriptor"
              ))
            })?;
          let arm_id = projection
            .choice_selections
            .get(&choice.choice_id)
            .cloned()
            .ok_or_else(|| {
              RuntimeExecutionError::Stalled(format!(
                "choice result {node_id:?} became ready without a recorded selection"
              ))
            })?;
          let node = engine
            .workflow()
            .node(node_id)
            .ok_or_else(|| RuntimeExecutionError::Stalled("ready node disappeared".to_string()))?;
          let ValueExpression::Object { fields } = &node.inputs else {
            return Err(RuntimeExecutionError::Stalled(format!(
              "choice result {node_id:?} has invalid compiled inputs"
            )));
          };
          let expression = fields.get(&arm_id).ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "choice result {node_id:?} has no value for selected arm {arm_id:?}"
            ))
          })?;
          let output =
            resolve_context_reference(expression, &projection.context).map_err(|error| {
              RuntimeExecutionError::Stalled(format!(
                "choice result {node_id:?} cannot read {:?}",
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
      if let Some(mut group) = ready.iter().find_map(|node_id| {
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
        let wait_node_ids = group
          .child_node_ids
          .iter()
          .filter(|child_id| {
            ready.contains(child_id)
              && engine
                .workflow()
                .node(child_id)
                .is_some_and(|node| node.handler == "engine.durable-wait")
          })
          .cloned()
          .collect::<Vec<_>>();
        for wait_node_id in wait_node_ids {
          match register_durable_wait_node(engine, run_id, &wait_node_id, options).await? {
            DurableWaitNodeProgress::Resolved => completion_order.push(wait_node_id),
            DurableWaitNodeProgress::Waiting(outcome) => {
              suspended_outcome.get_or_insert(outcome);
            }
          }
        }
        group.child_node_ids.retain(|child_id| {
          ready.contains(child_id)
            && engine
              .workflow()
              .node(child_id)
              .is_some_and(|node| node.handler == "runtime.script")
        });
        if !group.child_node_ids.is_empty() {
          let completed = execute_parallel_group(engine, run_id, group, options, host).await?;
          completion_order.extend(completed);
        }
        if let Some(outcome) = suspended_outcome.take() {
          return Ok(ForkRuntimeProgress::Waiting {
            completed: completion_order,
            outcome,
          });
        }
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
      ensure_local_script_execution(engine.workflow().node(node_id).ok_or_else(|| {
        RuntimeExecutionError::Stalled(format!("fork branch script {node_id:?} is unavailable"))
      })?)?;
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
      let reusable = reusable_step_binding(engine.workflow(), node_id, &context, &secrets)?;
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
          host_client,
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
              .filter(|module| !module.name.starts_with("__woml_"))
              .map(|module| RuntimeModuleBinding {
                name: module.name.clone(),
                bundle_digest: module.bundle_digest.clone(),
                exports: module.exports.clone(),
              })
              .collect(),
            reusable,
            artifact_store: options.managed_artifact_store.clone(),
          },
        )),
      });
    }

    if active.is_empty() {
      if let Some(outcome) = suspended_outcome.take() {
        return Ok(ForkRuntimeProgress::Waiting {
          completed: completion_order,
          outcome,
        });
      }
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
      let fork_waits_for_lock = projection.locks.values().any(|lock| {
        lock.status == crate::LockStatus::Waiting
          && engine
            .workflow()
            .durable_lock(&lock.lock_id)
            .is_some_and(|descriptor| {
              routes
                .values()
                .any(|route| route.contains(&descriptor.acquire_node_id))
            })
      });
      if fork_waits_for_lock {
        options.suspend_policy_execution_slot().await?;
        tokio::time::sleep(CANCELLATION_POLL_INTERVAL).await;
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
              let _ = host_client.cancel_run(&invocation_id).await;
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
              let _ = host_client.cancel_run(&invocation_id).await;
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
        let completed_node_id = completion.node_id.clone();
        append_step_attempt_success(
          engine,
          run_id,
          StepAttemptSucceededData {
            node_id: completion.node_id,
            attempt: completion.attempt_number,
            invocation_id: completion.invocation_id,
            output: output.clone(),
          },
        )?;
        drive_reusable_step_lifecycle(
          engine,
          run_id,
          &completed_node_id,
          ReusableInvocationOutcome::Succeeded,
          Some(output),
          None,
          options,
          host_client,
        )
        .await?;
      }
      Err(failure) => {
        let code = failure.code.clone();
        let lifecycle_error = ReusableLifecycleErrorV1 {
          code: failure.code.clone(),
          message: failure.message.clone(),
        };
        let cancelled = failure.kind == AttemptFailureKind::InvocationCancelled;
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
          StepFailureDisposition::StepFailed => {
            drive_reusable_step_lifecycle(
              engine,
              run_id,
              &completion.node_id,
              if cancelled {
                ReusableInvocationOutcome::Cancelled
              } else {
                ReusableInvocationOutcome::Failed
              },
              None,
              (!cancelled).then_some(lifecycle_error),
              options,
              host_client,
            )
            .await?;
          }
          StepFailureDisposition::RunFailureDeferred => {
            return Err(RuntimeExecutionError::Stalled(
              "fork-owned work deferred a top-level run failure".to_string(),
            ));
          }
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
  reusable: Option<ReusableScriptBindingV3>,
  artifact_store: Option<Arc<crate::ManagedArtifactStore>>,
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
  let request = match invocation.reusable.as_ref() {
    Some(reusable) => ExecuteMessage::reusable_script_with_modules(
      &invocation.invocation_id,
      &invocation.run_id,
      &invocation.node_id,
      attempt,
      invocation.timeout_ms,
      &invocation.source,
      &invocation.context,
      &invocation.secrets,
      &invocation.module_bindings,
      reusable,
    ),
    None => ExecuteMessage::runtime_script_with_modules(
      &invocation.invocation_id,
      &invocation.run_id,
      &invocation.node_id,
      attempt,
      invocation.timeout_ms,
      &invocation.source,
      &invocation.context,
      &invocation.secrets,
      &invocation.module_bindings,
    ),
  };
  let _artifact_cleanup = ArtifactInvocationCleanup::from_store(
    invocation.artifact_store.clone(),
    &invocation.invocation_id,
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
  let host_client = host;
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
        let _ = host_client.cancel_run(&invocation_id).await;
      }
    }
    if cancelling && !run_cancellation_sent {
      run_cancellation_sent = true;
      let active_invocation_ids = active
        .iter()
        .map(|invocation| invocation.invocation_id.clone())
        .collect::<Vec<_>>();
      for invocation_id in active_invocation_ids {
        let _ = host_client.cancel_run(&invocation_id).await;
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
      ensure_local_script_execution(engine.workflow().node(&node_id).ok_or_else(|| {
        RuntimeExecutionError::Stalled(format!("parallel child {node_id:?} is unavailable"))
      })?)?;
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
      let reusable =
        reusable_step_binding(engine.workflow(), &node_id, &invocation_context, &secrets)?;
      active.push(ActiveParallelInvocation {
        node_id: node_id.clone(),
        invocation_id: invocation_id.clone(),
        future: Box::pin(invoke_parallel_child(
          host_client,
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
              .filter(|module| !module.name.starts_with("__woml_"))
              .map(|module| RuntimeModuleBinding {
                name: module.name.clone(),
                bundle_digest: module.bundle_digest.clone(),
                exports: module.exports.clone(),
              })
              .collect(),
            reusable,
            artifact_store: options.managed_artifact_store.clone(),
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
              let _ = host_client.cancel_run(&invocation_id).await;
            }
          } else if current.status == RunStatus::Cancelling {
            run_cancellation_sent = true;
            let active_invocation_ids = active
              .iter()
              .map(|invocation| invocation.invocation_id.clone())
              .collect::<Vec<_>>();
            for invocation_id in active_invocation_ids {
              let _ = host_client.cancel_run(&invocation_id).await;
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
        append_step_attempt_success(
          engine,
          run_id,
          StepAttemptSucceededData {
            node_id: completion.node_id,
            attempt: completion.attempt_number,
            invocation_id: completion.invocation_id,
            output: output.clone(),
          },
        )?;
        drive_reusable_step_lifecycle(
          engine,
          run_id,
          &completed_node_id,
          ReusableInvocationOutcome::Succeeded,
          Some(output),
          None,
          options,
          host_client,
        )
        .await?;
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
        let lifecycle_error = ReusableLifecycleErrorV1 {
          code: failure.code.clone(),
          message: failure.message.clone(),
        };
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
            drive_reusable_step_lifecycle(
              engine,
              run_id,
              &completion.node_id,
              if cancellation {
                ReusableInvocationOutcome::Cancelled
              } else {
                ReusableInvocationOutcome::Failed
              },
              None,
              (!cancellation).then_some(lifecycle_error),
              options,
              host_client,
            )
            .await?;
            if !cancellation && policy == ParallelFailurePolicy::FailFast {
              fail_fast_closed = true;
              let active_invocation_ids = active
                .iter()
                .map(|invocation| invocation.invocation_id.clone())
                .collect::<Vec<_>>();
              for invocation_id in active_invocation_ids {
                let _ = host_client.cancel(&invocation_id).await;
              }
            }
          }
          StepFailureDisposition::RunFailureDeferred => {
            return Err(RuntimeExecutionError::Stalled(
              "parallel-owned work deferred a top-level run failure".to_string(),
            ));
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
    engine.append_payload(
      run_id,
      RunEventPayload::ParallelGroupCompleted(ParallelGroupCompletedData {
        parallel_id: group.parallel_id.clone(),
        outcome: ParallelGroupOutcome::Failed,
        failed_node_ids: failed_node_ids.clone(),
        cancelled_node_ids: cancelled_node_ids.clone(),
      }),
    )?;
    correct_saga_failure_for_node(engine, run_id, &primary_node_id, options).await?;
    engine.append_payload(
      run_id,
      RunEventPayload::RunFailed(RunFailedData::V3(RunFailedDataV3::Parallel {
        parallel_id: group.parallel_id.clone(),
        policy,
        primary_node_id: primary_node_id.clone(),
        failed_node_ids: failed_node_ids.clone(),
        cancelled_node_ids: cancelled_node_ids.clone(),
        failure: failure.clone(),
      })),
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

fn for_each_digest(value: &Value) -> Result<String, RuntimeExecutionError> {
  let bytes = serde_json_canonicalizer::to_vec(value)
    .map_err(|error| RuntimeExecutionError::Stalled(error.to_string()))?;
  Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ForEachTerminalDisposition {
  Failed,
  Cancelled,
}

fn for_each_latest_iteration_failure(
  projection: &RunProjection,
  for_each_id: &str,
  index: u32,
) -> Option<(String, AttemptFailure)> {
  let execution = projection
    .for_each
    .get(for_each_id)?
    .executions
    .get(&index)?;
  execution
    .attempts
    .iter()
    .rev()
    .filter_map(|attempt| match &attempt.status {
      AttemptStatus::Failed { failure } => {
        Some((attempt.identity.node_id.clone(), failure.clone()))
      }
      _ => None,
    })
    .min_by_key(|(_, failure)| matches!(failure.kind, AttemptFailureKind::InvocationCancelled))
}

fn for_each_settlement_digest(
  total: u32,
  succeeded: u32,
  failed: u32,
  skipped: u32,
) -> Result<String, RuntimeExecutionError> {
  for_each_digest(&serde_json::json!({
    "total": total,
    "succeeded": succeeded,
    "failed": failed,
    "skipped": skipped,
  }))
}

fn settle_open_for_each<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  descriptor: &crate::model::CompiledForEach,
  disposition: ForEachTerminalDisposition,
) -> Result<(), RuntimeExecutionError> {
  let projection = engine.projection(run_id)?;
  let Some(loop_state) = projection.for_each.get(&descriptor.for_each_id) else {
    return Ok(());
  };
  if loop_state.status != ForEachStatus::Open {
    return Ok(());
  }

  let active_indexes = loop_state
    .iterations
    .iter()
    .filter_map(|(index, status)| {
      matches!(status, ForEachIterationStatus::Started).then_some(*index)
    })
    .collect::<Vec<_>>();
  for index in active_indexes {
    let current = engine.projection(run_id)?;
    let execution = current
      .for_each
      .get(&descriptor.for_each_id)
      .and_then(|state| state.executions.get(&index))
      .cloned()
      .ok_or_else(|| {
        RuntimeExecutionError::Stalled(format!(
          "for-each {:?} active iteration {index} has no execution projection",
          descriptor.for_each_id
        ))
      })?;
    let scope = ForEachIterationScope {
      for_each_id: descriptor.for_each_id.clone(),
      index,
    };
    for operation in execution
      .operations
      .values()
      .filter(|operation| operation.status == OperationStatus::Started)
    {
      engine.append_scoped_payload(
        run_id,
        scope.clone(),
        RunEventPayload::OperationFailed(OperationFailedData {
          node_id: operation.node_id.clone(),
          attempt_number: operation.attempt_number,
          invocation_id: operation.identity.invocation_id.clone(),
          call_id: operation.identity.call_id.clone(),
          operation_key: operation.operation_key.clone(),
          capability: operation.capability.clone(),
          operation: operation.operation.clone(),
          execution_mode: operation.execution_mode,
          metadata: operation.metadata.clone(),
          duration_ms: 0.0,
          failure: CapabilityFailure {
            kind: CapabilityFailureKind::Ambiguous,
            code: "WOML_CAPABILITY_CANCELLATION_AMBIGUOUS".to_string(),
            message: "Loop settlement found an operation without a durable terminal event; its external outcome is ambiguous and it will not be replayed.".to_string(),
            retryable: false,
            ambiguous: true,
            details: None,
          },
        }),
      )?;
    }
    for attempt in execution
      .attempts
      .iter()
      .filter(|attempt| attempt.status == AttemptStatus::Started)
    {
      let kind = match disposition {
        ForEachTerminalDisposition::Failed => AttemptFailureKind::Interrupted,
        ForEachTerminalDisposition::Cancelled => AttemptFailureKind::InvocationCancelled,
      };
      engine.append_scoped_payload(
        run_id,
        scope.clone(),
        RunEventPayload::StepAttemptFailed(StepAttemptFailedData {
          node_id: attempt.identity.node_id.clone(),
          attempt: attempt.identity.attempt,
          invocation_id: attempt.identity.invocation_id.clone(),
          failure: AttemptFailure {
            kind,
            code: kind.code().to_string(),
            message: "The loop settled before this invocation produced a durable terminal outcome."
              .to_string(),
            details: None,
            ..AttemptFailure::legacy_defaults()
          },
        }),
      )?;
    }
    for group in execution
      .parallel_groups
      .values()
      .filter(|group| group.status == ParallelGroupStatus::Started)
    {
      let start_id = format!("__woml_parallel__{}__start", group.parallel_id);
      let cancelled_node_ids = descriptor
        .body
        .edges
        .iter()
        .filter(|edge| {
          edge.from == start_id && edge.parallel_id.as_deref() == Some(group.parallel_id.as_str())
        })
        .map(|edge| edge.to.clone())
        .collect::<Vec<_>>();
      engine.append_scoped_payload(
        run_id,
        scope.clone(),
        RunEventPayload::ParallelGroupCompleted(ParallelGroupCompletedData {
          parallel_id: group.parallel_id.clone(),
          outcome: ParallelGroupOutcome::Failed,
          failed_node_ids: Vec::new(),
          cancelled_node_ids,
        }),
      )?;
    }
    let current = engine.projection(run_id)?;
    let (failed_node_id, failure_kind) =
      for_each_latest_iteration_failure(&current, &descriptor.for_each_id, index)
        .map(|(node_id, failure)| (node_id, for_each_failure_kind(failure.kind)))
        .unwrap_or_else(|| {
          (
            descriptor.body.terminal_node_id.clone(),
            match disposition {
              ForEachTerminalDisposition::Failed => ForEachIterationFailureKind::Interrupted,
              ForEachTerminalDisposition::Cancelled => ForEachIterationFailureKind::Cancelled,
            },
          )
        });
    engine.append_scoped_payload(
      run_id,
      scope,
      RunEventPayload::ForEachIterationFailed(ForEachIterationFailedData {
        for_each_id: descriptor.for_each_id.clone(),
        failed_node_id,
        failure_kind,
      }),
    )?;
  }

  let projection = engine.projection(run_id)?;
  let loop_state = projection
    .for_each
    .get(&descriptor.for_each_id)
    .ok_or_else(|| RuntimeExecutionError::Stalled("for-each disappeared".to_string()))?;
  let skip_reason = match disposition {
    ForEachTerminalDisposition::Failed => ForEachSkipReason::Failure,
    ForEachTerminalDisposition::Cancelled => ForEachSkipReason::Cancellation,
  };
  let pending_indexes = (0..loop_state.total)
    .filter(|index| !loop_state.iterations.contains_key(index))
    .collect::<Vec<_>>();
  for index in pending_indexes {
    engine.append_scoped_payload(
      run_id,
      ForEachIterationScope {
        for_each_id: descriptor.for_each_id.clone(),
        index,
      },
      RunEventPayload::ForEachIterationSkipped(ForEachIterationSkippedData {
        for_each_id: descriptor.for_each_id.clone(),
        reason: skip_reason,
      }),
    )?;
  }

  let projection = engine.projection(run_id)?;
  let loop_state = projection
    .for_each
    .get(&descriptor.for_each_id)
    .ok_or_else(|| RuntimeExecutionError::Stalled("for-each disappeared".to_string()))?;
  let succeeded = loop_state
    .iterations
    .values()
    .filter(|status| matches!(status, ForEachIterationStatus::Succeeded { .. }))
    .count() as u32;
  let failed = loop_state
    .iterations
    .values()
    .filter(|status| matches!(status, ForEachIterationStatus::Failed { .. }))
    .count() as u32;
  let skipped = loop_state
    .iterations
    .values()
    .filter(|status| matches!(status, ForEachIterationStatus::Skipped))
    .count() as u32;
  let settled = ForEachSettledData {
    for_each_id: descriptor.for_each_id.clone(),
    total: loop_state.total,
    succeeded,
    failed,
    skipped,
    aggregate_digest: for_each_settlement_digest(loop_state.total, succeeded, failed, skipped)?,
  };
  let payload = match disposition {
    ForEachTerminalDisposition::Failed => RunEventPayload::ForEachFailed(settled),
    ForEachTerminalDisposition::Cancelled => RunEventPayload::ForEachCancelled(settled),
  };
  engine.append_payload(run_id, payload)
}

/// Serializes durable event/projection access while allowing iteration futures
/// to wait on the multiplexed Bun host concurrently. The compiled definition is
/// immutable and is shared without taking the engine lock.
struct SharedForEachEngine<'a, E> {
  inner: Arc<std::sync::Mutex<&'a mut E>>,
  workflow: Arc<CompiledWorkflowDefinition>,
  definition_hash: Arc<String>,
}

impl<E> Clone for SharedForEachEngine<'_, E> {
  fn clone(&self) -> Self {
    Self {
      inner: Arc::clone(&self.inner),
      workflow: Arc::clone(&self.workflow),
      definition_hash: Arc::clone(&self.definition_hash),
    }
  }
}

impl<'a, E: RuntimeDagEngine> SharedForEachEngine<'a, E> {
  fn new(engine: &'a mut E) -> Self {
    Self {
      workflow: Arc::new(engine.workflow().clone()),
      definition_hash: Arc::new(engine.definition_hash().to_string()),
      inner: Arc::new(std::sync::Mutex::new(engine)),
    }
  }

  fn with_engine<T>(
    &self,
    operation: impl FnOnce(&mut E) -> Result<T, RuntimeExecutionError>,
  ) -> Result<T, RuntimeExecutionError> {
    let mut engine = self.inner.lock().map_err(|_| {
      RuntimeExecutionError::Stalled("the shared for-each event authority was poisoned".to_string())
    })?;
    operation(&mut **engine)
  }
}

pub(crate) fn for_each_effect_key(
  run_id: &str,
  definition_hash: &str,
  for_each_id: &str,
  index: u32,
  node_id: &str,
) -> String {
  let identity = serde_json::json!({
    "contract": "woml.for-each-effect",
    "version": 1,
    "runId": run_id,
    "definitionHash": definition_hash,
    "forEachId": for_each_id,
    "index": index,
    "nodeId": node_id,
  });
  let bytes = serde_json_canonicalizer::to_vec(&identity)
    .expect("a static for-each effect identity is canonical JSON");
  format!("sha256:{:x}", Sha256::digest(bytes))
}

fn resolve_for_each_reference(
  expression: &ValueExpression,
  context: &WorkflowContext,
  item: &Value,
  index: u32,
  total: u32,
) -> Result<Value, RuntimeExecutionError> {
  let ValueExpression::ContextReference { path } = expression else {
    return Err(RuntimeExecutionError::Stalled(
      "for-each result is not a context reference".to_string(),
    ));
  };
  if matches!(path.first().map(String::as_str), Some("trigger" | "steps")) {
    return resolve_context_reference(expression, context).map_err(|_| {
      RuntimeExecutionError::Stalled(format!(
        "for-each result reference context.{} is unavailable",
        path.join(".")
      ))
    });
  }
  let (mut value, remaining): (Value, &[String]) = match path.first().map(String::as_str) {
    Some("item") => (item.clone(), &path[1..]),
    Some("iteration") => (
      serde_json::json!({ "index": index, "total": total }),
      &path[1..],
    ),
    _ => {
      return Err(RuntimeExecutionError::Stalled(
        "for-each result has an unsupported context root".to_string(),
      ));
    }
  };
  for segment in remaining {
    value = value
      .as_object()
      .and_then(|object| object.get(segment))
      .cloned()
      .ok_or_else(|| {
        RuntimeExecutionError::Stalled(format!(
          "for-each result reference context.{} is unavailable",
          path.join(".")
        ))
      })?;
  }
  Ok(value)
}

fn for_each_selected_choice(
  descriptor: &crate::model::CompiledForEach,
  selector_id: &str,
  context: &WorkflowContext,
  item: &Value,
  index: u32,
  total: u32,
) -> Result<(String, String), RuntimeExecutionError> {
  let choice = descriptor
    .body
    .choices
    .iter()
    .find(|choice| choice.selector_node_id == selector_id)
    .ok_or_else(|| {
      RuntimeExecutionError::Stalled(format!(
        "for-each choice selector {selector_id:?} has no descriptor"
      ))
    })?;
  if let Some(selector) = &choice.string_selector {
    let selected = resolve_for_each_reference(selector, context, item, index, total)?;
    let Value::String(selected) = selected else {
      return Err(RuntimeExecutionError::InvalidConfiguration(format!(
        "for-each switch {:?} must resolve to a string",
        choice.choice_id
      )));
    };
    let arm_id = choice
      .string_cases
      .as_deref()
      .unwrap_or_default()
      .iter()
      .find(|case| case.value == selected)
      .map(|case| case.arm_id.clone())
      .or_else(|| choice.default_arm_id.clone())
      .ok_or_else(|| {
        RuntimeExecutionError::InvalidConfiguration(format!(
          "for-each switch {:?} has no matching case or default",
          choice.choice_id
        ))
      })?;
    return Ok((choice.choice_id.clone(), arm_id));
  }
  for edge in descriptor
    .body
    .edges
    .iter()
    .filter(|edge| edge.from == selector_id && choice.arm_ids.contains(&edge.id))
  {
    match &edge.condition {
      crate::model::EdgeCondition::Boolean { value } => {
        match resolve_for_each_reference(value, context, item, index, total)? {
          Value::Bool(true) => return Ok((choice.choice_id.clone(), edge.id.clone())),
          Value::Bool(false) => {}
          _ => {
            return Err(RuntimeExecutionError::InvalidConfiguration(format!(
              "for-each choice {:?} condition must resolve to a boolean",
              choice.choice_id
            )))
          }
        }
      }
      crate::model::EdgeCondition::Always => {
        return Ok((choice.choice_id.clone(), edge.id.clone()))
      }
      _ => {
        return Err(RuntimeExecutionError::Stalled(format!(
          "for-each choice {:?} contains an unsupported condition",
          choice.choice_id
        )))
      }
    }
  }
  Err(RuntimeExecutionError::InvalidConfiguration(format!(
    "for-each choice {:?} has no selectable route",
    choice.choice_id
  )))
}

fn for_each_selected_branch(
  descriptor: &crate::model::CompiledForEach,
  selector_node_id: &str,
  context: &WorkflowContext,
  item: &Value,
  index: u32,
  total: u32,
) -> Result<(String, String), RuntimeExecutionError> {
  let branch_id = selector_node_id
    .strip_prefix("__woml_branch__")
    .and_then(|value| value.strip_suffix("__select"))
    .ok_or_else(|| {
      RuntimeExecutionError::InvalidConfiguration(format!(
        "for-each conditional selector {selector_node_id:?} has no canonical identity"
      ))
    })?;
  for edge in descriptor
    .body
    .edges
    .iter()
    .filter(|edge| edge.from == selector_node_id)
  {
    match &edge.condition {
      crate::model::EdgeCondition::Boolean { value } => {
        let selected = resolve_for_each_reference(value, context, item, index, total)?;
        match selected {
          Value::Bool(true) => return Ok((branch_id.to_string(), edge.id.clone())),
          Value::Bool(false) => {}
          _ => {
            return Err(RuntimeExecutionError::InvalidConfiguration(format!(
              "for-each condition for branch {branch_id:?} must resolve to a boolean"
            )))
          }
        }
      }
      crate::model::EdgeCondition::Always => return Ok((branch_id.to_string(), edge.id.clone())),
      _ => {
        return Err(RuntimeExecutionError::InvalidConfiguration(format!(
          "for-each branch {branch_id:?} uses an unsupported condition"
        )))
      }
    }
  }
  Err(RuntimeExecutionError::InvalidConfiguration(format!(
    "for-each branch {branch_id:?} has no selectable route"
  )))
}

fn for_each_edge_is_active(
  descriptor: &crate::model::CompiledForEach,
  edge: &crate::model::CompiledWorkflowEdge,
  selections: &BTreeMap<String, String>,
) -> bool {
  if let Some(branch_id) = edge.branch_id.as_deref() {
    return selections
      .get(branch_id)
      .is_some_and(|selected| selected == &edge.id);
  }
  if let Some(result_node) = descriptor
    .body
    .nodes
    .iter()
    .find(|node| node.id == edge.to && node.handler == "engine.branch-result")
  {
    if let Some(selected_arm) = selections.get(&result_node.id) {
      if let ValueExpression::Object { fields } = &result_node.inputs {
        if let Some(ValueExpression::ContextReference { path }) = fields.get(selected_arm) {
          if let [root, selected_node, ..] = path.as_slice() {
            if root == "steps" {
              return edge.from == *selected_node;
            }
          }
        }
      }
    }
  }
  for choice in &descriptor.body.choices {
    if choice.arm_ids.contains(&edge.id) {
      return selections
        .get(&choice.choice_id)
        .is_some_and(|selected| selected == &edge.id);
    }
    if let Some(selected) = selections.get(&choice.choice_id) {
      if edge.id == format!("{selected}:join") {
        return true;
      }
      if edge.to == choice.join_node_id && edge.id.ends_with(":join") {
        return false;
      }
    } else if edge.to == choice.join_node_id && edge.id.ends_with(":join") {
      return false;
    }
  }
  true
}

fn for_each_ready_nodes(
  descriptor: &crate::model::CompiledForEach,
  completed: &HashSet<String>,
  selections: &BTreeMap<String, String>,
) -> Vec<String> {
  descriptor
    .body
    .nodes
    .iter()
    .filter(|node| !completed.contains(&node.id))
    .filter(|node| {
      let incoming = descriptor
        .body
        .edges
        .iter()
        .filter(|edge| edge.to == node.id)
        .filter(|edge| for_each_edge_is_active(descriptor, edge, selections))
        .collect::<Vec<_>>();
      if incoming.is_empty() {
        descriptor.body.entry_node_ids.contains(&node.id)
      } else {
        incoming.iter().all(|edge| completed.contains(&edge.from))
      }
    })
    .map(|node| node.id.clone())
    .collect()
}

fn for_each_module_bindings(options: &RuntimeExecutionOptions) -> Vec<RuntimeModuleBinding> {
  options
    .runtime_modules
    .iter()
    .filter(|module| !module.name.starts_with("__woml_"))
    .map(|module| RuntimeModuleBinding {
      name: module.name.clone(),
      bundle_digest: module.bundle_digest.clone(),
      exports: module.exports.clone(),
    })
    .collect()
}

#[allow(clippy::too_many_arguments)]
async fn drive_for_each_step_lifecycle<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  descriptor: &crate::model::CompiledForEach,
  node_id: &str,
  event: LifecycleEventName,
  context: &WorkflowContext,
  item: &Value,
  index: u32,
  total: u32,
  attempts: u32,
  failure: Option<&AttemptFailure>,
  options: &RuntimeExecutionOptions,
  host: &ScriptHostClient,
) -> Result<(), RuntimeExecutionError> {
  let Some(hook) = engine
    .workflow()
    .lifecycle_hook_for_step_event(event, node_id)
    .cloned()
  else {
    return Ok(());
  };
  let scope = ForEachIterationScope {
    for_each_id: descriptor.for_each_id.clone(),
    index,
  };
  let subject_id = format!("{}[{index}].{node_id}", descriptor.for_each_id);
  let hook_invocation_id = crate::derive_lifecycle_hook_invocation_id(
    run_id,
    &hook.hook_id,
    LifecycleSubjectKind::Step,
    &subject_id,
  );
  engine.append_scoped_payload(
    run_id,
    scope.clone(),
    RunEventPayload::LifecycleHookRequested(LifecycleHookRequestedData {
      hook_invocation_id: hook_invocation_id.clone(),
      hook_id: hook.hook_id.clone(),
      event,
      subject: LifecycleSubject {
        kind: LifecycleSubjectKind::Step,
        id: subject_id,
      },
    }),
  )?;
  let binding = LifecycleBindingV1 {
    event,
    workflow: LifecycleWorkflowBindingV1 {
      id: engine.workflow().workflow_id.clone(),
      outcome: None,
    },
    step: Some(LifecycleStepBindingV1 {
      id: node_id.to_string(),
      outcome: match event {
        LifecycleEventName::StepSuccess => Some(crate::event::BusinessOutcome::Succeeded),
        LifecycleEventName::StepFailure => Some(crate::event::BusinessOutcome::Failed),
        _ => None,
      },
      attempts,
    }),
    failure: failure.map(|failure| LifecycleFailureBindingV1 {
      code: failure.code.clone(),
      message: failure.message.clone(),
    }),
  };
  let modules = for_each_module_bindings(options);
  let mut failed_actions = 0_u32;
  for action in &hook.actions {
    engine.append_scoped_payload(
      run_id,
      scope.clone(),
      RunEventPayload::LifecycleActionAttemptStarted(LifecycleActionIdentityData {
        hook_invocation_id: hook_invocation_id.clone(),
        action_id: action.action_id.clone(),
        attempt: 1,
      }),
    )?;
    let execution = if action.handler == "runtime.lifecycle-script" {
      let source = lifecycle_action_source(action).ok_or_else(|| {
        RuntimeExecutionError::InvalidConfiguration(format!(
          "for-each lifecycle action {:?} has no script source",
          action.action_id
        ))
      })?;
      let secrets = resolved_lifecycle_secrets(action, options)?;
      let idempotency_key = for_each_effect_key(
        run_id,
        engine.definition_hash(),
        &descriptor.for_each_id,
        index,
        &action.action_id,
      );
      let invocation_id = generated_id("lcinv");
      let request = ExecuteMessage::lifecycle_script_with_iteration_modules(
        &invocation_id,
        run_id,
        &action.action_id,
        ScriptAttempt::new(1, 1, &idempotency_key).map_err(RuntimeExecutionError::Stalled)?,
        options.script_timeout_ms,
        source,
        context,
        item,
        index,
        total,
        &binding,
        &secrets,
        &modules,
      )
      .map_err(RuntimeExecutionError::Stalled)?;
      let _artifact_cleanup = ArtifactInvocationCleanup::new(options, &invocation_id);
      host.execute(&request).await
    } else {
      Err(ScriptHostClientError::Protocol(format!(
        "iteration-scoped lifecycle action {:?} uses unsupported handler {:?}",
        action.action_id, action.handler
      )))
    };
    let payload = match execution {
      Ok(completed) if matches!(completed.outcome, HostOutcome::Success { .. }) => {
        RunEventPayload::LifecycleActionSucceeded(LifecycleActionIdentityData {
          hook_invocation_id: hook_invocation_id.clone(),
          action_id: action.action_id.clone(),
          attempt: 1,
        })
      }
      Ok(completed) => {
        failed_actions += 1;
        let HostOutcome::Failure { error } = completed.outcome else {
          unreachable!()
        };
        RunEventPayload::LifecycleActionFailed(LifecycleActionFailedData {
          hook_invocation_id: hook_invocation_id.clone(),
          action_id: action.action_id.clone(),
          attempt: 1,
          failure: lifecycle_failure_from_attempt(error.into_attempt_failure()),
        })
      }
      Err(error) => {
        failed_actions += 1;
        RunEventPayload::LifecycleActionFailed(LifecycleActionFailedData {
          hook_invocation_id: hook_invocation_id.clone(),
          action_id: action.action_id.clone(),
          attempt: 1,
          failure: LifecycleFailure {
            kind: LifecycleFailureKind::HostCrashed,
            code: "WOML_FOR_EACH_LIFECYCLE_FAILED".to_string(),
            message: error.to_string(),
          },
        })
      }
    };
    engine.append_scoped_payload(run_id, scope.clone(), payload)?;
  }
  engine.append_scoped_payload(
    run_id,
    scope,
    RunEventPayload::LifecycleHookCompleted(LifecycleHookCompletedData {
      hook_invocation_id,
      status: if failed_actions == 0 {
        LifecycleHookCompletionStatus::Completed
      } else {
        LifecycleHookCompletionStatus::CompletedWithWarnings
      },
      failed_actions,
    }),
  )?;
  Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn drive_for_each_reusable_lifecycle<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  loop_descriptor: &crate::model::CompiledForEach,
  node_id: &str,
  context: &WorkflowContext,
  item: &Value,
  index: u32,
  total: u32,
  outcome: ReusableInvocationOutcome,
  result: Option<Value>,
  error: Option<ReusableLifecycleErrorV1>,
  options: &RuntimeExecutionOptions,
  host: &ScriptHostClient,
) -> Result<(), RuntimeExecutionError> {
  let Some(descriptor) = reusable_step_descriptor(engine.workflow(), node_id).cloned() else {
    return Ok(());
  };
  let CompiledReusableInvocation::Step {
    invocation_id,
    definition_digest,
    lifecycle,
    ..
  } = &descriptor
  else {
    return Ok(());
  };
  let Some(lifecycle) = lifecycle else {
    return Ok(());
  };
  let hooks = match outcome {
    ReusableInvocationOutcome::Succeeded => vec![
      (
        ProtocolReusableLifecycleHook::OnSuccess,
        crate::ReusableLifecycleHook::OnSuccess,
        lifecycle.on_success.clone().unwrap_or_default(),
      ),
      (
        ProtocolReusableLifecycleHook::OnComplete,
        crate::ReusableLifecycleHook::OnComplete,
        lifecycle.on_complete.clone().unwrap_or_default(),
      ),
    ],
    ReusableInvocationOutcome::Failed => vec![
      (
        ProtocolReusableLifecycleHook::OnError,
        crate::ReusableLifecycleHook::OnError,
        lifecycle.on_error.clone().unwrap_or_default(),
      ),
      (
        ProtocolReusableLifecycleHook::OnComplete,
        crate::ReusableLifecycleHook::OnComplete,
        lifecycle.on_complete.clone().unwrap_or_default(),
      ),
    ],
    ReusableInvocationOutcome::Cancelled => vec![(
      ProtocolReusableLifecycleHook::OnComplete,
      crate::ReusableLifecycleHook::OnComplete,
      lifecycle.on_complete.clone().unwrap_or_default(),
    )],
  };
  let scope = ForEachIterationScope {
    for_each_id: loop_descriptor.for_each_id.clone(),
    index,
  };
  let secrets = resolved_reusable_descriptor_secrets(&descriptor, options)?;
  let reusable =
    reusable_definition_binding_for_iteration(&descriptor, context, item, index, total, &secrets)?;
  let modules = for_each_module_bindings(options);
  for (protocol_hook, event_hook, actions) in hooks {
    if actions.is_empty() {
      continue;
    }
    engine.append_scoped_payload(
      run_id,
      scope.clone(),
      RunEventPayload::ReusableLifecycleRequested(crate::ReusableLifecycleRequestedData {
        invocation_id: invocation_id.clone(),
        definition_digest: definition_digest.clone(),
        hook: event_hook,
      }),
    )?;
    let binding = ReusableLifecycleBindingV1 {
      hook: protocol_hook,
      outcome,
      result: result.clone(),
      error: error.clone(),
    };
    let mut warnings = 0_u32;
    for action_id in actions {
      let artifact_name = reusable_lifecycle_artifact_name(&action_id);
      let source = options
        .runtime_modules
        .iter()
        .find(|artifact| artifact.name == artifact_name)
        .map(|artifact| artifact.bundle.clone())
        .ok_or_else(|| {
          RuntimeExecutionError::InvalidConfiguration(format!(
            "reusable lifecycle artifact for {action_id:?} is unavailable"
          ))
        })?;
      engine.append_scoped_payload(
        run_id,
        scope.clone(),
        RunEventPayload::ReusableLifecycleActionStarted(
          crate::ReusableLifecycleActionStartedData {
            invocation_id: invocation_id.clone(),
            definition_digest: definition_digest.clone(),
            hook: event_hook,
            action_id: action_id.clone(),
          },
        ),
      )?;
      let host_invocation_id = generated_id("rlcinv");
      let idempotency_key = for_each_effect_key(
        run_id,
        engine.definition_hash(),
        &loop_descriptor.for_each_id,
        index,
        &action_id,
      );
      let request = ExecuteMessage::reusable_lifecycle_script_with_iteration_modules(
        &host_invocation_id,
        run_id,
        &action_id,
        ScriptAttempt::new(1, 1, &idempotency_key).map_err(RuntimeExecutionError::Stalled)?,
        options.script_timeout_ms,
        &source,
        context,
        item,
        index,
        total,
        &secrets,
        &modules,
        &reusable,
        &binding,
      )
      .map_err(RuntimeExecutionError::Stalled)?;
      let _artifact_cleanup = ArtifactInvocationCleanup::new(options, &host_invocation_id);
      let succeeded = host
        .execute(&request)
        .await
        .is_ok_and(|completed| matches!(completed.outcome, HostOutcome::Success { .. }));
      let payload = if succeeded {
        RunEventPayload::ReusableLifecycleActionSucceeded(
          crate::ReusableLifecycleActionSucceededData {
            invocation_id: invocation_id.clone(),
            definition_digest: definition_digest.clone(),
            hook: event_hook,
            action_id,
            outcome: crate::ReusableLifecycleOutcome::Succeeded,
          },
        )
      } else {
        warnings += 1;
        RunEventPayload::ReusableLifecycleActionFailed(crate::ReusableLifecycleActionFailedData {
          invocation_id: invocation_id.clone(),
          definition_digest: definition_digest.clone(),
          hook: event_hook,
          action_id,
          outcome: crate::ReusableLifecycleOutcome::Failed,
          warning_code: "WOML_REUSABLE_LIFECYCLE_ACTION_FAILED".to_string(),
        })
      };
      engine.append_scoped_payload(run_id, scope.clone(), payload)?;
    }
    engine.append_scoped_payload(
      run_id,
      scope.clone(),
      RunEventPayload::ReusableLifecycleCompleted(crate::ReusableLifecycleCompletedData {
        invocation_id: invocation_id.clone(),
        definition_digest: definition_digest.clone(),
        hook: event_hook,
        outcome: if warnings == 0 {
          crate::ReusableLifecycleOutcome::Succeeded
        } else {
          crate::ReusableLifecycleOutcome::CompletedWithWarnings
        },
      }),
    )?;
  }
  Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn execute_for_each_script<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  descriptor: &crate::model::CompiledForEach,
  node: &crate::model::CompiledWorkflowNode,
  context: &WorkflowContext,
  item: &Value,
  index: u32,
  total: u32,
  options: &RuntimeExecutionOptions,
  host: &ScriptHostClient,
  active_invocations: &Arc<tokio::sync::Mutex<HashSet<String>>>,
) -> Result<Value, AttemptFailure> {
  if let Err(error) = ensure_local_script_execution(node) {
    return Err(AttemptFailure {
      kind: AttemptFailureKind::Interrupted,
      code: "WOML_REMOTE_DISPATCH_UNAVAILABLE".to_string(),
      message: error.to_string(),
      retryable: Some(false),
      ambiguous: Some(false),
      details: None,
      ..AttemptFailure::legacy_defaults()
    });
  }
  let workflow = engine.workflow().clone();
  let definition_hash = engine.definition_hash().to_string();
  let source = node.script_source().ok_or_else(|| AttemptFailure {
    kind: AttemptFailureKind::InvalidScriptResult,
    code: "WOML_FOR_EACH_NODE_INVALID".to_string(),
    message: format!("For-each body node {:?} has no script source.", node.id),
    details: None,
    ..AttemptFailure::legacy_defaults()
  })?;
  let secrets = node
    .script_runtime
    .as_ref()
    .map(|runtime| {
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
            .ok_or_else(|| AttemptFailure {
              kind: AttemptFailureKind::ScriptThrew,
              code: "WOML_SECRET_NOT_FOUND".to_string(),
              message: format!("Required secret {name:?} is unavailable."),
              details: None,
              ..AttemptFailure::legacy_defaults()
            })
        })
        .collect::<Result<BTreeMap<_, _>, _>>()
    })
    .transpose()?
    .unwrap_or_default();
  let modules = for_each_module_bindings(options);
  let max_attempts = node
    .retry_policy
    .as_ref()
    .map_or(1, |policy| policy.max_attempts);
  let idempotency_key = for_each_effect_key(
    run_id,
    &definition_hash,
    &descriptor.for_each_id,
    index,
    &node.id,
  );
  let execution = engine
    .projection(run_id)
    .map_err(|error| AttemptFailure {
      kind: AttemptFailureKind::Interrupted,
      code: "WOML_FOR_EACH_RECOVERY_INVALID".to_string(),
      message: error.to_string(),
      details: None,
      ..AttemptFailure::legacy_defaults()
    })?
    .for_each
    .get(&descriptor.for_each_id)
    .and_then(|loop_state| loop_state.executions.get(&index))
    .cloned()
    .ok_or_else(|| AttemptFailure {
      kind: AttemptFailureKind::Interrupted,
      code: "WOML_FOR_EACH_RECOVERY_INVALID".to_string(),
      message: "The active iteration has no durable execution projection.".to_string(),
      details: None,
      ..AttemptFailure::legacy_defaults()
    })?;
  let latest_attempt = execution
    .attempts
    .iter()
    .rev()
    .find(|attempt| attempt.identity.node_id == node.id);
  let start_attempt = match latest_attempt {
    None => 1,
    Some(attempt) => match &attempt.status {
      AttemptStatus::Succeeded { output } => return Ok(output.clone()),
      AttemptStatus::Started => {
        return Err(AttemptFailure {
          kind: AttemptFailureKind::Interrupted,
          code: AttemptFailureKind::Interrupted.code().to_string(),
          message: "Recovery found an active iteration attempt without a durable outcome."
            .to_string(),
          details: None,
          ..AttemptFailure::legacy_defaults()
        });
      }
      AttemptStatus::Failed { failure } => {
        let Some(retry) = execution.pending_retries.get(&node.id) else {
          return Err(failure.clone());
        };
        while retry.scheduled_at > options.clock.now() {
          let wait = (retry.scheduled_at - options.clock.now())
            .to_std()
            .unwrap_or(CANCELLATION_POLL_INTERVAL)
            .min(CANCELLATION_POLL_INTERVAL);
          tokio::time::sleep(wait).await;
          let projection = engine.projection(run_id).map_err(|error| AttemptFailure {
            kind: AttemptFailureKind::Interrupted,
            code: "WOML_FOR_EACH_RECOVERY_INVALID".to_string(),
            message: error.to_string(),
            details: None,
            ..AttemptFailure::legacy_defaults()
          })?;
          if projection.status == RunStatus::Cancelling || projection.timeout_reached_at.is_some() {
            return Err(AttemptFailure {
              kind: AttemptFailureKind::InvocationCancelled,
              code: AttemptFailureKind::InvocationCancelled.code().to_string(),
              message: "The pending iteration retry was cancelled with its workflow run."
                .to_string(),
              details: None,
              ..AttemptFailure::legacy_defaults()
            });
          }
        }
        retry.next_attempt
      }
    },
  };
  for attempt_number in start_attempt..=max_attempts {
    let invocation_id = generated_id("inv");
    let attempt =
      ScriptAttempt::new(attempt_number, max_attempts, &idempotency_key).map_err(|message| {
        AttemptFailure {
          kind: AttemptFailureKind::InvalidScriptResult,
          code: "WOML_FOR_EACH_ATTEMPT_INVALID".to_string(),
          message,
          details: None,
          ..AttemptFailure::legacy_defaults()
        }
      })?;
    let reusable = reusable_step_binding_for_iteration(
      &workflow, &node.id, context, item, index, total, &secrets,
    )
    .map_err(|error| AttemptFailure {
      kind: AttemptFailureKind::ScriptThrew,
      code: "WOML_REUSABLE_BINDING_INVALID".to_string(),
      message: error.to_string(),
      details: None,
      ..AttemptFailure::legacy_defaults()
    })?;
    let scope = ForEachIterationScope {
      for_each_id: descriptor.for_each_id.clone(),
      index,
    };
    engine
      .append_scoped_payload(
        run_id,
        scope.clone(),
        RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
          node_id: node.id.clone(),
          attempt: attempt_number,
          invocation_id: invocation_id.clone(),
          handler: node.handler.clone(),
          idempotency_key: Some(idempotency_key.clone()),
        }),
      )
      .map_err(|error| AttemptFailure {
        kind: AttemptFailureKind::Interrupted,
        code: "WOML_FOR_EACH_EVENT_FAILED".to_string(),
        message: error.to_string(),
        details: None,
        ..AttemptFailure::legacy_defaults()
      })?;
    let request = match reusable.as_ref() {
      Some(reusable) => ExecuteMessage::reusable_script_with_iteration_modules(
        &invocation_id,
        run_id,
        &node.id,
        attempt,
        options.script_timeout_ms,
        source,
        context,
        item,
        index,
        total,
        &secrets,
        &modules,
        reusable,
      ),
      None => ExecuteMessage::runtime_script_with_iteration_modules(
        &invocation_id,
        run_id,
        &node.id,
        attempt,
        options.script_timeout_ms,
        source,
        context,
        item,
        index,
        total,
        &secrets,
        &modules,
      ),
    }
    .map_err(|message| AttemptFailure {
      kind: AttemptFailureKind::InvalidScriptResult,
      code: "WOML_FOR_EACH_CONTEXT_INVALID".to_string(),
      message,
      details: None,
      ..AttemptFailure::legacy_defaults()
    })?;
    if let Some(limit) = options.max_context_bytes {
      let actual = serde_json::to_vec(&request.context)
        .map_err(|error| AttemptFailure {
          kind: AttemptFailureKind::InvalidScriptResult,
          code: "WOML_FOR_EACH_CONTEXT_INVALID".to_string(),
          message: error.to_string(),
          details: None,
          ..AttemptFailure::legacy_defaults()
        })?
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
        engine
          .append_scoped_payload(
            run_id,
            scope,
            RunEventPayload::StepAttemptFailed(StepAttemptFailedData {
              node_id: node.id.clone(),
              attempt: attempt_number,
              invocation_id,
              failure: failure.clone(),
            }),
          )
          .map_err(|error| AttemptFailure {
            kind: AttemptFailureKind::Interrupted,
            code: "WOML_FOR_EACH_EVENT_FAILED".to_string(),
            message: error.to_string(),
            details: None,
            ..AttemptFailure::legacy_defaults()
          })?;
        return Err(failure);
      }
    }
    active_invocations
      .lock()
      .await
      .insert(invocation_id.clone());
    let _artifact_cleanup = ArtifactInvocationCleanup::new(options, &invocation_id);
    let execution = host.execute(&request).await;
    active_invocations.lock().await.remove(&invocation_id);
    match execution {
      Ok(completed) => match completed.outcome {
        HostOutcome::Success { value } => {
          let success = RunEventPayload::StepAttemptSucceeded(StepAttemptSucceededData {
            node_id: node.id.clone(),
            attempt: attempt_number,
            invocation_id,
            output: value.clone(),
          });
          let completion_sequence = engine
            .events(run_id)
            .map_err(|error| AttemptFailure {
              kind: AttemptFailureKind::Interrupted,
              code: "WOML_FOR_EACH_EVENT_FAILED".to_string(),
              message: error.to_string(),
              details: None,
              ..AttemptFailure::legacy_defaults()
            })?
            .len() as u64
            + 1;
          let mut payloads = vec![success];
          if let Some((saga_id, action_id)) =
            compensation_registration_for_iteration(engine.workflow(), descriptor, &node.id, index)
          {
            payloads.push(RunEventPayload::CompensationRegistered(
              CompensationRegisteredData {
                saga_id,
                action_id,
                for_step_id: node.id.clone(),
                completion_sequence,
              },
            ));
          }
          engine
            .append_scoped_payloads(run_id, scope, payloads)
            .map_err(|error| AttemptFailure {
              kind: AttemptFailureKind::Interrupted,
              code: "WOML_FOR_EACH_EVENT_FAILED".to_string(),
              message: error.to_string(),
              details: None,
              ..AttemptFailure::legacy_defaults()
            })?;
          return Ok(value);
        }
        HostOutcome::Failure { error } => {
          let failure = error.into_attempt_failure();
          engine
            .append_scoped_payload(
              run_id,
              scope.clone(),
              RunEventPayload::StepAttemptFailed(StepAttemptFailedData {
                node_id: node.id.clone(),
                attempt: attempt_number,
                invocation_id,
                failure: failure.clone(),
              }),
            )
            .map_err(|error| AttemptFailure {
              kind: AttemptFailureKind::Interrupted,
              code: "WOML_FOR_EACH_EVENT_FAILED".to_string(),
              message: error.to_string(),
              details: None,
              ..AttemptFailure::legacy_defaults()
            })?;
          if failure.kind == AttemptFailureKind::ScriptThrew && attempt_number < max_attempts {
            let delay = node
              .retry_policy
              .as_ref()
              .and_then(|policy| policy.delay_before_attempt(attempt_number + 1))
              .unwrap_or(0);
            engine
              .append_scoped_payload(
                run_id,
                scope,
                RunEventPayload::StepRetryScheduled(StepRetryScheduledData {
                  node_id: node.id.clone(),
                  failed_attempt: attempt_number,
                  next_attempt: attempt_number + 1,
                  scheduled_at: options.clock.now()
                    + chrono::Duration::milliseconds(i64::try_from(delay).unwrap_or(i64::MAX)),
                }),
              )
              .map_err(|error| AttemptFailure {
                kind: AttemptFailureKind::Interrupted,
                code: "WOML_FOR_EACH_EVENT_FAILED".to_string(),
                message: error.to_string(),
                details: None,
                ..AttemptFailure::legacy_defaults()
              })?;
            if delay > 0 {
              tokio::time::sleep(Duration::from_millis(delay)).await;
            }
            continue;
          }
          return Err(failure);
        }
      },
      Err(error) => {
        let failure = AttemptFailure {
          kind: AttemptFailureKind::HostCrashed,
          code: AttemptFailureKind::HostCrashed.code().to_string(),
          message: error.to_string(),
          details: None,
          ..AttemptFailure::legacy_defaults()
        };
        engine
          .append_scoped_payload(
            run_id,
            scope,
            RunEventPayload::StepAttemptFailed(StepAttemptFailedData {
              node_id: node.id.clone(),
              attempt: attempt_number,
              invocation_id,
              failure: failure.clone(),
            }),
          )
          .map_err(|error| AttemptFailure {
            kind: AttemptFailureKind::Interrupted,
            code: "WOML_FOR_EACH_EVENT_FAILED".to_string(),
            message: error.to_string(),
            details: None,
            ..AttemptFailure::legacy_defaults()
          })?;
        return Err(failure);
      }
    }
  }
  unreachable!("a for-each script attempt always returns an outcome")
}

fn for_each_failure_kind(kind: AttemptFailureKind) -> ForEachIterationFailureKind {
  match kind {
    AttemptFailureKind::ScriptThrew => ForEachIterationFailureKind::ScriptThrew,
    AttemptFailureKind::ScriptTimedOut => ForEachIterationFailureKind::TimedOut,
    AttemptFailureKind::InvalidScriptResult => ForEachIterationFailureKind::NonJson,
    AttemptFailureKind::ContextTooLarge => ForEachIterationFailureKind::ContextTooLarge,
    AttemptFailureKind::ResultTooLarge => ForEachIterationFailureKind::ResultTooLarge,
    AttemptFailureKind::WorkerCrashed => ForEachIterationFailureKind::WorkerCrashed,
    AttemptFailureKind::HostCrashed => ForEachIterationFailureKind::HostCrashed,
    AttemptFailureKind::InvocationCancelled => ForEachIterationFailureKind::Cancelled,
    AttemptFailureKind::Interrupted => ForEachIterationFailureKind::Interrupted,
    AttemptFailureKind::ServiceFailed => ForEachIterationFailureKind::CapabilityFailed,
  }
}

async fn cancel_active_for_each_invocations(
  host: &ScriptHostClient,
  active_invocations: &Arc<tokio::sync::Mutex<HashSet<String>>>,
) {
  let invocation_ids = active_invocations
    .lock()
    .await
    .iter()
    .cloned()
    .collect::<Vec<_>>();
  for invocation_id in invocation_ids {
    let _ = host.cancel_run(&invocation_id).await;
  }
}

async fn execute_bounded_for_each<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  descriptor: &crate::model::CompiledForEach,
  options: &RuntimeExecutionOptions,
  host: &ScriptHostClient,
) -> Result<(), RuntimeExecutionError> {
  let root_projection = engine.projection(run_id)?;
  let root_context = root_projection.context.clone();
  let items = resolve_context_reference(&descriptor.items, &root_context).map_err(|_| {
    RuntimeExecutionError::InvalidConfiguration(format!(
      "for-each {:?} items reference is unavailable",
      descriptor.for_each_id
    ))
  })?;
  let Value::Array(items) = items else {
    return Err(RuntimeExecutionError::InvalidConfiguration(format!(
      "for-each {:?} items must resolve to an array",
      descriptor.for_each_id
    )));
  };
  if items.len() > 10_000 {
    return Err(RuntimeExecutionError::InvalidConfiguration(format!(
      "for-each {:?} exceeds the 10000-item limit",
      descriptor.for_each_id
    )));
  }

  let total = items.len() as u32;
  let concurrency = descriptor.concurrency as usize;
  let items_digest = for_each_digest(&Value::Array(items.clone()))?;
  let mut shared_engine = SharedForEachEngine::new(engine);
  match root_projection.for_each.get(&descriptor.for_each_id) {
    None => shared_engine.append_payload(
      run_id,
      RunEventPayload::ForEachOpened(ForEachOpenedData {
        for_each_id: descriptor.for_each_id.clone(),
        total,
        items_digest: items_digest.clone(),
        concurrency: descriptor.concurrency,
      }),
    )?,
    Some(loop_state)
      if loop_state.total == total
        && loop_state.items_digest == items_digest
        && loop_state.concurrency == descriptor.concurrency =>
    {
      if loop_state.status == ForEachStatus::Succeeded {
        return Ok(());
      }
      if loop_state.status != ForEachStatus::Open {
        return Err(RuntimeExecutionError::Stalled(format!(
          "for-each {:?} is already terminal",
          descriptor.for_each_id
        )));
      }
    }
    Some(_) => {
      return Err(RuntimeExecutionError::Stalled(format!(
        "for-each {:?} recovery input does not match its captured count, digest, or concurrency",
        descriptor.for_each_id
      )));
    }
  }
  report_for_each_progress(&shared_engine, options, run_id, &descriptor.for_each_id)?;

  let active_invocations = Arc::new(tokio::sync::Mutex::new(HashSet::new()));
  let mut aggregate_results = vec![None; items.len()];
  let loop_projection = shared_engine.projection(run_id)?;
  let loop_state = loop_projection
    .for_each
    .get(&descriptor.for_each_id)
    .ok_or_else(|| RuntimeExecutionError::Stalled("for-each disappeared".to_string()))?;
  let mut resumed_indexes = loop_state
    .iterations
    .iter()
    .filter_map(|(index, status)| match status {
      ForEachIterationStatus::Started => Some(*index),
      ForEachIterationStatus::Succeeded { result } => {
        aggregate_results[*index as usize] = result.clone();
        None
      }
      ForEachIterationStatus::Failed { .. } | ForEachIterationStatus::Skipped => None,
    })
    .collect::<VecDeque<_>>();
  let mut pending_indexes = (0..total)
    .filter(|index| !loop_state.iterations.contains_key(index))
    .collect::<VecDeque<_>>();
  let mut stop_admission = loop_state
    .iterations
    .values()
    .any(|status| matches!(status, ForEachIterationStatus::Failed { .. }))
    || matches!(
      root_projection.status,
      RunStatus::Cancelling | RunStatus::Finalizing
    )
    || root_projection.timeout_reached_at.is_some();
  let mut first_error = None;
  let mut active = FuturesUnordered::new();

  loop {
    while !stop_admission && active.len() < concurrency {
      let (index, resumed) = if let Some(index) = resumed_indexes.pop_front() {
        (index, true)
      } else if let Some(index) = pending_indexes.pop_front() {
        (index, false)
      } else {
        break;
      };
      if !resumed {
        let scope = ForEachIterationScope {
          for_each_id: descriptor.for_each_id.clone(),
          index,
        };
        shared_engine.append_scoped_payload(
          run_id,
          scope,
          RunEventPayload::ForEachIterationStarted(ForEachIterationIdentityData {
            for_each_id: descriptor.for_each_id.clone(),
          }),
        )?;
      }

      let mut iteration_engine = shared_engine.clone();
      let iteration_descriptor = descriptor.clone();
      let iteration_context = root_context.clone();
      let item = items[index as usize].clone();
      let iteration_options = options.clone();
      let iteration_invocations = Arc::clone(&active_invocations);
      active.push(async move {
        let result = execute_for_each_iteration(
          &mut iteration_engine,
          run_id,
          &iteration_descriptor,
          &iteration_context,
          &item,
          index,
          total,
          &iteration_options,
          host,
          &iteration_invocations,
        )
        .await;
        (index, result)
      });
    }

    if active.is_empty() {
      break;
    }

    tokio::select! {
      completion = active.next() => {
        let Some((index, result)) = completion else { break };
        match result {
          Ok(result) => aggregate_results[index as usize] = result,
          Err(error) => {
            let projection = shared_engine.projection(run_id)?;
            let iteration_is_open = projection
              .for_each
              .get(&descriptor.for_each_id)
              .and_then(|state| state.iterations.get(&index))
              .is_some_and(|status| matches!(status, ForEachIterationStatus::Started));
            if iteration_is_open {
              let (failed_node_id, failure_kind) = for_each_latest_iteration_failure(
                &projection,
                &descriptor.for_each_id,
                index,
              )
              .map(|(node_id, failure)| (node_id, for_each_failure_kind(failure.kind)))
              .unwrap_or_else(|| {
                (
                  descriptor.body.terminal_node_id.clone(),
                  ForEachIterationFailureKind::Interrupted,
                )
              });
              shared_engine.append_scoped_payload(
                run_id,
                ForEachIterationScope {
                  for_each_id: descriptor.for_each_id.clone(),
                  index,
                },
                RunEventPayload::ForEachIterationFailed(ForEachIterationFailedData {
                  for_each_id: descriptor.for_each_id.clone(),
                  failed_node_id,
                  failure_kind,
                }),
              )?;
            }
            if first_error.is_none() {
              first_error = Some(error);
              stop_admission = true;
              cancel_active_for_each_invocations(host, &active_invocations).await;
            }
          }
        }
        report_for_each_progress(
          &shared_engine,
          options,
          run_id,
          &descriptor.for_each_id,
        )?;
      }
      _ = tokio::time::sleep(CANCELLATION_POLL_INTERVAL), if !stop_admission => {
        settle_workflow_timeout_if_due(
          &mut shared_engine,
          run_id,
          options.clock.now(),
          options,
        )?;
        let projection = shared_engine.projection(run_id)?;
        if projection.timeout_reached_at.is_some() || projection.status == RunStatus::Cancelling {
          stop_admission = true;
          cancel_active_for_each_invocations(host, &active_invocations).await;
        }
      }
    }
  }

  let projection = shared_engine.projection(run_id)?;
  if projection.status == RunStatus::Cancelling {
    settle_open_for_each(
      &mut shared_engine,
      run_id,
      descriptor,
      ForEachTerminalDisposition::Cancelled,
    )?;
    report_for_each_progress(&shared_engine, options, run_id, &descriptor.for_each_id)?;
    return Err(cancelled_run_error(&shared_engine, run_id)?);
  }
  if projection.timeout_reached_at.is_some() {
    return Err(resumed_failure(&shared_engine, run_id, projection)?);
  }
  if first_error.is_some() || stop_admission {
    let projection = shared_engine.projection(run_id)?;
    let primary_failure = projection
      .for_each
      .get(&descriptor.for_each_id)
      .and_then(|loop_state| {
        loop_state
          .iterations
          .keys()
          .filter_map(|index| {
            for_each_latest_iteration_failure(&projection, &descriptor.for_each_id, *index)
          })
          .find(|(_, failure)| failure.kind != AttemptFailureKind::InvocationCancelled)
      })
      .map(|(_, failure)| failure)
      .unwrap_or_else(|| AttemptFailure {
        kind: AttemptFailureKind::Interrupted,
        code: "WOML_FOR_EACH_INTERRUPTED".to_string(),
        message: format!(
          "For-each {:?} could not complete one of its iterations.",
          descriptor.for_each_id
        ),
        details: None,
        ..AttemptFailure::legacy_defaults()
      });
    settle_open_for_each(
      &mut shared_engine,
      run_id,
      descriptor,
      ForEachTerminalDisposition::Failed,
    )?;
    report_for_each_progress(&shared_engine, options, run_id, &descriptor.for_each_id)?;
    if shared_engine.projection(run_id)?.business_outcome.is_none() {
      shared_engine.decide_run_failed(
        run_id,
        lifecycle_failure_from_attempt(primary_failure),
        options.clock.now(),
      )?;
    }
    return Err(resumed_failure(
      &shared_engine,
      run_id,
      shared_engine.projection(run_id)?,
    )?);
  }

  let mut public_output = Map::new();
  public_output.insert("total".to_string(), Value::from(total));
  public_output.insert("succeeded".to_string(), Value::from(total));
  if descriptor.result.is_some() && !aggregate_results.is_empty() {
    let results = aggregate_results
      .into_iter()
      .enumerate()
      .map(|(index, result)| {
        result.ok_or_else(|| {
          RuntimeExecutionError::Stalled(format!(
            "for-each {:?} completed without result slot {index}",
            descriptor.for_each_id
          ))
        })
      })
      .collect::<Result<Vec<_>, _>>()?;
    public_output.insert("results".to_string(), Value::Array(results));
  }
  shared_engine.append_payload(
    run_id,
    RunEventPayload::ForEachSucceeded(ForEachSettledData {
      for_each_id: descriptor.for_each_id.clone(),
      total,
      succeeded: total,
      failed: 0,
      skipped: 0,
      aggregate_digest: for_each_digest(&Value::Object(public_output))?,
    }),
  )?;
  report_for_each_progress(&shared_engine, options, run_id, &descriptor.for_each_id)?;
  Ok(())
}

fn for_each_ready_script_concurrency(
  descriptor: &crate::model::CompiledForEach,
  node_ids: &[String],
) -> usize {
  let mut parallel_id = None;
  for node_id in node_ids {
    let owner = descriptor
      .body
      .edges
      .iter()
      .find(|edge| edge.to == *node_id)
      .and_then(|edge| edge.parallel_id.as_deref());
    match (parallel_id, owner) {
      (None, Some(owner)) => parallel_id = Some(owner),
      (Some(expected), Some(owner)) if expected == owner => {}
      _ => return 1,
    }
  }
  let Some(parallel_id) = parallel_id else {
    return 1;
  };
  let start_id = format!("__woml_parallel__{parallel_id}__start");
  let Some(ValueExpression::Literal {
    value: Value::Number(value),
  }) = descriptor
    .body
    .nodes
    .iter()
    .find(|node| node.id == start_id)
    .and_then(|node| match &node.inputs {
      ValueExpression::Object { fields } => fields.get("concurrency"),
      _ => None,
    })
  else {
    return 1;
  };
  value
    .as_u64()
    .and_then(|value| usize::try_from(value).ok())
    .unwrap_or(1)
    .clamp(1, node_ids.len())
}

#[allow(clippy::too_many_arguments)]
async fn execute_for_each_iteration_script<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  descriptor: &crate::model::CompiledForEach,
  node_id: &str,
  context: &WorkflowContext,
  item: &Value,
  index: u32,
  total: u32,
  options: &RuntimeExecutionOptions,
  host: &ScriptHostClient,
  active_invocations: &Arc<tokio::sync::Mutex<HashSet<String>>>,
) -> Result<Value, RuntimeExecutionError> {
  let node = descriptor
    .body
    .nodes
    .iter()
    .find(|node| node.id == node_id)
    .ok_or_else(|| RuntimeExecutionError::Stalled("for-each body node disappeared".to_string()))?;
  let visible_local = descriptor
    .body
    .context_visibility
    .iter()
    .find(|visibility| visibility.node_id == node_id)
    .map(|visibility| visibility.step_ids.as_slice())
    .unwrap_or_default();
  let mut invocation_context = context.clone();
  invocation_context.steps.retain(|step_id, _| {
    descriptor.outer_step_ids.contains(step_id) || visible_local.contains(step_id)
  });
  if let Some(lock) = lock_descriptor_for_body_node(engine.workflow(), node_id) {
    if let Some(held) = engine
      .projection(run_id)?
      .for_each
      .get(&descriptor.for_each_id)
      .and_then(|loop_| loop_.executions.get(&index))
      .and_then(|execution| execution.locks.get(&lock.lock_id))
      .filter(|lock| lock.status == crate::LockStatus::Held)
    {
      invocation_context.lock = Some(crate::LockContext {
        id: held.lock_id.clone(),
        key: held.key.clone(),
        fencing_token: held.fencing_token.clone().unwrap_or_default(),
        acquired_at: held
          .acquired_at
          .map(|value| value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
          .unwrap_or_default(),
      });
    }
  }
  drive_for_each_step_lifecycle(
    engine,
    run_id,
    descriptor,
    node_id,
    LifecycleEventName::StepStart,
    &invocation_context,
    item,
    index,
    total,
    0,
    None,
    options,
    host,
  )
  .await?;
  match execute_for_each_script(
    engine,
    run_id,
    descriptor,
    node,
    &invocation_context,
    item,
    index,
    total,
    options,
    host,
    active_invocations,
  )
  .await
  {
    Ok(output) => {
      let mut completion_context = context.clone();
      completion_context
        .steps
        .insert(node_id.to_string(), output.clone());
      let attempts = engine
        .projection(run_id)?
        .for_each
        .get(&descriptor.for_each_id)
        .and_then(|loop_| loop_.executions.get(&index))
        .map_or(1, |execution| {
          execution
            .attempts
            .iter()
            .filter(|attempt| attempt.identity.node_id == node_id)
            .count() as u32
        });
      drive_for_each_reusable_lifecycle(
        engine,
        run_id,
        descriptor,
        node_id,
        &completion_context,
        item,
        index,
        total,
        ReusableInvocationOutcome::Succeeded,
        Some(output.clone()),
        None,
        options,
        host,
      )
      .await?;
      drive_for_each_step_lifecycle(
        engine,
        run_id,
        descriptor,
        node_id,
        LifecycleEventName::StepSuccess,
        &completion_context,
        item,
        index,
        total,
        attempts,
        None,
        options,
        host,
      )
      .await?;
      drive_for_each_step_lifecycle(
        engine,
        run_id,
        descriptor,
        node_id,
        LifecycleEventName::StepComplete,
        &completion_context,
        item,
        index,
        total,
        attempts,
        None,
        options,
        host,
      )
      .await?;
      Ok(output)
    }
    Err(failure) => {
      let attempts = engine
        .projection(run_id)?
        .for_each
        .get(&descriptor.for_each_id)
        .and_then(|loop_| loop_.executions.get(&index))
        .map_or(1, |execution| {
          execution
            .attempts
            .iter()
            .filter(|attempt| attempt.identity.node_id == node_id)
            .count() as u32
        });
      drive_for_each_reusable_lifecycle(
        engine,
        run_id,
        descriptor,
        node_id,
        context,
        item,
        index,
        total,
        ReusableInvocationOutcome::Failed,
        None,
        Some(ReusableLifecycleErrorV1 {
          code: failure.code.clone(),
          message: failure.message.clone(),
        }),
        options,
        host,
      )
      .await?;
      drive_for_each_step_lifecycle(
        engine,
        run_id,
        descriptor,
        node_id,
        LifecycleEventName::StepFailure,
        context,
        item,
        index,
        total,
        attempts,
        Some(&failure),
        options,
        host,
      )
      .await?;
      drive_for_each_step_lifecycle(
        engine,
        run_id,
        descriptor,
        node_id,
        LifecycleEventName::StepComplete,
        context,
        item,
        index,
        total,
        attempts,
        Some(&failure),
        options,
        host,
      )
      .await?;
      Err(RuntimeExecutionError::InvalidConfiguration(format!(
        "for-each {:?} iteration {index} node {node_id:?} failed [{}]: {}",
        descriptor.for_each_id, failure.code, failure.message
      )))
    }
  }
}

#[allow(clippy::too_many_arguments)]
async fn execute_for_each_iteration<E: RuntimeDagEngine + Clone>(
  engine: &mut E,
  run_id: &str,
  descriptor: &crate::model::CompiledForEach,
  root_context: &WorkflowContext,
  item: &Value,
  index: u32,
  total: u32,
  options: &RuntimeExecutionOptions,
  host: &ScriptHostClient,
  active_invocations: &Arc<tokio::sync::Mutex<HashSet<String>>>,
) -> Result<Option<Value>, RuntimeExecutionError> {
  let scope = ForEachIterationScope {
    for_each_id: descriptor.for_each_id.clone(),
    index,
  };

  let projection = engine.projection(run_id)?;
  let execution = projection
    .for_each
    .get(&descriptor.for_each_id)
    .and_then(|loop_state| loop_state.executions.get(&index))
    .cloned()
    .ok_or_else(|| {
      RuntimeExecutionError::Stalled(format!(
        "for-each {:?} iteration {index} has no durable execution projection",
        descriptor.for_each_id
      ))
    })?;
  let mut context = execution.context.clone();
  if context.steps.is_empty() {
    context = root_context.clone();
  }
  context.steps.retain(|step_id, _| {
    descriptor.outer_step_ids.contains(step_id)
      || descriptor.body.nodes.iter().any(|node| node.id == *step_id)
  });
  let mut completed = execution
    .attempts
    .iter()
    .filter_map(|attempt| {
      matches!(attempt.status, AttemptStatus::Succeeded { .. })
        .then_some(attempt.identity.node_id.clone())
    })
    .collect::<HashSet<_>>();
  let mut selections = execution.choice_selections.clone();
  selections.extend(execution.branch_selections.clone());
  for choice in &descriptor.body.choices {
    if selections.contains_key(&choice.choice_id) {
      completed.insert(choice.selector_node_id.clone());
    }
  }
  for branch_id in execution.branch_selections.keys() {
    completed.insert(format!("__woml_branch__{branch_id}__select"));
  }
  for (parallel_id, group) in &execution.parallel_groups {
    completed.insert(format!("__woml_parallel__{parallel_id}__start"));
    if !matches!(group.status, ParallelGroupStatus::Started) {
      completed.insert(parallel_id.clone());
    }
  }
  while !completed.contains(&descriptor.body.terminal_node_id) {
    let ready = for_each_ready_nodes(descriptor, &completed, &selections);
    if ready.is_empty() {
      return Err(RuntimeExecutionError::Stalled(format!(
        "for-each {:?} iteration {index} local DAG stalled",
        descriptor.for_each_id
      )));
    }
    let ready_scripts = ready
      .iter()
      .filter(|node_id| {
        descriptor
          .body
          .nodes
          .iter()
          .find(|node| node.id == node_id.as_str())
          .is_some_and(|node| node.handler == "runtime.script")
      })
      .cloned()
      .collect::<Vec<_>>();
    let inner_concurrency = for_each_ready_script_concurrency(descriptor, &ready_scripts);
    if ready_scripts.len() > 1 && inner_concurrency > 1 {
      let script_engine = Clone::clone(&*engine);
      let script_descriptor = descriptor.clone();
      let script_context = context.clone();
      let script_item = item.clone();
      let script_options = options.clone();
      let script_invocations = Arc::clone(active_invocations);
      let mut active_scripts = futures_util::stream::iter(ready_scripts)
        .map(move |node_id| {
          let mut script_engine = script_engine.clone();
          let script_descriptor = script_descriptor.clone();
          let script_context = script_context.clone();
          let script_item = script_item.clone();
          let script_options = script_options.clone();
          let script_invocations = Arc::clone(&script_invocations);
          async move {
            let result = execute_for_each_iteration_script(
              &mut script_engine,
              run_id,
              &script_descriptor,
              &node_id,
              &script_context,
              &script_item,
              index,
              total,
              &script_options,
              host,
              &script_invocations,
            )
            .await;
            (node_id, result)
          }
        })
        .buffer_unordered(inner_concurrency);
      let mut outputs = Vec::new();
      let mut first_error = None;
      while let Some((node_id, result)) = active_scripts.next().await {
        match result {
          Ok(output) => outputs.push((node_id, output)),
          Err(error) if first_error.is_none() => first_error = Some(error),
          Err(_) => {}
        }
      }
      if let Some(error) = first_error {
        return Err(error);
      }
      for (node_id, output) in outputs {
        context.steps.insert(node_id.clone(), output);
        completed.insert(node_id);
      }
      continue;
    }
    for node_id in ready {
      let node = descriptor
        .body
        .nodes
        .iter()
        .find(|node| node.id == node_id)
        .ok_or_else(|| {
          RuntimeExecutionError::Stalled("for-each body node disappeared".to_string())
        })?;
      match node.handler.as_str() {
        "engine.lock-acquire" => {
          let lock = engine
            .workflow()
            .graph
            .locks
            .as_deref()
            .unwrap_or_default()
            .iter()
            .find(|lock| lock.acquire_node_id == node_id)
            .cloned()
            .ok_or_else(|| {
              RuntimeExecutionError::Stalled(
                "Iteration lock acquire node has no compiled descriptor.".to_string(),
              )
            })?;
          let projected = engine
            .projection(run_id)?
            .for_each
            .get(&descriptor.for_each_id)
            .and_then(|loop_| loop_.executions.get(&index))
            .and_then(|execution| execution.locks.get(&lock.lock_id))
            .cloned();
          if projected.is_none() {
            let key = resolve_for_each_lock_key(&lock.key, &context, item, index, total)?;
            let now = options.clock.now();
            let deadline = now
              + chrono::Duration::milliseconds(i64::try_from(lock.acquire_timeout_ms).map_err(
                |_| {
                  RuntimeExecutionError::InvalidConfiguration(
                    "Iteration lock acquisition timeout exceeds the runtime clock.".to_string(),
                  )
                },
              )?);
            engine.register_iteration_lock_waiter(
              run_id,
              &lock.lock_id,
              &key,
              scope.clone(),
              now,
              deadline,
            )?;
          }
          options.suspend_policy_execution_slot().await?;
          loop {
            let state = engine
              .projection(run_id)?
              .for_each
              .get(&descriptor.for_each_id)
              .and_then(|loop_| loop_.executions.get(&index))
              .and_then(|execution| execution.locks.get(&lock.lock_id))
              .cloned()
              .ok_or_else(|| {
                RuntimeExecutionError::Stalled(
                  "Iteration lock waiter disappeared from durable projection.".to_string(),
                )
              })?;
            match state.status {
              crate::LockStatus::Held => {
                match engine.renew_iteration_lock_lease(
                  run_id,
                  &lock.lock_id,
                  scope.clone(),
                  options.clock.now(),
                )? {
                  crate::LockLeaseMaintenance::Renewed => {
                    options.ensure_iteration_lock_renewal(
                      run_id,
                      &lock.lock_id,
                      scope.clone(),
                      lock.lease_ms,
                    );
                    break;
                  }
                  crate::LockLeaseMaintenance::Lost | crate::LockLeaseMaintenance::NotHeld => {
                    return Err(RuntimeExecutionError::InvalidConfiguration(format!(
                      "Iteration lock {:?} expired before its protected continuation resumed.",
                      lock.lock_id
                    )));
                  }
                }
              }
              crate::LockStatus::Waiting if options.clock.now() < state.deadline => {
                tokio::time::sleep(CANCELLATION_POLL_INTERVAL).await;
              }
              crate::LockStatus::Waiting => {
                engine.timeout_iteration_lock_waiter(
                  run_id,
                  &lock.lock_id,
                  scope.clone(),
                  options.clock.now(),
                )?;
                return Err(RuntimeExecutionError::InvalidConfiguration(format!(
                  "Iteration lock {:?} was not acquired before its deadline.",
                  lock.lock_id
                )));
              }
              _ => {
                return Err(RuntimeExecutionError::InvalidConfiguration(format!(
                  "Iteration lock {:?} did not acquire safely.",
                  lock.lock_id
                )));
              }
            }
          }
        }
        "engine.lock-release" => {
          let lock = engine
            .workflow()
            .graph
            .locks
            .as_deref()
            .unwrap_or_default()
            .iter()
            .find(|lock| lock.release_node_id == node_id)
            .cloned()
            .ok_or_else(|| {
              RuntimeExecutionError::Stalled(
                "Iteration lock release node has no compiled descriptor.".to_string(),
              )
            })?;
          let held = engine
            .projection(run_id)?
            .for_each
            .get(&descriptor.for_each_id)
            .and_then(|loop_| loop_.executions.get(&index))
            .and_then(|execution| execution.locks.get(&lock.lock_id))
            .cloned()
            .ok_or_else(|| {
              RuntimeExecutionError::Stalled(
                "Iteration lock release has no acquired lease.".to_string(),
              )
            })?;
          let acquired_at = held.acquired_at.ok_or_else(|| {
            RuntimeExecutionError::Stalled(
              "Iteration lock release has no acquisition timestamp.".to_string(),
            )
          })?;
          let released_at = options.clock.now();
          let result = serde_json::json!({
            "status": "released",
            "key": held.key,
            "fencingToken": held.fencing_token,
            "acquiredAt": acquired_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "releasedAt": released_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "waitedMs": (acquired_at - held.registered_at).num_milliseconds().max(0),
          });
          engine.release_iteration_lock(
            run_id,
            &lock.lock_id,
            scope.clone(),
            result.clone(),
            released_at,
          )?;
          options.stop_iteration_lock_renewal(run_id, &lock.lock_id, &scope);
          context.steps.insert(lock.lock_id, result);
        }
        "engine.lock-result" => {}
        "runtime.script" => {
          let output = execute_for_each_iteration_script(
            engine,
            run_id,
            descriptor,
            &node_id,
            &context,
            item,
            index,
            total,
            options,
            host,
            active_invocations,
          )
          .await?;
          context.steps.insert(node_id.clone(), output);
        }
        "engine.choice-select" => {
          let (choice_id, arm_id) =
            for_each_selected_choice(descriptor, &node_id, &context, item, index, total)?;
          engine.append_scoped_payload(
            run_id,
            scope.clone(),
            RunEventPayload::ChoiceSelected(ChoiceSelectedData {
              choice_id: choice_id.clone(),
              arm_id: arm_id.clone(),
            }),
          )?;
          selections.insert(choice_id, arm_id);
        }
        "engine.branch-select" => {
          let (branch_id, arm_id) =
            for_each_selected_branch(descriptor, &node_id, &context, item, index, total)?;
          engine.append_scoped_payload(
            run_id,
            scope.clone(),
            RunEventPayload::BranchSelected(BranchSelectedData {
              branch_id: branch_id.clone(),
              arm_id: arm_id.clone(),
            }),
          )?;
          selections.insert(branch_id, arm_id);
        }
        "engine.branch-result" => {
          let arm_id = selections.get(&node_id).ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "for-each conditional branch {node_id:?} has no durable selection"
            ))
          })?;
          let ValueExpression::Object { fields } = &node.inputs else {
            return Err(RuntimeExecutionError::Stalled(format!(
              "for-each conditional result {node_id:?} has invalid inputs"
            )));
          };
          let expression = fields.get(arm_id).ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "for-each conditional result {node_id:?} has no selected-arm value"
            ))
          })?;
          let output = resolve_for_each_reference(expression, &context, item, index, total)?;
          context.steps.insert(node_id.clone(), output);
        }
        "engine.choice-result" => {
          let choice = descriptor
            .body
            .choices
            .iter()
            .find(|choice| choice.result_node_id.as_deref() == Some(node_id.as_str()))
            .ok_or_else(|| {
              RuntimeExecutionError::Stalled(format!(
                "for-each result node {node_id:?} has no choice descriptor"
              ))
            })?;
          let arm_id = selections.get(&choice.choice_id).ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "for-each choice {:?} has no durable selection",
              choice.choice_id
            ))
          })?;
          let ValueExpression::Object { fields } = &node.inputs else {
            return Err(RuntimeExecutionError::Stalled(format!(
              "for-each choice result {node_id:?} has invalid inputs"
            )));
          };
          let expression = fields.get(arm_id).ok_or_else(|| {
            RuntimeExecutionError::Stalled(format!(
              "for-each choice result {node_id:?} has no selected arm value"
            ))
          })?;
          let output = resolve_for_each_reference(expression, &context, item, index, total)?;
          context.steps.insert(node_id.clone(), output);
        }
        "engine.choice-join" => {}
        "engine.parallel-start" => {
          let parallel_id = node_id
            .strip_prefix("__woml_parallel__")
            .and_then(|id| id.strip_suffix("__start"))
            .ok_or_else(|| {
              RuntimeExecutionError::Stalled(format!(
                "for-each parallel start node {node_id:?} has no canonical group identity"
              ))
            })?
            .to_string();
          engine.append_scoped_payload(
            run_id,
            scope.clone(),
            RunEventPayload::ParallelGroupStarted(ParallelGroupStartedData { parallel_id }),
          )?;
        }
        "engine.parallel-join" => {
          engine.append_scoped_payload(
            run_id,
            scope.clone(),
            RunEventPayload::ParallelGroupCompleted(ParallelGroupCompletedData {
              parallel_id: node_id.clone(),
              outcome: ParallelGroupOutcome::Succeeded,
              failed_node_ids: Vec::new(),
              cancelled_node_ids: Vec::new(),
            }),
          )?;
        }
        handler => {
          return Err(RuntimeExecutionError::Stalled(format!(
            "for-each body node {node_id:?} uses unsupported handler {handler:?}"
          )))
        }
      }
      completed.insert(node_id);
    }
  }

  let result = descriptor
    .result
    .as_ref()
    .map(|expression| resolve_for_each_reference(expression, &context, item, index, total))
    .transpose()?;
  let result_digest = result.as_ref().map(for_each_digest).transpose()?;
  engine.append_scoped_payload(
    run_id,
    scope,
    RunEventPayload::ForEachIterationSucceeded(ForEachIterationSucceededData {
      for_each_id: descriptor.for_each_id.clone(),
      result: result.clone(),
      result_digest,
    }),
  )?;
  Ok(result)
}

fn ensure_local_script_execution(
  node: &crate::model::CompiledWorkflowNode,
) -> Result<(), RuntimeExecutionError> {
  let Some(execution) = node.execution.as_ref() else {
    return Ok(());
  };
  if execution.target == "local" {
    return Ok(());
  }
  Err(RuntimeExecutionError::Stalled(format!(
    "remote target {:?} for node {:?} requires an activated remote dispatch coordinator",
    execution.target, node.id
  )))
}

async fn execute_script_node<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  node_id: &str,
  source: &str,
  publish_terminal_success: bool,
  options: &RuntimeExecutionOptions,
  host: &mut Option<ScriptHostClient>,
) -> Result<ScriptNodeOutcome, RuntimeExecutionError> {
  let node = engine.workflow().node(node_id).ok_or_else(|| {
    RuntimeExecutionError::Stalled(format!("script node {node_id:?} is unavailable"))
  })?;
  ensure_local_script_execution(node)?;
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
        host,
        run_id,
        node_id,
        attempt_number,
        &invocation_id,
        failure,
      )
      .await;
    }
  }

  if host.is_none() {
    match options.acquire_script_host().await {
      Ok(client) => *host = Some(client),
      Err(error) => {
        return settle_script_attempt_failure(
          engine,
          options,
          host,
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
        )
        .await;
      }
    }
  }

  let module_bindings = options
    .runtime_modules
    .iter()
    .filter(|module| !module.name.starts_with("__woml_"))
    .map(|module| RuntimeModuleBinding {
      name: module.name.clone(),
      bundle_digest: module.bundle_digest.clone(),
      exports: module.exports.clone(),
    })
    .collect::<Vec<_>>();
  let attempt = ScriptAttempt::new(attempt_number, max_attempts, &idempotency_key)
    .map_err(RuntimeExecutionError::Stalled)?;
  let reusable = reusable_step_binding(engine.workflow(), node_id, &context, &secrets)?;
  let request = match reusable.as_ref() {
    Some(reusable) => ExecuteMessage::reusable_script_with_modules(
      &invocation_id,
      run_id,
      node_id,
      attempt,
      options.script_timeout_ms,
      source,
      &context,
      &secrets,
      &module_bindings,
      reusable,
    ),
    None => ExecuteMessage::runtime_script_with_modules(
      &invocation_id,
      run_id,
      node_id,
      attempt,
      options.script_timeout_ms,
      source,
      &context,
      &secrets,
      &module_bindings,
    ),
  };
  let host_client = host.as_ref().expect("script host was initialized");
  let artifact_cleanup = ArtifactInvocationCleanup::new(options, &invocation_id);
  let mut execution = Box::pin(host_client.execute(&request));
  let mut cancellation_sent = false;
  let mut timeout_sent = false;
  let mut lease_loss_sent = false;
  let mut retire_interrupted_host = false;
  let completed = loop {
    tokio::select! {
      result = &mut execution => break result,
      _ = tokio::time::sleep(CANCELLATION_POLL_INTERVAL), if !cancellation_sent && !timeout_sent && !lease_loss_sent => {
        settle_workflow_timeout_if_due(engine, run_id, options.clock.now(), options)?;
        let current = engine.projection(run_id)?;
        let protected_lease_lost = lock_descriptor_for_body_node(engine.workflow(), node_id)
          .and_then(|lock| current.locks.get(&lock.lock_id))
          .is_some_and(|lock| lock.status == crate::LockStatus::LeaseLost);
        if protected_lease_lost {
          lease_loss_sent = true;
          if let Err(error) = interrupt_script_invocation(host_client, &invocation_id).await {
            retire_interrupted_host = true;
            break Err(error);
          }
        } else if current.timeout_reached_at.is_some() {
          timeout_sent = true;
          if let Err(error) = interrupt_script_invocation(host_client, &invocation_id).await {
            retire_interrupted_host = true;
            break Err(error);
          }
        } else if current.status == RunStatus::Cancelling {
          cancellation_sent = true;
          if let Err(error) = interrupt_script_invocation(host_client, &invocation_id).await {
            retire_interrupted_host = true;
            break Err(error);
          }
        }
      }
      _ = tokio::time::sleep(SCRIPT_HOST_INTERRUPT_GRACE), if cancellation_sent || timeout_sent || lease_loss_sent => {
        retire_interrupted_host = true;
        break Err(ScriptHostClientError::HostCrashed(
          "the script host did not acknowledge invocation interruption before its deadline"
            .to_string(),
        ));
      }
    }
  };
  drop(execution);
  drop(artifact_cleanup);
  if retire_interrupted_host {
    // A host that cannot settle an interrupted Worker cannot safely execute
    // correction or later workflow work. Dropping it terminates the child
    // process (`kill_on_drop`) and the next action acquires a fresh host.
    *host = None;
  }
  settle_workflow_timeout_if_due(engine, run_id, options.clock.now(), options)?;
  if timeout_sent || engine.projection(run_id)?.timeout_reached_at.is_some() {
    let projection = engine.projection(run_id)?;
    return Err(resumed_failure(engine, run_id, projection)?);
  }
  if lease_loss_sent {
    let failure = AttemptFailure {
      kind: AttemptFailureKind::Interrupted,
      code: "WOML_LOCK_LEASE_LOST".to_string(),
      message: "The protected step was stopped because its durable lock lease expired.".to_string(),
      details: None,
      ..AttemptFailure::legacy_defaults()
    };
    return settle_script_attempt_failure(
      engine,
      options,
      host,
      run_id,
      node_id,
      attempt_number,
      &invocation_id,
      failure,
    )
    .await;
  }
  let outcome = match completed {
    _ if cancellation_sent || engine.projection(run_id)?.status == RunStatus::Cancelling => {
      return settle_script_attempt_failure(
        engine,
        options,
        host,
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
      )
      .await;
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
        host,
        run_id,
        node_id,
        attempt_number,
        &invocation_id,
        failure,
      )
      .await;
    }
  };

  match outcome {
    HostOutcome::Success { value } => {
      let succeeded_data = StepAttemptSucceededData {
        node_id: node_id.to_string(),
        attempt: attempt_number,
        invocation_id,
        output: value.clone(),
      };
      let succeeded = RunEventPayload::StepAttemptSucceeded(succeeded_data);
      if publish_terminal_success {
        engine.append_payloads(
          run_id,
          vec![
            succeeded,
            RunEventPayload::RunSucceeded(RunSucceededData {
              terminal_node_id: node_id.to_string(),
              result: value.clone(),
            }),
          ],
        )?;
        report_attempt_succeeded(engine, options, run_id, node_id, attempt_number);
        return Ok(ScriptNodeOutcome::WorkflowSucceeded);
      }
      append_step_attempt_success_payload(engine, run_id, succeeded)?;
      drive_reusable_step_lifecycle(
        engine,
        run_id,
        node_id,
        ReusableInvocationOutcome::Succeeded,
        Some(value.clone()),
        None,
        options,
        host.as_ref().expect("script host was initialized"),
      )
      .await?;
      report_attempt_succeeded(engine, options, run_id, node_id, attempt_number);
      Ok(ScriptNodeOutcome::Succeeded(value))
    }
    HostOutcome::Failure { error } => {
      settle_script_attempt_failure(
        engine,
        options,
        host,
        run_id,
        node_id,
        attempt_number,
        &invocation_id,
        error.into_attempt_failure(),
      )
      .await
    }
  }
}

enum ScriptNodeOutcome {
  Succeeded(Value),
  WorkflowSucceeded,
  RetryScheduled,
  Cancelled,
  SagaFailed(Box<StepAttemptFailedData>),
}

fn compensation_registration(
  workflow: &CompiledWorkflowDefinition,
  node_id: &str,
) -> Option<(String, String)> {
  let saga = workflow.saga_for_body_node(node_id)?;
  let action = saga
    .compensations
    .iter()
    .find(|action| action.for_step_id.as_deref() == Some(node_id))?;
  Some((saga.saga_id.clone(), action.action_id.clone()))
}

fn compensation_registration_for_iteration(
  workflow: &CompiledWorkflowDefinition,
  descriptor: &crate::model::CompiledForEach,
  node_id: &str,
  index: u32,
) -> Option<(String, String)> {
  let saga = workflow.saga_for_body_node(&descriptor.open_node_id)?;
  let action = saga.compensations.iter().find(|action| {
    action.for_step_id.as_deref() == Some(node_id)
      && matches!(
        &action.identity,
        CompiledCorrectionIdentity::ForEachIteration { for_each_id, .. }
          if for_each_id == &descriptor.for_each_id
      )
  })?;
  let CompiledCorrectionIdentity::ForEachIteration {
    action_id_template, ..
  } = &action.identity
  else {
    return None;
  };
  Some((
    saga.saga_id.clone(),
    action_id_template.replace("{iterationKey}", &index.to_string()),
  ))
}

fn correction_iteration_identity<'a>(
  action: &'a CompiledCorrectiveAction,
  runtime_action_id: &str,
) -> Option<(&'a str, u32)> {
  let CompiledCorrectionIdentity::ForEachIteration {
    for_each_id,
    action_id_template,
  } = &action.identity
  else {
    return None;
  };
  let (prefix, suffix) = action_id_template.split_once("{iterationKey}")?;
  let key = runtime_action_id
    .strip_prefix(prefix)?
    .strip_suffix(suffix)?;
  Some((for_each_id, key.parse().ok()?))
}

fn corrective_action_matches_runtime(
  action: &CompiledCorrectiveAction,
  runtime_action_id: &str,
) -> bool {
  match &action.identity {
    CompiledCorrectionIdentity::Static { action_id } => action_id == runtime_action_id,
    CompiledCorrectionIdentity::ForEachIteration { .. } => {
      correction_iteration_identity(action, runtime_action_id).is_some()
    }
  }
}

fn append_step_attempt_success<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  succeeded: StepAttemptSucceededData,
) -> Result<(), RuntimeExecutionError> {
  append_step_attempt_success_payload(
    engine,
    run_id,
    RunEventPayload::StepAttemptSucceeded(succeeded),
  )
}

fn append_step_attempt_success_payload<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  succeeded: RunEventPayload,
) -> Result<(), RuntimeExecutionError> {
  let RunEventPayload::StepAttemptSucceeded(data) = &succeeded else {
    return Err(RuntimeExecutionError::Stalled(
      "compensation registration requires a step success event".to_string(),
    ));
  };
  let node_id = data.node_id.clone();
  let Some((saga_id, action_id)) = compensation_registration(engine.workflow(), &node_id) else {
    return engine.append_payload(run_id, succeeded);
  };
  let completion_sequence = engine.events(run_id)?.len() as u64 + 1;
  engine.append_payloads(
    run_id,
    vec![
      succeeded,
      RunEventPayload::CompensationRegistered(CompensationRegisteredData {
        saga_id,
        action_id,
        for_step_id: node_id,
        completion_sequence,
      }),
    ],
  )
}

async fn correct_saga_failure_for_node<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  node_id: &str,
  options: &RuntimeExecutionOptions,
) -> Result<(), RuntimeExecutionError> {
  let Some(saga) = engine.workflow().saga_for_body_node(node_id).cloned() else {
    return Ok(());
  };
  let mut correction_host = None;
  let correction_completed = execute_saga_correction(
    engine,
    run_id,
    &saga,
    SagaCorrectionReason::Failure,
    options,
    &mut correction_host,
  )
  .await?;
  if !correction_completed
    && engine
      .projection(run_id)?
      .sagas
      .get(&saga.saga_id)
      .is_some_and(|state| state.phase == crate::SagaPhase::ManualRecovery)
  {
    return Err(RuntimeExecutionError::Stalled(format!(
      "saga {:?} requires manual recovery",
      saga.saga_id
    )));
  }
  Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CorrectiveActionOutcome {
  Succeeded,
  Failed,
  ManualRecovery,
}

fn resolved_corrective_secrets(
  action: &CompiledCorrectiveAction,
  options: &RuntimeExecutionOptions,
) -> Result<BTreeMap<String, String>, RuntimeExecutionError> {
  let Some(runtime) = action.node.script_runtime.as_ref() else {
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
            "corrective action {:?} requires unresolved secret {name:?}",
            action.action_id
          ))
        })
    })
    .collect()
}

async fn execute_corrective_action<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  saga_id: &str,
  action: &CompiledCorrectiveAction,
  runtime_action_id: &str,
  action_kind: CorrectiveActionKind,
  reason: SagaCorrectionReason,
  compensation_status: CompensationStatusV1,
  options: &RuntimeExecutionOptions,
  host: &mut Option<ScriptHostClient>,
) -> Result<CorrectiveActionOutcome, RuntimeExecutionError> {
  ensure_local_script_execution(&action.node)?;
  let projected = engine
    .projection(run_id)?
    .sagas
    .get(saga_id)
    .and_then(|saga| saga.actions.get(runtime_action_id))
    .cloned();
  if projected
    .as_ref()
    .is_some_and(|state| state.status == crate::CorrectionActionStatus::Succeeded)
  {
    return Ok(CorrectiveActionOutcome::Succeeded);
  }
  if engine.correction_action_externally_resolved(run_id, saga_id, runtime_action_id)? {
    return Ok(CorrectiveActionOutcome::Succeeded);
  }
  if projected
    .as_ref()
    .is_some_and(|state| state.status == crate::CorrectionActionStatus::ManualRecovery)
  {
    return Ok(CorrectiveActionOutcome::ManualRecovery);
  }
  let source = action.node.script_source().ok_or_else(|| {
    RuntimeExecutionError::Stalled(format!(
      "corrective action {:?} has no script source",
      runtime_action_id
    ))
  })?;
  let secrets = resolved_corrective_secrets(action, options)?;
  let projection = engine.projection(run_id)?;
  let mut context = correction_iteration_identity(action, runtime_action_id)
    .and_then(|(for_each_id, index)| {
      projection
        .for_each
        .get(for_each_id)
        .and_then(|loop_state| loop_state.executions.get(&index))
        .map(|execution| execution.context.clone())
    })
    .unwrap_or_else(|| projection.context.clone());
  context
    .steps
    .retain(|step_id, _| action.visible_step_ids.contains(step_id));
  let module_bindings = options
    .runtime_modules
    .iter()
    .filter(|module| !module.name.starts_with("__woml_"))
    .map(|module| RuntimeModuleBinding {
      name: module.name.clone(),
      bundle_digest: module.bundle_digest.clone(),
      exports: module.exports.clone(),
    })
    .collect::<Vec<_>>();
  let configured_max_attempts = action
    .node
    .retry_policy
    .as_ref()
    .map_or(1, |policy| policy.max_attempts);
  let max_attempts = projected.as_ref().map_or(configured_max_attempts, |state| {
    if state.status == crate::CorrectionActionStatus::RetryScheduled {
      configured_max_attempts.max(state.attempts.saturating_add(1).min(10))
    } else {
      configured_max_attempts
    }
  });
  let timeout_ms = action.node.timeout_ms.unwrap_or(options.script_timeout_ms);
  let compensation_binding =
    (action_kind == CorrectiveActionKind::Compensation).then(|| CompensationBindingV1 {
      saga_id: saga_id.to_string(),
      for_step_id: action.for_step_id.clone().unwrap_or_default(),
      reason: match reason {
        SagaCorrectionReason::Cancel => CorrectionReasonV1::Cancel,
        SagaCorrectionReason::Failure | SagaCorrectionReason::Success => {
          CorrectionReasonV1::Failure
        }
      },
    });
  let cleanup_binding = (action_kind == CorrectiveActionKind::Cleanup).then(|| CleanupBindingV1 {
    saga_id: saga_id.to_string(),
    reason: match reason {
      SagaCorrectionReason::Success => CleanupReasonV1::Success,
      SagaCorrectionReason::Failure => CleanupReasonV1::Failure,
      SagaCorrectionReason::Cancel => CleanupReasonV1::Cancel,
    },
    compensation_status,
  });
  let correction = match action_kind {
    CorrectiveActionKind::Compensation => CorrectionBindingRef::Compensation(
      compensation_binding
        .as_ref()
        .expect("compensation binding was constructed"),
    ),
    CorrectiveActionKind::Cleanup => CorrectionBindingRef::Cleanup(
      cleanup_binding
        .as_ref()
        .expect("cleanup binding was constructed"),
    ),
  };

  let mut attempt = projected.as_ref().map_or(1, |state| state.attempts + 1);
  if let Some(retry_at) = projected.as_ref().and_then(|state| state.retry_at) {
    while retry_at > options.clock.now() {
      let wait = (retry_at - options.clock.now())
        .to_std()
        .unwrap_or(CANCELLATION_POLL_INTERVAL)
        .min(CANCELLATION_POLL_INTERVAL);
      tokio::time::sleep(wait).await;
    }
  }
  while attempt <= max_attempts {
    let identity = CorrectionAttemptIdentityData {
      saga_id: saga_id.to_string(),
      action_id: runtime_action_id.to_string(),
      action_kind,
      attempt,
    };
    engine.append_payload(
      run_id,
      RunEventPayload::CorrectionAttemptStarted(identity.clone()),
    )?;
    if host.is_none() {
      match options.acquire_script_host().await {
        Ok(client) => *host = Some(client),
        Err(_) => {
          engine.append_payloads(
            run_id,
            vec![
              RunEventPayload::CorrectionInterrupted(CorrectionInterruptedData {
                saga_id: saga_id.to_string(),
                action_id: runtime_action_id.to_string(),
                action_kind,
                attempt,
                ambiguous: true,
              }),
              RunEventPayload::ManualRecoveryRequired(ManualRecoveryRequiredData {
                saga_id: saga_id.to_string(),
                action_id: runtime_action_id.to_string(),
                action_kind,
                code: "WOML_CORRECTION_HOST_UNAVAILABLE".to_string(),
              }),
            ],
          )?;
          return Ok(CorrectiveActionOutcome::ManualRecovery);
        }
      }
    }
    let invocation_id =
      crate::durable::correction_invocation_id(run_id, saga_id, runtime_action_id, attempt);
    let idempotency_key = format!("sha256:{:x}", Sha256::digest(invocation_id.as_bytes()));
    let script_attempt = ScriptAttempt::new(attempt, max_attempts, &idempotency_key)
      .map_err(RuntimeExecutionError::Stalled)?;
    let request = ExecuteMessage::corrective_script_with_modules(
      &invocation_id,
      run_id,
      runtime_action_id,
      script_attempt,
      timeout_ms,
      source,
      &context,
      correction,
      &secrets,
      &module_bindings,
    );
    let cleanup = ArtifactInvocationCleanup::new(options, &invocation_id);
    let completed = host
      .as_ref()
      .expect("corrective script host was initialized")
      .execute(&request)
      .await;
    drop(cleanup);
    match completed {
      Ok(completed) => match completed.outcome {
        HostOutcome::Success { .. } => {
          engine.append_payload(
            run_id,
            RunEventPayload::CorrectionAttemptSucceeded(identity),
          )?;
          return Ok(CorrectiveActionOutcome::Succeeded);
        }
        HostOutcome::Failure { error } => {
          let failure = error.into_attempt_failure();
          let ambiguous = failure.ambiguous.unwrap_or(false)
            || matches!(
              failure.kind,
              AttemptFailureKind::ScriptTimedOut
                | AttemptFailureKind::WorkerCrashed
                | AttemptFailureKind::HostCrashed
                | AttemptFailureKind::InvocationCancelled
                | AttemptFailureKind::Interrupted
            );
          let retryable = !ambiguous
            && (failure.retryable.unwrap_or(false)
              || failure.kind == AttemptFailureKind::ScriptThrew);
          engine.append_payload(
            run_id,
            RunEventPayload::CorrectionAttemptFailed(CorrectionAttemptFailedData {
              saga_id: saga_id.to_string(),
              action_id: runtime_action_id.to_string(),
              action_kind,
              attempt,
              code: failure.code,
              retryable,
              ambiguous,
            }),
          )?;
          if ambiguous {
            engine.append_payload(
              run_id,
              RunEventPayload::ManualRecoveryRequired(ManualRecoveryRequiredData {
                saga_id: saga_id.to_string(),
                action_id: runtime_action_id.to_string(),
                action_kind,
                code: "WOML_CORRECTION_OUTCOME_AMBIGUOUS".to_string(),
              }),
            )?;
            return Ok(CorrectiveActionOutcome::ManualRecovery);
          }
          if !retryable || attempt >= max_attempts {
            return Ok(CorrectiveActionOutcome::Failed);
          }
          let delay = action
            .node
            .retry_policy
            .as_ref()
            .and_then(|policy| policy.delay_before_attempt(attempt + 1))
            .unwrap_or(0);
          let retry_at = options.clock.now()
            + chrono::Duration::milliseconds(i64::try_from(delay).unwrap_or(i64::MAX));
          engine.append_payload(
            run_id,
            RunEventPayload::CorrectionRetryScheduled(CorrectionRetryScheduledData {
              saga_id: saga_id.to_string(),
              action_id: runtime_action_id.to_string(),
              action_kind,
              attempt,
              retry_at,
            }),
          )?;
          if delay > 0 {
            tokio::time::sleep(Duration::from_millis(delay)).await;
          }
          attempt += 1;
        }
      },
      Err(_) => {
        *host = None;
        engine.append_payloads(
          run_id,
          vec![
            RunEventPayload::CorrectionInterrupted(CorrectionInterruptedData {
              saga_id: saga_id.to_string(),
              action_id: runtime_action_id.to_string(),
              action_kind,
              attempt,
              ambiguous: true,
            }),
            RunEventPayload::ManualRecoveryRequired(ManualRecoveryRequiredData {
              saga_id: saga_id.to_string(),
              action_id: runtime_action_id.to_string(),
              action_kind,
              code: "WOML_CORRECTION_INTERRUPTED".to_string(),
            }),
          ],
        )?;
        return Ok(CorrectiveActionOutcome::ManualRecovery);
      }
    }
  }
  Ok(CorrectiveActionOutcome::Failed)
}

async fn execute_saga_correction<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  saga: &CompiledSaga,
  reason: SagaCorrectionReason,
  options: &RuntimeExecutionOptions,
  host: &mut Option<ScriptHostClient>,
) -> Result<bool, RuntimeExecutionError> {
  let phase = engine
    .projection(run_id)?
    .sagas
    .get(&saga.saga_id)
    .map(|state| state.phase)
    .ok_or_else(|| RuntimeExecutionError::Stalled("saga correction has no open scope".into()))?;
  if !matches!(
    phase,
    crate::SagaPhase::Correcting | crate::SagaPhase::ManualRecovery
  ) {
    engine.append_payload(
      run_id,
      RunEventPayload::SagaCorrectionStarted(SagaCorrectionStartedData {
        saga_id: saga.saga_id.clone(),
        reason,
      }),
    )?;
  }

  let mut all_succeeded = true;
  let mut manual_recovery = false;
  if reason != SagaCorrectionReason::Success {
    let projection = engine.projection(run_id)?;
    let saga_state = projection
      .sagas
      .get(&saga.saga_id)
      .ok_or_else(|| RuntimeExecutionError::Stalled("saga correction state disappeared".into()))?;
    let mut registered = saga_state
      .actions
      .values()
      .filter(|action| action.action_kind == CorrectiveActionKind::Compensation)
      .filter_map(|action| {
        action
          .completion_sequence
          .map(|sequence| (sequence, action.action_id.clone()))
      })
      .collect::<Vec<_>>();
    registered.sort_by(|left, right| right.cmp(left));
    for (_, action_id) in registered {
      let action = saga
        .compensations
        .iter()
        .find(|candidate| corrective_action_matches_runtime(candidate, &action_id))
        .ok_or_else(|| {
          RuntimeExecutionError::Stalled("registered compensation disappeared".into())
        })?;
      match execute_corrective_action(
        engine,
        run_id,
        &saga.saga_id,
        action,
        &action_id,
        CorrectiveActionKind::Compensation,
        reason,
        CompensationStatusV1::NotRequired,
        options,
        host,
      )
      .await?
      {
        CorrectiveActionOutcome::Succeeded => {}
        CorrectiveActionOutcome::Failed => all_succeeded = false,
        CorrectiveActionOutcome::ManualRecovery => {
          all_succeeded = false;
          manual_recovery = true;
        }
      }
    }
  }

  let compensation_status = if reason == SagaCorrectionReason::Success {
    CompensationStatusV1::NotRequired
  } else if manual_recovery {
    CompensationStatusV1::ManualRecovery
  } else if all_succeeded {
    CompensationStatusV1::Completed
  } else {
    CompensationStatusV1::Incomplete
  };
  for action in &saga.cleanup {
    match execute_corrective_action(
      engine,
      run_id,
      &saga.saga_id,
      action,
      &action.action_id,
      CorrectiveActionKind::Cleanup,
      reason,
      compensation_status,
      options,
      host,
    )
    .await?
    {
      CorrectiveActionOutcome::Succeeded => {}
      CorrectiveActionOutcome::Failed => all_succeeded = false,
      CorrectiveActionOutcome::ManualRecovery => {
        all_succeeded = false;
        manual_recovery = true;
      }
    }
  }
  if manual_recovery {
    return Ok(false);
  }
  engine.append_payload(
    run_id,
    RunEventPayload::SagaCorrectionCompleted(SagaIdentityData {
      saga_id: saga.saga_id.clone(),
    }),
  )?;
  Ok(all_succeeded)
}

fn recorded_execution_order(events: &[RunEvent]) -> Vec<String> {
  events
    .iter()
    .filter_map(|event| match &event.payload {
      RunEventPayload::StepAttemptSucceeded(data) => Some(data.node_id.clone()),
      RunEventPayload::WaitSatisfied(data) => Some(data.wait_id.clone()),
      RunEventPayload::WaitTimedOut(data)
        if data.behavior == crate::DurableWaitTimeoutBehavior::Continue =>
      {
        Some(data.wait_id.clone())
      }
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
const SCRIPT_HOST_INTERRUPT_GRACE: Duration = Duration::from_secs(2);

async fn interrupt_script_invocation(
  host: &ScriptHostClient,
  invocation_id: &str,
) -> Result<(), ScriptHostClientError> {
  tokio::time::timeout(SCRIPT_HOST_INTERRUPT_GRACE, host.cancel_run(invocation_id))
    .await
    .map_err(|_| {
      ScriptHostClientError::HostCrashed(
        "the script host did not accept invocation interruption before its deadline".to_string(),
      )
    })?
}

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
  options: &RuntimeExecutionOptions,
) -> Result<(), RuntimeExecutionError> {
  let projection = engine.projection(run_id)?;
  if projection.status != RunStatus::Cancelling {
    return Ok(());
  }

  engine.settle_pending_rich_approval(
    run_id,
    RichApprovalResolutionReason::Cancellation,
    occurred_at,
  )?;

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

  let opened_loops = engine
    .workflow()
    .graph
    .for_each
    .as_deref()
    .unwrap_or_default()
    .iter()
    .filter(|descriptor| {
      engine.projection(run_id).ok().is_some_and(|projection| {
        projection
          .for_each
          .get(&descriptor.for_each_id)
          .is_some_and(|state| state.status == ForEachStatus::Open)
      })
    })
    .cloned()
    .collect::<Vec<_>>();
  for descriptor in opened_loops {
    settle_open_for_each(
      engine,
      run_id,
      &descriptor,
      ForEachTerminalDisposition::Cancelled,
    )?;
  }

  let projection = engine.projection(run_id)?;
  let cancellation_request_id = projection.cancellation_request_id.ok_or_else(|| {
    RuntimeExecutionError::Stalled("cancelling run has no durable request identity".to_string())
  })?;
  engine.cancel_lock_waiters(run_id, &cancellation_request_id, occurred_at)?;
  for lock in engine
    .projection(run_id)?
    .locks
    .values()
    .filter(|lock| lock.status == crate::LockStatus::Held)
  {
    options.stop_lock_renewal(run_id, &lock.lock_id);
  }
  for (for_each_id, loop_state) in &engine.projection(run_id)?.for_each {
    for (index, execution) in &loop_state.executions {
      let scope = ForEachIterationScope {
        for_each_id: for_each_id.clone(),
        index: *index,
      };
      for lock in execution
        .locks
        .values()
        .filter(|lock| lock.status == crate::LockStatus::Held)
      {
        options.stop_iteration_lock_renewal(run_id, &lock.lock_id, &scope);
      }
    }
  }
  engine.decide_run_cancelled(run_id, cancellation_request_id, occurred_at)
}

async fn settle_script_attempt_failure<E: RuntimeDagEngine>(
  engine: &mut E,
  options: &RuntimeExecutionOptions,
  host: &mut Option<ScriptHostClient>,
  run_id: &str,
  node_id: &str,
  attempt: u32,
  invocation_id: &str,
  failure: AttemptFailure,
) -> Result<ScriptNodeOutcome, RuntimeExecutionError> {
  let failure_event = StepAttemptFailedData {
    node_id: node_id.to_string(),
    attempt,
    invocation_id: invocation_id.to_string(),
    failure: failure.clone(),
  };
  let disposition =
    engine.record_step_attempt_failure(run_id, options.clock.now(), failure_event.clone())?;
  report_attempt_failed(
    engine,
    options,
    run_id,
    node_id,
    attempt,
    failure.code.clone(),
  );
  let saga_failure_deferred = matches!(&disposition, StepFailureDisposition::RunFailureDeferred)
    && engine.workflow().saga_for_body_node(node_id).is_some();
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
    StepFailureDisposition::RunFailureDeferred => {}
    StepFailureDisposition::RunFailed => {}
  }
  if saga_failure_deferred {
    return Ok(ScriptNodeOutcome::SagaFailed(Box::new(failure_event)));
  }
  if reusable_step_descriptor(engine.workflow(), node_id).is_some() && host.is_none() {
    *host = Some(options.acquire_script_host().await?);
  }
  if let Some(host) = host.as_ref() {
    drive_reusable_step_lifecycle(
      engine,
      run_id,
      node_id,
      ReusableInvocationOutcome::Failed,
      None,
      Some(ReusableLifecycleErrorV1 {
        code: failure.code.clone(),
        message: failure.message.clone(),
      }),
      options,
      host,
    )
    .await?;
  }
  release_held_lock_after_body_failure(engine, run_id, node_id, options.clock.now(), options)?;
  if engine.projection(run_id)?.business_outcome.is_none() {
    engine.append_payload(
      run_id,
      RunEventPayload::RunFailed(attempt_run_failed_data(
        engine.event_schema_version(),
        &StepAttemptFailedData {
          node_id: node_id.to_string(),
          attempt,
          invocation_id: invocation_id.to_string(),
          failure: failure.clone(),
        },
      )),
    )?;
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

fn release_held_lock_after_body_failure<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  node_id: &str,
  released_at: chrono::DateTime<chrono::Utc>,
  options: &RuntimeExecutionOptions,
) -> Result<(), RuntimeExecutionError> {
  let Some(descriptor) = lock_descriptor_for_body_node(engine.workflow(), node_id).cloned() else {
    return Ok(());
  };
  let projection = engine.projection(run_id)?;
  let Some(lock) = projection
    .locks
    .get(&descriptor.lock_id)
    .filter(|lock| lock.status == crate::LockStatus::Held)
    .cloned()
  else {
    return Ok(());
  };
  let acquired_at = lock.acquired_at.ok_or_else(|| {
    RuntimeExecutionError::Stalled("A held lock has no acquisition timestamp.".to_string())
  })?;
  let result = serde_json::json!({
    "status": "released",
    "key": lock.key,
    "fencingToken": lock.fencing_token,
    "acquiredAt": acquired_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    "releasedAt": released_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    "waitedMs": (acquired_at - lock.registered_at).num_milliseconds().max(0),
  });
  engine.release_lock(run_id, &descriptor.lock_id, result, released_at)?;
  options.stop_lock_renewal(run_id, &descriptor.lock_id);
  Ok(())
}

fn fail_choice<T, E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  error: BranchEvaluationError,
) -> Result<T, RuntimeExecutionError> {
  let BranchEvaluationError {
    branch_id,
    path,
    kind,
    ..
  } = error;
  let reference = path
    .as_ref()
    .map(|path| format!("context.{}", path.join(".")));
  let (code, message) = match kind {
    BranchEvaluationErrorKind::NotString(actual_type) => (
      "WOML_SWITCH_VALUE_INVALID".to_string(),
      format!(
        "Switch value {} must resolve to a JSON string; received {actual_type}.",
        reference.as_deref().unwrap_or("<unknown>")
      ),
    ),
    BranchEvaluationErrorKind::ReferenceNotAvailable => (
      "WOML_REFERENCE_NOT_AVAILABLE".to_string(),
      format!(
        "Switch reference {} is not available.",
        reference.as_deref().unwrap_or("<unknown>")
      ),
    ),
    BranchEvaluationErrorKind::SelectionInvalid => (
      "WOML_SWITCH_SELECTION_INVALID".to_string(),
      format!("Switch {branch_id:?} has no valid selectable case."),
    ),
    BranchEvaluationErrorKind::NotBoolean(_) => (
      "WOML_SWITCH_SELECTION_INVALID".to_string(),
      format!("Switch {branch_id:?} contains an invalid compiled selector."),
    ),
  };
  let node_id = engine
    .workflow()
    .graph
    .choices
    .as_deref()
    .unwrap_or_default()
    .iter()
    .find(|choice| choice.choice_id == branch_id)
    .map(|choice| choice.selector_node_id.clone())
    .unwrap_or(branch_id);
  let failure = AttemptFailure {
    kind: AttemptFailureKind::ServiceFailed,
    code: code.clone(),
    message: message.clone(),
    details: None,
    ..AttemptFailure::legacy_defaults()
  };
  engine.append_payload(
    run_id,
    RunEventPayload::RunFailed(RunFailedData::V2(RunFailedDataV2::Attempt {
      node_id: node_id.clone(),
      attempt: 1,
      invocation_id: generated_id("inv"),
      failure: failure.clone(),
    })),
  )?;
  Err(RuntimeExecutionError::RunFailed(Box::new(
    FailedRunDetails {
      code,
      message,
      node_id: Some(node_id),
      attempt: Some(1),
      max_attempts: Some(1),
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
    BranchEvaluationErrorKind::NotString(_) => BranchFailure::BranchSelectionInvalid {
      code: "WOML_BRANCH_SELECTION_INVALID".to_string(),
      message: "A string switch reached the conditional-branch failure path.".to_string(),
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
    | crate::RUN_EVENT_SCHEMA_VERSION_V12
    | crate::RUN_EVENT_SCHEMA_VERSION_V13
    | crate::RUN_EVENT_SCHEMA_VERSION_V14
    | crate::RUN_EVENT_SCHEMA_VERSION_V15
    | crate::RUN_EVENT_SCHEMA_VERSION_V16
    | crate::RUN_EVENT_SCHEMA_VERSION_V17
    | crate::RUN_EVENT_SCHEMA_VERSION_V18
    | crate::RUN_EVENT_SCHEMA_VERSION_V19
    | crate::RUN_EVENT_SCHEMA_VERSION_V20 => RunFailedData::V2(RunFailedDataV2::Attempt {
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
  fn append_scoped_payload(
    &mut self,
    run_id: &str,
    iteration: ForEachIterationScope,
    payload: RunEventPayload,
  ) -> Result<(), RuntimeExecutionError>;
  fn append_scoped_payloads(
    &mut self,
    run_id: &str,
    iteration: ForEachIterationScope,
    payloads: Vec<RunEventPayload>,
  ) -> Result<(), RuntimeExecutionError> {
    for payload in payloads {
      self.append_scoped_payload(run_id, iteration.clone(), payload)?;
    }
    Ok(())
  }
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
  fn decide_run_failed(
    &mut self,
    run_id: &str,
    failure: LifecycleFailure,
    _occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    self.append_payload(
      run_id,
      RunEventPayload::RunOutcomeDecided(RunOutcomeDecidedData::Failed { failure }),
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
  fn correction_action_externally_resolved(
    &self,
    _run_id: &str,
    _saga_id: &str,
    _action_id: &str,
  ) -> Result<bool, RuntimeExecutionError> {
    Ok(false)
  }
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
  fn request_rich_approval(
    &mut self,
    _run_id: &str,
    _occurred_at: chrono::DateTime<chrono::Utc>,
    _binding: &BoundRichApprovalReview,
  ) -> Result<(), RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "rich plan approval requires the durable runtime".to_string(),
    ))
  }
  fn settle_pending_rich_approval(
    &mut self,
    _run_id: &str,
    _reason: RichApprovalResolutionReason,
    _settled_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<bool, RuntimeExecutionError> {
    Ok(false)
  }
  fn register_durable_wait(
    &mut self,
    _run_id: &str,
    _occurred_at: chrono::DateTime<chrono::Utc>,
    _registration: crate::WaitRegisteredData,
  ) -> Result<(), RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "durable waits require the durable runtime".to_string(),
    ))
  }
  fn register_lock_waiter(
    &mut self,
    _run_id: &str,
    _lock_id: &str,
    _key: &str,
    _occurred_at: chrono::DateTime<chrono::Utc>,
    _deadline: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "durable locks require the durable runtime".to_string(),
    ))
  }
  fn release_lock(
    &mut self,
    _run_id: &str,
    _lock_id: &str,
    _result: Value,
    _occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "durable locks require the durable runtime".to_string(),
    ))
  }
  fn timeout_lock_waiter(
    &mut self,
    _run_id: &str,
    _lock_id: &str,
    _occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "durable locks require the durable runtime".to_string(),
    ))
  }
  fn renew_lock_lease(
    &mut self,
    _run_id: &str,
    _lock_id: &str,
    _now: chrono::DateTime<chrono::Utc>,
  ) -> Result<crate::LockLeaseMaintenance, RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "durable lock renewal requires the durable runtime".to_string(),
    ))
  }
  fn renew_iteration_lock_lease(
    &mut self,
    _run_id: &str,
    _lock_id: &str,
    _iteration: ForEachIterationScope,
    _now: chrono::DateTime<chrono::Utc>,
  ) -> Result<crate::LockLeaseMaintenance, RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "iteration lock renewal requires the durable runtime".to_string(),
    ))
  }
  fn cancel_lock_waiters(
    &mut self,
    _run_id: &str,
    _cancellation_id: &str,
    _now: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "durable lock cancellation requires the durable runtime".to_string(),
    ))
  }
  fn register_iteration_lock_waiter(
    &mut self,
    _run_id: &str,
    _lock_id: &str,
    _key: &str,
    _iteration: ForEachIterationScope,
    _now: chrono::DateTime<chrono::Utc>,
    _deadline: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "iteration lock acquisition requires the durable runtime".to_string(),
    ))
  }
  fn release_iteration_lock(
    &mut self,
    _run_id: &str,
    _lock_id: &str,
    _iteration: ForEachIterationScope,
    _result: Value,
    _now: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "iteration lock release requires the durable runtime".to_string(),
    ))
  }
  fn timeout_iteration_lock_waiter(
    &mut self,
    _run_id: &str,
    _lock_id: &str,
    _iteration: ForEachIterationScope,
    _now: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "iteration lock timeout requires the durable runtime".to_string(),
    ))
  }
  fn settle_timer_wait(
    &mut self,
    _run_id: &str,
    _wait_id: &str,
    _now: chrono::DateTime<chrono::Utc>,
  ) -> Result<crate::TimerWaitSettlement, RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "durable timer waits require the durable runtime".to_string(),
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
    let handler = self
      .workflow()
      .node(node_id)
      .map(|node| node.handler.clone())
      .ok_or_else(|| {
        RuntimeExecutionError::Stalled(format!("pure result node {node_id:?} disappeared"))
      })?;
    self.append_payload(
      run_id,
      RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
        node_id: node_id.to_string(),
        attempt: 1,
        invocation_id: invocation_id.clone(),
        handler,
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

impl<E: RuntimeDagEngine> RuntimeDagEngine for SharedForEachEngine<'_, E> {
  fn workflow(&self) -> &CompiledWorkflowDefinition {
    self.workflow.as_ref()
  }

  fn definition_hash(&self) -> &str {
    self.definition_hash.as_str()
  }

  fn start_run(
    &mut self,
    run_id: &str,
    trigger: Map<String, Value>,
  ) -> Result<(), RuntimeExecutionError> {
    self.with_engine(|engine| engine.start_run(run_id, trigger))
  }

  fn append_payload(
    &mut self,
    run_id: &str,
    payload: RunEventPayload,
  ) -> Result<(), RuntimeExecutionError> {
    self.with_engine(|engine| engine.append_payload(run_id, payload))
  }

  fn append_scoped_payload(
    &mut self,
    run_id: &str,
    iteration: ForEachIterationScope,
    payload: RunEventPayload,
  ) -> Result<(), RuntimeExecutionError> {
    self.with_engine(|engine| engine.append_scoped_payload(run_id, iteration, payload))
  }

  fn append_scoped_payloads(
    &mut self,
    run_id: &str,
    iteration: ForEachIterationScope,
    payloads: Vec<RunEventPayload>,
  ) -> Result<(), RuntimeExecutionError> {
    self.with_engine(|engine| engine.append_scoped_payloads(run_id, iteration, payloads))
  }

  fn settle_workflow_timeout(
    &mut self,
    run_id: &str,
    occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<bool, RuntimeExecutionError> {
    self.with_engine(|engine| engine.settle_workflow_timeout(run_id, occurred_at))
  }

  fn decide_run_cancelled(
    &mut self,
    run_id: &str,
    cancellation_request_id: String,
    occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    self.with_engine(|engine| {
      engine.decide_run_cancelled(run_id, cancellation_request_id, occurred_at)
    })
  }

  fn decide_run_failed(
    &mut self,
    run_id: &str,
    failure: LifecycleFailure,
    occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    self.with_engine(|engine| engine.decide_run_failed(run_id, failure, occurred_at))
  }

  fn record_step_attempt_failure(
    &mut self,
    run_id: &str,
    failed_at: chrono::DateTime<chrono::Utc>,
    failure: StepAttemptFailedData,
  ) -> Result<StepFailureDisposition, RuntimeExecutionError> {
    self.with_engine(|engine| engine.record_step_attempt_failure(run_id, failed_at, failure))
  }

  fn projection(&self, run_id: &str) -> Result<RunProjection, RuntimeExecutionError> {
    self.with_engine(|engine| engine.projection(run_id))
  }

  fn events(&self, run_id: &str) -> Result<Vec<RunEvent>, RuntimeExecutionError> {
    self.with_engine(|engine| engine.events(run_id))
  }

  fn ready_node_ids(&self, run_id: &str) -> Result<Vec<String>, RuntimeExecutionError> {
    self.with_engine(|engine| engine.ready_node_ids(run_id))
  }

  fn correction_action_externally_resolved(
    &self,
    run_id: &str,
    saga_id: &str,
    action_id: &str,
  ) -> Result<bool, RuntimeExecutionError> {
    self.with_engine(|engine| {
      engine.correction_action_externally_resolved(run_id, saga_id, action_id)
    })
  }

  fn request_approval(
    &mut self,
    run_id: &str,
    occurred_at: chrono::DateTime<chrono::Utc>,
    request: ApprovalRequestedData,
  ) -> Result<IssuedApprovalToken, RuntimeExecutionError> {
    self.with_engine(|engine| engine.request_approval(run_id, occurred_at, request))
  }

  fn request_rich_approval(
    &mut self,
    run_id: &str,
    occurred_at: chrono::DateTime<chrono::Utc>,
    binding: &BoundRichApprovalReview,
  ) -> Result<(), RuntimeExecutionError> {
    self.with_engine(|engine| engine.request_rich_approval(run_id, occurred_at, binding))
  }

  fn settle_pending_rich_approval(
    &mut self,
    run_id: &str,
    reason: RichApprovalResolutionReason,
    settled_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<bool, RuntimeExecutionError> {
    self.with_engine(|engine| engine.settle_pending_rich_approval(run_id, reason, settled_at))
  }

  fn register_durable_wait(
    &mut self,
    run_id: &str,
    occurred_at: chrono::DateTime<chrono::Utc>,
    registration: crate::WaitRegisteredData,
  ) -> Result<(), RuntimeExecutionError> {
    self.with_engine(|engine| engine.register_durable_wait(run_id, occurred_at, registration))
  }

  fn settle_timer_wait(
    &mut self,
    run_id: &str,
    wait_id: &str,
    now: chrono::DateTime<chrono::Utc>,
  ) -> Result<crate::TimerWaitSettlement, RuntimeExecutionError> {
    self.with_engine(|engine| engine.settle_timer_wait(run_id, wait_id, now))
  }

  fn reissue_waiting_outcome(
    &mut self,
    run_id: &str,
    now: chrono::DateTime<chrono::Utc>,
  ) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
    self.with_engine(|engine| engine.reissue_waiting_outcome(run_id, now))
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
          iteration: None,
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
        | crate::RUN_EVENT_SCHEMA_VERSION_V13
        | crate::RUN_EVENT_SCHEMA_VERSION_V14
        | crate::RUN_EVENT_SCHEMA_VERSION_V15
        | crate::RUN_EVENT_SCHEMA_VERSION_V16
        | crate::RUN_EVENT_SCHEMA_VERSION_V17
        | crate::RUN_EVENT_SCHEMA_VERSION_V18
        | crate::RUN_EVENT_SCHEMA_VERSION_V19
        | crate::RUN_EVENT_SCHEMA_VERSION_V20
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
        iteration: None,
        payload,
      })?;
    }
    Ok(())
  }

  fn append_scoped_payload(
    &mut self,
    run_id: &str,
    iteration: ForEachIterationScope,
    payload: RunEventPayload,
  ) -> Result<(), RuntimeExecutionError> {
    self.append_event(RunEvent {
      event_schema_version: self.event_schema_version(),
      event_id: generated_id("evt"),
      run_id: run_id.to_string(),
      sequence: self.events(run_id).len() as u64 + 1,
      occurred_at: chrono::Utc::now(),
      iteration: Some(iteration),
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

  fn correction_action_externally_resolved(
    &self,
    _run_id: &str,
    _saga_id: &str,
    _action_id: &str,
  ) -> Result<bool, RuntimeExecutionError> {
    Ok(false)
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

  fn append_scoped_payload(
    &mut self,
    run_id: &str,
    iteration: ForEachIterationScope,
    payload: RunEventPayload,
  ) -> Result<(), RuntimeExecutionError> {
    self.append_scoped_payload(
      generated_id("evt"),
      run_id,
      chrono::Utc::now(),
      iteration,
      payload,
    )?;
    Ok(())
  }

  fn append_scoped_payloads(
    &mut self,
    run_id: &str,
    iteration: ForEachIterationScope,
    payloads: Vec<RunEventPayload>,
  ) -> Result<(), RuntimeExecutionError> {
    self.append_scoped_payloads_atomically(run_id, iteration, payloads)?;
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

  fn decide_run_failed(
    &mut self,
    run_id: &str,
    failure: LifecycleFailure,
    occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::decide_run_outcome(
      self,
      run_id,
      RunOutcomeDecidedData::Failed { failure },
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

  fn correction_action_externally_resolved(
    &self,
    run_id: &str,
    saga_id: &str,
    action_id: &str,
  ) -> Result<bool, RuntimeExecutionError> {
    Ok(DurableDagEngine::correction_action_externally_resolved(
      self, run_id, saga_id, action_id,
    )?)
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

  fn request_rich_approval(
    &mut self,
    run_id: &str,
    occurred_at: chrono::DateTime<chrono::Utc>,
    binding: &BoundRichApprovalReview,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::request_rich_approval(self, run_id, occurred_at, binding)?;
    Ok(())
  }

  fn settle_pending_rich_approval(
    &mut self,
    run_id: &str,
    reason: RichApprovalResolutionReason,
    settled_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<bool, RuntimeExecutionError> {
    Ok(DurableDagEngine::settle_pending_rich_approval(
      self, run_id, reason, settled_at,
    )?)
  }

  fn register_durable_wait(
    &mut self,
    run_id: &str,
    occurred_at: chrono::DateTime<chrono::Utc>,
    registration: crate::WaitRegisteredData,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::register_durable_wait(self, run_id, occurred_at, registration)?;
    Ok(())
  }

  fn register_lock_waiter(
    &mut self,
    run_id: &str,
    lock_id: &str,
    key: &str,
    occurred_at: chrono::DateTime<chrono::Utc>,
    deadline: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::register_lock_waiter(self, run_id, lock_id, key, occurred_at, deadline)?;
    Ok(())
  }

  fn release_lock(
    &mut self,
    run_id: &str,
    lock_id: &str,
    result: Value,
    occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::release_lock(self, run_id, lock_id, result, occurred_at)?;
    Ok(())
  }

  fn timeout_lock_waiter(
    &mut self,
    run_id: &str,
    lock_id: &str,
    occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::timeout_lock_waiter(self, run_id, lock_id, occurred_at)?;
    Ok(())
  }

  fn renew_lock_lease(
    &mut self,
    run_id: &str,
    lock_id: &str,
    now: chrono::DateTime<chrono::Utc>,
  ) -> Result<crate::LockLeaseMaintenance, RuntimeExecutionError> {
    Ok(DurableDagEngine::renew_lock_lease(
      self, run_id, lock_id, now,
    )?)
  }

  fn renew_iteration_lock_lease(
    &mut self,
    run_id: &str,
    lock_id: &str,
    iteration: ForEachIterationScope,
    now: chrono::DateTime<chrono::Utc>,
  ) -> Result<crate::LockLeaseMaintenance, RuntimeExecutionError> {
    Ok(DurableDagEngine::renew_iteration_lock_lease(
      self, run_id, lock_id, iteration, now,
    )?)
  }

  fn cancel_lock_waiters(
    &mut self,
    run_id: &str,
    cancellation_id: &str,
    now: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::cancel_lock_waiters(self, run_id, cancellation_id, now)?;
    Ok(())
  }

  fn register_iteration_lock_waiter(
    &mut self,
    run_id: &str,
    lock_id: &str,
    key: &str,
    iteration: ForEachIterationScope,
    now: chrono::DateTime<chrono::Utc>,
    deadline: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::register_iteration_lock_waiter(
      self, run_id, lock_id, key, iteration, now, deadline,
    )?;
    Ok(())
  }

  fn release_iteration_lock(
    &mut self,
    run_id: &str,
    lock_id: &str,
    iteration: ForEachIterationScope,
    result: Value,
    now: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::release_iteration_lock(self, run_id, lock_id, iteration, result, now)?;
    Ok(())
  }

  fn timeout_iteration_lock_waiter(
    &mut self,
    run_id: &str,
    lock_id: &str,
    iteration: ForEachIterationScope,
    now: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::timeout_iteration_lock_waiter(self, run_id, lock_id, iteration, now)?;
    Ok(())
  }

  fn settle_timer_wait(
    &mut self,
    run_id: &str,
    wait_id: &str,
    now: chrono::DateTime<chrono::Utc>,
  ) -> Result<crate::TimerWaitSettlement, RuntimeExecutionError> {
    Ok(DurableDagEngine::settle_timer_wait(
      self, run_id, wait_id, now,
    )?)
  }

  fn reissue_waiting_outcome(
    &mut self,
    run_id: &str,
    now: chrono::DateTime<chrono::Utc>,
  ) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
    let projection = self.projection(run_id)?;
    if let Some(request) = projection
      .rich_approval_requests
      .values()
      .find(|request| matches!(request.status, crate::RichApprovalRequestStatus::Waiting))
      .cloned()
    {
      let approval = self
        .workflow()
        .approval(&request.approval_id)
        .ok_or_else(|| {
          RuntimeExecutionError::Stalled(
            "waiting run references an unknown rich approval definition".to_string(),
          )
        })?;
      let inspection = DurableDagEngine::inspect_rich_approval(self, run_id, &request.request_id)?;
      let expires_at = approval
        .timeout_ms
        .map(|milliseconds| {
          i64::try_from(milliseconds)
            .map(chrono::Duration::milliseconds)
            .map(|duration| request.requested_at + duration)
        })
        .transpose()
        .map_err(|_| {
          RuntimeExecutionError::Stalled(
            "rich approval timeout exceeds the clock range".to_string(),
          )
        })?;
      let waiting_expires_at = match (expires_at, projection.timeout_at) {
        (Some(approval), Some(workflow)) => Some(approval.min(workflow)),
        (Some(approval), None) => Some(approval),
        (None, Some(workflow)) => Some(workflow),
        (None, None) => None,
      };
      if waiting_expires_at.is_some_and(|deadline| now >= deadline) {
        return Err(RuntimeExecutionError::Stalled(
          "rich approval deadline must be settled before wait reissue".to_string(),
        ));
      }
      let on_timeout = if projection
        .timeout_at
        .is_some_and(|workflow| expires_at.is_none_or(|approval| workflow <= approval))
      {
        ApprovalTimeoutPolicy::Fail
      } else if approval.on_timeout == "reject" {
        ApprovalTimeoutPolicy::Reject
      } else {
        ApprovalTimeoutPolicy::Fail
      };
      let workflow_version = self
        .workflow()
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.version.as_deref())
        .unwrap_or("0.0.0")
        .to_string();
      return Ok(rich_waiting_outcome(
        self.workflow().workflow_id.clone(),
        run_id.to_string(),
        approval,
        waiting_expires_at,
        on_timeout,
        BoundRichApprovalReview {
          profile: crate::RICH_PLAN_APPROVAL_PROFILE.to_string(),
          definition_hash: self.definition_hash().to_string(),
          workflow_id: self.workflow().workflow_id.clone(),
          workflow_version,
          run_id: run_id.to_string(),
          approval_id: request.approval_id,
          request_id: request.request_id,
          artifacts: inspection.review.artifacts.clone(),
          review: inspection.review,
          required_approvals: inspection.required,
          required_roles: inspection.required_roles,
          directory_version: inspection.directory_version,
          review_digest: inspection.review_digest,
        },
        request
          .decisions
          .values()
          .filter(|decision| decision.decision == crate::ApprovalDecision::Approved)
          .count() as u32,
        request.represented_roles,
      ));
    }
    if let Some(request) = projection
      .approval_requests
      .values()
      .find(|request| matches!(request.status, ApprovalRequestStatus::Waiting))
      .cloned()
    {
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
      return Ok(waiting_outcome(
        self.workflow().workflow_id.clone(),
        run_id.to_string(),
        approval,
        request.request_id,
        request.expires_at,
        request.on_timeout,
        token,
      ));
    }
    let wait = projection
      .durable_waits
      .values()
      .find(|wait| wait.status == crate::DurableWaitStatus::Registered)
      .ok_or_else(|| {
        RuntimeExecutionError::Stalled(
          "waiting run has no unresolved approval or durable wait".to_string(),
        )
      })?;
    if wait.kind == crate::DurableWaitKind::Signal {
      return Ok(WorkflowRuntimeOutcome::WaitingSignal {
        contract: RUNTIME_OUTCOME_CONTRACT,
        version: RUNTIME_OUTCOME_VERSION,
        workflow_id: self.workflow().workflow_id.clone(),
        run_id: run_id.to_string(),
        wait: WaitingWorkflowSignal {
          wait_id: wait.wait_id.clone(),
          node_id: wait.node_id.clone(),
          registered_at: wait.registered_at,
          signals: wait.signals.clone(),
          deadline: wait.deadline,
        },
      });
    }
    let deadline = wait.deadline.ok_or_else(|| {
      RuntimeExecutionError::Stalled("durable timer wait has no deadline".to_string())
    })?;
    Ok(WorkflowRuntimeOutcome::WaitingTime {
      contract: RUNTIME_OUTCOME_CONTRACT,
      version: RUNTIME_OUTCOME_VERSION,
      workflow_id: self.workflow().workflow_id.clone(),
      run_id: run_id.to_string(),
      wait: WaitingWorkflowTimer {
        wait_id: wait.wait_id.clone(),
        node_id: wait.node_id.clone(),
        kind: wait.kind,
        registered_at: wait.registered_at,
        deadline,
      },
    })
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
