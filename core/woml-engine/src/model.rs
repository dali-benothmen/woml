use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

use crate::{
  COMPILED_MODEL_SCHEMA_VERSION_V1, COMPILED_MODEL_SCHEMA_VERSION_V2,
  COMPILED_MODEL_SCHEMA_VERSION_V3, COMPILED_MODEL_SCHEMA_VERSION_V4,
  COMPILED_MODEL_SCHEMA_VERSION_V5, COMPILED_MODEL_SCHEMA_VERSION_V6,
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
  pub metadata: Option<Map<String, Value>>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApprovalDefinition {
  pub approval_id: String,
  pub name: Option<String>,
  pub description: Option<String>,
  pub timeout_ms: Option<u64>,
  pub on_timeout: String,
  pub notifications: Vec<NotificationDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationDefinition {
  pub delivery_id: String,
  pub provider: String,
  pub destination: String,
  pub credentials: BTreeMap<String, String>,
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
        if let TemplatePart::ContextReference { path } = part {
          if path.is_empty() || path.iter().any(|segment| segment.is_empty()) {
            issues.push(issue(
              ModelIssueCode::InvalidValueExpression,
              format!(
                "Context reference at {at}.parts[{index}] must contain non-empty path segments."
              ),
            ));
          }
        }
      }
    }
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
    } else if matches!(edge.condition, EdgeCondition::Boolean { .. }) {
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
    if fields.len() != 4
      || !["deliveryId", "provider", "destination", "credentials"]
        .iter()
        .all(|key| fields.contains_key(*key))
    {
      return None;
    }
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
    if provider != "slack" || !valid_slack_destination(destination) || credentials.len() != 2 {
      return None;
    }
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
      "{}\0{}\0{}",
      names["botToken"], names["appToken"], destination
    );
    if !duplicate_keys.insert(duplicate_key) {
      return None;
    }
    notifications.push(NotificationDefinition {
      delivery_id: delivery_id.to_string(),
      provider: provider.to_string(),
      destination: destination.to_string(),
      credentials: names,
    });
    previous_tag = Some(tag);
    previous_channel = Some(channel);
  }
  Some(notifications)
}

fn approval_wait_inputs(node: Option<&CompiledWorkflowNode>) -> bool {
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
  let valid_notifications = approval_notifications(&node.id, fields).is_some();
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
      || !approval_wait_inputs(wait)
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

impl CompiledWorkflowDefinition {
  pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
    serde_json::from_str(json)
  }

  pub fn node(&self, node_id: &str) -> Option<&CompiledWorkflowNode> {
    self.graph.nodes.iter().find(|node| node.id == node_id)
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
    let notifications = approval_notifications(approval_id, fields)?;
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
    self.inspect_executable_profile(&mut issues, false, false);
    if issues.is_empty() {
      Ok(())
    } else {
      Err(ModelValidationError::new(issues))
    }
  }

  pub fn validate_for_durable_execution(&self) -> Result<(), ModelValidationError> {
    let mut issues = self.inspect_structure();
    self.inspect_executable_profile(&mut issues, true, true);
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

    if self.triggers.is_empty() {
      issues.push(issue(
        ModelIssueCode::MissingTrigger,
        "A compiled workflow requires at least one trigger.",
      ));
    }
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
      inspect_expression(
        &node.inputs,
        &format!("node[{}].inputs", node.id),
        &mut issues,
      );
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
    issues
  }

  fn inspect_executable_profile(
    &self,
    issues: &mut Vec<ModelIssue>,
    allow_approval: bool,
    allow_retry: bool,
  ) {
    for trigger in &self.triggers {
      let is_empty_object = matches!(
          &trigger.config,
          ValueExpression::Object { fields } if fields.is_empty()
      );
      if trigger.handler != "trigger.manual" || !is_empty_object {
        issues.push(issue(
                    ModelIssueCode::UnsupportedTrigger,
                    format!(
                        "Trigger {:?} is not executable in the current profile; only trigger.manual with empty config is supported.",
                        trigger.id
                    ),
                ));
      }
    }

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
          approval_wait_inputs(Some(node))
        }
        ("engine.approval-join", ValueExpression::Object { fields }) if allow_approval => {
          fields.is_empty()
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
          && self.schema_version == COMPILED_MODEL_SCHEMA_VERSION_V6
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
        || matches!(edge.condition, EdgeCondition::Boolean { .. }) && edge.branch_id.is_some()
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
          || allow_approval && node.handler == "engine.approval-join")
          && (outgoing_count <= 1
            || node.handler == "engine.branch-select"
            || node.handler == "engine.parallel-start"
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
