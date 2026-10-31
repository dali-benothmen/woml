//! model iteration validation.

use super::{
  empty_control_node, inspect_expression, issue, valid_id, valid_public_structural_id,
  valid_secret_name, CompiledControlChoice, CompiledWorkflowDefinition, CompiledWorkflowEdge,
  CompiledWorkflowNode, EdgeCondition, ModelIssue, ModelIssueCode, ValueExpression,
};
use crate::COMPILED_MODEL_SCHEMA_VERSION_V16;
use serde_json::Map;
use serde_json::Value;
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;

pub(super) fn valid_for_each_path_segment(value: &str) -> bool {
  let mut characters = value.chars();
  matches!(characters.next(), Some(first) if first == '_' || first == '$' || first.is_ascii_alphabetic())
    && characters
      .all(|character| character == '_' || character == '$' || character.is_ascii_alphanumeric())
}

fn valid_for_each_items_reference(
  expression: &ValueExpression,
  outer_step_ids: &HashSet<&str>,
) -> bool {
  let ValueExpression::ContextReference { path } = expression else {
    return false;
  };
  if path.is_empty()
    || path.len() > 64
    || path.iter().any(|part| !valid_for_each_path_segment(part))
  {
    return false;
  }
  match path.as_slice() {
    [root, ..] if root == "trigger" => true,
    [root, step_id, ..] if root == "steps" => outer_step_ids.contains(step_id.as_str()),
    _ => false,
  }
}

fn valid_for_each_result_reference(
  expression: &ValueExpression,
  outer_step_ids: &HashSet<&str>,
  body_node_ids: &HashSet<&str>,
) -> bool {
  let ValueExpression::ContextReference { path } = expression else {
    return false;
  };
  if path.is_empty()
    || path.len() > 64
    || path.iter().any(|part| !valid_for_each_path_segment(part))
  {
    return false;
  }
  match path.as_slice() {
    [root, ..] if root == "trigger" || root == "item" => true,
    [root, field] if root == "iteration" && matches!(field.as_str(), "index" | "total") => true,
    [root, step_id, ..] if root == "steps" => {
      outer_step_ids.contains(step_id.as_str()) || body_node_ids.contains(step_id.as_str())
    }
    _ => false,
  }
}

fn valid_for_each_metadata(metadata: Option<&Map<String, Value>>) -> bool {
  metadata.is_none_or(|metadata| {
    !metadata.is_empty()
      && metadata.len() <= 2
      && metadata.iter().all(|(name, value)| {
        matches!(name.as_str(), "name" | "description")
          && value.as_str().is_some_and(|value| !value.is_empty())
      })
  })
}

fn valid_for_each_body_choice(
  choice: &CompiledControlChoice,
  nodes: &HashMap<&str, &CompiledWorkflowNode>,
  incoming: &HashMap<&str, Vec<&CompiledWorkflowEdge>>,
  outgoing: &HashMap<&str, Vec<&CompiledWorkflowEdge>>,
  outer_step_ids: &HashSet<&str>,
  body_node_ids: &HashSet<&str>,
) -> bool {
  let selections = outgoing
    .get(choice.selector_node_id.as_str())
    .map(Vec::as_slice)
    .unwrap_or_default();
  let joins = incoming
    .get(choice.join_node_id.as_str())
    .map(Vec::as_slice)
    .unwrap_or_default();
  let string_choice = choice.string_selector.is_some();
  let string_contract = if string_choice {
    choice.string_selector.as_ref().is_some_and(|selector| {
      valid_for_each_result_reference(selector, outer_step_ids, body_node_ids)
    }) && choice.string_cases.as_ref().is_some_and(|cases| {
      !cases.is_empty()
        && cases.len() + 1 == choice.arm_ids.len()
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
  let mut valid = choice.choice_id.starts_with("__woml_choice__")
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
    && selections.len() == choice.arm_ids.len()
    && joins.len() == choice.arm_ids.len()
    && string_contract;
  if let Some(result_node_id) = &choice.result_node_id {
    let result_node = nodes.get(result_node_id.as_str());
    let result_incoming = incoming
      .get(result_node_id.as_str())
      .map(Vec::as_slice)
      .unwrap_or_default();
    valid &= string_choice
      && valid_public_structural_id(result_node_id)
      && result_node.is_some_and(|node| {
        node.handler == "engine.choice-result"
          && matches!(&node.inputs, ValueExpression::Object { fields } if fields.len() == choice.arm_ids.len()
            && choice.arm_ids.iter().all(|arm_id| fields.get(arm_id).is_some_and(|value| {
              valid_for_each_result_reference(value, outer_step_ids, body_node_ids)
            })))
      })
      && result_incoming.len() == 1
      && result_incoming[0].from == choice.join_node_id
      && matches!(result_incoming[0].condition, EdgeCondition::Always);
  }
  for (index, arm_id) in choice.arm_ids.iter().enumerate() {
    valid &= arm_id.starts_with(&format!("{}:", choice.choice_id))
      && selections.get(index).is_some_and(|edge| {
        edge.id == *arm_id
          && edge.branch_id.is_none()
          && edge.parallel_id.is_none()
          && edge.approval_id.is_none()
          && if string_choice || index + 1 == choice.arm_ids.len() {
            matches!(edge.condition, EdgeCondition::Always)
          } else {
            matches!(edge.condition, EdgeCondition::Boolean { .. })
          }
      })
      && joins.get(index).is_some_and(|edge| {
        edge.id == format!("{arm_id}:join")
          && edge.to == choice.join_node_id
          && matches!(edge.condition, EdgeCondition::Always)
      });
  }
  valid
}

pub(super) fn inspect_for_each_contract(
  workflow: &CompiledWorkflowDefinition,
  issues: &mut Vec<ModelIssue>,
) {
  if workflow.schema_version < COMPILED_MODEL_SCHEMA_VERSION_V16 {
    if workflow.graph.for_each.is_some() {
      issues.push(issue(
        ModelIssueCode::InvalidForEach,
        "forEach descriptors are available only in compiled Model v16 and later.",
      ));
    }
    return;
  }

  let descriptors = match &workflow.graph.for_each {
    Some(descriptors) if !descriptors.is_empty() => descriptors,
    _ if workflow.schema_version == COMPILED_MODEL_SCHEMA_VERSION_V16 => {
      issues.push(issue(
        ModelIssueCode::InvalidForEach,
        "Compiled Model v16 requires at least one forEach descriptor.",
      ));
      return;
    }
    _ => {
      if workflow.graph.nodes.iter().any(|node| {
        matches!(
          node.handler.as_str(),
          "engine.for-each-open" | "engine.for-each-result"
        )
      }) {
        issues.push(issue(
          ModelIssueCode::InvalidForEach,
          "forEach execution nodes require matching descriptors.",
        ));
      }
      return;
    }
  };
  if descriptors.len() > 256 {
    issues.push(issue(
      ModelIssueCode::InvalidForEach,
      "Compiled models permit at most 256 forEach descriptors.",
    ));
    return;
  }

  let root_nodes: HashMap<&str, &CompiledWorkflowNode> = workflow
    .graph
    .nodes
    .iter()
    .map(|node| (node.id.as_str(), node))
    .collect();
  let root_node_ids: HashSet<&str> = root_nodes.keys().copied().collect();
  let mut descriptor_ids = HashSet::new();
  let mut owned_body_ids = HashSet::new();

  for descriptor in descriptors {
    let expected_open = format!("__woml_for_each__{}__open", descriptor.for_each_id);
    let outer_step_ids: HashSet<&str> = descriptor
      .outer_step_ids
      .iter()
      .map(String::as_str)
      .collect();
    let body_node_ids: HashSet<&str> = descriptor
      .body
      .nodes
      .iter()
      .map(|node| node.id.as_str())
      .collect();
    let open_node = root_nodes.get(descriptor.open_node_id.as_str());
    let result_node = root_nodes.get(descriptor.result_node_id.as_str());
    let root_boundary_edges: Vec<_> = workflow
      .graph
      .edges
      .iter()
      .filter(|edge| edge.from == descriptor.open_node_id || edge.to == descriptor.result_node_id)
      .collect();
    let metadata_matches = result_node.is_some_and(|node| {
      node.handler == "engine.for-each-result"
        && node.timeout_ms.is_none()
        && node.retry_policy.is_none()
        && node.script_runtime.is_none()
        && matches!(&node.inputs, ValueExpression::Object { fields } if fields.is_empty())
        && node.metadata.as_ref() == descriptor.metadata.as_ref()
    });
    let root_boundary_valid = root_boundary_edges.len() == 1
      && root_boundary_edges[0].from == descriptor.open_node_id
      && root_boundary_edges[0].to == descriptor.result_node_id
      && matches!(root_boundary_edges[0].condition, EdgeCondition::Always)
      && root_boundary_edges[0].branch_id.is_none()
      && root_boundary_edges[0].parallel_id.is_none()
      && root_boundary_edges[0].approval_id.is_none();

    let mut valid = valid_public_structural_id(&descriptor.for_each_id)
      && descriptor_ids.insert(descriptor.for_each_id.as_str())
      && descriptor.open_node_id == expected_open
      && descriptor.result_node_id == descriptor.for_each_id
      && empty_control_node(open_node, "engine.for-each-open")
      && metadata_matches
      && valid_for_each_metadata(descriptor.metadata.as_ref())
      && (1..=64).contains(&descriptor.concurrency)
      && descriptor.outer_step_ids.len() == outer_step_ids.len()
      && outer_step_ids.iter().all(|id| root_node_ids.contains(id))
      && valid_for_each_items_reference(&descriptor.items, &outer_step_ids)
      && root_boundary_valid;

    let mut node_ids = HashSet::new();
    let mut nodes = HashMap::new();
    for node in &descriptor.body.nodes {
      let handler_allowed = matches!(
        node.handler.as_str(),
        "runtime.script"
          | "engine.branch-select"
          | "engine.branch-result"
          | "engine.choice-select"
          | "engine.choice-join"
          | "engine.choice-result"
          | "engine.parallel-start"
          | "engine.parallel-join"
          | "engine.lock-acquire"
          | "engine.lock-release"
          | "engine.lock-result"
      );
      let script_valid = if node.handler == "runtime.script" {
        matches!(&node.inputs, ValueExpression::Object { fields } if fields.len() == 1
          && matches!(fields.get("source"), Some(ValueExpression::Literal { value }) if value.is_string()))
          && node.timeout_ms.is_none()
          && node.script_runtime.as_ref().is_some_and(|runtime| {
            ((runtime.binding_version == 1
              && runtime.bindings == ["context", "attempt", "services", "secrets"])
              || (runtime.binding_version == 3
                && runtime.bindings == ["props", "context", "attempt", "services"]))
              && runtime.required_secrets.len() <= 64
              && runtime
                .required_secrets
                .windows(2)
                .all(|pair| pair[0] < pair[1])
              && runtime
                .required_secrets
                .iter()
                .all(|name| valid_secret_name(name))
          })
      } else {
        node.timeout_ms.is_none() && node.retry_policy.is_none() && node.script_runtime.is_none()
      };
      valid &= valid_id(&node.id)
        && node_ids.insert(node.id.as_str())
        && !root_node_ids.contains(node.id.as_str())
        && owned_body_ids.insert(node.id.as_str())
        && handler_allowed
        && script_valid;
      nodes.insert(node.id.as_str(), node);
      inspect_expression(
        &node.inputs,
        &format!(
          "forEach[{}].body.node[{}].inputs",
          descriptor.for_each_id, node.id
        ),
        issues,
      );
    }
    valid &= !descriptor.body.nodes.is_empty()
      && descriptor.body.entry_node_ids.len() == 1
      && node_ids.contains(descriptor.body.entry_node_ids[0].as_str())
      && node_ids.contains(descriptor.body.terminal_node_id.as_str());

    let mut incoming: HashMap<&str, Vec<&CompiledWorkflowEdge>> = node_ids
      .iter()
      .copied()
      .map(|id| (id, Vec::new()))
      .collect();
    let mut outgoing: HashMap<&str, Vec<&CompiledWorkflowEdge>> = node_ids
      .iter()
      .copied()
      .map(|id| (id, Vec::new()))
      .collect();
    let mut edge_ids = HashSet::new();
    for edge in &descriptor.body.edges {
      let condition_valid = match &edge.condition {
        EdgeCondition::Always => true,
        EdgeCondition::Boolean { value } => {
          valid_for_each_result_reference(value, &outer_step_ids, &body_node_ids)
        }
        EdgeCondition::Truthy { .. } | EdgeCondition::Equals { .. } => false,
      };
      let branch_edge_valid = edge.branch_id.as_ref().is_none_or(|branch_id| {
        edge.from == format!("__woml_branch__{branch_id}__select")
          && edge.id.starts_with(&format!("{branch_id}:"))
      });
      valid &= valid_id(&edge.id)
        && edge_ids.insert(edge.id.as_str())
        && node_ids.contains(edge.from.as_str())
        && node_ids.contains(edge.to.as_str())
        && edge.approval_id.is_none()
        && branch_edge_valid
        && condition_valid;
      if node_ids.contains(edge.from.as_str()) && node_ids.contains(edge.to.as_str()) {
        outgoing.entry(edge.from.as_str()).or_default().push(edge);
        incoming.entry(edge.to.as_str()).or_default().push(edge);
      }
    }

    let entry = descriptor.body.entry_node_ids.first().map(String::as_str);
    valid &= entry.is_some_and(|id| incoming.get(id).is_some_and(Vec::is_empty));
    let terminals: Vec<_> = node_ids
      .iter()
      .copied()
      .filter(|id| outgoing.get(id).is_none_or(Vec::is_empty))
      .collect();
    valid &= terminals.len() == 1 && terminals[0] == descriptor.body.terminal_node_id;

    let mut reachable = HashSet::new();
    let mut queue: VecDeque<&str> = entry.into_iter().collect();
    while let Some(node_id) = queue.pop_front() {
      if reachable.insert(node_id) {
        queue.extend(
          outgoing
            .get(node_id)
            .into_iter()
            .flatten()
            .map(|edge| edge.to.as_str()),
        );
      }
    }
    valid &= reachable.len() == node_ids.len();

    let mut remaining: HashMap<&str, usize> = incoming
      .iter()
      .map(|(id, edges)| (*id, edges.len()))
      .collect();
    let mut ready: VecDeque<&str> = remaining
      .iter()
      .filter_map(|(id, count)| (*count == 0).then_some(*id))
      .collect();
    let mut visited = 0;
    while let Some(node_id) = ready.pop_front() {
      visited += 1;
      for edge in outgoing.get(node_id).into_iter().flatten() {
        let count = remaining.entry(edge.to.as_str()).or_default();
        *count -= 1;
        if *count == 0 {
          ready.push_back(edge.to.as_str());
        }
      }
    }
    valid &= visited == node_ids.len();

    let mut choice_ids = HashSet::new();
    let mut described_choice_nodes = HashSet::new();
    for choice in &descriptor.body.choices {
      valid &= choice_ids.insert(choice.choice_id.as_str())
        && valid_for_each_body_choice(
          choice,
          &nodes,
          &incoming,
          &outgoing,
          &outer_step_ids,
          &body_node_ids,
        );
      described_choice_nodes.insert(choice.selector_node_id.as_str());
      described_choice_nodes.insert(choice.join_node_id.as_str());
      if let Some(result_node_id) = &choice.result_node_id {
        described_choice_nodes.insert(result_node_id.as_str());
      }
    }
    valid &= descriptor.body.nodes.iter().all(|node| {
      !matches!(
        node.handler.as_str(),
        "engine.choice-select" | "engine.choice-join" | "engine.choice-result"
      ) || described_choice_nodes.contains(node.id.as_str())
    });

    let mut described_branch_nodes = HashSet::new();
    for selector in descriptor
      .body
      .nodes
      .iter()
      .filter(|node| node.handler == "engine.branch-select")
    {
      let branch_id = selector
        .id
        .strip_prefix("__woml_branch__")
        .and_then(|value| value.strip_suffix("__select"));
      let Some(branch_id) = branch_id else {
        valid = false;
        continue;
      };
      let selections = outgoing
        .get(selector.id.as_str())
        .map(Vec::as_slice)
        .unwrap_or_default();
      let result = nodes.get(branch_id);
      let joins = incoming
        .get(branch_id)
        .map(Vec::as_slice)
        .unwrap_or_default();
      let result_fields = result.and_then(|node| match &node.inputs {
        ValueExpression::Object { fields } => Some(fields),
        _ => None,
      });
      let selection_valid = selections.len() >= 2
        && selections.iter().enumerate().all(|(index, edge)| {
          edge.branch_id.as_deref() == Some(branch_id)
            && edge.id.starts_with(&format!("{branch_id}:"))
            && if index + 1 == selections.len() {
              matches!(edge.condition, EdgeCondition::Always)
            } else {
              matches!(edge.condition, EdgeCondition::Boolean { .. })
            }
        });
      let result_valid = result.is_some_and(|node| node.handler == "engine.branch-result")
        && joins.len() == selections.len()
        && joins.iter().all(|edge| {
          edge.to == branch_id
            && edge.branch_id.is_none()
            && matches!(edge.condition, EdgeCondition::Always)
        })
        && result_fields.is_some_and(|fields| {
          fields.len() == selections.len()
            && selections.iter().all(|selection| {
              fields.get(&selection.id).is_some_and(|expression| {
                valid_for_each_result_reference(expression, &outer_step_ids, &body_node_ids)
              })
            })
        });
      valid &= selection_valid && result_valid;
      described_branch_nodes.insert(selector.id.as_str());
      described_branch_nodes.insert(branch_id);
    }
    valid &= descriptor.body.nodes.iter().all(|node| {
      !matches!(
        node.handler.as_str(),
        "engine.branch-select" | "engine.branch-result"
      ) || described_branch_nodes.contains(node.id.as_str())
    });

    let script_nodes: Vec<_> = descriptor
      .body
      .nodes
      .iter()
      .filter(|node| node.handler == "runtime.script")
      .collect();
    valid &= descriptor.body.context_visibility.len() == script_nodes.len()
      && descriptor
        .body
        .context_visibility
        .iter()
        .zip(script_nodes)
        .all(|(visibility, node)| {
          visibility.node_id == node.id
            && visibility.step_ids.iter().collect::<HashSet<_>>().len() == visibility.step_ids.len()
            && visibility.step_ids.iter().all(|id| {
              id != &node.id
                && node_ids.contains(id.as_str())
                && !outer_step_ids.contains(id.as_str())
            })
        });
    if let Some(result) = &descriptor.result {
      valid &= valid_for_each_result_reference(result, &outer_step_ids, &body_node_ids);
      inspect_expression(
        result,
        &format!("forEach[{}].result", descriptor.for_each_id),
        issues,
      );
    }

    if !valid {
      issues.push(issue(
        ModelIssueCode::InvalidForEach,
        format!(
          "For-each descriptor {:?} does not match the frozen Model v16 boundary, body-DAG, identity, reference, and visibility contract.",
          descriptor.for_each_id
        ),
      ));
    }
  }

  for node in &workflow.graph.nodes {
    if matches!(
      node.handler.as_str(),
      "engine.for-each-open" | "engine.for-each-result"
    ) && !descriptors
      .iter()
      .any(|descriptor| node.id == descriptor.open_node_id || node.id == descriptor.result_node_id)
    {
      issues.push(issue(
        ModelIssueCode::InvalidForEach,
        format!(
          "For-each control node {:?} has no owner descriptor.",
          node.id
        ),
      ));
    }
  }
}
