//! Private remote dispatch validation.

use crate::model::CompiledWorkflowNode;
use crate::remote_dispatch::valid_digest;
use crate::remote_dispatch::valid_id;
use crate::remote_dispatch::RemoteDispatchError;
use crate::remote_dispatch::RemoteDispatchRequestV1;
use crate::CompiledWorkflowDefinition;
use rusqlite::Connection;
use rusqlite::OptionalExtension;

pub(in crate::remote_dispatch) fn validate_request(
  request: &RemoteDispatchRequestV1,
) -> Result<(), RemoteDispatchError> {
  if !valid_id(&request.run_id)
    || !valid_id(&request.node_id)
    || (!request.iteration_identity.is_empty() && !valid_id(&request.iteration_identity))
    || !(1..=10).contains(&request.attempt)
    || !valid_id(&request.invocation_id)
    || !valid_id(&request.target)
    || !valid_digest(&request.target_digest)
    || !valid_digest(&request.package_digest)
  {
    return Err(RemoteDispatchError::Invalid);
  }
  Ok(())
}

pub(in crate::remote_dispatch) fn validate_request_against_definition(
  connection: &Connection,
  request: &RemoteDispatchRequestV1,
) -> Result<(), RemoteDispatchError> {
  let model_json: String = connection
    .query_row(
      "SELECT definitions.model_json
       FROM woml_runs AS runs
       JOIN woml_definitions AS definitions
         ON definitions.definition_hash = runs.definition_hash
       WHERE runs.run_id=?1",
      [&request.run_id],
      |row| row.get(0),
    )
    .optional()?
    .ok_or(RemoteDispatchError::NotFound)?;
  let workflow: CompiledWorkflowDefinition =
    serde_json::from_str(&model_json).map_err(|_| RemoteDispatchError::Corrupt)?;
  workflow
    .validate_structure()
    .map_err(|_| RemoteDispatchError::Corrupt)?;
  if workflow.schema_version != crate::COMPILED_MODEL_SCHEMA_VERSION_V21 {
    return Err(RemoteDispatchError::Conflict);
  }
  let nodes = workflow
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
    .chain(
      workflow
        .graph
        .sagas
        .as_deref()
        .unwrap_or_default()
        .iter()
        .flat_map(|saga| saga.compensations.iter().chain(saga.cleanup.iter()))
        .map(|action| &action.node),
    )
    .filter(|node| node.id == request.node_id)
    .collect::<Vec<&CompiledWorkflowNode>>();
  if nodes.len() != 1 {
    return Err(RemoteDispatchError::Conflict);
  }
  let node = nodes[0];
  let maximum_attempts = node
    .retry_policy
    .as_ref()
    .map_or(1, |retry| retry.max_attempts);
  if node.handler != "runtime.script"
    || node
      .execution
      .as_ref()
      .is_none_or(|execution| execution.target == "local" || execution.target != request.target)
    || request.attempt > maximum_attempts
  {
    return Err(RemoteDispatchError::Conflict);
  }
  Ok(())
}
