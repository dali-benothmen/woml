// WOML-only N-API boundary for the Rust execution path.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{
  atomic::{AtomicBool, AtomicU64, Ordering},
  mpsc, Arc, Mutex, OnceLock,
};
use std::thread::JoinHandle;

use chrono::{DateTime, Utc};
use napi::threadsafe_function::{ErrorStrategy, ThreadsafeFunctionCallMode};
use napi::{Env, JsFunction, JsObject, Status};
use napi_derive::napi;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use woml_engine::model::ValueExpression;
use woml_engine::{
  admit_external_signal_durable, create_online_backup, execute_retention, execute_workflow,
  execute_workflow_durable, execute_workflow_durable_outcome, export_artifact,
  export_artifact_backup, inspect_artifact, inspect_backup_store, inspect_run_artifacts,
  last_retention_result, maintain_artifacts, observe_artifacts, plan_retention,
  prepare_emergency_deny_overlay, prepare_infrastructure_policy, prepare_restored_store,
  recent_run_presentations_from_store_v1, record_verified_backup, recover_durable_runs,
  resolve_human_approval_durable, resolve_rich_approval_durable, restore_artifact_backup,
  resume_workflow_durable, resume_workflow_durable_any_outcome, resume_workflow_durable_outcome,
  run_notification_provider_journey_with_custom, run_presentation_from_store_v1,
  settle_approval_timeout_durable, verify_artifact_backup, ApprovalDecision,
  ApprovalDecisionOutcome, ArtifactPolicyV1, ArtifactRuntimeMode, BackupError,
  CompiledReusableInvocation, CompiledWorkflowDefinition, CorrectionRecoveryRequestV1,
  CustomNotificationJourneyOptions, CustomProviderScriptArtifact, DurableEventStore,
  DurableStoreError, ExternalSignalAdmissionOutcome, ExternalSignalAdmissionRequest,
  ExternalTriggerAdmissionCommand, InfrastructurePolicyBundleV1, InfrastructurePolicyTrustV1,
  IntervalProgress, IntervalProgressReporter, LifecycleProgress, NotificationHostClientError,
  NotificationHostProcessOptions, NotificationJourneyDiagnostics, NotificationJourneyError,
  OperationalOutputReporter, ParallelFailurePolicy, ProcessPolicyV1, ProcessRuntimeMode,
  RemoteDispatchResolutionV1, RetentionError, RetentionPolicyV1, ReviewerDirectoryAuthorityV1,
  ReviewerDirectoryV1, ReviewerIdentityClaimV1, ReviewerProviderV1, RichApprovalDecisionStatus,
  RichApprovalIdentityDecision, RunFailure, RunPresentationError, RunStatus, RuntimeExecutionError,
  RuntimeExecutionOptions, RuntimeModuleArtifact, RuntimePolicyProgress,
  RuntimePolicyProgressReporter, ScheduleProgress, ScheduleProgressReporter,
  ScriptHostProcessOptions, SystemEngineClock, TimerWaitSettlement, TriggerAdmissionRequest,
  TriggerProgress, TriggerProgressReporter, WebhookDefinitionRegistration, WebhookRuntimeError,
  WomlWebhookServer, WomlWebhookServerConfig, WorkflowCallProgress, WorkflowCallProgressReporter,
  WorkflowCallRunRelations, COMPILED_MODEL_SCHEMA_VERSION_V19,
};

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum NativeTimerWaitSettlement {
  Resolved,
  AlreadyResolved,
  NotDue {
    #[serde(rename = "deadline")]
    deadline: DateTime<Utc>,
  },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeParallelExecutionErrorDetails {
  parallel_id: String,
  policy: ParallelFailurePolicy,
  primary_node_id: String,
  failed_node_ids: Vec<String>,
  cancelled_node_ids: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeExecutionError {
  kind: &'static str,
  code: String,
  message: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  node_id: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  branch_id: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  arm_id: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  reference_path: Option<Vec<String>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  branch_site: Option<&'static str>,
  #[serde(skip_serializing_if = "Option::is_none")]
  approval_id: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  request_id: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  attempt: Option<u32>,
  #[serde(skip_serializing_if = "Option::is_none")]
  max_attempts: Option<u32>,
  #[serde(skip_serializing_if = "Option::is_none")]
  failure_code: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  details: Option<NativeParallelExecutionErrorDetails>,
}

fn native_execution_error(error: RuntimeExecutionError) -> napi::Error {
  let envelope = match error {
    RuntimeExecutionError::DurableStore(DurableStoreError::RuntimePolicyQueueFull) => {
      NativeExecutionError {
        kind: "woml_execution_error",
        code: "WOML_POLICY_QUEUE_FULL".to_string(),
        message: "The durable WOML policy queue is full; retry this run.".to_string(),
        node_id: None,
        branch_id: None,
        arm_id: None,
        reference_path: None,
        branch_site: None,
        approval_id: None,
        request_id: None,
        attempt: None,
        max_attempts: None,
        failure_code: None,
        details: None,
      }
    }
    RuntimeExecutionError::RunFailed(details) => NativeExecutionError {
      kind: "woml_execution_error",
      code: details.code.clone(),
      message: details.message.clone(),
      node_id: details.node_id.clone(),
      branch_id: None,
      arm_id: None,
      reference_path: None,
      branch_site: None,
      approval_id: None,
      request_id: None,
      attempt: details.attempt,
      max_attempts: details.max_attempts,
      failure_code: Some(details.failure.code.clone()),
      details: None,
    },
    RuntimeExecutionError::BranchFailed(details) => NativeExecutionError {
      kind: "woml_execution_error",
      code: details.code.clone(),
      message: details.message.clone(),
      node_id: None,
      branch_id: Some(details.branch_id.clone()),
      arm_id: details.arm_id.clone(),
      reference_path: details.path.clone(),
      branch_site: Some(details.site.as_str()),
      approval_id: None,
      request_id: None,
      attempt: None,
      max_attempts: None,
      failure_code: None,
      details: None,
    },
    RuntimeExecutionError::ParallelFailed(details) => NativeExecutionError {
      kind: "woml_execution_error",
      code: details.code.clone(),
      message: details.message.clone(),
      node_id: Some(details.primary_node_id.clone()),
      branch_id: None,
      arm_id: None,
      reference_path: None,
      branch_site: None,
      approval_id: None,
      request_id: None,
      attempt: None,
      max_attempts: None,
      failure_code: None,
      details: Some(NativeParallelExecutionErrorDetails {
        parallel_id: details.parallel_id.clone(),
        policy: details.policy,
        primary_node_id: details.primary_node_id.clone(),
        failed_node_ids: details.failed_node_ids.clone(),
        cancelled_node_ids: details.cancelled_node_ids.clone(),
      }),
    },
    RuntimeExecutionError::ApprovalFailed(details) => NativeExecutionError {
      kind: "woml_execution_error",
      code: details.code.clone(),
      message: details.message.clone(),
      node_id: None,
      branch_id: None,
      arm_id: None,
      reference_path: None,
      branch_site: None,
      approval_id: Some(details.approval_id.clone()),
      request_id: Some(details.request_id.clone()),
      attempt: None,
      max_attempts: None,
      failure_code: None,
      details: None,
    },
    RuntimeExecutionError::RunCancelled(details) => NativeExecutionError {
      kind: "woml_execution_error",
      code: details.code.clone(),
      message: format!(
        "Workflow run {:?} was cancelled by request {:?}.",
        details.run_id, details.cancellation_request_id
      ),
      node_id: None,
      branch_id: None,
      arm_id: None,
      reference_path: None,
      branch_site: None,
      approval_id: None,
      request_id: None,
      attempt: None,
      max_attempts: None,
      failure_code: None,
      details: None,
    },
    error => NativeExecutionError {
      kind: "woml_execution_error",
      code: "WOML_RUST_EXECUTION_FAILED".to_string(),
      message: error.to_string(),
      node_id: None,
      branch_id: None,
      arm_id: None,
      reference_path: None,
      branch_site: None,
      approval_id: None,
      request_id: None,
      attempt: None,
      max_attempts: None,
      failure_code: None,
      details: None,
    },
  };
  let reason = serde_json::to_string(&envelope).unwrap_or_else(|_| {
    "WOML Rust execution failed and its error could not be encoded.".to_string()
  });
  napi::Error::from_reason(reason)
}

#[derive(Serialize)]
struct NativeApprovalDecisionOutcome {
  contract: &'static str,
  version: u32,
  #[serde(flatten)]
  outcome: ApprovalDecisionOutcome,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeRichApprovalDecisionOutcome {
  contract: &'static str,
  version: u32,
  status: &'static str,
  run_id: String,
  approval_id: String,
  request_id: String,
  review_digest: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeApprovalError {
  kind: &'static str,
  code: &'static str,
  message: &'static str,
}

fn native_approval_error(error: RuntimeExecutionError) -> napi::Error {
  let (code, message) = match error {
    RuntimeExecutionError::DurableStore(DurableStoreError::InvalidApprovalToken) => (
      "WOML_APPROVAL_TOKEN_INVALID",
      "The approval capability is invalid.",
    ),
    RuntimeExecutionError::DurableStore(DurableStoreError::ExpiredApprovalToken) => (
      "WOML_APPROVAL_TOKEN_EXPIRED",
      "The approval capability expired.",
    ),
    RuntimeExecutionError::DurableStore(DurableStoreError::ApprovalExpired) => {
      ("WOML_APPROVAL_EXPIRED", "The approval request expired.")
    }
    RuntimeExecutionError::DurableStore(DurableStoreError::ApprovalDecisionConflict) => (
      "WOML_APPROVAL_DECISION_CONFLICT",
      "A different human decision is already durable.",
    ),
    RuntimeExecutionError::DurableStore(DurableStoreError::RichApprovalBindingMismatch) => (
      "WOML_APPROVAL_BINDING_MISMATCH",
      "The decision does not match the immutable review.",
    ),
    RuntimeExecutionError::DurableStore(DurableStoreError::RichApprovalReviewerConflict) => (
      "WOML_APPROVAL_DUPLICATE_REVIEWER",
      "The reviewer already recorded a different decision.",
    ),
    RuntimeExecutionError::DurableStore(DurableStoreError::ReviewerDirectory(error)) => (
      error.code(),
      "The reviewer identity is not authorized for this review.",
    ),
    RuntimeExecutionError::DurableStore(DurableStoreError::RichApprovalNotFound) => (
      "WOML_APPROVAL_ALREADY_RESOLVED",
      "The rich approval request is unavailable or already retired.",
    ),
    RuntimeExecutionError::InvalidConfiguration(message)
      if message.starts_with("WOML_APPROVAL_AUTHORITY_UNAVAILABLE") =>
    {
      (
        "WOML_APPROVAL_AUTHORITY_UNAVAILABLE",
        "The trusted reviewer authority is unavailable.",
      )
    }
    _ => (
      "WOML_APPROVAL_INTERNAL",
      "The approval decision could not be safely confirmed.",
    ),
  };
  let envelope = NativeApprovalError {
    kind: "woml_approval_error",
    code,
    message,
  };
  let reason = serde_json::to_string(&envelope)
    .unwrap_or_else(|_| "WOML approval failed and its error could not be encoded.".to_string());
  napi::Error::from_reason(reason)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeNotificationError {
  kind: &'static str,
  code: &'static str,
  message: &'static str,
  #[serde(skip_serializing_if = "Option::is_none")]
  diagnostics: Option<NotificationJourneyDiagnostics>,
}

fn native_notification_error(error: NotificationJourneyError) -> napi::Error {
  let (code, message, diagnostics) = match error {
    NotificationJourneyError::DeliveryFailed(diagnostics) => (
      "WOML_NOTIFICATION_DELIVERY_FAILED",
      "Every configured approval notification delivery failed.",
      Some(diagnostics),
    ),
    NotificationJourneyError::Host(NotificationHostClientError::InteractionTimedOut) => (
      "WOML_NOTIFICATION_INTERACTION_TIMEOUT",
      "No provider approval action arrived before the local wait deadline.",
      None,
    ),
    NotificationJourneyError::Host(NotificationHostClientError::Protocol(_)) => (
      "WOML_NOTIFICATION_RESPONSE_INVALID",
      "The notification provider host violated its frozen protocol.",
      None,
    ),
    NotificationJourneyError::Host(_) => (
      "WOML_NOTIFICATION_HOST_CRASHED",
      "The notification provider host stopped unexpectedly.",
      None,
    ),
    NotificationJourneyError::Store(DurableStoreError::ApprovalDecisionConflict) => (
      "WOML_APPROVAL_DECISION_CONFLICT",
      "A different human decision is already durable.",
      None,
    ),
    NotificationJourneyError::Store(DurableStoreError::ExpiredApprovalToken)
    | NotificationJourneyError::Store(DurableStoreError::ApprovalExpired) => (
      "WOML_APPROVAL_EXPIRED",
      "The approval request or provider capability expired.",
      None,
    ),
    _ => (
      "WOML_NOTIFICATION_INTERNAL",
      "The notification provider journey could not be completed safely.",
      None,
    ),
  };
  let envelope = NativeNotificationError {
    kind: "woml_notification_error",
    code,
    message,
    diagnostics,
  };
  let reason = serde_json::to_string(&envelope).unwrap_or_else(|_| {
    "WOML notification provider journey failed and its error could not be encoded.".to_string()
  });
  napi::Error::from_reason(reason)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeWebhookRegistration {
  workflow: CompiledWorkflowDefinition,
  definition_hash: String,
  resolved_secrets: BTreeMap<String, String>,
  #[serde(default)]
  runtime_modules: Vec<RuntimeModuleArtifact>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeWebhookRuntimeStarted {
  runtime_id: String,
  host: String,
  port: u16,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeStoredRunRequirements {
  contract: &'static str,
  version: u32,
  workflow_id: String,
  definition_hash: String,
  required_secrets: Vec<String>,
  module_count: usize,
  has_approval: bool,
  has_notifications: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeTriggerRuntimeError {
  kind: &'static str,
  code: &'static str,
  message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeRuntimeInstanceProgress {
  profile: &'static str,
  deployment_id: String,
  activation_id: String,
  runtime_instance_id: String,
  runtime_version: &'static str,
  native_version: &'static str,
  lifecycle: &'static str,
  started_at: DateTime<Utc>,
  heartbeat_at: DateTime<Utc>,
  lease_expires_at: DateTime<Utc>,
}

type NativeRuntimeInstanceReporter = Arc<dyn Fn(NativeRuntimeInstanceProgress) + Send + Sync>;

struct NativeWebhookRuntimeThread {
  control: mpsc::Sender<NativeWebhookRuntimeCommand>,
  ingress: tokio::sync::mpsc::UnboundedSender<ExternalTriggerAdmissionCommand>,
  manual_targets: BTreeMap<(String, String), NativeManualTarget>,
  join: JoinHandle<()>,
}

#[derive(Clone)]
struct NativeManualTarget {
  definition_hash: String,
  admission_status: &'static str,
}

enum NativeWebhookRuntimeCommand {
  Activate(mpsc::SyncSender<Result<(), NativeTriggerRuntimeError>>),
  Signal(
    ExternalSignalAdmissionRequest,
    mpsc::SyncSender<Result<ExternalSignalAdmissionOutcome, RuntimeExecutionError>>,
  ),
  OwnershipLost,
  Stop,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeTriggerIngressRequest {
  contract: String,
  contract_version: u32,
  message_type: String,
  request_id: String,
  workflow_id: String,
  definition_hash: String,
  trigger_id: String,
  trigger_handler: String,
  source_identity: String,
  payload: Map<String, Value>,
  received_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeTriggerIngressAccepted {
  contract: &'static str,
  contract_version: u32,
  message_type: &'static str,
  request_id: String,
  occurrence_id: String,
  run_id: String,
  duplicate: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeTriggerIngressRejected {
  contract: &'static str,
  contract_version: u32,
  message_type: &'static str,
  request_id: String,
  failure: NativeTriggerIngressFailure,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeTriggerIngressFailure {
  code: &'static str,
  message: &'static str,
  retryable: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeManualAdmissionRequest {
  profile: String,
  #[serde(rename = "type")]
  message_type: String,
  request_id: String,
  workflow_id: String,
  trigger_id: String,
  payload: Map<String, Value>,
  requested_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeManualAdmissionAccepted {
  profile: &'static str,
  #[serde(rename = "type")]
  message_type: &'static str,
  request_id: String,
  occurrence_id: String,
  run_id: String,
  status: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeManualAdmissionRejected {
  profile: &'static str,
  #[serde(rename = "type")]
  message_type: &'static str,
  request_id: String,
  code: &'static str,
  message: &'static str,
}

fn encode_manual_rejection(
  request_id: String,
  code: &'static str,
  message: &'static str,
) -> napi::Result<String> {
  serde_json::to_string(&NativeManualAdmissionRejected {
    profile: "woml.manual-trigger-admission/v1",
    message_type: "rejected",
    request_id,
    code,
    message,
  })
  .map_err(|error| {
    napi::Error::from_reason(format!(
      "Could not encode manual admission rejection: {error}"
    ))
  })
}

static WEBHOOK_RUNTIMES: OnceLock<Mutex<HashMap<String, NativeWebhookRuntimeThread>>> =
  OnceLock::new();

fn webhook_runtimes() -> &'static Mutex<HashMap<String, NativeWebhookRuntimeThread>> {
  WEBHOOK_RUNTIMES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn native_trigger_runtime_error(error: WebhookRuntimeError) -> NativeTriggerRuntimeError {
  let code = match error {
    WebhookRuntimeError::RouteConflict(_) => "WOML_WEBHOOK_ROUTE_CONFLICT",
    WebhookRuntimeError::SecretMissing(_) => "WOML_WEBHOOK_SECRET_MISSING",
    WebhookRuntimeError::DurableStore(DurableStoreError::RuntimePolicyDefinitionConflict(_)) => {
      "WOML_POLICY_CONFLICT"
    }
    WebhookRuntimeError::InvalidSchema { .. } => "WOML_TRIGGER_SCHEMA_INVALID",
    WebhookRuntimeError::DurableStore(DurableStoreError::WorkflowRuntimeDuplicateOwner(_)) => {
      "WOML_WORKFLOW_TARGET_AMBIGUOUS"
    }
    WebhookRuntimeError::DurableStore(DurableStoreError::DeploymentRuntimeOwned { .. }) => {
      "WOML_DEPLOYMENT_ALREADY_RUNNING"
    }
    WebhookRuntimeError::DurableStore(_) => "WOML_TRIGGER_UNAVAILABLE",
    WebhookRuntimeError::Io(_) => "WOML_WEBHOOK_BIND_FAILED",
    WebhookRuntimeError::InvalidRegistration(_) | WebhookRuntimeError::Model(_) => {
      "WOML_TRIGGER_REGISTRATION_INVALID"
    }
  };
  NativeTriggerRuntimeError {
    kind: "woml_trigger_runtime_error",
    code,
    message: error.to_string(),
  }
}

fn trigger_runtime_napi_error(error: NativeTriggerRuntimeError) -> napi::Error {
  let reason = serde_json::to_string(&error)
    .unwrap_or_else(|_| "WOML trigger runtime failed to start.".to_string());
  napi::Error::from_reason(reason)
}

fn native_operational_output_reporter(
  env: &Env,
  progress_callback: &JsFunction,
) -> napi::Result<OperationalOutputReporter> {
  let scheduled = Arc::new(AtomicBool::new(false));
  let newest_sequence = Arc::new(AtomicU64::new(0));
  let callback_scheduled = Arc::clone(&scheduled);
  let callback_sequence = Arc::clone(&newest_sequence);
  let mut wakeup = progress_callback
    .create_threadsafe_function::<(), String, _, ErrorStrategy::Fatal>(1, move |_| {
      callback_scheduled.store(false, Ordering::Release);
      Ok(vec![format!(
        "{{\"profile\":\"woml.operational-output-wakeup/v1\",\"newestSequence\":{}}}",
        callback_sequence.load(Ordering::Acquire)
      )])
    })?;
  wakeup.unref(env)?;
  let wakeup = Arc::new(wakeup);
  Ok(Arc::new(move |sequence| {
    newest_sequence.fetch_max(sequence, Ordering::AcqRel);
    if scheduled
      .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
      .is_ok()
    {
      if wakeup.call((), ThreadsafeFunctionCallMode::NonBlocking) != Status::Ok {
        scheduled.store(false, Ordering::Release);
      }
    }
  }))
}

fn native_runtime_progress_reporters(
  env: &Env,
  progress_callback: JsFunction,
  telemetry_enabled: bool,
) -> napi::Result<(
  TriggerProgressReporter,
  ScheduleProgressReporter,
  IntervalProgressReporter,
  WorkflowCallProgressReporter,
  RuntimePolicyProgressReporter,
  NativeRuntimeInstanceReporter,
  OperationalOutputReporter,
  Option<woml_engine::TelemetrySignalReporter>,
  Option<woml_engine::TelemetryAdmissionLinkReporter>,
)> {
  let operational_output_reporter = native_operational_output_reporter(env, &progress_callback)?;
  let (telemetry_reporter, telemetry_admission_link_reporter) = if telemetry_enabled {
    let mut telemetry = progress_callback
      .create_threadsafe_function::<String, String, _, ErrorStrategy::Fatal>(1024, |context| {
        Ok(vec![context.value])
      })?;
    telemetry.unref(env)?;
    let telemetry = Arc::new(telemetry);
    let admission_links = Arc::clone(&telemetry);
    (
      Some(woml_engine::bounded_telemetry_reporter(
        1024,
        Arc::new(move |message| {
          if let Ok(json) = serde_json::to_string(&message) {
            let _ = telemetry.call(json, ThreadsafeFunctionCallMode::NonBlocking);
          }
        }),
      )),
      Some(
        Arc::new(move |message: woml_engine::TelemetryAdmissionLinkV1| {
          if let Ok(json) = serde_json::to_string(&message) {
            let _ = admission_links.call(json, ThreadsafeFunctionCallMode::NonBlocking);
          }
        }) as woml_engine::TelemetryAdmissionLinkReporter,
      ),
    )
  } else {
    (None, None)
  };
  let mut progress = progress_callback
    .create_threadsafe_function::<String, String, _, ErrorStrategy::Fatal>(0, |context| {
      Ok(vec![context.value])
    })?;
  progress.unref(env)?;
  let progress = Arc::new(progress);
  let trigger_progress = progress.clone();
  let schedule_progress = progress.clone();
  let interval_progress = progress;
  let workflow_call_progress = interval_progress.clone();
  let runtime_policy_progress = workflow_call_progress.clone();
  let runtime_instance_progress = runtime_policy_progress.clone();
  Ok((
    Arc::new(move |message: TriggerProgress| {
      if let Ok(json) = serde_json::to_string(&message) {
        let _ = trigger_progress.call(json, ThreadsafeFunctionCallMode::Blocking);
      }
    }),
    Arc::new(move |message: ScheduleProgress| {
      if let Ok(json) = serde_json::to_string(&message) {
        let _ = schedule_progress.call(json, ThreadsafeFunctionCallMode::Blocking);
      }
    }),
    Arc::new(move |message: IntervalProgress| {
      if let Ok(json) = serde_json::to_string(&message) {
        let _ = interval_progress.call(json, ThreadsafeFunctionCallMode::Blocking);
      }
    }),
    Arc::new(move |message: WorkflowCallProgress| {
      if let Ok(json) = serde_json::to_string(&message) {
        let _ = workflow_call_progress.call(json, ThreadsafeFunctionCallMode::Blocking);
      }
    }),
    Arc::new(move |message: RuntimePolicyProgress| {
      if let Ok(json) = serde_json::to_string(&message) {
        let _ = runtime_policy_progress.call(json, ThreadsafeFunctionCallMode::Blocking);
      }
    }),
    Arc::new(move |message: NativeRuntimeInstanceProgress| {
      if let Ok(json) = serde_json::to_string(&message) {
        let _ = runtime_instance_progress.call(json, ThreadsafeFunctionCallMode::Blocking);
      }
    }),
    operational_output_reporter,
    telemetry_reporter,
    telemetry_admission_link_reporter,
  ))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeRunInspection {
  run_id: String,
  workflow_id: String,
  status: RunStatus,
  #[serde(skip_serializing_if = "Option::is_none")]
  terminal_node_id: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  result: Option<Value>,
  #[serde(skip_serializing_if = "Option::is_none")]
  failure_code: Option<String>,
  workflow_calls: WorkflowCallRunRelations,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeRunInspectionError {
  kind: &'static str,
  code: &'static str,
  message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeRunManagementError {
  kind: &'static str,
  code: &'static str,
  message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeBackupError {
  kind: &'static str,
  code: &'static str,
  message: String,
}

fn native_backup_error(error: BackupError, fallback: &'static str) -> napi::Error {
  let code = match error {
    BackupError::StorePathUnsafe => "WOML_BACKUP_SOURCE_INVALID",
    BackupError::DestinationUnsafe => "WOML_BACKUP_DESTINATION_INVALID",
    BackupError::UnsupportedStoreVersion(_) => "WOML_STORE_VERSION_UNSUPPORTED",
    BackupError::MaintenanceBusy => "WOML_MAINTENANCE_BUSY",
    BackupError::EmptyDefinitionInventory
    | BackupError::DefinitionInventoryMismatch
    | BackupError::IntegrityFailed => "WOML_BACKUP_VERIFICATION_FAILED",
    _ => fallback,
  };
  let reason = serde_json::to_string(&NativeBackupError {
    kind: "woml_backup_error",
    code,
    message: error.to_string(),
  })
  .unwrap_or_else(|_| "WOML backup operation failed.".to_string());
  napi::Error::from_reason(reason)
}

fn native_retention_error(error: RetentionError) -> napi::Error {
  let code = match error {
    RetentionError::StorePathUnsafe => "WOML_RETENTION_STORE_INVALID",
    RetentionError::InvalidPolicy => "WOML_RETENTION_PLAN_INVALID",
    RetentionError::MaintenanceBusy | RetentionError::StoreBusy => "WOML_MAINTENANCE_BUSY",
    RetentionError::IntegrityFailed => "WOML_RETENTION_INTEGRITY_FAILED",
    RetentionError::DiskFull => "WOML_MAINTENANCE_DISK_FULL",
    RetentionError::Sqlite(_) | RetentionError::Io(_) => "WOML_RETENTION_FAILED",
  };
  let reason = serde_json::to_string(&NativeBackupError {
    kind: "woml_retention_error",
    code,
    message: error.to_string(),
  })
  .unwrap_or_else(|_| "WOML retention operation failed.".to_string());
  napi::Error::from_reason(reason)
}

fn native_run_management_error(code: &'static str, error: DurableStoreError) -> napi::Error {
  let reason = serde_json::to_string(&NativeRunManagementError {
    kind: "woml_run_management_error",
    code,
    message: error.to_string(),
  })
  .unwrap_or_else(|_| "WOML run management failed.".to_string());
  napi::Error::from_reason(reason)
}

fn native_run_presentation_error(error: RunPresentationError) -> napi::Error {
  let code = match &error {
    RunPresentationError::Store(DurableStoreError::RunNotFound(_)) => "WOML_RUN_NOT_FOUND",
    RunPresentationError::Store(DurableStoreError::Sqlite(_)) => "WOML_RUN_STATE_UNAVAILABLE",
    RunPresentationError::Store(DurableStoreError::UnsupportedStoreVersion(_)) => {
      "WOML_RUN_PRESENTATION_VERSION_UNSUPPORTED"
    }
    RunPresentationError::Store(
      DurableStoreError::InvalidModel(_) | DurableStoreError::Json(_),
    ) => "WOML_RUN_PRESENTATION_COMPATIBILITY_FAILED",
    RunPresentationError::Store(DurableStoreError::InvalidStoredEvent(_))
    | RunPresentationError::Fold(_)
    | RunPresentationError::DefinitionMismatch => "WOML_RUN_PRESENTATION_HISTORY_INVALID",
    RunPresentationError::TooLarge | RunPresentationError::TooMany(_) => {
      "WOML_RUN_PRESENTATION_SIZE_LIMIT"
    }
    RunPresentationError::InvalidRecentLimit => "WOML_RUN_PRESENTATION_LIMIT_INVALID",
    _ => "WOML_RUN_PRESENTATION_FAILED",
  };
  native_run_management_error(code, DurableStoreError::Contract(error.to_string()))
}

fn native_run_inspection_error(error: DurableStoreError) -> napi::Error {
  let code = if matches!(error, DurableStoreError::RunNotFound(_)) {
    "WOML_RUN_NOT_FOUND"
  } else {
    "WOML_RUN_INSPECTION_FAILED"
  };
  let reason = serde_json::to_string(&NativeRunInspectionError {
    kind: "woml_run_inspection_error",
    code,
    message: error.to_string(),
  })
  .unwrap_or_else(|_| "WOML run inspection failed.".to_string());
  napi::Error::from_reason(reason)
}

fn run_failure_code(failure: &RunFailure) -> String {
  match failure {
    RunFailure::Attempt(failure) => failure.code.clone(),
    RunFailure::Branch(failure) => failure.code().to_string(),
    RunFailure::Parallel { failure, .. } => failure.code.clone(),
    RunFailure::Approval { failure, .. } => failure.code.clone(),
    RunFailure::Notification { failure, .. } => failure.code.clone(),
  }
}

fn runtime_options(
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
) -> RuntimeExecutionOptions {
  RuntimeExecutionOptions::new(
    ScriptHostProcessOptions::new(
      PathBuf::from(bun_executable),
      PathBuf::from(script_host_path),
    ),
    u64::from(script_timeout_ms),
  )
}

fn runtime_options_with_secrets(
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  resolved_secrets_json: String,
) -> napi::Result<RuntimeExecutionOptions> {
  let secrets: BTreeMap<String, String> = serde_json::from_str(&resolved_secrets_json)
    .map_err(|error| napi::Error::from_reason(format!("Invalid resolved secrets JSON: {error}")))?;
  Ok(
    runtime_options(bun_executable, script_host_path, script_timeout_ms)
      .with_resolved_secrets(secrets),
  )
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeRuntimeCapabilities {
  #[serde(default)]
  process: Option<NativeProcessRuntime>,
  #[serde(default)]
  artifacts: Option<NativeArtifactRuntime>,
  #[serde(default)]
  reviewers: Option<NativeReviewerRuntime>,
  #[serde(default)]
  infrastructure_policy: Option<NativeInfrastructurePolicyRuntime>,
  #[serde(default)]
  emergency_deny: Option<NativeInfrastructurePolicyRuntime>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeInfrastructurePolicyTrust {
  #[serde(default)]
  keys: BTreeMap<String, String>,
  #[serde(default)]
  revoked_key_ids: BTreeSet<String>,
  #[serde(default)]
  require_signature: bool,
}

impl From<NativeInfrastructurePolicyTrust> for InfrastructurePolicyTrustV1 {
  fn from(value: NativeInfrastructurePolicyTrust) -> Self {
    Self {
      keys: value.keys,
      revoked_key_ids: value.revoked_key_ids,
      require_signature: value.require_signature,
    }
  }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeInfrastructurePolicyRuntime {
  policy: InfrastructurePolicyBundleV1,
  #[serde(default)]
  trust: NativeInfrastructurePolicyTrust,
  #[serde(default)]
  required: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeProcessRuntime {
  mode: ProcessRuntimeMode,
  policy: ProcessPolicyV1,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeArtifactRuntime {
  mode: ArtifactRuntimeMode,
  policy: ArtifactPolicyV1,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeReviewerRuntime {
  directory: ReviewerDirectoryV1,
}

fn apply_runtime_capabilities(
  options: RuntimeExecutionOptions,
  runtime_capabilities_json: Option<String>,
) -> napi::Result<RuntimeExecutionOptions> {
  let Some(source) = runtime_capabilities_json else {
    return Ok(options);
  };
  let capabilities: NativeRuntimeCapabilities = serde_json::from_str(&source).map_err(|error| {
    napi::Error::from_reason(format!("Invalid runtime capabilities JSON: {error}"))
  })?;
  let options = match capabilities.process {
    Some(process) => options
      .with_process_policy(process.policy, process.mode)
      .map_err(|error| napi::Error::from_reason(error.to_string())),
    None => Ok(options),
  }?;
  let options = match capabilities.artifacts {
    Some(artifacts) => options
      .with_artifact_policy(artifacts.policy, artifacts.mode)
      .map_err(|error| napi::Error::from_reason(error.to_string())),
    None => Ok(options),
  }?;
  let options = match capabilities.reviewers {
    Some(reviewers) => ReviewerDirectoryAuthorityV1::from_document(reviewers.directory)
      .map(|authority| options.with_reviewer_directory(Arc::new(authority)))
      .map_err(|error| napi::Error::from_reason(format!("{}: {error}", error.code()))),
    None => Ok(options),
  }?;
  let mut base_policy_environment = None;
  let options = if let Some(policy) = capabilities.infrastructure_policy {
    let required = policy.required;
    let prepared = prepare_infrastructure_policy(policy.policy, &policy.trust.into())
      .map_err(|error| napi::Error::from_reason(format!("{}: {error}", error.code())))?;
    base_policy_environment = Some(prepared.bundle.environment.clone());
    let options = options.with_infrastructure_policy(prepared);
    if required {
      options.require_infrastructure_policy()
    } else {
      options
    }
  } else {
    options
  };
  match capabilities.emergency_deny {
    Some(policy) => {
      let prepared = prepare_emergency_deny_overlay(policy.policy, &policy.trust.into())
        .map_err(|error| napi::Error::from_reason(format!("{}: {error}", error.code())))?;
      if base_policy_environment.as_deref() != Some(prepared.bundle.environment.as_str()) {
        return Err(napi::Error::from_reason(
          "WOML_POLICY_INVALID: an emergency deny overlay requires a base policy in the same environment."
            .to_string(),
        ));
      }
      Ok(options.with_emergency_deny_overlay(prepared))
    }
    None => Ok(options),
  }
}

fn runtime_options_with_modules(
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  resolved_secrets_json: String,
  runtime_modules_json: Option<String>,
  runtime_capabilities_json: Option<String>,
) -> napi::Result<RuntimeExecutionOptions> {
  let modules: Vec<RuntimeModuleArtifact> =
    serde_json::from_str(runtime_modules_json.as_deref().unwrap_or("[]")).map_err(|error| {
      napi::Error::from_reason(format!("Invalid runtime modules JSON: {error}"))
    })?;
  apply_runtime_capabilities(
    runtime_options_with_secrets(
      bun_executable,
      script_host_path,
      script_timeout_ms,
      resolved_secrets_json,
    )?
    .with_runtime_modules(modules),
    runtime_capabilities_json,
  )
}

fn runtime_options_with_progress(
  env: &Env,
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  progress_callback: JsFunction,
  resolved_secrets_json: String,
  runtime_capabilities_json: Option<String>,
) -> napi::Result<RuntimeExecutionOptions> {
  let operational_output_reporter = native_operational_output_reporter(env, &progress_callback)?;
  let mut progress = progress_callback
    .create_threadsafe_function::<String, String, _, ErrorStrategy::Fatal>(0, |context| {
      Ok(vec![context.value])
    })?;
  progress.unref(env)?;
  let lifecycle_progress = progress.clone();
  let runtime_policy_progress = lifecycle_progress.clone();
  apply_runtime_capabilities(
    runtime_options_with_secrets(
      bun_executable,
      script_host_path,
      script_timeout_ms,
      resolved_secrets_json,
    )?
    .with_progress_reporter(Arc::new(move |message| {
      if let Ok(json) = serde_json::to_string(&message) {
        let _ = progress.call(json, ThreadsafeFunctionCallMode::Blocking);
      }
    }))
    .with_lifecycle_progress_reporter(Arc::new(move |message: LifecycleProgress| {
      if let Ok(json) = serde_json::to_string(&message) {
        let _ = lifecycle_progress.call(json, ThreadsafeFunctionCallMode::Blocking);
      }
    }))
    .with_runtime_policy_progress_reporter(Arc::new(move |message: RuntimePolicyProgress| {
      if let Ok(json) = serde_json::to_string(&message) {
        let _ = runtime_policy_progress.call(json, ThreadsafeFunctionCallMode::Blocking);
      }
    }))
    .with_operational_output_reporter(operational_output_reporter),
    runtime_capabilities_json,
  )
}

#[napi(ts_return_type = "Promise<string>")]
pub async fn execute_woml_workflow(
  compiled_model_json: String,
  definition_hash: String,
  trigger_json: String,
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  resolved_secrets_json: String,
  runtime_modules_json: Option<String>,
  runtime_capabilities_json: Option<String>,
) -> napi::Result<String> {
  let workflow: CompiledWorkflowDefinition =
    serde_json::from_str(&compiled_model_json).map_err(|error| {
      napi::Error::from_reason(format!("Invalid compiled workflow JSON: {error}"))
    })?;
  let trigger: Map<String, Value> = serde_json::from_str(&trigger_json)
    .map_err(|error| napi::Error::from_reason(format!("Invalid trigger JSON: {error}")))?;
  let options = runtime_options_with_modules(
    bun_executable,
    script_host_path,
    script_timeout_ms,
    resolved_secrets_json,
    runtime_modules_json,
    runtime_capabilities_json,
  )?;
  let result = execute_workflow(workflow, definition_hash, trigger, options)
    .await
    .map_err(native_execution_error)?;
  serde_json::to_string(&result)
    .map_err(|error| napi::Error::from_reason(format!("Could not encode WOML result: {error}")))
}

#[napi(ts_return_type = "Promise<string>")]
pub async fn execute_woml_workflow_durable(
  compiled_model_json: String,
  definition_hash: String,
  trigger_json: String,
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  event_store_path: String,
  resolved_secrets_json: String,
  runtime_modules_json: Option<String>,
  runtime_capabilities_json: Option<String>,
) -> napi::Result<String> {
  let mut decode_span = woml_engine::performance::PerformanceSpan::new_for_process(
    "native",
    "napi",
    "napi.decode_durable_request",
  );
  decode_span.bytes("model", compiled_model_json.len());
  decode_span.bytes("trigger", trigger_json.len());
  decode_span.bytes(
    "modules",
    runtime_modules_json.as_deref().unwrap_or("[]").len(),
  );
  let workflow: CompiledWorkflowDefinition =
    serde_json::from_str(&compiled_model_json).map_err(|error| {
      napi::Error::from_reason(format!("Invalid compiled workflow JSON: {error}"))
    })?;
  let trigger: Map<String, Value> = serde_json::from_str(&trigger_json)
    .map_err(|error| napi::Error::from_reason(format!("Invalid trigger JSON: {error}")))?;
  let options = runtime_options_with_modules(
    bun_executable,
    script_host_path,
    script_timeout_ms,
    resolved_secrets_json,
    runtime_modules_json,
    runtime_capabilities_json,
  )?;
  decode_span.count("nodes", workflow.graph.nodes.len());
  decode_span.count("modules", options.runtime_modules.len());
  decode_span.succeed();
  drop(decode_span);
  let mut execute_span = woml_engine::performance::PerformanceSpan::new_for_process(
    "native",
    "napi",
    "napi.execute_durable",
  );
  let result = execute_workflow_durable(
    workflow,
    definition_hash,
    trigger,
    options,
    PathBuf::from(event_store_path),
  )
  .await
  .map_err(native_execution_error)?;
  execute_span.run_id(result.run_id.clone());
  execute_span.succeed();
  drop(execute_span);
  let mut encode_span = woml_engine::performance::PerformanceSpan::new_for_process(
    "native",
    "napi",
    "napi.encode_durable_result",
  );
  encode_span.run_id(result.run_id.clone());
  let encoded = serde_json::to_string(&result)
    .map_err(|error| napi::Error::from_reason(format!("Could not encode WOML result: {error}")))?;
  encode_span.bytes("result", encoded.len());
  encode_span.succeed();
  Ok(encoded)
}

#[napi(ts_return_type = "Promise<string>")]
pub fn execute_woml_workflow_durable_with_progress(
  env: Env,
  compiled_model_json: String,
  definition_hash: String,
  trigger_json: String,
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  event_store_path: String,
  progress_callback: JsFunction,
  resolved_secrets_json: String,
  runtime_modules_json: Option<String>,
  runtime_capabilities_json: Option<String>,
) -> napi::Result<JsObject> {
  let mut decode_span = woml_engine::performance::PerformanceSpan::new_for_process(
    "native",
    "napi",
    "napi.decode_durable_request",
  );
  decode_span.bytes("model", compiled_model_json.len());
  decode_span.bytes("trigger", trigger_json.len());
  decode_span.bytes(
    "modules",
    runtime_modules_json.as_deref().unwrap_or("[]").len(),
  );
  let workflow: CompiledWorkflowDefinition =
    serde_json::from_str(&compiled_model_json).map_err(|error| {
      napi::Error::from_reason(format!("Invalid compiled workflow JSON: {error}"))
    })?;
  let trigger: Map<String, Value> = serde_json::from_str(&trigger_json)
    .map_err(|error| napi::Error::from_reason(format!("Invalid trigger JSON: {error}")))?;
  let options = runtime_options_with_progress(
    &env,
    bun_executable,
    script_host_path,
    script_timeout_ms,
    progress_callback,
    resolved_secrets_json,
    runtime_capabilities_json,
  )?;
  let modules: Vec<RuntimeModuleArtifact> =
    serde_json::from_str(runtime_modules_json.as_deref().unwrap_or("[]")).map_err(|error| {
      napi::Error::from_reason(format!("Invalid runtime modules JSON: {error}"))
    })?;
  let options = options.with_runtime_modules(modules);
  decode_span.count("nodes", workflow.graph.nodes.len());
  decode_span.count("modules", options.runtime_modules.len());
  decode_span.succeed();
  drop(decode_span);
  env.spawn_future(async move {
    let mut execute_span = woml_engine::performance::PerformanceSpan::new_for_process(
      "native",
      "napi",
      "napi.execute_durable",
    );
    let result = execute_workflow_durable(
      workflow,
      definition_hash,
      trigger,
      options,
      PathBuf::from(event_store_path),
    )
    .await
    .map_err(native_execution_error)?;
    execute_span.run_id(result.run_id.clone());
    execute_span.succeed();
    drop(execute_span);
    let mut encode_span = woml_engine::performance::PerformanceSpan::new_for_process(
      "native",
      "napi",
      "napi.encode_durable_result",
    );
    encode_span.run_id(result.run_id.clone());
    let encoded = serde_json::to_string(&result).map_err(|error| {
      napi::Error::from_reason(format!("Could not encode WOML result: {error}"))
    })?;
    encode_span.bytes("result", encoded.len());
    encode_span.succeed();
    Ok(encoded)
  })
}

fn verify_durable_run_definition(
  event_store_path: &str,
  run_id: &str,
  definition_hash: &str,
  workflow: &CompiledWorkflowDefinition,
) -> napi::Result<()> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_execution_error(RuntimeExecutionError::DurableStore(error)))?;
  let binding = store
    .run_binding(run_id)
    .map_err(|error| native_execution_error(RuntimeExecutionError::DurableStore(error)))?;
  let stored_workflow = store
    .definition(&binding.definition_hash)
    .map_err(|error| native_execution_error(RuntimeExecutionError::DurableStore(error)))?;
  if binding.definition_hash != definition_hash || stored_workflow != *workflow {
    return Err(native_execution_error(
      RuntimeExecutionError::InvalidConfiguration(
        "the supplied WOML definition does not match the durable run definition".to_string(),
      ),
    ));
  }
  Ok(())
}

#[napi(ts_return_type = "Promise<string>")]
pub fn resume_woml_workflow_durable_with_progress(
  env: Env,
  compiled_model_json: String,
  definition_hash: String,
  run_id: String,
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  event_store_path: String,
  progress_callback: JsFunction,
  resolved_secrets_json: String,
  runtime_modules_json: Option<String>,
  runtime_capabilities_json: Option<String>,
) -> napi::Result<JsObject> {
  let workflow: CompiledWorkflowDefinition =
    serde_json::from_str(&compiled_model_json).map_err(|error| {
      napi::Error::from_reason(format!("Invalid compiled workflow JSON: {error}"))
    })?;
  verify_durable_run_definition(&event_store_path, &run_id, &definition_hash, &workflow)?;
  let options = runtime_options_with_progress(
    &env,
    bun_executable,
    script_host_path,
    script_timeout_ms,
    progress_callback,
    resolved_secrets_json,
    runtime_capabilities_json,
  )?;
  let modules: Vec<RuntimeModuleArtifact> =
    serde_json::from_str(runtime_modules_json.as_deref().unwrap_or("[]")).map_err(|error| {
      napi::Error::from_reason(format!("Invalid runtime modules JSON: {error}"))
    })?;
  let options = options.with_runtime_modules(modules);
  env.spawn_future(async move {
    let result = resume_workflow_durable(PathBuf::from(event_store_path), &run_id, options)
      .await
      .map_err(native_execution_error)?;
    serde_json::to_string(&result)
      .map_err(|error| napi::Error::from_reason(format!("Could not encode WOML result: {error}")))
  })
}

#[napi(ts_return_type = "Promise<string>")]
pub async fn execute_woml_workflow_durable_outcome(
  compiled_model_json: String,
  definition_hash: String,
  trigger_json: String,
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  event_store_path: String,
  resolved_secrets_json: String,
  runtime_modules_json: Option<String>,
  runtime_capabilities_json: Option<String>,
) -> napi::Result<String> {
  let workflow: CompiledWorkflowDefinition =
    serde_json::from_str(&compiled_model_json).map_err(|error| {
      napi::Error::from_reason(format!("Invalid compiled workflow JSON: {error}"))
    })?;
  let trigger: Map<String, Value> = serde_json::from_str(&trigger_json)
    .map_err(|error| napi::Error::from_reason(format!("Invalid trigger JSON: {error}")))?;
  let outcome = execute_workflow_durable_outcome(
    workflow,
    definition_hash,
    trigger,
    runtime_options_with_modules(
      bun_executable,
      script_host_path,
      script_timeout_ms,
      resolved_secrets_json,
      runtime_modules_json,
      runtime_capabilities_json,
    )?,
    PathBuf::from(event_store_path),
  )
  .await
  .map_err(native_execution_error)?;
  serde_json::to_string(&outcome).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode WOML runtime outcome: {error}"))
  })
}

#[napi(ts_return_type = "Promise<string>")]
pub fn execute_woml_workflow_durable_outcome_with_progress(
  env: Env,
  compiled_model_json: String,
  definition_hash: String,
  trigger_json: String,
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  event_store_path: String,
  progress_callback: JsFunction,
  resolved_secrets_json: String,
  runtime_modules_json: Option<String>,
  runtime_capabilities_json: Option<String>,
) -> napi::Result<JsObject> {
  let workflow: CompiledWorkflowDefinition =
    serde_json::from_str(&compiled_model_json).map_err(|error| {
      napi::Error::from_reason(format!("Invalid compiled workflow JSON: {error}"))
    })?;
  let trigger: Map<String, Value> = serde_json::from_str(&trigger_json)
    .map_err(|error| napi::Error::from_reason(format!("Invalid trigger JSON: {error}")))?;
  let options = runtime_options_with_progress(
    &env,
    bun_executable,
    script_host_path,
    script_timeout_ms,
    progress_callback,
    resolved_secrets_json,
    runtime_capabilities_json,
  )?;
  let modules: Vec<RuntimeModuleArtifact> =
    serde_json::from_str(runtime_modules_json.as_deref().unwrap_or("[]")).map_err(|error| {
      napi::Error::from_reason(format!("Invalid runtime modules JSON: {error}"))
    })?;
  let options = options.with_runtime_modules(modules);
  env.spawn_future(async move {
    let outcome = execute_workflow_durable_outcome(
      workflow,
      definition_hash,
      trigger,
      options,
      PathBuf::from(event_store_path),
    )
    .await
    .map_err(native_execution_error)?;
    serde_json::to_string(&outcome).map_err(|error| {
      napi::Error::from_reason(format!("Could not encode WOML runtime outcome: {error}"))
    })
  })
}

#[napi(ts_return_type = "Promise<string>")]
pub async fn resume_woml_workflow_durable_outcome(
  compiled_model_json: String,
  definition_hash: String,
  run_id: String,
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  event_store_path: String,
  resolved_secrets_json: String,
  runtime_modules_json: Option<String>,
  runtime_capabilities_json: Option<String>,
) -> napi::Result<String> {
  let workflow: CompiledWorkflowDefinition =
    serde_json::from_str(&compiled_model_json).map_err(|error| {
      napi::Error::from_reason(format!("Invalid compiled workflow JSON: {error}"))
    })?;
  let store = DurableEventStore::open(PathBuf::from(&event_store_path))
    .map_err(|error| native_execution_error(RuntimeExecutionError::DurableStore(error)))?;
  let binding = store
    .run_binding(&run_id)
    .map_err(|error| native_execution_error(RuntimeExecutionError::DurableStore(error)))?;
  let stored_workflow = store
    .definition(&binding.definition_hash)
    .map_err(|error| native_execution_error(RuntimeExecutionError::DurableStore(error)))?;
  if binding.definition_hash != definition_hash || stored_workflow != workflow {
    return Err(native_execution_error(
      RuntimeExecutionError::InvalidConfiguration(
        "the supplied WOML definition does not match the durable run definition".to_string(),
      ),
    ));
  }
  let outcome = resume_workflow_durable_outcome(
    PathBuf::from(event_store_path),
    &run_id,
    runtime_options_with_modules(
      bun_executable,
      script_host_path,
      script_timeout_ms,
      resolved_secrets_json,
      runtime_modules_json,
      runtime_capabilities_json,
    )?,
  )
  .await
  .map_err(native_execution_error)?;
  serde_json::to_string(&outcome).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode WOML runtime outcome: {error}"))
  })
}

#[napi(ts_return_type = "Promise<string>")]
pub fn resume_woml_workflow_durable_outcome_with_progress(
  env: Env,
  compiled_model_json: String,
  definition_hash: String,
  run_id: String,
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  event_store_path: String,
  progress_callback: JsFunction,
  resolved_secrets_json: String,
  runtime_modules_json: Option<String>,
  runtime_capabilities_json: Option<String>,
) -> napi::Result<JsObject> {
  let workflow: CompiledWorkflowDefinition =
    serde_json::from_str(&compiled_model_json).map_err(|error| {
      napi::Error::from_reason(format!("Invalid compiled workflow JSON: {error}"))
    })?;
  verify_durable_run_definition(&event_store_path, &run_id, &definition_hash, &workflow)?;
  let options = runtime_options_with_progress(
    &env,
    bun_executable,
    script_host_path,
    script_timeout_ms,
    progress_callback,
    resolved_secrets_json,
    runtime_capabilities_json,
  )?;
  let modules: Vec<RuntimeModuleArtifact> =
    serde_json::from_str(runtime_modules_json.as_deref().unwrap_or("[]")).map_err(|error| {
      napi::Error::from_reason(format!("Invalid runtime modules JSON: {error}"))
    })?;
  let options = options.with_runtime_modules(modules);
  env.spawn_future(async move {
    let outcome =
      resume_workflow_durable_outcome(PathBuf::from(event_store_path), &run_id, options)
        .await
        .map_err(native_execution_error)?;
    serde_json::to_string(&outcome).map_err(|error| {
      napi::Error::from_reason(format!("Could not encode WOML runtime outcome: {error}"))
    })
  })
}

#[napi]
pub fn inspect_woml_stored_run_requirements(
  event_store_path: String,
  run_id: String,
) -> napi::Result<String> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_execution_error(RuntimeExecutionError::DurableStore(error)))?;
  let binding = store
    .run_binding(&run_id)
    .map_err(|error| native_execution_error(RuntimeExecutionError::DurableStore(error)))?;
  let workflow = store
    .definition(&binding.definition_hash)
    .map_err(|error| native_execution_error(RuntimeExecutionError::DurableStore(error)))?;
  let mut required_secrets = workflow
    .graph
    .nodes
    .iter()
    .flat_map(|node| {
      node
        .script_runtime
        .iter()
        .flat_map(|runtime| runtime.required_secrets.iter().cloned())
    })
    .collect::<Vec<_>>();
  required_secrets.extend(
    workflow
      .lifecycle
      .iter()
      .flat_map(|lifecycle| &lifecycle.hooks)
      .flat_map(|hook| &hook.actions)
      .flat_map(|action| action.script_runtime.iter())
      .flat_map(|runtime| runtime.required_secrets.iter().cloned()),
  );
  required_secrets.extend(
    workflow
      .reusable_definitions
      .iter()
      .flatten()
      .filter_map(|definition| match definition {
        CompiledReusableInvocation::NotificationProvider { props, .. } => Some(props),
        _ => None,
      })
      .flatten()
      .filter_map(|prop| match &prop.expression {
        woml_engine::model::CompiledReusablePropExpression::Secret { name } => Some(name.clone()),
        _ => None,
      }),
  );
  required_secrets.sort();
  required_secrets.dedup();
  let has_approval = workflow
    .graph
    .nodes
    .iter()
    .any(|node| node.handler == "engine.approval-wait");
  let has_notifications = workflow.graph.nodes.iter().any(|node| {
    node.handler == "engine.approval-wait"
      && matches!(
        &node.inputs,
        ValueExpression::Object { fields } if fields.contains_key("notifications")
      )
  });
  let requirements = NativeStoredRunRequirements {
    contract: "woml.stored-run-requirements",
    version: 1,
    workflow_id: binding.workflow_id,
    definition_hash: binding.definition_hash,
    required_secrets,
    module_count: workflow
      .module_runtime
      .as_ref()
      .map_or(0, |runtime| runtime.modules.len()),
    has_approval,
    has_notifications,
  };
  serde_json::to_string(&requirements).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode stored run requirements: {error}"))
  })
}

#[napi(ts_return_type = "Promise<string>")]
pub fn resume_woml_stored_run_with_progress(
  env: Env,
  run_id: String,
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  event_store_path: String,
  progress_callback: JsFunction,
  resolved_secrets_json: String,
  runtime_capabilities_json: Option<String>,
) -> napi::Result<JsObject> {
  let options = runtime_options_with_progress(
    &env,
    bun_executable,
    script_host_path,
    script_timeout_ms,
    progress_callback,
    resolved_secrets_json,
    runtime_capabilities_json,
  )?;
  env.spawn_future(async move {
    let outcome =
      resume_workflow_durable_any_outcome(PathBuf::from(event_store_path), &run_id, options)
        .await
        .map_err(native_execution_error)?;
    serde_json::to_string(&outcome).map_err(|error| {
      napi::Error::from_reason(format!("Could not encode WOML runtime outcome: {error}"))
    })
  })
}

#[napi]
pub fn resolve_woml_approval(
  event_store_path: String,
  token: String,
  decision: String,
) -> napi::Result<String> {
  let decision = match decision.as_str() {
    "approved" => ApprovalDecision::Approved,
    "rejected" => ApprovalDecision::Rejected,
    _ => {
      return Err(napi::Error::from_reason(
        "Approval decision must be approved or rejected.".to_string(),
      ))
    }
  };
  let outcome = resolve_human_approval_durable(
    PathBuf::from(event_store_path),
    &token,
    decision,
    &SystemEngineClock,
  )
  .map_err(native_approval_error)?;
  serde_json::to_string(&NativeApprovalDecisionOutcome {
    contract: "woml.approval-http",
    version: 1,
    outcome,
  })
  .map_err(|error| napi::Error::from_reason(format!("Could not encode approval decision: {error}")))
}

#[napi]
pub fn resolve_woml_notification_approval(
  event_store_path: String,
  capability: String,
  decision: String,
  provider_actor_id: Option<String>,
) -> napi::Result<String> {
  let decision = match decision.as_str() {
    "approved" => ApprovalDecision::Approved,
    "rejected" => ApprovalDecision::Rejected,
    _ => {
      return Err(napi::Error::from_reason(
        "Approval decision must be approved or rejected.".to_string(),
      ))
    }
  };
  let mut store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_approval_error(RuntimeExecutionError::DurableStore(error)))?;
  let provider_actor_id = provider_actor_id
    .filter(|value| !value.trim().is_empty())
    .unwrap_or_else(|| "custom-provider".to_string());
  let outcome = store
    .resolve_notification_approval(&capability, &provider_actor_id, decision, Utc::now())
    .map_err(|error| native_approval_error(RuntimeExecutionError::DurableStore(error)))?;
  serde_json::to_string(&NativeApprovalDecisionOutcome {
    contract: "woml.approval-http",
    version: 1,
    outcome,
  })
  .map_err(|error| napi::Error::from_reason(format!("Could not encode approval decision: {error}")))
}

/// Internal ingress used only after the provider host or trusted HTTP gateway
/// has authenticated the provider account and actor. The active Reviewer
/// Directory is runtime-owned and cannot be supplied by this decision call.
#[napi]
#[allow(clippy::too_many_arguments)]
pub fn resolve_woml_rich_approval(
  event_store_path: String,
  run_id: String,
  approval_id: String,
  request_id: String,
  review_digest: String,
  decision: String,
  provider: String,
  provider_account_id: String,
  provider_actor_id: String,
) -> napi::Result<String> {
  let decision = match decision.as_str() {
    "approved" => ApprovalDecision::Approved,
    "rejected" => ApprovalDecision::Rejected,
    _ => {
      return Err(napi::Error::from_reason(
        "Approval decision must be approved or rejected.".to_string(),
      ))
    }
  };
  let provider = match provider.as_str() {
    "slack" => ReviewerProviderV1::Slack,
    "telegram" => ReviewerProviderV1::Telegram,
    "discord" => ReviewerProviderV1::Discord,
    "trusted-http" => ReviewerProviderV1::TrustedHttp,
    _ => {
      return Err(napi::Error::from_reason(
        "Rich approval provider identity is unsupported.".to_string(),
      ))
    }
  };
  let outcome = resolve_rich_approval_durable(
    PathBuf::from(event_store_path),
    RichApprovalIdentityDecision {
      run_id: run_id.clone(),
      request_id: request_id.clone(),
      review_digest: review_digest.clone(),
      decision,
      identity: ReviewerIdentityClaimV1 {
        provider,
        account_id: provider_account_id,
        actor_id: provider_actor_id,
      },
      decided_at: Utc::now(),
    },
  )
  .map_err(native_approval_error)?;
  let status = match outcome.status {
    RichApprovalDecisionStatus::Pending => "recorded",
    RichApprovalDecisionStatus::Approved | RichApprovalDecisionStatus::Rejected => "resolved",
    RichApprovalDecisionStatus::Idempotent => "already_recorded",
    RichApprovalDecisionStatus::AlreadyResolved => "already_resolved",
  };
  serde_json::to_string(&NativeRichApprovalDecisionOutcome {
    contract: "woml.approval-http",
    version: 2,
    status,
    run_id,
    approval_id,
    request_id,
    review_digest,
  })
  .map_err(|error| {
    napi::Error::from_reason(format!("Could not encode rich approval decision: {error}"))
  })
}

#[napi]
pub fn settle_woml_approval_timeout(
  event_store_path: String,
  run_id: String,
  approval_id: String,
) -> napi::Result<String> {
  let outcome = settle_approval_timeout_durable(
    PathBuf::from(event_store_path),
    &run_id,
    &approval_id,
    &SystemEngineClock,
  )
  .map_err(native_approval_error)?;
  serde_json::to_string(&outcome).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode approval timeout: {error}"))
  })
}

#[napi(ts_return_type = "Promise<string>")]
pub fn start_woml_webhook_runtime(
  env: Env,
  registrations_json: String,
  startup_manual_triggers_json: String,
  bind_address: String,
  event_store_path: String,
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  shutdown_timeout_ms: u32,
  deployment_id: String,
  activation_id: String,
  start_suspended: bool,
  runtime_capabilities_json: Option<String>,
  signal_ingress_token: Option<String>,
  telemetry_enabled: bool,
  telemetry_propagation: bool,
  telemetry_sampling_ratio: f64,
  progress_callback: JsFunction,
) -> napi::Result<JsObject> {
  let mut decode_span = woml_engine::performance::PerformanceSpan::new_for_process(
    "native",
    "napi",
    "napi.decode_runtime_registration",
  );
  decode_span.bytes("registrations", registrations_json.len());
  decode_span.bytes("manual_triggers", startup_manual_triggers_json.len());
  let registrations: Vec<NativeWebhookRegistration> = serde_json::from_str(&registrations_json)
    .map_err(|error| {
      napi::Error::from_reason(format!("Invalid webhook registration JSON: {error}"))
    })?;
  let startup_manual_triggers: BTreeMap<String, String> =
    serde_json::from_str(&startup_manual_triggers_json).map_err(|error| {
      napi::Error::from_reason(format!("Invalid startup manual trigger JSON: {error}"))
    })?;
  let bind_address: SocketAddr = bind_address
    .parse()
    .map_err(|error| napi::Error::from_reason(format!("Invalid webhook bind address: {error}")))?;
  let runtime_id = format!("runtime_{}", uuid::Uuid::new_v4().simple());
  let (
    progress_reporter,
    schedule_progress_reporter,
    interval_progress_reporter,
    workflow_call_progress_reporter,
    runtime_policy_progress_reporter,
    runtime_instance_reporter,
    operational_output_reporter,
    telemetry_reporter,
    telemetry_admission_link_reporter,
  ) = native_runtime_progress_reporters(&env, progress_callback, telemetry_enabled)?;
  let mut runtime_secrets = BTreeMap::new();
  for registration in &registrations {
    for (name, value) in &registration.resolved_secrets {
      if runtime_secrets
        .insert(name.clone(), value.clone())
        .is_some_and(|existing| existing != *value)
      {
        return Err(napi::Error::from_reason(format!(
          "Resolved secret {name:?} has conflicting values across workflow registrations."
        )));
      }
    }
  }
  let manual_targets = registrations
    .iter()
    .flat_map(|registration| {
      registration
        .workflow
        .triggers
        .iter()
        .filter(|trigger| trigger.handler == "trigger.manual")
        .map(|trigger| {
          (
            (
              registration.workflow.workflow_id.clone(),
              trigger.id.clone(),
            ),
            NativeManualTarget {
              definition_hash: registration.definition_hash.clone(),
              admission_status: if registration.workflow.schema_version
                >= woml_engine::COMPILED_MODEL_SCHEMA_VERSION_V12
              {
                "queued"
              } else {
                "running"
              },
            },
          )
        })
    })
    .collect::<BTreeMap<_, _>>();
  decode_span.count("workflows", registrations.len());
  let registrations = registrations
    .into_iter()
    .map(|registration| WebhookDefinitionRegistration {
      workflow: registration.workflow,
      definition_hash: registration.definition_hash,
      resolved_secrets: registration.resolved_secrets,
      runtime_modules: registration.runtime_modules,
    })
    .collect();
  let execution = runtime_options(bun_executable, script_host_path, script_timeout_ms)
    .with_resolved_secrets(runtime_secrets)
    .with_schedule_progress_reporter(schedule_progress_reporter)
    .with_interval_progress_reporter(interval_progress_reporter)
    .with_workflow_call_progress_reporter(workflow_call_progress_reporter)
    .with_runtime_policy_progress_reporter(runtime_policy_progress_reporter)
    .with_operational_output_reporter(operational_output_reporter);
  let execution = if let Some(telemetry_reporter) = telemetry_reporter {
    execution.with_telemetry_reporter(
      woml_engine::TelemetrySignalContext {
        runtime_instance_id: runtime_id.clone(),
        deployment_id: deployment_id.clone(),
      },
      telemetry_reporter,
    )
  } else {
    execution
  };
  let execution = execution.with_telemetry_propagation(
    telemetry_enabled && telemetry_propagation,
    telemetry_sampling_ratio,
    telemetry_admission_link_reporter,
  );
  let execution = apply_runtime_capabilities(execution, runtime_capabilities_json)?
    .with_signal_ingress_token(signal_ingress_token)
    .with_shared_script_hosts();
  let signal_execution = execution.clone();
  let config = WomlWebhookServerConfig {
    bind_address,
    database_path: PathBuf::from(event_store_path),
    registrations,
    startup_manual_triggers,
    execution,
    progress_reporter: Some(progress_reporter),
  };
  decode_span.succeed();
  drop(decode_span);

  env.spawn_future(async move {
    let mut start_span = woml_engine::performance::PerformanceSpan::new_for_process(
      "native",
      "napi",
      "napi.start_trigger_runtime",
    );
    let started_at = Utc::now();
    let shutdown_deadline = std::time::Duration::from_millis(u64::from(shutdown_timeout_ms));
    let (startup_sender, startup_receiver) =
      mpsc::sync_channel::<Result<SocketAddr, NativeTriggerRuntimeError>>(1);
    let (control_sender, mut control_receiver) = mpsc::channel::<NativeWebhookRuntimeCommand>();
    let ownership_control = control_sender.clone();
    let (ingress_sender, ingress_receiver) = tokio::sync::mpsc::unbounded_channel();
    let thread_runtime_id = runtime_id.clone();
    let join = std::thread::Builder::new()
      .name(format!("woml-trigger-{runtime_id}"))
      .spawn(move || {
        actix_web::rt::System::new().block_on(async move {
          let ownership = (|| {
            let now = Utc::now();
            let mut store = DurableEventStore::open(&config.database_path)?;
            store.audit_integrity()?;
            store.acquire_runtime_owner(
              &deployment_id,
              &activation_id,
              &thread_runtime_id,
              now,
              now + chrono::Duration::seconds(10),
            )
          })();
          if let Err(error) = ownership {
            let _ = startup_sender.send(Err(native_trigger_runtime_error(
              WebhookRuntimeError::DurableStore(error),
            )));
            return;
          }
          let runtime_database_path = config.database_path.clone();
          match WomlWebhookServer::prepare_with_external_ingress(config, Some(ingress_receiver))
            .await
          {
            Ok(mut server) => {
              if !start_suspended {
                if let Err(error) = server.activate().await {
                  let _ = startup_sender.send(Err(native_trigger_runtime_error(error)));
                  server.stop_with_deadline(shutdown_deadline).await;
                  let _ = DurableEventStore::open(&runtime_database_path).and_then(|mut store| {
                    store.release_runtime_owner(&thread_runtime_id).map(|_| ())
                  });
                  return;
                }
              }
              let address = server.local_address();
              if startup_sender.send(Ok(address)).is_err() {
                server.stop_with_deadline(shutdown_deadline).await;
                return;
              }
              let heartbeat_path = runtime_database_path.clone();
              let heartbeat_runtime_id = thread_runtime_id.clone();
              let heartbeat_control = ownership_control.clone();
              let heartbeat = actix_web::rt::spawn(async move {
                loop {
                  tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                  let now = Utc::now();
                  let renewed = DurableEventStore::open(&heartbeat_path).and_then(|mut store| {
                    store.renew_runtime_owner(
                      &heartbeat_runtime_id,
                      now,
                      now + chrono::Duration::seconds(10),
                    )
                  });
                  if !renewed.is_ok_and(|renewed| renewed) {
                    let _ = heartbeat_control.send(NativeWebhookRuntimeCommand::OwnershipLost);
                    break;
                  }
                }
              });
              loop {
                let received = actix_web::rt::task::spawn_blocking(move || {
                  let command = control_receiver.recv();
                  (control_receiver, command)
                })
                .await;
                let (receiver, command) = match received {
                  Ok(value) => value,
                  Err(_) => break,
                };
                control_receiver = receiver;
                match command {
                  Ok(NativeWebhookRuntimeCommand::Activate(response)) => {
                    let outcome = server
                      .activate()
                      .await
                      .map_err(native_trigger_runtime_error);
                    let _ = response.send(outcome);
                  }
                  Ok(NativeWebhookRuntimeCommand::Signal(request, response)) => {
                    let outcome = admit_external_signal_durable(
                      runtime_database_path.clone(),
                      request,
                      signal_execution.clone(),
                    )
                    .await
                    .map(|outcome| outcome.admission);
                    let _ = response.send(outcome);
                  }
                  Ok(NativeWebhookRuntimeCommand::OwnershipLost) => {
                    let now = Utc::now();
                    runtime_instance_reporter(NativeRuntimeInstanceProgress {
                      profile: "woml.runtime-instance/v1",
                      deployment_id: deployment_id.clone(),
                      activation_id: activation_id.clone(),
                      runtime_instance_id: thread_runtime_id.clone(),
                      runtime_version: env!("CARGO_PKG_VERSION"),
                      native_version: env!("CARGO_PKG_VERSION"),
                      lifecycle: "degraded",
                      started_at,
                      heartbeat_at: now,
                      lease_expires_at: now,
                    });
                    break;
                  }
                  Ok(NativeWebhookRuntimeCommand::Stop) | Err(_) => break,
                }
              }
              heartbeat.abort();
              server.stop_with_deadline(shutdown_deadline).await;
              let _ = DurableEventStore::open(&runtime_database_path)
                .and_then(|mut store| store.release_runtime_owner(&thread_runtime_id).map(|_| ()));
            }
            Err(error) => {
              let _ = startup_sender.send(Err(native_trigger_runtime_error(error)));
              let _ = DurableEventStore::open(&runtime_database_path)
                .and_then(|mut store| store.release_runtime_owner(&thread_runtime_id).map(|_| ()));
            }
          }
        });
      })
      .map_err(|error| {
        napi::Error::from_reason(format!("Could not start WOML webhook runtime: {error}"))
      })?;

    let startup = tokio::task::spawn_blocking(move || startup_receiver.recv())
      .await
      .map_err(|error| napi::Error::from_reason(format!("Webhook startup task failed: {error}")))?
      .map_err(|_| napi::Error::from_reason("Webhook startup channel closed.".to_string()))?;
    let address = match startup {
      Ok(address) => address,
      Err(error) => {
        let _ = tokio::task::spawn_blocking(move || join.join()).await;
        return Err(trigger_runtime_napi_error(error));
      }
    };

    webhook_runtimes()
      .lock()
      .map_err(|_| {
        napi::Error::from_reason("Webhook runtime registry is unavailable.".to_string())
      })?
      .insert(
        runtime_id.clone(),
        NativeWebhookRuntimeThread {
          control: control_sender,
          ingress: ingress_sender,
          manual_targets,
          join,
        },
      );
    let encoded = serde_json::to_string(&NativeWebhookRuntimeStarted {
      runtime_id,
      host: address.ip().to_string(),
      port: address.port(),
    })
    .map_err(|error| {
      napi::Error::from_reason(format!("Could not encode webhook runtime startup: {error}"))
    })?;
    start_span.bytes("result", encoded.len());
    start_span.succeed();
    Ok(encoded)
  })
}

#[napi(ts_return_type = "Promise<void>")]
pub async fn activate_woml_webhook_runtime(runtime_id: String) -> napi::Result<()> {
  let mut span = woml_engine::performance::PerformanceSpan::new_for_process(
    "native",
    "napi",
    "napi.activate_trigger_runtime",
  );
  let control = webhook_runtimes()
    .lock()
    .map_err(|_| napi::Error::from_reason("Webhook runtime registry is unavailable.".to_string()))?
    .get(&runtime_id)
    .map(|runtime| runtime.control.clone())
    .ok_or_else(|| napi::Error::from_reason("WOML webhook runtime does not exist.".to_string()))?;
  let (response_sender, response_receiver) = mpsc::sync_channel(1);
  control
    .send(NativeWebhookRuntimeCommand::Activate(response_sender))
    .map_err(|_| napi::Error::from_reason("WOML webhook runtime is stopping.".to_string()))?;
  let result = tokio::task::spawn_blocking(move || response_receiver.recv())
    .await
    .map_err(|error| napi::Error::from_reason(format!("Runtime activation task failed: {error}")))?
    .map_err(|_| napi::Error::from_reason("Runtime activation response was lost.".to_string()))?;
  result.map_err(trigger_runtime_napi_error)?;
  span.succeed();
  Ok(())
}

#[napi(ts_return_type = "Promise<string>")]
pub async fn submit_woml_external_signal(
  runtime_id: String,
  request_json: String,
) -> napi::Result<String> {
  let request: ExternalSignalAdmissionRequest = serde_json::from_str(&request_json)
    .map_err(|error| napi::Error::from_reason(format!("Invalid external signal JSON: {error}")))?;
  let control = webhook_runtimes()
    .lock()
    .map_err(|_| napi::Error::from_reason("Trigger runtime registry is unavailable.".to_string()))?
    .get(&runtime_id)
    .map(|runtime| runtime.control.clone())
    .ok_or_else(|| napi::Error::from_reason("WOML trigger runtime does not exist.".to_string()))?;
  let (response_sender, response_receiver) = mpsc::sync_channel(1);
  control
    .send(NativeWebhookRuntimeCommand::Signal(
      request,
      response_sender,
    ))
    .map_err(|_| napi::Error::from_reason("WOML trigger runtime is stopping.".to_string()))?;
  let outcome = tokio::task::spawn_blocking(move || response_receiver.recv())
    .await
    .map_err(|error| napi::Error::from_reason(format!("Signal admission task failed: {error}")))?
    .map_err(|_| napi::Error::from_reason("Signal admission response was lost.".to_string()))?
    .map_err(native_execution_error)?;
  serde_json::to_string(&outcome)
    .map_err(|error| napi::Error::from_reason(format!("Could not encode signal outcome: {error}")))
}

#[napi(ts_return_type = "Promise<string>")]
pub async fn submit_woml_trigger_occurrence(
  runtime_id: String,
  ingress_json: String,
) -> napi::Result<String> {
  let ingress: NativeTriggerIngressRequest = serde_json::from_str(&ingress_json)
    .map_err(|error| napi::Error::from_reason(format!("Invalid trigger ingress JSON: {error}")))?;
  if ingress.contract != "woml.trigger-ingress"
    || ingress.contract_version != 1
    || ingress.message_type != "admit"
    || ingress.request_id.is_empty()
    || !matches!(
      ingress.trigger_handler.as_str(),
      "trigger.slack" | "trigger.telegram" | "trigger.discord"
    )
  {
    return Err(napi::Error::from_reason(
      "Invalid communication trigger ingress contract.".to_string(),
    ));
  }
  let request_id = ingress.request_id;
  let sender = webhook_runtimes()
    .lock()
    .map_err(|_| napi::Error::from_reason("Trigger runtime registry is unavailable.".to_string()))?
    .get(&runtime_id)
    .map(|runtime| runtime.ingress.clone())
    .ok_or_else(|| napi::Error::from_reason("WOML trigger runtime does not exist.".to_string()))?;
  let (response_sender, response_receiver) = tokio::sync::oneshot::channel();
  sender
    .send(ExternalTriggerAdmissionCommand {
      request: TriggerAdmissionRequest {
        workflow_id: ingress.workflow_id,
        definition_hash: ingress.definition_hash,
        trigger_id: ingress.trigger_id,
        trigger_handler: ingress.trigger_handler,
        source_identity: ingress.source_identity,
        payload: ingress.payload,
        received_at: ingress.received_at,
      },
      response: response_sender,
    })
    .map_err(|_| napi::Error::from_reason("WOML trigger runtime is stopping.".to_string()))?;
  let outcome = response_receiver
    .await
    .map_err(|_| napi::Error::from_reason("WOML trigger ingress response was lost.".to_string()))?;
  let json = match outcome {
    Ok(outcome) => serde_json::to_string(&NativeTriggerIngressAccepted {
      contract: "woml.trigger-ingress",
      contract_version: 1,
      message_type: "accepted",
      request_id,
      occurrence_id: outcome.occurrence_id,
      run_id: outcome.run_id,
      duplicate: outcome.duplicate,
    }),
    Err(DurableStoreError::TriggerIdempotencyConflict) => {
      serde_json::to_string(&NativeTriggerIngressRejected {
        contract: "woml.trigger-ingress",
        contract_version: 1,
        message_type: "rejected",
        request_id,
        failure: NativeTriggerIngressFailure {
          code: "WOML_TRIGGER_IDEMPOTENCY_CONFLICT",
          message: "The source identity is already bound to a different payload.",
          retryable: false,
        },
      })
    }
    Err(DurableStoreError::RuntimePolicyQueueFull) => {
      serde_json::to_string(&NativeTriggerIngressRejected {
        contract: "woml.trigger-ingress",
        contract_version: 1,
        message_type: "rejected",
        request_id,
        failure: NativeTriggerIngressFailure {
          code: "WOML_POLICY_QUEUE_FULL",
          message: "The durable WOML policy queue is full; Slack may redeliver this event.",
          retryable: true,
        },
      })
    }
    Err(error) => {
      let (code, message, retryable) = match error {
        DurableStoreError::DefinitionNotFound(_)
        | DurableStoreError::TriggerDefinitionMismatch => (
          "WOML_TRIGGER_DEFINITION_MISMATCH",
          "The provider event does not match the active workflow definition; restart the WOML runtime.",
          false,
        ),
        DurableStoreError::TriggerNotFound { .. } => (
          "WOML_TRIGGER_NOT_FOUND",
          "The provider event references a trigger that is not registered in the active workflow.",
          false,
        ),
        DurableStoreError::TriggerHandlerMismatch => (
          "WOML_TRIGGER_HANDLER_MISMATCH",
          "The provider event type does not match the compiled workflow trigger.",
          false,
        ),
        DurableStoreError::Contract(_) | DurableStoreError::InvalidModel(_) => (
          "WOML_TRIGGER_CONTRACT_INVALID",
          "The compiled workflow and durable trigger contracts are incompatible; update or rebuild WOML.",
          false,
        ),
        _ => (
          "WOML_TRIGGER_UNAVAILABLE",
          "The durable WOML trigger authority is temporarily unavailable.",
          true,
        ),
      };
      serde_json::to_string(&NativeTriggerIngressRejected {
        contract: "woml.trigger-ingress",
        contract_version: 1,
        message_type: "rejected",
        request_id,
        failure: NativeTriggerIngressFailure {
          code,
          message,
          retryable,
        },
      })
    }
  };
  json.map_err(|error| {
    napi::Error::from_reason(format!("Could not encode trigger ingress outcome: {error}"))
  })
}

#[napi(ts_return_type = "Promise<string>")]
pub async fn submit_woml_manual_trigger(
  runtime_id: String,
  request_json: String,
) -> napi::Result<String> {
  let mut span = woml_engine::performance::PerformanceSpan::new_for_process(
    "native",
    "napi",
    "napi.submit_manual_trigger",
  );
  span.bytes("request", request_json.len());
  let request: NativeManualAdmissionRequest = serde_json::from_str(&request_json)
    .map_err(|error| napi::Error::from_reason(format!("Invalid manual admission JSON: {error}")))?;
  if request.profile != "woml.manual-trigger-admission/v1"
    || request.message_type != "request"
    || request.request_id.is_empty()
    || request.request_id.len() > 256
    || request.workflow_id.is_empty()
    || request.workflow_id.len() > 256
    || request.trigger_id.is_empty()
    || request.trigger_id.len() > 256
    || !request.payload.is_empty()
  {
    return Err(napi::Error::from_reason(
      "Invalid Manual Trigger Admission v1 request.".to_string(),
    ));
  }

  let request_id = request.request_id;
  span.invocation_id(request_id.clone());
  let target_key = (request.workflow_id.clone(), request.trigger_id.clone());
  let runtime = webhook_runtimes()
    .lock()
    .map_err(|_| napi::Error::from_reason("Trigger runtime registry is unavailable.".to_string()))?
    .get(&runtime_id)
    .map(|runtime| {
      (
        runtime.ingress.clone(),
        runtime.manual_targets.get(&target_key).cloned(),
      )
    });
  let Some((sender, target)) = runtime else {
    return encode_manual_rejection(
      request_id,
      "WOML_MANUAL_TRIGGER_ADMISSION_CLOSED",
      "The runtime is draining and no longer accepts manual triggers.",
    );
  };
  let Some(target) = target else {
    return encode_manual_rejection(
      request_id,
      "WOML_MANUAL_TRIGGER_SELECTION_REQUIRED",
      "Select one registered manual workflow trigger.",
    );
  };

  let (response_sender, response_receiver) = tokio::sync::oneshot::channel();
  if sender
    .send(ExternalTriggerAdmissionCommand {
      request: TriggerAdmissionRequest {
        workflow_id: request.workflow_id,
        definition_hash: target.definition_hash,
        trigger_id: request.trigger_id,
        trigger_handler: "trigger.manual".to_string(),
        source_identity: format!("manual:{request_id}"),
        payload: request.payload,
        received_at: request.requested_at,
      },
      response: response_sender,
    })
    .is_err()
  {
    return encode_manual_rejection(
      request_id,
      "WOML_MANUAL_TRIGGER_ADMISSION_CLOSED",
      "The runtime is draining and no longer accepts manual triggers.",
    );
  }

  let outcome = match response_receiver.await {
    Ok(outcome) => outcome,
    Err(_) => {
      return encode_manual_rejection(
        request_id,
        "WOML_MANUAL_TRIGGER_ADMISSION_CLOSED",
        "The runtime stopped before the manual trigger could be admitted.",
      );
    }
  };
  if let Ok(admitted) = &outcome {
    span.run_id(admitted.run_id.clone());
  }
  let encoded = match outcome {
    Ok(outcome) => serde_json::to_string(&NativeManualAdmissionAccepted {
      profile: "woml.manual-trigger-admission/v1",
      message_type: "accepted",
      request_id,
      occurrence_id: outcome.occurrence_id,
      run_id: outcome.run_id,
      status: target.admission_status,
    })
    .map_err(|error| {
      napi::Error::from_reason(format!(
        "Could not encode manual admission outcome: {error}"
      ))
    }),
    Err(DurableStoreError::RuntimePolicyQueueFull) => encode_manual_rejection(
      request_id,
      "WOML_POLICY_QUEUE_FULL",
      "The durable workflow queue is full; try the manual trigger again later.",
    ),
    Err(_) => encode_manual_rejection(
      request_id,
      "WOML_MANUAL_TRIGGER_ADMISSION_CLOSED",
      "The durable runtime cannot accept this manual trigger right now.",
    ),
  };
  if encoded.is_ok() {
    span.succeed();
  }
  encoded
}

#[napi(ts_return_type = "Promise<void>")]
pub async fn stop_woml_webhook_runtime(runtime_id: String) -> napi::Result<()> {
  let runtime = webhook_runtimes()
    .lock()
    .map_err(|_| napi::Error::from_reason("Webhook runtime registry is unavailable.".to_string()))?
    .remove(&runtime_id)
    .ok_or_else(|| napi::Error::from_reason("WOML webhook runtime does not exist.".to_string()))?;
  drop(runtime.ingress);
  let _ = runtime.control.send(NativeWebhookRuntimeCommand::Stop);
  tokio::task::spawn_blocking(move || runtime.join.join())
    .await
    .map_err(|error| napi::Error::from_reason(format!("Webhook shutdown task failed: {error}")))?
    .map_err(|_| {
      napi::Error::from_reason("WOML webhook runtime panicked during shutdown.".to_string())
    })?;
  Ok(())
}

#[napi]
pub fn inspect_woml_run(event_store_path: String, run_id: String) -> napi::Result<String> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(native_run_inspection_error)?;
  let projection = store
    .projection(&run_id)
    .map_err(native_run_inspection_error)?;
  let workflow_calls = store
    .workflow_call_relations_for_run(&run_id)
    .map_err(native_run_inspection_error)?;
  let inspection = NativeRunInspection {
    run_id: projection.run_id.unwrap_or(run_id),
    workflow_id: projection.workflow_id.unwrap_or_default(),
    status: projection.status,
    terminal_node_id: projection.terminal_node_id,
    result: projection.result,
    failure_code: projection.failure.as_ref().map(run_failure_code),
    workflow_calls,
  };
  serde_json::to_string(&inspection)
    .map_err(|error| napi::Error::from_reason(format!("Could not encode WOML run: {error}")))
}

#[napi]
pub fn inspect_woml_remote_dispatch(
  event_store_path: String,
  dispatch_id: String,
) -> napi::Result<String> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_management_error("WOML_TARGET_UNAVAILABLE", error))?;
  let inspection = store
    .inspect_remote_dispatch(&dispatch_id)
    .map_err(|error| napi::Error::from_reason(format!("{}: {}", error.code(), error)))?;
  serde_json::to_string(&inspection).map_err(|error| {
    napi::Error::from_reason(format!(
      "Could not encode remote dispatch inspection: {error}"
    ))
  })
}

#[napi]
pub fn reconcile_woml_remote_dispatch(
  event_store_path: String,
  dispatch_id: String,
  resolution: String,
  actor: String,
  evidence_digest: String,
) -> napi::Result<String> {
  let resolution = match resolution.as_str() {
    "record-failure" => RemoteDispatchResolutionV1::RecordFailure,
    "authorize-possible-duplicate-retry" => {
      RemoteDispatchResolutionV1::AuthorizePossibleDuplicateRetry
    }
    _ => {
      return Err(napi::Error::from_reason(
        "WOML_REMOTE_PROTOCOL_INVALID: invalid reconciliation resolution".to_string(),
      ))
    }
  };
  let mut store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_management_error("WOML_TARGET_UNAVAILABLE", error))?;
  let record = store
    .reconcile_remote_dispatch(
      &dispatch_id,
      resolution,
      &actor,
      &evidence_digest,
      Utc::now(),
    )
    .map_err(|error| napi::Error::from_reason(format!("{}: {}", error.code(), error)))?;
  serde_json::to_string(&record).map_err(|error| {
    napi::Error::from_reason(format!(
      "Could not encode remote dispatch reconciliation: {error}"
    ))
  })
}

#[napi]
pub fn inspect_woml_run_presentation(
  event_store_path: String,
  run_id: String,
) -> napi::Result<String> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_presentation_error(RunPresentationError::Store(error)))?;
  let presentation =
    run_presentation_from_store_v1(&store, &run_id).map_err(native_run_presentation_error)?;
  serde_json::to_string(&presentation).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode WOML run presentation: {error}"))
  })
}

#[napi]
pub fn list_woml_run_presentations(
  event_store_path: String,
  workflow_id: String,
  limit: u32,
) -> napi::Result<String> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_presentation_error(RunPresentationError::Store(error)))?;
  let presentations = recent_run_presentations_from_store_v1(
    &store,
    &workflow_id,
    usize::try_from(limit).unwrap_or(usize::MAX),
  )
  .map_err(native_run_presentation_error)?;
  serde_json::to_string(&presentations).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode WOML run presentations: {error}"))
  })
}

#[napi]
pub fn has_woml_workflow_definition(
  event_store_path: String,
  workflow_id: String,
) -> napi::Result<bool> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_presentation_error(RunPresentationError::Store(error)))?;
  store
    .has_definition_for_workflow(&workflow_id)
    .map_err(|error| native_run_presentation_error(RunPresentationError::Store(error)))
}

#[napi]
pub fn list_woml_runs(
  event_store_path: String,
  limit: u32,
  workflow_id: Option<String>,
  status: Option<String>,
) -> napi::Result<String> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_management_error("WOML_RUN_LIST_FAILED", error))?;
  let list = store
    .list_runs_v2_filtered(
      usize::try_from(limit).unwrap_or(usize::MAX),
      workflow_id.as_deref(),
      status.as_deref(),
    )
    .map_err(|error| native_run_management_error("WOML_RUN_LIST_FAILED", error))?;
  serde_json::to_string(&list)
    .map_err(|error| napi::Error::from_reason(format!("Could not encode WOML run list: {error}")))
}

#[napi]
pub fn observe_woml_runtime(event_store_path: String) -> napi::Result<String> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_management_error("WOML_OBSERVABILITY_UNAVAILABLE", error))?;
  let observation = store
    .runtime_observation_v1()
    .map_err(|error| native_run_management_error("WOML_OBSERVABILITY_UNAVAILABLE", error))?;
  serde_json::to_string(&observation).map_err(|error| {
    napi::Error::from_reason(format!(
      "Could not encode WOML runtime observation: {error}"
    ))
  })
}

fn operational_output_napi_error(error: woml_engine::OperationalOutputError) -> napi::Error {
  napi::Error::from_reason(format!("{}: {error}", error.code()))
}

fn operational_cursor(value: f64) -> napi::Result<u64> {
  if !value.is_finite() || value < 0.0 || value.fract() != 0.0 || value > 9_007_199_254_740_991.0 {
    return Err(napi::Error::from_reason(
      "WOML_OUTPUT_CURSOR_INVALID: the operational-output cursor is invalid".to_string(),
    ));
  }
  Ok(value as u64)
}

#[napi]
pub fn read_woml_operational_output_page(
  event_store_path: String,
  run_id: Option<String>,
  after: f64,
  limit: u32,
) -> napi::Result<String> {
  let store = woml_engine::operational_output_store_for_state(&PathBuf::from(event_store_path))
    .and_then(|store| store.activate())
    .map_err(operational_output_napi_error)?;
  let after = operational_cursor(after)?;
  let page = match run_id {
    Some(run_id) => {
      store.read_run_page(&run_id, after, usize::try_from(limit).unwrap_or(usize::MAX))
    }
    None => store.read_page(after, usize::try_from(limit).unwrap_or(usize::MAX)),
  }
  .map_err(operational_output_napi_error)?;
  serde_json::to_string(&page).map_err(|error| {
    napi::Error::from_reason(format!(
      "WOML_OUTPUT_STORE_UNAVAILABLE: could not encode operational output: {error}"
    ))
  })
}

#[napi]
pub fn inspect_woml_operational_progress(
  event_store_path: String,
  run_id: String,
) -> napi::Result<String> {
  let store = woml_engine::operational_output_store_for_state(&PathBuf::from(event_store_path))
    .and_then(|store| store.activate())
    .map_err(operational_output_napi_error)?;
  let progress = store
    .latest_progress(&run_id)
    .map_err(operational_output_napi_error)?;
  serde_json::to_string(&progress).map_err(|error| {
    napi::Error::from_reason(format!(
      "WOML_OUTPUT_STORE_UNAVAILABLE: could not encode operational progress: {error}"
    ))
  })
}

#[napi]
pub fn observe_woml_operational_output(event_store_path: String) -> napi::Result<String> {
  let store = woml_engine::operational_output_store_for_state(&PathBuf::from(event_store_path))
    .and_then(|store| store.activate())
    .map_err(operational_output_napi_error)?;
  let observation = store.inspect().map_err(operational_output_napi_error)?;
  serde_json::to_string(&observation).map_err(|error| {
    napi::Error::from_reason(format!(
      "WOML_OUTPUT_STORE_UNAVAILABLE: could not encode operational observation: {error}"
    ))
  })
}

#[napi]
pub fn plan_woml_operational_output_prune(
  event_store_path: String,
  cutoff: String,
) -> napi::Result<String> {
  let cutoff = DateTime::parse_from_rfc3339(&cutoff)
    .map_err(|_| {
      napi::Error::from_reason(
        "WOML_OUTPUT_RETENTION_INVALID: the output retention cutoff is invalid".to_string(),
      )
    })?
    .with_timezone(&Utc);
  let lazy = woml_engine::operational_output_store_for_state(&PathBuf::from(event_store_path))
    .map_err(operational_output_napi_error)?;
  let plan = match lazy
    .activate_if_exists()
    .map_err(operational_output_napi_error)?
  {
    Some(store) => store
      .plan_prune_terminal_before(cutoff, 1_000)
      .map_err(operational_output_napi_error)?,
    None => woml_engine::OperationalOutputPrunePlanV1 {
      profile: "woml.operational-output-prune-plan/v1".to_string(),
      eligible_runs: 0,
      eligible_records: 0,
      eligible_output_bytes: 0,
    },
  };
  serde_json::to_string(&plan).map_err(|error| {
    napi::Error::from_reason(format!(
      "WOML_OUTPUT_STORE_UNAVAILABLE: could not encode output retention plan: {error}"
    ))
  })
}

#[napi]
pub fn prune_woml_operational_output(
  event_store_path: String,
  cutoff: String,
) -> napi::Result<String> {
  let cutoff = DateTime::parse_from_rfc3339(&cutoff)
    .map_err(|_| {
      napi::Error::from_reason(
        "WOML_OUTPUT_RETENTION_INVALID: the output retention cutoff is invalid".to_string(),
      )
    })?
    .with_timezone(&Utc);
  let lazy = woml_engine::operational_output_store_for_state(&PathBuf::from(event_store_path))
    .map_err(operational_output_napi_error)?;
  let result = match lazy
    .activate_if_exists()
    .map_err(operational_output_napi_error)?
  {
    Some(store) => store
      .prune_terminal_before(cutoff, 1_000)
      .map_err(operational_output_napi_error)?,
    None => woml_engine::OperationalOutputPruneResultV1 {
      profile: "woml.operational-output-prune/v1".to_string(),
      pruned_runs: 0,
      pruned_records: 0,
      pruned_output_bytes: 0,
      pruned_through: 0,
    },
  };
  serde_json::to_string(&result).map_err(|error| {
    napi::Error::from_reason(format!(
      "WOML_OUTPUT_STORE_UNAVAILABLE: could not encode output retention result: {error}"
    ))
  })
}

#[napi]
pub fn create_woml_backup(
  event_store_path: String,
  destination_path: String,
  lease_id: String,
  owner_id: String,
  fallback_deployment_id: String,
) -> napi::Result<String> {
  let inspection = create_online_backup(
    PathBuf::from(event_store_path),
    PathBuf::from(destination_path),
    &lease_id,
    &owner_id,
    &fallback_deployment_id,
  )
  .map_err(|error| native_backup_error(error, "WOML_BACKUP_FAILED"))?;
  serde_json::to_string(&inspection).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode backup inventory: {error}"))
  })
}

#[napi]
pub fn inspect_woml_backup_store(event_store_path: String) -> napi::Result<String> {
  let inspection = inspect_backup_store(PathBuf::from(event_store_path))
    .map_err(|error| native_backup_error(error, "WOML_BACKUP_VERIFICATION_FAILED"))?;
  serde_json::to_string(&inspection).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode backup inspection: {error}"))
  })
}

#[napi]
pub fn record_woml_verified_backup(
  event_store_path: String,
  backup_id: String,
  completed_at: String,
) -> napi::Result<()> {
  let completed_at = DateTime::parse_from_rfc3339(&completed_at)
    .map_err(|_| napi::Error::from_reason("Backup completion time is invalid.".to_string()))?
    .with_timezone(&Utc);
  record_verified_backup(PathBuf::from(event_store_path), &backup_id, completed_at)
    .map_err(|error| native_backup_error(error, "WOML_BACKUP_FAILED"))
}

#[napi]
pub fn prepare_woml_restored_store(
  event_store_path: String,
  expected_definition_hashes_json: String,
  backup_id: String,
  restored_at: String,
) -> napi::Result<String> {
  let expected: Vec<String> =
    serde_json::from_str(&expected_definition_hashes_json).map_err(|_| {
      napi::Error::from_reason("Restore definition inventory is invalid.".to_string())
    })?;
  let restored_at = DateTime::parse_from_rfc3339(&restored_at)
    .map_err(|_| napi::Error::from_reason("Restore time is invalid.".to_string()))?
    .with_timezone(&Utc);
  let inspection = prepare_restored_store(
    PathBuf::from(event_store_path),
    &expected,
    &backup_id,
    restored_at,
  )
  .map_err(|error| native_backup_error(error, "WOML_RESTORE_FAILED"))?;
  serde_json::to_string(&inspection).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode restore inspection: {error}"))
  })
}

#[napi]
pub fn plan_woml_retention(
  event_store_path: String,
  policy_json: String,
  now: String,
) -> napi::Result<String> {
  let policy: RetentionPolicyV1 = serde_json::from_str(&policy_json)
    .map_err(|_| napi::Error::from_reason("Retention Policy v1 is invalid.".to_string()))?;
  let now = DateTime::parse_from_rfc3339(&now)
    .map_err(|_| napi::Error::from_reason("Retention planning time is invalid.".to_string()))?
    .with_timezone(&Utc);
  let plan = plan_retention(PathBuf::from(event_store_path), &policy, now)
    .map_err(native_retention_error)?;
  serde_json::to_string(&plan)
    .map_err(|error| napi::Error::from_reason(format!("Could not encode retention plan: {error}")))
}

#[napi]
pub fn execute_woml_retention(
  event_store_path: String,
  policy_json: String,
  lease_id: String,
  owner_id: String,
  compact: bool,
  now: String,
) -> napi::Result<String> {
  let policy: RetentionPolicyV1 = serde_json::from_str(&policy_json)
    .map_err(|_| napi::Error::from_reason("Retention Policy v1 is invalid.".to_string()))?;
  let now = DateTime::parse_from_rfc3339(&now)
    .map_err(|_| napi::Error::from_reason("Retention execution time is invalid.".to_string()))?
    .with_timezone(&Utc);
  let outcome = execute_retention(
    PathBuf::from(event_store_path),
    &policy,
    &lease_id,
    &owner_id,
    compact,
    now,
  )
  .map_err(native_retention_error)?;
  serde_json::to_string(&outcome).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode retention result: {error}"))
  })
}

#[napi]
pub async fn execute_woml_retention_async(
  event_store_path: String,
  policy_json: String,
  lease_id: String,
  owner_id: String,
  compact: bool,
  now: String,
) -> napi::Result<String> {
  tokio::task::spawn_blocking(move || {
    execute_woml_retention(
      event_store_path,
      policy_json,
      lease_id,
      owner_id,
      compact,
      now,
    )
  })
  .await
  .map_err(|error| {
    napi::Error::from_reason(format!("Retention worker could not complete: {error}"))
  })?
}

#[napi]
pub fn read_woml_last_retention_result(event_store_path: String) -> napi::Result<String> {
  let result =
    last_retention_result(PathBuf::from(event_store_path)).map_err(native_retention_error)?;
  serde_json::to_string(&result)
    .map_err(|error| napi::Error::from_reason(format!("Could not encode retention audit: {error}")))
}

#[napi]
pub fn maintain_woml_artifacts(
  event_store_path: String,
  policy_json: String,
  mode_json: String,
  resolved_secrets_json: String,
  now: String,
  dry_run: bool,
) -> napi::Result<String> {
  let policy: ArtifactPolicyV1 = serde_json::from_str(&policy_json)
    .map_err(|_| napi::Error::from_reason("Artifact Policy v1 is invalid.".to_string()))?;
  let mode: ArtifactRuntimeMode = serde_json::from_str(&mode_json)
    .map_err(|_| napi::Error::from_reason("Artifact runtime mode is invalid.".to_string()))?;
  let secrets: BTreeMap<String, String> = serde_json::from_str(&resolved_secrets_json)
    .map_err(|_| napi::Error::from_reason("Resolved artifact secrets are invalid.".to_string()))?;
  let now = DateTime::parse_from_rfc3339(&now)
    .map_err(|_| napi::Error::from_reason("Artifact maintenance time is invalid.".to_string()))?
    .with_timezone(&Utc);
  let result = maintain_artifacts(
    PathBuf::from(event_store_path),
    policy,
    mode,
    &secrets,
    now,
    dry_run,
  )
  .map_err(|failure| napi::Error::from_reason(format!("{}: {}", failure.code, failure.message)))?;
  serde_json::to_string(&result).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode artifact maintenance: {error}"))
  })
}

#[napi]
pub fn inspect_woml_artifact(
  event_store_path: String,
  artifact_id: String,
) -> napi::Result<String> {
  let result =
    inspect_artifact(PathBuf::from(event_store_path), &artifact_id).map_err(|failure| {
      napi::Error::from_reason(format!("{}: {}", failure.code, failure.message))
    })?;
  serde_json::to_string(&result).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode artifact inspection: {error}"))
  })
}

#[napi]
pub fn inspect_woml_run_artifacts(
  event_store_path: String,
  run_id: String,
) -> napi::Result<String> {
  let result =
    inspect_run_artifacts(PathBuf::from(event_store_path), &run_id).map_err(|failure| {
      napi::Error::from_reason(format!("{}: {}", failure.code, failure.message))
    })?;
  serde_json::to_string(&result).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode run artifact inspection: {error}"))
  })
}

#[napi]
pub fn export_woml_artifact(
  event_store_path: String,
  destination_path: String,
  artifact_id: String,
  policy_json: String,
  mode_json: String,
  resolved_secrets_json: String,
) -> napi::Result<String> {
  let (policy, mode, secrets) =
    decode_artifact_maintenance_authority(&policy_json, &mode_json, &resolved_secrets_json)?;
  let result = export_artifact(
    PathBuf::from(event_store_path),
    PathBuf::from(destination_path),
    &artifact_id,
    policy,
    mode,
    &secrets,
  )
  .map_err(|failure| napi::Error::from_reason(format!("{}: {}", failure.code, failure.message)))?;
  serde_json::to_string(&result)
    .map_err(|error| napi::Error::from_reason(format!("Could not encode artifact export: {error}")))
}

#[napi]
pub fn observe_woml_artifacts(
  event_store_path: String,
  policy_json: String,
  mode_json: String,
  resolved_secrets_json: String,
) -> napi::Result<String> {
  let (policy, mode, secrets) =
    decode_artifact_maintenance_authority(&policy_json, &mode_json, &resolved_secrets_json)?;
  let result = observe_artifacts(PathBuf::from(event_store_path), policy, mode, &secrets).map_err(
    |failure| napi::Error::from_reason(format!("{}: {}", failure.code, failure.message)),
  )?;
  serde_json::to_string(&result).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode artifact observation: {error}"))
  })
}

fn decode_artifact_maintenance_authority(
  policy_json: &str,
  mode_json: &str,
  resolved_secrets_json: &str,
) -> napi::Result<(
  ArtifactPolicyV1,
  ArtifactRuntimeMode,
  BTreeMap<String, String>,
)> {
  let policy = serde_json::from_str(policy_json)
    .map_err(|_| napi::Error::from_reason("Artifact Policy v1 is invalid.".to_string()))?;
  let mode = serde_json::from_str(mode_json)
    .map_err(|_| napi::Error::from_reason("Artifact runtime mode is invalid.".to_string()))?;
  let secrets = serde_json::from_str(resolved_secrets_json)
    .map_err(|_| napi::Error::from_reason("Resolved artifact secrets are invalid.".to_string()))?;
  Ok((policy, mode, secrets))
}

#[napi]
pub fn export_woml_artifact_backup(
  event_store_path: String,
  destination_path: String,
  policy_json: String,
  mode_json: String,
  resolved_secrets_json: String,
) -> napi::Result<String> {
  let (policy, mode, secrets) =
    decode_artifact_maintenance_authority(&policy_json, &mode_json, &resolved_secrets_json)?;
  let result = export_artifact_backup(
    PathBuf::from(event_store_path),
    PathBuf::from(destination_path),
    policy,
    mode,
    &secrets,
  )
  .map_err(|failure| napi::Error::from_reason(format!("{}: {}", failure.code, failure.message)))?;
  serde_json::to_string(&result).map_err(|error| {
    napi::Error::from_reason(format!(
      "Could not encode artifact backup inventory: {error}"
    ))
  })
}

#[napi]
pub fn restore_woml_artifact_backup(
  event_store_path: String,
  source_path: String,
  policy_json: String,
  mode_json: String,
  resolved_secrets_json: String,
) -> napi::Result<String> {
  let (policy, mode, secrets) =
    decode_artifact_maintenance_authority(&policy_json, &mode_json, &resolved_secrets_json)?;
  let result = restore_artifact_backup(
    PathBuf::from(event_store_path),
    PathBuf::from(source_path),
    policy,
    mode,
    &secrets,
  )
  .map_err(|failure| napi::Error::from_reason(format!("{}: {}", failure.code, failure.message)))?;
  serde_json::to_string(&result).map_err(|error| {
    napi::Error::from_reason(format!(
      "Could not encode artifact restore inventory: {error}"
    ))
  })
}

#[napi]
pub fn verify_woml_artifact_backup(
  event_store_path: String,
  source_path: String,
) -> napi::Result<String> {
  let result = verify_artifact_backup(PathBuf::from(event_store_path), PathBuf::from(source_path))
    .map_err(|failure| {
      napi::Error::from_reason(format!("{}: {}", failure.code, failure.message))
    })?;
  serde_json::to_string(&result).map_err(|error| {
    napi::Error::from_reason(format!(
      "Could not encode artifact backup verification: {error}"
    ))
  })
}

#[napi]
pub fn inspect_woml_run_v2(event_store_path: String, run_id: String) -> napi::Result<String> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_management_error("WOML_RUN_INSPECTION_FAILED", error))?;
  let binding = store.run_binding(&run_id).map_err(|error| {
    let code = if matches!(error, DurableStoreError::RunNotFound(_)) {
      "WOML_RUN_NOT_FOUND"
    } else {
      "WOML_RUN_INSPECTION_FAILED"
    };
    native_run_management_error(code, error)
  })?;
  let workflow = store
    .definition(&binding.definition_hash)
    .map_err(|error| native_run_management_error("WOML_RUN_INSPECTION_FAILED", error))?;
  let mut inspection = if workflow.schema_version >= 20 {
    serde_json::to_value(
      store
        .inspect_run_v11(&run_id)
        .map_err(|error| native_run_management_error("WOML_RUN_INSPECTION_FAILED", error))?,
    )
  } else if workflow.schema_version >= 16 {
    serde_json::to_value(
      store
        .inspect_run_v6(&run_id)
        .map_err(|error| native_run_management_error("WOML_RUN_INSPECTION_FAILED", error))?,
    )
  } else if workflow.schema_version >= 14 {
    serde_json::to_value(
      store
        .inspect_run_v5(&run_id)
        .map_err(|error| native_run_management_error("WOML_RUN_INSPECTION_FAILED", error))?,
    )
  } else if workflow.schema_version >= 13 {
    serde_json::to_value(
      store
        .inspect_run_v4(&run_id)
        .map_err(|error| native_run_management_error("WOML_RUN_INSPECTION_FAILED", error))?,
    )
  } else if workflow.schema_version >= 12 {
    serde_json::to_value(
      store
        .inspect_run_v3(&run_id)
        .map_err(|error| native_run_management_error("WOML_RUN_INSPECTION_FAILED", error))?,
    )
  } else {
    serde_json::to_value(
      store
        .inspect_run_v2(&run_id)
        .map_err(|error| native_run_management_error("WOML_RUN_INSPECTION_FAILED", error))?,
    )
  }
  .map_err(|error| {
    napi::Error::from_reason(format!("Could not encode WOML run inspection: {error}"))
  })?;
  if let Some(policy) = store
    .infrastructure_policy_binding(&run_id)
    .map_err(|error| native_run_management_error("WOML_RUN_INSPECTION_FAILED", error))?
  {
    let projection = store
      .projection(&run_id)
      .map_err(|error| native_run_management_error("WOML_RUN_INSPECTION_FAILED", error))?;
    let total_decisions = projection.infrastructure_policy_decisions.len();
    let decisions = projection
      .infrastructure_policy_decisions
      .iter()
      .rev()
      .take(100)
      .map(|event| event.decision.clone())
      .collect::<Vec<_>>();
    if let Some(object) = inspection.as_object_mut() {
      object.insert(
        "infrastructurePolicy".to_string(),
        serde_json::json!({
          "profile": "woml.infrastructure-policy-run-inspection/v1",
          "binding": policy,
          "decisions": decisions,
          "totalDecisions": total_decisions,
          "truncated": total_decisions > 100,
        }),
      );
    }
  }
  serde_json::to_string(&inspection).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode WOML run inspection: {error}"))
  })
}

#[napi]
pub fn inspect_woml_waits_v1(event_store_path: String, run_id: String) -> napi::Result<String> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_management_error("WOML_WAIT_INSPECTION_FAILED", error))?;
  let inspection = store
    .inspect_waits_v1(&run_id)
    .map_err(|error| native_run_management_error("WOML_WAIT_INSPECTION_FAILED", error))?;
  serde_json::to_string(&inspection).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode WOML wait inspection: {error}"))
  })
}

#[napi]
pub fn inspect_woml_correction_v1(
  event_store_path: String,
  run_id: String,
) -> napi::Result<String> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_management_error("WOML_CORRECTION_INSPECTION_FAILED", error))?;
  let binding = store
    .run_binding(&run_id)
    .map_err(|error| native_run_management_error("WOML_CORRECTION_INSPECTION_FAILED", error))?;
  let workflow = store
    .definition(&binding.definition_hash)
    .map_err(|error| native_run_management_error("WOML_CORRECTION_INSPECTION_FAILED", error))?;
  if workflow.schema_version != COMPILED_MODEL_SCHEMA_VERSION_V19 {
    return Ok("null".to_string());
  }
  let inspection = store
    .inspect_correction_v1(&run_id)
    .map_err(|error| native_run_management_error("WOML_CORRECTION_INSPECTION_FAILED", error))?;
  serde_json::to_string(&inspection).map_err(|error| {
    napi::Error::from_reason(format!(
      "Could not encode WOML correction inspection: {error}"
    ))
  })
}

#[napi]
pub fn recover_woml_correction(
  event_store_path: String,
  request_json: String,
) -> napi::Result<String> {
  let request: CorrectionRecoveryRequestV1 = serde_json::from_str(&request_json).map_err(|_| {
    napi::Error::from_reason(
      "WOML_CORRECTION_NOT_RECOVERABLE: invalid Correction Recovery v1 request.".to_string(),
    )
  })?;
  let mut store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_management_error("WOML_CORRECTION_RECOVERY_FAILED", error))?;
  let result = store
    .recover_correction_v1(request, Utc::now())
    .map_err(|error| native_run_management_error("WOML_CORRECTION_RECOVERY_FAILED", error))?;
  serde_json::to_string(&result).map_err(|error| {
    napi::Error::from_reason(format!(
      "Could not encode WOML correction recovery: {error}"
    ))
  })
}

#[napi]
pub fn observe_woml_corrections_v1(event_store_path: String) -> napi::Result<String> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_management_error("WOML_CORRECTION_OBSERVATION_FAILED", error))?;
  let observation = store
    .correction_observation_v1()
    .map_err(|error| native_run_management_error("WOML_CORRECTION_OBSERVATION_FAILED", error))?;
  serde_json::to_string(&observation).map_err(|error| {
    napi::Error::from_reason(format!(
      "Could not encode WOML correction observation: {error}"
    ))
  })
}

#[napi]
pub fn inspect_woml_locks_v1(event_store_path: String, run_id: String) -> napi::Result<String> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_management_error("WOML_LOCK_INSPECTION_FAILED", error))?;
  let inspection = store
    .inspect_locks_v1(&run_id)
    .map_err(|error| native_run_management_error("WOML_LOCK_INSPECTION_FAILED", error))?;
  serde_json::to_string(&inspection).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode WOML lock inspection: {error}"))
  })
}

#[napi]
pub fn observe_woml_locks_v1(event_store_path: String) -> napi::Result<String> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_management_error("WOML_LOCK_OBSERVATION_FAILED", error))?;
  let observation = store
    .lock_observation_v1()
    .map_err(|error| native_run_management_error("WOML_LOCK_OBSERVATION_FAILED", error))?;
  serde_json::to_string(&observation).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode WOML lock observation: {error}"))
  })
}

#[napi]
pub fn cancel_woml_run(
  event_store_path: String,
  run_id: String,
  command_id: String,
) -> napi::Result<String> {
  let mut store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_management_error("WOML_RUN_CANCELLATION_FAILED", error))?;
  let result = store
    .request_run_cancellation(&run_id, &command_id, Utc::now())
    .map_err(|error| native_run_management_error("WOML_RUN_CANCELLATION_FAILED", error))?;
  serde_json::to_string(&result).map_err(|error| {
    napi::Error::from_reason(format!(
      "Could not encode WOML cancellation result: {error}"
    ))
  })
}

#[napi]
pub fn recover_woml_runs(event_store_path: String) -> napi::Result<String> {
  let report = recover_durable_runs(PathBuf::from(event_store_path))
    .map_err(|error| napi::Error::from_reason(error.to_string()))?;
  serde_json::to_string(&report)
    .map_err(|error| napi::Error::from_reason(format!("Could not encode recovery report: {error}")))
}

#[napi]
pub fn settle_woml_timer_wait(
  event_store_path: String,
  run_id: String,
  wait_id: String,
) -> napi::Result<String> {
  let mut store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_execution_error(RuntimeExecutionError::DurableStore(error)))?;
  let settlement = match store
    .settle_timer_wait(&run_id, &wait_id, Utc::now())
    .map_err(|error| native_execution_error(RuntimeExecutionError::DurableStore(error)))?
  {
    TimerWaitSettlement::Resolved { .. } => NativeTimerWaitSettlement::Resolved,
    TimerWaitSettlement::AlreadyResolved { .. } => NativeTimerWaitSettlement::AlreadyResolved,
    TimerWaitSettlement::NotDue { deadline } => NativeTimerWaitSettlement::NotDue { deadline },
  };
  serde_json::to_string(&settlement).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode WOML timer settlement: {error}"))
  })
}

#[napi(ts_return_type = "Promise<string>")]
pub async fn run_woml_notification_provider_journey(
  event_store_path: String,
  run_id: String,
  bun_executable: String,
  notification_host_path: String,
  interaction_timeout_ms: u32,
  custom_notification_host_path: Option<String>,
  script_host_path: Option<String>,
  approval_base_url: Option<String>,
  resolved_secrets_json: Option<String>,
) -> napi::Result<String> {
  if interaction_timeout_ms == 0 {
    return Err(napi::Error::from_reason(
      "Notification interaction timeout must be positive.".to_string(),
    ));
  }
  let event_store_path = PathBuf::from(event_store_path);
  let store = DurableEventStore::open(&event_store_path)
    .map_err(|error| native_notification_error(NotificationJourneyError::Store(error)))?;
  let binding = store
    .run_binding(&run_id)
    .map_err(|error| native_notification_error(NotificationJourneyError::Store(error)))?;
  let workflow = store
    .definition(&binding.definition_hash)
    .map_err(|error| native_notification_error(NotificationJourneyError::Store(error)))?;
  let stored_artifacts = store
    .definition_module_artifacts(&binding.definition_hash)
    .map_err(|error| native_notification_error(NotificationJourneyError::Store(error)))?;
  let custom_descriptors: HashMap<&str, &str> = workflow
    .reusable_definitions
    .iter()
    .flatten()
    .filter_map(|definition| match definition {
      CompiledReusableInvocation::NotificationProvider {
        definition_digest,
        script_artifact_id,
        ..
      } => Some((script_artifact_id.as_str(), definition_digest.as_str())),
      _ => None,
    })
    .collect::<HashMap<_, _>>();
  let custom_artifacts = stored_artifacts
    .iter()
    .filter_map(|artifact| {
      let artifact_id = artifact.name.strip_prefix("__woml_provider__")?;
      Some(CustomProviderScriptArtifact {
        script_artifact_id: artifact_id.to_string(),
        definition_digest: custom_descriptors.get(artifact_id)?.to_string(),
        source: artifact.bundle.clone(),
      })
    })
    .collect::<Vec<_>>();
  let has_custom = !custom_descriptors.is_empty();
  let custom = if has_custom {
    let host_script_path = custom_notification_host_path.ok_or_else(|| {
      napi::Error::from_reason("Custom notification provider host path is required.".to_string())
    })?;
    let lifecycle_script_host_path = script_host_path.map(PathBuf::from).unwrap_or_else(|| {
      let host = PathBuf::from(&host_script_path);
      let extension = host
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("js");
      host
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(format!("script-host.{extension}"))
    });
    let approval_base_url = approval_base_url.ok_or_else(|| {
      napi::Error::from_reason("Custom provider approval base URL is required.".to_string())
    })?;
    let resolved_secrets = serde_json::from_str(resolved_secrets_json.as_deref().unwrap_or("{}"))
      .map_err(|_| {
      napi::Error::from_reason("Resolved provider secrets JSON is invalid.".to_string())
    })?;
    Some(CustomNotificationJourneyOptions {
      bun_executable: PathBuf::from(&bun_executable),
      host_script_path: PathBuf::from(host_script_path),
      script_host_path: lifecycle_script_host_path,
      approval_base_url,
      resolved_secrets,
      artifacts: custom_artifacts,
    })
  } else {
    None
  };
  let result = run_notification_provider_journey_with_custom(
    event_store_path,
    &run_id,
    NotificationHostProcessOptions::new(
      PathBuf::from(bun_executable),
      PathBuf::from(notification_host_path),
    ),
    std::time::Duration::from_millis(u64::from(interaction_timeout_ms)),
    custom,
  )
  .await
  .map_err(native_notification_error)?;
  serde_json::to_string(&result).map_err(|error| {
    napi::Error::from_reason(format!(
      "Could not encode notification provider journey: {error}"
    ))
  })
}
