//! Total workflow timeout settlement across active operations, attempts, containers, and lifecycle. Existing writer ownership and ambiguity rules remain intact.

use crate::durable::append_to_history;
use crate::durable::definition_for_run;
use crate::durable::ensure_run_exists;
use crate::durable::generated_event_id;
use crate::durable::load_events;
use crate::durable::run_binding_in_transaction;
use crate::durable::DurableStoreError;
use crate::durable::RunTimeoutSettlement;
use crate::durable::{
  derive_lifecycle_hook_invocation_id, recover_open_for_each_failure, DurableEventStore,
};
use crate::engine::validate_event_history_against_definition;
use crate::event::LifecycleHookRequestedData;
use crate::event::LifecycleSubject;
use crate::event::LifecycleSubjectKind;
use crate::event::OperationFailedData;
use crate::event::StepAttemptFailedData;
use crate::fold_events;
use crate::projection::AttemptStatus;
use crate::AttemptFailure;
use crate::AttemptFailureKind;
use crate::RunEventPayload;
use crate::RunStatus;
use chrono::DateTime;
use chrono::Utc;
use rusqlite::TransactionBehavior;

impl DurableEventStore {
  /// Atomically lets the immutable workflow deadline race the normal outcome
  /// and operator cancellation authorities. A committed outcome or
  /// cancellation request always wins and can never be rewritten by timeout.
  pub fn settle_run_timeout(
    &mut self,
    run_id: &str,
    now: DateTime<Utc>,
  ) -> Result<RunTimeoutSettlement, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    if workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V12 {
      return Err(DurableStoreError::Contract(
        "Workflow timeout authority requires compiled Model v12.".to_string(),
      ));
    }
    let mut events = load_events(&transaction, run_id)?;
    let projection = fold_events(&events)?;
    let event_schema_version = projection.event_schema_version.ok_or_else(|| {
      DurableStoreError::Contract("Workflow timeout run has no event version.".to_string())
    })?;
    let Some(deadline_at) = projection.timeout_at else {
      return Ok(RunTimeoutSettlement::NotConfigured);
    };
    if projection.business_outcome.is_some()
      || projection.cancellation_request_id.is_some()
      || projection.timeout_reached_at.is_some()
    {
      return Ok(RunTimeoutSettlement::LostRace { projection });
    }
    if now < deadline_at {
      return Ok(RunTimeoutSettlement::NotDue { deadline_at });
    }
    if !matches!(projection.status, RunStatus::Running | RunStatus::Waiting) {
      return Ok(RunTimeoutSettlement::LostRace { projection });
    }
    append_to_history(
      &transaction,
      &mut events,
      run_id,
      generated_event_id(),
      deadline_at,
      event_schema_version,
      RunEventPayload::RunTimeoutReached(crate::event::RunTimeoutReachedData {
        deadline_at,
        code: "WOML_WORKFLOW_TIMED_OUT".to_string(),
      }),
    )?;
    for operation in projection
      .operations
      .values()
      .filter(|operation| operation.status == crate::projection::OperationStatus::Started)
    {
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        now,
        event_schema_version,
        RunEventPayload::OperationFailed(OperationFailedData {
          node_id: operation.node_id.clone(),
          attempt_number: operation.attempt_number,
          invocation_id: operation.identity.invocation_id.clone(),
          call_id: operation.identity.call_id.clone(),
          operation_key: operation.operation_key.clone(),
          capability: operation.capability.clone(),
          operation: operation.operation.clone(),
          execution_mode: operation.execution_mode,
          metadata: operation.metadata.clone(),
          duration_ms: 0.0,
          failure: crate::CapabilityFailure {
            kind: crate::CapabilityFailureKind::Ambiguous,
            code: "WOML_CAPABILITY_TIMEOUT_AMBIGUOUS".to_string(),
            message: "The workflow deadline interrupted a managed operation before its terminal outcome became durable; it will not be replayed.".to_string(),
            retryable: false,
            ambiguous: true,
            details: None,
          },
        }),
      )?;
    }
    for attempt in projection
      .attempts
      .iter()
      .filter(|attempt| attempt.status == AttemptStatus::Started)
    {
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        now,
        event_schema_version,
        RunEventPayload::StepAttemptFailed(StepAttemptFailedData {
          node_id: attempt.identity.node_id.clone(),
          attempt: attempt.identity.attempt,
          invocation_id: attempt.identity.invocation_id.clone(),
          failure: AttemptFailure {
            kind: AttemptFailureKind::Interrupted,
            code: AttemptFailureKind::Interrupted.code().to_string(),
            message: "The workflow deadline interrupted this attempt before its terminal outcome became durable.".to_string(),
            details: None,
            ..AttemptFailure::legacy_defaults()
          },
        }),
      )?;
    }
    // Model v13 owns every opened fork branch until it reaches a durable
    // terminal boundary. A workflow timeout closes those boundaries before
    // deciding the run outcome so recovery cannot leave orphaned branch work.
    if workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V13 {
      for fork in workflow.graph.forks.as_deref().unwrap_or_default() {
        let Some(projected) = projection.forks.get(&fork.fork_id) else {
          continue;
        };
        for branch in &fork.branches {
          if projected.branches.contains_key(&branch.branch_id) {
            continue;
          }
          append_to_history(
            &transaction,
            &mut events,
            run_id,
            generated_event_id(),
            now,
            event_schema_version,
            RunEventPayload::ForkBranchSettled(crate::event::ForkBranchSettledData {
              fork_id: fork.fork_id.clone(),
              branch_id: branch.branch_id.clone(),
              terminal_node_id: branch.terminal_node_id.clone(),
              outcome: crate::event::ForkBranchOutcome::Cancelled,
            }),
          )?;
        }
        if projected.join_status == crate::projection::ForkJoinStatus::Pending {
          let (outcome, blocking_branch_id) =
            if let Some(branch_id) = fork.joined_branch_ids.first() {
              (
                crate::event::ForkJoinOutcome::Cancelled,
                Some(branch_id.clone()),
              )
            } else {
              (crate::event::ForkJoinOutcome::Succeeded, None)
            };
          append_to_history(
            &transaction,
            &mut events,
            run_id,
            generated_event_id(),
            now,
            event_schema_version,
            RunEventPayload::ForkJoinSettled(crate::event::ForkJoinSettledData {
              fork_id: fork.fork_id.clone(),
              outcome,
              blocking_branch_id,
            }),
          )?;
        }
      }
    }
    recover_open_for_each_failure(
      &transaction,
      &workflow,
      &mut events,
      run_id,
      event_schema_version,
      now,
      Some(AttemptFailure {
        kind: AttemptFailureKind::ScriptTimedOut,
        code: AttemptFailureKind::ScriptTimedOut.code().to_string(),
        message: "The workflow deadline interrupted this loop attempt before its terminal outcome became durable.".to_string(),
        details: None,
        ..AttemptFailure::legacy_defaults()
      }),
    )?;
    append_to_history(
      &transaction,
      &mut events,
      run_id,
      generated_event_id(),
      now,
      event_schema_version,
      RunEventPayload::RunOutcomeDecided(crate::event::RunOutcomeDecidedData::Failed {
        failure: crate::event::LifecycleFailure {
          kind: crate::event::LifecycleFailureKind::TimedOut,
          code: "WOML_WORKFLOW_TIMED_OUT".to_string(),
          message: "The workflow exceeded its configured total run timeout.".to_string(),
        },
      }),
    )?;
    if let Some(hook) = workflow
      .lifecycle_hook_for_event(crate::model::LifecycleEventName::RunFailure)
      .or_else(|| workflow.lifecycle_hook_for_event(crate::model::LifecycleEventName::RunComplete))
    {
      let subject = LifecycleSubject {
        kind: LifecycleSubjectKind::Workflow,
        id: run_id.to_string(),
      };
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        now,
        event_schema_version,
        RunEventPayload::LifecycleHookRequested(LifecycleHookRequestedData {
          hook_invocation_id: derive_lifecycle_hook_invocation_id(
            run_id,
            &hook.hook_id,
            subject.kind,
            &subject.id,
          ),
          hook_id: hook.hook_id.clone(),
          event: hook.event,
          subject,
        }),
      )?;
    }
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = fold_events(&events)?;
    transaction.commit()?;
    Ok(RunTimeoutSettlement::TimedOut { projection })
  }
}
