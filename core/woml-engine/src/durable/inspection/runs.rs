//! Versioned bounded run lists, runtime observations, and run inspection views.

use crate::durable::{
  parse_stored_timestamp, DurableEventStore, DurableStoreError, InspectedForEachStatusV6,
  InspectedForkJoinModeV4, InspectedForkStatusV4, InspectedReusableKindV5,
  InspectedReusableLifecycleStatusV5, InspectedReusableStatusV5, PolicyWaitingFor, PublicRunStatus,
  RunInspectionCancellationV2, RunInspectionForEachCountsV6, RunInspectionForEachItemV6,
  RunInspectionForEachV6, RunInspectionForkCountsV4, RunInspectionForkJoinV4, RunInspectionForkV4,
  RunInspectionForksV4, RunInspectionHookV2, RunInspectionPolicyV3, RunInspectionReusableCountsV5,
  RunInspectionReusableDefinitionsV5, RunInspectionReusableItemV5, RunInspectionRichApprovalV11,
  RunInspectionRichApprovalsV11, RunInspectionV11, RunInspectionV2, RunInspectionV3,
  RunInspectionV4, RunInspectionV5, RunInspectionV6, RunListV1, RunListV2, RunSummaryV1,
  RunSummaryV2, RuntimeObservationV1, RICH_APPROVAL_INSPECTION_DECISION_LIMIT,
  RUN_INSPECTION_RICH_APPROVAL_LIMIT,
};
use crate::event::ApprovalDecision;
use crate::projection::AttemptStatus;
use crate::projection::ForEachIterationStatus;
use crate::projection::ForEachStatus;
use crate::AttemptFailureKind;
use std::collections::BTreeMap;

impl DurableEventStore {
  pub fn list_runs(&self, limit: usize) -> Result<RunListV1, DurableStoreError> {
    self.list_runs_filtered(limit, None, None)
  }

  pub fn list_runs_filtered(
    &self,
    limit: usize,
    workflow_id: Option<&str>,
    status: Option<&str>,
  ) -> Result<RunListV1, DurableStoreError> {
    if !(1..=200).contains(&limit) {
      return Err(DurableStoreError::Contract(
        "Run list limit must be between 1 and 200.".to_string(),
      ));
    }
    if workflow_id.is_some_and(str::is_empty) {
      return Err(DurableStoreError::Contract(
        "Run list workflow filter must not be empty.".to_string(),
      ));
    }
    let status = status
      .map(|value| {
        PublicRunStatus::parse(value).ok_or_else(|| {
          DurableStoreError::Contract(format!("Run list status filter {value:?} is invalid."))
        })
      })
      .transpose()?;
    let mut statement = self.connection.prepare(
      "SELECT run_id, workflow_id, status, started_at, updated_at
       FROM woml_run_summaries
       WHERE (?2 IS NULL OR workflow_id = ?2)
         AND (?3 IS NULL OR status = ?3)
         AND started_at IS NOT NULL
         AND status != 'queued'
       ORDER BY updated_at DESC, run_id DESC
       LIMIT ?1",
    )?;
    let rows = statement
      .query_map(
        rusqlite::params![
          i64::try_from(limit).unwrap_or(200),
          workflow_id,
          status.map(PublicRunStatus::as_str),
        ],
        |row| {
          Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
          ))
        },
      )?
      .collect::<Result<Vec<_>, _>>()?;
    let mut runs = Vec::with_capacity(rows.len());
    for (run_id, workflow_id, status, started_at, updated_at) in rows {
      runs.push(RunSummaryV1 {
        run_id,
        workflow_id,
        status: PublicRunStatus::parse(&status).ok_or_else(|| {
          DurableStoreError::Contract("Stored run summary status is invalid.".to_string())
        })?,
        started_at: parse_stored_timestamp(&started_at)?,
        updated_at: parse_stored_timestamp(&updated_at)?,
      });
    }
    Ok(RunListV1 {
      profile: "woml.run-list/v1",
      runs,
    })
  }

  pub fn list_runs_v2(&self, limit: usize) -> Result<RunListV2, DurableStoreError> {
    self.list_runs_v2_filtered(limit, None, None)
  }

  pub fn list_runs_v2_filtered(
    &self,
    limit: usize,
    workflow_id: Option<&str>,
    status: Option<&str>,
  ) -> Result<RunListV2, DurableStoreError> {
    if !(1..=200).contains(&limit) {
      return Err(DurableStoreError::Contract(
        "Run list limit must be between 1 and 200.".to_string(),
      ));
    }
    if workflow_id.is_some_and(str::is_empty) {
      return Err(DurableStoreError::Contract(
        "Run list workflow filter must not be empty.".to_string(),
      ));
    }
    let status = status
      .map(|value| {
        PublicRunStatus::parse(value).ok_or_else(|| {
          DurableStoreError::Contract(format!("Run list status filter {value:?} is invalid."))
        })
      })
      .transpose()?;
    let mut statement = self.connection.prepare(
      "SELECT run_id, workflow_id, status, admitted_at, started_at, updated_at,
              queue_name, waiting_for, eligible_at
       FROM woml_run_summaries
       WHERE (?2 IS NULL OR workflow_id = ?2)
         AND (?3 IS NULL OR status = ?3)
       ORDER BY updated_at DESC, run_id DESC
       LIMIT ?1",
    )?;
    let rows = statement
      .query_map(
        rusqlite::params![
          i64::try_from(limit).unwrap_or(200),
          workflow_id,
          status.map(PublicRunStatus::as_str),
        ],
        |row| {
          Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, Option<String>>(7)?,
            row.get::<_, Option<String>>(8)?,
          ))
        },
      )?
      .collect::<Result<Vec<_>, _>>()?;
    let mut runs = Vec::with_capacity(rows.len());
    for row in rows {
      runs.push(RunSummaryV2 {
        run_id: row.0,
        workflow_id: row.1,
        status: PublicRunStatus::parse(&row.2).ok_or_else(|| {
          DurableStoreError::Contract("Stored run summary status is invalid.".to_string())
        })?,
        admitted_at: parse_stored_timestamp(&row.3)?,
        started_at: row.4.as_deref().map(parse_stored_timestamp).transpose()?,
        updated_at: parse_stored_timestamp(&row.5)?,
        queue: row.6,
        waiting_for: row
          .7
          .as_deref()
          .map(|value| {
            PolicyWaitingFor::parse(value).ok_or_else(|| {
              DurableStoreError::Contract("Stored policy wait reason is invalid.".to_string())
            })
          })
          .transpose()?,
        eligible_at: row.8.as_deref().map(parse_stored_timestamp).transpose()?,
      });
    }
    Ok(RunListV2 {
      profile: "woml.run-list/v2",
      runs,
    })
  }

  /// Returns a compact, payload-free projection for Runtime Operations v1.
  /// Counts come from durable coordination/event facts and therefore survive
  /// process replacement without making telemetry authoritative.
  pub fn runtime_observation_v1(&self) -> Result<RuntimeObservationV1, DurableStoreError> {
    let mut status_totals = BTreeMap::new();
    let mut statement = self
      .connection
      .prepare("SELECT status, COUNT(*) FROM woml_run_summaries GROUP BY status ORDER BY status")?;
    let rows = statement
      .query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
      })?
      .collect::<Result<Vec<_>, _>>()?;
    for (status, count) in rows {
      if PublicRunStatus::parse(&status).is_none() {
        return Err(DurableStoreError::Contract(
          "Stored runtime observation status is invalid.".to_string(),
        ));
      }
      status_totals.insert(status, count);
    }

    let latest_event_run_ids = |event_type: &str| -> Result<Vec<String>, DurableStoreError> {
      let pattern = format!("%\"type\":\"{event_type}\"%");
      let mut statement = self.connection.prepare(
        "SELECT summaries.run_id
         FROM woml_run_summaries AS summaries
         JOIN woml_run_events AS events
           ON events.run_id = summaries.run_id
          AND events.sequence = (
            SELECT MAX(latest.sequence)
            FROM woml_run_events AS latest
            WHERE latest.run_id = summaries.run_id
          )
         WHERE events.event_json LIKE ?1
         ORDER BY summaries.updated_at DESC, summaries.run_id DESC
         LIMIT 1000",
      )?;
      let rows = statement
        .query_map([pattern], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
      Ok(rows)
    };
    let retrying_run_ids = latest_event_run_ids("step_retry_scheduled")?;
    let mut approval_statement = self.connection.prepare(
      "SELECT summaries.run_id
       FROM woml_run_summaries AS summaries
       WHERE summaries.status = 'waiting'
         AND EXISTS (
           SELECT 1 FROM woml_run_events AS requested
           WHERE requested.run_id = summaries.run_id
             AND requested.event_json LIKE '%\"type\":\"approval_requested\"%'
         )
         AND NOT EXISTS (
           SELECT 1 FROM woml_run_events AS resolved
           WHERE resolved.run_id = summaries.run_id
             AND resolved.event_json LIKE '%\"type\":\"approval_resolved\"%'
         )
       ORDER BY summaries.updated_at DESC, summaries.run_id DESC
       LIMIT 1000",
    )?;
    let approval_waiting_run_ids = approval_statement
      .query_map([], |row| row.get::<_, String>(0))?
      .collect::<Result<Vec<_>, _>>()?;
    let count = |sql: &str| -> Result<u64, DurableStoreError> {
      Ok(self.connection.query_row(sql, [], |row| row.get(0))?)
    };

    Ok(RuntimeObservationV1 {
      profile: "woml.runtime-observation/v1",
      status_totals,
      retrying_run_ids,
      approval_waiting_run_ids,
      retries_total: count(
        "SELECT COUNT(*) FROM woml_run_events
         WHERE event_json LIKE '%\"type\":\"step_retry_scheduled\"%'",
      )?,
      triggers_total: count("SELECT COUNT(*) FROM woml_trigger_occurrences")?,
      workflow_calls_active: count(
        "SELECT COUNT(*) FROM woml_workflow_calls WHERE state IN ('admitted', 'running')",
      )?,
    })
  }

  pub fn inspect_run_v2(&self, run_id: &str) -> Result<RunInspectionV2, DurableStoreError> {
    let projection = self.projection(run_id)?;
    let run_id = projection.run_id.clone().ok_or_else(|| {
      DurableStoreError::Contract("Run inspection requires run_started.".to_string())
    })?;
    let workflow_id = projection.workflow_id.clone().ok_or_else(|| {
      DurableStoreError::Contract("Run inspection requires a workflow identity.".to_string())
    })?;
    let hooks = projection
      .lifecycle_hooks
      .values()
      .map(|hook| RunInspectionHookV2 {
        hook_id: hook.hook_id.clone(),
        subject_kind: hook.subject.kind,
        subject_id: hook.subject.id.clone(),
        status: hook.status,
        failed_actions: hook.failed_actions,
      })
      .collect();
    Ok(RunInspectionV2 {
      profile: "woml.run-inspection/v2",
      run_id,
      workflow_id,
      status: projection.status.into(),
      business_outcome: projection.business_outcome.into(),
      lifecycle_status: projection.lifecycle_status,
      hooks,
      warnings: projection.lifecycle_warnings,
      cancellation: RunInspectionCancellationV2 {
        requested: projection.cancellation_request_id.is_some(),
        request_id: projection.cancellation_request_id,
      },
    })
  }

  pub fn inspect_run_v3(&self, run_id: &str) -> Result<RunInspectionV3, DurableStoreError> {
    let binding = self.run_binding(run_id)?;
    let projection = self.projection(run_id)?;
    let run_id = projection.run_id.clone().ok_or_else(|| {
      DurableStoreError::Contract("Run inspection requires an admission event.".to_string())
    })?;
    let queue = projection.queue.clone().ok_or_else(|| {
      DurableStoreError::Contract(
        "Run Inspection v3 is available only for Event v11 policy runs.".to_string(),
      )
    })?;
    let (waiting_for, eligible_at): (Option<String>, Option<String>) = self.connection.query_row(
      "SELECT waiting_for, eligible_at FROM woml_run_summaries WHERE run_id = ?1",
      [&run_id],
      |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let hooks = projection
      .lifecycle_hooks
      .values()
      .map(|hook| RunInspectionHookV2 {
        hook_id: hook.hook_id.clone(),
        subject_kind: hook.subject.kind,
        subject_id: hook.subject.id.clone(),
        status: hook.status,
        failed_actions: hook.failed_actions,
      })
      .collect();
    Ok(RunInspectionV3 {
      profile: "woml.run-inspection/v3",
      run_id,
      workflow_id: binding.workflow_id,
      status: projection.status.into(),
      business_outcome: projection.business_outcome.into(),
      lifecycle_status: projection.lifecycle_status,
      hooks,
      warnings: projection.lifecycle_warnings,
      cancellation: RunInspectionCancellationV2 {
        requested: projection.cancellation_request_id.is_some(),
        request_id: projection.cancellation_request_id,
      },
      policy: RunInspectionPolicyV3 {
        queue,
        waiting_for: waiting_for
          .as_deref()
          .map(|value| {
            PolicyWaitingFor::parse(value).ok_or_else(|| {
              DurableStoreError::Contract("Stored policy wait reason is invalid.".to_string())
            })
          })
          .transpose()?,
        eligible_at: eligible_at
          .as_deref()
          .map(parse_stored_timestamp)
          .transpose()?,
        timeout_at: projection.timeout_at,
      },
    })
  }

  pub fn inspect_run_v4(&self, run_id: &str) -> Result<RunInspectionV4, DurableStoreError> {
    let base = self.inspect_run_v3(run_id)?;
    let binding = self.run_binding(run_id)?;
    let workflow = self.definition(&binding.definition_hash)?;
    if workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V13 {
      return Err(DurableStoreError::Contract(
        "Run Inspection v4 is available only for compiled Model v13 runs.".to_string(),
      ));
    }
    let projection = self.projection(run_id)?;
    let mut items = Vec::new();
    for fork in workflow.graph.forks.as_deref().unwrap_or_default() {
      let Some(projected) = projection.forks.get(&fork.fork_id) else {
        continue;
      };
      let mut branches = RunInspectionForkCountsV4 {
        opened: u32::try_from(fork.branches.len()).unwrap_or(u32::MAX),
        active: 0,
        waiting: 0,
        succeeded: 0,
        failed: 0,
        cancelled: 0,
      };
      for branch in &fork.branches {
        match projected
          .branches
          .get(&branch.branch_id)
          .and_then(|branch| branch.outcome)
        {
          None => branches.active += 1,
          Some(crate::event::ForkBranchOutcome::Succeeded) => branches.succeeded += 1,
          Some(crate::event::ForkBranchOutcome::Failed) => branches.failed += 1,
          Some(crate::event::ForkBranchOutcome::Cancelled) => branches.cancelled += 1,
        }
      }
      let status = if projected.join_status == crate::projection::ForkJoinStatus::Failed
        || branches.failed > 0
      {
        InspectedForkStatusV4::Failed
      } else if projected.join_status == crate::projection::ForkJoinStatus::Cancelled
        || branches.cancelled > 0
      {
        InspectedForkStatusV4::Cancelled
      } else if branches.active > 0 {
        InspectedForkStatusV4::Active
      } else if projected.join_status == crate::projection::ForkJoinStatus::Pending {
        InspectedForkStatusV4::Waiting
      } else {
        InspectedForkStatusV4::Succeeded
      };
      let mode = if fork.joined_branch_ids.is_empty() {
        InspectedForkJoinModeV4::None
      } else if fork.joined_branch_ids.len() == fork.branches.len() {
        InspectedForkJoinModeV4::All
      } else {
        InspectedForkJoinModeV4::Selected
      };
      items.push(RunInspectionForkV4 {
        fork_id: fork.fork_id.clone(),
        status,
        join: RunInspectionForkJoinV4 {
          mode,
          branch_ids: fork.joined_branch_ids.clone(),
          status: projected.join_status,
        },
        branches,
      });
    }
    let mut counts = RunInspectionForkCountsV4 {
      opened: u32::try_from(items.len()).unwrap_or(u32::MAX),
      active: 0,
      waiting: 0,
      succeeded: 0,
      failed: 0,
      cancelled: 0,
    };
    for fork in &items {
      match fork.status {
        InspectedForkStatusV4::Active => counts.active += 1,
        InspectedForkStatusV4::Waiting => counts.waiting += 1,
        InspectedForkStatusV4::Succeeded => counts.succeeded += 1,
        InspectedForkStatusV4::Failed => counts.failed += 1,
        InspectedForkStatusV4::Cancelled => counts.cancelled += 1,
      }
    }
    Ok(RunInspectionV4 {
      profile: "woml.run-inspection/v4",
      run_id: base.run_id,
      workflow_id: base.workflow_id,
      status: base.status,
      business_outcome: base.business_outcome,
      lifecycle_status: base.lifecycle_status,
      hooks: base.hooks,
      warnings: base.warnings,
      cancellation: base.cancellation,
      policy: base.policy,
      forks: RunInspectionForksV4 { counts, items },
    })
  }

  pub fn inspect_run_v5(&self, run_id: &str) -> Result<RunInspectionV5, DurableStoreError> {
    let base = self.inspect_run_v4(run_id)?;
    let binding = self.run_binding(run_id)?;
    let workflow = self.definition(&binding.definition_hash)?;
    if workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V14 {
      return Err(DurableStoreError::Contract(
        "Run Inspection v5 is available only for compiled Model v14 or later runs.".to_string(),
      ));
    }
    let projection = self.projection(run_id)?;
    let mut items = Vec::new();
    for definition in workflow.reusable_definitions.iter().flatten() {
      let (invocation_id, alias, definition_digest, kind, lifecycle, status) = match definition {
        crate::model::CompiledReusableInvocation::Step {
          invocation_id,
          node_id,
          alias,
          definition_digest,
          lifecycle,
          ..
        } => {
          let status = match projection.latest_attempt(node_id) {
            None => InspectedReusableStatusV5::Pending,
            Some(attempt) => match &attempt.status {
              AttemptStatus::Started => InspectedReusableStatusV5::Running,
              AttemptStatus::Succeeded { .. } => InspectedReusableStatusV5::Succeeded,
              AttemptStatus::Failed { failure }
                if failure.kind == AttemptFailureKind::InvocationCancelled =>
              {
                InspectedReusableStatusV5::Cancelled
              }
              AttemptStatus::Failed { .. } => InspectedReusableStatusV5::Failed,
            },
          };
          (
            invocation_id,
            alias,
            definition_digest,
            InspectedReusableKindV5::Step,
            lifecycle,
            status,
          )
        }
        crate::model::CompiledReusableInvocation::NotificationProvider {
          provider_id,
          alias,
          definition_digest,
          lifecycle,
          ..
        } => {
          let status = match projection.notification_deliveries.get(provider_id) {
            None => InspectedReusableStatusV5::Pending,
            Some(delivery) => match &delivery.status {
              crate::projection::NotificationDeliveryStatus::Requested
              | crate::projection::NotificationDeliveryStatus::AttemptStarted { .. } => {
                InspectedReusableStatusV5::Running
              }
              crate::projection::NotificationDeliveryStatus::Succeeded { .. } => {
                InspectedReusableStatusV5::Succeeded
              }
              crate::projection::NotificationDeliveryStatus::Failed { failure, .. }
                if failure.kind == "cancelled" =>
              {
                InspectedReusableStatusV5::Cancelled
              }
              crate::projection::NotificationDeliveryStatus::Failed { .. } => {
                InspectedReusableStatusV5::Failed
              }
            },
          };
          (
            provider_id,
            alias,
            definition_digest,
            InspectedReusableKindV5::NotificationProvider,
            lifecycle,
            status,
          )
        }
      };
      let lifecycle_hooks = projection
        .reusable_lifecycle_hooks
        .values()
        .filter(|hook| hook.invocation_id == *invocation_id)
        .collect::<Vec<_>>();
      let lifecycle_status = if lifecycle.is_none() || lifecycle_hooks.is_empty() {
        InspectedReusableLifecycleStatusV5::Idle
      } else if lifecycle_hooks.iter().any(|hook| {
        hook.status == crate::projection::ReusableLifecycleStatus::CompletedWithWarnings
      }) {
        InspectedReusableLifecycleStatusV5::CompletedWithWarnings
      } else if lifecycle_hooks
        .iter()
        .all(|hook| hook.status == crate::projection::ReusableLifecycleStatus::Completed)
      {
        InspectedReusableLifecycleStatusV5::Completed
      } else {
        InspectedReusableLifecycleStatusV5::Running
      };
      items.push(RunInspectionReusableItemV5 {
        invocation_id: invocation_id.clone(),
        alias: alias.clone(),
        definition_digest_prefix: definition_digest
          .trim_start_matches("sha256:")
          .chars()
          .take(12)
          .collect(),
        kind,
        status,
        lifecycle_status,
      });
    }
    items.sort_by(|left, right| left.invocation_id.cmp(&right.invocation_id));
    let mut counts = RunInspectionReusableCountsV5::default();
    for item in &items {
      match item.status {
        InspectedReusableStatusV5::Pending => counts.pending += 1,
        InspectedReusableStatusV5::Running => counts.running += 1,
        InspectedReusableStatusV5::Succeeded => counts.succeeded += 1,
        InspectedReusableStatusV5::Failed => counts.failed += 1,
        InspectedReusableStatusV5::Cancelled => counts.cancelled += 1,
      }
      if item.lifecycle_status == InspectedReusableLifecycleStatusV5::CompletedWithWarnings {
        counts.completed_with_warnings += 1;
      }
    }
    Ok(RunInspectionV5 {
      profile: "woml.run-inspection/v5",
      run_id: base.run_id,
      workflow_id: base.workflow_id,
      status: base.status,
      business_outcome: base.business_outcome,
      lifecycle_status: base.lifecycle_status,
      hooks: base.hooks,
      warnings: base.warnings,
      cancellation: base.cancellation,
      policy: base.policy,
      forks: base.forks,
      reusable_definitions: RunInspectionReusableDefinitionsV5 { counts, items },
    })
  }

  pub fn inspect_run_v6(&self, run_id: &str) -> Result<RunInspectionV6, DurableStoreError> {
    let base = self.inspect_run_v5(run_id)?;
    let binding = self.run_binding(run_id)?;
    let workflow = self.definition(&binding.definition_hash)?;
    if workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V16 {
      return Err(DurableStoreError::Contract(
        "Run Inspection v6 is available only for compiled Model v16 or later runs.".to_string(),
      ));
    }
    let projection = self.projection(run_id)?;
    let mut items = Vec::new();
    for descriptor in workflow.graph.for_each.as_deref().unwrap_or_default() {
      let Some(state) = projection.for_each.get(&descriptor.for_each_id) else {
        continue;
      };
      let succeeded = state
        .iterations
        .values()
        .filter(|status| matches!(status, ForEachIterationStatus::Succeeded { .. }))
        .count() as u32;
      let failed = state
        .iterations
        .values()
        .filter(|status| matches!(status, ForEachIterationStatus::Failed { .. }))
        .count() as u32;
      let skipped = state
        .iterations
        .values()
        .filter(|status| matches!(status, ForEachIterationStatus::Skipped))
        .count() as u32;
      let active = state
        .iterations
        .values()
        .filter(|status| matches!(status, ForEachIterationStatus::Started))
        .count() as u32;
      let pending = state
        .total
        .saturating_sub(succeeded + failed + skipped + active);
      let (failed_index, failed_node_id) = state
        .iterations
        .iter()
        .find_map(|(index, status)| match status {
          ForEachIterationStatus::Failed { failed_node_id } => {
            Some((Some(*index), Some(failed_node_id.clone())))
          }
          _ => None,
        })
        .unwrap_or((None, None));
      items.push(RunInspectionForEachItemV6 {
        for_each_id: descriptor.for_each_id.clone(),
        status: match state.status {
          ForEachStatus::Open => InspectedForEachStatusV6::Active,
          ForEachStatus::Succeeded => InspectedForEachStatusV6::Succeeded,
          ForEachStatus::Failed => InspectedForEachStatusV6::Failed,
          ForEachStatus::Cancelled => InspectedForEachStatusV6::Cancelled,
        },
        total: state.total,
        succeeded,
        failed,
        skipped,
        active,
        pending,
        concurrency: state.concurrency,
        failed_index,
        failed_node_id,
      });
    }
    let mut counts = RunInspectionForEachCountsV6 {
      opened: u32::try_from(items.len()).unwrap_or(u32::MAX),
      ..RunInspectionForEachCountsV6::default()
    };
    for item in &items {
      match item.status {
        InspectedForEachStatusV6::Active => counts.active += 1,
        InspectedForEachStatusV6::Succeeded => counts.succeeded += 1,
        InspectedForEachStatusV6::Failed => counts.failed += 1,
        InspectedForEachStatusV6::Cancelled => counts.cancelled += 1,
      }
    }
    Ok(RunInspectionV6 {
      profile: "woml.run-inspection/v6",
      run_id: base.run_id,
      workflow_id: base.workflow_id,
      status: base.status,
      business_outcome: base.business_outcome,
      lifecycle_status: base.lifecycle_status,
      hooks: base.hooks,
      warnings: base.warnings,
      cancellation: base.cancellation,
      policy: base.policy,
      forks: base.forks,
      reusable_definitions: base.reusable_definitions,
      for_each: RunInspectionForEachV6 { counts, items },
    })
  }

  pub fn inspect_run_v11(&self, run_id: &str) -> Result<RunInspectionV11, DurableStoreError> {
    let base = self.inspect_run_v6(run_id)?;
    let binding = self.run_binding(run_id)?;
    let workflow = self.definition(&binding.definition_hash)?;
    if workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V20 {
      return Err(DurableStoreError::Contract(
        "Run Inspection v11 is available only for compiled Model v20 or later runs.".to_string(),
      ));
    }
    let projection = self.projection(run_id)?;
    let total = projection.rich_approval_requests.len();
    let mut items = Vec::new();
    for request in projection
      .rich_approval_requests
      .values()
      .take(RUN_INSPECTION_RICH_APPROVAL_LIMIT)
    {
      let inspection = self.inspect_rich_approval(
        run_id,
        &request.request_id,
        RICH_APPROVAL_INSPECTION_DECISION_LIMIT,
      )?;
      let approval_deadline = workflow
        .approval(&request.approval_id)
        .and_then(|approval| approval.timeout_ms)
        .and_then(|milliseconds| i64::try_from(milliseconds).ok())
        .and_then(|milliseconds| {
          request
            .requested_at
            .checked_add_signed(chrono::Duration::milliseconds(milliseconds))
        });
      let deadline = match (approval_deadline, projection.timeout_at) {
        (Some(approval), Some(workflow)) => Some(approval.min(workflow)),
        (Some(approval), None) => Some(approval),
        (None, Some(workflow)) => Some(workflow),
        (None, None) => None,
      };
      let received = request
        .decisions
        .values()
        .filter(|decision| decision.decision == ApprovalDecision::Approved)
        .count() as u32;
      items.push(RunInspectionRichApprovalV11 {
        approval_id: inspection.approval_id,
        request_id: inspection.request_id,
        status: inspection.status,
        review_digest: inspection.review_digest,
        review: inspection.review,
        required: inspection.required,
        received,
        required_roles: inspection.required_roles,
        represented_roles: request.represented_roles.clone(),
        directory_version: inspection.directory_version,
        requested_at: request.requested_at,
        deadline,
        resolved_at: inspection.resolved_at,
        decisions: inspection.decisions,
        total_decisions: inspection.total_decisions,
        decisions_truncated: inspection.decisions_truncated,
      });
    }
    let pending = items
      .iter()
      .filter(|approval| approval.status == "pending")
      .count() as u32;
    Ok(RunInspectionV11 {
      profile: "woml.run-inspection/v11",
      run_id: base.run_id,
      workflow_id: base.workflow_id,
      status: base.status,
      business_outcome: base.business_outcome,
      lifecycle_status: base.lifecycle_status,
      hooks: base.hooks,
      warnings: base.warnings,
      cancellation: base.cancellation,
      policy: base.policy,
      forks: base.forks,
      reusable_definitions: base.reusable_definitions,
      for_each: base.for_each,
      approvals: RunInspectionRichApprovalsV11 {
        pending,
        items,
        truncated: total > RUN_INSPECTION_RICH_APPROVAL_LIMIT,
      },
    })
  }
}
