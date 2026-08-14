use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use serde_json_canonicalizer::to_vec as canonical_json;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
  COMPILED_MODEL_SCHEMA_VERSION_V1, COMPILED_MODEL_SCHEMA_VERSION_V10,
  COMPILED_MODEL_SCHEMA_VERSION_V11, COMPILED_MODEL_SCHEMA_VERSION_V12,
  COMPILED_MODEL_SCHEMA_VERSION_V13, COMPILED_MODEL_SCHEMA_VERSION_V14,
  COMPILED_MODEL_SCHEMA_VERSION_V2, COMPILED_MODEL_SCHEMA_VERSION_V3,
  COMPILED_MODEL_SCHEMA_VERSION_V4, COMPILED_MODEL_SCHEMA_VERSION_V5,
  COMPILED_MODEL_SCHEMA_VERSION_V6, COMPILED_MODEL_SCHEMA_VERSION_V7,
  COMPILED_MODEL_SCHEMA_VERSION_V8, COMPILED_MODEL_SCHEMA_VERSION_V9,
};

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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledRuntimePolicy {
  pub profile_version: u32,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub concurrency: Option<u32>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub timeout_ms: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub rate_limit: Option<CompiledRateLimitPolicy>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub queue: Option<CompiledQueuePolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledRateLimitPolicy {
  pub count: u32,
  pub window_ms: u64,
  pub algorithm: RateLimitAlgorithm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitAlgorithm {
  RollingWindow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledQueuePolicy {
  pub name: String,
  pub discipline: QueueDiscipline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueDiscipline {
  WorkConservingFifo,
}

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
  pub metadata: Option<Map<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScriptRuntimeBindings {
  pub binding_version: u32,
  pub bindings: Vec<String>,
  pub required_secrets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum CompiledReusableInvocation {
  Step {
    #[serde(rename = "invocationId")]
    invocation_id: String,
    #[serde(rename = "nodeId")]
    node_id: String,
    alias: String,
    #[serde(rename = "definitionDigest")]
    definition_digest: String,
    source: String,
    #[serde(rename = "scriptArtifactId")]
    script_artifact_id: String,
    props: Vec<CompiledReusableBoundProp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lifecycle: Option<CompiledReusableLifecycle>,
  },
  NotificationProvider {
    #[serde(rename = "providerId")]
    provider_id: String,
    alias: String,
    #[serde(rename = "definitionDigest")]
    definition_digest: String,
    source: String,
    #[serde(rename = "scriptArtifactId")]
    script_artifact_id: String,
    props: Vec<CompiledReusableBoundProp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lifecycle: Option<CompiledReusableLifecycle>,
  },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledReusableBoundProp {
  pub name: String,
  pub binding_name: String,
  pub secret: bool,
  pub expression: CompiledReusablePropExpression,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum CompiledReusablePropExpression {
  Literal { value: String },
  Context { path: String },
  Secret { name: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledReusableLifecycle {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub on_success: Option<Vec<String>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub on_error: Option<Vec<String>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub on_complete: Option<Vec<String>>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ValueExpression {
  #[serde(rename = "literal")]
  Literal { value: Value },
  #[serde(rename = "contextReference")]
  ContextReference { path: Vec<String> },
  #[serde(rename = "lifecycleReference")]
  LifecycleReference { path: Vec<String> },
  #[serde(rename = "secretReference")]
  SecretReference { name: String },
  #[serde(rename = "object")]
  Object {
    fields: BTreeMap<String, ValueExpression>,
  },
  #[serde(rename = "array")]
  Array { items: Vec<ValueExpression> },
  #[serde(rename = "template")]
  Template { parts: Vec<TemplatePart> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum TemplatePart {
  #[serde(rename = "text")]
  Text { text: String },
  #[serde(rename = "contextReference")]
  ContextReference { path: Vec<String> },
  #[serde(rename = "lifecycleReference")]
  LifecycleReference { path: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum EdgeCondition {
  #[serde(rename = "always")]
  Always,
  #[serde(rename = "boolean")]
  Boolean { value: ValueExpression },
  #[serde(rename = "truthy")]
  Truthy { value: ValueExpression },
  #[serde(rename = "equals")]
  Equals {
    left: ValueExpression,
    right: ValueExpression,
  },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetryPolicy {
  pub max_attempts: u32,
  pub backoff: BackoffPolicy,
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum BackoffPolicy {
  None,
  Fixed {
    #[serde(rename = "delayMs")]
    delay_ms: u64,
  },
  Exponential {
    #[serde(rename = "initialDelayMs")]
    initial_delay_ms: u64,
    multiplier: f64,
    #[serde(rename = "maximumDelayMs")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    maximum_delay_ms: Option<u64>,
  },
}

impl RetryPolicy {
  pub const MAX_ATTEMPTS: u32 = 10;
  pub const MAX_DELAY_MS: u64 = 86_400_000;

  pub fn delay_before_attempt(&self, next_attempt: u32) -> Option<u64> {
    if next_attempt < 2 || next_attempt > self.max_attempts {
      return None;
    }
    match self.backoff {
      BackoffPolicy::None => None,
      BackoffPolicy::Fixed { delay_ms } => Some(delay_ms),
      BackoffPolicy::Exponential {
        initial_delay_ms,
        multiplier,
        maximum_delay_ms,
      } => {
        let exponent = i32::try_from(next_attempt - 2).ok()?;
        let calculated = (initial_delay_ms as f64) * multiplier.powi(exponent);
        let capped = calculated.min(maximum_delay_ms? as f64);
        if !capped.is_finite() || capped < 1.0 || capped > u64::MAX as f64 {
          return None;
        }
        Some(capped as u64)
      }
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ModelIssueCode {
  UnsupportedSchemaVersion,
  InvalidIdentifier,
  InvalidMetadata,
  MissingTrigger,
  DuplicateTriggerId,
  MissingNode,
  MissingEntryNode,
  DuplicateNodeId,
  DuplicateEdgeId,
  DuplicateEntryNodeId,
  UnknownEdgeEndpoint,
  InvalidEntryNode,
  UnreachableNode,
  CyclicGraph,
  TerminalNodeCount,
  UnknownHandler,
  UnsupportedTrigger,
  UnsupportedNodeInputs,
  UnsupportedEdgeCondition,
  UnsupportedBranch,
  UnsupportedRetry,
  UnsupportedTimeout,
  UnsupportedNonSequentialDag,
  InvalidValueExpression,
  InvalidBranchSelector,
  InvalidBranchGroup,
  InvalidBranchResult,
  InvalidParallelGroup,
  UnsupportedParallelExecution,
  InvalidApprovalGroup,
  InvalidNotificationGroup,
  InvalidScriptRuntime,
  InvalidModuleRuntime,
  InvalidLifecycle,
  InvalidRuntimePolicy,
  UnsupportedRuntimePolicyExecution,
  InvalidForkGraph,
  InvalidControlChoice,
  InvalidContextVisibility,
  InvalidWorkflowSettlement,
  UnsupportedForkExecution,
  InvalidReusableDefinition,
  UnsupportedReusableExecution,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelIssue {
  pub code: ModelIssueCode,
  pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("compiled workflow model validation failed: {first_message}")]
pub struct ModelValidationError {
  pub issues: Vec<ModelIssue>,
  first_message: String,
}

impl ModelValidationError {
  fn new(issues: Vec<ModelIssue>) -> Self {
    let first_message = issues
      .first()
      .map(|issue| issue.message.clone())
      .unwrap_or_else(|| "unknown validation error".to_string());
    Self {
      issues,
      first_message,
    }
  }
}

fn issue(code: ModelIssueCode, message: impl Into<String>) -> ModelIssue {
  ModelIssue {
    code,
    message: message.into(),
  }
}

fn valid_id(value: &str) -> bool {
  !value.is_empty() && value.chars().count() <= 256
}

fn valid_public_structural_id(value: &str) -> bool {
  let mut characters = value.chars();
  matches!(characters.next(), Some(first) if first.is_ascii_lowercase())
    && characters.all(|character| character.is_ascii_alphanumeric())
    && value.chars().count() <= 256
}

fn valid_secret_name(value: &str) -> bool {
  let mut characters = value.chars();
  matches!(characters.next(), Some(first) if first.is_ascii_uppercase())
    && characters.all(|character| {
      character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
    })
}

fn valid_reusable_alias(value: &str) -> bool {
  let mut parts = value.split('-');
  parts.clone().all(|part| {
    !part.is_empty()
      && matches!(part.chars().next(), Some(first) if first.is_ascii_lowercase())
      && part
        .chars()
        .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
  }) && parts.next().is_some()
    && value.len() <= 128
}

fn valid_sha256(value: &str) -> bool {
  value.len() == 71
    && value.starts_with("sha256:")
    && value[7..]
      .bytes()
      .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_portable_path(value: &str) -> bool {
  !value.is_empty()
    && value.len() <= 1024
    && !value.starts_with('/')
    && !value.contains('\\')
    && !value.split('/').any(|part| part == ".." || part.is_empty())
}

fn valid_reusable_context_path(path: &str) -> bool {
  let identifiers_valid = |segments: &[&str]| {
    segments.iter().all(|segment| {
      let mut chars = segment.chars();
      matches!(chars.next(), Some(first) if first == '_' || first == '$' || first.is_ascii_alphabetic())
        && chars.all(|character| character == '_' || character == '$' || character.is_ascii_alphanumeric())
    })
  };
  let segments = path.split('.').collect::<Vec<_>>();
  match segments.as_slice() {
    ["payload", rest @ ..] => identifiers_valid(rest),
    ["steps", step, rest @ ..] => valid_public_structural_id(step) && identifiers_valid(rest),
    _ => false,
  }
}

fn reusable_prop_secrets(
  props: &[CompiledReusableBoundProp],
  subject: &str,
  issues: &mut Vec<ModelIssue>,
) -> Vec<String> {
  let mut names = HashSet::new();
  let mut bindings = HashSet::new();
  let mut secrets = Vec::new();
  for prop in props {
    let valid_binding = {
      let mut chars = prop.binding_name.chars();
      matches!(chars.next(), Some(first) if first.is_ascii_lowercase())
        && chars.all(|character| character.is_ascii_alphanumeric())
        && prop.binding_name.len() <= 128
    };
    let expression_valid = match &prop.expression {
      CompiledReusablePropExpression::Literal { value } => !prop.secret && value.len() <= 65_536,
      CompiledReusablePropExpression::Context { path } => {
        !prop.secret && path.len() <= 1024 && valid_reusable_context_path(path)
      }
      CompiledReusablePropExpression::Secret { name } => {
        if prop.secret && valid_secret_name(name) && name.len() <= 128 {
          secrets.push(name.clone());
          true
        } else {
          false
        }
      }
    };
    if !valid_reusable_alias(&prop.name)
      || !valid_binding
      || !names.insert(prop.name.as_str())
      || !bindings.insert(prop.binding_name.as_str())
      || !expression_valid
    {
      issues.push(issue(
        ModelIssueCode::InvalidReusableDefinition,
        format!(
          "Reusable operation {subject} has an invalid or duplicate prop {:?}.",
          prop.name
        ),
      ));
    }
  }
  secrets.sort();
  secrets.dedup();
  secrets
}

fn valid_reusable_lifecycle(lifecycle: &CompiledReusableLifecycle) -> bool {
  [
    &lifecycle.on_success,
    &lifecycle.on_error,
    &lifecycle.on_complete,
  ]
  .into_iter()
  .flatten()
  .all(|ids| !ids.is_empty() && ids.iter().all(|id| valid_id(id)))
}

fn inspect_reusable_contract(
  workflow: &CompiledWorkflowDefinition,
  issues: &mut Vec<ModelIssue>,
) -> HashSet<String> {
  let Some(definitions) = &workflow.reusable_definitions else {
    return HashSet::new();
  };
  if workflow.schema_version != COMPILED_MODEL_SCHEMA_VERSION_V14
    || definitions.is_empty()
    || definitions.len() > 256
  {
    issues.push(issue(
      ModelIssueCode::InvalidReusableDefinition,
      "reusableDefinitions requires between 1 and 256 entries on Model v14 only.",
    ));
  }
  let nodes = workflow
    .graph
    .nodes
    .iter()
    .map(|node| (node.id.as_str(), node))
    .collect::<HashMap<_, _>>();
  let mut identities = HashSet::new();
  let mut step_nodes = HashSet::new();
  let mut provider_ids = HashSet::new();
  for definition in definitions {
    match definition {
      CompiledReusableInvocation::Step {
        invocation_id,
        node_id,
        alias,
        definition_digest,
        source,
        script_artifact_id,
        props,
        lifecycle,
      } => {
        let identity = format!("step:{invocation_id}");
        let secrets = reusable_prop_secrets(props, &identity, issues);
        let runtime_valid = nodes.get(node_id.as_str()).is_some_and(|node| {
          node.handler == "runtime.script"
            && node.script_runtime.as_ref().is_some_and(|runtime| {
              runtime.binding_version == 3
                && runtime.bindings == ["props", "context", "attempt", "services"]
                && runtime.required_secrets == secrets
            })
        });
        if !identities.insert(identity)
          || !step_nodes.insert(node_id.clone())
          || !valid_public_structural_id(invocation_id)
          || invocation_id != node_id
          || !valid_reusable_alias(alias)
          || !valid_sha256(definition_digest)
          || !valid_portable_path(source)
          || !valid_id(script_artifact_id)
          || lifecycle
            .as_ref()
            .is_some_and(|value| !valid_reusable_lifecycle(value))
          || !runtime_valid
        {
          issues.push(issue(
            ModelIssueCode::InvalidReusableDefinition,
            format!("Reusable step invocation {invocation_id:?} does not match the frozen Model v14 contract."),
          ));
        }
      }
      CompiledReusableInvocation::NotificationProvider {
        provider_id,
        alias,
        definition_digest,
        source,
        script_artifact_id,
        props,
        lifecycle,
      } => {
        let identity = format!("provider:{provider_id}");
        reusable_prop_secrets(props, &identity, issues);
        if !identities.insert(identity)
          || !provider_ids.insert(provider_id.clone())
          || !valid_id(provider_id)
          || !valid_reusable_alias(alias)
          || !valid_sha256(definition_digest)
          || !valid_portable_path(source)
          || !valid_id(script_artifact_id)
          || lifecycle
            .as_ref()
            .is_some_and(|value| !valid_reusable_lifecycle(value))
        {
          issues.push(issue(
            ModelIssueCode::InvalidReusableDefinition,
            format!("Reusable notification provider {provider_id:?} does not match the frozen Model v14 contract."),
          ));
        }
      }
    }
  }
  let mut used_provider_ids = Vec::new();
  for node in &workflow.graph.nodes {
    collect_custom_provider_ids(&node.inputs, &mut used_provider_ids);
  }
  if let Some(lifecycle) = &workflow.lifecycle {
    for hook in &lifecycle.hooks {
      for action in &hook.actions {
        collect_custom_provider_ids(&action.inputs, &mut used_provider_ids);
      }
    }
  }
  for provider_id in &provider_ids {
    if used_provider_ids
      .iter()
      .filter(|used| *used == provider_id)
      .count()
      != 1
    {
      issues.push(issue(
        ModelIssueCode::InvalidReusableDefinition,
        format!(
          "Reusable notification provider {provider_id:?} must identify exactly one generic delivery."
        ),
      ));
    }
  }
  for used_provider_id in used_provider_ids {
    if !provider_ids.contains(&used_provider_id) {
      issues.push(issue(
        ModelIssueCode::InvalidReusableDefinition,
        format!("Generic notification delivery references unknown provider {used_provider_id:?}."),
      ));
    }
  }
  step_nodes
}

fn collect_custom_provider_ids(expression: &ValueExpression, result: &mut Vec<String>) {
  match expression {
    ValueExpression::Object { fields } => {
      if literal_string(fields.get("provider")) == Some("custom") {
        if let Some(provider_id) = literal_string(fields.get("providerId")) {
          result.push(provider_id.to_string());
        }
      }
      for value in fields.values() {
        collect_custom_provider_ids(value, result);
      }
    }
    ValueExpression::Array { items } => {
      for value in items {
        collect_custom_provider_ids(value, result);
      }
    }
    _ => {}
  }
}

fn object_fields(expression: &ValueExpression) -> Option<&BTreeMap<String, ValueExpression>> {
  match expression {
    ValueExpression::Object { fields } => Some(fields),
    _ => None,
  }
}

fn literal_string(expression: Option<&ValueExpression>) -> Option<&str> {
  match expression {
    Some(ValueExpression::Literal { value }) => value.as_str(),
    _ => None,
  }
}

fn exact_fields(fields: &BTreeMap<String, ValueExpression>, names: &[&str]) -> bool {
  fields.len() == names.len() && names.iter().all(|name| fields.contains_key(*name))
}

fn valid_schema_literal(expression: Option<&ValueExpression>) -> bool {
  matches!(expression, Some(ValueExpression::Literal { value }) if value.is_object())
}

fn valid_webhook_path(path: &str) -> bool {
  path.len() <= 2048
    && path.starts_with('/')
    && path != "/_woml"
    && !path.starts_with("/_woml/")
    && (path == "/"
      || path[1..].split('/').all(|segment| {
        !segment.is_empty()
          && segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'~' | b'-'))
      }))
}

fn valid_webhook_authentication(expression: Option<&ValueExpression>) -> bool {
  let Some(fields) = expression.and_then(object_fields) else {
    return false;
  };
  match literal_string(fields.get("kind")) {
    Some("none") => exact_fields(fields, &["kind"]),
    Some("bearer") => {
      exact_fields(fields, &["kind", "secret"])
        && matches!(fields.get("secret"), Some(ValueExpression::SecretReference { name }) if valid_secret_name(name))
    }
    _ => false,
  }
}

fn valid_webhook_trigger(config: &ValueExpression) -> bool {
  let Some(fields) = object_fields(config) else {
    return false;
  };
  let keys_are_valid = exact_fields(fields, &["path", "method", "authentication"])
    || exact_fields(fields, &["path", "method", "authentication", "schema"]);
  keys_are_valid
    && literal_string(fields.get("path")).is_some_and(valid_webhook_path)
    && literal_string(fields.get("method")) == Some("POST")
    && valid_webhook_authentication(fields.get("authentication"))
    && (!fields.contains_key("schema") || valid_schema_literal(fields.get("schema")))
}

fn literal_string_array(expression: Option<&ValueExpression>) -> Option<Vec<&str>> {
  let Some(ValueExpression::Array { items }) = expression else {
    return None;
  };
  items
    .iter()
    .map(|item| match item {
      ValueExpression::Literal { value } => value.as_str(),
      _ => None,
    })
    .collect()
}

fn unique_non_empty(values: &[&str]) -> bool {
  let mut seen = HashSet::new();
  values
    .iter()
    .all(|value| !value.is_empty() && seen.insert(*value))
}

fn valid_slack_trigger(config: &ValueExpression) -> bool {
  let Some(fields) = object_fields(config) else {
    return false;
  };
  let Some(events) = literal_string_array(fields.get("events")) else {
    return false;
  };
  let Some(channels) = literal_string_array(fields.get("channels")) else {
    return false;
  };
  exact_fields(fields, &["events", "channels", "botToken", "appToken"])
    && (1..=2).contains(&events.len())
    && unique_non_empty(&events)
    && events
      .iter()
      .all(|event| matches!(*event, "app-mention" | "direct-message"))
    && unique_non_empty(&channels)
    && matches!(fields.get("botToken"), Some(ValueExpression::SecretReference { name }) if valid_secret_name(name))
    && matches!(fields.get("appToken"), Some(ValueExpression::SecretReference { name }) if valid_secret_name(name))
}

fn valid_on_missed(expression: Option<&ValueExpression>) -> bool {
  matches!(literal_string(expression), Some("skip" | "run-once"))
}

fn valid_schedule_trigger(config: &ValueExpression) -> bool {
  let Some(fields) = object_fields(config) else {
    return false;
  };
  exact_fields(fields, &["cron", "timezone", "onMissed"])
    && literal_string(fields.get("cron")).is_some_and(|value| !value.is_empty())
    && literal_string(fields.get("timezone")).is_some_and(|value| !value.is_empty())
    && valid_on_missed(fields.get("onMissed"))
}

fn valid_interval_trigger(config: &ValueExpression) -> bool {
  let Some(fields) = object_fields(config) else {
    return false;
  };
  exact_fields(fields, &["everyMs", "onMissed"])
    && matches!(fields.get("everyMs"), Some(ValueExpression::Literal { value }) if value.as_u64().is_some_and(|value| (1_000..=2_592_000_000).contains(&value)))
    && valid_on_missed(fields.get("onMissed"))
}

fn valid_event_name(value: &str) -> bool {
  if value.len() > 256
    || !value
      .bytes()
      .next()
      .is_some_and(|byte| byte.is_ascii_lowercase())
  {
    return false;
  }
  let mut has_separator = false;
  let mut previous_separator = false;
  for byte in value.bytes() {
    let separator = matches!(byte, b'.' | b'_' | b'-');
    if separator {
      if previous_separator {
        return false;
      }
      has_separator = true;
    } else if !byte.is_ascii_lowercase() && !byte.is_ascii_digit() {
      return false;
    }
    previous_separator = separator;
  }
  has_separator && !previous_separator
}

fn valid_event_trigger(config: &ValueExpression) -> bool {
  let Some(fields) = object_fields(config) else {
    return false;
  };
  // Model v7 event definitions were persisted before publisher authentication
  // became an authored frontend requirement. Keep those immutable definitions
  // readable for recovery; the active ingress runtime separately requires the
  // symbolic secret on newly registered event triggers.
  let keys_are_valid = exact_fields(fields, &["name"])
    || exact_fields(fields, &["name", "schema"])
    || exact_fields(fields, &["name", "secret"])
    || exact_fields(fields, &["name", "schema", "secret"]);
  keys_are_valid
    && literal_string(fields.get("name")).is_some_and(valid_event_name)
    && (!fields.contains_key("secret")
      || matches!(fields.get("secret"), Some(ValueExpression::SecretReference { name }) if valid_secret_name(name)))
    && (!fields.contains_key("schema") || valid_schema_literal(fields.get("schema")))
}

fn valid_model_v7_trigger(trigger: &CompiledTrigger) -> bool {
  match trigger.handler.as_str() {
    "trigger.manual" => {
      matches!(&trigger.config, ValueExpression::Object { fields } if fields.is_empty())
    }
    "trigger.webhook" => valid_webhook_trigger(&trigger.config),
    "trigger.slack" => valid_slack_trigger(&trigger.config),
    "trigger.schedule" => valid_schedule_trigger(&trigger.config),
    "trigger.interval" => valid_interval_trigger(&trigger.config),
    "trigger.event" => valid_event_trigger(&trigger.config),
    _ => false,
  }
}

fn valid_slack_destination(value: &str) -> bool {
  if let Some(alias) = value.strip_prefix('#') {
    return !alias.is_empty()
      && alias.len() <= 80
      && alias.bytes().enumerate().all(|(index, byte)| {
        byte.is_ascii_lowercase()
          || byte.is_ascii_digit()
          || (index > 0 && matches!(byte, b'_' | b'-'))
      });
  }
  let bytes = value.as_bytes();
  (9..=32).contains(&bytes.len())
    && matches!(bytes.first(), Some(b'C' | b'G'))
    && bytes[1..]
      .iter()
      .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

fn selector_branch_id(value: &str) -> Option<&str> {
  value
    .strip_prefix("__woml_branch__")?
    .strip_suffix("__select")
    .filter(|branch_id| valid_public_structural_id(branch_id))
}

fn parallel_start_id(value: &str) -> Option<&str> {
  value
    .strip_prefix("__woml_parallel__")?
    .strip_suffix("__start")
    .filter(|parallel_id| valid_public_structural_id(parallel_id))
}

fn parallel_start_inputs(node: Option<&CompiledWorkflowNode>, child_count: usize) -> bool {
  let Some(node) = node else {
    return false;
  };
  if node.handler != "engine.parallel-start" {
    return false;
  }
  let ValueExpression::Object { fields } = &node.inputs else {
    return false;
  };
  if fields.len() != 2 {
    return false;
  }
  let valid_concurrency = matches!(
    fields.get("concurrency"),
    Some(ValueExpression::Literal { value })
      if value.as_u64().is_some_and(|value| value >= 1 && value <= child_count as u64)
  );
  let valid_policy = matches!(
    fields.get("onError"),
    Some(ValueExpression::Literal { value })
      if matches!(value.as_str(), Some("fail-fast" | "wait-all"))
  );
  valid_concurrency && valid_policy
}

fn parallel_join_inputs(node: Option<&CompiledWorkflowNode>) -> bool {
  matches!(
    node,
    Some(CompiledWorkflowNode {
      handler,
      inputs: ValueExpression::Object { fields },
      ..
    }) if handler == "engine.parallel-join" && fields.is_empty()
  )
}

fn is_branch_context_reference(expression: &ValueExpression) -> bool {
  let ValueExpression::ContextReference { path } = expression else {
    return false;
  };
  match path.as_slice() {
    [root, rest @ ..] if root == "trigger" => rest.iter().all(|segment| !segment.is_empty()),
    [root, structural_id, rest @ ..] if root == "steps" => {
      valid_public_structural_id(structural_id) && rest.iter().all(|segment| !segment.is_empty())
    }
    _ => false,
  }
}

fn inspect_expression(expression: &ValueExpression, at: &str, issues: &mut Vec<ModelIssue>) {
  match expression {
    ValueExpression::Literal { .. } => {}
    ValueExpression::ContextReference { path } => {
      if path.is_empty() || path.iter().any(|part| part.is_empty()) {
        issues.push(issue(
          ModelIssueCode::InvalidValueExpression,
          format!("Context reference at {at} must contain non-empty path segments."),
        ));
      }
    }
    ValueExpression::LifecycleReference { path } => {
      if path.is_empty() || path.len() > 32 || path.iter().any(|part| part.is_empty()) {
        issues.push(issue(
          ModelIssueCode::InvalidValueExpression,
          format!("Lifecycle reference at {at} must contain 1 to 32 non-empty path segments."),
        ));
      }
    }
    ValueExpression::SecretReference { name } => {
      if !valid_secret_name(name) {
        issues.push(issue(
          ModelIssueCode::InvalidValueExpression,
          format!("Secret reference at {at} has an invalid symbolic name."),
        ));
      }
    }
    ValueExpression::Object { fields } => {
      for (name, value) in fields {
        inspect_expression(value, &format!("{at}.{name}"), issues);
      }
    }
    ValueExpression::Array { items } => {
      for (index, value) in items.iter().enumerate() {
        inspect_expression(value, &format!("{at}[{index}]"), issues);
      }
    }
    ValueExpression::Template { parts } => {
      if parts.is_empty() {
        issues.push(issue(
          ModelIssueCode::InvalidValueExpression,
          format!("Template expression at {at} must contain at least one part."),
        ));
      }
      for (index, part) in parts.iter().enumerate() {
        if let TemplatePart::ContextReference { path } | TemplatePart::LifecycleReference { path } =
          part
        {
          if path.is_empty() || path.len() > 32 || path.iter().any(|segment| segment.is_empty()) {
            issues.push(issue(
              ModelIssueCode::InvalidValueExpression,
              format!(
                "Reference at {at}.parts[{index}] must contain 1 to 32 non-empty path segments."
              ),
            ));
          }
        }
      }
    }
  }
}

fn contains_lifecycle_reference(expression: &ValueExpression) -> bool {
  match expression {
    ValueExpression::LifecycleReference { .. } => true,
    ValueExpression::Object { fields } => fields.values().any(contains_lifecycle_reference),
    ValueExpression::Array { items } => items.iter().any(contains_lifecycle_reference),
    ValueExpression::Template { parts } => parts
      .iter()
      .any(|part| matches!(part, TemplatePart::LifecycleReference { .. })),
    ValueExpression::Literal { .. }
    | ValueExpression::ContextReference { .. }
    | ValueExpression::SecretReference { .. } => false,
  }
}

fn inspect_branch_contract(workflow: &CompiledWorkflowDefinition, issues: &mut Vec<ModelIssue>) {
  let nodes: HashMap<&str, &CompiledWorkflowNode> = workflow
    .graph
    .nodes
    .iter()
    .map(|node| (node.id.as_str(), node))
    .collect();
  let mut groups: BTreeMap<&str, Vec<&CompiledWorkflowEdge>> = BTreeMap::new();
  let mut adjacency: HashMap<&str, Vec<&str>> = workflow
    .graph
    .nodes
    .iter()
    .map(|node| (node.id.as_str(), Vec::new()))
    .collect();
  let control_choice_arm_ids: HashSet<&str> = workflow
    .graph
    .choices
    .iter()
    .flatten()
    .flat_map(|choice| choice.arm_ids.iter().map(String::as_str))
    .collect();
  for edge in &workflow.graph.edges {
    adjacency
      .entry(edge.from.as_str())
      .or_default()
      .push(edge.to.as_str());
  }

  for edge in &workflow.graph.edges {
    let is_v4_approval_equals = workflow.schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V4
      && edge.approval_id.is_some()
      && matches!(edge.condition, EdgeCondition::Equals { .. });
    if workflow.schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V2
      && (matches!(edge.condition, EdgeCondition::Truthy { .. })
        || matches!(edge.condition, EdgeCondition::Equals { .. }) && !is_v4_approval_equals)
    {
      issues.push(issue(
        ModelIssueCode::UnsupportedEdgeCondition,
        format!(
          "Model v2 edge {:?} uses a condition outside the frozen branch profile.",
          edge.id
        ),
      ));
    }
    if workflow.schema_version == COMPILED_MODEL_SCHEMA_VERSION_V1
      && matches!(edge.condition, EdgeCondition::Boolean { .. })
    {
      issues.push(issue(
        ModelIssueCode::UnsupportedEdgeCondition,
        format!(
          "Model v1 edge {:?} cannot use a boolean condition.",
          edge.id
        ),
      ));
    }

    if let Some(branch_id) = edge.branch_id.as_deref() {
      groups.entry(branch_id).or_default().push(edge);
    } else if matches!(edge.condition, EdgeCondition::Boolean { .. })
      && !control_choice_arm_ids.contains(edge.id.as_str())
    {
      issues.push(issue(
        ModelIssueCode::InvalidBranchGroup,
        format!("Boolean edge {:?} must carry a branchId.", edge.id),
      ));
    }
  }

  if workflow.schema_version == COMPILED_MODEL_SCHEMA_VERSION_V1 && !groups.is_empty() {
    issues.push(issue(
      ModelIssueCode::InvalidBranchGroup,
      "Compiled model v1 cannot contain frozen model-v2 branch groups.",
    ));
  }

  for (branch_id, edges) in &groups {
    let selector_id = format!("__woml_branch__{branch_id}__select");
    let selector = nodes.get(selector_id.as_str()).copied();
    let selector_inputs_are_empty = matches!(
      selector.map(|node| &node.inputs),
      Some(ValueExpression::Object { fields }) if fields.is_empty()
    );
    let selector_outgoing: Vec<_> = workflow
      .graph
      .edges
      .iter()
      .filter(|edge| edge.from == selector_id)
      .collect();
    if !valid_public_structural_id(branch_id)
      || selector.map(|node| node.handler.as_str()) != Some("engine.branch-select")
      || !selector_inputs_are_empty
      || edges.iter().any(|edge| edge.from != selector_id)
      || selector_outgoing.len() != edges.len()
      || selector_outgoing
        .iter()
        .any(|edge| edge.branch_id.as_deref() != Some(*branch_id))
    {
      issues.push(issue(
        ModelIssueCode::InvalidBranchSelector,
        format!(
          "Branch group {branch_id:?} must originate exclusively from its canonical empty selector."
        ),
      ));
    }

    let (otherwise, when_edges) = match edges.split_last() {
      Some(parts) => parts,
      None => {
        issues.push(issue(
          ModelIssueCode::InvalidBranchGroup,
          format!("Branch group {branch_id:?} has no cases."),
        ));
        continue;
      }
    };
    let valid_when_edges = !when_edges.is_empty()
      && when_edges.iter().enumerate().all(|(index, edge)| {
        edge.id == format!("{branch_id}:when:{index}")
          && matches!(&edge.condition, EdgeCondition::Boolean { value } if is_branch_context_reference(value))
      });
    let valid_otherwise = otherwise.id == format!("{branch_id}:otherwise")
      && matches!(otherwise.condition, EdgeCondition::Always);
    if !valid_when_edges || !valid_otherwise {
      issues.push(issue(
        ModelIssueCode::InvalidBranchGroup,
        format!(
          "Branch group {branch_id:?} must contain contiguous ordered boolean cases followed by one fallback."
        ),
      ));
    }

    let result = nodes.get(branch_id).copied();
    let expected_keys: HashSet<&str> = edges.iter().map(|edge| edge.id.as_str()).collect();
    let valid_result_inputs = match result.map(|node| &node.inputs) {
      Some(ValueExpression::Object { fields }) => {
        fields.len() == expected_keys.len()
          && fields.iter().all(|(key, value)| {
            expected_keys.contains(key.as_str()) && is_branch_context_reference(value)
          })
      }
      _ => false,
    };
    if result.map(|node| node.handler.as_str()) != Some("engine.branch-result")
      || !valid_result_inputs
    {
      issues.push(issue(
        ModelIssueCode::InvalidBranchResult,
        format!(
          "Branch result {branch_id:?} must expose one context reference for every branch arm."
        ),
      ));
    }

    let incoming_result_edges: Vec<_> = workflow
      .graph
      .edges
      .iter()
      .filter(|edge| edge.to.as_str() == *branch_id)
      .collect();
    let valid_joins = incoming_result_edges.len() == edges.len()
      && incoming_result_edges
        .iter()
        .all(|edge| edge.branch_id.is_none() && matches!(edge.condition, EdgeCondition::Always));
    let route_sets: Vec<_> = edges
      .iter()
      .map(|edge| {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::from([edge.to.as_str()]);
        let mut reaches_result = false;
        while let Some(current) = queue.pop_front() {
          if current == *branch_id {
            reaches_result = true;
            continue;
          }
          if !visited.insert(current) {
            continue;
          }
          queue.extend(adjacency.get(current).into_iter().flatten().copied());
        }
        (reaches_result, visited)
      })
      .collect();
    let routes_are_disjoint = route_sets.iter().enumerate().all(|(index, (_, route))| {
      route_sets[index + 1..]
        .iter()
        .all(|(_, other)| route.is_disjoint(other))
    });
    let joins_belong_to_distinct_routes = incoming_result_edges.iter().all(|join| {
      route_sets
        .iter()
        .filter(|(_, route)| route.contains(join.from.as_str()))
        .count()
        == 1
    });
    if !valid_joins
      || route_sets.iter().any(|(reaches, _)| !reaches)
      || !routes_are_disjoint
      || !joins_belong_to_distinct_routes
    {
      issues.push(issue(
        ModelIssueCode::InvalidBranchGroup,
        format!(
          "Branch group {branch_id:?} must contain disjoint routes with one ordinary join into its result per arm."
        ),
      ));
    }
  }

  for node in &workflow.graph.nodes {
    if node.handler == "engine.branch-select" {
      let branch_id = selector_branch_id(&node.id);
      if branch_id.is_none_or(|id| !groups.contains_key(id)) {
        issues.push(issue(
          ModelIssueCode::InvalidBranchSelector,
          format!(
            "Branch selector {:?} has no matching canonical edge group.",
            node.id
          ),
        ));
      }
    } else if node.handler == "engine.branch-result" && !groups.contains_key(node.id.as_str()) {
      issues.push(issue(
        ModelIssueCode::InvalidBranchResult,
        format!("Branch result {:?} has no matching edge group.", node.id),
      ));
    }
  }
}

fn lifecycle_hook_id(event: LifecycleEventName) -> &'static str {
  match event {
    LifecycleEventName::RunStart => "lifecycle:run_start",
    LifecycleEventName::StepStart => "lifecycle:step_start",
    LifecycleEventName::StepSuccess => "lifecycle:step_success",
    LifecycleEventName::StepFailure => "lifecycle:step_failure",
    LifecycleEventName::StepComplete => "lifecycle:step_complete",
    LifecycleEventName::RunSuccess => "lifecycle:run_success",
    LifecycleEventName::RunFailure => "lifecycle:run_failure",
    LifecycleEventName::RunCancel => "lifecycle:run_cancel",
    LifecycleEventName::RunComplete => "lifecycle:run_complete",
  }
}

fn valid_lifecycle_script_inputs(inputs: &ValueExpression) -> bool {
  matches!(
    inputs,
    ValueExpression::Object { fields }
      if fields.len() == 1
        && matches!(fields.get("source"), Some(ValueExpression::Literal { value }) if value.as_str().is_some_and(|source| source.len() <= 1_048_576))
  )
}

fn reusable_provider_ids(workflow: &CompiledWorkflowDefinition) -> HashSet<&str> {
  workflow
    .reusable_definitions
    .iter()
    .flatten()
    .filter_map(|definition| match definition {
      CompiledReusableInvocation::NotificationProvider { provider_id, .. } => {
        Some(provider_id.as_str())
      }
      CompiledReusableInvocation::Step { .. } => None,
    })
    .collect()
}

fn valid_custom_provider_delivery(
  fields: &BTreeMap<String, ValueExpression>,
  expected_domain: &str,
  provider_ids: &HashSet<&str>,
  message_required: bool,
) -> bool {
  let required_fields = if message_required { 7 } else { 6 };
  if fields.len() < required_fields || fields.len() > 7 {
    return false;
  }
  if fields.keys().any(|key| {
    !matches!(
      key.as_str(),
      "deliveryId"
        | "provider"
        | "destination"
        | "credentials"
        | "providerId"
        | "domain"
        | "message"
    )
  }) {
    return false;
  }
  let delivery_id = literal_string(fields.get("deliveryId"));
  let provider_id = literal_string(fields.get("providerId"));
  let message_valid = fields.get("message").is_none_or(|message| {
    matches!(message, ValueExpression::Template { parts } if (1..=65).contains(&parts.len()) && parts.iter().all(|part| !matches!(part, TemplatePart::Text { text } if text.len() > 4096)))
  });
  literal_string(fields.get("provider")) == Some("custom")
    && literal_string(fields.get("domain")) == Some(expected_domain)
    && delivery_id.is_some_and(|value| valid_id(value) && Some(value) == provider_id)
    && provider_id.is_some_and(|value| provider_ids.contains(value))
    && literal_string(fields.get("destination")).is_some_and(valid_reusable_alias)
    && fields
      .get("credentials")
      .and_then(object_fields)
      .is_some_and(BTreeMap::is_empty)
    && (!message_required || fields.contains_key("message"))
    && message_valid
}

fn valid_lifecycle_notification_inputs(
  inputs: &ValueExpression,
  provider_ids: &HashSet<&str>,
) -> bool {
  let Some(fields) = object_fields(inputs) else {
    return false;
  };
  let Some(ValueExpression::Array { items }) = fields.get("deliveries") else {
    return false;
  };
  fields.len() == 1
    && (1..=256).contains(&items.len())
    && items.iter().all(|delivery| {
      let Some(fields) = object_fields(delivery) else {
        return false;
      };
      if literal_string(fields.get("provider")) == Some("custom") {
        return valid_custom_provider_delivery(fields, "informational", provider_ids, true);
      }
      let credentials = fields.get("credentials").and_then(object_fields);
      fields.len() == 5
        && matches!(fields.get("deliveryId"), Some(ValueExpression::Literal { value }) if value.as_str().is_some_and(|value| !value.is_empty() && value.len() <= 512))
        && matches!(fields.get("provider"), Some(ValueExpression::Literal { value }) if value.as_str() == Some("slack"))
        && matches!(fields.get("destination"), Some(ValueExpression::Literal { value }) if value.as_str().is_some_and(|value| !value.is_empty() && value.len() <= 512))
        && credentials.is_some_and(|credentials| {
          credentials.len() == 2
            && matches!(credentials.get("botToken"), Some(ValueExpression::SecretReference { name }) if valid_secret_name(name))
            && matches!(credentials.get("appToken"), Some(ValueExpression::SecretReference { name }) if valid_secret_name(name))
        })
        && matches!(fields.get("message"), Some(ValueExpression::Template { parts }) if (1..=65).contains(&parts.len()) && parts.iter().all(|part| !matches!(part, TemplatePart::Text { text } if text.len() > 4096)))
    })
}

fn inspect_lifecycle_contract(workflow: &CompiledWorkflowDefinition, issues: &mut Vec<ModelIssue>) {
  let Some(lifecycle) = &workflow.lifecycle else {
    return;
  };
  if !matches!(
    workflow.schema_version,
    COMPILED_MODEL_SCHEMA_VERSION_V11
      | COMPILED_MODEL_SCHEMA_VERSION_V12
      | COMPILED_MODEL_SCHEMA_VERSION_V13
      | COMPILED_MODEL_SCHEMA_VERSION_V14
  ) {
    issues.push(issue(
      ModelIssueCode::InvalidLifecycle,
      "lifecycle is available only in compiled Model v11 or later.",
    ));
    return;
  }
  if lifecycle.profile_version != 1 || lifecycle.hooks.is_empty() || lifecycle.hooks.len() > 9 {
    issues.push(issue(
      ModelIssueCode::InvalidLifecycle,
      "Model v11 lifecycle must use profileVersion 1 and contain 1 to 9 hooks.",
    ));
    return;
  }
  let node_ids = workflow
    .graph
    .nodes
    .iter()
    .filter(|node| node.handler == "runtime.script")
    .map(|node| node.id.as_str())
    .collect::<HashSet<_>>();
  let mut events = HashSet::new();
  let mut action_ids = HashSet::new();
  let provider_ids = reusable_provider_ids(workflow);
  for hook in &lifecycle.hooks {
    let valid_steps = match (&hook.step_ids, hook.event.is_step()) {
      (None, true) => true,
      (Some(step_ids), true) => {
        !step_ids.is_empty()
          && step_ids.iter().all(|id| node_ids.contains(id.as_str()))
          && step_ids.iter().collect::<HashSet<_>>().len() == step_ids.len()
      }
      (None, false) => true,
      (Some(_), false) => false,
    };
    if hook.hook_id != lifecycle_hook_id(hook.event)
      || !events.insert(hook.event)
      || !valid_steps
      || hook.actions.is_empty()
      || hook.actions.len() > 64
    {
      issues.push(issue(
        ModelIssueCode::InvalidLifecycle,
        format!(
          "Lifecycle hook {:?} does not match the frozen Lifecycle Binding v1 contract.",
          hook.hook_id
        ),
      ));
      continue;
    }
    for (index, action) in hook.actions.iter().enumerate() {
      let expected_id = format!("{}:action:{index}", hook.hook_id);
      let valid_handler = match (action.handler.as_str(), &action.script_runtime) {
        ("runtime.lifecycle-script", Some(runtime)) => {
          runtime.binding_version == 2
            && runtime.bindings == ["context", "lifecycle", "attempt", "services", "secrets"]
            && runtime.required_secrets.len() <= 64
            && runtime
              .required_secrets
              .windows(2)
              .all(|pair| pair[0] < pair[1])
            && runtime
              .required_secrets
              .iter()
              .all(|name| valid_secret_name(name))
            && valid_lifecycle_script_inputs(&action.inputs)
        }
        ("notification.informational", None) => {
          valid_lifecycle_notification_inputs(&action.inputs, &provider_ids)
        }
        _ => false,
      };
      if action.action_id != expected_id
        || !action_ids.insert(action.action_id.as_str())
        || !valid_handler
      {
        issues.push(issue(
          ModelIssueCode::InvalidLifecycle,
          format!(
            "Lifecycle action {:?} does not match the frozen Lifecycle Binding v1 contract.",
            action.action_id
          ),
        ));
      }
      inspect_expression(
        &action.inputs,
        &format!("lifecycle[{}].action[{index}].inputs", hook.hook_id),
        issues,
      );
    }
  }
}

fn valid_policy_queue_name(value: &str) -> bool {
  if value.is_empty() || value.len() > 128 {
    return false;
  }
  let mut chars = value.chars();
  if !matches!(chars.next(), Some(first) if first.is_ascii_lowercase()) {
    return false;
  }
  let mut separator = false;
  for character in chars {
    if matches!(character, '.' | '_' | '-') {
      if separator {
        return false;
      }
      separator = true;
    } else if character.is_ascii_lowercase() || character.is_ascii_digit() {
      separator = false;
    } else {
      return false;
    }
  }
  !separator
}

fn inspect_runtime_policy_contract(
  workflow: &CompiledWorkflowDefinition,
  issues: &mut Vec<ModelIssue>,
) {
  match (workflow.schema_version, &workflow.runtime_policy) {
    (
      COMPILED_MODEL_SCHEMA_VERSION_V12
      | COMPILED_MODEL_SCHEMA_VERSION_V13
      | COMPILED_MODEL_SCHEMA_VERSION_V14,
      Some(policy),
    ) => {
      let has_policy = policy.concurrency.is_some()
        || policy.timeout_ms.is_some()
        || policy.rate_limit.is_some()
        || policy.queue.is_some();
      let concurrency_valid = policy
        .concurrency
        .is_none_or(|value| (1..=1_000_000).contains(&value));
      let timeout_valid = policy
        .timeout_ms
        .is_none_or(|value| (1..=31_536_000_000).contains(&value));
      let rate_valid = policy.rate_limit.as_ref().is_none_or(|rate| {
        (1..=1_000_000).contains(&rate.count)
          && (1..=31_536_000_000).contains(&rate.window_ms)
          && rate.algorithm == RateLimitAlgorithm::RollingWindow
      });
      let queue_valid = policy.queue.as_ref().is_none_or(|queue| {
        valid_policy_queue_name(&queue.name)
          && queue.discipline == QueueDiscipline::WorkConservingFifo
      });
      if policy.profile_version != 1
        || !has_policy
        || !concurrency_valid
        || !timeout_valid
        || !rate_valid
        || !queue_valid
      {
        issues.push(issue(
          ModelIssueCode::InvalidRuntimePolicy,
          "Model v12+ runtimePolicy does not match the frozen Runtime Policy v1 contract.",
        ));
      }
    }
    (
      COMPILED_MODEL_SCHEMA_VERSION_V12
      | COMPILED_MODEL_SCHEMA_VERSION_V13
      | COMPILED_MODEL_SCHEMA_VERSION_V14,
      None,
    ) => issues.push(issue(
      ModelIssueCode::InvalidRuntimePolicy,
      "Compiled Model v12+ requires runtimePolicy.",
    )),
    (_, Some(_)) => issues.push(issue(
      ModelIssueCode::InvalidRuntimePolicy,
      "runtimePolicy is unavailable before compiled Model v12.",
    )),
    _ => {}
  }
}

fn inspect_parallel_contract(workflow: &CompiledWorkflowDefinition, issues: &mut Vec<ModelIssue>) {
  let nodes: HashMap<&str, &CompiledWorkflowNode> = workflow
    .graph
    .nodes
    .iter()
    .map(|node| (node.id.as_str(), node))
    .collect();
  let mut groups: BTreeMap<&str, Vec<&CompiledWorkflowEdge>> = BTreeMap::new();
  for edge in &workflow.graph.edges {
    if let Some(parallel_id) = edge.parallel_id.as_deref() {
      groups.entry(parallel_id).or_default().push(edge);
      if edge.branch_id.is_some()
        || edge.approval_id.is_some()
        || !matches!(edge.condition, EdgeCondition::Always)
      {
        issues.push(issue(
          ModelIssueCode::InvalidParallelGroup,
          format!(
            "Parallel edge {:?} must be unconditional and cannot also belong to a branch.",
            edge.id
          ),
        ));
      }
    }
  }

  if workflow.schema_version < COMPILED_MODEL_SCHEMA_VERSION_V3 && !groups.is_empty() {
    issues.push(issue(
      ModelIssueCode::InvalidParallelGroup,
      "Compiled model v1/v2 cannot contain model-v3 parallel groups.",
    ));
  }

  let mut child_owners: HashMap<&str, &str> = HashMap::new();
  for (parallel_id, edges) in &groups {
    let start_id = format!("__woml_parallel__{parallel_id}__start");
    let start = nodes.get(start_id.as_str()).copied();
    let join = nodes.get(*parallel_id).copied();
    let child_edges: Vec<_> = edges
      .iter()
      .copied()
      .filter(|edge| edge.from == start_id)
      .collect();
    let join_edges: Vec<_> = edges
      .iter()
      .copied()
      .filter(|edge| edge.to.as_str() == *parallel_id)
      .collect();
    let children: Vec<_> = child_edges.iter().map(|edge| edge.to.as_str()).collect();

    let ordered_edges_are_valid = !child_edges.is_empty()
      && join_edges.len() == child_edges.len()
      && edges.len() == child_edges.len() * 2
      && child_edges.iter().enumerate().all(|(index, edge)| {
        edge.id == format!("{parallel_id}:child:{index}")
          && edge.parallel_id.as_deref() == Some(*parallel_id)
          && edge.branch_id.is_none()
          && matches!(edge.condition, EdgeCondition::Always)
          && nodes
            .get(edge.to.as_str())
            .is_some_and(|node| node.handler == "runtime.script")
      })
      && join_edges.iter().enumerate().all(|(index, edge)| {
        edge.id == format!("{parallel_id}:join:{index}")
          && edge.from == children[index]
          && edge.parallel_id.as_deref() == Some(*parallel_id)
          && edge.branch_id.is_none()
          && matches!(edge.condition, EdgeCondition::Always)
      });
    let start_outgoing: Vec<_> = workflow
      .graph
      .edges
      .iter()
      .filter(|edge| edge.from == start_id)
      .collect();
    let join_incoming: Vec<_> = workflow
      .graph
      .edges
      .iter()
      .filter(|edge| edge.to.as_str() == *parallel_id)
      .collect();
    let child_boundaries_are_closed = children.iter().all(|child_id| {
      let incoming: Vec<_> = workflow
        .graph
        .edges
        .iter()
        .filter(|edge| edge.to == *child_id)
        .collect();
      let outgoing: Vec<_> = workflow
        .graph
        .edges
        .iter()
        .filter(|edge| edge.from == *child_id)
        .collect();
      incoming.len() == 1
        && incoming[0].from == start_id
        && incoming[0].parallel_id.as_deref() == Some(*parallel_id)
        && outgoing.len() == 1
        && outgoing[0].to.as_str() == *parallel_id
        && outgoing[0].parallel_id.as_deref() == Some(*parallel_id)
    });
    let boundaries_are_valid = start_outgoing.len() == child_edges.len()
      && start_outgoing
        .iter()
        .all(|edge| edge.parallel_id.as_deref() == Some(*parallel_id))
      && join_incoming.len() == join_edges.len()
      && join_incoming
        .iter()
        .all(|edge| edge.parallel_id.as_deref() == Some(*parallel_id))
      && child_boundaries_are_closed
      && workflow
        .graph
        .edges
        .iter()
        .any(|edge| edge.from.as_str() == *parallel_id);

    let mut duplicate_child = false;
    for child_id in &children {
      if child_owners.insert(child_id, parallel_id).is_some() {
        duplicate_child = true;
      }
    }

    if !valid_public_structural_id(parallel_id)
      || !parallel_start_inputs(start, child_edges.len())
      || !parallel_join_inputs(join)
      || !ordered_edges_are_valid
      || !boundaries_are_valid
      || duplicate_child
    {
      issues.push(issue(
        ModelIssueCode::InvalidParallelGroup,
        format!(
          "Parallel group {parallel_id:?} does not match the frozen start, ordered child, join, policy, and concurrency contract."
        ),
      ));
    }
  }

  for node in &workflow.graph.nodes {
    if node.handler == "engine.parallel-start" {
      if parallel_start_id(&node.id).is_none_or(|parallel_id| !groups.contains_key(parallel_id)) {
        issues.push(issue(
          ModelIssueCode::InvalidParallelGroup,
          format!("Parallel start {:?} has no matching edge group.", node.id),
        ));
      }
    } else if node.handler == "engine.parallel-join" && !groups.contains_key(node.id.as_str()) {
      issues.push(issue(
        ModelIssueCode::InvalidParallelGroup,
        format!("Parallel join {:?} has no matching edge group.", node.id),
      ));
    }
  }
}

fn approval_join_id(value: &str) -> Option<&str> {
  value
    .strip_prefix("__woml_approval__")?
    .strip_suffix("__join")
    .filter(|approval_id| valid_public_structural_id(approval_id))
}

fn approval_notifications(
  approval_id: &str,
  fields: &BTreeMap<String, ValueExpression>,
  provider_ids: &HashSet<&str>,
) -> Option<Vec<NotificationDefinition>> {
  let Some(expression) = fields.get("notifications") else {
    return Some(Vec::new());
  };
  let ValueExpression::Array { items } = expression else {
    return None;
  };
  if items.is_empty() {
    return None;
  }
  let mut notifications = Vec::with_capacity(items.len());
  let mut previous_tag = None;
  let mut previous_channel = None;
  let mut duplicate_keys = HashSet::new();
  for item in items {
    let ValueExpression::Object { fields } = item else {
      return None;
    };
    let ValueExpression::Literal { value: delivery } = fields.get("deliveryId")? else {
      return None;
    };
    let ValueExpression::Literal { value: provider } = fields.get("provider")? else {
      return None;
    };
    let ValueExpression::Literal { value: destination } = fields.get("destination")? else {
      return None;
    };
    let ValueExpression::Object {
      fields: credentials,
    } = fields.get("credentials")?
    else {
      return None;
    };
    let delivery_id = delivery.as_str()?;
    let provider = provider.as_str()?;
    let destination = destination.as_str()?;
    let prefix = format!("{approval_id}:notify:");
    let (tag, channel) = delivery_id.strip_prefix(&prefix)?.split_once(":channel:")?;
    let tag = tag.parse::<usize>().ok()?;
    let channel = channel.parse::<usize>().ok()?;
    let ordered = match (previous_tag, previous_channel) {
      (None, None) => tag == 0 && channel == 0,
      (Some(previous_tag), Some(previous_channel)) if tag == previous_tag => {
        channel == previous_channel + 1
      }
      (Some(previous_tag), Some(_)) => tag == previous_tag + 1 && channel == 0,
      _ => false,
    };
    if !ordered {
      return None;
    }
    let (names, provider_id, message, duplicate_key) = if provider == "slack" {
      if fields.len() != 4
        || !["deliveryId", "provider", "destination", "credentials"]
          .iter()
          .all(|key| fields.contains_key(*key))
        || !valid_slack_destination(destination)
        || credentials.len() != 2
      {
        return None;
      }
      let mut names = BTreeMap::new();
      for key in ["botToken", "appToken"] {
        let ValueExpression::SecretReference { name } = credentials.get(key)? else {
          return None;
        };
        if !valid_secret_name(name) {
          return None;
        }
        names.insert(key.to_string(), name.clone());
      }
      let duplicate_key = format!(
        "slack\0{}\0{}\0{}",
        names["botToken"], names["appToken"], destination
      );
      (names, None, None, duplicate_key)
    } else if provider == "custom"
      && valid_custom_provider_delivery(fields, "approval", provider_ids, false)
    {
      let provider_id = literal_string(fields.get("providerId"))?.to_string();
      (
        BTreeMap::new(),
        Some(provider_id.clone()),
        fields.get("message").cloned(),
        format!("custom\0{provider_id}"),
      )
    } else {
      return None;
    };
    if !duplicate_keys.insert(duplicate_key) {
      return None;
    }
    notifications.push(NotificationDefinition {
      delivery_id: delivery_id.to_string(),
      provider: provider.to_string(),
      destination: destination.to_string(),
      credentials: names,
      provider_id,
      message,
    });
    previous_tag = Some(tag);
    previous_channel = Some(channel);
  }
  Some(notifications)
}

fn approval_wait_inputs(node: Option<&CompiledWorkflowNode>, provider_ids: &HashSet<&str>) -> bool {
  let Some(node) = node else {
    return false;
  };
  if node.handler != "engine.approval-wait"
    || node.timeout_ms.is_some()
    || node.retry_policy.is_some()
  {
    return false;
  }
  let ValueExpression::Object { fields } = &node.inputs else {
    return false;
  };
  if !(1..=3).contains(&fields.len()) {
    return false;
  }
  if fields
    .keys()
    .any(|key| key != "timeoutMs" && key != "onTimeout" && key != "notifications")
  {
    return false;
  }
  let valid_timeout = fields.get("timeoutMs").is_none_or(|value| {
    matches!(
      value,
      ValueExpression::Literal { value }
        if value.as_u64().is_some_and(|milliseconds| {
          (1..=9_007_199_254_740_991).contains(&milliseconds)
        })
    )
  });
  let valid_policy = matches!(
    fields.get("onTimeout"),
    Some(ValueExpression::Literal { value })
      if matches!(value.as_str(), Some("reject" | "fail"))
  );
  let valid_metadata = node.metadata.as_ref().is_none_or(|metadata| {
    metadata
      .keys()
      .all(|key| key == "name" || key == "description")
      && metadata
        .values()
        .all(|value| value.as_str().is_some_and(|text| !text.is_empty()))
  });
  let valid_notifications = approval_notifications(&node.id, fields, provider_ids).is_some();
  valid_timeout && valid_policy && valid_metadata && valid_notifications
}

fn approval_join_inputs(node: Option<&CompiledWorkflowNode>) -> bool {
  matches!(
    node,
    Some(CompiledWorkflowNode {
      handler,
      inputs: ValueExpression::Object { fields },
      timeout_ms: None,
      retry_policy: None,
      metadata: None,
      ..
    }) if handler == "engine.approval-join" && fields.is_empty()
  )
}

fn is_approval_decision_condition(
  condition: &EdgeCondition,
  approval_id: &str,
  decision: &str,
) -> bool {
  matches!(
    condition,
    EdgeCondition::Equals {
      left: ValueExpression::ContextReference { path },
      right: ValueExpression::Literal { value },
    } if path == &["steps", approval_id, "decision"] && value.as_str() == Some(decision)
  )
}

fn inspect_approval_contract(workflow: &CompiledWorkflowDefinition, issues: &mut Vec<ModelIssue>) {
  let provider_ids = reusable_provider_ids(workflow);
  let nodes: HashMap<&str, &CompiledWorkflowNode> = workflow
    .graph
    .nodes
    .iter()
    .map(|node| (node.id.as_str(), node))
    .collect();
  let mut groups: BTreeMap<&str, Vec<&CompiledWorkflowEdge>> = BTreeMap::new();

  for edge in &workflow.graph.edges {
    if let Some(approval_id) = edge.approval_id.as_deref() {
      groups.entry(approval_id).or_default().push(edge);
      if edge.branch_id.is_some() || edge.parallel_id.is_some() {
        issues.push(issue(
          ModelIssueCode::InvalidApprovalGroup,
          format!(
            "Approval edge {:?} cannot also belong to a branch or parallel group.",
            edge.id
          ),
        ));
      }
    } else if matches!(edge.condition, EdgeCondition::Equals { .. }) {
      issues.push(issue(
        ModelIssueCode::InvalidApprovalGroup,
        format!("Equals edge {:?} must carry an approvalId.", edge.id),
      ));
    }
  }

  if workflow.schema_version < COMPILED_MODEL_SCHEMA_VERSION_V4 && !groups.is_empty() {
    issues.push(issue(
      ModelIssueCode::InvalidApprovalGroup,
      "Compiled model v1-v3 cannot contain model-v4 approval groups.",
    ));
  }

  for (approval_id, edges) in &groups {
    let join_id = format!("__woml_approval__{approval_id}__join");
    let wait = nodes.get(*approval_id).copied();
    let join = nodes.get(join_id.as_str()).copied();
    let approved_route_id = format!("{approval_id}:approved");
    let rejected_route_id = format!("{approval_id}:rejected");
    let approved_join_id = format!("{approval_id}:approved:join");
    let rejected_join_id = format!("{approval_id}:rejected:join");
    let approved_route = edges
      .iter()
      .copied()
      .find(|edge| edge.id == approved_route_id);
    let rejected_route = edges
      .iter()
      .copied()
      .find(|edge| edge.id == rejected_route_id);
    let approved_join = edges
      .iter()
      .copied()
      .find(|edge| edge.id == approved_join_id);
    let rejected_join = edges
      .iter()
      .copied()
      .find(|edge| edge.id == rejected_join_id);

    let route_is_valid = |route: Option<&CompiledWorkflowEdge>,
                          join: Option<&CompiledWorkflowEdge>,
                          decision: &str| {
      let Some(route) = route else {
        return false;
      };
      if route.from != **approval_id
        || route.approval_id.as_deref() != Some(*approval_id)
        || !is_approval_decision_condition(&route.condition, approval_id, decision)
      {
        return false;
      }
      if route.to == join_id {
        return join.is_none();
      }
      matches!(
        join,
        Some(join_edge)
          if join_edge.from != **approval_id
            && join_edge.to == join_id
            && join_edge.approval_id.as_deref() == Some(*approval_id)
            && matches!(join_edge.condition, EdgeCondition::Always)
      )
    };
    let routes_are_valid = route_is_valid(approved_route, approved_join, "approved")
      && route_is_valid(rejected_route, rejected_join, "rejected");
    let expected_edges = 2
      + usize::from(approved_route.is_some_and(|edge| edge.to != join_id))
      + usize::from(rejected_route.is_some_and(|edge| edge.to != join_id));
    let wait_outgoing: Vec<_> = workflow
      .graph
      .edges
      .iter()
      .filter(|edge| edge.from.as_str() == *approval_id)
      .collect();
    let join_incoming: Vec<_> = workflow
      .graph
      .edges
      .iter()
      .filter(|edge| edge.to == join_id)
      .collect();
    let boundaries_are_closed = wait_outgoing.len() == 2
      && wait_outgoing
        .iter()
        .all(|edge| edge.approval_id.as_deref() == Some(*approval_id))
      && join_incoming
        .iter()
        .all(|edge| edge.approval_id.as_deref() == Some(*approval_id));
    let has_notifications = wait.is_some_and(|node| {
      matches!(&node.inputs, ValueExpression::Object { fields } if fields.contains_key("notifications"))
    });
    if has_notifications && workflow.schema_version < COMPILED_MODEL_SCHEMA_VERSION_V5 {
      issues.push(issue(
        ModelIssueCode::InvalidNotificationGroup,
        format!("Approval {approval_id:?} notifications require compiled model v5 or later."),
      ));
    }

    if !valid_public_structural_id(approval_id)
      || !approval_wait_inputs(wait, &provider_ids)
      || !approval_join_inputs(join)
      || !routes_are_valid
      || edges.len() != expected_edges
      || !boundaries_are_closed
    {
      issues.push(issue(
        ModelIssueCode::InvalidApprovalGroup,
        format!(
          "Approval group {approval_id:?} does not match the frozen wait, decision-route, empty-arm, and join contract."
        ),
      ));
    }
  }

  for node in &workflow.graph.nodes {
    if node.handler == "engine.approval-wait" && !groups.contains_key(node.id.as_str()) {
      issues.push(issue(
        ModelIssueCode::InvalidApprovalGroup,
        format!("Approval wait {:?} has no matching edge group.", node.id),
      ));
    } else if node.handler == "engine.approval-join"
      && approval_join_id(&node.id).is_none_or(|approval_id| !groups.contains_key(approval_id))
    {
      issues.push(issue(
        ModelIssueCode::InvalidApprovalGroup,
        format!("Approval join {:?} has no matching edge group.", node.id),
      ));
    }
  }
}

fn empty_control_node(node: Option<&&CompiledWorkflowNode>, handler: &str) -> bool {
  node.is_some_and(|node| {
    node.handler == handler
      && node.timeout_ms.is_none()
      && node.retry_policy.is_none()
      && node.script_runtime.is_none()
      && node.metadata.is_none()
      && matches!(&node.inputs, ValueExpression::Object { fields } if fields.is_empty())
  })
}

fn inspect_fork_contract(workflow: &CompiledWorkflowDefinition, issues: &mut Vec<ModelIssue>) {
  let descriptors = (
    &workflow.graph.forks,
    &workflow.graph.choices,
    &workflow.graph.context_visibility,
    &workflow.graph.settlement,
  );
  if workflow.schema_version < COMPILED_MODEL_SCHEMA_VERSION_V13 {
    if !matches!(descriptors, (None, None, None, None)) {
      issues.push(issue(
        ModelIssueCode::InvalidForkGraph,
        "Fork graph metadata is available only in compiled Model v13 and later.",
      ));
    }
    return;
  }
  let (Some(forks), Some(choices), Some(visibility), Some(settlement)) = descriptors else {
    issues.push(issue(
      ModelIssueCode::InvalidForkGraph,
      "Model v13 requires forks, choices, contextVisibility, and settlement descriptors.",
    ));
    return;
  };
  let nodes: HashMap<&str, &CompiledWorkflowNode> = workflow
    .graph
    .nodes
    .iter()
    .map(|node| (node.id.as_str(), node))
    .collect();
  let mut incoming: HashMap<&str, Vec<&CompiledWorkflowEdge>> = HashMap::new();
  let mut outgoing: HashMap<&str, Vec<&CompiledWorkflowEdge>> = HashMap::new();
  for edge in &workflow.graph.edges {
    incoming.entry(edge.to.as_str()).or_default().push(edge);
    outgoing.entry(edge.from.as_str()).or_default().push(edge);
  }
  let mut fork_ids = HashSet::new();
  let mut described_fork_nodes = HashSet::new();
  let mut all_owned_terminals = Vec::new();
  let mut all_branch_route_nodes = HashSet::new();
  for fork in forks {
    let expected_open = format!("__woml_fork__{}__open", fork.fork_id);
    let expected_join = format!("__woml_fork__{}__join", fork.fork_id);
    let mut valid = valid_public_structural_id(&fork.fork_id)
      && fork_ids.insert(fork.fork_id.as_str())
      && fork.open_node_id == expected_open
      && fork.join_node_id == expected_join
      && empty_control_node(nodes.get(fork.open_node_id.as_str()), "engine.fork-open")
      && empty_control_node(nodes.get(fork.join_node_id.as_str()), "engine.fork-join")
      && !fork.branches.is_empty();
    described_fork_nodes.insert(fork.open_node_id.as_str());
    described_fork_nodes.insert(fork.join_node_id.as_str());
    let mut branch_ids = HashSet::new();
    let mut terminal_by_branch = HashMap::new();
    for branch in &fork.branches {
      let expected_terminal = format!(
        "__woml_fork__{}__{}__terminal",
        fork.fork_id, branch.branch_id
      );
      valid &= valid_public_structural_id(&branch.branch_id)
        && !matches!(branch.branch_id.as_str(), "all" | "none")
        && branch_ids.insert(branch.branch_id.as_str())
        && branch.terminal_node_id == expected_terminal
        && nodes.contains_key(branch.entry_node_id.as_str())
        && empty_control_node(
          nodes.get(branch.terminal_node_id.as_str()),
          "engine.fork-branch-terminal",
        );
      described_fork_nodes.insert(branch.terminal_node_id.as_str());
      all_owned_terminals.push(branch.terminal_node_id.as_str());
      terminal_by_branch.insert(branch.branch_id.as_str(), branch.terminal_node_id.as_str());
      let entry_incoming = incoming
        .get(branch.entry_node_id.as_str())
        .map(Vec::as_slice)
        .unwrap_or_default();
      valid &= entry_incoming.len() == 1
        && entry_incoming[0].from == fork.open_node_id
        && matches!(entry_incoming[0].condition, EdgeCondition::Always);

      let mut pending = vec![branch.entry_node_id.as_str()];
      let mut route_nodes = HashSet::new();
      let mut reached_terminal = false;
      while let Some(node_id) = pending.pop() {
        if !route_nodes.insert(node_id) {
          continue;
        }
        if node_id == branch.terminal_node_id {
          reached_terminal = true;
          continue;
        }
        for edge in outgoing.get(node_id).into_iter().flatten() {
          if edge.to != fork.join_node_id && edge.to != settlement.node_id {
            pending.push(edge.to.as_str());
          }
        }
      }
      valid &= reached_terminal && all_branch_route_nodes.is_disjoint(&route_nodes);
      all_branch_route_nodes.extend(route_nodes);
    }
    let canonical_joined: Vec<_> = fork
      .branches
      .iter()
      .filter(|branch| fork.joined_branch_ids.contains(&branch.branch_id))
      .map(|branch| branch.branch_id.as_str())
      .collect();
    let configured_joined: Vec<_> = fork.joined_branch_ids.iter().map(String::as_str).collect();
    valid &= configured_joined == canonical_joined
      && configured_joined.iter().collect::<HashSet<_>>().len() == configured_joined.len()
      && configured_joined.iter().all(|id| branch_ids.contains(id));
    let open_edges = outgoing
      .get(fork.open_node_id.as_str())
      .map(Vec::as_slice)
      .unwrap_or_default();
    valid &= fork.branches.iter().all(|branch| {
      open_edges.iter().any(|edge| {
        edge.to == branch.entry_node_id && matches!(edge.condition, EdgeCondition::Always)
      })
    });
    let join_sources: Vec<_> = incoming
      .get(fork.join_node_id.as_str())
      .into_iter()
      .flatten()
      .map(|edge| edge.from.as_str())
      .collect();
    let expected_join_sources: Vec<_> = if configured_joined.is_empty() {
      vec![fork.open_node_id.as_str()]
    } else {
      configured_joined
        .iter()
        .filter_map(|id| terminal_by_branch.get(id).copied())
        .collect()
    };
    valid &= join_sources == expected_join_sources
      && incoming
        .get(fork.join_node_id.as_str())
        .into_iter()
        .flatten()
        .all(|edge| matches!(edge.condition, EdgeCondition::Always));
    if !valid {
      issues.push(issue(
        ModelIssueCode::InvalidForkGraph,
        format!(
          "Fork {:?} does not match the frozen Model v13 ownership, route, and join contract.",
          fork.fork_id
        ),
      ));
    }
  }
  for node in &workflow.graph.nodes {
    if matches!(
      node.handler.as_str(),
      "engine.fork-open" | "engine.fork-branch-terminal" | "engine.fork-join"
    ) && !described_fork_nodes.contains(node.id.as_str())
    {
      issues.push(issue(
        ModelIssueCode::InvalidForkGraph,
        format!("Fork control node {:?} has no owner descriptor.", node.id),
      ));
    }
  }

  let mut choice_ids = HashSet::new();
  let mut described_choice_nodes = HashSet::new();
  for choice in choices {
    let is_string_choice = choice.string_selector.is_some();
    let string_contract_valid = if is_string_choice {
      matches!(
        choice.string_selector,
        Some(ValueExpression::ContextReference { ref path }) if !path.is_empty()
      ) && choice.string_cases.as_ref().is_some_and(|cases| {
        cases.len() + 1 == choice.arm_ids.len()
          && !cases.is_empty()
          && cases
            .iter()
            .enumerate()
            .all(|(index, case)| !case.value.is_empty() && case.arm_id == choice.arm_ids[index])
          && cases
            .iter()
            .map(|case| case.value.as_str())
            .collect::<HashSet<_>>()
            .len()
            == cases.len()
      }) && choice.default_arm_id.as_deref() == choice.arm_ids.last().map(String::as_str)
    } else {
      choice.string_cases.is_none()
        && choice.default_arm_id.is_none()
        && choice.result_node_id.is_none()
    };
    described_choice_nodes.insert(choice.selector_node_id.as_str());
    described_choice_nodes.insert(choice.join_node_id.as_str());
    if let Some(result_node_id) = &choice.result_node_id {
      described_choice_nodes.insert(result_node_id.as_str());
    }
    let selection = outgoing
      .get(choice.selector_node_id.as_str())
      .map(Vec::as_slice)
      .unwrap_or_default();
    let joins = incoming
      .get(choice.join_node_id.as_str())
      .map(Vec::as_slice)
      .unwrap_or_default();
    let mut valid = choice.choice_id.starts_with("__woml_choice__")
      && choice_ids.insert(choice.choice_id.as_str())
      && choice.selector_node_id == format!("{}__select", choice.choice_id)
      && choice.join_node_id == format!("{}__join", choice.choice_id)
      && empty_control_node(
        nodes.get(choice.selector_node_id.as_str()),
        "engine.choice-select",
      )
      && empty_control_node(
        nodes.get(choice.join_node_id.as_str()),
        "engine.choice-join",
      )
      && choice.arm_ids.len() >= 2
      && choice.arm_ids.iter().collect::<HashSet<_>>().len() == choice.arm_ids.len()
      && selection.len() == choice.arm_ids.len()
      && joins.len() == choice.arm_ids.len()
      && string_contract_valid;
    if let Some(result_node_id) = &choice.result_node_id {
      let result_node = nodes.get(result_node_id.as_str());
      let result_fields = result_node.and_then(|node| match &node.inputs {
        ValueExpression::Object { fields } => Some(fields),
        _ => None,
      });
      let result_incoming = incoming
        .get(result_node_id.as_str())
        .map(Vec::as_slice)
        .unwrap_or_default();
      valid &= is_string_choice
        && valid_public_structural_id(result_node_id)
        && result_node.is_some_and(|node| node.handler == "engine.choice-result")
        && result_fields.is_some_and(|fields| {
          fields.len() == choice.arm_ids.len()
            && choice.arm_ids.iter().all(|arm_id| {
              matches!(
                fields.get(arm_id),
                Some(ValueExpression::ContextReference { .. })
              )
            })
        })
        && result_incoming.len() == 1
        && result_incoming[0].from == choice.join_node_id
        && matches!(result_incoming[0].condition, EdgeCondition::Always);
    }
    for (index, arm_id) in choice.arm_ids.iter().enumerate() {
      let select = selection.get(index);
      let join = joins.get(index);
      valid &= arm_id.starts_with(&format!("{}:", choice.choice_id))
        && select.is_some_and(|edge| {
          edge.id == *arm_id
            && edge.branch_id.is_none()
            && edge.parallel_id.is_none()
            && edge.approval_id.is_none()
            && if is_string_choice {
              matches!(edge.condition, EdgeCondition::Always)
            } else if index + 1 == choice.arm_ids.len() {
              matches!(edge.condition, EdgeCondition::Always)
            } else {
              matches!(edge.condition, EdgeCondition::Boolean { .. })
            }
        })
        && join.is_some_and(|edge| {
          edge.id == format!("{arm_id}:join")
            && edge.to == choice.join_node_id
            && matches!(edge.condition, EdgeCondition::Always)
        });
    }
    if !valid {
      issues.push(issue(
        ModelIssueCode::InvalidControlChoice,
        format!(
          "Control choice {:?} does not match the frozen selector, ordered-arm, and join contract.",
          choice.choice_id
        ),
      ));
    }
  }
  for node in &workflow.graph.nodes {
    if matches!(
      node.handler.as_str(),
      "engine.choice-select" | "engine.choice-join" | "engine.choice-result"
    ) && !described_choice_nodes.contains(node.id.as_str())
    {
      issues.push(issue(
        ModelIssueCode::InvalidControlChoice,
        format!("Control-choice node {:?} has no descriptor.", node.id),
      ));
    }
  }

  let script_nodes: Vec<_> = workflow
    .graph
    .nodes
    .iter()
    .filter(|node| node.handler == "runtime.script")
    .collect();
  let visibility_valid = visibility.len() == script_nodes.len()
    && visibility.iter().zip(script_nodes).all(|(item, node)| {
      item.node_id == node.id
        && item.step_ids.iter().collect::<HashSet<_>>().len() == item.step_ids.len()
        && item
          .step_ids
          .iter()
          .all(|id| valid_public_structural_id(id) && nodes.contains_key(id.as_str()))
    });
  if !visibility_valid {
    issues.push(issue(
      ModelIssueCode::InvalidContextVisibility,
      "Model v13 contextVisibility must describe every runtime.script node once in graph order.",
    ));
  }

  let settlement_incoming = incoming
    .get(settlement.node_id.as_str())
    .map(Vec::as_slice)
    .unwrap_or_default();
  let owned: HashSet<_> = all_owned_terminals.iter().copied().collect();
  let main_edges = settlement_incoming
    .iter()
    .filter(|edge| !owned.contains(edge.from.as_str()))
    .count();
  let settlement_valid = settlement.node_id == "__woml_workflow__settlement"
    && empty_control_node(
      nodes.get(settlement.node_id.as_str()),
      "engine.workflow-settlement",
    )
    && nodes.contains_key(settlement.main_result_node_id.as_str())
    && settlement
      .owned_branch_terminal_node_ids
      .iter()
      .map(String::as_str)
      .eq(all_owned_terminals.iter().copied())
    && main_edges == 1
    && settlement_incoming.len() == all_owned_terminals.len() + 1
    && settlement_incoming
      .iter()
      .all(|edge| matches!(edge.condition, EdgeCondition::Always))
    && outgoing
      .get(settlement.node_id.as_str())
      .is_none_or(Vec::is_empty);
  if !settlement_valid {
    issues.push(issue(
      ModelIssueCode::InvalidWorkflowSettlement,
      "Model v13 settlement must preserve one main result and depend on the main continuation plus every owned branch terminal.",
    ));
  }
}

impl CompiledWorkflowDefinition {
  pub fn runtime_policy_hash(&self) -> Option<String> {
    let policy = self.runtime_policy.as_ref()?;
    let bytes = canonical_json(policy).ok()?;
    Some(format!("sha256:{:x}", Sha256::digest(bytes)))
  }

  pub fn runtime_policy_queue_name(&self) -> Option<String> {
    let policy = self.runtime_policy.as_ref()?;
    Some(policy.queue.as_ref().map_or_else(
      || {
        let digest = Sha256::digest(self.workflow_id.as_bytes());
        format!("workflow-{:x}", digest)
      },
      |queue| queue.name.clone(),
    ))
  }

  pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
    serde_json::from_str(json)
  }

  pub fn node(&self, node_id: &str) -> Option<&CompiledWorkflowNode> {
    self.graph.nodes.iter().find(|node| node.id == node_id)
  }

  pub fn trigger(&self, trigger_id: &str) -> Option<&CompiledTrigger> {
    self
      .triggers
      .iter()
      .find(|trigger| trigger.id == trigger_id)
  }

  pub fn lifecycle_hook(&self, hook_id: &str) -> Option<&CompiledLifecycleHook> {
    self
      .lifecycle
      .as_ref()?
      .hooks
      .iter()
      .find(|hook| hook.hook_id == hook_id)
  }

  pub fn lifecycle_hook_for_event(
    &self,
    event: LifecycleEventName,
  ) -> Option<&CompiledLifecycleHook> {
    self
      .lifecycle
      .as_ref()?
      .hooks
      .iter()
      .find(|hook| hook.event == event)
  }

  pub fn lifecycle_hook_for_step_event(
    &self,
    event: LifecycleEventName,
    step_id: &str,
  ) -> Option<&CompiledLifecycleHook> {
    let hook = self.lifecycle_hook_for_event(event)?;
    if !event.is_step()
      || self
        .node(step_id)
        .is_none_or(|node| node.handler != "runtime.script")
      || hook
        .step_ids
        .as_ref()
        .is_some_and(|step_ids| !step_ids.iter().any(|id| id == step_id))
    {
      return None;
    }
    Some(hook)
  }

  pub(crate) fn approval(&self, approval_id: &str) -> Option<ApprovalDefinition> {
    let wait = self.node(approval_id)?;
    if wait.handler != "engine.approval-wait" {
      return None;
    }
    let ValueExpression::Object { fields } = &wait.inputs else {
      return None;
    };
    let timeout_ms = fields
      .get("timeoutMs")
      .and_then(|expression| match expression {
        ValueExpression::Literal { value } => value.as_u64(),
        _ => None,
      });
    let on_timeout = fields
      .get("onTimeout")
      .and_then(|expression| match expression {
        ValueExpression::Literal { value } => value.as_str().map(str::to_string),
        _ => None,
      })?;
    let provider_ids = reusable_provider_ids(self);
    let notifications = approval_notifications(approval_id, fields, &provider_ids)?;
    Some(ApprovalDefinition {
      approval_id: approval_id.to_string(),
      name: wait
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string),
      description: wait
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("description"))
        .and_then(Value::as_str)
        .map(str::to_string),
      timeout_ms,
      on_timeout,
      notifications,
    })
  }

  pub(crate) fn parallel_group(&self, parallel_id: &str) -> Option<ParallelGroupDefinition> {
    let start_node_id = format!("__woml_parallel__{parallel_id}__start");
    let start = self.node(&start_node_id)?;
    if start.handler != "engine.parallel-start" {
      return None;
    }
    let ValueExpression::Object { fields } = &start.inputs else {
      return None;
    };
    let concurrency = fields
      .get("concurrency")
      .and_then(|expression| match expression {
        ValueExpression::Literal { value } => {
          value.as_u64().and_then(|value| usize::try_from(value).ok())
        }
        _ => None,
      })?;
    let on_error = fields
      .get("onError")
      .and_then(|expression| match expression {
        ValueExpression::Literal { value } => value.as_str().map(str::to_string),
        _ => None,
      })?;
    let mut child_edges = self
      .graph
      .edges
      .iter()
      .filter(|edge| edge.from == start_node_id && edge.parallel_id.as_deref() == Some(parallel_id))
      .collect::<Vec<_>>();
    child_edges.sort_by_key(|edge| {
      edge
        .id
        .strip_prefix(&format!("{parallel_id}:child:"))
        .and_then(|index| index.parse::<usize>().ok())
        .unwrap_or(usize::MAX)
    });
    Some(ParallelGroupDefinition {
      parallel_id: parallel_id.to_string(),
      start_node_id,
      child_node_ids: child_edges.iter().map(|edge| edge.to.clone()).collect(),
      concurrency,
      on_error,
    })
  }

  pub(crate) fn parallel_group_for_child(&self, node_id: &str) -> Option<ParallelGroupDefinition> {
    let parallel_id = self
      .graph
      .edges
      .iter()
      .find(|edge| edge.to == node_id && edge.parallel_id.is_some())?
      .parallel_id
      .as_deref()?;
    self.parallel_group(parallel_id)
  }

  pub(crate) fn fork_branch_owner(
    &self,
    node_id: &str,
  ) -> Option<(&CompiledFork, &CompiledForkBranch)> {
    for fork in self.graph.forks.as_deref().unwrap_or_default() {
      for branch in &fork.branches {
        let mut visited = HashSet::new();
        let mut pending = vec![branch.entry_node_id.as_str()];
        while let Some(candidate) = pending.pop() {
          if !visited.insert(candidate) {
            continue;
          }
          if candidate == node_id {
            return Some((fork, branch));
          }
          if candidate == branch.terminal_node_id {
            continue;
          }
          pending.extend(
            self
              .graph
              .edges
              .iter()
              .filter(|edge| edge.from == candidate)
              .map(|edge| edge.to.as_str()),
          );
        }
      }
    }
    None
  }

  pub fn terminal_node_id(&self) -> Option<&str> {
    let mut outgoing: HashMap<&str, usize> = self
      .graph
      .nodes
      .iter()
      .map(|node| (node.id.as_str(), 0))
      .collect();
    for edge in &self.graph.edges {
      if let Some(count) = outgoing.get_mut(edge.from.as_str()) {
        *count += 1;
      }
    }
    let mut terminals = outgoing
      .into_iter()
      .filter_map(|(id, count)| (count == 0).then_some(id));
    let first = terminals.next()?;
    terminals.next().is_none().then_some(first)
  }

  pub fn validate_for_execution(&self) -> Result<(), ModelValidationError> {
    let mut issues = self.inspect_structure();
    self.inspect_executable_profile(&mut issues, false, false, false);
    if issues.is_empty() {
      Ok(())
    } else {
      Err(ModelValidationError::new(issues))
    }
  }

  pub fn validate_for_durable_execution(&self) -> Result<(), ModelValidationError> {
    let mut issues = self.inspect_structure();
    self.inspect_executable_profile(&mut issues, true, true, true);
    if issues.is_empty() {
      Ok(())
    } else {
      Err(ModelValidationError::new(issues))
    }
  }

  pub fn validate_structure(&self) -> Result<(), ModelValidationError> {
    let issues = self.inspect_structure();
    if issues.is_empty() {
      Ok(())
    } else {
      Err(ModelValidationError::new(issues))
    }
  }

  pub fn inspect_structure(&self) -> Vec<ModelIssue> {
    let mut issues = Vec::new();
    if !matches!(
      self.schema_version,
      COMPILED_MODEL_SCHEMA_VERSION_V1
        | COMPILED_MODEL_SCHEMA_VERSION_V2
        | COMPILED_MODEL_SCHEMA_VERSION_V3
        | COMPILED_MODEL_SCHEMA_VERSION_V4
        | COMPILED_MODEL_SCHEMA_VERSION_V5
        | COMPILED_MODEL_SCHEMA_VERSION_V6
        | COMPILED_MODEL_SCHEMA_VERSION_V7
        | COMPILED_MODEL_SCHEMA_VERSION_V8
        | COMPILED_MODEL_SCHEMA_VERSION_V9
        | COMPILED_MODEL_SCHEMA_VERSION_V10
        | COMPILED_MODEL_SCHEMA_VERSION_V11
        | COMPILED_MODEL_SCHEMA_VERSION_V12
        | COMPILED_MODEL_SCHEMA_VERSION_V13
        | COMPILED_MODEL_SCHEMA_VERSION_V14
    ) {
      issues.push(issue(
        ModelIssueCode::UnsupportedSchemaVersion,
        format!(
          "Unsupported compiled workflow schema version {}.",
          self.schema_version
        ),
      ));
    }
    if !valid_id(&self.workflow_id) {
      issues.push(issue(
        ModelIssueCode::InvalidIdentifier,
        "workflowId must contain between 1 and 256 characters.",
      ));
    }
    if let Some(metadata) = &self.metadata {
      if metadata.name.as_deref() == Some("") {
        issues.push(issue(
          ModelIssueCode::InvalidMetadata,
          "metadata.name must not be empty when present.",
        ));
      }
      if metadata.version.as_deref() == Some("") {
        issues.push(issue(
          ModelIssueCode::InvalidMetadata,
          "metadata.version must not be empty when present.",
        ));
      }
    }

    match (self.schema_version, &self.module_runtime) {
      (
        COMPILED_MODEL_SCHEMA_VERSION_V9
        | COMPILED_MODEL_SCHEMA_VERSION_V10
        | COMPILED_MODEL_SCHEMA_VERSION_V11
        | COMPILED_MODEL_SCHEMA_VERSION_V12
        | COMPILED_MODEL_SCHEMA_VERSION_V13
        | COMPILED_MODEL_SCHEMA_VERSION_V14,
        Some(runtime),
      ) => {
        let reserved = [
          "http",
          "db",
          "storage",
          "cache",
          "events",
          "queue",
          "workflows",
        ];
        let valid = runtime.profile_version == 1
          && (1..=64).contains(&runtime.modules.len())
          && runtime.modules.windows(2).all(|pair| pair[0].name < pair[1].name)
          && runtime.modules.iter().all(|module| {
            let mut alias = module.name.chars();
            let valid_alias = matches!(alias.next(), Some(first) if first.is_ascii_lowercase())
              && alias.all(|character| character.is_ascii_alphanumeric())
              && module.name.len() <= 128
              && !reserved.contains(&module.name.as_str());
            let valid_digest = |value: &str| {
              value.len() == 71
                && value.starts_with("sha256:")
                && value[7..].bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            };
            valid_alias
              && valid_digest(&module.bundle_digest)
              && valid_digest(&module.source_map_digest)
              && !module.exports.is_empty()
              && module.exports.windows(2).all(|pair| pair[0] < pair[1])
              && module.exports.iter().all(|name| {
                let mut characters = name.chars();
                matches!(characters.next(), Some(first) if first == '_' || first == '$' || first.is_ascii_alphabetic())
                  && characters.all(|character| character == '_' || character == '$' || character.is_ascii_alphanumeric())
              })
          });
        if !valid {
          issues.push(issue(
            ModelIssueCode::InvalidModuleRuntime,
            "Model v9 moduleRuntime does not match the frozen Module Runtime v1 contract.",
          ));
        }
      }
      (COMPILED_MODEL_SCHEMA_VERSION_V9, None) => issues.push(issue(
        ModelIssueCode::InvalidModuleRuntime,
        "Model v9 requires moduleRuntime.",
      )),
      (version, Some(_)) if version < COMPILED_MODEL_SCHEMA_VERSION_V9 => issues.push(issue(
        ModelIssueCode::InvalidModuleRuntime,
        "moduleRuntime is unavailable before Model v9.",
      )),
      _ => {}
    }

    if self.triggers.is_empty()
      && !matches!(
        self.schema_version,
        COMPILED_MODEL_SCHEMA_VERSION_V10
          | COMPILED_MODEL_SCHEMA_VERSION_V11
          | COMPILED_MODEL_SCHEMA_VERSION_V12
          | COMPILED_MODEL_SCHEMA_VERSION_V13
          | COMPILED_MODEL_SCHEMA_VERSION_V14
      )
    {
      issues.push(issue(
        ModelIssueCode::MissingTrigger,
        "A compiled workflow requires at least one trigger.",
      ));
    }
    if self.schema_version == COMPILED_MODEL_SCHEMA_VERSION_V10 && !self.triggers.is_empty() {
      issues.push(issue(
        ModelIssueCode::UnsupportedTrigger,
        "Compiled Model v10 is the call-only profile and requires an empty triggers array.",
      ));
    }
    inspect_lifecycle_contract(self, &mut issues);
    inspect_runtime_policy_contract(self, &mut issues);
    let mut trigger_ids = HashSet::new();
    for trigger in &self.triggers {
      if !valid_id(&trigger.id)
        || trigger.handler.is_empty()
        || trigger.handler.chars().count() > 512
      {
        issues.push(issue(
          ModelIssueCode::InvalidIdentifier,
          format!("Trigger {:?} has an invalid ID or handler.", trigger.id),
        ));
      }
      if !trigger_ids.insert(trigger.id.as_str()) {
        issues.push(issue(
          ModelIssueCode::DuplicateTriggerId,
          format!("Compiled workflow repeats trigger ID {:?}.", trigger.id),
        ));
      }
      inspect_expression(&trigger.config, "trigger.config", &mut issues);
      if contains_lifecycle_reference(&trigger.config) {
        issues.push(issue(
          ModelIssueCode::InvalidLifecycle,
          format!(
            "Trigger {:?} cannot read lifecycle; the binding exists only inside lifecycle actions.",
            trigger.id
          ),
        ));
      }
      if self.schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V7 && !valid_model_v7_trigger(trigger)
      {
        issues.push(issue(
          ModelIssueCode::UnsupportedTrigger,
          format!(
            "Trigger {:?} does not match a frozen Model v7 trigger contract.",
            trigger.id
          ),
        ));
      }
    }

    if self.graph.nodes.is_empty() {
      issues.push(issue(
        ModelIssueCode::MissingNode,
        "A compiled workflow graph requires at least one node.",
      ));
    }
    if self.graph.entry_node_ids.is_empty() {
      issues.push(issue(
        ModelIssueCode::MissingEntryNode,
        "A compiled workflow graph requires at least one entry node.",
      ));
    }

    let reusable_step_nodes = inspect_reusable_contract(self, &mut issues);
    let mut node_ids = HashSet::new();
    for node in &self.graph.nodes {
      if !valid_id(&node.id) || node.handler.is_empty() || node.handler.chars().count() > 512 {
        issues.push(issue(
          ModelIssueCode::InvalidIdentifier,
          format!("Node {:?} has an invalid ID or handler.", node.id),
        ));
      }
      if !node_ids.insert(node.id.as_str()) {
        issues.push(issue(
          ModelIssueCode::DuplicateNodeId,
          format!("Compiled graph contains duplicate node ID {:?}.", node.id),
        ));
      }
      if node.timeout_ms == Some(0) {
        issues.push(issue(
          ModelIssueCode::UnsupportedTimeout,
          format!("Node {:?} has an invalid zero timeout.", node.id),
        ));
      }
      if self.schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V8 && node.timeout_ms.is_some() {
        issues.push(issue(
          ModelIssueCode::UnsupportedTimeout,
          format!(
            "Model v8 node {:?} cannot carry timeoutMs in the frozen contract.",
            node.id
          ),
        ));
      }
      if let Some(retry) = &node.retry_policy {
        if self.schema_version < COMPILED_MODEL_SCHEMA_VERSION_V6 {
          issues.push(issue(
            ModelIssueCode::UnsupportedRetry,
            format!(
              "Node {:?} cannot carry retryPolicy before compiled model v6.",
              node.id
            ),
          ));
        } else {
          let valid_handler = node.handler == "runtime.script";
          let valid_attempts = (2..=RetryPolicy::MAX_ATTEMPTS).contains(&retry.max_attempts);
          let valid_backoff = match retry.backoff {
            BackoffPolicy::None => false,
            BackoffPolicy::Fixed { delay_ms } => {
              (1..=RetryPolicy::MAX_DELAY_MS).contains(&delay_ms)
            }
            BackoffPolicy::Exponential {
              initial_delay_ms,
              multiplier,
              maximum_delay_ms,
            } => {
              let maximum_delay_ms = maximum_delay_ms.unwrap_or(0);
              multiplier == 2.0
                && (1..=RetryPolicy::MAX_DELAY_MS).contains(&initial_delay_ms)
                && (initial_delay_ms..=RetryPolicy::MAX_DELAY_MS).contains(&maximum_delay_ms)
            }
          };
          if !valid_handler || !valid_attempts || !valid_backoff {
            issues.push(issue(
              ModelIssueCode::UnsupportedRetry,
              format!(
                "Node {:?} does not match the frozen Model v6 runtime.script retry contract.",
                node.id
              ),
            ));
          }
        }
      }
      match (
        self.schema_version,
        node.handler.as_str(),
        &node.script_runtime,
      ) {
        (version, "runtime.script", Some(runtime))
          if version >= COMPILED_MODEL_SCHEMA_VERSION_V8 =>
        {
          let reusable = reusable_step_nodes.contains(&node.id);
          let expected: &[&str] = if reusable {
            &["props", "context", "attempt", "services"]
          } else {
            &["context", "attempt", "services", "secrets"]
          };
          let valid_secrets = runtime.required_secrets.len() <= 64
            && runtime
              .required_secrets
              .windows(2)
              .all(|pair| pair[0] < pair[1])
            && runtime.required_secrets.iter().all(|name| {
              let mut chars = name.chars();
              matches!(chars.next(), Some(first) if first.is_ascii_uppercase())
                && chars.all(|character| {
                  character == '_' || character.is_ascii_uppercase() || character.is_ascii_digit()
                })
                && name.len() <= 128
            });
          if runtime.binding_version != if reusable { 3 } else { 1 }
            || runtime
              .bindings
              .iter()
              .map(String::as_str)
              .ne(expected.iter().copied())
            || !valid_secrets
          {
            issues.push(issue(
              ModelIssueCode::InvalidScriptRuntime,
              format!(
                "Node {:?} does not match its frozen scriptRuntime contract.",
                node.id
              ),
            ));
          }
        }
        (version, "runtime.script", None) if version >= COMPILED_MODEL_SCHEMA_VERSION_V8 => issues
          .push(issue(
            ModelIssueCode::InvalidScriptRuntime,
            format!(
              "Model v8 runtime.script node {:?} requires scriptRuntime.",
              node.id
            ),
          )),
        (version, _, Some(_)) if version >= COMPILED_MODEL_SCHEMA_VERSION_V8 => issues.push(issue(
          ModelIssueCode::InvalidScriptRuntime,
          format!(
            "Only Model v8 runtime.script nodes may carry scriptRuntime (node {:?}).",
            node.id
          ),
        )),
        (version, _, Some(_)) if version < COMPILED_MODEL_SCHEMA_VERSION_V8 => issues.push(issue(
          ModelIssueCode::InvalidScriptRuntime,
          format!(
            "Node {:?} cannot carry scriptRuntime before Model v8.",
            node.id
          ),
        )),
        _ => {}
      }
      inspect_expression(
        &node.inputs,
        &format!("node[{}].inputs", node.id),
        &mut issues,
      );
      if contains_lifecycle_reference(&node.inputs) {
        issues.push(issue(
          ModelIssueCode::InvalidLifecycle,
          format!(
            "Business node {:?} cannot read lifecycle; lifecycle outputs never enter the DAG.",
            node.id
          ),
        ));
      }
    }

    let mut adjacency: HashMap<&str, Vec<&str>> = self
      .graph
      .nodes
      .iter()
      .map(|node| (node.id.as_str(), Vec::new()))
      .collect();
    let mut incoming: HashMap<&str, usize> = self
      .graph
      .nodes
      .iter()
      .map(|node| (node.id.as_str(), 0))
      .collect();
    let mut edge_ids = HashSet::new();
    for edge in &self.graph.edges {
      if !valid_id(&edge.id) || !valid_id(&edge.from) || !valid_id(&edge.to) {
        issues.push(issue(
          ModelIssueCode::InvalidIdentifier,
          format!("Edge {:?} has an invalid ID or endpoint.", edge.id),
        ));
      }
      if !edge_ids.insert(edge.id.as_str()) {
        issues.push(issue(
          ModelIssueCode::DuplicateEdgeId,
          format!("Compiled graph contains duplicate edge ID {:?}.", edge.id),
        ));
      }
      if !node_ids.contains(edge.from.as_str()) || !node_ids.contains(edge.to.as_str()) {
        issues.push(issue(
          ModelIssueCode::UnknownEdgeEndpoint,
          format!(
            "Edge {:?} references an unknown endpoint ({} -> {}).",
            edge.id, edge.from, edge.to
          ),
        ));
        continue;
      }
      adjacency
        .entry(edge.from.as_str())
        .or_default()
        .push(edge.to.as_str());
      *incoming.entry(edge.to.as_str()).or_default() += 1;
    }

    let mut entry_ids = HashSet::new();
    for entry_id in &self.graph.entry_node_ids {
      if !entry_ids.insert(entry_id.as_str()) {
        issues.push(issue(
          ModelIssueCode::DuplicateEntryNodeId,
          format!("Compiled graph repeats entry node ID {entry_id:?}."),
        ));
        continue;
      }
      if !node_ids.contains(entry_id.as_str()) {
        issues.push(issue(
          ModelIssueCode::InvalidEntryNode,
          format!("Entry node {entry_id:?} does not exist."),
        ));
      } else if incoming.get(entry_id.as_str()).copied().unwrap_or(0) != 0 {
        issues.push(issue(
          ModelIssueCode::InvalidEntryNode,
          format!("Entry node {entry_id:?} has an incoming edge."),
        ));
      }
    }

    let mut reachable = HashSet::new();
    let mut queue: VecDeque<&str> = self
      .graph
      .entry_node_ids
      .iter()
      .map(String::as_str)
      .filter(|id| node_ids.contains(id))
      .collect();
    while let Some(id) = queue.pop_front() {
      if !reachable.insert(id) {
        continue;
      }
      queue.extend(adjacency.get(id).into_iter().flatten().copied());
    }
    for node in &self.graph.nodes {
      if !reachable.contains(node.id.as_str()) {
        issues.push(issue(
          ModelIssueCode::UnreachableNode,
          format!("Node {:?} is not reachable from an entry node.", node.id),
        ));
      }
    }

    let mut remaining_incoming = incoming.clone();
    let mut topological: VecDeque<&str> = self
      .graph
      .nodes
      .iter()
      .map(|node| node.id.as_str())
      .filter(|id| remaining_incoming.get(id).copied().unwrap_or(0) == 0)
      .collect();
    let mut visited = 0;
    while let Some(id) = topological.pop_front() {
      visited += 1;
      for next in adjacency.get(id).into_iter().flatten() {
        let count = remaining_incoming.entry(next).or_default();
        *count -= 1;
        if *count == 0 {
          topological.push_back(next);
        }
      }
    }
    if visited != node_ids.len() {
      issues.push(issue(
        ModelIssueCode::CyclicGraph,
        "Compiled graph contains a cycle.",
      ));
    }

    let terminal_count = self
      .graph
      .nodes
      .iter()
      .filter(|node| adjacency.get(node.id.as_str()).is_none_or(Vec::is_empty))
      .count();
    if terminal_count != 1 {
      issues.push(issue(
                ModelIssueCode::TerminalNodeCount,
                format!(
                    "The first executable profile requires exactly one terminal node; found {terminal_count}."
                ),
            ));
    }
    inspect_branch_contract(self, &mut issues);
    inspect_parallel_contract(self, &mut issues);
    inspect_approval_contract(self, &mut issues);
    inspect_fork_contract(self, &mut issues);
    issues
  }

  fn inspect_executable_profile(
    &self,
    issues: &mut Vec<ModelIssue>,
    allow_approval: bool,
    allow_retry: bool,
    durable: bool,
  ) {
    if self.reusable_definitions.as_ref().is_some_and(|items| {
      items
        .iter()
        .any(|item| matches!(item, CompiledReusableInvocation::Step { .. }))
    }) {
      issues.push(issue(
        ModelIssueCode::UnsupportedReusableExecution,
        "Model v14 custom steps are compiled and validated, but durable execution begins in SCP4.",
      ));
    }
    if self.schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V13 {
      if !durable {
        issues.push(issue(
          ModelIssueCode::UnsupportedRuntimePolicyExecution,
          "Compiled Model v13 requires the durable runtime-policy scheduler.",
        ));
      }
    }
    if self.schema_version == COMPILED_MODEL_SCHEMA_VERSION_V12 && !durable {
      issues.push(issue(
        ModelIssueCode::UnsupportedRuntimePolicyExecution,
        "Compiled Model v12 requires the durable runtime-policy scheduler.",
      ));
    }
    for trigger in &self.triggers {
      let is_empty_object = matches!(
          &trigger.config,
          ValueExpression::Object { fields } if fields.is_empty()
      );
      let executable = if self.schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V7 {
        valid_model_v7_trigger(trigger)
      } else {
        trigger.handler == "trigger.manual" && is_empty_object
      };
      if !executable {
        issues.push(issue(
          ModelIssueCode::UnsupportedTrigger,
          format!(
            "Trigger {:?} is not executable in the current compiled-model profile.",
            trigger.id
          ),
        ));
      }
    }

    let provider_ids = reusable_provider_ids(self);
    for node in &self.graph.nodes {
      let valid_inputs = match (node.handler.as_str(), &node.inputs) {
        ("runtime.script", ValueExpression::Object { fields }) if fields.len() == 1 => {
          matches!(fields.get("source"), Some(ValueExpression::Literal { value }) if value.is_string())
        }
        ("engine.branch-select", ValueExpression::Object { fields }) => fields.is_empty(),
        ("engine.branch-result", ValueExpression::Object { fields }) => {
          !fields.is_empty()
            && fields
              .values()
              .all(|value| matches!(value, ValueExpression::ContextReference { .. }))
        }
        ("engine.parallel-start", ValueExpression::Object { .. }) => {
          parallel_start_inputs(Some(node), usize::MAX)
        }
        ("engine.parallel-join", ValueExpression::Object { fields }) => fields.is_empty(),
        ("engine.approval-wait", ValueExpression::Object { .. }) if allow_approval => {
          approval_wait_inputs(Some(node), &provider_ids)
        }
        ("engine.approval-join", ValueExpression::Object { fields }) if allow_approval => {
          fields.is_empty()
        }
        (
          "engine.fork-open"
          | "engine.fork-branch-terminal"
          | "engine.fork-join"
          | "engine.workflow-settlement"
          | "engine.choice-select"
          | "engine.choice-join",
          ValueExpression::Object { fields },
        ) if self.schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V13 => fields.is_empty(),
        ("engine.choice-result", ValueExpression::Object { fields })
          if self.schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V14 =>
        {
          !fields.is_empty()
            && fields
              .values()
              .all(|value| matches!(value, ValueExpression::ContextReference { .. }))
        }
        (
          "runtime.script"
          | "engine.branch-select"
          | "engine.branch-result"
          | "engine.parallel-start"
          | "engine.parallel-join",
          _,
        ) => false,
        _ => {
          issues.push(issue(
            ModelIssueCode::UnknownHandler,
            format!(
              "No executable WOML handler is registered for {:?} on node {:?}.",
              node.handler, node.id
            ),
          ));
          true
        }
      };
      if !valid_inputs {
        issues.push(issue(
          ModelIssueCode::UnsupportedNodeInputs,
          format!(
            "Node {:?} does not match the input contract for handler {:?}.",
            node.id, node.handler
          ),
        ));
      }
      if node.retry_policy.is_some()
        && !(allow_retry
          && self.schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V6
          && node.handler == "runtime.script")
      {
        issues.push(issue(
          ModelIssueCode::UnsupportedRetry,
          format!(
            "Retry is not executable in the current profile (node {:?}).",
            node.id
          ),
        ));
      }
      if node.timeout_ms.is_some() {
        issues.push(issue(
          ModelIssueCode::UnsupportedTimeout,
          format!(
            "Per-node timeout is not executable in the current profile (node {:?}).",
            node.id
          ),
        ));
      }
    }

    for edge in &self.graph.edges {
      let executable_condition = matches!(edge.condition, EdgeCondition::Always)
        || matches!(edge.condition, EdgeCondition::Boolean { .. })
          && (edge.branch_id.is_some() || self.schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V13)
        || allow_approval
          && matches!(edge.condition, EdgeCondition::Equals { .. })
          && edge.approval_id.is_some();
      if !executable_condition {
        issues.push(issue(
          ModelIssueCode::UnsupportedEdgeCondition,
          format!(
            "Edge {:?} uses a condition outside the executable WOML profile.",
            edge.id
          ),
        ));
      }
    }

    let mut incoming: HashMap<&str, usize> = self
      .graph
      .nodes
      .iter()
      .map(|node| (node.id.as_str(), 0))
      .collect();
    let mut outgoing = incoming.clone();
    for edge in &self.graph.edges {
      if let Some(count) = outgoing.get_mut(edge.from.as_str()) {
        *count += 1;
      }
      if let Some(count) = incoming.get_mut(edge.to.as_str()) {
        *count += 1;
      }
    }
    let topology_is_sequential_or_branching = self.graph.entry_node_ids.len() == 1
      && self.graph.nodes.iter().all(|node| {
        let incoming_count = incoming.get(node.id.as_str()).copied().unwrap_or(0);
        let outgoing_count = outgoing.get(node.id.as_str()).copied().unwrap_or(0);
        (incoming_count <= 1
          || node.handler == "engine.branch-result"
          || node.handler == "engine.parallel-join"
          || self.schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V13
            && matches!(
              node.handler.as_str(),
              "engine.fork-join" | "engine.choice-join" | "engine.workflow-settlement"
            )
          || allow_approval && node.handler == "engine.approval-join")
          && (outgoing_count <= 1
            || node.handler == "engine.branch-select"
            || node.handler == "engine.parallel-start"
            || self.schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V13
              && matches!(
                node.handler.as_str(),
                "engine.fork-open" | "engine.fork-branch-terminal" | "engine.choice-select"
              )
            || allow_approval && node.handler == "engine.approval-wait")
      });
    if !topology_is_sequential_or_branching {
      issues.push(issue(
        ModelIssueCode::UnsupportedNonSequentialDag,
        "The executable profile supports only frozen sequential, branch, and parallel DAG shapes; unrelated fan-out remains staged.",
      ));
    }
  }
}

impl CompiledWorkflowNode {
  pub fn script_source(&self) -> Option<&str> {
    let ValueExpression::Object { fields } = &self.inputs else {
      return None;
    };
    let ValueExpression::Literal { value } = fields.get("source")? else {
      return None;
    };
    value.as_str()
  }
}
