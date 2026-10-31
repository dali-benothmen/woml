//! Private webhook waits.

use crate::webhook::runtime_failure_code;
use crate::webhook::ActiveRunGuard;
use crate::webhook::TriggerProgress;
use crate::webhook::WebhookRuntimeState;
use crate::webhook::TRIGGER_PROGRESS_CONTRACT;
use crate::webhook::TRIGGER_PROGRESS_CONTRACT_VERSION;
use crate::DurableEventStore;
use actix_web::web;
use chrono::Utc;
use std::collections::HashSet;

const DURABLE_WAIT_SCAN_MILLISECONDS: u64 = 250;

const DURABLE_WAIT_SCAN_LIMIT: usize = 256;

pub(in crate::webhook) async fn run_durable_wait_scheduler(state: web::Data<WebhookRuntimeState>) {
  let workflow_ids = state
    .workflow_targets
    .targets()
    .into_iter()
    .map(|target| target.workflow_id)
    .collect::<HashSet<_>>();
  loop {
    let due = DurableEventStore::open_ready(&state.database_path).is_ok_and(|mut store| {
      let timer_due = store
        .due_timer_waits_for_workflows(
          state.execution.clock.now(),
          DURABLE_WAIT_SCAN_LIMIT,
          &workflow_ids,
        )
        .is_ok_and(|waits| !waits.is_empty());
      let signal_due = store
        .due_signal_waits_for_workflows(
          state.execution.clock.now(),
          DURABLE_WAIT_SCAN_LIMIT,
          &workflow_ids,
        )
        .is_ok_and(|waits| !waits.is_empty());
      let correction_due = store
        .correction_recovery_batch_for_workflows_v1(
          crate::CORRECTION_RECOVERY_BATCH_LIMIT,
          &workflow_ids,
        )
        .is_ok_and(|candidates| {
          candidates.into_iter().any(|candidate| {
            !store
              .policy_run_has_live_claim(&candidate.run_id, chrono::Utc::now())
              .unwrap_or(true)
          })
        });
      timer_due || signal_due || correction_due
    });
    if due {
      let _active = ActiveRunGuard::acquire(&state);
      if let Ok(outcomes) = crate::runtime::resume_due_timer_waits_for_workflows_durable(
        state.database_path.clone(),
        state.execution.clone(),
        DURABLE_WAIT_SCAN_LIMIT,
        Some(&workflow_ids),
      )
      .await
      {
        for outcome in outcomes {
          let crate::WorkflowRuntimeOutcome::Succeeded { execution, .. } = outcome else {
            continue;
          };
          let workflow_id = DurableEventStore::open_ready(&state.database_path)
            .and_then(|store| store.run_binding(&execution.run_id))
            .map(|binding| binding.workflow_id);
          if let (Some(reporter), Ok(workflow_id)) = (&state.progress_reporter, workflow_id) {
            reporter(TriggerProgress::RunTerminal {
              contract: TRIGGER_PROGRESS_CONTRACT,
              contract_version: TRIGGER_PROGRESS_CONTRACT_VERSION,
              workflow_id,
              run_id: execution.run_id,
              status: "succeeded",
              failure_code: None,
              occurred_at: Utc::now(),
            });
          }
        }
      }
      if let Ok(resumed) = crate::runtime::resume_due_signal_waits_for_workflows_durable(
        state.database_path.clone(),
        state.execution.clone(),
        DURABLE_WAIT_SCAN_LIMIT,
        &workflow_ids,
      )
      .await
      {
        for resumed in resumed {
          let workflow_id = DurableEventStore::open_ready(&state.database_path)
            .and_then(|store| store.run_binding(&resumed.run_id))
            .map(|binding| binding.workflow_id);
          let Ok(workflow_id) = workflow_id else {
            continue;
          };
          let (status, failure_code) = match resumed.outcome {
            Ok(crate::WorkflowRuntimeOutcome::Succeeded { .. }) => ("succeeded", None),
            Ok(crate::WorkflowRuntimeOutcome::Waiting { .. })
            | Ok(crate::WorkflowRuntimeOutcome::WaitingTime { .. })
            | Ok(crate::WorkflowRuntimeOutcome::WaitingSignal { .. }) => continue,
            Err(error) => ("failed", Some(runtime_failure_code(&error))),
          };
          state.report(TriggerProgress::RunTerminal {
            contract: TRIGGER_PROGRESS_CONTRACT,
            contract_version: TRIGGER_PROGRESS_CONTRACT_VERSION,
            workflow_id,
            run_id: resumed.run_id,
            status,
            failure_code,
            occurred_at: Utc::now(),
          });
        }
      }
      if let Ok(resumed) = crate::runtime::resume_corrections_for_workflows_durable(
        state.database_path.clone(),
        state.execution.clone(),
        crate::CORRECTION_RECOVERY_BATCH_LIMIT,
        &workflow_ids,
      )
      .await
      {
        for resumed in resumed {
          let workflow_id = DurableEventStore::open_ready(&state.database_path)
            .and_then(|store| store.run_binding(&resumed.run_id))
            .map(|binding| binding.workflow_id);
          let Ok(workflow_id) = workflow_id else {
            continue;
          };
          let (status, failure_code) = match resumed.outcome {
            Ok(crate::WorkflowRuntimeOutcome::Succeeded { .. }) => ("succeeded", None),
            Ok(crate::WorkflowRuntimeOutcome::Waiting { .. })
            | Ok(crate::WorkflowRuntimeOutcome::WaitingTime { .. })
            | Ok(crate::WorkflowRuntimeOutcome::WaitingSignal { .. }) => continue,
            Err(error) => ("failed", Some(runtime_failure_code(&error))),
          };
          state.report(TriggerProgress::RunTerminal {
            contract: TRIGGER_PROGRESS_CONTRACT,
            contract_version: TRIGGER_PROGRESS_CONTRACT_VERSION,
            workflow_id,
            run_id: resumed.run_id,
            status,
            failure_code,
            occurred_at: Utc::now(),
          });
        }
      }
    }
    tokio::time::sleep(std::time::Duration::from_millis(
      DURABLE_WAIT_SCAN_MILLISECONDS,
    ))
    .await;
  }
}
