//! model fork validation.

use super::{
  empty_control_node, issue, valid_public_structural_id, CompiledWorkflowDefinition,
  CompiledWorkflowEdge, CompiledWorkflowNode, EdgeCondition, ModelIssue, ModelIssueCode,
  ValueExpression,
};
use crate::COMPILED_MODEL_SCHEMA_VERSION_V13;
use std::collections::HashMap;
use std::collections::HashSet;

pub(super) fn inspect_fork_contract(
  workflow: &CompiledWorkflowDefinition,
  issues: &mut Vec<ModelIssue>,
) {
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
