//! event lifecycle.

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleSubjectKind {
  Workflow,
  Step,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LifecycleSubject {
  pub kind: LifecycleSubjectKind,
  pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LifecycleHookRequestedData {
  pub hook_invocation_id: String,
  pub hook_id: String,
  pub event: crate::model::LifecycleEventName,
  pub subject: LifecycleSubject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LifecycleActionIdentityData {
  pub hook_invocation_id: String,
  pub action_id: String,
  pub attempt: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleFailureKind {
  ScriptThrew,
  TimedOut,
  NonJson,
  WorkerCrashed,
  HostCrashed,
  Interrupted,
  SizeLimitExceeded,
  Cancelled,
  ProviderFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleFailure {
  pub kind: LifecycleFailureKind,
  pub code: String,
  pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LifecycleActionFailedData {
  pub hook_invocation_id: String,
  pub action_id: String,
  pub attempt: u32,
  pub failure: LifecycleFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleHookCompletionStatus {
  Completed,
  CompletedWithWarnings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LifecycleHookCompletedData {
  pub hook_invocation_id: String,
  pub status: LifecycleHookCompletionStatus,
  pub failed_actions: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReusableLifecycleHook {
  OnSuccess,
  OnError,
  OnComplete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReusableLifecycleOutcome {
  Succeeded,
  Failed,
  CompletedWithWarnings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReusableLifecycleRequestedData {
  pub invocation_id: String,
  pub definition_digest: String,
  pub hook: ReusableLifecycleHook,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReusableLifecycleActionStartedData {
  pub invocation_id: String,
  pub definition_digest: String,
  pub hook: ReusableLifecycleHook,
  pub action_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReusableLifecycleActionSucceededData {
  pub invocation_id: String,
  pub definition_digest: String,
  pub hook: ReusableLifecycleHook,
  pub action_id: String,
  pub outcome: ReusableLifecycleOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReusableLifecycleActionFailedData {
  pub invocation_id: String,
  pub definition_digest: String,
  pub hook: ReusableLifecycleHook,
  pub action_id: String,
  pub outcome: ReusableLifecycleOutcome,
  pub warning_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReusableLifecycleCompletedData {
  pub invocation_id: String,
  pub definition_digest: String,
  pub hook: ReusableLifecycleHook,
  pub outcome: ReusableLifecycleOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BusinessOutcome {
  Succeeded,
  Failed,
  Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunOutcomeDecidedData {
  Succeeded {
    result: Value,
  },
  Failed {
    failure: LifecycleFailure,
  },
  Cancelled {
    #[serde(rename = "cancellationRequestId")]
    cancellation_request_id: String,
  },
}

impl RunOutcomeDecidedData {
  pub const fn outcome(&self) -> BusinessOutcome {
    match self {
      Self::Succeeded { .. } => BusinessOutcome::Succeeded,
      Self::Failed { .. } => BusinessOutcome::Failed,
      Self::Cancelled { .. } => BusinessOutcome::Cancelled,
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinalLifecycleStatus {
  Completed,
  CompletedWithWarnings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LifecycleWarning {
  pub hook_id: String,
  pub action_id: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub step_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub provider: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub destination: Option<String>,
  pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunFinalizedData {
  pub outcome: BusinessOutcome,
  pub lifecycle_status: FinalLifecycleStatus,
  pub warnings: Vec<LifecycleWarning>,
}
