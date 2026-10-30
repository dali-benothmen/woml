//! Atomic cancellation request and associated wait/approval/queue transitions. A request is not proof an external effect stopped.

use crate::durable::append_to_history;
use crate::durable::definition_by_hash;
use crate::durable::generated_event_id;
use crate::durable::load_events;
use crate::durable::load_run_binding_optional;
use crate::durable::DurableStoreError;
use crate::durable::RunCancellationCode;
use crate::durable::RunCancellationResult;
use crate::durable::RunCancellationStatus;
use crate::durable::{derive_lifecycle_hook_invocation_id, DurableEventStore};
use crate::engine::validate_event_history_against_definition;
use crate::event::LifecycleHookRequestedData;
use crate::event::LifecycleSubject;
use crate::event::LifecycleSubjectKind;
use crate::event::RunCancellationRequestedData;
use crate::fold_events;
use crate::RunEventPayload;
use crate::RunStatus;
use chrono::DateTime;
use chrono::Utc;
use rusqlite::params;
use rusqlite::TransactionBehavior;

impl DurableEventStore {
  pub fn request_run_cancellation(
    &mut self,
    run_id: &str,
    command_id: &str,
    now: DateTime<Utc>,
  ) -> Result<RunCancellationResult, DurableStoreError> {
    if run_id.is_empty() || run_id.len() > 320 || command_id.is_empty() || command_id.len() > 320 {
      return Err(DurableStoreError::Contract(
        "Run cancellation requires bounded run and command identities.".to_string(),
      ));
    }
    let result = |status, code| RunCancellationResult {
      profile: "woml.run-control.result/v1",
      command_id: command_id.to_string(),
      run_id: run_id.to_string(),
      status,
      code,
    };
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(binding) = load_run_binding_optional(&transaction, run_id)? else {
      return Ok(result(
        RunCancellationStatus::Rejected,
        Some(RunCancellationCode::WomlRunNotFound),
      ));
    };
    let workflow = definition_by_hash(&transaction, &binding.definition_hash)?;
    if !matches!(
      workflow.schema_version,
      crate::COMPILED_MODEL_SCHEMA_VERSION_V11
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V12
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V13
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V14
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V15
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V16
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V17
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V18
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V19
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V20
    ) {
      return Ok(result(
        RunCancellationStatus::Rejected,
        Some(RunCancellationCode::WomlRunControlVersionUnsupported),
      ));
    }
    let mut events = load_events(&transaction, run_id)?;
    let projection = fold_events(&events)?;
    if projection.business_outcome == Some(crate::event::BusinessOutcome::Cancelled) {
      return Ok(result(RunCancellationStatus::AlreadyCancelled, None));
    }
    if projection.cancellation_request_id.is_some() {
      return Ok(result(RunCancellationStatus::AlreadyRequested, None));
    }
    if projection.business_outcome.is_some() {
      return Ok(result(
        RunCancellationStatus::Rejected,
        Some(RunCancellationCode::WomlRunOutcomeAlreadyDecided),
      ));
    }
    if matches!(
      projection.status,
      RunStatus::Succeeded | RunStatus::Failed | RunStatus::Cancelled
    ) {
      return Ok(result(
        RunCancellationStatus::Rejected,
        Some(RunCancellationCode::WomlRunAlreadyTerminal),
      ));
    }
    let event_schema_version = projection.event_schema_version.ok_or_else(|| {
      DurableStoreError::Contract("A run cancellation requires run_started.".to_string())
    })?;
    append_to_history(
      &transaction,
      &mut events,
      run_id,
      generated_event_id(),
      now,
      event_schema_version,
      RunEventPayload::RunCancellationRequested(RunCancellationRequestedData {
        request_id: command_id.to_string(),
      }),
    )?;
    if event_schema_version == crate::RUN_EVENT_SCHEMA_VERSION_V16
      && projection.status == RunStatus::Waiting
    {
      let active_wait_ids = projection
        .durable_waits
        .values()
        .filter(|wait| wait.status == crate::DurableWaitStatus::Registered)
        .map(|wait| wait.wait_id.clone())
        .collect::<Vec<_>>();
      for wait_id in active_wait_ids {
        let resolution_event_id = generated_event_id();
        append_to_history(
          &transaction,
          &mut events,
          run_id,
          resolution_event_id.clone(),
          now,
          event_schema_version,
          RunEventPayload::WaitCancelled(crate::WaitCancelledData {
            wait_id: wait_id.clone(),
            cancelled_at: now,
            cancellation_id: command_id.to_string(),
          }),
        )?;
        let changed = transaction.execute(
          "UPDATE woml_durable_waits
           SET state = 'cancelled', resolution_event_id = ?1, resolved_at = ?2
           WHERE run_id = ?3 AND wait_id = ?4 AND iteration_identity = ''
             AND state = 'registered'",
          params![resolution_event_id, now.to_rfc3339(), run_id, wait_id],
        )?;
        if changed != 1 {
          return Err(DurableStoreError::Contract(
            "Run cancellation lost the durable wait winner transaction.".to_string(),
          ));
        }
      }
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        now,
        event_schema_version,
        RunEventPayload::RunOutcomeDecided(crate::RunOutcomeDecidedData::Cancelled {
          cancellation_request_id: command_id.to_string(),
        }),
      )?;
    }
    if let Some(hook) =
      workflow.lifecycle_hook_for_event(crate::model::LifecycleEventName::RunCancel)
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
    transaction.commit()?;
    Ok(result(RunCancellationStatus::Accepted, None))
  }
}
