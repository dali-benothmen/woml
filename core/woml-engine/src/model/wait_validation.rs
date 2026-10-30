//! model wait validation.

use super::{
  issue, valid_for_each_path_segment, valid_public_structural_id, CompiledWait,
  CompiledWorkflowDefinition, ModelIssue, ModelIssueCode, ValueExpression,
};
use crate::COMPILED_MODEL_SCHEMA_VERSION_V17;
use serde_json::Map;
use serde_json::Value;
use std::collections::HashSet;

fn valid_wait_metadata(metadata: Option<&Map<String, Value>>) -> bool {
  metadata.is_none_or(|metadata| {
    !metadata.is_empty()
      && metadata.len() <= 2
      && metadata.iter().all(|(name, value)| {
        matches!(name.as_str(), "name" | "description")
          && value.as_str().is_some_and(|value| !value.is_empty())
      })
  })
}

fn valid_wait_reference(value: &ValueExpression) -> bool {
  matches!(value, ValueExpression::ContextReference { path }
    if !path.is_empty() && path.len() <= 64 && path.iter().all(|part| valid_for_each_path_segment(part)))
}

fn valid_signal_name(value: &str) -> bool {
  !value.is_empty()
    && value.len() <= 128
    && value.split('.').all(|segment| {
      let mut characters = segment.chars();
      matches!(characters.next(), Some(first) if first.is_ascii_lowercase())
        && characters.all(|character| {
          character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
    })
}

fn valid_wait_timestamp(value: &ValueExpression) -> bool {
  match value {
    ValueExpression::ContextReference { .. } => valid_wait_reference(value),
    ValueExpression::Literal { value } => value.as_str().is_some_and(|value| {
      (value.ends_with('Z') || value.rfind(['+', '-']).is_some_and(|index| index > 9))
        && chrono::DateTime::parse_from_rfc3339(value).is_ok()
    }),
    _ => false,
  }
}

pub(super) fn inspect_durable_wait_contract(
  workflow: &CompiledWorkflowDefinition,
  issues: &mut Vec<ModelIssue>,
) {
  if workflow.schema_version < COMPILED_MODEL_SCHEMA_VERSION_V17 {
    if workflow.graph.waits.is_some() {
      issues.push(issue(
        ModelIssueCode::InvalidDurableWait,
        "Durable wait descriptors are available only in compiled Model v17.",
      ));
    }
    return;
  }
  let Some(waits) = &workflow.graph.waits else {
    issues.push(issue(
      ModelIssueCode::InvalidDurableWait,
      "Compiled Model v17 requires a waits descriptor collection.",
    ));
    return;
  };
  if (workflow.schema_version == COMPILED_MODEL_SCHEMA_VERSION_V17 && waits.is_empty())
    || waits.len() > 1024
  {
    issues.push(issue(
      ModelIssueCode::InvalidDurableWait,
      "Compiled Model v17 requires between 1 and 1024 durable waits.",
    ));
    return;
  }
  let wait_nodes = workflow
    .graph
    .nodes
    .iter()
    .chain(
      workflow
        .graph
        .for_each
        .as_deref()
        .unwrap_or_default()
        .iter()
        .flat_map(|descriptor| descriptor.body.nodes.iter()),
    )
    .filter(|node| node.handler == "engine.durable-wait")
    .collect::<Vec<_>>();
  let mut wait_ids = HashSet::new();
  let mut node_ids = HashSet::new();
  for descriptor in waits {
    let node = wait_nodes
      .iter()
      .find(|node| node.id == descriptor.node_id())
      .copied();
    let common_valid = valid_public_structural_id(descriptor.wait_id())
      && descriptor.wait_id() == descriptor.node_id()
      && wait_ids.insert(descriptor.wait_id())
      && node_ids.insert(descriptor.node_id())
      && node.is_some_and(|node| {
        node.handler == "engine.durable-wait"
          && node.timeout_ms.is_none()
          && node.retry_policy.is_none()
          && node.script_runtime.is_none()
          && matches!(&node.inputs, ValueExpression::Object { fields } if fields.is_empty())
      });
    let variant_valid = match descriptor {
      CompiledWait::Duration {
        metadata,
        duration_ms,
        ..
      } => valid_wait_metadata(metadata.as_ref()) && (1_000..=31_536_000_000).contains(duration_ms),
      CompiledWait::Until { metadata, at, .. } => {
        valid_wait_metadata(metadata.as_ref()) && valid_wait_timestamp(at)
      }
      CompiledWait::Signal {
        metadata,
        signals,
        correlation,
        timeout_ms,
        on_timeout,
        ..
      } => {
        let mut names = HashSet::new();
        valid_wait_metadata(metadata.as_ref())
          && (1..=16).contains(&signals.len())
          && signals.iter().all(|signal| {
            valid_signal_name(&signal.signal)
              && names.insert(signal.signal.as_str())
              && signal.schema.is_object()
          })
          && match correlation {
            ValueExpression::Literal { value } => value.as_str().is_some_and(|value| {
              !value.is_empty()
                && value.len() <= 256
                && value.trim() == value
                && !value.chars().any(char::is_control)
            }),
            ValueExpression::ContextReference { .. } => valid_wait_reference(correlation),
            _ => false,
          }
          && timeout_ms.is_none_or(|value| (1_000..=31_536_000_000).contains(&value))
          && matches!(on_timeout.as_str(), "fail" | "continue")
      }
    };
    if !common_valid || !variant_valid {
      issues.push(issue(
        ModelIssueCode::InvalidDurableWait,
        format!(
          "Durable wait {:?} does not match the frozen Model v17 descriptor contract.",
          descriptor.wait_id()
        ),
      ));
    }
  }
  if wait_nodes.len() != waits.len()
    || wait_nodes
      .iter()
      .any(|node| !node_ids.contains(node.id.as_str()))
  {
    issues.push(issue(
      ModelIssueCode::InvalidDurableWait,
      "Model v17 waits must describe every engine.durable-wait node exactly once.",
    ));
  }
}
