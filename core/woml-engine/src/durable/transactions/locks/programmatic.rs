//! Callback-scoped lease requests sharing markup lock resources and fencing. Capability operation history remains their attempt authority.

use crate::durable::definition_for_run;
use crate::durable::ensure_run_exists;
use crate::durable::load_events;
use crate::durable::DurableStoreError;
use crate::durable::LockLeaseMaintenance;
use crate::durable::ProgrammaticLockAcquire;
use crate::durable::ProgrammaticLockAcquireState;
use crate::durable::ProgrammaticLockLease;
use crate::durable::{
  ensure_lock_wait_capacity, expire_lock_owner, grant_oldest_lock_waiter,
  lock_descriptor_for_protected_node, parse_stored_timestamp, valid_lock_key, DurableEventStore,
};
use crate::fold_events;
use crate::projection::AttemptStatus;
use chrono::DateTime;
use chrono::Utc;
use rusqlite::params;
use rusqlite::OptionalExtension;
use rusqlite::Transaction;
use rusqlite::TransactionBehavior;
use sha2::Digest;
use sha2::Sha256;

const PROGRAMMATIC_LOCK_IDENTITY_PREFIX: &str = "capability:";

fn validate_programmatic_lock_input(
  node_id: &str,
  invocation_id: &str,
  session_id: &str,
  key: &str,
  acquire_timeout_ms: u64,
  lease_ms: u64,
) -> Result<(), DurableStoreError> {
  if node_id.is_empty()
    || invocation_id.is_empty()
    || session_id.is_empty()
    || session_id.len() > 128
    || !session_id
      .bytes()
      .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    || !valid_lock_key(key)
    || !(1_000..=604_800_000).contains(&acquire_timeout_ms)
    || !(5_000..=600_000).contains(&lease_ms)
  {
    return Err(DurableStoreError::Contract(
      "Programmatic lock acquisition input is invalid.".to_string(),
    ));
  }
  Ok(())
}

fn programmatic_lock_execution_identity(session_id: &str, lease_ms: u64) -> String {
  format!("{PROGRAMMATIC_LOCK_IDENTITY_PREFIX}{lease_ms}:{session_id}")
}

pub(in crate::durable) fn programmatic_lock_lease_ms(execution_identity: &str) -> Option<u64> {
  let remainder = execution_identity.strip_prefix(PROGRAMMATIC_LOCK_IDENTITY_PREFIX)?;
  let (lease_ms, session_id) = remainder.split_once(':')?;
  (!session_id.is_empty())
    .then(|| lease_ms.parse::<u64>().ok())
    .flatten()
    .filter(|lease_ms| (5_000..=600_000).contains(lease_ms))
}

pub(in crate::durable) fn programmatic_lock_session_ref(
  execution_identity: &str,
) -> Option<String> {
  let remainder = execution_identity.strip_prefix(PROGRAMMATIC_LOCK_IDENTITY_PREFIX)?;
  let (_, session_id) = remainder.split_once(':')?;
  (!session_id.is_empty()).then(|| {
    format!(
      "sha256:{}",
      hex::encode(Sha256::digest(session_id.as_bytes()))
    )
  })
}

pub(in crate::durable) fn finite_duration_ms(duration_ms: f64) -> Option<u64> {
  (duration_ms.is_finite() && duration_ms >= 0.0 && duration_ms <= u64::MAX as f64)
    .then(|| duration_ms.round() as u64)
}

fn programmatic_lock_state_in_transaction(
  transaction: &Transaction<'_>,
  run_id: &str,
  session_id: &str,
  lease_ms: u64,
  now: DateTime<Utc>,
) -> Result<ProgrammaticLockAcquire, DurableStoreError> {
  let identity = programmatic_lock_execution_identity(session_id, lease_ms);
  let waiter: Option<(String, String)> = transaction
    .query_row(
      "SELECT key, status FROM woml_lock_waiters WHERE run_id = ?1 AND node_execution_id = ?2",
      params![run_id, identity],
      |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()?;
  let Some((key, status)) = waiter else {
    return Err(DurableStoreError::LockHistoryInvalid(
      "programmatic lease session does not exist".to_string(),
    ));
  };
  let state = match status.as_str() {
    "waiting" => ProgrammaticLockAcquireState::Waiting,
    "timed_out" => ProgrammaticLockAcquireState::TimedOut,
    "cancelled" => ProgrammaticLockAcquireState::Cancelled,
    "lost" => ProgrammaticLockAcquireState::Lost,
    "released" => ProgrammaticLockAcquireState::Lost,
    "held" => {
      let owner: Option<(String, String, String, String)> = transaction
        .query_row(
          "SELECT owner_lease_id, CAST(owner_fencing_token AS TEXT), acquired_at, expires_at
           FROM woml_lock_resources
           WHERE key = ?1 AND owner_run_id = ?2 AND owner_node_execution_id = ?3",
          params![key, run_id, identity],
          |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
      let Some((_lease_id, fencing_token, acquired_at, expires_at)) = owner else {
        return Ok(ProgrammaticLockAcquire {
          state: ProgrammaticLockAcquireState::Lost,
          lease: None,
        });
      };
      let expires_at = parse_stored_timestamp(&expires_at)?;
      if expires_at <= now {
        ProgrammaticLockAcquireState::Lost
      } else {
        return Ok(ProgrammaticLockAcquire {
          state: ProgrammaticLockAcquireState::Held,
          lease: Some(ProgrammaticLockLease {
            session_id: session_id.to_string(),
            key,
            fencing_token,
            acquired_at: parse_stored_timestamp(&acquired_at)?,
            expires_at,
          }),
        });
      }
    }
    _ => {
      return Err(DurableStoreError::LockHistoryInvalid(
        "programmatic lease has an unsupported stored state".to_string(),
      ))
    }
  };
  Ok(ProgrammaticLockAcquire { state, lease: None })
}

impl DurableEventStore {
  /// Registers or reattaches one callback-scoped lease request. Programmatic
  /// sessions share the exact same resource row, fair queue, and fencing
  /// high-water as markup locks, but they deliberately do not append workflow
  /// control-flow events: the surrounding capability operation is their
  /// durable attempt record.
  #[allow(clippy::too_many_arguments)]
  pub fn acquire_programmatic_lock_atomically(
    &mut self,
    run_id: &str,
    node_id: &str,
    invocation_id: &str,
    session_id: &str,
    key: &str,
    acquire_timeout_ms: u64,
    lease_ms: u64,
    registered_at: DateTime<Utc>,
  ) -> Result<ProgrammaticLockAcquire, DurableStoreError> {
    validate_programmatic_lock_input(
      node_id,
      invocation_id,
      session_id,
      key,
      acquire_timeout_ms,
      lease_ms,
    )?;
    let deadline = registered_at
      + chrono::Duration::milliseconds(i64::try_from(acquire_timeout_ms).map_err(|_| {
        DurableStoreError::Contract(
          "Lock acquisition timeout exceeds the runtime clock.".to_string(),
        )
      })?);
    let execution_identity = programmatic_lock_execution_identity(session_id, lease_ms);
    let input_digest = format!(
      "sha256:{:x}",
      Sha256::digest(
        format!("{key}\0{node_id}\0{invocation_id}\0{acquire_timeout_ms}\0{lease_ms}").as_bytes()
      )
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
    if lock_descriptor_for_protected_node(&workflow, node_id).is_some() {
      return Err(DurableStoreError::Contract(
        "services.locks.withLease() cannot run inside a markup <lock> scope.".to_string(),
      ));
    }
    let projection = fold_events(&load_events(&transaction, run_id)?)?;
    let active = projection.attempts.iter().any(|attempt| {
      attempt.identity.node_id == node_id
        && attempt.identity.invocation_id == invocation_id
        && attempt.status == AttemptStatus::Started
    }) || projection.for_each.values().any(|loop_state| {
      loop_state.executions.values().any(|execution| {
        execution.attempts.iter().any(|attempt| {
          attempt.identity.node_id == node_id
            && attempt.identity.invocation_id == invocation_id
            && attempt.status == AttemptStatus::Started
        })
      })
    });
    if !active {
      return Err(DurableStoreError::Contract(
        "Programmatic lock acquisition requires its exact active script attempt.".to_string(),
      ));
    }
    let existing: Option<(String, String)> = transaction
      .query_row(
        "SELECT key, input_digest FROM woml_lock_waiters WHERE run_id = ?1 AND node_execution_id = ?2",
        params![run_id, execution_identity],
        |row| Ok((row.get(0)?, row.get(1)?)),
      )
      .optional()?;
    if let Some((stored_key, stored_digest)) = existing {
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
    }
    grant_oldest_lock_waiter(&transaction, key, registered_at)?;
    let state = programmatic_lock_state_in_transaction(
      &transaction,
      run_id,
      session_id,
      lease_ms,
      registered_at,
    )?;
    transaction.commit()?;
    Ok(state)
  }

  pub fn programmatic_lock_state(
    &mut self,
    run_id: &str,
    session_id: &str,
    lease_ms: u64,
    now: DateTime<Utc>,
  ) -> Result<ProgrammaticLockAcquire, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let identity = programmatic_lock_execution_identity(session_id, lease_ms);
    let waiting: Option<(String, String)> = transaction
      .query_row(
        "SELECT key, deadline FROM woml_lock_waiters WHERE run_id = ?1 AND node_execution_id = ?2",
        params![run_id, identity],
        |row| Ok((row.get(0)?, row.get(1)?)),
      )
      .optional()?;
    if let Some((key, deadline)) = &waiting {
      grant_oldest_lock_waiter(&transaction, key, now)?;
      let status: Option<String> = transaction
        .query_row(
          "SELECT status FROM woml_lock_waiters WHERE run_id = ?1 AND node_execution_id = ?2",
          params![run_id, identity],
          |row| row.get(0),
        )
        .optional()?;
      if status.as_deref() == Some("waiting") && parse_stored_timestamp(deadline)? <= now {
        transaction.execute(
          "UPDATE woml_lock_waiters SET status = 'timed_out'
           WHERE run_id = ?1 AND node_execution_id = ?2 AND status = 'waiting'",
          params![run_id, identity],
        )?;
        grant_oldest_lock_waiter(&transaction, key, now)?;
      }
    }
    let state =
      programmatic_lock_state_in_transaction(&transaction, run_id, session_id, lease_ms, now)?;
    transaction.commit()?;
    Ok(state)
  }

  pub fn renew_programmatic_lock_atomically(
    &mut self,
    run_id: &str,
    session_id: &str,
    lease_ms: u64,
    now: DateTime<Utc>,
  ) -> Result<LockLeaseMaintenance, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let identity = programmatic_lock_execution_identity(session_id, lease_ms);
    let owner: Option<(String, String, String)> = transaction
      .query_row(
        "SELECT resources.key, resources.owner_lease_id, resources.expires_at
         FROM woml_lock_resources AS resources
         WHERE resources.owner_run_id = ?1 AND resources.owner_node_execution_id = ?2",
        params![run_id, identity],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
      )
      .optional()?;
    let Some((key, lease_id, expires_at)) = owner else {
      transaction.commit()?;
      return Ok(LockLeaseMaintenance::NotHeld);
    };
    let stored_expiry = parse_stored_timestamp(&expires_at)?;
    if stored_expiry <= now {
      expire_lock_owner(&transaction, &key, now)?;
      grant_oldest_lock_waiter(&transaction, &key, now)?;
      transaction.commit()?;
      return Ok(LockLeaseMaintenance::Lost);
    }
    let requested_expiry = now
      + chrono::Duration::milliseconds(i64::try_from(lease_ms).map_err(|_| {
        DurableStoreError::Contract("Lock lease exceeds the runtime clock.".to_string())
      })?);
    let next_expiry = requested_expiry.max(stored_expiry);
    let changed = transaction.execute(
      "UPDATE woml_lock_resources SET expires_at = ?1, updated_at = ?2
       WHERE key = ?3 AND owner_run_id = ?4 AND owner_node_execution_id = ?5
         AND owner_lease_id = ?6 AND expires_at = ?7",
      params![
        next_expiry.to_rfc3339(),
        now.to_rfc3339(),
        key,
        run_id,
        identity,
        lease_id,
        expires_at
      ],
    )?;
    transaction.commit()?;
    Ok(if changed == 1 {
      LockLeaseMaintenance::Renewed
    } else {
      LockLeaseMaintenance::Lost
    })
  }

  pub fn release_programmatic_lock_atomically(
    &mut self,
    run_id: &str,
    session_id: &str,
    lease_ms: u64,
    released_at: DateTime<Utc>,
  ) -> Result<bool, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let identity = programmatic_lock_execution_identity(session_id, lease_ms);
    let key: Option<String> = transaction
      .query_row(
        "SELECT key FROM woml_lock_waiters WHERE run_id = ?1 AND node_execution_id = ?2",
        params![run_id, identity],
        |row| row.get(0),
      )
      .optional()?;
    let Some(key) = key else {
      transaction.commit()?;
      return Ok(false);
    };
    grant_oldest_lock_waiter(&transaction, &key, released_at)?;
    let changed = transaction.execute(
      "UPDATE woml_lock_resources SET owner_lease_id = NULL, owner_run_id = NULL,
         owner_node_execution_id = NULL, owner_fencing_token = NULL, acquired_at = NULL,
         expires_at = NULL, updated_at = ?1
       WHERE key = ?2 AND owner_run_id = ?3 AND owner_node_execution_id = ?4",
      params![released_at.to_rfc3339(), key, run_id, identity],
    )?;
    if changed == 1 {
      transaction.execute(
        "UPDATE woml_lock_waiters SET status = 'released'
         WHERE run_id = ?1 AND node_execution_id = ?2 AND status = 'held'",
        params![run_id, identity],
      )?;
      grant_oldest_lock_waiter(&transaction, &key, released_at)?;
    }
    transaction.commit()?;
    Ok(changed == 1)
  }

  pub fn cancel_programmatic_lock_waiter_atomically(
    &mut self,
    run_id: &str,
    session_id: &str,
    lease_ms: u64,
    cancelled_at: DateTime<Utc>,
  ) -> Result<bool, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let identity = programmatic_lock_execution_identity(session_id, lease_ms);
    let key: Option<String> = transaction
      .query_row(
        "SELECT key FROM woml_lock_waiters WHERE run_id = ?1 AND node_execution_id = ?2",
        params![run_id, identity],
        |row| row.get(0),
      )
      .optional()?;
    let changed = transaction.execute(
      "UPDATE woml_lock_waiters SET status = 'cancelled'
       WHERE run_id = ?1 AND node_execution_id = ?2 AND status = 'waiting'",
      params![run_id, identity],
    )?;
    if changed == 1 {
      if let Some(key) = key {
        grant_oldest_lock_waiter(&transaction, &key, cancelled_at)?;
      }
    }
    transaction.commit()?;
    Ok(changed == 1)
  }
}
