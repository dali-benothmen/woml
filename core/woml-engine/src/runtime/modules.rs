//! runtime modules.

use super::{sha256_identity, RuntimeExecutionError, RuntimeExecutionOptions};
use crate::CompiledWorkflowDefinition;
use crate::DurableEventStore;
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeModuleArtifact {
  pub name: String,
  pub bundle_digest: String,
  pub source_map_digest: String,
  pub exports: Vec<String>,
  pub bundle: String,
  pub source_map: String,
}

pub(super) fn runtime_modules_from_store(
  options: RuntimeExecutionOptions,
  store: &DurableEventStore,
  definition_hash: &str,
) -> Result<RuntimeExecutionOptions, RuntimeExecutionError> {
  let stored = store.definition_module_artifacts(definition_hash)?;
  if options.runtime_modules.is_empty() {
    return Ok(options.with_runtime_modules(stored));
  }
  if options.runtime_modules.as_ref() != &stored {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "supplied module artifacts do not match the durable definition artifacts".to_string(),
    ));
  }
  Ok(options)
}

pub(super) fn reusable_lifecycle_artifact_name(action_id: &str) -> String {
  format!(
    "__woml_lifecycle__{}",
    sha256_identity(action_id).trim_start_matches("sha256:")
  )
}

pub(super) fn validate_runtime_modules(
  workflow: &CompiledWorkflowDefinition,
  options: &RuntimeExecutionOptions,
) -> Result<(), RuntimeExecutionError> {
  let module_artifacts = options
    .runtime_modules
    .iter()
    .filter(|artifact| !artifact.name.starts_with("__woml_"))
    .collect::<Vec<_>>();
  let provider_artifacts = options
    .runtime_modules
    .iter()
    .filter(|artifact| artifact.name.starts_with("__woml_provider__"))
    .collect::<Vec<_>>();
  let reusable_step_artifacts = options
    .runtime_modules
    .iter()
    .filter(|artifact| artifact.name.starts_with("__woml_reusable__"))
    .collect::<Vec<_>>();
  let reusable_lifecycle_artifacts = options
    .runtime_modules
    .iter()
    .filter(|artifact| artifact.name.starts_with("__woml_lifecycle__"))
    .collect::<Vec<_>>();
  let runtime_bindings = workflow
    .module_runtime
    .as_ref()
    .map_or(&[][..], |runtime| runtime.modules.as_slice());
  if runtime_bindings.len() != module_artifacts.len() {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "the runtime module artifacts do not match Model v9".to_string(),
    ));
  }
  let mut total_bytes = 0usize;
  for (binding, artifact) in runtime_bindings.iter().zip(module_artifacts) {
    total_bytes = total_bytes
      .checked_add(artifact.bundle.len())
      .and_then(|value| value.checked_add(artifact.source_map.len()))
      .ok_or_else(|| {
        RuntimeExecutionError::InvalidConfiguration(
          "runtime module artifact byte count overflowed".to_string(),
        )
      })?;
    let matches = binding.name == artifact.name
      && binding.bundle_digest == artifact.bundle_digest
      && binding.source_map_digest == artifact.source_map_digest
      && binding.exports == artifact.exports
      && sha256_identity(&artifact.bundle) == artifact.bundle_digest
      && sha256_identity(&artifact.source_map) == artifact.source_map_digest
      && artifact.bundle.len() <= crate::durable::MAX_MODULE_ARTIFACT_BYTES
      && artifact.source_map.len() <= crate::durable::MAX_MODULE_ARTIFACT_BYTES;
    if !matches {
      return Err(RuntimeExecutionError::InvalidConfiguration(format!(
        "runtime module artifact {:?} failed its Model v9 identity or size check",
        binding.name
      )));
    }
    if options.resolved_secrets.values().any(|secret| {
      !secret.is_empty()
        && (artifact.bundle.contains(secret) || artifact.source_map.contains(secret))
    }) {
      return Err(RuntimeExecutionError::InvalidConfiguration(format!(
        "runtime module artifact {:?} contains a resolved secret value",
        binding.name
      )));
    }
  }
  let provider_descriptors = workflow
    .reusable_definitions
    .iter()
    .flatten()
    .filter_map(|definition| match definition {
      crate::model::CompiledReusableInvocation::NotificationProvider {
        script_artifact_id, ..
      } => Some(script_artifact_id.as_str()),
      _ => None,
    })
    .collect::<HashSet<_>>();
  let mut provider_ids = HashSet::new();
  for artifact in provider_artifacts {
    let artifact_id = artifact.name.trim_start_matches("__woml_provider__");
    total_bytes = total_bytes
      .checked_add(artifact.bundle.len())
      .and_then(|value| value.checked_add(artifact.source_map.len()))
      .ok_or_else(|| {
        RuntimeExecutionError::InvalidConfiguration(
          "custom-provider artifact byte count overflowed".to_string(),
        )
      })?;
    if !provider_descriptors.contains(artifact_id)
      || !provider_ids.insert(artifact_id)
      || !artifact.exports.is_empty()
      || !artifact.source_map.is_empty()
      || sha256_identity(&artifact.bundle) != artifact.bundle_digest
      || sha256_identity("") != artifact.source_map_digest
      || artifact.bundle.len() > crate::durable::MAX_MODULE_ARTIFACT_BYTES
    {
      return Err(RuntimeExecutionError::InvalidConfiguration(format!(
        "custom-provider artifact {artifact_id:?} failed its immutable identity or size check"
      )));
    }
  }
  if provider_ids.len() != provider_descriptors.len() {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "the custom-provider runtime artifacts do not match Model v14".to_string(),
    ));
  }
  let reusable_step_descriptors = workflow
    .reusable_definitions
    .iter()
    .flatten()
    .filter_map(|definition| match definition {
      crate::model::CompiledReusableInvocation::Step {
        script_artifact_id, ..
      } => Some(script_artifact_id.as_str()),
      _ => None,
    })
    .collect::<HashSet<_>>();
  let mut reusable_step_ids = HashSet::new();
  for artifact in reusable_step_artifacts {
    let artifact_id = artifact.name.trim_start_matches("__woml_reusable__");
    total_bytes = total_bytes
      .checked_add(artifact.bundle.len())
      .and_then(|value| value.checked_add(artifact.source_map.len()))
      .ok_or_else(|| {
        RuntimeExecutionError::InvalidConfiguration(
          "custom-step artifact byte count overflowed".to_string(),
        )
      })?;
    if !reusable_step_descriptors.contains(artifact_id)
      || !reusable_step_ids.insert(artifact_id)
      || !artifact.exports.is_empty()
      || !artifact.source_map.is_empty()
      || sha256_identity(&artifact.bundle) != artifact.bundle_digest
      || sha256_identity("") != artifact.source_map_digest
      || artifact.bundle.len() > crate::durable::MAX_MODULE_ARTIFACT_BYTES
    {
      return Err(RuntimeExecutionError::InvalidConfiguration(format!(
        "custom-step artifact {artifact_id:?} failed its immutable identity or size check"
      )));
    }
  }
  if reusable_step_ids.len() != reusable_step_descriptors.len() {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "the custom-step runtime artifacts do not match Model v14".to_string(),
    ));
  }
  let reusable_lifecycle_names = workflow
    .reusable_definitions
    .iter()
    .flatten()
    .flat_map(|definition| match definition {
      crate::model::CompiledReusableInvocation::Step { lifecycle, .. }
      | crate::model::CompiledReusableInvocation::NotificationProvider { lifecycle, .. } => {
        lifecycle
          .iter()
          .flat_map(|lifecycle| {
            lifecycle
              .on_success
              .iter()
              .chain(&lifecycle.on_error)
              .chain(&lifecycle.on_complete)
              .flat_map(|actions| actions.iter())
              .map(|action_id| reusable_lifecycle_artifact_name(action_id))
              .collect::<Vec<_>>()
          })
          .collect::<Vec<_>>()
      }
    })
    .collect::<HashSet<_>>();
  let mut found_lifecycle_names = HashSet::new();
  for artifact in reusable_lifecycle_artifacts {
    total_bytes = total_bytes
      .checked_add(artifact.bundle.len())
      .and_then(|value| value.checked_add(artifact.source_map.len()))
      .ok_or_else(|| {
        RuntimeExecutionError::InvalidConfiguration(
          "reusable lifecycle artifact byte count overflowed".to_string(),
        )
      })?;
    if !reusable_lifecycle_names.contains(&artifact.name)
      || !found_lifecycle_names.insert(artifact.name.as_str())
      || !artifact.exports.is_empty()
      || !artifact.source_map.is_empty()
      || sha256_identity(&artifact.bundle) != artifact.bundle_digest
      || sha256_identity("") != artifact.source_map_digest
      || artifact.bundle.len() > crate::durable::MAX_MODULE_ARTIFACT_BYTES
    {
      return Err(RuntimeExecutionError::InvalidConfiguration(format!(
        "reusable lifecycle artifact {:?} failed its immutable identity or size check",
        artifact.name
      )));
    }
  }
  if found_lifecycle_names.len() != reusable_lifecycle_names.len() {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "the reusable lifecycle runtime artifacts do not match Model v14".to_string(),
    ));
  }
  if total_bytes > crate::durable::MAX_MODULE_ARTIFACT_SET_BYTES {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "runtime module artifacts exceed the per-definition cache limit".to_string(),
    ));
  }
  Ok(())
}
