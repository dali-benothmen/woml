use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

use crate::COMPILED_MODEL_SCHEMA_VERSION;

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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ValueExpression {
  #[serde(rename = "literal")]
  Literal { value: Value },
  #[serde(rename = "contextReference")]
  ContextReference { path: Vec<String> },
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

impl CompiledWorkflowDefinition {
  pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
    serde_json::from_str(json)
  }

  pub fn node(&self, node_id: &str) -> Option<&CompiledWorkflowNode> {
    self.graph.nodes.iter().find(|node| node.id == node_id)
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
    self.inspect_executable_profile(&mut issues);
    if issues.is_empty() {
      Ok(())
    } else {
      Err(ModelValidationError::new(issues))
    }
  }

  pub fn inspect_structure(&self) -> Vec<ModelIssue> {
    let mut issues = Vec::new();
    if self.schema_version != COMPILED_MODEL_SCHEMA_VERSION {
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
        if retry.max_attempts == 0 {
          issues.push(issue(
            ModelIssueCode::UnsupportedRetry,
            format!(
              "Node {:?} has an invalid zero-attempt retry policy.",
              node.id
            ),
          ));
        }
        if let BackoffPolicy::Exponential { multiplier, .. } = retry.backoff {
          if !multiplier.is_finite() || multiplier <= 1.0 {
            issues.push(issue(
              ModelIssueCode::UnsupportedRetry,
              format!(
                "Node {:?} has an invalid exponential retry multiplier.",
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
    issues
  }

  fn inspect_executable_profile(&self, issues: &mut Vec<ModelIssue>) {
    for trigger in &self.triggers {
      let is_empty_object = matches!(
          &trigger.config,
          ValueExpression::Object { fields } if fields.is_empty()
      );
      if trigger.handler != "trigger.manual" || !is_empty_object {
        issues.push(issue(
                    ModelIssueCode::UnsupportedTrigger,
                    format!(
                        "Trigger {:?} is not executable in R2; only trigger.manual with empty config is supported.",
                        trigger.id
                    ),
                ));
      }
    }

    for node in &self.graph.nodes {
      if node.handler != "runtime.script" {
        issues.push(issue(
          ModelIssueCode::UnknownHandler,
          format!(
            "No R2 handler is registered for {:?} on node {:?}.",
            node.handler, node.id
          ),
        ));
      }
      let valid_script_input = match &node.inputs {
        ValueExpression::Object { fields } if fields.len() == 1 => {
          matches!(fields.get("source"), Some(ValueExpression::Literal { value }) if value.is_string())
        }
        _ => false,
      };
      if !valid_script_input {
        issues.push(issue(
          ModelIssueCode::UnsupportedNodeInputs,
          format!(
            "Node {:?} must have exactly one literal string input named source in R2.",
            node.id
          ),
        ));
      }
      if node.retry_policy.is_some() {
        issues.push(issue(
          ModelIssueCode::UnsupportedRetry,
          format!("Retry is not executable in R2 (node {:?}).", node.id),
        ));
      }
      if node.timeout_ms.is_some() {
        issues.push(issue(
          ModelIssueCode::UnsupportedTimeout,
          format!(
            "Per-node timeout is not executable in R2 (node {:?}).",
            node.id
          ),
        ));
      }
    }

    for edge in &self.graph.edges {
      if !matches!(edge.condition, EdgeCondition::Always) {
        issues.push(issue(
          ModelIssueCode::UnsupportedEdgeCondition,
          format!(
            "Edge {:?} uses a condition that is not executable in R2.",
            edge.id
          ),
        ));
      }
      if edge.branch_id.is_some() {
        issues.push(issue(
          ModelIssueCode::UnsupportedBranch,
          format!(
            "Edge {:?} carries a branch identity, which is staged.",
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
    let is_linear = self.graph.entry_node_ids.len() == 1
      && self.graph.nodes.iter().all(|node| {
        incoming.get(node.id.as_str()).copied().unwrap_or(0) <= 1
          && outgoing.get(node.id.as_str()).copied().unwrap_or(0) <= 1
      })
      && self.graph.edges.len() + 1 == self.graph.nodes.len();
    if !is_linear {
      issues.push(issue(
                ModelIssueCode::UnsupportedNonSequentialDag,
                "R2 executes only one unconditional sequential path; parallel and branching DAGs are staged.",
            ));
    }
  }
}
