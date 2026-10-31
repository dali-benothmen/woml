//! Ordered per-run fail-closed recovery on one immediate transaction. Preserve version branches and early returns: correction, loops, lifecycle, notification sends/updates, operations, attempts, joins, retries, then justified settlement.

mod for_each;
mod interruption;
mod queries;
mod state_mutation;

pub(in crate::durable) use for_each::recover_open_for_each_failure;
pub(in crate::durable) use interruption::{
  attempt_run_failed_data, interrupted_failure, interrupted_operation_failure,
  interrupted_operation_metadata, parallel_failure_message,
};
pub(in crate::durable) use state_mutation::recovered_state_mutation_settlement;

use crate::durable::append_to_history;
use crate::durable::definition_for_run;
use crate::durable::ensure_run_exists;
use crate::durable::expand_model_v11_payload;
use crate::durable::generated_event_id;
use crate::durable::lifecycle_failure_from_attempt;
use crate::durable::load_events;
use crate::durable::run_binding_in_transaction;
use crate::durable::write_correction_indexes_from_projection;
use crate::durable::DurableStoreError;
use crate::durable::RecoveryReport;
use crate::durable::CORRECTION_RECOVERY_BATCH_LIMIT;
use crate::durable::{
  derive_lifecycle_hook_invocation_id, notification_request_all_failed, DurableEventStore,
};
use crate::engine::validate_event_history_against_definition;
use crate::event::LifecycleHookRequestedData;
use crate::event::LifecycleSubject;
use crate::event::LifecycleSubjectKind;
use crate::event::NotificationDeliveryFailedData;
use crate::event::NotificationMessageUpdateFailedData;
use crate::event::NotificationRunFailure;
use crate::event::NotificationSafeFailure;
use crate::event::OperationFailedData;
use crate::event::OperationSucceededData;
use crate::event::ParallelFailure;
use crate::event::ParallelFailurePolicy;
use crate::event::ParallelGroupCompletedData;
use crate::event::ParallelGroupOutcome;
use crate::event::RunFailedData;
use crate::event::RunFailedDataV3;
use crate::event::RunFailedDataV5;
use crate::event::RunSucceededData;
use crate::event::StepAttemptFailedData;
use crate::fold_events;
use crate::projection::AttemptStatus;
use crate::projection::NotificationDeliveryStatus;
use crate::projection::NotificationMessageUpdateStatus;
use crate::projection::ParallelGroupStatus;
use crate::workflow_calls::WorkflowTarget;
use crate::AttemptFailureKind;
use crate::RunEventPayload;
use crate::RunStatus;
use chrono::Utc;
use rusqlite::params;
use rusqlite::TransactionBehavior;

pub(in crate::durable) enum RunRecovery {
  Unchanged,
  Resumable,
  Recovered { interrupted_attempts: usize },
}

impl DurableEventStore {
  pub fn recover_interrupted_runs(&mut self) -> Result<RecoveryReport, DurableStoreError> {
    let run_ids = self.run_ids()?;
    let mut report = RecoveryReport {
      inspected_runs: run_ids.len(),
      ..RecoveryReport::default()
    };
    for run_id in run_ids {
      match self.recover_run(&run_id)? {
        RunRecovery::Recovered {
          interrupted_attempts,
        } => {
          report.recovered_runs += 1;
          report.interrupted_attempts += interrupted_attempts;
        }
        RunRecovery::Resumable => {
          report.resumable_runs += 1;
        }
        RunRecovery::Unchanged => {}
      }
    }
    self.reconcile_workflow_call_states()?;
    Ok(report)
  }

  /// Recovers only workflow-call children owned by the supplied local target
  /// definitions. This avoids treating runs executing in other live WOML
  /// processes as interrupted during cross-process startup.
  pub fn recover_workflow_call_children_for_targets(
    &mut self,
    targets: &[WorkflowTarget],
  ) -> Result<RecoveryReport, DurableStoreError> {
    let mut run_ids = Vec::new();
    for target in targets {
      let mut statement = self.connection.prepare(
        "SELECT child_run_id FROM woml_workflow_calls
         WHERE target_workflow_id = ?1 AND target_definition_hash = ?2
           AND state IN ('admitted', 'running')
         ORDER BY admitted_at, call_key",
      )?;
      run_ids.extend(
        statement
          .query_map([&target.workflow_id, &target.definition_hash], |row| {
            row.get::<_, String>(0)
          })?
          .collect::<Result<Vec<_>, _>>()?,
      );
    }
    run_ids.sort();
    run_ids.dedup();
    let mut report = RecoveryReport {
      inspected_runs: run_ids.len(),
      ..RecoveryReport::default()
    };
    for run_id in run_ids {
      match self.recover_run(&run_id)? {
        RunRecovery::Recovered {
          interrupted_attempts,
        } => {
          report.recovered_runs += 1;
          report.interrupted_attempts += interrupted_attempts;
        }
        RunRecovery::Resumable => report.resumable_runs += 1,
        RunRecovery::Unchanged => {}
      }
    }
    self.reconcile_workflow_call_states()?;
    Ok(report)
  }

  /// Repairs the reconstructible call index from authoritative child history
  /// after run recovery. No result value is copied into the index.
  pub fn reconcile_workflow_call_states(&mut self) -> Result<(), DurableStoreError> {
    let pending = {
      let mut statement = self.connection.prepare(
        "SELECT call_key, child_run_id, state FROM woml_workflow_calls
         WHERE state IN ('admitted', 'running') ORDER BY admitted_at, call_key",
      )?;
      let rows = statement
        .query_map([], |row| {
          Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
          ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
      rows
    };
    for (call_key, child_run_id, stored_state) in pending {
      let projection = self.projection(&child_run_id)?;
      let next = match projection.status {
        RunStatus::Succeeded => Some("succeeded"),
        RunStatus::Failed | RunStatus::Cancelled | RunStatus::Waiting => Some("failed"),
        RunStatus::Running if stored_state == "running" => Some("admitted"),
        RunStatus::NotStarted
        | RunStatus::Queued
        | RunStatus::Running
        | RunStatus::Cancelling
        | RunStatus::Finalizing => None,
      };
      if let Some(next) = next {
        self.connection.execute(
          "UPDATE woml_workflow_calls SET state = ?1 WHERE call_key = ?2",
          params![next, call_key],
        )?;
      }
    }
    Ok(())
  }

  pub(in crate::durable) fn run_ids(&self) -> Result<Vec<String>, DurableStoreError> {
    let mut statement = self
      .connection
      .prepare("SELECT run_id FROM woml_runs ORDER BY created_at, run_id")?;
    let rows = statement.query_map([], |row| row.get(0))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
  }

  pub(in crate::durable) fn recover_run(
    &mut self,
    run_id: &str,
  ) -> Result<RunRecovery, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let mut events = load_events(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = fold_events(&events)?;
    let event_schema_version = projection.event_schema_version.ok_or_else(|| {
      DurableStoreError::Contract("A stored run has no event schema version.".to_string())
    })?;
    if matches!(
      event_schema_version,
      crate::RUN_EVENT_SCHEMA_VERSION_V18
        | crate::RUN_EVENT_SCHEMA_VERSION_V19
        | crate::RUN_EVENT_SCHEMA_VERSION_V20
    ) {
      let active_corrections = projection
        .sagas
        .values()
        .flat_map(|saga| {
          saga.actions.values().filter_map(|action| {
            (action.status == crate::CorrectionActionStatus::Started).then_some((
              saga.saga_id.clone(),
              action.action_id.clone(),
              action.action_kind,
              action.active_attempt.unwrap_or(action.attempts),
            ))
          })
        })
        .collect::<Vec<_>>();
      if !active_corrections.is_empty() {
        if active_corrections.len() > CORRECTION_RECOVERY_BATCH_LIMIT {
          return Err(DurableStoreError::Contract(
            "WOML_CORRECTION_LIMIT_EXCEEDED: interrupted correction batch exceeds 100 actions."
              .to_string(),
          ));
        }
        let now = Utc::now();
        for (saga_id, action_id, action_kind, attempt) in &active_corrections {
          append_to_history(
            &transaction,
            &mut events,
            run_id,
            generated_event_id(),
            now,
            event_schema_version,
            RunEventPayload::CorrectionInterrupted(crate::CorrectionInterruptedData {
              saga_id: saga_id.clone(),
              action_id: action_id.clone(),
              action_kind: *action_kind,
              attempt: *attempt,
              ambiguous: true,
            }),
          )?;
          append_to_history(
            &transaction,
            &mut events,
            run_id,
            generated_event_id(),
            now,
            event_schema_version,
            RunEventPayload::ManualRecoveryRequired(crate::ManualRecoveryRequiredData {
              saga_id: saga_id.clone(),
              action_id: action_id.clone(),
              action_kind: *action_kind,
              code: "WOML_CORRECTION_INTERRUPTED".to_string(),
            }),
          )?;
        }
        validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
          .map_err(DurableStoreError::Contract)?;
        let recovered = fold_events(&events)?;
        write_correction_indexes_from_projection(
          &transaction,
          run_id,
          &binding.definition_hash,
          &events,
          &recovered,
        )?;
        transaction.commit()?;
        return Ok(RunRecovery::Recovered {
          interrupted_attempts: active_corrections.len(),
        });
      }
      let correction_can_resume = projection.sagas.values().any(|saga| {
        matches!(
          saga.phase,
          crate::SagaPhase::Correcting | crate::SagaPhase::CorrectionCompleted
        )
      });
      if correction_can_resume
        && matches!(
          projection.status,
          RunStatus::Running | RunStatus::Cancelling | RunStatus::Finalizing
        )
      {
        return Ok(RunRecovery::Resumable);
      }
    }
    if let Some((interrupted_attempts, failure)) = recover_open_for_each_failure(
      &transaction,
      &workflow,
      &mut events,
      run_id,
      event_schema_version,
      Utc::now(),
      None,
    )? {
      let now = Utc::now();
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        now,
        event_schema_version,
        RunEventPayload::RunOutcomeDecided(crate::event::RunOutcomeDecidedData::Failed {
          failure: lifecycle_failure_from_attempt(&failure),
        }),
      )?;
      if let Some(hook) = workflow
        .lifecycle_hook_for_event(crate::model::LifecycleEventName::RunFailure)
        .or_else(|| {
          workflow.lifecycle_hook_for_event(crate::model::LifecycleEventName::RunComplete)
        })
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
      } else {
        append_to_history(
          &transaction,
          &mut events,
          run_id,
          generated_event_id(),
          now,
          event_schema_version,
          RunEventPayload::RunFinalized(crate::event::RunFinalizedData {
            outcome: crate::event::BusinessOutcome::Failed,
            lifecycle_status: crate::event::FinalLifecycleStatus::Completed,
            warnings: Vec::new(),
          }),
        )?;
      }
      validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
        .map_err(DurableStoreError::Contract)?;
      transaction.commit()?;
      return Ok(RunRecovery::Recovered {
        interrupted_attempts,
      });
    }
    if matches!(
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
      let ambiguous_actions = projection
        .lifecycle_hooks
        .values()
        .flat_map(|hook| {
          hook.actions.values().filter_map(|action| {
            (action.status == crate::projection::LifecycleActionStatus::Started).then_some((
              hook.hook_invocation_id.clone(),
              action.action_id.clone(),
              action.attempt,
            ))
          })
        })
        .collect::<Vec<_>>();
      if !ambiguous_actions.is_empty() {
        let now = Utc::now();
        for (hook_invocation_id, action_id, attempt) in &ambiguous_actions {
          append_to_history(
            &transaction,
            &mut events,
            run_id,
            generated_event_id(),
            now,
            event_schema_version,
            RunEventPayload::LifecycleActionFailed(crate::event::LifecycleActionFailedData {
              hook_invocation_id: hook_invocation_id.clone(),
              action_id: action_id.clone(),
              attempt: *attempt,
              failure: crate::event::LifecycleFailure {
                kind: crate::event::LifecycleFailureKind::Interrupted,
                code: "WOML_LIFECYCLE_ACTION_INTERRUPTED".to_string(),
                message: "Recovery found a started lifecycle action without a terminal event; its outcome is ambiguous and it will not be replayed.".to_string(),
              },
            }),
          )?;
        }
        let recovered = fold_events(&events)?;
        for hook_invocation_id in ambiguous_actions
          .iter()
          .map(|(hook_id, _, _)| hook_id)
          .collect::<std::collections::HashSet<_>>()
        {
          let hook_projection = recovered.lifecycle_hooks.get(hook_invocation_id).unwrap();
          let compiled = workflow
            .lifecycle_hook(&hook_projection.hook_id)
            .ok_or_else(|| {
              DurableStoreError::Contract("Recovered hook is absent from Model v11.".to_string())
            })?;
          if hook_projection.actions.len() == compiled.actions.len() {
            let completed_event = compiled.event;
            let completed_subject = hook_projection.subject.clone();
            append_to_history(
              &transaction,
              &mut events,
              run_id,
              generated_event_id(),
              now,
              event_schema_version,
              RunEventPayload::LifecycleHookCompleted(crate::event::LifecycleHookCompletedData {
                hook_invocation_id: hook_invocation_id.clone(),
                status: crate::event::LifecycleHookCompletionStatus::CompletedWithWarnings,
                failed_actions: hook_projection.failed_actions,
              }),
            )?;
            let next_hook = match completed_event {
              crate::model::LifecycleEventName::StepSuccess
              | crate::model::LifecycleEventName::StepFailure => workflow
                .lifecycle_hook_for_step_event(
                  crate::model::LifecycleEventName::StepComplete,
                  &completed_subject.id,
                ),
              crate::model::LifecycleEventName::RunSuccess
              | crate::model::LifecycleEventName::RunFailure
              | crate::model::LifecycleEventName::RunCancel => {
                workflow.lifecycle_hook_for_event(crate::model::LifecycleEventName::RunComplete)
              }
              _ => None,
            };
            if let Some(next_hook) = next_hook {
              let subject = if next_hook.event.is_step() {
                completed_subject
              } else {
                LifecycleSubject {
                  kind: LifecycleSubjectKind::Workflow,
                  id: run_id.to_string(),
                }
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
                    &next_hook.hook_id,
                    subject.kind,
                    &subject.id,
                  ),
                  hook_id: next_hook.hook_id.clone(),
                  event: next_hook.event,
                  subject,
                }),
              )?;
            }
          }
        }
        validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
          .map_err(DurableStoreError::Contract)?;
        transaction.commit()?;
        return Ok(RunRecovery::Recovered {
          interrupted_attempts: 0,
        });
      }
      let has_pending_hook = projection.lifecycle_hooks.values().any(|hook| {
        matches!(
          hook.status,
          crate::projection::LifecycleHookStatus::Requested
            | crate::projection::LifecycleHookStatus::Running
        )
      });
      let has_active_business_attempt = projection
        .attempts
        .iter()
        .any(|attempt| attempt.status == AttemptStatus::Started);
      let has_active_operation = projection
        .operations
        .values()
        .any(|operation| operation.status == crate::projection::OperationStatus::Started);
      if (has_pending_hook && !has_active_business_attempt && !has_active_operation)
        || matches!(
          projection.status,
          RunStatus::Cancelling | RunStatus::Finalizing
        )
      {
        return Ok(RunRecovery::Resumable);
      }
    }
    if projection.status == RunStatus::Waiting {
      let started = projection
        .notification_deliveries
        .values()
        .filter_map(|delivery| {
          if let NotificationDeliveryStatus::AttemptStarted {
            attempt,
            attempt_id,
            ..
          } = &delivery.status
          {
            Some((delivery.clone(), *attempt, attempt_id.clone()))
          } else {
            None
          }
        })
        .collect::<Vec<_>>();
      if started.is_empty() {
        return Ok(RunRecovery::Unchanged);
      }
      let now = Utc::now();
      for (delivery, attempt, attempt_id) in &started {
        append_to_history(
          &transaction,
          &mut events,
          run_id,
          generated_event_id(),
          now,
          event_schema_version,
          RunEventPayload::NotificationDeliveryFailed(NotificationDeliveryFailedData {
            approval_id: delivery.approval_id.clone(),
            request_id: delivery.request_id.clone(),
            delivery_id: delivery.delivery_id.clone(),
            attempt: *attempt,
            attempt_id: attempt_id.clone(),
            final_: true,
            failure: NotificationSafeFailure {
              kind: "delivery_ambiguous".to_string(),
              code: "WOML_NOTIFICATION_DELIVERY_AMBIGUOUS".to_string(),
              message: "Recovery found an uncertain notification send and will not replay it."
                .to_string(),
              retryable: false,
              retry_after_ms: None,
            },
          }),
        )?;
      }
      let mut recovered = fold_events(&events)?;
      for (approval_id, request_id) in started
        .iter()
        .map(|(delivery, _, _)| (&delivery.approval_id, &delivery.request_id))
      {
        if recovered.status == RunStatus::Waiting
          && notification_request_all_failed(&workflow, &recovered, approval_id, request_id)
        {
          let ids = workflow
            .approval(approval_id)
            .unwrap()
            .notifications
            .into_iter()
            .map(|item| item.delivery_id)
            .collect();
          append_to_history(
            &transaction,
            &mut events,
            run_id,
            generated_event_id(),
            now,
            event_schema_version,
            RunEventPayload::RunFailed(RunFailedData::V5(RunFailedDataV5::Notification {
              approval_id: approval_id.clone(),
              request_id: request_id.clone(),
              failed_delivery_ids: ids,
              failure: NotificationRunFailure {
                kind: "all_deliveries_failed".to_string(),
                code: "WOML_NOTIFICATION_DELIVERY_FAILED".to_string(),
                message: "Every configured approval notification delivery failed.".to_string(),
              },
            })),
          )?;
          recovered = fold_events(&events)?;
          break;
        }
      }
      validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
        .map_err(DurableStoreError::Contract)?;
      transaction.commit()?;
      let _ = recovered;
      return Ok(RunRecovery::Recovered {
        interrupted_attempts: 0,
      });
    }
    let started_updates = projection
      .notification_updates
      .values()
      .filter_map(|update| {
        if let NotificationMessageUpdateStatus::AttemptStarted {
          attempt,
          attempt_id,
        } = &update.status
        {
          Some((update.clone(), *attempt, attempt_id.clone()))
        } else {
          None
        }
      })
      .collect::<Vec<_>>();
    if !started_updates.is_empty() {
      let now = Utc::now();
      for (update, attempt, attempt_id) in &started_updates {
        append_to_history(
          &transaction,
          &mut events,
          run_id,
          generated_event_id(),
          now,
          event_schema_version,
          RunEventPayload::NotificationMessageUpdateFailed(
            NotificationMessageUpdateFailedData {
              approval_id: update.approval_id.clone(),
              request_id: update.request_id.clone(),
              delivery_id: update.delivery_id.clone(),
              update_id: update.update_id.clone(),
              attempt: *attempt,
              attempt_id: attempt_id.clone(),
              final_: *attempt >= 3,
              failure: NotificationSafeFailure {
                kind: "update_failed".to_string(),
                code: "WOML_NOTIFICATION_UPDATE_INTERRUPTED".to_string(),
                message: "Recovery found an interrupted message update; the durable update remains retryable."
                  .to_string(),
                retryable: *attempt < 3,
                retry_after_ms: None,
              },
            },
          ),
        )?;
      }
      validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
        .map_err(DurableStoreError::Contract)?;
      transaction.commit()?;
      return Ok(RunRecovery::Recovered {
        interrupted_attempts: 0,
      });
    }
    if projection.status != RunStatus::Running {
      return Ok(RunRecovery::Unchanged);
    }
    let active_operations = projection
      .operations
      .values()
      .filter(|operation| operation.status == crate::projection::OperationStatus::Started)
      .cloned()
      .collect::<Vec<_>>();
    if !active_operations.is_empty() {
      let now = Utc::now();
      for operation in &active_operations {
        if let Some(settlement) = recovered_state_mutation_settlement(&transaction, operation)? {
          append_to_history(
            &transaction,
            &mut events,
            run_id,
            generated_event_id(),
            now,
            event_schema_version,
            RunEventPayload::OperationSucceeded(OperationSucceededData {
              node_id: operation.node_id.clone(),
              attempt_number: operation.attempt_number,
              invocation_id: operation.identity.invocation_id.clone(),
              call_id: operation.identity.call_id.clone(),
              operation_key: operation.operation_key.clone(),
              capability: operation.capability.clone(),
              operation: operation.operation.clone(),
              execution_mode: operation.execution_mode,
              metadata: settlement.metadata,
              duration_ms: 0.0,
              result_bytes: settlement.result_bytes,
              result_digest: settlement.result_digest,
            }),
          )?;
          continue;
        }
        let observed = operation.execution_mode == crate::event::OperationExecutionMode::Observed;
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
            metadata: interrupted_operation_metadata(operation),
            duration_ms: 0.0,
            failure: interrupted_operation_failure(operation, observed, false),
          }),
        )?;
      }
    }
    let started = projection
      .attempts
      .iter()
      .filter(|attempt| attempt.status == AttemptStatus::Started)
      .map(|attempt| attempt.identity.clone())
      .collect::<Vec<_>>();
    if let Some(first) = started.first().cloned() {
      let failure = interrupted_failure();
      for attempt in &started {
        let payload = RunEventPayload::StepAttemptFailed(StepAttemptFailedData {
          node_id: attempt.node_id.clone(),
          attempt: attempt.attempt,
          invocation_id: attempt.invocation_id.clone(),
          failure: failure.clone(),
        });
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
          expand_model_v11_payload(&workflow, run_id, payload)?
        } else {
          vec![payload]
        };
        for payload in payloads {
          append_to_history(
            &transaction,
            &mut events,
            run_id,
            generated_event_id(),
            Utc::now(),
            event_schema_version,
            payload,
          )?;
        }
      }
      let has_unsettled_fork_work = workflow.schema_version
        >= crate::COMPILED_MODEL_SCHEMA_VERSION_V13
        && projection.forks.values().any(|fork| {
          fork.join_status == crate::projection::ForkJoinStatus::Pending
            || fork
              .branches
              .values()
              .any(|branch| branch.outcome.is_none())
        });
      if has_unsettled_fork_work {
        // The interrupted attempts are now durably failed, but Model v13 owns
        // every opened branch until it settles. Let the resumed fork scheduler
        // derive branch and join outcomes before it decides the run outcome;
        // appending RunFailed here would create an impossible half-open fork.
        validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
          .map_err(DurableStoreError::Contract)?;
        transaction.commit()?;
        return Ok(RunRecovery::Recovered {
          interrupted_attempts: started.len(),
        });
      }
      let run_failure = RunEventPayload::RunFailed(attempt_run_failed_data(
        event_schema_version,
        first.node_id,
        first.attempt,
        first.invocation_id,
        failure,
      ));
      let run_failure_payloads = if matches!(
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
        expand_model_v11_payload(&workflow, run_id, run_failure)?
      } else {
        vec![run_failure]
      };
      for payload in run_failure_payloads {
        append_to_history(
          &transaction,
          &mut events,
          run_id,
          generated_event_id(),
          Utc::now(),
          event_schema_version,
          payload,
        )?;
      }
      validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
        .map_err(DurableStoreError::Contract)?;
      transaction.commit()?;
      return Ok(RunRecovery::Recovered {
        interrupted_attempts: started.len(),
      });
    }

    for group_projection in projection.parallel_groups.values() {
      if group_projection.status != ParallelGroupStatus::Started {
        continue;
      }
      let group = workflow
        .parallel_group(&group_projection.parallel_id)
        .ok_or_else(|| {
          DurableStoreError::Contract(format!(
            "Stored run references unknown parallel group {:?}.",
            group_projection.parallel_id
          ))
        })?;
      let child_statuses = group
        .child_node_ids
        .iter()
        .map(|node_id| {
          (
            node_id,
            projection
              .attempts
              .iter()
              .rev()
              .find(|attempt| attempt.identity.node_id == *node_id)
              .map(|attempt| &attempt.status),
          )
        })
        .collect::<Vec<_>>();
      let every_succeeded = child_statuses
        .iter()
        .all(|(_, status)| matches!(status, Some(AttemptStatus::Succeeded { .. })));
      if every_succeeded {
        append_to_history(
          &transaction,
          &mut events,
          run_id,
          generated_event_id(),
          Utc::now(),
          event_schema_version,
          RunEventPayload::ParallelGroupCompleted(ParallelGroupCompletedData {
            parallel_id: group.parallel_id.clone(),
            outcome: ParallelGroupOutcome::Succeeded,
            failed_node_ids: Vec::new(),
            cancelled_node_ids: Vec::new(),
          }),
        )?;
        validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
          .map_err(DurableStoreError::Contract)?;
        transaction.commit()?;
        return Ok(RunRecovery::Recovered {
          interrupted_attempts: 0,
        });
      }

      let every_terminal = child_statuses.iter().all(|(node_id, status)| {
        matches!(
          status,
          Some(AttemptStatus::Succeeded { .. } | AttemptStatus::Failed { .. })
        ) && !projection.pending_retries.contains_key(*node_id)
      });
      let failed_node_ids = child_statuses
        .iter()
        .filter_map(|(node_id, status)| match status {
          Some(AttemptStatus::Failed { failure })
            if failure.kind != AttemptFailureKind::InvocationCancelled
              && !projection.pending_retries.contains_key(*node_id) =>
          {
            Some((*node_id).clone())
          }
          _ => None,
        })
        .collect::<Vec<_>>();
      if failed_node_ids.is_empty() || (group.on_error == "wait-all" && !every_terminal) {
        continue;
      }
      let cancelled_node_ids = child_statuses
        .iter()
        .filter_map(|(node_id, status)| match status {
          Some(AttemptStatus::Failed { failure })
            if failure.kind == AttemptFailureKind::InvocationCancelled =>
          {
            Some((*node_id).clone())
          }
          _ => None,
        })
        .collect::<Vec<_>>();
      let primary_node_id = events
        .iter()
        .find_map(|event| match &event.payload {
          RunEventPayload::StepAttemptFailed(data) if failed_node_ids.contains(&data.node_id) => {
            Some(data.node_id.clone())
          }
          _ => None,
        })
        .ok_or_else(|| {
          DurableStoreError::Contract(format!(
            "Parallel group {:?} has failed children without a failure event.",
            group.parallel_id
          ))
        })?;
      let policy = if group.on_error == "wait-all" {
        ParallelFailurePolicy::WaitAll
      } else {
        ParallelFailurePolicy::FailFast
      };
      let message = parallel_failure_message(policy, failed_node_ids.len());
      let failure = ParallelFailure {
        kind: "parallel_child_failed".to_string(),
        code: "WOML_PARALLEL_CHILD_FAILED".to_string(),
        message,
      };
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        Utc::now(),
        event_schema_version,
        RunEventPayload::ParallelGroupCompleted(ParallelGroupCompletedData {
          parallel_id: group.parallel_id.clone(),
          outcome: ParallelGroupOutcome::Failed,
          failed_node_ids: failed_node_ids.clone(),
          cancelled_node_ids: cancelled_node_ids.clone(),
        }),
      )?;
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        Utc::now(),
        event_schema_version,
        RunEventPayload::RunFailed(RunFailedData::V3(RunFailedDataV3::Parallel {
          parallel_id: group.parallel_id.clone(),
          policy,
          primary_node_id,
          failed_node_ids,
          cancelled_node_ids,
          failure,
        })),
      )?;
      validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
        .map_err(DurableStoreError::Contract)?;
      transaction.commit()?;
      return Ok(RunRecovery::Recovered {
        interrupted_attempts: 0,
      });
    }

    // A failed attempt followed by a durable retry schedule is unfinished work,
    // not a terminal failure. The schedule is the persistence boundary that
    // makes starting the next attempt safe after a process restart.
    if !projection.pending_retries.is_empty() {
      return Ok(RunRecovery::Resumable);
    }

    // Only the latest attempt for a node can determine its current state. An
    // older failure must never override a later successful retry.
    let failed_attempt = projection.attempts.iter().rev().find_map(|attempt| {
      let is_latest_for_node = projection
        .attempts
        .iter()
        .rev()
        .find(|candidate| candidate.identity.node_id == attempt.identity.node_id)
        .is_some_and(|latest| latest.identity == attempt.identity);
      if is_latest_for_node {
        if let AttemptStatus::Failed { failure } = &attempt.status {
          return Some((attempt.identity.clone(), failure.clone()));
        }
      }
      None
    });
    if let Some((identity, failure)) = failed_attempt {
      if workflow
        .parallel_group_for_child(&identity.node_id)
        .is_some_and(|group| projection.parallel_groups.contains_key(&group.parallel_id))
      {
        return Ok(RunRecovery::Resumable);
      }
      let run_failure = RunEventPayload::RunFailed(attempt_run_failed_data(
        event_schema_version,
        identity.node_id,
        identity.attempt,
        identity.invocation_id,
        failure,
      ));
      let run_failure_payloads = if matches!(
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
        expand_model_v11_payload(&workflow, run_id, run_failure)?
      } else {
        vec![run_failure]
      };
      for payload in run_failure_payloads {
        append_to_history(
          &transaction,
          &mut events,
          run_id,
          generated_event_id(),
          Utc::now(),
          event_schema_version,
          payload,
        )?;
      }
      transaction.commit()?;
      return Ok(RunRecovery::Recovered {
        interrupted_attempts: 0,
      });
    }

    let terminal_node_id = workflow.terminal_node_id().ok_or_else(|| {
      DurableStoreError::Contract("Stored workflow has no terminal node.".to_string())
    })?;
    if let Some(result) = projection.context.steps.get(terminal_node_id).cloned() {
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        Utc::now(),
        event_schema_version,
        RunEventPayload::RunSucceeded(RunSucceededData {
          terminal_node_id: terminal_node_id.to_string(),
          result,
        }),
      )?;
      transaction.commit()?;
      return Ok(RunRecovery::Recovered {
        interrupted_attempts: 0,
      });
    }

    Ok(RunRecovery::Resumable)
  }
}
