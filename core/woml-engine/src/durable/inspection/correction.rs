//! Bounded correction inspection and health without performing recovery actions.

use crate::durable::{
  CorrectionInspectionV1, CorrectionObservationV1, CorrectionSagaInspectionV1, DurableEventStore,
  DurableStoreError, UnresolvedCorrectionActionV1, CORRECTION_INSPECTION_PROFILE,
  CORRECTION_OBSERVATION_PROFILE,
};
use chrono::DateTime;
use chrono::Utc;

const MAX_CORRECTION_INSPECTION_ROWS: usize = 100;

const MAX_CORRECTION_INSPECTION_BYTES: usize = 1024 * 1024;

impl DurableEventStore {
  /// Builds the bounded public correction view exclusively from immutable run
  /// events. Store v17 tables remain disposable acceleration indexes.
  pub fn inspect_correction_v1(
    &self,
    run_id: &str,
  ) -> Result<CorrectionInspectionV1, DurableStoreError> {
    let binding = self.run_binding(run_id)?;
    let workflow = self.definition(&binding.definition_hash)?;
    if workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V19 {
      return Err(DurableStoreError::Contract(
        "Correction Inspection v1 is available only for compiled Model v19 or later runs."
          .to_string(),
      ));
    }
    let projection = self.projection(run_id)?;
    let phase = if projection
      .sagas
      .values()
      .any(|saga| saga.phase == crate::SagaPhase::ManualRecovery)
    {
      "manual_recovery"
    } else if projection
      .sagas
      .values()
      .any(|saga| saga.phase == crate::SagaPhase::Correcting)
    {
      let cleanup_started = projection.sagas.values().any(|saga| {
        saga.actions.values().any(|action| {
          action.action_kind == crate::CorrectiveActionKind::Cleanup
            && action.status != crate::CorrectionActionStatus::Registered
        })
      });
      if cleanup_started {
        "cleanup"
      } else {
        "compensating"
      }
    } else if projection
      .sagas
      .values()
      .any(|saga| saga.phase == crate::SagaPhase::Forward)
    {
      "forward"
    } else if projection
      .sagas
      .values()
      .all(|saga| saga.phase == crate::SagaPhase::CorrectionCompleted)
    {
      "completed"
    } else {
      "forward"
    };
    let original_outcome = projection
      .sagas
      .values()
      .find_map(|saga| saga.reason)
      .map_or("running", |reason| match reason {
        crate::SagaCorrectionReason::Failure => "failed",
        crate::SagaCorrectionReason::Cancel => "cancelled",
        crate::SagaCorrectionReason::Success => "succeeded",
      });
    let mut sagas = Vec::new();
    let mut unresolved = Vec::new();
    for saga in projection.sagas.values() {
      let status = match saga.phase {
        crate::SagaPhase::Forward => "forward",
        crate::SagaPhase::ForwardSucceeded => "forward_succeeded",
        crate::SagaPhase::Correcting => "correcting",
        crate::SagaPhase::CorrectionCompleted => {
          if saga.actions.values().any(|action| {
            matches!(
              action.status,
              crate::CorrectionActionStatus::Failed | crate::CorrectionActionStatus::Interrupted
            )
          }) {
            "correction_incomplete"
          } else {
            "corrected"
          }
        }
        crate::SagaPhase::ManualRecovery => "correction_incomplete",
      };
      sagas.push(CorrectionSagaInspectionV1 {
        saga_id: saga.saga_id.clone(),
        status: status.to_string(),
        registered_compensations: u32::try_from(
          saga
            .actions
            .values()
            .filter(|action| action.action_kind == crate::CorrectiveActionKind::Compensation)
            .count(),
        )
        .unwrap_or(u32::MAX),
        completed_compensations: u32::try_from(
          saga
            .actions
            .values()
            .filter(|action| {
              action.action_kind == crate::CorrectiveActionKind::Compensation
                && action.status == crate::CorrectionActionStatus::Succeeded
            })
            .count(),
        )
        .unwrap_or(u32::MAX),
        completed_cleanup: u32::try_from(
          saga
            .actions
            .values()
            .filter(|action| {
              action.action_kind == crate::CorrectiveActionKind::Cleanup
                && action.status == crate::CorrectionActionStatus::Succeeded
            })
            .count(),
        )
        .unwrap_or(u32::MAX),
      });
      unresolved.extend(saga.actions.values().filter_map(|action| {
        let externally_settled = saga.phase == crate::SagaPhase::CorrectionCompleted
          && action.status == crate::CorrectionActionStatus::ManualRecovery;
        let unresolved = matches!(
          action.status,
          crate::CorrectionActionStatus::Failed
            | crate::CorrectionActionStatus::Interrupted
            | crate::CorrectionActionStatus::ManualRecovery
        );
        (unresolved && !externally_settled).then(|| UnresolvedCorrectionActionV1 {
          action_id: action.action_id.clone(),
          kind: action.action_kind,
          code: action
            .failure_code
            .clone()
            .unwrap_or_else(|| "WOML_CORRECTION_INCOMPLETE".to_string()),
          ambiguous: action.ambiguous,
        })
      }));
    }
    sagas.sort_by(|left, right| left.saga_id.cmp(&right.saga_id));
    unresolved.sort_by(|left, right| left.action_id.cmp(&right.action_id));
    let truncated = unresolved.len() > MAX_CORRECTION_INSPECTION_ROWS;
    unresolved.truncate(MAX_CORRECTION_INSPECTION_ROWS);
    let inspection = CorrectionInspectionV1 {
      profile: CORRECTION_INSPECTION_PROFILE,
      run_id: run_id.to_string(),
      phase: phase.to_string(),
      original_outcome: original_outcome.to_string(),
      sagas,
      unresolved_actions: unresolved,
      truncated,
    };
    if serde_json::to_vec(&inspection)?.len() > MAX_CORRECTION_INSPECTION_BYTES {
      return Err(DurableStoreError::Contract(
        "Correction Inspection v1 exceeded its frozen 1 MiB response limit.".to_string(),
      ));
    }
    Ok(inspection)
  }

  pub fn correction_observation_v1(&self) -> Result<CorrectionObservationV1, DurableStoreError> {
    let count = |sql: &str| -> Result<u64, DurableStoreError> {
      let value: i64 = self.connection.query_row(sql, [], |row| row.get(0))?;
      u64::try_from(value)
        .map_err(|_| DurableStoreError::Contract("Correction metric is invalid.".to_string()))
    };
    let oldest: Option<String> = self.connection.query_row(
      "SELECT MIN(created_at) FROM woml_correction_manual_recovery WHERE status = 'required'",
      [],
      |row| row.get(0),
    )?;
    let oldest_manual_recovery_age_ms = oldest
      .map(|value| {
        DateTime::parse_from_rfc3339(&value)
          .map(|time| Utc::now().signed_duration_since(time.with_timezone(&Utc)))
          .map_err(|_| DurableStoreError::Contract("Correction timestamp is invalid.".to_string()))
      })
      .transpose()?
      .and_then(|age| u64::try_from(age.num_milliseconds().max(0)).ok());
    Ok(CorrectionObservationV1 {
      profile: CORRECTION_OBSERVATION_PROFILE,
      open_sagas: count(
        "SELECT COUNT(*) FROM woml_saga_scopes WHERE phase IN ('correcting', 'manual_recovery')",
      )?,
      pending_actions: count(
        "SELECT COUNT(*) FROM woml_correction_actions WHERE status IN ('registered', 'started', 'retry_scheduled', 'failed', 'interrupted', 'manual_recovery')",
      )?,
      active_attempts: count(
        "SELECT COUNT(*) FROM woml_correction_attempts WHERE status = 'started'",
      )?,
      manual_recovery_actions: count(
        "SELECT COUNT(*) FROM woml_correction_manual_recovery WHERE status = 'required'",
      )?,
      oldest_manual_recovery_age_ms,
    })
  }
}
