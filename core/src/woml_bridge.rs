//! Minimal native boundary for the WOML Rust execution path.

use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread::JoinHandle;

use chrono::{DateTime, Utc};
use napi::threadsafe_function::{ErrorStrategy, ThreadsafeFunctionCallMode};
use napi::{Env, JsFunction, JsObject};
use napi_derive::napi;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use woml_engine::model::ValueExpression;
use woml_engine::{
  execute_workflow, execute_workflow_durable, execute_workflow_durable_outcome,
  recover_durable_runs, resolve_human_approval_durable, resume_workflow_durable,
  resume_workflow_durable_any_outcome, resume_workflow_durable_outcome,
  run_notification_provider_journey, settle_approval_timeout_durable, ApprovalDecision,
  ApprovalDecisionOutcome, CompiledWorkflowDefinition, DurableEventStore, DurableStoreError,
  ExternalTriggerAdmissionCommand, IntervalProgress, IntervalProgressReporter,
  NotificationHostClientError, NotificationHostProcessOptions, NotificationJourneyDiagnostics,
  NotificationJourneyError, ParallelFailurePolicy, RunFailure, RunStatus, RuntimeExecutionError,
  RuntimeExecutionOptions, RuntimeModuleArtifact, ScheduleProgress, ScheduleProgressReporter,
  ScriptHostProcessOptions, SystemEngineClock, TriggerAdmissionRequest, TriggerProgress,
  TriggerProgressReporter, WebhookDefinitionRegistration, WebhookRuntimeError, WomlWebhookServer,
  WomlWebhookServerConfig,
};

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

struct NativeWebhookRuntimeThread {
  stop: mpsc::Sender<()>,
  ingress: tokio::sync::mpsc::UnboundedSender<ExternalTriggerAdmissionCommand>,
  join: JoinHandle<()>,
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

static WEBHOOK_RUNTIMES: OnceLock<Mutex<HashMap<String, NativeWebhookRuntimeThread>>> =
  OnceLock::new();

fn webhook_runtimes() -> &'static Mutex<HashMap<String, NativeWebhookRuntimeThread>> {
  WEBHOOK_RUNTIMES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn native_trigger_runtime_error(error: WebhookRuntimeError) -> NativeTriggerRuntimeError {
  let code = match error {
    WebhookRuntimeError::RouteConflict(_) => "WOML_WEBHOOK_ROUTE_CONFLICT",
    WebhookRuntimeError::SecretMissing(_) => "WOML_WEBHOOK_SECRET_MISSING",
    WebhookRuntimeError::InvalidSchema { .. } => "WOML_TRIGGER_SCHEMA_INVALID",
    WebhookRuntimeError::DurableStore(DurableStoreError::WorkflowRuntimeDuplicateOwner(_)) => {
      "WOML_WORKFLOW_TARGET_AMBIGUOUS"
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

fn native_runtime_progress_reporters(
  env: &Env,
  progress_callback: JsFunction,
) -> napi::Result<(
  TriggerProgressReporter,
  ScheduleProgressReporter,
  IntervalProgressReporter,
)> {
  let mut progress = progress_callback
    .create_threadsafe_function::<String, String, _, ErrorStrategy::Fatal>(0, |context| {
      Ok(vec![context.value])
    })?;
  progress.unref(env)?;
  let progress = Arc::new(progress);
  let trigger_progress = progress.clone();
  let schedule_progress = progress.clone();
  let interval_progress = progress;
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
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeRunInspectionError {
  kind: &'static str,
  code: &'static str,
  message: String,
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

fn runtime_options_with_modules(
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  resolved_secrets_json: String,
  runtime_modules_json: Option<String>,
) -> napi::Result<RuntimeExecutionOptions> {
  let modules: Vec<RuntimeModuleArtifact> =
    serde_json::from_str(runtime_modules_json.as_deref().unwrap_or("[]")).map_err(|error| {
      napi::Error::from_reason(format!("Invalid runtime modules JSON: {error}"))
    })?;
  Ok(
    runtime_options_with_secrets(
      bun_executable,
      script_host_path,
      script_timeout_ms,
      resolved_secrets_json,
    )?
    .with_runtime_modules(modules),
  )
}

fn runtime_options_with_progress(
  env: &Env,
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  progress_callback: JsFunction,
  resolved_secrets_json: String,
) -> napi::Result<RuntimeExecutionOptions> {
  let mut progress = progress_callback
    .create_threadsafe_function::<String, String, _, ErrorStrategy::Fatal>(0, |context| {
      Ok(vec![context.value])
    })?;
  progress.unref(env)?;
  Ok(
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
    })),
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
  )?;
  let result = execute_workflow_durable(
    workflow,
    definition_hash,
    trigger,
    options,
    PathBuf::from(event_store_path),
  )
  .await
  .map_err(native_execution_error)?;
  serde_json::to_string(&result)
    .map_err(|error| napi::Error::from_reason(format!("Could not encode WOML result: {error}")))
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
  )?;
  let modules: Vec<RuntimeModuleArtifact> =
    serde_json::from_str(runtime_modules_json.as_deref().unwrap_or("[]")).map_err(|error| {
      napi::Error::from_reason(format!("Invalid runtime modules JSON: {error}"))
    })?;
  let options = options.with_runtime_modules(modules);
  env.spawn_future(async move {
    let result = execute_workflow_durable(
      workflow,
      definition_hash,
      trigger,
      options,
      PathBuf::from(event_store_path),
    )
    .await
    .map_err(native_execution_error)?;
    serde_json::to_string(&result)
      .map_err(|error| napi::Error::from_reason(format!("Could not encode WOML result: {error}")))
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
) -> napi::Result<JsObject> {
  let options = runtime_options_with_progress(
    &env,
    bun_executable,
    script_host_path,
    script_timeout_ms,
    progress_callback,
    resolved_secrets_json,
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
  progress_callback: JsFunction,
) -> napi::Result<JsObject> {
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
  let (progress_reporter, schedule_progress_reporter, interval_progress_reporter) =
    native_runtime_progress_reporters(&env, progress_callback)?;
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
  let registrations = registrations
    .into_iter()
    .map(|registration| WebhookDefinitionRegistration {
      workflow: registration.workflow,
      definition_hash: registration.definition_hash,
      resolved_secrets: registration.resolved_secrets,
      runtime_modules: registration.runtime_modules,
    })
    .collect();
  let config = WomlWebhookServerConfig {
    bind_address,
    database_path: PathBuf::from(event_store_path),
    registrations,
    startup_manual_triggers,
    execution: runtime_options(bun_executable, script_host_path, script_timeout_ms)
      .with_resolved_secrets(runtime_secrets)
      .with_schedule_progress_reporter(schedule_progress_reporter)
      .with_interval_progress_reporter(interval_progress_reporter),
    progress_reporter: Some(progress_reporter),
  };

  env.spawn_future(async move {
    let runtime_id = format!("runtime_{}", uuid::Uuid::new_v4().simple());
    let (startup_sender, startup_receiver) =
      mpsc::sync_channel::<Result<SocketAddr, NativeTriggerRuntimeError>>(1);
    let (stop_sender, stop_receiver) = mpsc::channel::<()>();
    let (ingress_sender, ingress_receiver) = tokio::sync::mpsc::unbounded_channel();
    let join = std::thread::Builder::new()
      .name(format!("woml-trigger-{runtime_id}"))
      .spawn(move || {
        actix_web::rt::System::new().block_on(async move {
          match WomlWebhookServer::start_with_external_ingress(config, Some(ingress_receiver)).await
          {
            Ok(server) => {
              let address = server.local_address();
              if startup_sender.send(Ok(address)).is_err() {
                server.stop().await;
                return;
              }
              let _ = actix_web::rt::task::spawn_blocking(move || stop_receiver.recv()).await;
              server.stop().await;
            }
            Err(error) => {
              let _ = startup_sender.send(Err(native_trigger_runtime_error(error)));
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
          stop: stop_sender,
          ingress: ingress_sender,
          join,
        },
      );
    serde_json::to_string(&NativeWebhookRuntimeStarted {
      runtime_id,
      host: address.ip().to_string(),
      port: address.port(),
    })
    .map_err(|error| {
      napi::Error::from_reason(format!("Could not encode webhook runtime startup: {error}"))
    })
  })
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
    || ingress.trigger_handler != "trigger.slack"
  {
    return Err(napi::Error::from_reason(
      "Invalid Slack trigger ingress contract.".to_string(),
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
    Err(_) => serde_json::to_string(&NativeTriggerIngressRejected {
      contract: "woml.trigger-ingress",
      contract_version: 1,
      message_type: "rejected",
      request_id,
      failure: NativeTriggerIngressFailure {
        code: "WOML_TRIGGER_UNAVAILABLE",
        message: "The durable WOML trigger authority is unavailable.",
        retryable: true,
      },
    }),
  };
  json.map_err(|error| {
    napi::Error::from_reason(format!("Could not encode trigger ingress outcome: {error}"))
  })
}

#[napi(ts_return_type = "Promise<void>")]
pub async fn stop_woml_webhook_runtime(runtime_id: String) -> napi::Result<()> {
  let runtime = webhook_runtimes()
    .lock()
    .map_err(|_| napi::Error::from_reason("Webhook runtime registry is unavailable.".to_string()))?
    .remove(&runtime_id)
    .ok_or_else(|| napi::Error::from_reason("WOML webhook runtime does not exist.".to_string()))?;
  drop(runtime.ingress);
  let _ = runtime.stop.send(());
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
  let inspection = NativeRunInspection {
    run_id: projection.run_id.unwrap_or(run_id),
    workflow_id: projection.workflow_id.unwrap_or_default(),
    status: projection.status,
    terminal_node_id: projection.terminal_node_id,
    result: projection.result,
    failure_code: projection.failure.as_ref().map(run_failure_code),
  };
  serde_json::to_string(&inspection)
    .map_err(|error| napi::Error::from_reason(format!("Could not encode WOML run: {error}")))
}

#[napi]
pub fn recover_woml_runs(event_store_path: String) -> napi::Result<String> {
  let report = recover_durable_runs(PathBuf::from(event_store_path))
    .map_err(|error| napi::Error::from_reason(error.to_string()))?;
  serde_json::to_string(&report)
    .map_err(|error| napi::Error::from_reason(format!("Could not encode recovery report: {error}")))
}

#[napi(ts_return_type = "Promise<string>")]
pub async fn run_woml_notification_provider_journey(
  event_store_path: String,
  run_id: String,
  bun_executable: String,
  notification_host_path: String,
  interaction_timeout_ms: u32,
) -> napi::Result<String> {
  if interaction_timeout_ms == 0 {
    return Err(napi::Error::from_reason(
      "Notification interaction timeout must be positive.".to_string(),
    ));
  }
  let result = run_notification_provider_journey(
    PathBuf::from(event_store_path),
    &run_id,
    NotificationHostProcessOptions::new(
      PathBuf::from(bun_executable),
      PathBuf::from(notification_host_path),
    ),
    std::time::Duration::from_millis(u64::from(interaction_timeout_ms)),
  )
  .await
  .map_err(native_notification_error)?;
  serde_json::to_string(&result).map_err(|error| {
    napi::Error::from_reason(format!(
      "Could not encode notification provider journey: {error}"
    ))
  })
}
