//! Cancellation settlement after owned work has been interrupted or classified.
//!
//! Close operations, attempts, parallel/fork/iteration scopes, and waiters in
//! their existing order. Renewal cancellation is not proof about external
//! effects; saga correction remains an obligation of the continuation loop.

use crate::event::ForEachIterationScope;
use crate::event::OperationFailedData;
use crate::event::ParallelGroupCompletedData;
use crate::event::ParallelGroupOutcome;
use crate::event::StepAttemptFailedData;
use crate::projection::AttemptStatus;
use crate::projection::ForEachStatus;
use crate::projection::ParallelGroupStatus;
use crate::runtime::errors::CancelledRunDetails;
use crate::runtime::errors::RuntimeExecutionError;
use crate::runtime::options::RuntimeExecutionOptions;
use crate::runtime::{
  settle_open_for_each, settle_open_fork_cancellation, ForEachTerminalDisposition, RuntimeDagEngine,
};
use crate::AttemptFailure;
use crate::AttemptFailureKind;
use crate::CapabilityFailure;
use crate::CapabilityFailureKind;
use crate::OperationStatus;
use crate::RichApprovalResolutionReason;
use crate::RunEventPayload;
use crate::RunStatus;
use crate::StepFailureDisposition;

pub(in crate::runtime) fn cancelled_run_error<E: RuntimeDagEngine>(
  engine: &E,
  run_id: &str,
) -> Result<RuntimeExecutionError, RuntimeExecutionError> {
  let projection = engine.projection(run_id)?;
  let cancellation_request_id = projection.cancellation_request_id.ok_or_else(|| {
    RuntimeExecutionError::Stalled("cancelled run has no durable cancellation request".to_string())
  })?;
  Ok(RuntimeExecutionError::RunCancelled(Box::new(
    CancelledRunDetails {
      code: "WOML_RUN_CANCELLED".to_string(),
      run_id: run_id.to_string(),
      cancellation_request_id,
      events: engine.events(run_id)?,
    },
  )))
}

pub(in crate::runtime) fn settle_cancelling_run<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  occurred_at: chrono::DateTime<chrono::Utc>,
  options: &RuntimeExecutionOptions,
) -> Result<(), RuntimeExecutionError> {
  let projection = engine.projection(run_id)?;
  if projection.status != RunStatus::Cancelling {
    return Ok(());
  }

  engine.settle_pending_rich_approval(
    run_id,
    RichApprovalResolutionReason::Cancellation,
    occurred_at,
  )?;

  let active_operations = projection
    .operations
    .values()
    .filter(|operation| operation.status == OperationStatus::Started)
    .cloned()
    .collect::<Vec<_>>();
  for operation in active_operations {
    engine.append_payload(
      run_id,
      RunEventPayload::OperationFailed(OperationFailedData {
        node_id: operation.node_id,
        attempt_number: operation.attempt_number,
        invocation_id: operation.identity.invocation_id,
        call_id: operation.identity.call_id,
        operation_key: operation.operation_key,
        capability: operation.capability,
        operation: operation.operation,
        execution_mode: operation.execution_mode,
        metadata: operation.metadata,
        duration_ms: 0.0,
        failure: CapabilityFailure {
          kind: CapabilityFailureKind::Ambiguous,
          code: "WOML_CAPABILITY_CANCELLATION_AMBIGUOUS".to_string(),
          message: "Cancellation found an operation without a durable terminal event; its external outcome is ambiguous and it will not be replayed.".to_string(),
          retryable: false,
          ambiguous: true,
          details: None,
        },
      }),
    )?;
  }

  let projection = engine.projection(run_id)?;
  let active_attempts = projection
    .attempts
    .iter()
    .filter(|attempt| attempt.status == AttemptStatus::Started)
    .map(|attempt| attempt.identity.clone())
    .collect::<Vec<_>>();
  for attempt in active_attempts {
    let disposition = engine.record_step_attempt_failure(
      run_id,
      occurred_at,
      StepAttemptFailedData {
        node_id: attempt.node_id,
        attempt: attempt.attempt,
        invocation_id: attempt.invocation_id,
        failure: AttemptFailure {
          kind: AttemptFailureKind::InvocationCancelled,
          code: AttemptFailureKind::InvocationCancelled.code().to_string(),
          message: "The step invocation was cancelled with its workflow run.".to_string(),
          details: None,
          ..AttemptFailure::legacy_defaults()
        },
      },
    )?;
    if disposition != StepFailureDisposition::StepFailed {
      return Err(RuntimeExecutionError::Stalled(
        "cancellation attempted to retry or fail the workflow".to_string(),
      ));
    }
  }

  let projection = engine.projection(run_id)?;
  let started_parallel_ids = projection
    .parallel_groups
    .values()
    .filter(|group| group.status == ParallelGroupStatus::Started)
    .map(|group| group.parallel_id.clone())
    .collect::<Vec<_>>();
  for parallel_id in started_parallel_ids {
    let group = engine
      .workflow()
      .parallel_group(&parallel_id)
      .ok_or_else(|| {
        RuntimeExecutionError::Stalled(
          "cancelling run references an unknown parallel group".to_string(),
        )
      })?;
    let projection = engine.projection(run_id)?;
    let mut failed_node_ids = Vec::new();
    let mut cancelled_node_ids = Vec::new();
    for node_id in &group.child_node_ids {
      let Some(attempt) = projection.latest_attempt(node_id) else {
        continue;
      };
      let AttemptStatus::Failed { failure } = &attempt.status else {
        continue;
      };
      if failure.kind == AttemptFailureKind::InvocationCancelled {
        cancelled_node_ids.push(node_id.clone());
      } else {
        failed_node_ids.push(node_id.clone());
      }
    }
    if failed_node_ids.is_empty() && cancelled_node_ids.is_empty() {
      continue;
    }
    engine.append_payload(
      run_id,
      RunEventPayload::ParallelGroupCompleted(ParallelGroupCompletedData {
        parallel_id,
        outcome: ParallelGroupOutcome::Failed,
        failed_node_ids,
        cancelled_node_ids,
      }),
    )?;
  }

  // A run may be cancelled while a fork is waiting on an approval, a retry,
  // or queued branch work. Close every opened ownership boundary before the
  // terminal cancellation event so replay and retention never see an orphaned
  // branch.
  let opened_forks = engine
    .workflow()
    .graph
    .forks
    .as_deref()
    .unwrap_or_default()
    .iter()
    .filter(|fork| {
      engine
        .projection(run_id)
        .ok()
        .is_some_and(|projection| projection.forks.contains_key(&fork.fork_id))
    })
    .cloned()
    .collect::<Vec<_>>();
  for fork in opened_forks {
    settle_open_fork_cancellation(engine, run_id, &fork)?;
  }

  let opened_loops = engine
    .workflow()
    .graph
    .for_each
    .as_deref()
    .unwrap_or_default()
    .iter()
    .filter(|descriptor| {
      engine.projection(run_id).ok().is_some_and(|projection| {
        projection
          .for_each
          .get(&descriptor.for_each_id)
          .is_some_and(|state| state.status == ForEachStatus::Open)
      })
    })
    .cloned()
    .collect::<Vec<_>>();
  for descriptor in opened_loops {
    settle_open_for_each(
      engine,
      run_id,
      &descriptor,
      ForEachTerminalDisposition::Cancelled,
    )?;
  }

  let projection = engine.projection(run_id)?;
  let cancellation_request_id = projection.cancellation_request_id.ok_or_else(|| {
    RuntimeExecutionError::Stalled("cancelling run has no durable request identity".to_string())
  })?;
  engine.cancel_lock_waiters(run_id, &cancellation_request_id, occurred_at)?;
  for lock in engine
    .projection(run_id)?
    .locks
    .values()
    .filter(|lock| lock.status == crate::LockStatus::Held)
  {
    options.stop_lock_renewal(run_id, &lock.lock_id);
  }
  for (for_each_id, loop_state) in &engine.projection(run_id)?.for_each {
    for (index, execution) in &loop_state.executions {
      let scope = ForEachIterationScope {
        for_each_id: for_each_id.clone(),
        index: *index,
      };
      for lock in execution
        .locks
        .values()
        .filter(|lock| lock.status == crate::LockStatus::Held)
      {
        options.stop_iteration_lock_renewal(run_id, &lock.lock_id, &scope);
      }
    }
  }
  engine.decide_run_cancelled(run_id, cancellation_request_id, occurred_at)
}
