//! model policy validation.

use super::{
  issue, CompiledWorkflowDefinition, ModelIssue, ModelIssueCode, QueueDiscipline,
  RateLimitAlgorithm,
};
use crate::COMPILED_MODEL_SCHEMA_VERSION_V12;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V13;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V14;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V15;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V16;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V17;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V18;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V19;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V20;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V21;

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

pub(super) fn inspect_runtime_policy_contract(
  workflow: &CompiledWorkflowDefinition,
  issues: &mut Vec<ModelIssue>,
) {
  match (workflow.schema_version, &workflow.runtime_policy) {
    (
      COMPILED_MODEL_SCHEMA_VERSION_V12
      | COMPILED_MODEL_SCHEMA_VERSION_V13
      | COMPILED_MODEL_SCHEMA_VERSION_V14
      | COMPILED_MODEL_SCHEMA_VERSION_V15
      | COMPILED_MODEL_SCHEMA_VERSION_V16
      | COMPILED_MODEL_SCHEMA_VERSION_V17
      | COMPILED_MODEL_SCHEMA_VERSION_V18
      | COMPILED_MODEL_SCHEMA_VERSION_V19
      | COMPILED_MODEL_SCHEMA_VERSION_V20
      | COMPILED_MODEL_SCHEMA_VERSION_V21,
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
      | COMPILED_MODEL_SCHEMA_VERSION_V14
      | COMPILED_MODEL_SCHEMA_VERSION_V15
      | COMPILED_MODEL_SCHEMA_VERSION_V16
      | COMPILED_MODEL_SCHEMA_VERSION_V17
      | COMPILED_MODEL_SCHEMA_VERSION_V18
      | COMPILED_MODEL_SCHEMA_VERSION_V19
      | COMPILED_MODEL_SCHEMA_VERSION_V20
      | COMPILED_MODEL_SCHEMA_VERSION_V21,
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
