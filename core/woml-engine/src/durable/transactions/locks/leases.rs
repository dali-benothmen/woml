//! Exact held-lease renewal, release, timeout, and waiter cancellation. Late release intentionally commits expiry/handoff before returning rejection when ownership was lost.

use crate::durable::append_to_history_scoped;
use crate::durable::definition_for_run;
use crate::durable::ensure_run_exists;
use crate::durable::generated_event_id;
use crate::durable::load_events;
use crate::durable::DurableStoreError;
use crate::durable::LockLeaseMaintenance;
use crate::durable::{
  expire_lock_owner, grant_oldest_lock_waiter, lock_descriptor_for_protected_node,
  parse_stored_timestamp, DurableEventStore,
};
use crate::event::ForEachIterationScope;
use crate::fold_events;
use crate::RunEventPayload;
use crate::RunProjection;
use crate::RunStatus;
use chrono::DateTime;
use chrono::Utc;
use rusqlite::params;
use rusqlite::OptionalExtension;
use rusqlite::TransactionBehavior;
use serde_json::Value;

impl DurableEventStore {
  pub fn release_lock_atomically(
    &mut self,
    run_id: &str,
    lock_id: &str,
    result: Value,
    released_at: DateTime<Utc>,
  ) -> Result<RunProjection, DurableStoreError> {
    self.release_lock_atomically_scoped(run_id, lock_id, None, result, released_at)
  }

  pub fn release_lock_atomically_scoped(
    &mut self,
    run_id: &str,
    lock_id: &str,
    iteration: Option<ForEachIterationScope>,
    result: Value,
    released_at: DateTime<Utc>,
  ) -> Result<RunProjection, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let mut events = load_events(&transaction, run_id)?;
    let initial = fold_events(&events)?;
    let initial_lock = iteration
      .as_ref()
      .and_then(|scope| initial.for_each.get(&scope.for_each_id))
      .and_then(|loop_| loop_.executions.get(&iteration.as_ref()?.index))
      .and_then(|execution| execution.locks.get(lock_id))
      .or_else(|| {
        iteration
          .is_none()
          .then(|| initial.locks.get(lock_id))
          .flatten()
      })
      .ok_or_else(|| {
        DurableStoreError::LockHistoryInvalid("release has no projected lock".to_string())
      })?;
    if initial_lock.status != crate::LockStatus::Held {
      return Err(DurableStoreError::LockHistoryInvalid(
        "release requires one held lease".to_string(),
      ));
    }
    let key = initial_lock.key.clone();
    // Expiry wins atomically over a late release. This both records lease loss
    // before clearing the old owner and grants the oldest eligible successor.
    grant_oldest_lock_waiter(&transaction, &key, released_at)?;
    events = load_events(&transaction, run_id)?;
    let projection = fold_events(&events)?;
    let held = iteration
      .as_ref()
      .and_then(|scope| projection.for_each.get(&scope.for_each_id))
      .and_then(|loop_| loop_.executions.get(&iteration.as_ref()?.index))
      .and_then(|execution| execution.locks.get(lock_id))
      .or_else(|| {
        iteration
          .is_none()
          .then(|| projection.locks.get(lock_id))
          .flatten()
      })
      .ok_or_else(|| {
        DurableStoreError::LockHistoryInvalid("release has no projected lock".to_string())
      })?;
    if held.status != crate::LockStatus::Held {
      transaction.commit()?;
      return Err(DurableStoreError::LockHistoryInvalid(
        "release rejected because the lease expired before settlement".to_string(),
      ));
    }
    let lease_id = held.lease_id.clone().ok_or_else(|| {
      DurableStoreError::LockHistoryInvalid("held lock has no lease ID".to_string())
    })?;
    let fencing_token = held.fencing_token.clone().ok_or_else(|| {
      DurableStoreError::LockHistoryInvalid("held lock has no fencing token".to_string())
    })?;
    let changed = transaction.execute(
      "UPDATE woml_lock_resources SET owner_lease_id = NULL, owner_run_id = NULL,
       owner_node_execution_id = NULL, owner_fencing_token = NULL, acquired_at = NULL,
       expires_at = NULL, updated_at = ?1 WHERE key = ?2 AND owner_run_id = ?3
       AND owner_lease_id = ?4 AND CAST(owner_fencing_token AS TEXT) = ?5",
      params![
        released_at.to_rfc3339(),
        held.key,
        run_id,
        lease_id,
        fencing_token
      ],
    )?;
    if changed != 1 {
      return Err(DurableStoreError::LockHistoryInvalid(
        "durable owner differs from projected held lease".to_string(),
      ));
    }
    transaction.execute(
      "UPDATE woml_lock_waiters SET status = 'released' WHERE run_id = ?1 AND node_execution_id = ?2 AND status = 'held'",
      params![run_id, held.execution_identity],
    )?;
    let schema = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| {
        DurableStoreError::LockHistoryInvalid("release history is empty".to_string())
      })?;
    append_to_history_scoped(
      &transaction,
      &mut events,
      run_id,
      generated_event_id(),
      released_at,
      schema,
      iteration,
      RunEventPayload::LockReleased(crate::LockReleasedData {
        lock_id: lock_id.to_string(),
        key: held.key.clone(),
        lease_id,
        fencing_token,
        released_at,
        result,
      }),
    )?;
    grant_oldest_lock_waiter(&transaction, &held.key, released_at)?;
    let projection = fold_events(&events)?;
    transaction.commit()?;
    Ok(projection)
  }

  /// Renews one exact lease or durably fences it when its expiry has already
  /// passed. Renewal is a compact coordination update: it deliberately does
  /// not append an event on every heartbeat.
  pub fn renew_lock_lease_atomically(
    &mut self,
    run_id: &str,
    lock_id: &str,
    now: DateTime<Utc>,
  ) -> Result<LockLeaseMaintenance, DurableStoreError> {
    self.renew_lock_lease_atomically_scoped(run_id, lock_id, None, now)
  }

  pub fn renew_lock_lease_atomically_scoped(
    &mut self,
    run_id: &str,
    lock_id: &str,
    iteration: Option<ForEachIterationScope>,
    now: DateTime<Utc>,
  ) -> Result<LockLeaseMaintenance, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let descriptor = workflow.durable_lock(lock_id).ok_or_else(|| {
      DurableStoreError::Contract("Lock renewal references an unknown Model v18 lock.".to_string())
    })?;
    let events = load_events(&transaction, run_id)?;
    let projection = fold_events(&events)?;
    let held = iteration
      .as_ref()
      .and_then(|scope| projection.for_each.get(&scope.for_each_id))
      .and_then(|loop_| loop_.executions.get(&iteration.as_ref()?.index))
      .and_then(|execution| execution.locks.get(lock_id))
      .or_else(|| {
        iteration
          .is_none()
          .then(|| projection.locks.get(lock_id))
          .flatten()
      });
    let Some(held) = held.filter(|lock| lock.status == crate::LockStatus::Held) else {
      transaction.commit()?;
      return Ok(LockLeaseMaintenance::NotHeld);
    };
    let lease_id = held.lease_id.as_deref().ok_or_else(|| {
      DurableStoreError::LockHistoryInvalid("held lock has no lease ID".to_string())
    })?;
    let fencing_token = held.fencing_token.as_deref().ok_or_else(|| {
      DurableStoreError::LockHistoryInvalid("held lock has no fencing token".to_string())
    })?;
    let stored_expiry: Option<String> = transaction
      .query_row(
        "SELECT expires_at FROM woml_lock_resources
         WHERE key = ?1 AND owner_run_id = ?2 AND owner_node_execution_id = ?3
           AND owner_lease_id = ?4 AND CAST(owner_fencing_token AS TEXT) = ?5",
        params![
          held.key,
          run_id,
          held.execution_identity,
          lease_id,
          fencing_token
        ],
        |row| row.get(0),
      )
      .optional()?;
    let Some(stored_expiry) = stored_expiry else {
      transaction.commit()?;
      return Ok(LockLeaseMaintenance::Lost);
    };
    let parsed_stored_expiry = parse_stored_timestamp(&stored_expiry)?;
    if parsed_stored_expiry <= now {
      expire_lock_owner(&transaction, &held.key, now)?;
      grant_oldest_lock_waiter(&transaction, &held.key, now)?;
      transaction.commit()?;
      return Ok(LockLeaseMaintenance::Lost);
    }
    if projection.business_outcome.is_some() {
      transaction.commit()?;
      return Ok(LockLeaseMaintenance::NotHeld);
    }
    let requested_expiry = now
      + chrono::Duration::milliseconds(i64::try_from(descriptor.lease_ms).map_err(|_| {
        DurableStoreError::Contract("Lock lease exceeds the runtime clock.".to_string())
      })?);
    let next_expiry = requested_expiry.max(parsed_stored_expiry);
    let changed = transaction.execute(
      "UPDATE woml_lock_resources SET expires_at = ?1, updated_at = ?2
       WHERE key = ?3 AND owner_run_id = ?4 AND owner_node_execution_id = ?5
         AND owner_lease_id = ?6 AND CAST(owner_fencing_token AS TEXT) = ?7
         AND expires_at = ?8",
      params![
        next_expiry.to_rfc3339(),
        now.to_rfc3339(),
        held.key,
        run_id,
        held.execution_identity,
        lease_id,
        fencing_token,
        stored_expiry
      ],
    )?;
    transaction.commit()?;
    Ok(if changed == 1 {
      LockLeaseMaintenance::Renewed
    } else {
      LockLeaseMaintenance::Lost
    })
  }

  /// Verifies that a managed effect is still protected by the exact live
  /// lease projected for its node. Expiry is settled before returning false,
  /// so a stale owner cannot race a newer grant through WOML.
  pub fn managed_effect_has_current_lock(
    &mut self,
    run_id: &str,
    node_id: &str,
    invocation_id: &str,
    now: DateTime<Utc>,
  ) -> Result<bool, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let Some(descriptor) = lock_descriptor_for_protected_node(&workflow, node_id) else {
      transaction.commit()?;
      return Ok(true);
    };
    let projection = fold_events(&load_events(&transaction, run_id)?)?;
    let iteration_lock = projection.for_each.values().find_map(|loop_| {
      loop_.executions.values().find_map(|execution| {
        execution
          .attempts
          .iter()
          .any(|attempt| {
            attempt.identity.node_id == node_id
              && attempt.identity.invocation_id == invocation_id
              && attempt.status == crate::projection::AttemptStatus::Started
          })
          .then(|| execution.locks.get(&descriptor.lock_id))
          .flatten()
      })
    });
    let Some(held) = projection
      .locks
      .get(&descriptor.lock_id)
      .or(iteration_lock)
      .filter(|lock| lock.status == crate::LockStatus::Held)
    else {
      transaction.commit()?;
      return Ok(false);
    };
    let lease_id = held.lease_id.as_deref().unwrap_or_default();
    let fencing_token = held.fencing_token.as_deref().unwrap_or_default();
    let expiry: Option<String> = transaction
      .query_row(
        "SELECT expires_at FROM woml_lock_resources
         WHERE key = ?1 AND owner_run_id = ?2 AND owner_node_execution_id = ?3
           AND owner_lease_id = ?4 AND CAST(owner_fencing_token AS TEXT) = ?5",
        params![
          held.key,
          run_id,
          held.execution_identity,
          lease_id,
          fencing_token
        ],
        |row| row.get(0),
      )
      .optional()?;
    let current = expiry
      .as_deref()
      .map(parse_stored_timestamp)
      .transpose()?
      .is_some_and(|expires_at| expires_at > now);
    if !current {
      expire_lock_owner(&transaction, &held.key, now)?;
      grant_oldest_lock_waiter(&transaction, &held.key, now)?;
    }
    transaction.commit()?;
    Ok(current)
  }

  pub fn timeout_lock_waiter_atomically(
    &mut self,
    run_id: &str,
    lock_id: &str,
    resolved_at: DateTime<Utc>,
  ) -> Result<RunProjection, DurableStoreError> {
    self.timeout_lock_waiter_atomically_scoped(run_id, lock_id, None, resolved_at)
  }

  pub fn timeout_lock_waiter_atomically_scoped(
    &mut self,
    run_id: &str,
    lock_id: &str,
    iteration: Option<ForEachIterationScope>,
    resolved_at: DateTime<Utc>,
  ) -> Result<RunProjection, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let mut events = load_events(&transaction, run_id)?;
    let projection = fold_events(&events)?;
    let waiting = iteration
      .as_ref()
      .and_then(|scope| projection.for_each.get(&scope.for_each_id))
      .and_then(|loop_| loop_.executions.get(&iteration.as_ref()?.index))
      .and_then(|execution| execution.locks.get(lock_id))
      .or_else(|| {
        iteration
          .is_none()
          .then(|| projection.locks.get(lock_id))
          .flatten()
      })
      .ok_or_else(|| {
        DurableStoreError::LockHistoryInvalid("timeout has no projected lock waiter".to_string())
      })?;
    if waiting.status == crate::LockStatus::AcquireTimedOut {
      return Ok(projection);
    }
    if waiting.status != crate::LockStatus::Waiting || resolved_at < waiting.deadline {
      return Err(DurableStoreError::LockHistoryInvalid(
        "lock timeout requires an expired waiting acquisition".to_string(),
      ));
    }
    if transaction.execute(
      "UPDATE woml_lock_waiters SET status = 'timed_out'
       WHERE run_id = ?1 AND node_execution_id = ?2 AND status = 'waiting'",
      params![run_id, waiting.execution_identity],
    )? != 1
    {
      return Err(DurableStoreError::LockHistoryInvalid(
        "durable waiter changed before timeout settlement".to_string(),
      ));
    }
    let schema = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| {
        DurableStoreError::LockHistoryInvalid("timeout history is empty".to_string())
      })?;
    append_to_history_scoped(
      &transaction,
      &mut events,
      run_id,
      generated_event_id(),
      resolved_at,
      schema,
      iteration,
      RunEventPayload::LockAcquireTimedOut(crate::LockAcquireTimedOutData {
        lock_id: lock_id.to_string(),
        key: waiting.key.clone(),
        deadline: waiting.deadline,
        resolved_at,
      }),
    )?;
    grant_oldest_lock_waiter(&transaction, &waiting.key, resolved_at)?;
    let projection = fold_events(&events)?;
    transaction.commit()?;
    Ok(projection)
  }

  /// Removes durable waiters during cancellation. Held leases are
  /// intentionally not released here: cancellation history alone does not
  /// prove that supervised protected work has stopped, so an owner that
  /// cannot complete normal release must age out through lease expiry.
  pub fn cancel_lock_waiters_atomically(
    &mut self,
    run_id: &str,
    cancellation_id: &str,
    cancelled_at: DateTime<Utc>,
  ) -> Result<RunProjection, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let mut events = load_events(&transaction, run_id)?;
    let initial = fold_events(&events)?;
    if initial.status != RunStatus::Cancelling
      || initial.cancellation_request_id.as_deref() != Some(cancellation_id)
    {
      return Err(DurableStoreError::LockHistoryInvalid(
        "lock waiter cancellation requires the matching durable run cancellation".to_string(),
      ));
    }
    let schema = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| DurableStoreError::LockHistoryInvalid("lock history is empty".to_string()))?;
    let mut waiting = initial
      .locks
      .values()
      .filter(|lock| lock.status == crate::LockStatus::Waiting)
      .cloned()
      .map(|lock| (lock, None))
      .collect::<Vec<_>>();
    waiting.extend(
      initial
        .for_each
        .iter()
        .flat_map(|(for_each_id, loop_state)| {
          loop_state
            .executions
            .iter()
            .flat_map(move |(index, execution)| {
              execution
                .locks
                .values()
                .filter(|lock| lock.status == crate::LockStatus::Waiting)
                .cloned()
                .map(move |lock| {
                  (
                    lock,
                    Some(ForEachIterationScope {
                      for_each_id: for_each_id.clone(),
                      index: *index,
                    }),
                  )
                })
            })
        }),
    );
    for (lock, iteration) in waiting {
      if transaction.execute(
        "UPDATE woml_lock_waiters SET status = 'cancelled'
         WHERE run_id = ?1 AND node_execution_id = ?2 AND status = 'waiting'",
        params![run_id, lock.execution_identity],
      )? != 1
      {
        return Err(DurableStoreError::LockHistoryInvalid(
          "lock waiter changed before cancellation settlement".to_string(),
        ));
      }
      append_to_history_scoped(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        cancelled_at,
        schema,
        iteration,
        RunEventPayload::LockAcquisitionCancelled(crate::LockAcquisitionCancelledData {
          lock_id: lock.lock_id,
          key: lock.key.clone(),
          cancelled_at,
          cancellation_id: cancellation_id.to_string(),
        }),
      )?;
      grant_oldest_lock_waiter(&transaction, &lock.key, cancelled_at)?;
    }
    let projection = fold_events(&events)?;
    transaction.commit()?;
    Ok(projection)
  }
}
