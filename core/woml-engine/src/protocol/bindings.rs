//! protocol bindings.

use crate::WorkflowContext;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptExecutionContext<'a> {
  pub trigger: &'a serde_json::Map<String, Value>,
  pub steps: &'a serde_json::Map<String, Value>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub lock: Option<&'a crate::LockContext>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub item: Option<&'a Value>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub iteration: Option<ScriptIterationBinding>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub compensation: Option<&'a CompensationBindingV1>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub cleanup: Option<&'a CleanupBindingV1>,
}

impl<'a> ScriptExecutionContext<'a> {
  pub(super) fn root(context: &'a WorkflowContext) -> Self {
    Self {
      trigger: &context.trigger,
      steps: &context.steps,
      lock: context.lock.as_ref(),
      item: None,
      iteration: None,
      compensation: None,
      cleanup: None,
    }
  }

  pub(super) fn iteration(
    context: &'a WorkflowContext,
    item: &'a Value,
    index: u32,
    total: u32,
  ) -> Self {
    Self {
      trigger: &context.trigger,
      steps: &context.steps,
      lock: context.lock.as_ref(),
      item: Some(item),
      iteration: Some(ScriptIterationBinding { index, total }),
      compensation: None,
      cleanup: None,
    }
  }

  pub(super) fn correction(
    context: &'a WorkflowContext,
    binding: CorrectionBindingRef<'a>,
  ) -> Self {
    let (compensation, cleanup) = match binding {
      CorrectionBindingRef::Compensation(value) => (Some(value), None),
      CorrectionBindingRef::Cleanup(value) => (None, Some(value)),
    };
    Self {
      trigger: &context.trigger,
      steps: &context.steps,
      lock: context.lock.as_ref(),
      item: None,
      iteration: None,
      compensation,
      cleanup,
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CorrectionReasonV1 {
  Failure,
  Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompensationBindingV1 {
  pub saga_id: String,
  pub for_step_id: String,
  pub reason: CorrectionReasonV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupReasonV1 {
  Success,
  Failure,
  Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompensationStatusV1 {
  NotRequired,
  Completed,
  Incomplete,
  ManualRecovery,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupBindingV1 {
  pub saga_id: String,
  pub reason: CleanupReasonV1,
  pub compensation_status: CompensationStatusV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorrectionBindingRef<'a> {
  Compensation(&'a CompensationBindingV1),
  Cleanup(&'a CleanupBindingV1),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptIterationBinding {
  pub index: u32,
  pub total: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptExecutionMode {
  Step,
  Lifecycle,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReusableScriptBindingV3 {
  pub profile: String,
  pub invocation_id: String,
  pub definition: ReusableDefinitionBindingV3,
  pub props: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReusableDefinitionBindingV3 {
  pub kind: ReusableDefinitionKind,
  pub alias: String,
  pub digest: String,
  pub source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReusableDefinitionKind {
  Step,
  NotificationProvider,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReusableLifecycleBindingV1 {
  pub hook: ReusableLifecycleHook,
  pub outcome: ReusableInvocationOutcome,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub result: Option<Value>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub error: Option<ReusableLifecycleErrorV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReusableLifecycleHook {
  #[serde(rename = "on-success")]
  OnSuccess,
  #[serde(rename = "on-error")]
  OnError,
  #[serde(rename = "on-complete")]
  OnComplete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReusableInvocationOutcome {
  Succeeded,
  Failed,
  Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReusableLifecycleErrorV1 {
  pub code: String,
  pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LifecycleBindingV1 {
  pub event: crate::model::LifecycleEventName,
  pub workflow: LifecycleWorkflowBindingV1,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub step: Option<LifecycleStepBindingV1>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub failure: Option<LifecycleFailureBindingV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LifecycleWorkflowBindingV1 {
  pub id: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub outcome: Option<crate::event::BusinessOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LifecycleStepBindingV1 {
  pub id: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub outcome: Option<crate::event::BusinessOutcome>,
  pub attempts: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LifecycleFailureBindingV1 {
  pub code: String,
  pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptBindings<'a> {
  pub binding_version: u32,
  pub services_version: u32,
  pub secrets: &'a BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptAttempt<'a> {
  pub number: u32,
  pub max_attempts: u32,
  pub idempotency_key: &'a str,
}

impl<'a> ScriptAttempt<'a> {
  pub fn new(number: u32, max_attempts: u32, idempotency_key: &'a str) -> Result<Self, String> {
    if number == 0 || max_attempts == 0 || max_attempts > 10 || number > max_attempts {
      return Err(
        "Script attempt numbers must satisfy 1 <= number <= maxAttempts <= 10.".to_string(),
      );
    }
    if !crate::event::is_definition_hash(idempotency_key) {
      return Err("Script attempts require a canonical sha256 idempotency key.".to_string());
    }
    Ok(Self {
      number,
      max_attempts,
      idempotency_key,
    })
  }
}
