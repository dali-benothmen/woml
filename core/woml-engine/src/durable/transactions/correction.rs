//! Audited operator retry/external-resolution decisions and stable corrective identities. External decisions are not fabricated forward success.

use crate::durable::generated_event_id;
use crate::durable::CorrectionRecoveryActionV1;
use crate::durable::CorrectionRecoveryRequestV1;
use crate::durable::CorrectionRecoveryResultV1;
use crate::durable::CorrectionRecoveryStatusV1;
use crate::durable::DurableStoreError;
use crate::durable::CORRECTION_RECOVERY_REQUEST_PROFILE;
use crate::durable::CORRECTION_RECOVERY_RESULT_PROFILE;
use crate::durable::{sha256_prefixed, DurableEventStore};
use crate::RunEventPayload;
use chrono::DateTime;
use chrono::Utc;
use rusqlite::params;
use rusqlite::TransactionBehavior;
use sha2::Digest;
use sha2::Sha256;

pub fn correction_invocation_id(
  run_id: &str,
  saga_id: &str,
  action_id: &str,
  attempt: u32,
) -> String {
  let mut digest = Sha256::new();
  digest.update(b"woml:correction-invocation:v1\0");
  for value in [run_id, saga_id, action_id] {
    digest.update(value.as_bytes());
    digest.update([0]);
  }
  digest.update(attempt.to_be_bytes());
  format!("corr_{}", hex::encode(digest.finalize()))
}

impl DurableEventStore {
  pub fn correction_action_externally_resolved(
    &self,
    run_id: &str,
    saga_id: &str,
    action_id: &str,
  ) -> Result<bool, DurableStoreError> {
    let resolved: bool = self.connection.query_row(
      "SELECT EXISTS(
         SELECT 1 FROM woml_correction_manual_recovery
         WHERE run_id = ?1 AND saga_id = ?2 AND action_id = ?3
           AND status = 'externally_resolved'
       )",
      params![run_id, saga_id, action_id],
      |row| row.get(0),
    )?;
    Ok(resolved)
  }

  pub fn recover_correction_v1(
    &mut self,
    request: CorrectionRecoveryRequestV1,
    now: DateTime<Utc>,
  ) -> Result<CorrectionRecoveryResultV1, DurableStoreError> {
    if request.profile != CORRECTION_RECOVERY_REQUEST_PROFILE
      || request.run_id.is_empty()
      || request.run_id.len() > 320
      || request.action_id.is_empty()
      || request.action_id.len() > 768
      || request.note.as_ref().is_some_and(|note| {
        note.is_empty() || note.len() > 1000 || note.chars().any(|character| character.is_control())
      })
      || request.action == CorrectionRecoveryActionV1::ExternallyResolve && request.note.is_none()
    {
      return Err(DurableStoreError::Contract(
        "Correction Recovery v1 request is invalid.".to_string(),
      ));
    }
    let projection = self.projection(&request.run_id)?;
    let mut matches = projection.sagas.values().filter_map(|saga| {
      saga
        .actions
        .get(&request.action_id)
        .map(|action| (saga.saga_id.clone(), action.clone()))
    });
    let Some((saga_id, action)) = matches.next() else {
      return Err(DurableStoreError::Contract(
        "WOML_CORRECTION_NOT_RECOVERABLE: the requested action is unknown.".to_string(),
      ));
    };
    if matches.next().is_some()
      || action.status != crate::CorrectionActionStatus::ManualRecovery
      || !action.ambiguous
    {
      return Err(DurableStoreError::Contract(
        "WOML_CORRECTION_NOT_RECOVERABLE: the requested action is not awaiting manual recovery."
          .to_string(),
      ));
    }
    let status = match request.action {
      CorrectionRecoveryActionV1::Retry => {
        if action.attempts >= 10 {
          return Err(DurableStoreError::Contract(
            "WOML_CORRECTION_LIMIT_EXCEEDED: the action reached the frozen ten-attempt limit."
              .to_string(),
          ));
        }
        self.append_payloads_atomically(
          &request.run_id,
          vec![(
            generated_event_id(),
            now,
            RunEventPayload::CorrectionRetryScheduled(crate::CorrectionRetryScheduledData {
              saga_id,
              action_id: request.action_id.clone(),
              action_kind: action.action_kind,
              attempt: action.attempts,
              retry_at: now,
            }),
          )],
        )?;
        CorrectionRecoveryStatusV1::RetryScheduled
      }
      CorrectionRecoveryActionV1::ExternallyResolve => {
        let note = request
          .note
          .as_deref()
          .expect("external resolution requires a note");
        let note_digest = sha256_prefixed(note.as_bytes());
        let transaction = self
          .connection
          .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
          "UPDATE woml_correction_manual_recovery
           SET status = 'externally_resolved',
               reason_code = ?1,
               resolved_at = ?2
           WHERE run_id = ?3 AND saga_id = ?4 AND action_id = ?5
             AND status = 'required'",
          params![
            format!("WOML_CORRECTION_EXTERNALLY_RESOLVED:{note_digest}"),
            now.to_rfc3339(),
            request.run_id,
            saga_id,
            request.action_id
          ],
        )?;
        if changed != 1 {
          return Err(DurableStoreError::Contract(
            "WOML_CORRECTION_NOT_RECOVERABLE: the recovery decision was already recorded."
              .to_string(),
          ));
        }
        transaction.commit()?;
        CorrectionRecoveryStatusV1::ExternallyResolved
      }
    };
    Ok(CorrectionRecoveryResultV1 {
      profile: CORRECTION_RECOVERY_RESULT_PROFILE,
      run_id: request.run_id,
      action_id: request.action_id,
      status,
      audit_recorded: true,
    })
  }
}
