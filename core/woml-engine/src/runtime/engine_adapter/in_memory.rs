//! Runtime adaptation to validated in-memory execution, including shared historical compatibility semantics and deliberate rejection of unsupported durable operations.

use crate::event::ForEachIterationScope;
use crate::runtime::RuntimeExecutionError;
use crate::runtime::{generated_id, RuntimeDagEngine};
use crate::CompiledWorkflowDefinition;
use crate::InMemoryDagEngine;
use crate::RunEvent;
use crate::RunEventPayload;
use crate::RunProjection;
use serde_json::Map;
use serde_json::Value;

impl RuntimeDagEngine for InMemoryDagEngine {
  fn workflow(&self) -> &CompiledWorkflowDefinition {
    self.workflow()
  }

  fn definition_hash(&self) -> &str {
    self.definition_hash()
  }

  fn start_run(
    &mut self,
    run_id: &str,
    trigger: Map<String, Value>,
  ) -> Result<(), RuntimeExecutionError> {
    if self.event_schema_version() >= crate::RUN_EVENT_SCHEMA_VERSION_V7 {
      let trigger_definition = self
        .workflow()
        .triggers
        .iter()
        .find(|candidate| candidate.handler == "trigger.manual")
        .ok_or_else(|| {
          RuntimeExecutionError::InvalidConfiguration(
            "a direct Model v7+ run requires a manual trigger".to_string(),
          )
        })?;
      let started = RunEventPayload::RunStarted(crate::RunStartedData {
        workflow_id: self.workflow().workflow_id.clone(),
        definition_hash: self.definition_hash().to_string(),
        trigger_id: Some(trigger_definition.id.clone()),
        trigger_handler: Some(trigger_definition.handler.clone()),
        trigger_occurrence_id: Some(generated_id("occ")),
        ingress: None,
        trigger,
      });
      let payloads = crate::durable::expand_model_v11_payload(self.workflow(), run_id, started)?;
      for payload in payloads {
        self.append_event(RunEvent {
          event_schema_version: self.event_schema_version(),
          event_id: generated_id("evt"),
          run_id: run_id.to_string(),
          sequence: self.events(run_id).len() as u64 + 1,
          occurred_at: chrono::Utc::now(),
          iteration: None,
          payload,
        })?;
      }
      return Ok(());
    }
    self.start_run(generated_id("evt"), run_id, chrono::Utc::now(), trigger)?;
    Ok(())
  }

  fn append_payload(
    &mut self,
    run_id: &str,
    payload: RunEventPayload,
  ) -> Result<(), RuntimeExecutionError> {
    let event_schema_version = self.event_schema_version();
    let payloads = if matches!(
      event_schema_version,
      crate::RUN_EVENT_SCHEMA_VERSION_V10
        | crate::RUN_EVENT_SCHEMA_VERSION_V11
        | crate::RUN_EVENT_SCHEMA_VERSION_V12
        | crate::RUN_EVENT_SCHEMA_VERSION_V13
        | crate::RUN_EVENT_SCHEMA_VERSION_V14
        | crate::RUN_EVENT_SCHEMA_VERSION_V15
        | crate::RUN_EVENT_SCHEMA_VERSION_V16
        | crate::RUN_EVENT_SCHEMA_VERSION_V17
        | crate::RUN_EVENT_SCHEMA_VERSION_V18
        | crate::RUN_EVENT_SCHEMA_VERSION_V19
        | crate::RUN_EVENT_SCHEMA_VERSION_V20
    ) {
      crate::durable::expand_model_v11_payload(self.workflow(), run_id, payload)?
    } else {
      vec![payload]
    };
    let occurred_at = chrono::Utc::now();
    for payload in payloads {
      let sequence = self.events(run_id).len() as u64 + 1;
      self.append_event(RunEvent {
        event_schema_version,
        event_id: generated_id("evt"),
        run_id: run_id.to_string(),
        sequence,
        occurred_at,
        iteration: None,
        payload,
      })?;
    }
    Ok(())
  }

  fn append_scoped_payload(
    &mut self,
    run_id: &str,
    iteration: ForEachIterationScope,
    payload: RunEventPayload,
  ) -> Result<(), RuntimeExecutionError> {
    self.append_event(RunEvent {
      event_schema_version: self.event_schema_version(),
      event_id: generated_id("evt"),
      run_id: run_id.to_string(),
      sequence: self.events(run_id).len() as u64 + 1,
      occurred_at: chrono::Utc::now(),
      iteration: Some(iteration),
      payload,
    })?;
    Ok(())
  }

  fn projection(&self, run_id: &str) -> Result<RunProjection, RuntimeExecutionError> {
    Ok(self.projection(run_id)?)
  }

  fn events(&self, run_id: &str) -> Result<Vec<RunEvent>, RuntimeExecutionError> {
    Ok(self.events(run_id).to_vec())
  }

  fn ready_node_ids(&self, run_id: &str) -> Result<Vec<String>, RuntimeExecutionError> {
    Ok(self.ready_node_ids(run_id)?)
  }

  fn correction_action_externally_resolved(
    &self,
    _run_id: &str,
    _saga_id: &str,
    _action_id: &str,
  ) -> Result<bool, RuntimeExecutionError> {
    Ok(false)
  }
}
