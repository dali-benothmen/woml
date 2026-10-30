//! Caller-transaction lock queues, expiry, and successor grant. These helpers can affect multiple runs and must never open or commit nested independent transactions.

use crate::durable::append_to_history_scoped;
use crate::durable::definition_for_run;
use crate::durable::ensure_run_exists;
use crate::durable::generated_event_id;
use crate::durable::load_events;
use crate::durable::DurableStoreError;
use crate::durable::LOCK_WAITERS_PER_KEY_LIMIT;
use crate::durable::LOCK_WAITERS_PER_STORE_LIMIT;
use crate::durable::{
  parse_stored_timestamp, programmatic_lock_lease_ms, resolve_lock_execution, valid_lock_key,
  DurableEventStore,
};
use crate::event::ForEachIterationScope;
use crate::fold_events;
use crate::RunEventPayload;
use crate::RunProjection;
use chrono::DateTime;
use chrono::Utc;
use rusqlite::params;
use rusqlite::OptionalExtension;
use rusqlite::Transaction;
use rusqlite::TransactionBehavior;
use sha2::Digest;
use sha2::Sha256;
use uuid::Uuid;

pub(in crate::durable) fn ensure_lock_wait_capacity(
  transaction: &Transaction<'_>,
  key: &str,
) -> Result<(), DurableStoreError> {
  let occupied: bool = transaction.query_row(
    "SELECT owner_lease_id IS NOT NULL OR EXISTS(
       SELECT 1 FROM woml_lock_waiters WHERE key = ?1 AND status = 'waiting'
     ) FROM woml_lock_resources WHERE key = ?1",
    [key],
    |row| row.get(0),
  )?;
  if !occupied {
    return Ok(());
  }
  let key_waiters: i64 = transaction.query_row(
    "SELECT COUNT(*) FROM woml_lock_waiters WHERE key = ?1 AND status = 'waiting'",
    [key],
    |row| row.get(0),
  )?;
  let store_waiters: i64 = transaction.query_row(
    "SELECT COUNT(*) FROM woml_lock_waiters WHERE status = 'waiting'",
    [],
    |row| row.get(0),
  )?;
  if key_waiters >= LOCK_WAITERS_PER_KEY_LIMIT as i64
    || store_waiters >= LOCK_WAITERS_PER_STORE_LIMIT as i64
  {
    return Err(DurableStoreError::LockWaitLimit);
  }
  Ok(())
}

pub(in crate::durable) fn grant_oldest_lock_waiter(
  transaction: &Transaction<'_>,
  key: &str,
  now: DateTime<Utc>,
) -> Result<Option<String>, DurableStoreError> {
  expire_lock_owner(transaction, key, now)?;
  let owner: Option<String> = transaction
    .query_row(
      "SELECT owner_lease_id FROM woml_lock_resources WHERE key = ?1",
      [key],
      |row| row.get(0),
    )
    .optional()?
    .flatten();
  if owner.is_some() {
    return Ok(None);
  }
  let selected: Option<(String, String, String)> = transaction
    .query_row(
      "SELECT waiter_id, run_id, node_execution_id FROM woml_lock_waiters
     WHERE key = ?1 AND status = 'waiting' AND deadline >= ?2
     ORDER BY sequence, waiter_id LIMIT 1",
      params![key, now.to_rfc3339()],
      |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .optional()?;
  let Some((waiter_id, run_id, execution_identity)) = selected else {
    return Ok(None);
  };
  let programmatic_lease_ms = programmatic_lock_lease_ms(&execution_identity);
  let (descriptor, iteration, lease_ms) = if let Some(lease_ms) = programmatic_lease_ms {
    (None, None, lease_ms)
  } else {
    let workflow = definition_for_run(transaction, &run_id)?;
    let projection = fold_events(&load_events(transaction, &run_id)?)?;
    let (descriptor, iteration) =
      resolve_lock_execution(&workflow, &projection, &execution_identity).ok_or_else(|| {
        DurableStoreError::LockHistoryInvalid(
          "waiting acquisition has no exact active Model v18 execution identity".to_string(),
        )
      })?;
    (
      Some(descriptor.lock_id.clone()),
      iteration,
      descriptor.lease_ms,
    )
  };
  let current_high_water: i64 = transaction.query_row(
    "SELECT fencing_high_water FROM woml_lock_resources WHERE key = ?1",
    [key],
    |row| row.get(0),
  )?;
  let next_token = current_high_water
    .checked_add(1)
    .ok_or(DurableStoreError::LockFencingExhausted)?;
  let lease_id = format!("lease_{}", Uuid::new_v4().simple());
  let expires_at = now
    + chrono::Duration::milliseconds(i64::try_from(lease_ms).map_err(|_| {
      DurableStoreError::Contract("Lock lease exceeds the runtime clock.".to_string())
    })?);
  let changed = transaction.execute(
    "UPDATE woml_lock_resources SET fencing_high_water = ?1, owner_lease_id = ?2,
     owner_run_id = ?3, owner_node_execution_id = ?4, owner_fencing_token = ?1,
     acquired_at = ?5, expires_at = ?6, updated_at = ?5
     WHERE key = ?7 AND owner_lease_id IS NULL AND fencing_high_water = ?8",
    params![
      next_token,
      lease_id,
      run_id,
      execution_identity,
      now.to_rfc3339(),
      expires_at.to_rfc3339(),
      key,
      current_high_water
    ],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::LockHistoryInvalid(
      "atomic lock grant lost ownership race".to_string(),
    ));
  }
  if transaction.execute(
    "UPDATE woml_lock_waiters SET status = 'held' WHERE waiter_id = ?1 AND status = 'waiting'",
    [&waiter_id],
  )? != 1
  {
    return Err(DurableStoreError::LockHistoryInvalid(
      "selected waiter changed before grant".to_string(),
    ));
  }
  if let Some(lock_id) = descriptor {
    let mut events = load_events(transaction, &run_id)?;
    let schema = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| {
        DurableStoreError::LockHistoryInvalid(
          "selected waiter run has no event history".to_string(),
        )
      })?;
    append_to_history_scoped(
      transaction,
      &mut events,
      &run_id,
      generated_event_id(),
      now,
      schema,
      iteration,
      RunEventPayload::LockAcquired(crate::LockAcquiredData {
        lock_id,
        key: key.to_string(),
        lease_id,
        fencing_token: next_token.to_string(),
        acquired_at: now,
        expires_at,
        execution_identity,
      }),
    )?;
  }
  Ok(Some(run_id))
}

pub(in crate::durable) fn expire_lock_owner(
  transaction: &Transaction<'_>,
  key: &str,
  now: DateTime<Utc>,
) -> Result<bool, DurableStoreError> {
  let owner: Option<(String, String, String, String, String)> = transaction
    .query_row(
      "SELECT owner_lease_id, owner_run_id, owner_node_execution_id,
              CAST(owner_fencing_token AS TEXT), expires_at
       FROM woml_lock_resources WHERE key = ?1 AND owner_lease_id IS NOT NULL",
      [key],
      |row| {
        Ok((
          row.get(0)?,
          row.get(1)?,
          row.get(2)?,
          row.get(3)?,
          row.get(4)?,
        ))
      },
    )
    .optional()?;
  let Some((lease_id, run_id, execution_identity, fencing_token, expires_at)) = owner else {
    return Ok(false);
  };
  let expired_at = parse_stored_timestamp(&expires_at)?;
  if expired_at > now {
    return Ok(false);
  }
  if programmatic_lock_lease_ms(&execution_identity).is_some() {
    if transaction.execute(
      "UPDATE woml_lock_resources SET owner_lease_id = NULL, owner_run_id = NULL,
         owner_node_execution_id = NULL, owner_fencing_token = NULL, acquired_at = NULL,
         expires_at = NULL, updated_at = ?1
       WHERE key = ?2 AND owner_run_id = ?3 AND owner_lease_id = ?4",
      params![now.to_rfc3339(), key, run_id, lease_id],
    )? != 1
    {
      return Err(DurableStoreError::LockHistoryInvalid(
        "expired programmatic owner changed before lease-loss settlement".to_string(),
      ));
    }
    transaction.execute(
      "UPDATE woml_lock_waiters SET status = 'lost'
       WHERE run_id = ?1 AND node_execution_id = ?2 AND status = 'held'",
      params![run_id, execution_identity],
    )?;
    return Ok(true);
  }
  let workflow = definition_for_run(transaction, &run_id)?;
  let mut events = load_events(transaction, &run_id)?;
  let projection = fold_events(&events)?;
  let (descriptor, iteration) = resolve_lock_execution(&workflow, &projection, &execution_identity)
    .ok_or_else(|| {
      DurableStoreError::LockHistoryInvalid(
        "expired owner has no exact active Model v18 execution identity".to_string(),
      )
    })?;
  let held = iteration
    .as_ref()
    .and_then(|scope| projection.for_each.get(&scope.for_each_id))
    .and_then(|loop_| loop_.executions.get(&iteration.as_ref()?.index))
    .and_then(|execution| execution.locks.get(&descriptor.lock_id))
    .or_else(|| {
      iteration
        .is_none()
        .then(|| projection.locks.get(&descriptor.lock_id))
        .flatten()
    })
    .ok_or_else(|| {
      DurableStoreError::LockHistoryInvalid("expired owner has no projected lock".to_string())
    })?;
  if held.status != crate::LockStatus::Held
    || held.key != key
    || held.lease_id.as_deref() != Some(lease_id.as_str())
    || held.fencing_token.as_deref() != Some(fencing_token.as_str())
  {
    return Err(DurableStoreError::LockHistoryInvalid(
      "expired durable owner differs from projected held lease".to_string(),
    ));
  }
  let active_effect = iteration.as_ref().map_or_else(
    || {
      projection
        .operations
        .values()
        .any(|operation| operation.status == crate::projection::OperationStatus::Started)
    },
    |scope| {
      projection
        .for_each
        .get(&scope.for_each_id)
        .and_then(|loop_| loop_.executions.get(&scope.index))
        .is_some_and(|execution| {
          execution
            .operations
            .values()
            .any(|operation| operation.status == crate::projection::OperationStatus::Started)
        })
    },
  );
  let schema = events
    .first()
    .map(|event| event.event_schema_version)
    .ok_or_else(|| DurableStoreError::LockHistoryInvalid("owner history is empty".to_string()))?;
  append_to_history_scoped(
    transaction,
    &mut events,
    &run_id,
    generated_event_id(),
    now,
    schema,
    iteration,
    RunEventPayload::LockLeaseLost(crate::LockLeaseLostData {
      lock_id: descriptor.lock_id.clone(),
      key: key.to_string(),
      lease_id: lease_id.clone(),
      fencing_token,
      expired_at,
      detected_at: now,
      ambiguous: active_effect,
    }),
  )?;
  if transaction.execute(
    "UPDATE woml_lock_resources SET owner_lease_id = NULL, owner_run_id = NULL,
       owner_node_execution_id = NULL, owner_fencing_token = NULL, acquired_at = NULL,
       expires_at = NULL, updated_at = ?1
     WHERE key = ?2 AND owner_run_id = ?3 AND owner_lease_id = ?4",
    params![now.to_rfc3339(), key, run_id, lease_id],
  )? != 1
  {
    return Err(DurableStoreError::LockHistoryInvalid(
      "expired owner changed before lease-loss settlement".to_string(),
    ));
  }
  transaction.execute(
    "UPDATE woml_lock_waiters SET status = 'lost'
     WHERE run_id = ?1 AND node_execution_id = ?2 AND status = 'held'",
    params![run_id, execution_identity],
  )?;
  Ok(true)
}

impl DurableEventStore {
  pub fn register_lock_waiter_atomically(
    &mut self,
    run_id: &str,
    lock_id: &str,
    key: &str,
    execution_identity: &str,
    registered_at: DateTime<Utc>,
    deadline: DateTime<Utc>,
  ) -> Result<RunProjection, DurableStoreError> {
    self.register_lock_waiter_atomically_scoped(
      run_id,
      lock_id,
      key,
      execution_identity,
      None,
      registered_at,
      deadline,
    )
  }

  #[allow(clippy::too_many_arguments)]
  pub fn register_lock_waiter_atomically_scoped(
    &mut self,
    run_id: &str,
    lock_id: &str,
    key: &str,
    execution_identity: &str,
    iteration: Option<ForEachIterationScope>,
    registered_at: DateTime<Utc>,
    deadline: DateTime<Utc>,
  ) -> Result<RunProjection, DurableStoreError> {
    if !valid_lock_key(key) || execution_identity.is_empty() || deadline < registered_at {
      return Err(DurableStoreError::Contract(
        "Lock acquisition input is invalid.".to_string(),
      ));
    }
    let input_digest = format!(
      "sha256:{:x}",
      Sha256::digest(format!("{key}\0{}", deadline.to_rfc3339()).as_bytes())
    );
    let waiter_id = format!(
      "waiter_{:x}",
      Sha256::digest(format!("{run_id}\0{execution_identity}").as_bytes())
    );
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let descriptor = workflow.durable_lock(lock_id).ok_or_else(|| {
      DurableStoreError::Contract(
        "Lock acquisition references an unknown Model v18 lock.".to_string(),
      )
    })?;
    let expected_execution_identity = iteration.as_ref().map_or_else(
      || descriptor.acquire_node_id.clone(),
      |scope| {
        format!(
          "{}:{}:{}",
          scope.for_each_id, scope.index, descriptor.acquire_node_id
        )
      },
    );
    if expected_execution_identity != execution_identity {
      return Err(DurableStoreError::Contract(
        "Lock execution identity does not match the compiled acquire node.".to_string(),
      ));
    }
    let mut events = load_events(&transaction, run_id)?;
    let event_schema_version = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| {
        DurableStoreError::Contract("A lock cannot be registered before run_started.".to_string())
      })?;
    if !matches!(
      event_schema_version,
      crate::RUN_EVENT_SCHEMA_VERSION_V17
        | crate::RUN_EVENT_SCHEMA_VERSION_V18
        | crate::RUN_EVENT_SCHEMA_VERSION_V19
        | crate::RUN_EVENT_SCHEMA_VERSION_V20
    ) {
      return Err(DurableStoreError::Contract(
        "Durable locks require Event v17.".to_string(),
      ));
    }
    let existing: Option<(String, String, String)> = transaction.query_row(
      "SELECT key, status, input_digest FROM woml_lock_waiters WHERE run_id = ?1 AND node_execution_id = ?2",
      params![run_id, execution_identity], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).optional()?;
    if let Some((stored_key, _, stored_digest)) = existing {
      if stored_key != key || stored_digest != input_digest {
        return Err(DurableStoreError::LockIdempotencyConflict);
      }
    } else {
      transaction.execute(
        "INSERT INTO woml_lock_resources(key, fencing_high_water, updated_at) VALUES (?1, 0, ?2)
         ON CONFLICT(key) DO NOTHING",
        params![key, registered_at.to_rfc3339()],
      )?;
      ensure_lock_wait_capacity(&transaction, key)?;
      let sequence: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(sequence), 0) + 1 FROM woml_lock_waiters WHERE key = ?1",
        [key],
        |row| row.get(0),
      )?;
      transaction.execute(
        "INSERT INTO woml_lock_waiters(waiter_id, key, run_id, node_execution_id, sequence, registered_at, deadline, status, input_digest)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'waiting', ?8)",
        params![waiter_id, key, run_id, execution_identity, sequence, registered_at.to_rfc3339(), deadline.to_rfc3339(), input_digest],
      )?;
      append_to_history_scoped(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        registered_at,
        event_schema_version,
        iteration,
        RunEventPayload::LockWaitRegistered(crate::LockWaitRegisteredData {
          lock_id: lock_id.to_string(),
          key: key.to_string(),
          waiter_sequence: u64::try_from(sequence).map_err(|_| {
            DurableStoreError::LockHistoryInvalid(
              "waiter sequence exceeded the public contract".to_string(),
            )
          })?,
          registered_at,
          deadline,
          execution_identity: execution_identity.to_string(),
        }),
      )?;
    }
    grant_oldest_lock_waiter(&transaction, key, registered_at)?;
    let projection = fold_events(&load_events(&transaction, run_id)?)?;
    transaction.commit()?;
    Ok(projection)
  }
}
