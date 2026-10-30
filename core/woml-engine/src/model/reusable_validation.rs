//! model reusable validation.

use super::{
  collect_custom_provider_ids, issue, reusable_prop_secrets, valid_id, valid_portable_path,
  valid_public_structural_id, valid_reusable_alias, valid_reusable_lifecycle, valid_sha256,
  CompiledReusableInvocation, CompiledWorkflowDefinition, ModelIssue, ModelIssueCode,
};
use crate::COMPILED_MODEL_SCHEMA_VERSION_V14;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V15;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V16;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V17;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V18;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V19;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V20;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V21;
use std::collections::HashMap;
use std::collections::HashSet;

pub(super) fn inspect_reusable_contract(
  workflow: &CompiledWorkflowDefinition,
  issues: &mut Vec<ModelIssue>,
) -> HashSet<String> {
  let Some(definitions) = &workflow.reusable_definitions else {
    return HashSet::new();
  };
  if !matches!(
    workflow.schema_version,
    COMPILED_MODEL_SCHEMA_VERSION_V14
      | COMPILED_MODEL_SCHEMA_VERSION_V15
      | COMPILED_MODEL_SCHEMA_VERSION_V16
      | COMPILED_MODEL_SCHEMA_VERSION_V17
      | COMPILED_MODEL_SCHEMA_VERSION_V18
      | COMPILED_MODEL_SCHEMA_VERSION_V19
      | COMPILED_MODEL_SCHEMA_VERSION_V20
      | COMPILED_MODEL_SCHEMA_VERSION_V21
  ) || definitions.is_empty()
    || definitions.len() > 256
  {
    issues.push(issue(
      ModelIssueCode::InvalidReusableDefinition,
      "reusableDefinitions requires between 1 and 256 entries on Model v14 or v15.",
    ));
  }
  let nodes = workflow
    .graph
    .nodes
    .iter()
    .chain(
      workflow
        .graph
        .for_each
        .iter()
        .flatten()
        .flat_map(|loop_| loop_.body.nodes.iter()),
    )
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
        let secrets = reusable_prop_secrets(
          props,
          &identity,
          workflow.schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V16,
          issues,
        );
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
        reusable_prop_secrets(
          props,
          &identity,
          workflow.schema_version >= COMPILED_MODEL_SCHEMA_VERSION_V16,
          issues,
        );
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
