//! Minimal native boundary for the WOML Rust execution path.

use std::path::PathBuf;

use napi_derive::napi;
use serde::Serialize;
use serde_json::{Map, Value};
use woml_engine::{
  execute_workflow, execute_workflow_durable, execute_workflow_durable_outcome,
  recover_durable_runs, resolve_human_approval_durable, resume_workflow_durable_outcome,
  run_notification_provider_journey, settle_approval_timeout_durable, ApprovalDecision,
  ApprovalDecisionOutcome, CompiledWorkflowDefinition, DurableEventStore, DurableStoreError,
  NotificationHostClientError, NotificationHostProcessOptions, NotificationJourneyError,
  NotificationJourneyDiagnostics,
  ParallelFailurePolicy, RunEventPayload, RunFailedData, RunFailedDataV2, RunFailedDataV3,
  RuntimeExecutionError, RuntimeExecutionOptions, ScriptHostProcessOptions, SystemEngineClock,
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
  details: Option<NativeParallelExecutionErrorDetails>,
}

fn native_execution_error(error: RuntimeExecutionError) -> napi::Error {
  let envelope = match error {
    RuntimeExecutionError::RunFailed(details) => {
      let node_id = details.events.iter().rev().find_map(|event| {
        if let RunEventPayload::RunFailed(data) = &event.payload {
          match data {
            RunFailedData::V1(data) => data.node_id.clone(),
            RunFailedData::V2(RunFailedDataV2::Attempt { node_id, .. }) => Some(node_id.clone()),
            RunFailedData::V2(RunFailedDataV2::Branch { .. }) => None,
            RunFailedData::V3(RunFailedDataV3::Parallel {
              primary_node_id, ..
            }) => Some(primary_node_id.clone()),
            RunFailedData::V4(_) => None,
            RunFailedData::V5(_) => None,
          }
        } else {
          None
        }
      });
      NativeExecutionError {
        kind: "woml_execution_error",
        code: details.code.clone(),
        message: details.message.clone(),
        node_id,
        branch_id: None,
        arm_id: None,
        reference_path: None,
        branch_site: None,
        approval_id: None,
        request_id: None,
        details: None,
      }
    }
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

#[napi(ts_return_type = "Promise<string>")]
pub async fn execute_woml_workflow(
  compiled_model_json: String,
  definition_hash: String,
  trigger_json: String,
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
) -> napi::Result<String> {
  let workflow: CompiledWorkflowDefinition =
    serde_json::from_str(&compiled_model_json).map_err(|error| {
      napi::Error::from_reason(format!("Invalid compiled workflow JSON: {error}"))
    })?;
  let trigger: Map<String, Value> = serde_json::from_str(&trigger_json)
    .map_err(|error| napi::Error::from_reason(format!("Invalid trigger JSON: {error}")))?;
  let host_options = ScriptHostProcessOptions::new(
    PathBuf::from(bun_executable),
    PathBuf::from(script_host_path),
  );
  let options = RuntimeExecutionOptions::new(host_options, u64::from(script_timeout_ms));
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
) -> napi::Result<String> {
  let workflow: CompiledWorkflowDefinition =
    serde_json::from_str(&compiled_model_json).map_err(|error| {
      napi::Error::from_reason(format!("Invalid compiled workflow JSON: {error}"))
    })?;
  let trigger: Map<String, Value> = serde_json::from_str(&trigger_json)
    .map_err(|error| napi::Error::from_reason(format!("Invalid trigger JSON: {error}")))?;
  let host_options = ScriptHostProcessOptions::new(
    PathBuf::from(bun_executable),
    PathBuf::from(script_host_path),
  );
  let options = RuntimeExecutionOptions::new(host_options, u64::from(script_timeout_ms));
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
pub async fn execute_woml_workflow_durable_outcome(
  compiled_model_json: String,
  definition_hash: String,
  trigger_json: String,
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  event_store_path: String,
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
    runtime_options(bun_executable, script_host_path, script_timeout_ms),
    PathBuf::from(event_store_path),
  )
  .await
  .map_err(native_execution_error)?;
  serde_json::to_string(&outcome).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode WOML runtime outcome: {error}"))
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
    runtime_options(bun_executable, script_host_path, script_timeout_ms),
  )
  .await
  .map_err(native_execution_error)?;
  serde_json::to_string(&outcome).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode WOML runtime outcome: {error}"))
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
