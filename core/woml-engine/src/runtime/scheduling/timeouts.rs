//! Total workflow deadlines and timeout-related lifecycle continuation.
//!
//! Timeout observation does not bypass claims: follow-up lifecycle work returns
//! through the same policy coordinator. A deadline is not proof about effects.

use crate::runtime::errors::RuntimeExecutionError;
use crate::runtime::modules::runtime_modules_from_store;
use crate::runtime::options::RuntimeExecutionOptions;
use crate::runtime::progress::RuntimePolicyProgress;
use crate::runtime::progress::RuntimePolicyProgressPhase;
use crate::runtime::progress::RUNTIME_POLICY_PROGRESS_PROFILE;
use crate::runtime::script_execution::CANCELLATION_POLL_INTERVAL;
use crate::runtime::{
  acquire_policy_execution_lease_with_store, policy_wakeup, resume_with_engine,
  PolicyClaimAcquisition, PolicyExecutionCoordinator, RuntimeDagEngine,
};
use crate::DurableDagEngine;
use crate::DurableEventStore;
use crate::RunTimeoutSettlement;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub(in crate::runtime) fn schedule_workflow_timeout(
  database_path: PathBuf,
  run_id: String,
  workflow_id: String,
  queue: String,
  deadline_at: chrono::DateTime<chrono::Utc>,
  options: RuntimeExecutionOptions,
) {
  tokio::spawn(async move {
    let mut deadline = deadline_at;
    loop {
      let wait = (deadline - options.clock.now())
        .to_std()
        .unwrap_or(Duration::ZERO);
      tokio::time::sleep(wait).await;
      let settlement = DurableEventStore::open(database_path.clone()).and_then(|mut store| {
        let settlement = store.settle_run_timeout(&run_id, options.clock.now())?;
        let needs_lifecycle = matches!(
          &settlement,
          RunTimeoutSettlement::TimedOut { projection }
            if !projection.lifecycle_hooks.is_empty()
              && !store.policy_run_has_live_claim(&run_id, chrono::Utc::now())?
        );
        if matches!(
          &settlement,
          RunTimeoutSettlement::TimedOut { projection }
            if projection.lifecycle_hooks.is_empty()
        ) {
          let _ = store.finalize_run_v11(&run_id, options.clock.now());
        }
        Ok((settlement, needs_lifecycle))
      });
      match settlement {
        Ok((RunTimeoutSettlement::NotDue { deadline_at }, _)) => deadline = deadline_at,
        Ok((RunTimeoutSettlement::TimedOut { .. }, needs_lifecycle)) => {
          if let Some(reporter) = &options.runtime_policy_progress_reporter {
            reporter(RuntimePolicyProgress {
              profile: RUNTIME_POLICY_PROGRESS_PROFILE,
              run_id: run_id.clone(),
              workflow_id: workflow_id.clone(),
              phase: RuntimePolicyProgressPhase::TimedOut,
              queue: queue.clone(),
              waiting_for: None,
              eligible_at: None,
              code: Some("WOML_WORKFLOW_TIMED_OUT".to_string()),
            });
          }
          policy_wakeup(&database_path).notify_waiters();
          if needs_lifecycle {
            let _ =
              finish_timed_out_policy_lifecycle(database_path.clone(), &run_id, options.clone())
                .await;
          }
          break;
        }
        Ok(_) => {
          policy_wakeup(&database_path).notify_waiters();
          break;
        }
        Err(_) => tokio::time::sleep(CANCELLATION_POLL_INTERVAL).await,
      }
    }
  });
}

async fn finish_timed_out_policy_lifecycle(
  database_path: PathBuf,
  run_id: &str,
  options: RuntimeExecutionOptions,
) -> Result<(), RuntimeExecutionError> {
  let mut store = DurableEventStore::open(database_path.clone())?;
  let acquisition =
    acquire_policy_execution_lease_with_store(&database_path, run_id, &options, &mut store).await?;
  let binding = store.run_binding(run_id)?;
  let mut options = runtime_modules_from_store(options, &store, &binding.definition_hash)?;
  let mut engine = DurableDagEngine::resume(store, run_id)?;
  match acquisition {
    PolicyClaimAcquisition::Recovered => {
      let _ = resume_with_engine(&mut engine, run_id, options).await;
    }
    PolicyClaimAcquisition::Claimed(lease) => {
      let coordinator = Arc::new(PolicyExecutionCoordinator {
        database_path: database_path.clone(),
        run_id: run_id.to_string(),
        lease: tokio::sync::Mutex::new(Some(lease)),
      });
      options.policy_execution = Some(Arc::clone(&coordinator));
      options
        .policy_execution_registry
        .write()
        .await
        .insert(run_id.to_string(), Arc::downgrade(&coordinator));
      let _ = resume_with_engine(&mut engine, run_id, options.clone()).await;
      let mut store = engine.into_store();
      let released = options.release_policy_execution_slot(&mut store).await;
      options
        .policy_execution_registry
        .write()
        .await
        .remove(run_id);
      released?;
    }
  }
  Ok(())
}

pub(in crate::runtime) fn settle_workflow_timeout_if_due<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  now: chrono::DateTime<chrono::Utc>,
  options: &RuntimeExecutionOptions,
) -> Result<bool, RuntimeExecutionError> {
  let projection = engine.projection(run_id)?;
  if projection.business_outcome.is_some() || projection.cancellation_request_id.is_some() {
    return Ok(false);
  }
  if projection
    .timeout_at
    .is_none_or(|deadline_at| now < deadline_at)
  {
    return Ok(false);
  }
  let settled = engine.settle_workflow_timeout(run_id, now)?;
  if settled {
    options.report_runtime_policy(RuntimePolicyProgress {
      profile: RUNTIME_POLICY_PROGRESS_PROFILE,
      run_id: run_id.to_string(),
      workflow_id: engine.workflow().workflow_id.clone(),
      phase: RuntimePolicyProgressPhase::TimedOut,
      queue: engine
        .workflow()
        .runtime_policy_queue_name()
        .unwrap_or_else(|| engine.workflow().workflow_id.clone()),
      waiting_for: None,
      eligible_at: None,
      code: Some("WOML_WORKFLOW_TIMED_OUT".to_string()),
    });
  }
  Ok(settled)
}
