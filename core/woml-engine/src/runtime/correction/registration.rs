//! Scoped compensation identities and atomic forward-success registration.
//!
//! Keep success and its registration in one append operation; the existing
//! durable engine/store remains the transaction owner. Unknown success cannot
//! authorize a guessed compensation.

use crate::event::CompensationRegisteredData;
use crate::event::StepAttemptSucceededData;
use crate::model::CompiledCorrectionIdentity;
use crate::model::CompiledCorrectiveAction;
use crate::runtime::errors::RuntimeExecutionError;
use crate::runtime::RuntimeDagEngine;
use crate::CompiledWorkflowDefinition;
use crate::RunEventPayload;

fn compensation_registration(
  workflow: &CompiledWorkflowDefinition,
  node_id: &str,
) -> Option<(String, String)> {
  let saga = workflow.saga_for_body_node(node_id)?;
  let action = saga
    .compensations
    .iter()
    .find(|action| action.for_step_id.as_deref() == Some(node_id))?;
  Some((saga.saga_id.clone(), action.action_id.clone()))
}

pub(in crate::runtime) fn compensation_registration_for_iteration(
  workflow: &CompiledWorkflowDefinition,
  descriptor: &crate::model::CompiledForEach,
  node_id: &str,
  index: u32,
) -> Option<(String, String)> {
  let saga = workflow.saga_for_body_node(&descriptor.open_node_id)?;
  let action = saga.compensations.iter().find(|action| {
    action.for_step_id.as_deref() == Some(node_id)
      && matches!(
        &action.identity,
        CompiledCorrectionIdentity::ForEachIteration { for_each_id, .. }
          if for_each_id == &descriptor.for_each_id
      )
  })?;
  let CompiledCorrectionIdentity::ForEachIteration {
    action_id_template, ..
  } = &action.identity
  else {
    return None;
  };
  Some((
    saga.saga_id.clone(),
    action_id_template.replace("{iterationKey}", &index.to_string()),
  ))
}

pub(in crate::runtime) fn correction_iteration_identity<'a>(
  action: &'a CompiledCorrectiveAction,
  runtime_action_id: &str,
) -> Option<(&'a str, u32)> {
  let CompiledCorrectionIdentity::ForEachIteration {
    for_each_id,
    action_id_template,
  } = &action.identity
  else {
    return None;
  };
  let (prefix, suffix) = action_id_template.split_once("{iterationKey}")?;
  let key = runtime_action_id
    .strip_prefix(prefix)?
    .strip_suffix(suffix)?;
  Some((for_each_id, key.parse().ok()?))
}

pub(in crate::runtime) fn corrective_action_matches_runtime(
  action: &CompiledCorrectiveAction,
  runtime_action_id: &str,
) -> bool {
  match &action.identity {
    CompiledCorrectionIdentity::Static { action_id } => action_id == runtime_action_id,
    CompiledCorrectionIdentity::ForEachIteration { .. } => {
      correction_iteration_identity(action, runtime_action_id).is_some()
    }
  }
}

pub(in crate::runtime) fn append_step_attempt_success<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  succeeded: StepAttemptSucceededData,
) -> Result<(), RuntimeExecutionError> {
  append_step_attempt_success_payload(
    engine,
    run_id,
    RunEventPayload::StepAttemptSucceeded(succeeded),
  )
}

pub(in crate::runtime) fn append_step_attempt_success_payload<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  succeeded: RunEventPayload,
) -> Result<(), RuntimeExecutionError> {
  let RunEventPayload::StepAttemptSucceeded(data) = &succeeded else {
    return Err(RuntimeExecutionError::Stalled(
      "compensation registration requires a step success event".to_string(),
    ));
  };
  let node_id = data.node_id.clone();
  let Some((saga_id, action_id)) = compensation_registration(engine.workflow(), &node_id) else {
    return engine.append_payload(run_id, succeeded);
  };
  let completion_sequence = engine.events(run_id)?.len() as u64 + 1;
  engine.append_payloads(
    run_id,
    vec![
      succeeded,
      RunEventPayload::CompensationRegistered(CompensationRegisteredData {
        saga_id,
        action_id,
        for_step_id: node_id,
        completion_sequence,
      }),
    ],
  )
}
