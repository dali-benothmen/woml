//! Policy capacity, durable ownership leases, heartbeats, and local wakeups.
//!
//! Suspension releases active capacity without changing run identity. Wakeups
//! do not replace the store's cross-process claim transaction. Preserve lease
//! ownership, heartbeat cancellation, and release ordering across every await.

use crate::runtime::errors::RuntimeExecutionError;
use crate::runtime::modules::runtime_modules_from_store;
use crate::runtime::options::RuntimeExecutionOptions;
use crate::runtime::outcomes::WorkflowRuntimeOutcome;
use crate::runtime::progress::RuntimePolicyProgress;
use crate::runtime::progress::RuntimePolicyProgressPhase;
use crate::runtime::progress::RUNTIME_POLICY_PROGRESS_PROFILE;
use crate::runtime::{
  recover_operational_run, resume_with_engine, schedule_workflow_timeout,
  validate_rich_approval_authority,
};
use crate::DurableDagEngine;
use crate::DurableEventStore;
use crate::DurableStoreError;
use crate::PolicyClaimWaitReason;
use crate::PolicyExecutionClaimResult;
use crate::PolicyWaitingFor;
use crate::RunStatus;
use crate::SchedulerClaimV1;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;
use uuid::Uuid;

pub(crate) type PolicyExecutionRegistry =
  Arc<tokio::sync::RwLock<HashMap<String, std::sync::Weak<PolicyExecutionCoordinator>>>>;

impl RuntimeExecutionOptions {
  pub(in crate::runtime) async fn release_policy_execution_slot(
    &self,
    store: &mut DurableEventStore,
  ) -> Result<(), RuntimeExecutionError> {
    let Some(coordinator) = &self.policy_execution else {
      return Ok(());
    };
    let lease = coordinator.lease.lock().await.take();
    if let Some(lease) = lease {
      lease.release(store).await?;
    }
    Ok(())
  }
  pub(crate) async fn suspend_policy_execution_slot(&self) -> Result<(), RuntimeExecutionError> {
    let Some(coordinator) = &self.policy_execution else {
      return Ok(());
    };
    coordinator.suspend().await
  }
  pub(crate) fn policy_execution_registry(&self) -> PolicyExecutionRegistry {
    Arc::clone(&self.policy_execution_registry)
  }
  pub(in crate::runtime) async fn ensure_policy_execution_slot(
    &self,
  ) -> Result<(), RuntimeExecutionError> {
    let Some(coordinator) = &self.policy_execution else {
      return Ok(());
    };
    let mut current = coordinator.lease.lock().await;
    if let Some(lease) = current.as_mut() {
      lease.resume().await?;
      return Ok(());
    }
    match acquire_policy_execution_lease(&coordinator.database_path, &coordinator.run_id, self)
      .await?
    {
      PolicyClaimAcquisition::Claimed(lease) => {
        *current = Some(lease);
        Ok(())
      }
      PolicyClaimAcquisition::Recovered => Err(RuntimeExecutionError::Stalled(format!(
        "run {:?} required fail-closed recovery while reacquiring a policy slot",
        coordinator.run_id
      ))),
    }
  }
}

const POLICY_CLAIM_LEASE: Duration = Duration::from_secs(15);

const POLICY_CLAIM_HEARTBEAT: Duration = Duration::from_secs(5);

const POLICY_QUEUE_RECHECK: Duration = Duration::from_millis(250);

pub(in crate::runtime) fn policy_wakeup(
  database_path: &std::path::Path,
) -> Arc<tokio::sync::Notify> {
  static WAKEUPS: OnceLock<
    std::sync::Mutex<HashMap<PathBuf, std::sync::Weak<tokio::sync::Notify>>>,
  > = OnceLock::new();
  let wakeups = WAKEUPS.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
  let mut wakeups = wakeups
    .lock()
    .unwrap_or_else(|poisoned| poisoned.into_inner());
  if let Some(wakeup) = wakeups
    .get(database_path)
    .and_then(std::sync::Weak::upgrade)
  {
    return wakeup;
  }
  let wakeup = Arc::new(tokio::sync::Notify::new());
  wakeups.insert(database_path.to_path_buf(), Arc::downgrade(&wakeup));
  wakeup
}

async fn wait_for_policy_wakeup(database_path: &std::path::Path) {
  let wakeup = policy_wakeup(database_path);
  tokio::select! {
    _ = wakeup.notified() => {}
    _ = tokio::time::sleep(POLICY_QUEUE_RECHECK) => {}
  }
}

async fn wait_for_policy_rate_eligibility(
  database_path: &std::path::Path,
  eligible_at: chrono::DateTime<chrono::Utc>,
  policy_now: chrono::DateTime<chrono::Utc>,
) {
  let wakeup = policy_wakeup(database_path);
  let wait = (eligible_at - policy_now)
    .to_std()
    .unwrap_or(Duration::ZERO);
  tokio::select! {
    _ = wakeup.notified() => {}
    _ = tokio::time::sleep(wait) => {}
  }
}

pub(in crate::runtime) struct PolicyExecutionLease {
  pub(in crate::runtime) database_path: PathBuf,
  pub(in crate::runtime) claim: SchedulerClaimV1,
  pub(in crate::runtime) stop: Option<tokio::sync::oneshot::Sender<()>>,
  pub(in crate::runtime) heartbeat: tokio::task::JoinHandle<()>,
  pub(in crate::runtime) lost: Arc<AtomicBool>,
  pub(in crate::runtime) suspended: bool,
}

pub(crate) struct PolicyExecutionCoordinator {
  pub(in crate::runtime) database_path: PathBuf,
  pub(in crate::runtime) run_id: String,
  pub(in crate::runtime) lease: tokio::sync::Mutex<Option<PolicyExecutionLease>>,
}

impl PolicyExecutionCoordinator {
  pub(crate) async fn suspend(&self) -> Result<(), RuntimeExecutionError> {
    let mut lease = self.lease.lock().await;
    if let Some(lease) = lease.as_mut() {
      lease.suspend()?;
    }
    Ok(())
  }

  pub(crate) async fn resume(&self) -> Result<(), RuntimeExecutionError> {
    let mut lease = self.lease.lock().await;
    if let Some(lease) = lease.as_mut() {
      lease.resume().await?;
    }
    Ok(())
  }
}

impl PolicyExecutionLease {
  fn start(database_path: PathBuf, claim: SchedulerClaimV1) -> Self {
    let (stop, mut stopped) = tokio::sync::oneshot::channel();
    let heartbeat_path = database_path.clone();
    let heartbeat_claim = claim.clone();
    let lost = Arc::new(AtomicBool::new(false));
    let heartbeat_lost = Arc::clone(&lost);
    let heartbeat = tokio::spawn(async move {
      loop {
        tokio::select! {
          _ = &mut stopped => break,
          _ = tokio::time::sleep(POLICY_CLAIM_HEARTBEAT) => {
            let renewed = DurableEventStore::open(heartbeat_path.clone()).and_then(|mut store| {
              store.renew_policy_claim(
                &heartbeat_claim.run_id,
                &heartbeat_claim.owner_id,
                &heartbeat_claim.claim_id,
                chrono::Utc::now(),
                POLICY_CLAIM_LEASE,
              )
            });
            if renewed.is_err() {
              heartbeat_lost.store(true, Ordering::Release);
              break;
            }
          }
        }
      }
    });
    Self {
      database_path,
      claim,
      stop: Some(stop),
      heartbeat,
      lost,
      suspended: false,
    }
  }

  fn suspend(&mut self) -> Result<(), RuntimeExecutionError> {
    if self.suspended {
      return Ok(());
    }
    let mut store = DurableEventStore::open(self.database_path.clone())?;
    if !store.suspend_policy_claim(
      &self.claim.run_id,
      &self.claim.owner_id,
      &self.claim.claim_id,
    )? {
      return Err(RuntimeExecutionError::Stalled(format!(
        "scheduler ownership was lost for run {:?}",
        self.claim.run_id
      )));
    }
    self.suspended = true;
    policy_wakeup(&self.database_path).notify_waiters();
    Ok(())
  }

  async fn resume(&mut self) -> Result<(), RuntimeExecutionError> {
    while self.suspended {
      if self.lost.load(Ordering::Acquire) {
        return Err(RuntimeExecutionError::Stalled(format!(
          "scheduler ownership was lost for run {:?}",
          self.claim.run_id
        )));
      }
      let resumed = DurableEventStore::open(self.database_path.clone())?.resume_policy_claim(
        &self.claim.run_id,
        &self.claim.owner_id,
        &self.claim.claim_id,
        chrono::Utc::now(),
      )?;
      if resumed {
        self.suspended = false;
        break;
      }
      wait_for_policy_wakeup(&self.database_path).await;
    }
    Ok(())
  }

  async fn release(mut self, store: &mut DurableEventStore) -> Result<(), RuntimeExecutionError> {
    if let Some(stop) = self.stop.take() {
      let _ = stop.send(());
    }
    let _ = self.heartbeat.await;
    let released = store.release_policy_claim(
      &self.claim.run_id,
      &self.claim.owner_id,
      &self.claim.claim_id,
    )?;
    if !released || self.lost.load(Ordering::Acquire) {
      return Err(RuntimeExecutionError::Stalled(format!(
        "scheduler ownership was lost for run {:?}",
        self.claim.run_id
      )));
    }
    policy_wakeup(&self.database_path).notify_waiters();
    Ok(())
  }
}

pub(in crate::runtime) enum PolicyClaimAcquisition {
  Claimed(PolicyExecutionLease),
  Recovered,
}

async fn acquire_policy_execution_lease(
  database_path: &std::path::Path,
  run_id: &str,
  options: &RuntimeExecutionOptions,
) -> Result<PolicyClaimAcquisition, RuntimeExecutionError> {
  let mut store = DurableEventStore::open(database_path.to_path_buf())?;
  acquire_policy_execution_lease_with_store(database_path, run_id, options, &mut store).await
}

pub(in crate::runtime) async fn acquire_policy_execution_lease_with_store(
  database_path: &std::path::Path,
  run_id: &str,
  options: &RuntimeExecutionOptions,
  store: &mut DurableEventStore,
) -> Result<PolicyClaimAcquisition, RuntimeExecutionError> {
  let mut span =
    crate::performance::PerformanceSpan::new("runtime", "runtime.acquire_policy_lease");
  span.run_id(run_id.to_string());
  let result =
    acquire_policy_execution_lease_unprofiled(database_path, run_id, options, store).await;
  if result.is_ok() {
    span.succeed();
  }
  result
}

async fn acquire_policy_execution_lease_unprofiled(
  database_path: &std::path::Path,
  run_id: &str,
  options: &RuntimeExecutionOptions,
  store: &mut DurableEventStore,
) -> Result<PolicyClaimAcquisition, RuntimeExecutionError> {
  let owner_id = format!("scheduler_{}", Uuid::new_v4().simple());
  let mut reported_wait = false;
  loop {
    let lease_now = chrono::Utc::now();
    let policy_now = options.clock.now();
    let rate_eligible_at = match store.try_claim_policy_run_at(
      run_id,
      &owner_id,
      lease_now,
      policy_now,
      POLICY_CLAIM_LEASE,
    ) {
      Ok(PolicyExecutionClaimResult::Claimed { claim, .. }) => {
        let binding = store.run_binding(run_id)?;
        let queue = store
          .definition(&binding.definition_hash)?
          .runtime_policy_queue_name()
          .ok_or_else(|| {
            RuntimeExecutionError::InvalidConfiguration(
              "Model v12 has no runtime policy queue".to_string(),
            )
          })?;
        options.report_runtime_policy(RuntimePolicyProgress {
          profile: RUNTIME_POLICY_PROGRESS_PROFILE,
          run_id: run_id.to_string(),
          workflow_id: claim.workflow_id.clone(),
          phase: RuntimePolicyProgressPhase::Eligible,
          queue: queue.clone(),
          waiting_for: None,
          eligible_at: None,
          code: None,
        });
        options.report_runtime_policy(RuntimePolicyProgress {
          profile: RUNTIME_POLICY_PROGRESS_PROFILE,
          run_id: run_id.to_string(),
          workflow_id: claim.workflow_id.clone(),
          phase: RuntimePolicyProgressPhase::Started,
          queue,
          waiting_for: None,
          eligible_at: None,
          code: None,
        });
        return Ok(PolicyClaimAcquisition::Claimed(
          PolicyExecutionLease::start(database_path.to_path_buf(), claim),
        ));
      }
      Ok(PolicyExecutionClaimResult::Waiting {
        workflow_id,
        queue,
        reason,
        eligible_at,
        ..
      }) => {
        let waiting_for = match reason {
          PolicyClaimWaitReason::Concurrency => PolicyWaitingFor::Concurrency,
          PolicyClaimWaitReason::RateLimit => PolicyWaitingFor::RateLimit,
        };
        if !reported_wait {
          options.report_runtime_policy(RuntimePolicyProgress {
            profile: RUNTIME_POLICY_PROGRESS_PROFILE,
            run_id: run_id.to_string(),
            workflow_id,
            phase: RuntimePolicyProgressPhase::Queued,
            queue,
            waiting_for: Some(waiting_for),
            eligible_at,
            code: None,
          });
          reported_wait = true;
        }
        eligible_at
      }
      Err(DurableStoreError::SchedulerRecoveryRequired(_)) => {
        store.recover_policy_run_after_lease_loss(run_id, lease_now)?;
        return Ok(PolicyClaimAcquisition::Recovered);
      }
      Err(error) => return Err(error.into()),
    };
    if let Some(eligible_at) = rate_eligible_at {
      wait_for_policy_rate_eligibility(database_path, eligible_at, policy_now).await;
    } else {
      wait_for_policy_wakeup(database_path).await;
    }
  }
}

pub(in crate::runtime) async fn execute_policy_run_durable(
  database_path: PathBuf,
  run_id: &str,
  options: RuntimeExecutionOptions,
  store: DurableEventStore,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  let mut span = crate::performance::PerformanceSpan::new("runtime", "runtime.execute_policy_run");
  span.run_id(run_id.to_string());
  let result = execute_policy_run_durable_unprofiled(database_path, run_id, options, store).await;
  if result.is_ok() {
    span.succeed();
  }
  result
}

async fn execute_policy_run_durable_unprofiled(
  database_path: PathBuf,
  run_id: &str,
  options: RuntimeExecutionOptions,
  mut store: DurableEventStore,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  {
    let binding = store.run_binding(run_id)?;
    let workflow = store.definition(&binding.definition_hash)?;
    validate_rich_approval_authority(&workflow, &options)?;
    let policy = workflow.runtime_policy.as_ref().ok_or_else(|| {
      RuntimeExecutionError::InvalidConfiguration(
        "Model v12 execution requires a runtime policy".to_string(),
      )
    })?;
    let projection = store.projection(run_id)?;
    if let Some(deadline_at) = projection.timeout_at {
      schedule_workflow_timeout(
        database_path.clone(),
        run_id.to_string(),
        workflow.workflow_id.clone(),
        policy
          .queue
          .as_ref()
          .map_or_else(|| workflow.workflow_id.clone(), |queue| queue.name.clone()),
        deadline_at,
        options.clone(),
      );
    }
    if matches!(
      projection.status,
      RunStatus::Waiting | RunStatus::Succeeded | RunStatus::Failed | RunStatus::Cancelled
    ) {
      let binding = store.run_binding(run_id)?;
      let options = runtime_modules_from_store(options, &store, &binding.definition_hash)?;
      let mut engine = DurableDagEngine::resume(store, run_id)?;
      return resume_with_engine(&mut engine, run_id, options).await;
    }
  }
  let acquisition =
    acquire_policy_execution_lease_with_store(&database_path, run_id, &options, &mut store).await?;
  let binding = store.run_binding(run_id)?;
  let mut options = runtime_modules_from_store(options, &store, &binding.definition_hash)?;
  let workflow = store.definition(&binding.definition_hash)?;
  if let Some(deadline_at) = store.projection(run_id)?.timeout_at {
    let queue = workflow.runtime_policy_queue_name().ok_or_else(|| {
      RuntimeExecutionError::InvalidConfiguration(
        "Model v12 has no runtime policy queue".to_string(),
      )
    })?;
    schedule_workflow_timeout(
      database_path.clone(),
      run_id.to_string(),
      workflow.workflow_id,
      queue,
      deadline_at,
      options.clone(),
    );
  }
  let mut engine = DurableDagEngine::resume(store, run_id)?;
  match acquisition {
    PolicyClaimAcquisition::Recovered => {
      recover_operational_run(&options, run_id)?;
      resume_with_engine(&mut engine, run_id, options).await
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
      let outcome = resume_with_engine(&mut engine, run_id, options.clone()).await;
      let mut store = engine.into_store();
      let released = options.release_policy_execution_slot(&mut store).await;
      options
        .policy_execution_registry
        .write()
        .await
        .remove(run_id);
      match (outcome, released) {
        (Ok(outcome), Ok(())) => Ok(outcome),
        (Err(error), Ok(())) => Err(error),
        (_, Err(error)) => Err(error),
      }
    }
  }
}
