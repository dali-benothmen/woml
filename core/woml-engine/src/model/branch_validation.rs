//! model branch validation.

use super::{
  issue, valid_public_structural_id, valid_secret_name, CompiledWorkflowDefinition,
  CompiledWorkflowEdge, CompiledWorkflowNode, EdgeCondition, ModelIssue, ModelIssueCode,
  TemplatePart, ValueExpression,
};
use crate::COMPILED_MODEL_SCHEMA_VERSION_V1;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V2;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V4;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;

fn selector_branch_id(value: &str) -> Option<&str> {
  value
    .strip_prefix("__woml_branch__")?
    .strip_suffix("__select")
    .filter(|branch_id| valid_public_structural_id(branch_id))
}

pub(super) fn parallel_start_id(value: &str) -> Option<&str> {
  value
    .strip_prefix("__woml_parallel__")?
    .strip_suffix("__start")
    .filter(|parallel_id| valid_public_structural_id(parallel_id))
}

pub(super) fn parallel_start_inputs(
  node: Option<&CompiledWorkflowNode>,
  child_count: usize,
) -> bool {
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

pub(super) fn parallel_join_inputs(node: Option<&CompiledWorkflowNode>) -> bool {
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

pub(super) fn inspect_expression(
  expression: &ValueExpression,
  at: &str,
  issues: &mut Vec<ModelIssue>,
) {
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

pub(super) fn contains_lifecycle_reference(expression: &ValueExpression) -> bool {
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

pub(super) fn inspect_branch_contract(
  workflow: &CompiledWorkflowDefinition,
  issues: &mut Vec<ModelIssue>,
) {
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
