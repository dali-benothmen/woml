//! Durable queue capacity, rate eligibility, scheduler claims, and lease loss. Wakeups do not replace committed ownership checks.

use crate::durable::append_to_history;
use crate::durable::definition_for_run;
use crate::durable::expand_model_v11_payload;
use crate::durable::generated_event_id;
use crate::durable::load_events;
use crate::durable::run_binding_in_transaction;
use crate::durable::DurableStoreError;
use crate::durable::PolicyClaimWaitReason;
use crate::durable::PolicyExecutionClaimResult;
use crate::durable::SchedulerClaimV1;
use crate::durable::{parse_durable_timestamp, DurableEventStore, RunRecovery};
use crate::engine::validate_event_history_against_definition;
use crate::fold_events;
use crate::projection::AttemptStatus;
use crate::projection::ForEachStatus;
use crate::CompiledWorkflowDefinition;
use crate::RunEventPayload;
use crate::RunStatus;
use chrono::DateTime;
use chrono::Utc;
use rusqlite::params;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::TransactionBehavior;
use std::time::Duration;
use uuid::Uuid;

pub const RUNTIME_POLICY_QUEUE_CEILING: i64 = 10_000;

const SUSPENDED_POLICY_WORKFLOW_ID: &str = "__woml_policy_wait__";

fn workflow_has_policy_capacity(
  connection: &Connection,
  workflow_id: &str,
  concurrency: Option<u32>,
) -> Result<bool, DurableStoreError> {
  let Some(limit) = concurrency else {
    return Ok(true);
  };
  let active: i64 = connection.query_row(
    "SELECT COUNT(*) FROM woml_scheduler_claims WHERE workflow_id = ?1",
    [workflow_id],
    |row| row.get(0),
  )?;
  Ok(active < i64::from(limit))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PolicyRateEligibility {
  Eligible,
  Waiting { eligible_at: DateTime<Utc> },
}

fn workflow_rate_eligibility(
  connection: &Connection,
  workflow_id: &str,
  rate_limit: Option<&crate::model::CompiledRateLimitPolicy>,
  now: DateTime<Utc>,
) -> Result<PolicyRateEligibility, DurableStoreError> {
  let Some(rate_limit) = rate_limit else {
    return Ok(PolicyRateEligibility::Eligible);
  };
  let window =
    chrono::Duration::milliseconds(i64::try_from(rate_limit.window_ms).map_err(|_| {
      DurableStoreError::Contract("Rate-limit window exceeds the runtime clock range.".to_string())
    })?);
  let cutoff = now - window;
  let starts = {
    let mut statement = connection.prepare(
      "SELECT started_at FROM woml_runtime_policy_starts
       WHERE workflow_id = ?1 AND started_at > ?2 ORDER BY started_at",
    )?;
    let rows = statement
      .query_map(params![workflow_id, cutoff.to_rfc3339()], |row| {
        row.get::<_, String>(0)
      })?
      .collect::<Result<Vec<_>, _>>()?;
    rows
  };
  let mut active = starts
    .iter()
    .map(|value| parse_durable_timestamp(value, "runtime-policy start"))
    .collect::<Result<Vec<_>, _>>()?
    .into_iter()
    // Future starts are deliberately retained so a wall-clock rollback fails
    // closed instead of granting extra capacity.
    .collect::<Vec<_>>();
  active.sort_unstable();
  if active.len() < rate_limit.count as usize {
    return Ok(PolicyRateEligibility::Eligible);
  }
  Ok(PolicyRateEligibility::Waiting {
    eligible_at: active[active.len() - rate_limit.count as usize] + window,
  })
}

fn earlier_eligible_policy_run_exists(
  connection: &Connection,
  run_id: &str,
  queue: &str,
  admitted_at: DateTime<Utc>,
  occurrence_sequence: u64,
  now: DateTime<Utc>,
) -> Result<bool, DurableStoreError> {
  let occurrence_sequence = i64::try_from(occurrence_sequence).map_err(|_| {
    DurableStoreError::Contract("occurrenceSequence exceeds SQLite integer range".to_string())
  })?;
  let earlier = {
    let mut statement = connection.prepare(
      "SELECT run_id, workflow_id
       FROM woml_runtime_policy_queue
       WHERE queue_name = ?1 AND run_id != ?2
         AND (
           admitted_at < ?3
           OR (admitted_at = ?3 AND occurrence_sequence < ?4)
           OR (admitted_at = ?3 AND occurrence_sequence = ?4 AND run_id < ?2)
         )
       ORDER BY admitted_at, occurrence_sequence, run_id",
    )?;
    let rows = statement
      .query_map(
        params![queue, run_id, admitted_at.to_rfc3339(), occurrence_sequence],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
      )?
      .collect::<Result<Vec<_>, _>>()?;
    rows
  };
  for (earlier_run_id, workflow_id) in earlier {
    let definition_hash: String = connection.query_row(
      "SELECT definition_hash FROM woml_runtime_policy_bindings WHERE run_id = ?1",
      [&earlier_run_id],
      |row| row.get(0),
    )?;
    let model_json: String = connection.query_row(
      "SELECT model_json FROM woml_definitions WHERE definition_hash = ?1",
      [&definition_hash],
      |row| row.get(0),
    )?;
    let workflow: CompiledWorkflowDefinition = serde_json::from_str(&model_json)?;
    let policy = workflow.runtime_policy.as_ref().ok_or_else(|| {
      DurableStoreError::Contract("Queued policy run has no Model v12 runtime policy.".to_string())
    })?;
    if workflow_has_policy_capacity(connection, &workflow_id, policy.concurrency)?
      && matches!(
        workflow_rate_eligibility(connection, &workflow_id, policy.rate_limit.as_ref(), now)?,
        PolicyRateEligibility::Eligible
      )
    {
      return Ok(true);
    }
  }
  Ok(false)
}

impl DurableEventStore {
  pub fn validate_runtime_policy_activation(
    &self,
    workflow: &CompiledWorkflowDefinition,
    definition_hash: &str,
  ) -> Result<(), DurableStoreError> {
    if workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V12 {
      return Ok(());
    }
    let policy_hash = workflow.runtime_policy_hash().ok_or_else(|| {
      DurableStoreError::Contract("Model v12 is missing its runtime policy identity.".to_string())
    })?;
    let conflict: bool = self.connection.query_row(
      "SELECT EXISTS(
       SELECT 1
       FROM woml_runtime_policy_bindings AS bindings
       JOIN woml_run_summaries AS summaries ON summaries.run_id = bindings.run_id
       WHERE bindings.workflow_id = ?1
         AND summaries.status IN ('queued', 'running', 'waiting', 'cancelling', 'finalizing')
         AND (bindings.definition_hash != ?2 OR bindings.policy_hash != ?3)
     )",
      params![workflow.workflow_id, definition_hash, policy_hash],
      |row| row.get(0),
    )?;
    if conflict {
      return Err(DurableStoreError::RuntimePolicyDefinitionConflict(
        workflow.workflow_id.clone(),
      ));
    }
    Ok(())
  }

  pub fn claim_policy_run(
    &mut self,
    run_id: &str,
    owner_id: &str,
    now: DateTime<Utc>,
    lease_duration: Duration,
  ) -> Result<SchedulerClaimV1, DurableStoreError> {
    match self.try_claim_policy_run(run_id, owner_id, now, lease_duration)? {
      PolicyExecutionClaimResult::Claimed { claim, .. } => Ok(claim),
      PolicyExecutionClaimResult::Waiting { .. } => Err(DurableStoreError::SchedulerClaimConflict(
        run_id.to_string(),
      )),
    }
  }

  pub fn try_claim_policy_run(
    &mut self,
    run_id: &str,
    owner_id: &str,
    now: DateTime<Utc>,
    lease_duration: Duration,
  ) -> Result<PolicyExecutionClaimResult, DurableStoreError> {
    self.try_claim_policy_run_at(run_id, owner_id, now, now, lease_duration)
  }

  /// Claims executable capacity using separate clocks for the expiring owner
  /// lease and the durable policy timeline. Production uses wall time for the
  /// lease and the runtime clock for deterministic policy eligibility.
  pub fn try_claim_policy_run_at(
    &mut self,
    run_id: &str,
    owner_id: &str,
    lease_now: DateTime<Utc>,
    policy_now: DateTime<Utc>,
    lease_duration: Duration,
  ) -> Result<PolicyExecutionClaimResult, DurableStoreError> {
    if owner_id.is_empty() || owner_id.len() > 320 || lease_duration.is_zero() {
      return Err(DurableStoreError::Contract(
        "A scheduler claim requires a valid owner and positive lease.".to_string(),
      ));
    }
    let expires_at = lease_now
      + chrono::Duration::from_std(lease_duration).map_err(|_| {
        DurableStoreError::Contract("Scheduler lease duration is out of range.".to_string())
      })?;
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute(
      "DELETE FROM woml_scheduler_claims WHERE expires_at <= ?1",
      [lease_now.to_rfc3339()],
    )?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let policy = workflow.runtime_policy.as_ref().ok_or_else(|| {
      DurableStoreError::Contract("Scheduler claims require compiled Model v12.".to_string())
    })?;
    let queue = workflow.runtime_policy_queue_name().ok_or_else(|| {
      DurableStoreError::Contract("Model v12 has no runtime policy queue.".to_string())
    })?;
    let events = load_events(&transaction, run_id)?;
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = fold_events(&events)?;
    if matches!(
      projection.status,
      RunStatus::NotStarted
        | RunStatus::Waiting
        | RunStatus::Succeeded
        | RunStatus::Failed
        | RunStatus::Cancelled
    ) {
      return Err(DurableStoreError::Contract(
        "Only an executable Event v11 policy run may acquire a scheduler claim.".to_string(),
      ));
    }
    let existing: Option<(String, String, String, String)> = transaction
      .query_row(
        "SELECT owner_id, claim_id, claimed_at, expires_at
         FROM woml_scheduler_claims WHERE run_id = ?1",
        [run_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
      )
      .optional()?;
    if let Some((existing_owner, claim_id, claimed_at, expires_at)) = existing {
      if existing_owner != owner_id {
        return Ok(PolicyExecutionClaimResult::Waiting {
          run_id: run_id.to_string(),
          workflow_id: binding.workflow_id,
          queue,
          reason: PolicyClaimWaitReason::Concurrency,
          eligible_at: None,
        });
      }
      return Ok(PolicyExecutionClaimResult::Claimed {
        claim: SchedulerClaimV1 {
          profile: "woml.scheduler-claim/v1",
          run_id: run_id.to_string(),
          workflow_id: binding.workflow_id,
          owner_id: existing_owner,
          claim_id,
          claimed_at: parse_durable_timestamp(&claimed_at, "scheduler claim instant")?,
          expires_at: parse_durable_timestamp(&expires_at, "scheduler claim expiry")?,
        },
        first_start: false,
      });
    }
    let has_ambiguous_work = projection
      .attempts
      .iter()
      .any(|attempt| attempt.status == AttemptStatus::Started)
      || projection
        .operations
        .values()
        .any(|operation| operation.status == crate::projection::OperationStatus::Started)
      || projection.lifecycle_hooks.values().any(|hook| {
        hook
          .actions
          .values()
          .any(|action| action.status == crate::projection::LifecycleActionStatus::Started)
      })
      || projection.for_each.values().any(|loop_state| {
        loop_state.status == ForEachStatus::Open
          && loop_state.executions.values().any(|execution| {
            execution
              .attempts
              .iter()
              .any(|attempt| attempt.status == AttemptStatus::Started)
              || execution
                .operations
                .values()
                .any(|operation| operation.status == crate::projection::OperationStatus::Started)
              || execution.lifecycle_hooks.values().any(|hook| {
                matches!(
                  hook.status,
                  crate::projection::LifecycleHookStatus::Requested
                    | crate::projection::LifecycleHookStatus::Running
                )
              })
              || execution.reusable_lifecycle_hooks.values().any(|hook| {
                matches!(
                  hook.status,
                  crate::projection::ReusableLifecycleStatus::Requested
                    | crate::projection::ReusableLifecycleStatus::Running
                )
              })
          })
      });
    if has_ambiguous_work {
      return Err(DurableStoreError::SchedulerRecoveryRequired(
        run_id.to_string(),
      ));
    }
    let first_start = projection.started_at.is_none() && projection.status == RunStatus::Queued;
    let rate_eligibility = if first_start {
      workflow_rate_eligibility(
        &transaction,
        &binding.workflow_id,
        policy.rate_limit.as_ref(),
        policy_now,
      )?
    } else {
      PolicyRateEligibility::Eligible
    };
    if let PolicyRateEligibility::Waiting { eligible_at } = rate_eligibility {
      transaction.execute(
        "UPDATE woml_run_summaries
         SET waiting_for = 'rate_limit', eligible_at = ?1, updated_at = ?2
         WHERE run_id = ?3",
        params![eligible_at.to_rfc3339(), policy_now.to_rfc3339(), run_id],
      )?;
      transaction.commit()?;
      return Ok(PolicyExecutionClaimResult::Waiting {
        run_id: run_id.to_string(),
        workflow_id: binding.workflow_id,
        queue,
        reason: PolicyClaimWaitReason::RateLimit,
        eligible_at: Some(eligible_at),
      });
    }
    if !workflow_has_policy_capacity(&transaction, &binding.workflow_id, policy.concurrency)?
      || (projection.status == RunStatus::Queued
        && earlier_eligible_policy_run_exists(
          &transaction,
          run_id,
          &queue,
          projection.admitted_at.ok_or_else(|| {
            DurableStoreError::Contract("Queued policy run has no admittedAt.".to_string())
          })?,
          projection.occurrence_sequence.unwrap_or(0),
          policy_now,
        )?)
    {
      transaction.execute(
        "UPDATE woml_run_summaries
         SET waiting_for = 'concurrency', eligible_at = NULL, updated_at = ?1
         WHERE run_id = ?2",
        params![policy_now.to_rfc3339(), run_id],
      )?;
      transaction.commit()?;
      return Ok(PolicyExecutionClaimResult::Waiting {
        run_id: run_id.to_string(),
        workflow_id: binding.workflow_id,
        queue,
        reason: PolicyClaimWaitReason::Concurrency,
        eligible_at: None,
      });
    }
    let claim_id = format!("claim_{}", Uuid::new_v4().simple());
    transaction.execute(
      "INSERT INTO woml_scheduler_claims(
         run_id, workflow_id, owner_id, claim_id, claimed_at, expires_at
       ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
      params![
        run_id,
        binding.workflow_id,
        owner_id,
        claim_id,
        lease_now.to_rfc3339(),
        expires_at.to_rfc3339(),
      ],
    )?;
    if first_start {
      let timeout_at = policy.timeout_ms.map(|timeout_ms| {
        policy_now + chrono::Duration::milliseconds(i64::try_from(timeout_ms).unwrap_or(i64::MAX))
      });
      let mut events = events;
      let event_schema_version = events
        .first()
        .map(|event| event.event_schema_version)
        .ok_or_else(|| {
          DurableStoreError::Contract("Policy run has no admission event.".to_string())
        })?;
      // Policy admission only queues the run. Request its start hook with the
      // first execution claim, in the same transaction and canonical order as
      // legacy RunStarted expansion. Reacquisition must never request it again.
      let payloads = expand_model_v11_payload(
        &workflow,
        run_id,
        RunEventPayload::RunExecutionStarted(crate::event::RunExecutionStartedData {
          started_at: policy_now,
          timeout_at,
        }),
      )?;
      for payload in payloads {
        append_to_history(
          &transaction,
          &mut events,
          run_id,
          generated_event_id(),
          policy_now,
          event_schema_version,
          payload,
        )?;
      }
      validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
        .map_err(DurableStoreError::Contract)?;
    }
    transaction.execute(
      "UPDATE woml_run_summaries
       SET waiting_for = NULL, eligible_at = NULL, updated_at = ?1
       WHERE run_id = ?2",
      params![policy_now.to_rfc3339(), run_id],
    )?;
    transaction.commit()?;
    Ok(PolicyExecutionClaimResult::Claimed {
      claim: SchedulerClaimV1 {
        profile: "woml.scheduler-claim/v1",
        run_id: run_id.to_string(),
        workflow_id: binding.workflow_id,
        owner_id: owner_id.to_string(),
        claim_id,
        claimed_at: lease_now,
        expires_at,
      },
      first_start,
    })
  }

  pub fn renew_policy_claim(
    &mut self,
    run_id: &str,
    owner_id: &str,
    claim_id: &str,
    now: DateTime<Utc>,
    lease_duration: Duration,
  ) -> Result<SchedulerClaimV1, DurableStoreError> {
    let expires_at = now
      + chrono::Duration::from_std(lease_duration).map_err(|_| {
        DurableStoreError::Contract("Scheduler lease duration is out of range.".to_string())
      })?;
    let changed = self.connection.execute(
      "UPDATE woml_scheduler_claims SET claimed_at = ?1, expires_at = ?2
       WHERE run_id = ?3 AND owner_id = ?4 AND claim_id = ?5 AND expires_at > ?1",
      params![
        now.to_rfc3339(),
        expires_at.to_rfc3339(),
        run_id,
        owner_id,
        claim_id,
      ],
    )?;
    if changed != 1 {
      return Err(DurableStoreError::SchedulerClaimConflict(
        run_id.to_string(),
      ));
    }
    let workflow_id = self.run_binding(run_id)?.workflow_id;
    Ok(SchedulerClaimV1 {
      profile: "woml.scheduler-claim/v1",
      run_id: run_id.to_string(),
      workflow_id,
      owner_id: owner_id.to_string(),
      claim_id: claim_id.to_string(),
      claimed_at: now,
      expires_at,
    })
  }

  pub fn suspend_policy_claim(
    &mut self,
    run_id: &str,
    owner_id: &str,
    claim_id: &str,
  ) -> Result<bool, DurableStoreError> {
    Ok(
      self.connection.execute(
        "UPDATE woml_scheduler_claims SET workflow_id = ?1
         WHERE run_id = ?2 AND owner_id = ?3 AND claim_id = ?4",
        params![SUSPENDED_POLICY_WORKFLOW_ID, run_id, owner_id, claim_id],
      )? == 1,
    )
  }

  pub fn resume_policy_claim(
    &mut self,
    run_id: &str,
    owner_id: &str,
    claim_id: &str,
    now: DateTime<Utc>,
  ) -> Result<bool, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute(
      "DELETE FROM woml_scheduler_claims WHERE expires_at <= ?1",
      [now.to_rfc3339()],
    )?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let concurrency = workflow
      .runtime_policy
      .as_ref()
      .ok_or_else(|| {
        DurableStoreError::Contract("Policy resume requires compiled Model v12.".to_string())
      })?
      .concurrency;
    let owned: bool = transaction.query_row(
      "SELECT EXISTS(
         SELECT 1 FROM woml_scheduler_claims
         WHERE run_id = ?1 AND owner_id = ?2 AND claim_id = ?3
           AND workflow_id = ?4 AND expires_at > ?5
       )",
      params![
        run_id,
        owner_id,
        claim_id,
        SUSPENDED_POLICY_WORKFLOW_ID,
        now.to_rfc3339(),
      ],
      |row| row.get(0),
    )?;
    if !owned {
      return Err(DurableStoreError::SchedulerClaimConflict(
        run_id.to_string(),
      ));
    }
    if !workflow_has_policy_capacity(&transaction, &binding.workflow_id, concurrency)? {
      transaction.commit()?;
      return Ok(false);
    }
    let changed = transaction.execute(
      "UPDATE woml_scheduler_claims SET workflow_id = ?1
       WHERE run_id = ?2 AND owner_id = ?3 AND claim_id = ?4
         AND workflow_id = ?5",
      params![
        binding.workflow_id,
        run_id,
        owner_id,
        claim_id,
        SUSPENDED_POLICY_WORKFLOW_ID,
      ],
    )?;
    transaction.commit()?;
    Ok(changed == 1)
  }

  pub fn release_policy_claim(
    &mut self,
    run_id: &str,
    owner_id: &str,
    claim_id: &str,
  ) -> Result<bool, DurableStoreError> {
    Ok(
      self.connection.execute(
        "DELETE FROM woml_scheduler_claims
         WHERE run_id = ?1 AND owner_id = ?2 AND claim_id = ?3",
        params![run_id, owner_id, claim_id],
      )? == 1,
    )
  }

  pub fn active_policy_claim_count(
    &self,
    workflow_id: &str,
    now: DateTime<Utc>,
  ) -> Result<u32, DurableStoreError> {
    let count: i64 = self.connection.query_row(
      "SELECT COUNT(*) FROM woml_scheduler_claims
       WHERE workflow_id = ?1 AND expires_at > ?2",
      params![workflow_id, now.to_rfc3339()],
      |row| row.get(0),
    )?;
    u32::try_from(count)
      .map_err(|_| DurableStoreError::Contract("Scheduler claim count exceeded u32.".to_string()))
  }

  pub(crate) fn policy_run_has_live_claim(
    &mut self,
    run_id: &str,
    now: DateTime<Utc>,
  ) -> Result<bool, DurableStoreError> {
    self.connection.execute(
      "DELETE FROM woml_scheduler_claims WHERE expires_at <= ?1",
      [now.to_rfc3339()],
    )?;
    self
      .connection
      .query_row(
        "SELECT EXISTS(SELECT 1 FROM woml_scheduler_claims WHERE run_id = ?1)",
        [run_id],
        |row| row.get(0),
      )
      .map_err(Into::into)
  }

  /// Applies the normal fail-closed recovery policy after a scheduler owner
  /// disappeared. A live owner can never be recovered by another process.
  pub fn recover_policy_run_after_lease_loss(
    &mut self,
    run_id: &str,
    now: DateTime<Utc>,
  ) -> Result<bool, DurableStoreError> {
    self.connection.execute(
      "DELETE FROM woml_scheduler_claims WHERE expires_at <= ?1",
      [now.to_rfc3339()],
    )?;
    let live_claim: bool = self.connection.query_row(
      "SELECT EXISTS(
         SELECT 1 FROM woml_scheduler_claims
         WHERE run_id = ?1 AND expires_at > ?2
       )",
      params![run_id, now.to_rfc3339()],
      |row| row.get(0),
    )?;
    if live_claim {
      return Err(DurableStoreError::SchedulerClaimConflict(
        run_id.to_string(),
      ));
    }
    Ok(matches!(
      self.recover_run(run_id)?,
      RunRecovery::Recovered { .. }
    ))
  }
}
