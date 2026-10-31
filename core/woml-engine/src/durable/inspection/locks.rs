//! Safe lock/lease inspection and aggregate health, not lease ownership grants.

use crate::durable::{
  ensure_run_exists, finite_duration_ms, load_events, parse_stored_timestamp,
  programmatic_lock_session_ref, DurableEventStore, DurableStoreError, InspectedLockStatusV1,
  LockInspectionV1, LockObservationV1,
};
use crate::RunEventPayload;
use chrono::DateTime;
use chrono::Utc;
use serde_json::Value;
use std::collections::HashMap;

impl DurableEventStore {
  /// Returns at most 100 redacted lock records for one run. Each item exactly
  /// matches the frozen Lock Inspection v1 schema; internal lease IDs,
  /// execution identities, digests, payloads, and runtime-owner data never
  /// cross this boundary.
  pub fn inspect_locks_v1(&self, run_id: &str) -> Result<Vec<LockInspectionV1>, DurableStoreError> {
    ensure_run_exists(&self.connection, run_id)?;
    let projection = self.projection(run_id)?;
    #[derive(Default)]
    struct ProgrammaticLockHistory {
      acquired_at: Option<DateTime<Utc>>,
      released_at: Option<DateTime<Utc>>,
      resolved_at: Option<DateTime<Utc>>,
      fencing_token: Option<String>,
      waited_ms: Option<u64>,
    }
    let mut programmatic_history = HashMap::<String, ProgrammaticLockHistory>::new();
    for event in load_events(&self.connection, run_id)? {
      match &event.payload {
        RunEventPayload::OperationSucceeded(operation) if operation.capability == "locks" => {
          let Some(session_ref) = operation.metadata.get("sessionRef").and_then(Value::as_str)
          else {
            continue;
          };
          let history = programmatic_history
            .entry(session_ref.to_string())
            .or_default();
          match operation.operation.as_str() {
            "acquire" => {
              history.acquired_at = operation
                .metadata
                .get("acquiredAt")
                .and_then(Value::as_str)
                .map(parse_stored_timestamp)
                .transpose()?
                .or(Some(event.occurred_at));
              history.fencing_token = operation
                .metadata
                .get("fence")
                .and_then(Value::as_str)
                .map(str::to_string);
              history.waited_ms = finite_duration_ms(operation.duration_ms);
            }
            "release" => {
              history.released_at = operation
                .metadata
                .get("releasedAt")
                .and_then(Value::as_str)
                .map(parse_stored_timestamp)
                .transpose()?
                .or(Some(event.occurred_at));
              history.resolved_at = history.released_at;
            }
            _ => {}
          }
        }
        RunEventPayload::OperationFailed(operation) if operation.capability == "locks" => {
          if let Some(session_ref) = operation.metadata.get("sessionRef").and_then(Value::as_str) {
            programmatic_history
              .entry(session_ref.to_string())
              .or_default()
              .resolved_at = Some(event.occurred_at);
          }
        }
        _ => {}
      }
    }
    let now = Utc::now();
    let mut projected = HashMap::new();
    for lock in projection.locks.values() {
      projected.insert(lock.execution_identity.clone(), lock.clone());
    }
    for loop_state in projection.for_each.values() {
      for execution in loop_state.executions.values() {
        for lock in execution.locks.values() {
          projected.insert(lock.execution_identity.clone(), lock.clone());
        }
      }
    }
    let mut statement = self.connection.prepare(
      "SELECT waiters.key, waiters.node_execution_id, waiters.registered_at,
              waiters.deadline, waiters.status, resources.acquired_at,
              resources.expires_at, CAST(resources.owner_fencing_token AS TEXT)
       FROM woml_lock_waiters AS waiters
       LEFT JOIN woml_lock_resources AS resources
         ON resources.key = waiters.key
        AND resources.owner_run_id = waiters.run_id
        AND resources.owner_node_execution_id = waiters.node_execution_id
       WHERE waiters.run_id = ?1
       ORDER BY waiters.registered_at, waiters.sequence, waiters.waiter_id
       LIMIT 100",
    )?;
    let rows = statement
      .query_map([run_id], |row| {
        Ok((
          row.get::<_, String>(0)?,
          row.get::<_, String>(1)?,
          row.get::<_, String>(2)?,
          row.get::<_, String>(3)?,
          row.get::<_, String>(4)?,
          row.get::<_, Option<String>>(5)?,
          row.get::<_, Option<String>>(6)?,
          row.get::<_, Option<String>>(7)?,
        ))
      })?
      .collect::<Result<Vec<_>, _>>()?;
    let mut locks = Vec::with_capacity(rows.len());
    for (
      key,
      execution_identity,
      registered_at,
      deadline,
      stored_status,
      stored_acquired_at,
      stored_expires_at,
      stored_fencing_token,
    ) in rows
    {
      let registered_at = parse_stored_timestamp(&registered_at)?;
      let deadline = parse_stored_timestamp(&deadline)?;
      let projected_lock = projected.get(&execution_identity);
      let capability_history = programmatic_lock_session_ref(&execution_identity)
        .and_then(|session_ref| programmatic_history.get(&session_ref));
      let acquired_at = stored_acquired_at
        .as_deref()
        .map(parse_stored_timestamp)
        .transpose()?
        .or_else(|| projected_lock.and_then(|lock| lock.acquired_at))
        .or_else(|| capability_history.and_then(|history| history.acquired_at));
      let expires_at = stored_expires_at
        .as_deref()
        .map(parse_stored_timestamp)
        .transpose()?
        .or_else(|| projected_lock.and_then(|lock| lock.expires_at));
      let released_at = projected_lock
        .and_then(|lock| lock.resolved_at)
        .or_else(|| capability_history.and_then(|history| history.released_at));
      let mut status = match stored_status.as_str() {
        "waiting" => InspectedLockStatusV1::Waiting,
        "held" => InspectedLockStatusV1::Held,
        "released" => InspectedLockStatusV1::Released,
        "timed_out" => InspectedLockStatusV1::TimedOut,
        "cancelled" => InspectedLockStatusV1::Cancelled,
        "lost" => InspectedLockStatusV1::Lost,
        _ => {
          return Err(DurableStoreError::LockHistoryInvalid(
            "lock inspection found an unsupported stored status".to_string(),
          ))
        }
      };
      if status == InspectedLockStatusV1::Held && expires_at.is_some_and(|expiry| expiry <= now) {
        status = InspectedLockStatusV1::Lost;
      }
      let waited_ms = capability_history
        .and_then(|history| history.waited_ms)
        .or_else(|| {
          let waited_until = match status {
            InspectedLockStatusV1::Waiting => Some(now),
            InspectedLockStatusV1::Held | InspectedLockStatusV1::Released => acquired_at,
            InspectedLockStatusV1::TimedOut
            | InspectedLockStatusV1::Cancelled
            | InspectedLockStatusV1::Lost => capability_history
              .and_then(|history| history.resolved_at)
              .or(released_at),
          }?;
          u64::try_from(
            waited_until
              .signed_duration_since(registered_at)
              .num_milliseconds()
              .max(0),
          )
          .ok()
        });
      let held_ms = acquired_at.and_then(|acquired| {
        let held_until = released_at
          .or_else(|| capability_history.and_then(|history| history.resolved_at))
          .or_else(|| (status == InspectedLockStatusV1::Held).then_some(now))?;
        u64::try_from(
          held_until
            .signed_duration_since(acquired)
            .num_milliseconds()
            .max(0),
        )
        .ok()
      });
      let failure_code = match status {
        InspectedLockStatusV1::TimedOut => Some("WOML_LOCK_ACQUIRE_TIMED_OUT".to_string()),
        InspectedLockStatusV1::Cancelled => Some("WOML_LOCK_ACQUISITION_CANCELLED".to_string()),
        InspectedLockStatusV1::Lost => Some("WOML_LOCK_LEASE_LOST".to_string()),
        _ => None,
      };
      locks.push(LockInspectionV1 {
        profile: "woml.lock-inspection/v1",
        run_id: run_id.to_string(),
        lock_id: projected_lock
          .map(|lock| lock.lock_id.clone())
          .unwrap_or_else(|| "services.locks.withLease".to_string()),
        key,
        status,
        registered_at,
        acquired_at,
        released_at,
        deadline: Some(deadline),
        expires_at,
        fencing_token: stored_fencing_token
          .or_else(|| projected_lock.and_then(|lock| lock.fencing_token.clone()))
          .or_else(|| capability_history.and_then(|history| history.fencing_token.clone())),
        waited_ms,
        held_ms,
        failure_code,
      });
    }
    Ok(locks)
  }

  pub fn lock_observation_v1(&self) -> Result<LockObservationV1, DurableStoreError> {
    let now = Utc::now();
    let held_locks = self.connection.query_row(
      "SELECT COUNT(*) FROM woml_lock_resources WHERE owner_lease_id IS NOT NULL AND expires_at > ?1",
      [now.to_rfc3339()],
      |row| row.get(0),
    )?;
    let waiting_acquisitions = self.connection.query_row(
      "SELECT COUNT(*) FROM woml_lock_waiters WHERE status = 'waiting'",
      [],
      |row| row.get(0),
    )?;
    let oldest_waiter: Option<String> = self.connection.query_row(
      "SELECT MIN(registered_at) FROM woml_lock_waiters WHERE status = 'waiting'",
      [],
      |row| row.get(0),
    )?;
    let oldest_waiter_age_ms = oldest_waiter
      .as_deref()
      .map(parse_stored_timestamp)
      .transpose()?
      .and_then(|registered_at| {
        u64::try_from(
          now
            .signed_duration_since(registered_at)
            .num_milliseconds()
            .max(0),
        )
        .ok()
      });
    let count = |status: &str| -> Result<u64, DurableStoreError> {
      Ok(self.connection.query_row(
        "SELECT COUNT(*) FROM woml_lock_waiters WHERE status = ?1",
        [status],
        |row| row.get(0),
      )?)
    };
    let renewal_failures = count("lost")?;
    let expired_takeovers = self.connection.query_row(
      "SELECT COUNT(*) FROM woml_lock_waiters AS lost
       WHERE lost.status = 'lost'
         AND EXISTS (
           SELECT 1 FROM woml_lock_waiters AS successor
           WHERE successor.key = lost.key
             AND successor.sequence > lost.sequence
             AND successor.status IN ('held', 'released', 'lost')
         )",
      [],
      |row| row.get(0),
    )?;
    Ok(LockObservationV1 {
      profile: "woml.lock-observation/v1",
      held_locks,
      waiting_acquisitions,
      oldest_waiter_age_ms,
      renewal_failures,
      expired_takeovers,
      acquisition_timeouts: count("timed_out")?,
    })
  }
}
