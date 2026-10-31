//! Scoped open-iteration interruption settlement on the caller transaction. Recovery and workflow deadlines share this unchanged helper.

use crate::durable::append_to_history;
use crate::durable::append_to_history_scoped;
use crate::durable::generated_event_id;
use crate::durable::DurableStoreError;
use crate::durable::{
  interrupted_failure, interrupted_operation_failure, interrupted_operation_metadata,
  sha256_prefixed,
};
use crate::event::ForEachIterationFailedData;
use crate::event::ForEachIterationFailureKind;
use crate::event::ForEachIterationScope;
use crate::event::ForEachIterationSkippedData;
use crate::event::ForEachSettledData;
use crate::event::ForEachSkipReason;
use crate::event::OperationFailedData;
use crate::event::ParallelGroupCompletedData;
use crate::event::ParallelGroupOutcome;
use crate::event::StepAttemptFailedData;
use crate::fold_events;
use crate::projection::AttemptStatus;
use crate::projection::ForEachIterationStatus;
use crate::projection::ForEachStatus;
use crate::projection::ParallelGroupStatus;
use crate::AttemptFailure;
use crate::AttemptFailureKind;
use crate::CompiledWorkflowDefinition;
use crate::RunEvent;
use crate::RunEventPayload;
use crate::RunStatus;
use chrono::DateTime;
use chrono::Utc;
use rusqlite::Transaction;
use serde_json_canonicalizer::to_vec as canonical_json;

fn for_each_recovery_failure_kind(kind: AttemptFailureKind) -> ForEachIterationFailureKind {
  match kind {
    AttemptFailureKind::ScriptThrew => ForEachIterationFailureKind::ScriptThrew,
    AttemptFailureKind::ScriptTimedOut => ForEachIterationFailureKind::TimedOut,
    AttemptFailureKind::InvalidScriptResult => ForEachIterationFailureKind::NonJson,
    AttemptFailureKind::WorkerCrashed => ForEachIterationFailureKind::WorkerCrashed,
    AttemptFailureKind::HostCrashed => ForEachIterationFailureKind::HostCrashed,
    AttemptFailureKind::ResultTooLarge => ForEachIterationFailureKind::ResultTooLarge,
    AttemptFailureKind::ContextTooLarge => ForEachIterationFailureKind::ContextTooLarge,
    AttemptFailureKind::Interrupted => ForEachIterationFailureKind::Interrupted,
    AttemptFailureKind::ServiceFailed => ForEachIterationFailureKind::CapabilityFailed,
    AttemptFailureKind::InvocationCancelled => ForEachIterationFailureKind::Cancelled,
  }
}

pub(in crate::durable) fn recover_open_for_each_failure(
  transaction: &Transaction<'_>,
  workflow: &CompiledWorkflowDefinition,
  events: &mut Vec<RunEvent>,
  run_id: &str,
  event_schema_version: u32,
  now: DateTime<Utc>,
  forced_failure: Option<AttemptFailure>,
) -> Result<Option<(usize, AttemptFailure)>, DurableStoreError> {
  let projection = fold_events(events)?;
  if !matches!(
    event_schema_version,
    crate::RUN_EVENT_SCHEMA_VERSION_V15 | crate::RUN_EVENT_SCHEMA_VERSION_V16
  ) || projection.status == RunStatus::Cancelling
  {
    return Ok(None);
  }
  let force_settlement = forced_failure.is_some();
  let Some(descriptor) = workflow
    .graph
    .for_each
    .as_deref()
    .unwrap_or_default()
    .iter()
    .find(|descriptor| {
      projection
        .for_each
        .get(&descriptor.for_each_id)
        .is_some_and(|loop_state| {
          loop_state.status == ForEachStatus::Open
            && (force_settlement
              || loop_state
                .iterations
                .values()
                .any(|status| matches!(status, ForEachIterationStatus::Failed { .. }))
              || loop_state.executions.values().any(|execution| {
                execution
                  .attempts
                  .iter()
                  .any(|attempt| attempt.status == AttemptStatus::Started)
                  || execution.operations.values().any(|operation| {
                    operation.status == crate::projection::OperationStatus::Started
                  })
                  || execution.lifecycle_hooks.values().any(|hook| {
                    matches!(
                      hook.status,
                      crate::projection::LifecycleHookStatus::Requested
                        | crate::projection::LifecycleHookStatus::Running
                    )
                  })
                  || execution.reusable_lifecycle_hooks.values().any(|hook| {
                    matches!(
                      hook.status,
                      crate::projection::ReusableLifecycleStatus::Requested
                        | crate::projection::ReusableLifecycleStatus::Running
                    )
                  })
              }))
        })
    })
  else {
    return Ok(None);
  };

  let loop_state = projection
    .for_each
    .get(&descriptor.for_each_id)
    .expect("the selected loop exists");
  let mut interrupted_attempts = 0_usize;
  let mut primary_failure = forced_failure.clone().or_else(|| {
    loop_state.executions.values().find_map(|execution| {
      execution.attempts.iter().rev().find_map(|attempt| {
        if let AttemptStatus::Failed { failure } = &attempt.status {
          Some(failure.clone())
        } else {
          None
        }
      })
    })
  });

  for (index, execution) in &loop_state.executions {
    let scope = ForEachIterationScope {
      for_each_id: descriptor.for_each_id.clone(),
      index: *index,
    };
    for hook in execution.reusable_lifecycle_hooks.values().filter(|hook| {
      matches!(
        hook.status,
        crate::projection::ReusableLifecycleStatus::Requested
          | crate::projection::ReusableLifecycleStatus::Running
      )
    }) {
      if let Some(action_id) = &hook.active_action_id {
        append_to_history_scoped(
          transaction,
          events,
          run_id,
          generated_event_id(),
          now,
          event_schema_version,
          Some(scope.clone()),
          RunEventPayload::ReusableLifecycleActionFailed(
            crate::event::ReusableLifecycleActionFailedData {
              invocation_id: hook.invocation_id.clone(),
              definition_digest: hook.definition_digest.clone(),
              hook: hook.hook,
              action_id: action_id.clone(),
              outcome: crate::event::ReusableLifecycleOutcome::Failed,
              warning_code: "WOML_REUSABLE_LIFECYCLE_INTERRUPTED".to_string(),
            },
          ),
        )?;
      }
      append_to_history_scoped(
        transaction,
        events,
        run_id,
        generated_event_id(),
        now,
        event_schema_version,
        Some(scope.clone()),
        RunEventPayload::ReusableLifecycleCompleted(crate::event::ReusableLifecycleCompletedData {
          invocation_id: hook.invocation_id.clone(),
          definition_digest: hook.definition_digest.clone(),
          hook: hook.hook,
          outcome: crate::event::ReusableLifecycleOutcome::CompletedWithWarnings,
        }),
      )?;
      primary_failure.get_or_insert_with(interrupted_failure);
    }
    for hook in execution.lifecycle_hooks.values().filter(|hook| {
      matches!(
        hook.status,
        crate::projection::LifecycleHookStatus::Requested
          | crate::projection::LifecycleHookStatus::Running
      )
    }) {
      let compiled = workflow.lifecycle_hook(&hook.hook_id).ok_or_else(|| {
        DurableStoreError::Contract(format!(
          "Recovered iteration hook {:?} is absent from Model v16.",
          hook.hook_id
        ))
      })?;
      let failed_actions = compiled
        .actions
        .iter()
        .filter(|action| {
          !matches!(
            hook
              .actions
              .get(&action.action_id)
              .map(|action| action.status),
            Some(crate::projection::LifecycleActionStatus::Succeeded)
          )
        })
        .count() as u32;
      for action in &compiled.actions {
        match hook
          .actions
          .get(&action.action_id)
          .map(|action| action.status)
        {
          Some(crate::projection::LifecycleActionStatus::Succeeded)
          | Some(crate::projection::LifecycleActionStatus::Failed) => continue,
          None => {
            append_to_history_scoped(
              transaction,
              events,
              run_id,
              generated_event_id(),
              now,
              event_schema_version,
              Some(scope.clone()),
              RunEventPayload::LifecycleActionAttemptStarted(
                crate::event::LifecycleActionIdentityData {
                  hook_invocation_id: hook.hook_invocation_id.clone(),
                  action_id: action.action_id.clone(),
                  attempt: 1,
                },
              ),
            )?;
          }
          Some(crate::projection::LifecycleActionStatus::Started) => {}
        }
        append_to_history_scoped(
          transaction,
          events,
          run_id,
          generated_event_id(),
          now,
          event_schema_version,
          Some(scope.clone()),
          RunEventPayload::LifecycleActionFailed(crate::event::LifecycleActionFailedData {
            hook_invocation_id: hook.hook_invocation_id.clone(),
            action_id: action.action_id.clone(),
            attempt: 1,
            failure: crate::event::LifecycleFailure {
              kind: crate::event::LifecycleFailureKind::Interrupted,
              code: "WOML_LIFECYCLE_ACTION_INTERRUPTED".to_string(),
              message: "Recovery failed closed for an unfinished loop-owned lifecycle action."
                .to_string(),
            },
          }),
        )?;
      }
      append_to_history_scoped(
        transaction,
        events,
        run_id,
        generated_event_id(),
        now,
        event_schema_version,
        Some(scope.clone()),
        RunEventPayload::LifecycleHookCompleted(crate::event::LifecycleHookCompletedData {
          hook_invocation_id: hook.hook_invocation_id.clone(),
          status: crate::event::LifecycleHookCompletionStatus::CompletedWithWarnings,
          failed_actions,
        }),
      )?;
      primary_failure.get_or_insert_with(interrupted_failure);
    }
    for operation in execution
      .operations
      .values()
      .filter(|operation| operation.status == crate::projection::OperationStatus::Started)
    {
      append_to_history_scoped(
        transaction,
        events,
        run_id,
        generated_event_id(),
        now,
        event_schema_version,
        Some(scope.clone()),
        RunEventPayload::OperationFailed(OperationFailedData {
          node_id: operation.node_id.clone(),
          attempt_number: operation.attempt_number,
          invocation_id: operation.identity.invocation_id.clone(),
          call_id: operation.identity.call_id.clone(),
          operation_key: operation.operation_key.clone(),
          capability: operation.capability.clone(),
          operation: operation.operation.clone(),
          execution_mode: operation.execution_mode,
          metadata: interrupted_operation_metadata(operation),
          duration_ms: 0.0,
          failure: interrupted_operation_failure(operation, false, true),
        }),
      )?;
    }
    for attempt in execution
      .attempts
      .iter()
      .filter(|attempt| attempt.status == AttemptStatus::Started)
    {
      let failure = forced_failure.clone().unwrap_or_else(interrupted_failure);
      primary_failure.get_or_insert_with(|| failure.clone());
      interrupted_attempts += 1;
      append_to_history_scoped(
        transaction,
        events,
        run_id,
        generated_event_id(),
        now,
        event_schema_version,
        Some(scope.clone()),
        RunEventPayload::StepAttemptFailed(StepAttemptFailedData {
          node_id: attempt.identity.node_id.clone(),
          attempt: attempt.identity.attempt,
          invocation_id: attempt.identity.invocation_id.clone(),
          failure,
        }),
      )?;
    }
    for group in execution
      .parallel_groups
      .values()
      .filter(|group| group.status == ParallelGroupStatus::Started)
    {
      let start_id = format!("__woml_parallel__{}__start", group.parallel_id);
      let cancelled_node_ids = descriptor
        .body
        .edges
        .iter()
        .filter(|edge| {
          edge.from == start_id && edge.parallel_id.as_deref() == Some(group.parallel_id.as_str())
        })
        .map(|edge| edge.to.clone())
        .collect::<Vec<_>>();
      append_to_history_scoped(
        transaction,
        events,
        run_id,
        generated_event_id(),
        now,
        event_schema_version,
        Some(scope.clone()),
        RunEventPayload::ParallelGroupCompleted(ParallelGroupCompletedData {
          parallel_id: group.parallel_id.clone(),
          outcome: ParallelGroupOutcome::Failed,
          failed_node_ids: Vec::new(),
          cancelled_node_ids,
        }),
      )?;
    }
  }

  let recovered = fold_events(events)?;
  let loop_state = recovered
    .for_each
    .get(&descriptor.for_each_id)
    .expect("the recovering loop exists");
  let active_indexes = loop_state
    .iterations
    .iter()
    .filter_map(|(index, status)| {
      matches!(status, ForEachIterationStatus::Started).then_some(*index)
    })
    .collect::<Vec<_>>();
  for index in active_indexes {
    let projection = fold_events(events)?;
    let failure = projection
      .for_each
      .get(&descriptor.for_each_id)
      .and_then(|loop_state| loop_state.executions.get(&index))
      .and_then(|execution| {
        execution.attempts.iter().rev().find_map(|attempt| {
          if let AttemptStatus::Failed { failure } = &attempt.status {
            Some((attempt.identity.node_id.clone(), failure.clone()))
          } else {
            None
          }
        })
      });
    let (failed_node_id, failure_kind) = failure
      .map(|(node_id, failure)| (node_id, for_each_recovery_failure_kind(failure.kind)))
      .unwrap_or_else(|| {
        (
          descriptor.body.terminal_node_id.clone(),
          forced_failure
            .as_ref()
            .map_or(ForEachIterationFailureKind::Interrupted, |failure| {
              for_each_recovery_failure_kind(failure.kind)
            }),
        )
      });
    append_to_history_scoped(
      transaction,
      events,
      run_id,
      generated_event_id(),
      now,
      event_schema_version,
      Some(ForEachIterationScope {
        for_each_id: descriptor.for_each_id.clone(),
        index,
      }),
      RunEventPayload::ForEachIterationFailed(ForEachIterationFailedData {
        for_each_id: descriptor.for_each_id.clone(),
        failed_node_id,
        failure_kind,
      }),
    )?;
  }

  let recovered = fold_events(events)?;
  let loop_state = recovered
    .for_each
    .get(&descriptor.for_each_id)
    .expect("the recovering loop exists");
  let pending_indexes = (0..loop_state.total)
    .filter(|index| !loop_state.iterations.contains_key(index))
    .collect::<Vec<_>>();
  for index in pending_indexes {
    append_to_history_scoped(
      transaction,
      events,
      run_id,
      generated_event_id(),
      now,
      event_schema_version,
      Some(ForEachIterationScope {
        for_each_id: descriptor.for_each_id.clone(),
        index,
      }),
      RunEventPayload::ForEachIterationSkipped(ForEachIterationSkippedData {
        for_each_id: descriptor.for_each_id.clone(),
        reason: ForEachSkipReason::Failure,
      }),
    )?;
  }

  let recovered = fold_events(events)?;
  let loop_state = recovered
    .for_each
    .get(&descriptor.for_each_id)
    .expect("the recovering loop exists");
  let succeeded = loop_state
    .iterations
    .values()
    .filter(|status| matches!(status, ForEachIterationStatus::Succeeded { .. }))
    .count() as u32;
  let failed = loop_state
    .iterations
    .values()
    .filter(|status| matches!(status, ForEachIterationStatus::Failed { .. }))
    .count() as u32;
  let skipped = loop_state
    .iterations
    .values()
    .filter(|status| matches!(status, ForEachIterationStatus::Skipped))
    .count() as u32;
  let aggregate = serde_json::json!({
    "total": loop_state.total,
    "succeeded": succeeded,
    "failed": failed,
    "skipped": skipped,
  });
  append_to_history(
    transaction,
    events,
    run_id,
    generated_event_id(),
    now,
    event_schema_version,
    RunEventPayload::ForEachFailed(ForEachSettledData {
      for_each_id: descriptor.for_each_id.clone(),
      total: loop_state.total,
      succeeded,
      failed,
      skipped,
      aggregate_digest: sha256_prefixed(&canonical_json(&aggregate)?),
    }),
  )?;
  Ok(Some((
    interrupted_attempts,
    primary_failure.unwrap_or_else(interrupted_failure),
  )))
}
