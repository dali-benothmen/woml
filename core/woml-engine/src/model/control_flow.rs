//! model control flow.

use super::{CompiledWorkflowEdge, CompiledWorkflowNode, ValueExpression};
use serde::Deserialize;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
  tag = "kind",
  rename_all = "camelCase",
  rename_all_fields = "camelCase",
  deny_unknown_fields
)]
pub enum CompiledCorrectionIdentity {
  Static {
    action_id: String,
  },
  ForEachIteration {
    for_each_id: String,
    action_id_template: String,
  },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledCorrectiveAction {
  pub action_id: String,
  pub node: CompiledWorkflowNode,
  pub identity: CompiledCorrectionIdentity,
  pub visible_step_ids: Vec<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub for_step_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledSaga {
  pub saga_id: String,
  pub open_node_id: String,
  pub commit_node_id: String,
  pub result_node_id: String,
  pub body_entry_node_id: String,
  pub body_terminal_node_id: String,
  pub body_result_node_id: String,
  pub outer_step_ids: Vec<String>,
  pub compensations: Vec<CompiledCorrectiveAction>,
  pub cleanup: Vec<CompiledCorrectiveAction>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub metadata: Option<Map<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledLock {
  pub lock_id: String,
  pub acquire_node_id: String,
  pub release_node_id: String,
  pub result_node_id: String,
  pub body_entry_node_id: String,
  pub body_terminal_node_id: String,
  pub key: ValueExpression,
  pub acquire_timeout_ms: u64,
  pub lease_ms: u64,
  pub outer_step_ids: Vec<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub metadata: Option<Map<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
  tag = "kind",
  rename_all = "camelCase",
  rename_all_fields = "camelCase",
  deny_unknown_fields
)]
pub enum CompiledWait {
  Duration {
    wait_id: String,
    node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    metadata: Option<Map<String, Value>>,
    duration_ms: u64,
  },
  Until {
    wait_id: String,
    node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    metadata: Option<Map<String, Value>>,
    at: ValueExpression,
  },
  Signal {
    wait_id: String,
    node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    metadata: Option<Map<String, Value>>,
    signals: Vec<CompiledWaitSignal>,
    correlation: ValueExpression,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_ms: Option<u64>,
    on_timeout: String,
  },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledWaitSignal {
  pub signal: String,
  pub schema: Value,
}

impl CompiledWait {
  pub fn wait_id(&self) -> &str {
    match self {
      Self::Duration { wait_id, .. }
      | Self::Until { wait_id, .. }
      | Self::Signal { wait_id, .. } => wait_id,
    }
  }

  pub fn node_id(&self) -> &str {
    match self {
      Self::Duration { node_id, .. }
      | Self::Until { node_id, .. }
      | Self::Signal { node_id, .. } => node_id,
    }
  }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledForEachBody {
  pub entry_node_ids: Vec<String>,
  pub nodes: Vec<CompiledWorkflowNode>,
  pub edges: Vec<CompiledWorkflowEdge>,
  pub choices: Vec<CompiledControlChoice>,
  pub context_visibility: Vec<CompiledContextVisibility>,
  pub terminal_node_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledForEach {
  pub for_each_id: String,
  pub open_node_id: String,
  pub result_node_id: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub metadata: Option<Map<String, Value>>,
  pub items: ValueExpression,
  pub concurrency: u32,
  pub outer_step_ids: Vec<String>,
  pub body: CompiledForEachBody,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub result: Option<ValueExpression>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledForkBranch {
  pub branch_id: String,
  pub entry_node_id: String,
  pub terminal_node_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledFork {
  pub fork_id: String,
  pub open_node_id: String,
  pub join_node_id: String,
  pub branches: Vec<CompiledForkBranch>,
  pub joined_branch_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledControlChoice {
  pub choice_id: String,
  pub selector_node_id: String,
  pub join_node_id: String,
  pub arm_ids: Vec<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub string_selector: Option<ValueExpression>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub string_cases: Option<Vec<CompiledStringChoiceCase>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub default_arm_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub result_node_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledStringChoiceCase {
  pub arm_id: String,
  pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledContextVisibility {
  pub node_id: String,
  pub step_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledWorkflowSettlement {
  pub node_id: String,
  pub main_result_node_id: String,
  pub owned_branch_terminal_node_ids: Vec<String>,
}
