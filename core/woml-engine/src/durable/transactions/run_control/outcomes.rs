//! Business outcome decisions and lifecycle finalization, retaining separate facts and original settlement ordering.

use crate::durable::append_to_history;
use crate::durable::definition_for_run;
use crate::durable::ensure_run_exists;
use crate::durable::generated_event_id;
use crate::durable::lifecycle_warnings_from_projection;
use crate::durable::load_events;
use crate::durable::run_binding_in_transaction;
use crate::durable::DurableStoreError;
use crate::durable::{derive_lifecycle_hook_invocation_id, DurableEventStore};
use crate::engine::validate_event_history_against_definition;
use crate::event::LifecycleHookRequestedData;
use crate::event::LifecycleSubject;
use crate::event::LifecycleSubjectKind;
use crate::fold_events;
use crate::RunEventPayload;
use crate::RunProjection;
use chrono::DateTime;
use chrono::Utc;
use rusqlite::TransactionBehavior;

impl DurableEventStore {
  pub fn decide_run_outcome(
    &mut self,
    run_id: &str,
    outcome: crate::event::RunOutcomeDecidedData,
    now: DateTime<Utc>,
  ) -> Result<RunProjection, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
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
      return Err(DurableStoreError::Contract(
        "Business-outcome authority requires compiled Model v11+.".to_string(),
      ));
    }
    let mut events = load_events(&transaction, run_id)?;
    let projection = fold_events(&events)?;
    if projection.business_outcome.is_some() {
      return Err(DurableStoreError::Contract(
        "A run business outcome is already durable.".to_string(),
      ));
    }
    let event_schema_version = projection.event_schema_version.ok_or_else(|| {
      DurableStoreError::Contract("Business outcome requires run_started.".to_string())
    })?;
    let lifecycle_event = match outcome.outcome() {
      crate::event::BusinessOutcome::Succeeded => crate::model::LifecycleEventName::RunSuccess,
      crate::event::BusinessOutcome::Failed => crate::model::LifecycleEventName::RunFailure,
      crate::event::BusinessOutcome::Cancelled => crate::model::LifecycleEventName::RunCancel,
    };
    append_to_history(
      &transaction,
      &mut events,
      run_id,
      generated_event_id(),
      now,
      event_schema_version,
      RunEventPayload::RunOutcomeDecided(outcome),
    )?;
    if let Some(hook) = workflow
      .lifecycle_hook_for_event(lifecycle_event)
      .or_else(|| workflow.lifecycle_hook_for_event(crate::model::LifecycleEventName::RunComplete))
    {
      let already_requested = fold_events(&events)?
        .lifecycle_hooks
        .values()
        .any(|candidate| {
          candidate.hook_id == hook.hook_id
            && candidate.subject.kind == LifecycleSubjectKind::Workflow
            && candidate.subject.id == run_id
        });
      if !already_requested {
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
    }
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = fold_events(&events)?;
    transaction.commit()?;
    Ok(projection)
  }

  pub fn finalize_run_v11(
    &mut self,
    run_id: &str,
    now: DateTime<Utc>,
  ) -> Result<RunProjection, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    if !matches!(
      workflow.schema_version,
      crate::COMPILED_MODEL_SCHEMA_VERSION_V11
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V12
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V13
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V14
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V15
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V16
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V17
    ) {
      return Err(DurableStoreError::Contract(
        "Run finalization authority requires compiled Model v11+.".to_string(),
      ));
    }
    let mut events = load_events(&transaction, run_id)?;
    let projection = fold_events(&events)?;
    let outcome = projection.business_outcome.ok_or_else(|| {
      DurableStoreError::Contract("Run finalization requires a decided outcome.".to_string())
    })?;
    if projection.lifecycle_hooks.values().any(|hook| {
      matches!(
        hook.status,
        crate::projection::LifecycleHookStatus::Requested
          | crate::projection::LifecycleHookStatus::Running
      )
    }) {
      return Err(DurableStoreError::Contract(
        "Run finalization requires every admitted lifecycle hook to complete.".to_string(),
      ));
    }
    let warnings = lifecycle_warnings_from_projection(&projection);
    let lifecycle_status = if warnings.is_empty() {
      crate::event::FinalLifecycleStatus::Completed
    } else {
      crate::event::FinalLifecycleStatus::CompletedWithWarnings
    };
    let event_schema_version = projection.event_schema_version.unwrap();
    append_to_history(
      &transaction,
      &mut events,
      run_id,
      generated_event_id(),
      now,
      event_schema_version,
      RunEventPayload::RunFinalized(crate::event::RunFinalizedData {
        outcome,
        lifecycle_status,
        warnings,
      }),
    )?;
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = fold_events(&events)?;
    transaction.commit()?;
    Ok(projection)
  }
}
