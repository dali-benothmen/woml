//! model lifecycle.

use super::{ScriptRuntimeBindings, ValueExpression};
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledLifecycleDefinition {
  pub profile_version: u32,
  pub hooks: Vec<CompiledLifecycleHook>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledLifecycleHook {
  pub hook_id: String,
  pub event: LifecycleEventName,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub step_ids: Option<Vec<String>>,
  pub actions: Vec<CompiledLifecycleAction>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleEventName {
  RunStart,
  StepStart,
  StepSuccess,
  StepFailure,
  StepComplete,
  RunSuccess,
  RunFailure,
  RunCancel,
  RunComplete,
}

impl LifecycleEventName {
  pub const fn is_step(self) -> bool {
    matches!(
      self,
      Self::StepStart | Self::StepSuccess | Self::StepFailure | Self::StepComplete
    )
  }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledLifecycleAction {
  pub action_id: String,
  pub handler: String,
  pub inputs: ValueExpression,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub script_runtime: Option<ScriptRuntimeBindings>,
}
