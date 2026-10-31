//! Saga correction ordered by durable completion, followed by explicit cleanup.
//!
//! Bounded recovery discovery returns through existing execution claims. It
//! does not schedule independently or erase the original business failure.

use crate::event::CorrectiveActionKind;
use crate::event::SagaCorrectionReason;
use crate::event::SagaCorrectionStartedData;
use crate::event::SagaIdentityData;
use crate::model::CompiledSaga;
use crate::protocol::CompensationStatusV1;
use crate::runtime::errors::RuntimeExecutionError;
use crate::runtime::options::RuntimeExecutionOptions;
use crate::runtime::{
  corrective_action_matches_runtime, execute_admitted_trigger_run_durable,
  execute_corrective_action, CorrectiveActionOutcome, RuntimeDagEngine,
};
use crate::DurableEventStore;
use crate::RunEventPayload;
use crate::ScriptHostClient;
use std::collections::HashSet;
use std::path::PathBuf;

use crate::runtime::CorrectionRecoveryResume;

/// Resumes only bounded correction work owned by the calling production
/// runtime. The durable policy claim remains the single execution authority;
/// this scan is discovery, not a second scheduler or an in-memory claim.
pub(crate) async fn resume_corrections_for_workflows_durable(
  database_path: PathBuf,
  options: RuntimeExecutionOptions,
  limit: usize,
  workflow_ids: &HashSet<String>,
) -> Result<Vec<CorrectionRecoveryResume>, RuntimeExecutionError> {
  let candidates = DurableEventStore::open(database_path.clone())?
    .correction_recovery_batch_for_workflows_v1(limit, workflow_ids)?;
  let mut run_ids = candidates
    .into_iter()
    .map(|candidate| candidate.run_id)
    .collect::<Vec<_>>();
  run_ids.sort();
  run_ids.dedup();

  let mut resumed = Vec::with_capacity(run_ids.len());
  for run_id in run_ids {
    let mut store = DurableEventStore::open(database_path.clone())?;
    if store.policy_run_has_live_claim(&run_id, chrono::Utc::now())? {
      continue;
    }
    resumed.push(CorrectionRecoveryResume {
      run_id: run_id.clone(),
      outcome: execute_admitted_trigger_run_durable(
        database_path.clone(),
        &run_id,
        options.clone(),
      )
      .await,
    });
  }
  Ok(resumed)
}

pub(in crate::runtime) async fn correct_saga_failure_for_node<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  node_id: &str,
  options: &RuntimeExecutionOptions,
) -> Result<(), RuntimeExecutionError> {
  let Some(saga) = engine.workflow().saga_for_body_node(node_id).cloned() else {
    return Ok(());
  };
  let mut correction_host = None;
  let correction_completed = execute_saga_correction(
    engine,
    run_id,
    &saga,
    SagaCorrectionReason::Failure,
    options,
    &mut correction_host,
  )
  .await?;
  if !correction_completed
    && engine
      .projection(run_id)?
      .sagas
      .get(&saga.saga_id)
      .is_some_and(|state| state.phase == crate::SagaPhase::ManualRecovery)
  {
    return Err(RuntimeExecutionError::Stalled(format!(
      "saga {:?} requires manual recovery",
      saga.saga_id
    )));
  }
  Ok(())
}

pub(in crate::runtime) async fn execute_saga_correction<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  saga: &CompiledSaga,
  reason: SagaCorrectionReason,
  options: &RuntimeExecutionOptions,
  host: &mut Option<ScriptHostClient>,
) -> Result<bool, RuntimeExecutionError> {
  let phase = engine
    .projection(run_id)?
    .sagas
    .get(&saga.saga_id)
    .map(|state| state.phase)
    .ok_or_else(|| RuntimeExecutionError::Stalled("saga correction has no open scope".into()))?;
  if !matches!(
    phase,
    crate::SagaPhase::Correcting | crate::SagaPhase::ManualRecovery
  ) {
    engine.append_payload(
      run_id,
      RunEventPayload::SagaCorrectionStarted(SagaCorrectionStartedData {
        saga_id: saga.saga_id.clone(),
        reason,
      }),
    )?;
  }

  let mut all_succeeded = true;
  let mut manual_recovery = false;
  if reason != SagaCorrectionReason::Success {
    let projection = engine.projection(run_id)?;
    let saga_state = projection
      .sagas
      .get(&saga.saga_id)
      .ok_or_else(|| RuntimeExecutionError::Stalled("saga correction state disappeared".into()))?;
    let mut registered = saga_state
      .actions
      .values()
      .filter(|action| action.action_kind == CorrectiveActionKind::Compensation)
      .filter_map(|action| {
        action
          .completion_sequence
          .map(|sequence| (sequence, action.action_id.clone()))
      })
      .collect::<Vec<_>>();
    registered.sort_by(|left, right| right.cmp(left));
    for (_, action_id) in registered {
      let action = saga
        .compensations
        .iter()
        .find(|candidate| corrective_action_matches_runtime(candidate, &action_id))
        .ok_or_else(|| {
          RuntimeExecutionError::Stalled("registered compensation disappeared".into())
        })?;
      match execute_corrective_action(
        engine,
        run_id,
        &saga.saga_id,
        action,
        &action_id,
        CorrectiveActionKind::Compensation,
        reason,
        CompensationStatusV1::NotRequired,
        options,
        host,
      )
      .await?
      {
        CorrectiveActionOutcome::Succeeded => {}
        CorrectiveActionOutcome::Failed => all_succeeded = false,
        CorrectiveActionOutcome::ManualRecovery => {
          all_succeeded = false;
          manual_recovery = true;
        }
      }
    }
  }

  let compensation_status = if reason == SagaCorrectionReason::Success {
    CompensationStatusV1::NotRequired
  } else if manual_recovery {
    CompensationStatusV1::ManualRecovery
  } else if all_succeeded {
    CompensationStatusV1::Completed
  } else {
    CompensationStatusV1::Incomplete
  };
  for action in &saga.cleanup {
    match execute_corrective_action(
      engine,
      run_id,
      &saga.saga_id,
      action,
      &action.action_id,
      CorrectiveActionKind::Cleanup,
      reason,
      compensation_status,
      options,
      host,
    )
    .await?
    {
      CorrectiveActionOutcome::Succeeded => {}
      CorrectiveActionOutcome::Failed => all_succeeded = false,
      CorrectiveActionOutcome::ManualRecovery => {
        all_succeeded = false;
        manual_recovery = true;
      }
    }
  }
  if manual_recovery {
    return Ok(false);
  }
  engine.append_payload(
    run_id,
    RunEventPayload::SagaCorrectionCompleted(SagaIdentityData {
      saga_id: saga.saga_id.clone(),
    }),
  )?;
  Ok(all_succeeded)
}
