//! model validation.

use super::{
  approval_wait_inputs, contains_lifecycle_reference, inspect_approval_contract,
  inspect_branch_contract, inspect_durable_wait_contract, inspect_execution_placement_contract,
  inspect_expression, inspect_for_each_contract, inspect_fork_contract, inspect_lifecycle_contract,
  inspect_lock_contract, inspect_parallel_contract, inspect_reusable_contract,
  inspect_runtime_policy_contract, inspect_saga_contract, parallel_start_inputs,
  reusable_provider_ids, valid_model_v7_trigger, BackoffPolicy, CompiledReusableBoundProp,
  CompiledReusableLifecycle, CompiledReusablePropExpression, CompiledWorkflowDefinition,
  CompiledWorkflowNode, EdgeCondition, ModelIssue, ModelIssueCode, ModelValidationError,
  RetryPolicy, ValueExpression,
};
use crate::COMPILED_MODEL_SCHEMA_VERSION_V1;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V10;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V11;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V12;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V13;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V14;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V15;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V16;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V17;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V18;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V19;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V2;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V20;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V21;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V3;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V4;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V5;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V6;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V7;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V8;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V9;
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;

pub(super) fn issue(code: ModelIssueCode, message: impl Into<String>) -> ModelIssue {
  ModelIssue {
    code,
    message: message.into(),
  }
}

pub(super) fn valid_id(value: &str) -> bool {
  !value.is_empty() && value.chars().count() <= 256
}

pub(super) fn valid_public_structural_id(value: &str) -> bool {
  let mut characters = value.chars();
  matches!(characters.next(), Some(first) if first.is_ascii_lowercase())
    && characters.all(|character| character.is_ascii_alphanumeric())
    && value.chars().count() <= 256
}

pub(super) fn valid_secret_name(value: &str) -> bool {
  let mut characters = value.chars();
  matches!(characters.next(), Some(first) if first.is_ascii_uppercase())
    && characters.all(|character| {
      character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
    })
}

pub(super) fn valid_reusable_alias(value: &str) -> bool {
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

pub(super) fn valid_sha256(value: &str) -> bool {
  value.len() == 71
    && value.starts_with("sha256:")
    && value[7..]
      .bytes()
      .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub(super) fn valid_portable_path(value: &str) -> bool {
  !value.is_empty()
    && value.len() <= 1024
    && !value.starts_with('/')
    && !value.contains('\\')
    && !value.split('/').any(|part| part == ".." || part.is_empty())
}

fn valid_reusable_context_path(path: &str, allow_iteration: bool) -> bool {
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
    ["item", rest @ ..] => allow_iteration && identifiers_valid(rest),
    ["iteration", "index" | "total"] => allow_iteration,
    _ => false,
  }
}

pub(super) fn reusable_prop_secrets(
  props: &[CompiledReusableBoundProp],
  subject: &str,
  allow_iteration: bool,
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
        !prop.secret && path.len() <= 1024 && valid_reusable_context_path(path, allow_iteration)
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

pub(super) fn valid_reusable_lifecycle(lifecycle: &CompiledReusableLifecycle) -> bool {
  [
    &lifecycle.on_success,
    &lifecycle.on_error,
    &lifecycle.on_complete,
  ]
  .into_iter()
  .flatten()
  .all(|ids| !ids.is_empty() && ids.iter().all(|id| valid_id(id)))
}

pub(super) fn empty_control_node(node: Option<&&CompiledWorkflowNode>, handler: &str) -> bool {
  node.is_some_and(|node| {
    node.handler == handler
      && node.timeout_ms.is_none()
      && node.retry_policy.is_none()
      && node.script_runtime.is_none()
      && node.metadata.is_none()
      && matches!(&node.inputs, ValueExpression::Object { fields } if fields.is_empty())
  })
}

impl CompiledWorkflowDefinition {
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
        | COMPILED_MODEL_SCHEMA_VERSION_V15
        | COMPILED_MODEL_SCHEMA_VERSION_V16
        | COMPILED_MODEL_SCHEMA_VERSION_V17
        | COMPILED_MODEL_SCHEMA_VERSION_V18
        | COMPILED_MODEL_SCHEMA_VERSION_V19
        | COMPILED_MODEL_SCHEMA_VERSION_V20
        | COMPILED_MODEL_SCHEMA_VERSION_V21
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
        | COMPILED_MODEL_SCHEMA_VERSION_V14
        | COMPILED_MODEL_SCHEMA_VERSION_V15
        | COMPILED_MODEL_SCHEMA_VERSION_V16
        | COMPILED_MODEL_SCHEMA_VERSION_V17
        | COMPILED_MODEL_SCHEMA_VERSION_V18
        | COMPILED_MODEL_SCHEMA_VERSION_V19
        | COMPILED_MODEL_SCHEMA_VERSION_V20
        | COMPILED_MODEL_SCHEMA_VERSION_V21,
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
          | COMPILED_MODEL_SCHEMA_VERSION_V15
          | COMPILED_MODEL_SCHEMA_VERSION_V16
          | COMPILED_MODEL_SCHEMA_VERSION_V17
          | COMPILED_MODEL_SCHEMA_VERSION_V18
          | COMPILED_MODEL_SCHEMA_VERSION_V19
          | COMPILED_MODEL_SCHEMA_VERSION_V20
          | COMPILED_MODEL_SCHEMA_VERSION_V21
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
    match (self.schema_version, &self.communication) {
      (
        COMPILED_MODEL_SCHEMA_VERSION_V15
        | COMPILED_MODEL_SCHEMA_VERSION_V16
        | COMPILED_MODEL_SCHEMA_VERSION_V17
        | COMPILED_MODEL_SCHEMA_VERSION_V18
        | COMPILED_MODEL_SCHEMA_VERSION_V19
        | COMPILED_MODEL_SCHEMA_VERSION_V20
        | COMPILED_MODEL_SCHEMA_VERSION_V21,
        Some(communication),
      ) => {
        let valid = communication.profile_version == 1
          && !communication.providers.is_empty()
          && communication.providers.len() <= 4
          && communication
            .providers
            .windows(2)
            .all(|pair| pair[0].provider < pair[1].provider)
          && communication.providers.iter().all(|provider| {
            matches!(
              provider.provider.as_str(),
              "telegram" | "discord" | "whatsapp"
            ) && (!provider.trigger_ids.is_empty()
              || !provider.notification_delivery_ids.is_empty()
              || provider.messaging)
              && provider.trigger_ids.iter().all(|id| valid_id(id))
              && provider
                .notification_delivery_ids
                .iter()
                .all(|id| valid_id(id))
              && !provider.credential_names.is_empty()
              && provider
                .credential_names
                .windows(2)
                .all(|pair| pair[0] < pair[1])
              && provider
                .credential_names
                .iter()
                .all(|name| valid_secret_name(name))
          });
        if !valid {
          issues.push(issue(
            ModelIssueCode::InvalidValueExpression,
            "Model v15 communication requirements do not match Communication Requirements v1.",
          ));
        }
      }
      (COMPILED_MODEL_SCHEMA_VERSION_V15, None) => issues.push(issue(
        ModelIssueCode::InvalidValueExpression,
        "Model v15 requires communication requirements.",
      )),
      (_, Some(_)) => issues.push(issue(
        ModelIssueCode::InvalidValueExpression,
        "communication requirements are available only on Model v15 or later.",
      )),
      _ => {}
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
    inspect_for_each_contract(self, &mut issues);
    inspect_durable_wait_contract(self, &mut issues);
    inspect_lock_contract(self, &mut issues);
    inspect_saga_contract(self, &mut issues);
    inspect_execution_placement_contract(self, &mut issues);
    issues
  }

  fn inspect_executable_profile(
    &self,
    issues: &mut Vec<ModelIssue>,
    allow_approval: bool,
    allow_retry: bool,
    durable: bool,
  ) {
    if self.schema_version == COMPILED_MODEL_SCHEMA_VERSION_V21 {
      issues.push(issue(
        ModelIssueCode::UnsupportedRemoteExecution,
        "Compiled Model v21 remote execution is staged until the durable dispatch authority is enabled.",
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
          approval_wait_inputs(Some(node), &provider_ids, self.schema_version)
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
        ("engine.for-each-open" | "engine.for-each-result", ValueExpression::Object { fields })
          if self.schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V16 =>
        {
          fields.is_empty()
        }
        ("engine.durable-wait", ValueExpression::Object { fields })
          if self.schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V17 && durable =>
        {
          fields.is_empty()
        }
        (
          "engine.lock-acquire" | "engine.lock-release" | "engine.lock-result",
          ValueExpression::Object { fields },
        ) if self.schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V18 && durable => {
          fields.is_empty()
        }
        (
          "engine.saga-open" | "engine.saga-commit" | "engine.saga-result",
          ValueExpression::Object { fields },
        ) if self.schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V19 && durable => {
          fields.is_empty()
        }
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

    if self.schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V16 {
      for descriptor in self.graph.for_each.as_deref().unwrap_or_default() {
        let fe5_body = descriptor.body.entry_node_ids.len() == 1
          && descriptor.body.nodes.iter().all(|node| {
            matches!(
              node.handler.as_str(),
              "runtime.script"
                | "engine.choice-select"
                | "engine.choice-join"
                | "engine.choice-result"
                | "engine.parallel-start"
                | "engine.parallel-join"
            ) && node.timeout_ms.is_none()
          });
        if !fe5_body {
          issues.push(issue(
            ModelIssueCode::UnsupportedForEachExecution,
            format!(
              "For-each {:?} contains a body that is not executable by the FE5 bounded iteration runtime.",
              descriptor.for_each_id
            ),
          ));
        }
        if descriptor
          .body
          .nodes
          .iter()
          .any(|node| node.handler == "engine.durable-wait")
        {
          issues.push(issue(
            ModelIssueCode::UnsupportedDurableWaitExecution,
            format!(
              "For-each {:?} contains durable waits; iteration suspension is staged for DW6.",
              descriptor.for_each_id
            ),
          ));
        }
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
