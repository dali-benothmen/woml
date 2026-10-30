//! Immutable definitions, stored module artifacts, and exact run bindings.
//!
//! Existing hashes are checked against decoded content; recovery loads the
//! registered program rather than substituting current filesystem source.

use crate::durable::{DurableEventStore, DurableStoreError, RunDefinitionBinding};
use crate::event::is_definition_hash;
use crate::runtime::RuntimeModuleArtifact;
use crate::CompiledWorkflowDefinition;
use chrono::Utc;
use rusqlite::params;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::TransactionBehavior;
use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;
use std::collections::HashSet;

pub const MAX_MODULE_ARTIFACT_BYTES: usize = 3 * 1024 * 1024;

pub const MAX_MODULE_ARTIFACT_SET_BYTES: usize = 32 * 1024 * 1024;

fn module_artifact_sha256(content: &str) -> String {
  format!("sha256:{:x}", Sha256::digest(content.as_bytes()))
}

fn reusable_lifecycle_artifact_name(action_id: &str) -> String {
  format!(
    "__woml_lifecycle__{}",
    module_artifact_sha256(action_id).trim_start_matches("sha256:")
  )
}

fn validate_definition_module_artifacts(
  workflow: &CompiledWorkflowDefinition,
  artifacts: &[RuntimeModuleArtifact],
) -> Result<(), DurableStoreError> {
  let module_artifacts = artifacts
    .iter()
    .filter(|artifact| !artifact.name.starts_with("__woml_"))
    .collect::<Vec<_>>();
  let provider_artifacts = artifacts
    .iter()
    .filter(|artifact| artifact.name.starts_with("__woml_provider__"))
    .collect::<Vec<_>>();
  let reusable_step_artifacts = artifacts
    .iter()
    .filter(|artifact| artifact.name.starts_with("__woml_reusable__"))
    .collect::<Vec<_>>();
  let reusable_lifecycle_artifacts = artifacts
    .iter()
    .filter(|artifact| artifact.name.starts_with("__woml_lifecycle__"))
    .collect::<Vec<_>>();
  let runtime_bindings = workflow
    .module_runtime
    .as_ref()
    .map_or(&[][..], |runtime| runtime.modules.as_slice());
  if runtime_bindings.len() != module_artifacts.len() {
    return Err(DurableStoreError::Contract(
      "Stored module artifacts do not match the compiled definition.".to_string(),
    ));
  }
  let mut total_bytes = 0usize;
  for (binding, artifact) in runtime_bindings.iter().zip(module_artifacts) {
    let bundle_bytes = artifact.bundle.len();
    let source_map_bytes = artifact.source_map.len();
    total_bytes = total_bytes
      .checked_add(bundle_bytes)
      .and_then(|value| value.checked_add(source_map_bytes))
      .ok_or_else(|| {
        DurableStoreError::Contract("Module artifact byte count overflowed.".to_string())
      })?;
    if binding.name != artifact.name
      || binding.bundle_digest != artifact.bundle_digest
      || binding.source_map_digest != artifact.source_map_digest
      || binding.exports != artifact.exports
      || module_artifact_sha256(&artifact.bundle) != artifact.bundle_digest
      || module_artifact_sha256(&artifact.source_map) != artifact.source_map_digest
      || bundle_bytes > MAX_MODULE_ARTIFACT_BYTES
      || source_map_bytes > MAX_MODULE_ARTIFACT_BYTES
    {
      return Err(DurableStoreError::Contract(format!(
        "Module artifact {:?} failed its immutable identity or size contract.",
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
        DurableStoreError::Contract("Provider artifact byte count overflowed.".to_string())
      })?;
    if !provider_descriptors.contains(artifact_id)
      || !provider_ids.insert(artifact_id)
      || !artifact.exports.is_empty()
      || !artifact.source_map.is_empty()
      || module_artifact_sha256(&artifact.bundle) != artifact.bundle_digest
      || module_artifact_sha256("") != artifact.source_map_digest
      || artifact.bundle.len() > MAX_MODULE_ARTIFACT_BYTES
    {
      return Err(DurableStoreError::Contract(format!(
        "Provider artifact {artifact_id:?} failed its immutable identity or size contract."
      )));
    }
  }
  if provider_ids.len() != provider_descriptors.len() {
    return Err(DurableStoreError::Contract(
      "Stored provider artifacts do not match the compiled definition.".to_string(),
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
        DurableStoreError::Contract("Custom-step artifact byte count overflowed.".to_string())
      })?;
    if !reusable_step_descriptors.contains(artifact_id)
      || !reusable_step_ids.insert(artifact_id)
      || !artifact.exports.is_empty()
      || !artifact.source_map.is_empty()
      || module_artifact_sha256(&artifact.bundle) != artifact.bundle_digest
      || module_artifact_sha256("") != artifact.source_map_digest
      || artifact.bundle.len() > MAX_MODULE_ARTIFACT_BYTES
    {
      return Err(DurableStoreError::Contract(format!(
        "Custom-step artifact {artifact_id:?} failed its immutable identity or size contract."
      )));
    }
  }
  if reusable_step_ids.len() != reusable_step_descriptors.len() {
    return Err(DurableStoreError::Contract(
      "Stored custom-step artifacts do not match the compiled definition.".to_string(),
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
        DurableStoreError::Contract(
          "Reusable lifecycle artifact byte count overflowed.".to_string(),
        )
      })?;
    if !reusable_lifecycle_names.contains(&artifact.name)
      || !found_lifecycle_names.insert(artifact.name.as_str())
      || !artifact.exports.is_empty()
      || !artifact.source_map.is_empty()
      || module_artifact_sha256(&artifact.bundle) != artifact.bundle_digest
      || module_artifact_sha256("") != artifact.source_map_digest
      || artifact.bundle.len() > MAX_MODULE_ARTIFACT_BYTES
    {
      return Err(DurableStoreError::Contract(format!(
        "Reusable lifecycle artifact {:?} failed its immutable identity or size contract.",
        artifact.name
      )));
    }
  }
  if found_lifecycle_names.len() != reusable_lifecycle_names.len() {
    return Err(DurableStoreError::Contract(
      "Stored reusable lifecycle artifacts do not match the compiled definition.".to_string(),
    ));
  }
  if total_bytes > MAX_MODULE_ARTIFACT_SET_BYTES {
    return Err(DurableStoreError::Contract(format!(
      "Module artifacts exceed the {} byte definition limit.",
      MAX_MODULE_ARTIFACT_SET_BYTES
    )));
  }
  Ok(())
}

pub(in crate::durable) fn definition_by_hash(
  connection: &Connection,
  definition_hash: &str,
) -> Result<CompiledWorkflowDefinition, DurableStoreError> {
  let model_json: String = connection
    .query_row(
      "SELECT model_json FROM woml_definitions WHERE definition_hash = ?1",
      [definition_hash],
      |row| row.get(0),
    )
    .optional()?
    .ok_or_else(|| DurableStoreError::DefinitionNotFound(definition_hash.to_string()))?;
  let workflow: CompiledWorkflowDefinition = serde_json::from_str(&model_json)?;
  workflow.validate_structure()?;
  Ok(workflow)
}

pub(in crate::durable) fn load_run_binding_optional(
  connection: &Connection,
  run_id: &str,
) -> Result<Option<RunDefinitionBinding>, DurableStoreError> {
  connection
    .query_row(
      "SELECT run_id, workflow_id, definition_hash FROM woml_runs WHERE run_id = ?1",
      [run_id],
      |row| {
        Ok(RunDefinitionBinding {
          run_id: row.get(0)?,
          workflow_id: row.get(1)?,
          definition_hash: row.get(2)?,
        })
      },
    )
    .optional()
    .map_err(DurableStoreError::from)
}

pub(in crate::durable) fn run_binding_in_transaction(
  connection: &Connection,
  run_id: &str,
) -> Result<RunDefinitionBinding, DurableStoreError> {
  let mut statement = connection
    .prepare_cached("SELECT workflow_id, definition_hash FROM woml_runs WHERE run_id = ?1")?;
  statement
    .query_row([run_id], |row| {
      Ok(RunDefinitionBinding {
        run_id: run_id.to_string(),
        workflow_id: row.get(0)?,
        definition_hash: row.get(1)?,
      })
    })
    .optional()?
    .ok_or_else(|| DurableStoreError::RunNotFound(run_id.to_string()))
}

pub(in crate::durable) fn definition_for_run(
  connection: &Connection,
  run_id: &str,
) -> Result<CompiledWorkflowDefinition, DurableStoreError> {
  let mut statement = connection.prepare_cached(
    "SELECT definitions.model_json
     FROM woml_runs AS runs
     JOIN woml_definitions AS definitions
       ON definitions.definition_hash = runs.definition_hash
     WHERE runs.run_id = ?1",
  )?;
  let model_json: String = statement
    .query_row([run_id], |row| row.get(0))
    .optional()?
    .ok_or_else(|| DurableStoreError::RunNotFound(run_id.to_string()))?;
  let workflow: CompiledWorkflowDefinition = serde_json::from_str(&model_json)?;
  workflow.validate_structure()?;
  Ok(workflow)
}

impl DurableEventStore {
  pub fn register_definition(
    &mut self,
    workflow: &CompiledWorkflowDefinition,
    definition_hash: &str,
  ) -> Result<(), DurableStoreError> {
    workflow.validate_structure()?;
    if !is_definition_hash(definition_hash) {
      return Err(DurableStoreError::Contract(
        "A durable definition requires a valid RFC 8785 SHA-256 hash.".to_string(),
      ));
    }
    let model_json = serde_json::to_string(workflow)?;
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let stored: Option<(String, String)> = transaction
      .query_row(
        "SELECT workflow_id, model_json FROM woml_definitions WHERE definition_hash = ?1",
        [definition_hash],
        |row| Ok((row.get(0)?, row.get(1)?)),
      )
      .optional()?;
    if let Some((workflow_id, stored_json)) = stored {
      let stored_value: Value = serde_json::from_str(&stored_json)?;
      let incoming_value: Value = serde_json::from_str(&model_json)?;
      if workflow_id != workflow.workflow_id || stored_value != incoming_value {
        return Err(DurableStoreError::DefinitionConflict(
          definition_hash.to_string(),
        ));
      }
    } else {
      transaction.execute(
        "INSERT INTO woml_definitions(
           definition_hash, workflow_id, schema_version, model_json, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
          definition_hash,
          workflow.workflow_id,
          i64::from(workflow.schema_version),
          model_json,
          Utc::now().to_rfc3339(),
        ],
      )?;
    }
    transaction.commit()?;
    Ok(())
  }

  pub fn definition(
    &self,
    definition_hash: &str,
  ) -> Result<CompiledWorkflowDefinition, DurableStoreError> {
    let mut statement = self
      .connection
      .prepare_cached("SELECT model_json FROM woml_definitions WHERE definition_hash = ?1")?;
    let model_json: String = statement
      .query_row([definition_hash], |row| row.get(0))
      .optional()?
      .ok_or_else(|| DurableStoreError::DefinitionNotFound(definition_hash.to_string()))?;
    let workflow: CompiledWorkflowDefinition = serde_json::from_str(&model_json)?;
    workflow.validate_structure()?;
    Ok(workflow)
  }

  pub fn has_definition_for_workflow(&self, workflow_id: &str) -> Result<bool, DurableStoreError> {
    if workflow_id.is_empty() || workflow_id.len() > 256 {
      return Err(DurableStoreError::Contract(
        "A workflow definition lookup requires a valid workflow ID.".to_string(),
      ));
    }
    self
      .connection
      .query_row(
        "SELECT EXISTS(SELECT 1 FROM woml_definitions WHERE workflow_id = ?1)",
        [workflow_id],
        |row| row.get::<_, bool>(0),
      )
      .map_err(DurableStoreError::from)
  }

  pub fn register_definition_module_artifacts(
    &mut self,
    workflow: &CompiledWorkflowDefinition,
    definition_hash: &str,
    artifacts: &[RuntimeModuleArtifact],
  ) -> Result<(), DurableStoreError> {
    let mut span = crate::performance::PerformanceSpan::new("sqlite", "sqlite.register_definition");
    span.count("nodes", workflow.graph.nodes.len());
    span.count("modules", artifacts.len());
    span.count("transactions", 2);
    self.register_definition(workflow, definition_hash)?;
    validate_definition_module_artifacts(workflow, artifacts)?;

    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut statement = transaction.prepare(
      "SELECT module_name, bundle_digest, source_map_digest, exports_json, bundle, source_map
       FROM woml_definition_module_artifacts
       WHERE definition_hash = ?1
       ORDER BY module_name",
    )?;
    let stored = statement
      .query_map([definition_hash], |row| {
        Ok((
          row.get::<_, String>(0)?,
          row.get::<_, String>(1)?,
          row.get::<_, String>(2)?,
          row.get::<_, String>(3)?,
          row.get::<_, String>(4)?,
          row.get::<_, String>(5)?,
        ))
      })?
      .collect::<Result<Vec<_>, _>>()?;
    drop(statement);

    if !stored.is_empty() {
      if stored.len() != artifacts.len() {
        return Err(DurableStoreError::DefinitionConflict(
          definition_hash.to_string(),
        ));
      }
      for (stored, incoming) in stored.iter().zip(artifacts) {
        let exports: Vec<String> = serde_json::from_str(&stored.3)?;
        if stored.0 != incoming.name
          || stored.1 != incoming.bundle_digest
          || stored.2 != incoming.source_map_digest
          || exports != incoming.exports
          || stored.4 != incoming.bundle
          || stored.5 != incoming.source_map
        {
          return Err(DurableStoreError::DefinitionConflict(
            definition_hash.to_string(),
          ));
        }
      }
      transaction.commit()?;
      span.succeed();
      return Ok(());
    }

    for artifact in artifacts {
      transaction.execute(
        "INSERT INTO woml_definition_module_artifacts(
           definition_hash, module_name, bundle_digest, source_map_digest,
           exports_json, bundle, source_map, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
          definition_hash,
          artifact.name,
          artifact.bundle_digest,
          artifact.source_map_digest,
          serde_json::to_string(&artifact.exports)?,
          artifact.bundle,
          artifact.source_map,
          Utc::now().to_rfc3339(),
        ],
      )?;
    }
    transaction.commit()?;
    span.succeed();
    Ok(())
  }

  pub fn definition_module_artifacts(
    &self,
    definition_hash: &str,
  ) -> Result<Vec<RuntimeModuleArtifact>, DurableStoreError> {
    let workflow = self.definition(definition_hash)?;
    let mut statement = self.connection.prepare(
      "SELECT module_name, bundle_digest, source_map_digest, exports_json, bundle, source_map
       FROM woml_definition_module_artifacts
       WHERE definition_hash = ?1
       ORDER BY module_name",
    )?;
    let artifacts = statement
      .query_map([definition_hash], |row| {
        let exports_json: String = row.get(3)?;
        let exports = serde_json::from_str(&exports_json).map_err(|error| {
          rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(error))
        })?;
        Ok(RuntimeModuleArtifact {
          name: row.get(0)?,
          bundle_digest: row.get(1)?,
          source_map_digest: row.get(2)?,
          exports,
          bundle: row.get(4)?,
          source_map: row.get(5)?,
        })
      })?
      .collect::<Result<Vec<_>, _>>()?;
    validate_definition_module_artifacts(&workflow, &artifacts)?;
    Ok(artifacts)
  }

  pub fn run_binding(&self, run_id: &str) -> Result<RunDefinitionBinding, DurableStoreError> {
    let mut statement = self
      .connection
      .prepare_cached("SELECT workflow_id, definition_hash FROM woml_runs WHERE run_id = ?1")?;
    statement
      .query_row([run_id], |row| {
        Ok(RunDefinitionBinding {
          run_id: run_id.to_string(),
          workflow_id: row.get(0)?,
          definition_hash: row.get(1)?,
        })
      })
      .optional()?
      .ok_or_else(|| DurableStoreError::RunNotFound(run_id.to_string()))
  }
}
