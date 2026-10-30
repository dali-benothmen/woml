//! Private bridge errors.

use serde::Serialize;
use woml_engine::BackupError;
use woml_engine::DurableStoreError;
use woml_engine::NotificationHostClientError;
use woml_engine::NotificationJourneyDiagnostics;
use woml_engine::NotificationJourneyError;
use woml_engine::ParallelFailurePolicy;
use woml_engine::RetentionError;
use woml_engine::RunFailure;
use woml_engine::RunPresentationError;
use woml_engine::RuntimeExecutionError;
use woml_engine::WebhookRuntimeError;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NativeParallelExecutionErrorDetails {
  pub(super) parallel_id: String,
  pub(super) policy: ParallelFailurePolicy,
  pub(super) primary_node_id: String,
  pub(super) failed_node_ids: Vec<String>,
  pub(super) cancelled_node_ids: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NativeExecutionError {
  pub(super) kind: &'static str,
  pub(super) code: String,
  pub(super) message: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub(super) node_id: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub(super) branch_id: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub(super) arm_id: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub(super) reference_path: Option<Vec<String>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub(super) branch_site: Option<&'static str>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub(super) approval_id: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub(super) request_id: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub(super) attempt: Option<u32>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub(super) max_attempts: Option<u32>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub(super) failure_code: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub(super) details: Option<NativeParallelExecutionErrorDetails>,
}

pub(super) fn native_execution_error(error: RuntimeExecutionError) -> napi::Error {
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
#[serde(rename_all = "camelCase")]
pub(super) struct NativeApprovalError {
  pub(super) kind: &'static str,
  pub(super) code: &'static str,
  pub(super) message: &'static str,
}

pub(super) fn native_approval_error(error: RuntimeExecutionError) -> napi::Error {
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
pub(super) struct NativeNotificationError {
  pub(super) kind: &'static str,
  pub(super) code: &'static str,
  pub(super) message: &'static str,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub(super) diagnostics: Option<NotificationJourneyDiagnostics>,
}

pub(super) fn native_notification_error(error: NotificationJourneyError) -> napi::Error {
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NativeTriggerRuntimeError {
  pub(super) kind: &'static str,
  pub(super) code: &'static str,
  pub(super) message: String,
}

pub(super) fn native_trigger_runtime_error(
  error: WebhookRuntimeError,
) -> NativeTriggerRuntimeError {
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

pub(super) fn trigger_runtime_napi_error(error: NativeTriggerRuntimeError) -> napi::Error {
  let reason = serde_json::to_string(&error)
    .unwrap_or_else(|_| "WOML trigger runtime failed to start.".to_string());
  napi::Error::from_reason(reason)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NativeRunInspectionError {
  pub(super) kind: &'static str,
  pub(super) code: &'static str,
  pub(super) message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NativeRunManagementError {
  pub(super) kind: &'static str,
  pub(super) code: &'static str,
  pub(super) message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NativeBackupError {
  pub(super) kind: &'static str,
  pub(super) code: &'static str,
  pub(super) message: String,
}

pub(super) fn native_backup_error(error: BackupError, fallback: &'static str) -> napi::Error {
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

pub(super) fn native_retention_error(error: RetentionError) -> napi::Error {
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

pub(super) fn native_run_management_error(
  code: &'static str,
  error: DurableStoreError,
) -> napi::Error {
  let reason = serde_json::to_string(&NativeRunManagementError {
    kind: "woml_run_management_error",
    code,
    message: error.to_string(),
  })
  .unwrap_or_else(|_| "WOML run management failed.".to_string());
  napi::Error::from_reason(reason)
}

pub(super) fn native_run_presentation_error(error: RunPresentationError) -> napi::Error {
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

pub(super) fn native_run_inspection_error(error: DurableStoreError) -> napi::Error {
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

pub(super) fn run_failure_code(failure: &RunFailure) -> String {
  match failure {
    RunFailure::Attempt(failure) => failure.code.clone(),
    RunFailure::Branch(failure) => failure.code().to_string(),
    RunFailure::Parallel { failure, .. } => failure.code.clone(),
    RunFailure::Approval { failure, .. } => failure.code.clone(),
    RunFailure::Notification { failure, .. } => failure.code.clone(),
  }
}

pub(super) fn operational_output_napi_error(
  error: woml_engine::OperationalOutputError,
) -> napi::Error {
  napi::Error::from_reason(format!("{}: {error}", error.code()))
}
