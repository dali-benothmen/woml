//! Private webhook dispatch.

use crate::execute_admitted_trigger_run_durable;
use crate::webhook::RunProgressIdentity;
use crate::webhook::TriggerProgress;
use crate::webhook::WebhookRuntimeState;
use crate::webhook::TRIGGER_PROGRESS_CONTRACT;
use crate::webhook::TRIGGER_PROGRESS_CONTRACT_VERSION;
use crate::DurableEventStore;
use crate::RuntimeExecutionOptions;
use chrono::Utc;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub(in crate::webhook) fn dispatch_run(state: &WebhookRuntimeState, identity: RunProgressIdentity) {
  let database_path = state.database_path.clone();
  let execution = state.execution.clone();
  let reporter = state.progress_reporter.clone();
  let active_runs = Arc::clone(&state.active_runs);
  let active_runs_changed = Arc::clone(&state.active_runs_changed);
  active_runs.fetch_add(1, Ordering::AcqRel);
  actix_web::rt::spawn(async move {
    let mut span = crate::performance::PerformanceSpan::new("runtime", "runtime.dispatch_run");
    span.run_id(identity.run_id.clone());
    let result =
      execute_trigger_run_with_builtin_notifications(database_path, &identity.run_id, execution)
        .await;
    if result.is_ok() {
      span.succeed();
    }
    let progress = match result {
      Ok(crate::WorkflowRuntimeOutcome::Succeeded { .. }) => Some(TriggerProgress::RunTerminal {
        contract: TRIGGER_PROGRESS_CONTRACT,
        contract_version: TRIGGER_PROGRESS_CONTRACT_VERSION,
        workflow_id: identity.workflow_id,
        run_id: identity.run_id,
        status: "succeeded",
        failure_code: None,
        occurred_at: Utc::now(),
      }),
      Ok(crate::WorkflowRuntimeOutcome::Waiting { .. })
      | Ok(crate::WorkflowRuntimeOutcome::WaitingTime { .. })
      | Ok(crate::WorkflowRuntimeOutcome::WaitingSignal { .. }) => None,
      Err(error) => Some(TriggerProgress::RunTerminal {
        contract: TRIGGER_PROGRESS_CONTRACT,
        contract_version: TRIGGER_PROGRESS_CONTRACT_VERSION,
        workflow_id: identity.workflow_id,
        run_id: identity.run_id,
        status: "failed",
        failure_code: Some(runtime_failure_code(&error)),
        occurred_at: Utc::now(),
      }),
    };
    if let (Some(reporter), Some(progress)) = (reporter, progress) {
      reporter(progress);
    }
    active_runs.fetch_sub(1, Ordering::AcqRel);
    active_runs_changed.notify_waiters();
  });
}

async fn execute_trigger_run_with_builtin_notifications(
  database_path: PathBuf,
  run_id: &str,
  execution: RuntimeExecutionOptions,
) -> Result<crate::WorkflowRuntimeOutcome, crate::RuntimeExecutionError> {
  loop {
    let outcome =
      execute_admitted_trigger_run_durable(database_path.clone(), run_id, execution.clone())
        .await?;
    let crate::WorkflowRuntimeOutcome::Waiting { approval, .. } = &outcome else {
      return Ok(outcome);
    };

    let store = DurableEventStore::open(&database_path)?;
    let binding = store.run_binding(run_id)?;
    let workflow = store.definition(&binding.definition_hash)?;
    let Some(definition) = workflow.approval(&approval.approval_id) else {
      return Err(crate::RuntimeExecutionError::InvalidConfiguration(
        "The waiting approval is missing from its immutable workflow definition.".to_string(),
      ));
    };
    if definition.notifications.is_empty()
      || definition.notifications.iter().any(|delivery| {
        !matches!(
          delivery.provider.as_str(),
          "slack" | "telegram" | "discord" | "whatsapp"
        )
      })
    {
      return Ok(outcome);
    }
    let Some(host) = execution.notification_host.clone() else {
      return Err(crate::RuntimeExecutionError::InvalidConfiguration(
        "The built-in notification provider host is unavailable.".to_string(),
      ));
    };
    let wait = approval
      .expires_at
      .and_then(|expires_at| (expires_at - Utc::now()).to_std().ok())
      .unwrap_or(std::time::Duration::from_secs(u32::MAX.into()))
      .max(std::time::Duration::from_millis(1));
    if let Err(error) =
      crate::run_notification_provider_journey(&database_path, run_id, host, wait).await
    {
      let store = DurableEventStore::open(&database_path)?;
      if store.projection(run_id)?.status == crate::RunStatus::Failed {
        return execute_admitted_trigger_run_durable(database_path, run_id, execution).await;
      }
      return Err(crate::RuntimeExecutionError::InvalidConfiguration(format!(
        "The notification provider journey stopped before the approval was resolved: {error}"
      )));
    }
  }
}

pub(in crate::webhook) async fn wait_for_active_runs(
  state: &WebhookRuntimeState,
  deadline: std::time::Duration,
) {
  let wait = async {
    while state.active_runs.load(Ordering::Acquire) > 0 {
      state.active_runs_changed.notified().await;
    }
  };
  let _ = tokio::time::timeout(deadline, wait).await;
}

pub(in crate::webhook) fn runtime_failure_code(error: &crate::RuntimeExecutionError) -> String {
  match error {
    crate::RuntimeExecutionError::RunFailed(details) => details.code.clone(),
    crate::RuntimeExecutionError::BranchFailed(details) => details.code.clone(),
    crate::RuntimeExecutionError::ParallelFailed(details) => details.code.clone(),
    crate::RuntimeExecutionError::ApprovalFailed(details) => details.code.clone(),
    crate::RuntimeExecutionError::NotificationFailed(details) => details.code.clone(),
    crate::RuntimeExecutionError::RunCancelled(details) => details.code.clone(),
    _ => "WOML_TRIGGER_EXECUTION_FAILED".to_string(),
  }
}
