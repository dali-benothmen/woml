//! model contracts.

use super::{
  CompiledContextVisibility, CompiledControlChoice, CompiledForEach, CompiledFork,
  CompiledLifecycleDefinition, CompiledLock, CompiledReusableInvocation, CompiledRuntimePolicy,
  CompiledSaga, CompiledWait, CompiledWorkflowSettlement, EdgeCondition, RetryPolicy,
  ValueExpression,
};
use serde::Deserialize;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledWorkflowDefinition {
  pub schema_version: u32,
  pub workflow_id: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub metadata: Option<CompiledWorkflowMetadata>,
  pub triggers: Vec<CompiledTrigger>,
  pub graph: CompiledWorkflowGraph,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub module_runtime: Option<CompiledModuleRuntime>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub lifecycle: Option<CompiledLifecycleDefinition>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub runtime_policy: Option<CompiledRuntimePolicy>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub reusable_definitions: Option<Vec<CompiledReusableInvocation>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub communication: Option<CompiledCommunicationRequirements>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledCommunicationRequirements {
  pub profile_version: u32,
  pub providers: Vec<CompiledCommunicationProviderRequirement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledCommunicationProviderRequirement {
  pub provider: String,
  pub trigger_ids: Vec<String>,
  pub notification_delivery_ids: Vec<String>,
  pub messaging: bool,
  pub credential_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledModuleRuntime {
  pub profile_version: u32,
  pub modules: Vec<CompiledModuleBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledModuleBinding {
  pub name: String,
  pub bundle_digest: String,
  pub source_map_digest: String,
  pub exports: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledWorkflowMetadata {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub name: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub description: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub version: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub labels: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledTrigger {
  pub id: String,
  pub handler: String,
  pub config: ValueExpression,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledWorkflowGraph {
  pub entry_node_ids: Vec<String>,
  pub nodes: Vec<CompiledWorkflowNode>,
  pub edges: Vec<CompiledWorkflowEdge>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub forks: Option<Vec<CompiledFork>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub choices: Option<Vec<CompiledControlChoice>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub context_visibility: Option<Vec<CompiledContextVisibility>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub settlement: Option<CompiledWorkflowSettlement>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub for_each: Option<Vec<CompiledForEach>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub waits: Option<Vec<CompiledWait>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub locks: Option<Vec<CompiledLock>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub sagas: Option<Vec<CompiledSaga>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledWorkflowNode {
  pub id: String,
  pub handler: String,
  pub inputs: ValueExpression,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub timeout_ms: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub retry_policy: Option<RetryPolicy>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub script_runtime: Option<ScriptRuntimeBindings>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub execution: Option<CompiledExecutionPlacement>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub metadata: Option<Map<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledExecutionPlacement {
  pub profile_version: u32,
  pub target: String,
  pub required_capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScriptRuntimeBindings {
  pub binding_version: u32,
  pub bindings: Vec<String>,
  pub required_secrets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledWorkflowEdge {
  pub id: String,
  pub from: String,
  pub to: String,
  pub condition: EdgeCondition,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub branch_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub parallel_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub approval_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParallelGroupDefinition {
  pub parallel_id: String,
  pub start_node_id: String,
  pub child_node_ids: Vec<String>,
  pub concurrency: usize,
  pub on_error: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ApprovalDefinition {
  pub approval_id: String,
  pub name: Option<String>,
  pub description: Option<String>,
  pub timeout_ms: Option<u64>,
  pub on_timeout: String,
  pub review: Option<ValueExpression>,
  pub required_approvals: Option<u32>,
  pub required_roles: Vec<String>,
  pub notifications: Vec<NotificationDefinition>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NotificationDefinition {
  pub delivery_id: String,
  pub provider: String,
  pub destination: String,
  pub credentials: BTreeMap<String, String>,
  pub provider_id: Option<String>,
  pub message: Option<ValueExpression>,
  pub template_name: Option<String>,
  pub language: Option<String>,
}
