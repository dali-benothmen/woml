//! Bounded deterministic recovery work discovery for admitted runs, corrections, locks, and fork branches. Discovery does not execute effects.

use crate::durable::CorrectionRecoveryCandidateV1;
use crate::durable::DurableStoreError;
use crate::durable::ForkRecoveryBranchV1;
use crate::durable::ForkRecoveryWorkV1;
use crate::durable::LockRecoveryCandidateV1;
use crate::durable::LockRecoveryKindV1;
use crate::durable::TriggerRecoveryWork;
use crate::durable::CORRECTION_RECOVERY_BATCH_LIMIT;
use crate::durable::LOCK_RECOVERY_BATCH_LIMIT;
use crate::durable::{
  load_trigger_occurrence_by_id, parse_stored_timestamp, validate_trigger_occurrence_history,
  DurableEventStore,
};
use crate::engine::ready_node_ids_for_projection;
use chrono::DateTime;
use chrono::Utc;
use rusqlite::params;
use std::collections::HashSet;

impl DurableEventStore {
  pub fn recover_undispatched_trigger_runs(
    &self,
  ) -> Result<Vec<TriggerRecoveryWork>, DurableStoreError> {
    let now = Utc::now().to_rfc3339();
    let mut statement = self.connection.prepare(
      "SELECT occurrences.occurrence_id
       FROM woml_trigger_occurrences AS occurrences
       JOIN woml_runs AS runs ON runs.run_id = occurrences.run_id
       JOIN woml_run_summaries AS summaries ON summaries.run_id = occurrences.run_id
       WHERE
         (
           SELECT COUNT(*) FROM woml_run_events AS events
           WHERE events.run_id = occurrences.run_id
         ) = 1
         OR (
           EXISTS (
             SELECT 1 FROM woml_run_events AS events
             WHERE events.run_id = occurrences.run_id
               AND events.event_schema_version = 11
           )
           AND summaries.status IN ('queued', 'running', 'cancelling', 'finalizing')
           AND NOT EXISTS (
             SELECT 1 FROM woml_scheduler_claims AS claims
             WHERE claims.run_id = occurrences.run_id
               AND claims.expires_at > ?1
           )
         )
       ORDER BY occurrences.received_at, occurrences.occurrence_id",
    )?;
    let occurrence_ids = statement
      .query_map([now], |row| row.get::<_, String>(0))?
      .collect::<Result<Vec<_>, _>>()?;
    drop(statement);

    occurrence_ids
      .into_iter()
      .map(|occurrence_id| {
        let occurrence = load_trigger_occurrence_by_id(&self.connection, &occurrence_id)?
          .ok_or_else(|| {
            DurableStoreError::TriggerHistoryInvalid(format!(
              "trigger occurrence {occurrence_id:?} disappeared during recovery"
            ))
          })?;
        let trigger = validate_trigger_occurrence_history(&self.connection, &occurrence)?;
        Ok(TriggerRecoveryWork {
          occurrence,
          trigger,
        })
      })
      .collect()
  }

  pub fn correction_recovery_batch_v1(
    &self,
    limit: usize,
  ) -> Result<Vec<CorrectionRecoveryCandidateV1>, DurableStoreError> {
    self.query_correction_recovery_batch_v1(limit, None)
  }

  pub(crate) fn correction_recovery_batch_for_workflows_v1(
    &self,
    limit: usize,
    workflow_ids: &HashSet<String>,
  ) -> Result<Vec<CorrectionRecoveryCandidateV1>, DurableStoreError> {
    if workflow_ids.is_empty() {
      return Ok(Vec::new());
    }
    self.query_correction_recovery_batch_v1(limit, Some(workflow_ids))
  }

  fn query_correction_recovery_batch_v1(
    &self,
    limit: usize,
    workflow_ids: Option<&HashSet<String>>,
  ) -> Result<Vec<CorrectionRecoveryCandidateV1>, DurableStoreError> {
    if limit == 0 || limit > CORRECTION_RECOVERY_BATCH_LIMIT {
      return Err(DurableStoreError::Contract(format!(
        "Correction recovery limit must be between 1 and {CORRECTION_RECOVERY_BATCH_LIMIT}."
      )));
    }
    let mut parameters = Vec::new();
    let workflow_filter = workflow_ids.map(|workflow_ids| {
      let mut workflow_ids = workflow_ids.iter().cloned().collect::<Vec<_>>();
      workflow_ids.sort();
      let placeholders = workflow_ids
        .iter()
        .map(|workflow_id| {
          parameters.push(rusqlite::types::Value::Text(workflow_id.clone()));
          format!("?{}", parameters.len())
        })
        .collect::<Vec<_>>()
        .join(", ");
      format!(" AND runs.workflow_id IN ({placeholders})")
    });
    parameters.push(rusqlite::types::Value::Integer(
      i64::try_from(limit).unwrap_or(i64::MAX),
    ));
    let limit_parameter = parameters.len();
    let query = format!(
      "SELECT actions.run_id, actions.saga_id, actions.action_id, actions.status
       FROM woml_correction_actions AS actions
       JOIN woml_saga_scopes AS sagas
         ON sagas.run_id = actions.run_id AND sagas.saga_id = actions.saga_id
       JOIN woml_runs AS runs ON runs.run_id = actions.run_id
       LEFT JOIN woml_correction_manual_recovery AS manual
         ON manual.run_id = actions.run_id
        AND manual.saga_id = actions.saga_id
        AND manual.action_id = actions.action_id
       WHERE (
         (sagas.phase = 'correcting' AND actions.status IN ('registered', 'retry_scheduled'))
         OR
         (sagas.phase = 'manual_recovery' AND actions.status = 'manual_recovery'
           AND manual.status = 'externally_resolved')
       ){}
       ORDER BY actions.updated_at, actions.run_id, actions.saga_id, actions.action_id
       LIMIT ?{limit_parameter}",
      workflow_filter.as_deref().unwrap_or_default()
    );
    let mut statement = self.connection.prepare(&query)?;
    let candidates = statement
      .query_map(rusqlite::params_from_iter(parameters.iter()), |row| {
        Ok(CorrectionRecoveryCandidateV1 {
          run_id: row.get(0)?,
          saga_id: row.get(1)?,
          action_id: row.get(2)?,
          status: row.get(3)?,
        })
      })?
      .collect::<Result<Vec<_>, _>>()
      .map_err(DurableStoreError::from)?;
    Ok(candidates)
  }

  /// Returns at most one frozen recovery batch. Repeated calls after settling
  /// the returned work drain large backlogs without loading the whole queue
  /// into memory or monopolizing the durable store.
  pub fn lock_recovery_batch_v1(
    &self,
    now: DateTime<Utc>,
  ) -> Result<Vec<LockRecoveryCandidateV1>, DurableStoreError> {
    let limit = i64::try_from(LOCK_RECOVERY_BATCH_LIMIT)
      .map_err(|_| DurableStoreError::Contract("Lock recovery limit is invalid.".to_string()))?;
    let mut statement = self.connection.prepare(
      "SELECT kind, key, run_id, node_execution_id, due_at FROM (
         SELECT 'acquisition_timeout' AS kind, key, run_id, node_execution_id,
                deadline AS due_at
         FROM woml_lock_waiters
         WHERE status = 'waiting' AND deadline <= ?1
         UNION ALL
         SELECT 'lease_expiry' AS kind, key, owner_run_id AS run_id,
                owner_node_execution_id AS node_execution_id, expires_at AS due_at
         FROM woml_lock_resources
         WHERE owner_lease_id IS NOT NULL AND expires_at <= ?1
       )
       ORDER BY due_at, kind, key, run_id, node_execution_id
       LIMIT ?2",
    )?;
    let rows = statement
      .query_map(params![now.to_rfc3339(), limit], |row| {
        Ok((
          row.get::<_, String>(0)?,
          row.get::<_, String>(1)?,
          row.get::<_, String>(2)?,
          row.get::<_, String>(3)?,
          row.get::<_, String>(4)?,
        ))
      })?
      .collect::<Result<Vec<_>, _>>()?;
    rows
      .into_iter()
      .map(|(kind, key, run_id, execution_identity, due_at)| {
        let kind = match kind.as_str() {
          "acquisition_timeout" => LockRecoveryKindV1::AcquisitionTimeout,
          "lease_expiry" => LockRecoveryKindV1::LeaseExpiry,
          _ => {
            return Err(DurableStoreError::LockHistoryInvalid(
              "lock recovery found an unsupported work kind".to_string(),
            ))
          }
        };
        Ok(LockRecoveryCandidateV1 {
          profile: "woml.lock-recovery-candidate/v1",
          kind,
          key,
          run_id,
          execution_identity,
          due_at: parse_stored_timestamp(&due_at)?,
        })
      })
      .collect()
  }

  pub fn fork_recovery_work_v1(
    &self,
    run_id: &str,
  ) -> Result<ForkRecoveryWorkV1, DurableStoreError> {
    let binding = self.run_binding(run_id)?;
    let workflow = self.definition(&binding.definition_hash)?;
    if workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V13 {
      return Err(DurableStoreError::Contract(
        "Fork recovery work is available only for compiled Model v13 runs.".to_string(),
      ));
    }
    let projection = self.projection(run_id)?;
    let ready = ready_node_ids_for_projection(&workflow, &binding.definition_hash, &projection)
      .map_err(DurableStoreError::Contract)?;
    let mut openable_fork_ids = Vec::new();
    let mut pending_branches = Vec::new();
    let mut joinable_fork_ids = Vec::new();
    let mut pending_choice_ids = Vec::new();
    let mut ambiguous_active_node_ids = projection
      .active_attempt_node_ids()
      .into_iter()
      .map(str::to_string)
      .collect::<Vec<_>>();
    ambiguous_active_node_ids.sort();

    for choice in workflow.graph.choices.as_deref().unwrap_or_default() {
      if !projection.choice_selections.contains_key(&choice.choice_id)
        && ready.contains(&choice.selector_node_id)
      {
        pending_choice_ids.push(choice.choice_id.clone());
      }
    }
    for fork in workflow.graph.forks.as_deref().unwrap_or_default() {
      let Some(projected) = projection.forks.get(&fork.fork_id) else {
        if ready.contains(&fork.open_node_id) {
          openable_fork_ids.push(fork.fork_id.clone());
        }
        continue;
      };
      for branch in &fork.branches {
        if projected.branches.contains_key(&branch.branch_id) {
          continue;
        }
        let mut route = std::collections::HashSet::new();
        let mut queue = std::collections::VecDeque::from([branch.entry_node_id.as_str()]);
        while let Some(node_id) = queue.pop_front() {
          if !route.insert(node_id) || node_id == branch.terminal_node_id {
            continue;
          }
          for edge in workflow
            .graph
            .edges
            .iter()
            .filter(|edge| edge.from == node_id)
          {
            queue.push_back(edge.to.as_str());
          }
        }
        if !ambiguous_active_node_ids
          .iter()
          .any(|node_id| route.contains(node_id.as_str()))
        {
          pending_branches.push(ForkRecoveryBranchV1 {
            fork_id: fork.fork_id.clone(),
            branch_id: branch.branch_id.clone(),
          });
        }
      }
      if projected.join_status == crate::projection::ForkJoinStatus::Pending
        && fork.joined_branch_ids.iter().all(|branch_id| {
          projected
            .branches
            .get(branch_id)
            .and_then(|branch| branch.outcome)
            == Some(crate::event::ForkBranchOutcome::Succeeded)
        })
      {
        joinable_fork_ids.push(fork.fork_id.clone());
      }
    }
    Ok(ForkRecoveryWorkV1 {
      profile: "woml.fork-recovery-work/v1",
      openable_fork_ids,
      pending_branches,
      joinable_fork_ids,
      pending_choice_ids,
      ambiguous_active_node_ids,
    })
  }
}
