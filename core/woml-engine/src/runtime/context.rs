//! Invocation snapshots, compiled visibility, reusable bindings, and secrets.
//!
//! Context is a permitted view of proven outputs, not mutable workflow state.
//! Control-flow callers retain responsibility for choosing the correct fork or
//! iteration snapshot before preparing its protocol bindings.

use super::{
  resolved_reusable_props_with_iteration, reusable_step_descriptor, RuntimeExecutionError,
  RuntimeExecutionOptions,
};
use crate::model::CompiledReusableInvocation;
use crate::protocol::ReusableDefinitionBindingV3;
use crate::protocol::ReusableDefinitionKind;
use crate::protocol::ReusableScriptBindingV3;
use crate::CompiledWorkflowDefinition;
use crate::RunProjection;
use crate::WorkflowContext;
use serde_json::Value;
use std::collections::BTreeMap;
use std::collections::HashSet;

pub(super) fn resolved_script_secrets(
  workflow: &CompiledWorkflowDefinition,
  node_id: &str,
  options: &RuntimeExecutionOptions,
) -> Result<BTreeMap<String, String>, RuntimeExecutionError> {
  let Some(runtime) = workflow
    .node(node_id)
    .and_then(|node| node.script_runtime.as_ref())
  else {
    return Ok(BTreeMap::new());
  };
  runtime
    .required_secrets
    .iter()
    .map(|name| {
      options
        .resolved_secrets
        .get(name)
        .filter(|value| !value.is_empty())
        .cloned()
        .map(|value| (name.clone(), value))
        .ok_or_else(|| {
          RuntimeExecutionError::InvalidConfiguration(format!(
            "script node {node_id:?} requires unresolved secret {name:?}"
          ))
        })
    })
    .collect()
}

fn resolved_reusable_props(
  descriptor: &CompiledReusableInvocation,
  context: &WorkflowContext,
  secrets: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, Value>, RuntimeExecutionError> {
  resolved_reusable_props_with_iteration(descriptor, context, secrets, None)
}

pub(super) fn reusable_step_binding_for_iteration(
  workflow: &CompiledWorkflowDefinition,
  node_id: &str,
  context: &WorkflowContext,
  item: &Value,
  index: u32,
  total: u32,
  secrets: &BTreeMap<String, String>,
) -> Result<Option<ReusableScriptBindingV3>, RuntimeExecutionError> {
  let Some(descriptor) = reusable_step_descriptor(workflow, node_id) else {
    return Ok(None);
  };
  let CompiledReusableInvocation::Step {
    invocation_id,
    alias,
    definition_digest,
    source,
    ..
  } = descriptor
  else {
    unreachable!()
  };
  Ok(Some(ReusableScriptBindingV3 {
    profile: "woml.reusable-script-binding/v3".to_string(),
    invocation_id: invocation_id.clone(),
    definition: ReusableDefinitionBindingV3 {
      kind: ReusableDefinitionKind::Step,
      alias: alias.clone(),
      digest: definition_digest.clone(),
      source: source.clone(),
    },
    props: resolved_reusable_props_with_iteration(
      descriptor,
      context,
      secrets,
      Some((item, index, total)),
    )?,
  }))
}

pub(super) fn reusable_step_binding(
  workflow: &CompiledWorkflowDefinition,
  node_id: &str,
  context: &WorkflowContext,
  secrets: &BTreeMap<String, String>,
) -> Result<Option<ReusableScriptBindingV3>, RuntimeExecutionError> {
  let Some(descriptor) = reusable_step_descriptor(workflow, node_id) else {
    return Ok(None);
  };
  let CompiledReusableInvocation::Step {
    invocation_id,
    alias,
    definition_digest,
    source,
    ..
  } = descriptor
  else {
    unreachable!()
  };
  Ok(Some(ReusableScriptBindingV3 {
    profile: "woml.reusable-script-binding/v3".to_string(),
    invocation_id: invocation_id.clone(),
    definition: ReusableDefinitionBindingV3 {
      kind: ReusableDefinitionKind::Step,
      alias: alias.clone(),
      digest: definition_digest.clone(),
      source: source.clone(),
    },
    props: resolved_reusable_props(descriptor, context, secrets)?,
  }))
}

pub(super) fn reusable_definition_binding(
  descriptor: &CompiledReusableInvocation,
  context: &WorkflowContext,
  secrets: &BTreeMap<String, String>,
) -> Result<ReusableScriptBindingV3, RuntimeExecutionError> {
  let (invocation_id, kind, alias, definition_digest, source) = match descriptor {
    CompiledReusableInvocation::Step {
      invocation_id,
      alias,
      definition_digest,
      source,
      ..
    } => (
      invocation_id,
      ReusableDefinitionKind::Step,
      alias,
      definition_digest,
      source,
    ),
    CompiledReusableInvocation::NotificationProvider {
      provider_id,
      alias,
      definition_digest,
      source,
      ..
    } => (
      provider_id,
      ReusableDefinitionKind::NotificationProvider,
      alias,
      definition_digest,
      source,
    ),
  };
  Ok(ReusableScriptBindingV3 {
    profile: "woml.reusable-script-binding/v3".to_string(),
    invocation_id: invocation_id.clone(),
    definition: ReusableDefinitionBindingV3 {
      kind,
      alias: alias.clone(),
      digest: definition_digest.clone(),
      source: source.clone(),
    },
    props: resolved_reusable_props(descriptor, context, secrets)?,
  })
}

pub(super) fn reusable_definition_binding_for_iteration(
  descriptor: &CompiledReusableInvocation,
  context: &WorkflowContext,
  item: &Value,
  index: u32,
  total: u32,
  secrets: &BTreeMap<String, String>,
) -> Result<ReusableScriptBindingV3, RuntimeExecutionError> {
  let (invocation_id, kind, alias, definition_digest, source) = match descriptor {
    CompiledReusableInvocation::Step {
      invocation_id,
      alias,
      definition_digest,
      source,
      ..
    } => (
      invocation_id,
      ReusableDefinitionKind::Step,
      alias,
      definition_digest,
      source,
    ),
    CompiledReusableInvocation::NotificationProvider {
      provider_id,
      alias,
      definition_digest,
      source,
      ..
    } => (
      provider_id,
      ReusableDefinitionKind::NotificationProvider,
      alias,
      definition_digest,
      source,
    ),
  };
  Ok(ReusableScriptBindingV3 {
    profile: "woml.reusable-script-binding/v3".to_string(),
    invocation_id: invocation_id.clone(),
    definition: ReusableDefinitionBindingV3 {
      kind,
      alias: alias.clone(),
      digest: definition_digest.clone(),
      source: source.clone(),
    },
    props: resolved_reusable_props_with_iteration(
      descriptor,
      context,
      secrets,
      Some((item, index, total)),
    )?,
  })
}

pub(super) fn resolved_reusable_descriptor_secrets(
  descriptor: &CompiledReusableInvocation,
  options: &RuntimeExecutionOptions,
) -> Result<BTreeMap<String, String>, RuntimeExecutionError> {
  let props = match descriptor {
    CompiledReusableInvocation::Step { props, .. }
    | CompiledReusableInvocation::NotificationProvider { props, .. } => props,
  };
  props
    .iter()
    .filter_map(|prop| match &prop.expression {
      crate::model::CompiledReusablePropExpression::Secret { name } => Some(name),
      _ => None,
    })
    .map(|name| {
      options
        .resolved_secrets
        .get(name)
        .filter(|value| !value.is_empty())
        .cloned()
        .map(|value| (name.clone(), value))
        .ok_or_else(|| {
          RuntimeExecutionError::InvalidConfiguration(format!(
            "reusable lifecycle requires unresolved secret {name:?}"
          ))
        })
    })
    .collect()
}

pub(super) fn resolve_json_path(root: &Value, path: &[String]) -> Option<Value> {
  let mut value = root;
  for segment in path {
    value = value.as_object()?.get(segment)?;
  }
  Some(value.clone())
}

pub(super) fn runtime_context_for_node(
  workflow: &CompiledWorkflowDefinition,
  projection: &RunProjection,
  node_id: &str,
) -> WorkflowContext {
  let mut context = projection.context.clone();
  if let Some(visibility) = workflow
    .graph
    .context_visibility
    .as_deref()
    .unwrap_or_default()
    .iter()
    .find(|visibility| visibility.node_id == node_id)
  {
    context
      .steps
      .retain(|step_id, _| visibility.step_ids.contains(step_id));
  }
  if let Some(descriptor) = lock_descriptor_for_body_node(workflow, node_id) {
    if let Some(lock) = projection
      .locks
      .get(&descriptor.lock_id)
      .filter(|lock| lock.status == crate::LockStatus::Held)
    {
      context.lock = Some(crate::LockContext {
        id: lock.lock_id.clone(),
        key: lock.key.clone(),
        fencing_token: lock.fencing_token.clone().unwrap_or_default(),
        acquired_at: lock
          .acquired_at
          .map(|value| value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
          .unwrap_or_default(),
      });
    }
  }
  context
}

pub(super) fn lock_descriptor_for_body_node<'a>(
  workflow: &'a CompiledWorkflowDefinition,
  node_id: &str,
) -> Option<&'a crate::CompiledLock> {
  fn contains_node(
    lock: &crate::CompiledLock,
    node_id: &str,
    edges: &[crate::model::CompiledWorkflowEdge],
  ) -> bool {
    let mut pending = vec![lock.body_entry_node_id.as_str()];
    let mut visited = HashSet::new();
    while let Some(candidate) = pending.pop() {
      if !visited.insert(candidate) {
        continue;
      }
      if candidate == node_id {
        return true;
      }
      if candidate == lock.body_terminal_node_id {
        continue;
      }
      pending.extend(
        edges
          .iter()
          .filter(|edge| edge.from == candidate)
          .map(|edge| edge.to.as_str()),
      );
    }
    false
  }
  workflow
    .graph
    .locks
    .as_deref()
    .unwrap_or_default()
    .iter()
    .find(|lock| {
      contains_node(lock, node_id, &workflow.graph.edges)
        || workflow
          .graph
          .for_each
          .as_deref()
          .unwrap_or_default()
          .iter()
          .any(|loop_| contains_node(lock, node_id, &loop_.body.edges))
    })
}
