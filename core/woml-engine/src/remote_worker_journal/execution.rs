//! One prepared remote attempt, supervised host, cancellation and active-slot Drop.
//!
//! Terminal replay precedes drain rejection. Start is journalled immediately
//! before the execute frame; unknown started outcomes retain ambiguity.

use crate::host::ScriptHostClient;
use crate::host::ScriptHostProcessOptions;
use crate::protocol::ExecuteMessage;
use crate::protocol::HostOutcome;
use crate::protocol::RuntimeModuleBinding;
use crate::protocol::ScriptAttempt;
use crate::remote_worker_journal::maximum_attempts;
use crate::remote_worker_journal::redact_json_value;
use crate::remote_worker_journal::script_source;
use crate::remote_worker_journal::PreparedRemoteDispatchV1;
use crate::remote_worker_journal::RemoteSecretProvider;
use crate::remote_worker_journal::RemoteWorkerCancellationTokenV1;
use crate::remote_worker_journal::RemoteWorkerError;
use crate::remote_worker_journal::RemoteWorkerTerminalV1;
use crate::remote_worker_journal::WorkerAttemptStateV1;
use crate::remote_worker_journal::WorkerJournalV1;
use crate::remote_worker_journal::REMOTE_CANCELLATION_CONFIRMATION_TIMEOUT_MS;
use crate::DurableCapabilityAuthority;
use chrono::DateTime;
use chrono::Utc;
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub struct RemoteWorkerAttemptRuntimeV1 {
  pub(in crate::remote_worker_journal) journal: WorkerJournalV1,
  pub(in crate::remote_worker_journal) worker_id: String,
  pub(in crate::remote_worker_journal) session_id: String,
  pub(in crate::remote_worker_journal) capacity: usize,
  pub(in crate::remote_worker_journal) active: Arc<AtomicUsize>,
  pub(in crate::remote_worker_journal) draining: Arc<AtomicBool>,
  pub(in crate::remote_worker_journal) host_options: ScriptHostProcessOptions,
  pub(in crate::remote_worker_journal) capability_authority:
    Option<Arc<DurableCapabilityAuthority>>,
  pub(in crate::remote_worker_journal) secret_provider: Arc<dyn RemoteSecretProvider>,
}

impl RemoteWorkerAttemptRuntimeV1 {
  #[allow(clippy::too_many_arguments)]
  pub fn new(
    journal: WorkerJournalV1,
    worker_id: impl Into<String>,
    session_id: impl Into<String>,
    capacity: usize,
    host_options: ScriptHostProcessOptions,
    capability_authority: Option<Arc<DurableCapabilityAuthority>>,
    secret_provider: Arc<dyn RemoteSecretProvider>,
  ) -> Result<Self, RemoteWorkerError> {
    if capacity == 0 || capacity > crate::WORKER_PROTOCOL_MAX_CAPACITY as usize {
      return Err(RemoteWorkerError::Capacity);
    }
    Ok(Self {
      journal,
      worker_id: worker_id.into(),
      session_id: session_id.into(),
      capacity,
      active: Arc::new(AtomicUsize::new(0)),
      draining: Arc::new(AtomicBool::new(false)),
      host_options,
      capability_authority,
      secret_provider,
    })
  }

  pub fn begin_drain(&self) {
    self.draining.store(true, Ordering::Release);
  }

  pub fn is_drained(&self) -> bool {
    self.active.load(Ordering::Acquire) == 0
  }

  pub fn journal(&self) -> &WorkerJournalV1 {
    &self.journal
  }

  pub async fn execute(
    &mut self,
    prepared: &PreparedRemoteDispatchV1,
    now: DateTime<Utc>,
  ) -> Result<RemoteWorkerTerminalV1, RemoteWorkerError> {
    self
      .execute_with_cancellation_mode(
        prepared,
        now,
        RemoteWorkerCancellationTokenV1::default(),
        false,
      )
      .await
  }

  pub async fn execute_prestarted(
    &mut self,
    prepared: &PreparedRemoteDispatchV1,
    now: DateTime<Utc>,
  ) -> Result<RemoteWorkerTerminalV1, RemoteWorkerError> {
    self
      .execute_with_cancellation_mode(
        prepared,
        now,
        RemoteWorkerCancellationTokenV1::default(),
        true,
      )
      .await
  }

  pub async fn execute_with_cancellation(
    &mut self,
    prepared: &PreparedRemoteDispatchV1,
    now: DateTime<Utc>,
    cancellation: RemoteWorkerCancellationTokenV1,
  ) -> Result<RemoteWorkerTerminalV1, RemoteWorkerError> {
    self
      .execute_with_cancellation_mode(prepared, now, cancellation, false)
      .await
  }

  pub(in crate::remote_worker_journal) async fn execute_with_cancellation_mode(
    &mut self,
    prepared: &PreparedRemoteDispatchV1,
    now: DateTime<Utc>,
    cancellation: RemoteWorkerCancellationTokenV1,
    prestarted: bool,
  ) -> Result<RemoteWorkerTerminalV1, RemoteWorkerError> {
    if prepared.worker_id != self.worker_id {
      return Err(RemoteWorkerError::Conflict);
    }
    if self.capability_authority.is_none()
      && prepared
        .effective_authority
        .capabilities
        .iter()
        .any(|capability| capability.id != "runtime.script")
    {
      return Err(RemoteWorkerError::CapabilityUnavailable);
    }
    if let Some(terminal) = self
      .journal
      .terminal_for_resend(&prepared.package.dispatch_id)?
    {
      return Ok(terminal);
    }
    if self.draining.load(Ordering::Acquire) {
      return Err(RemoteWorkerError::Draining);
    }
    let active = self.active.fetch_add(1, Ordering::AcqRel);
    if active >= self.capacity {
      self.active.fetch_sub(1, Ordering::AcqRel);
      return Err(RemoteWorkerError::Capacity);
    }
    let _slot = ActiveSlot(Arc::clone(&self.active));
    let secrets = prepared
      .effective_authority
      .secret_names
      .iter()
      .map(|name| {
        self
          .secret_provider
          .resolve(name)
          .map(|value| (name.clone(), value))
      })
      .collect::<Result<BTreeMap<_, _>, _>>()?;
    let sensitive_values = secrets.values().cloned().collect::<Vec<_>>();
    let source = script_source(&prepared.package.node)
      .ok_or(RemoteWorkerError::InvalidPackage)?
      .to_string();
    let timeout_ms = prepared.package.node.timeout_ms.unwrap_or(30_000).min(
      (prepared.package.deadline - now)
        .num_milliseconds()
        .try_into()
        .unwrap_or(0),
    );
    if timeout_ms == 0 {
      return Err(RemoteWorkerError::Expired);
    }
    let accepted = self
      .journal
      .accept(prepared, &self.worker_id, &self.session_id, now)?;
    if let Some(terminal) = accepted.terminal {
      return Ok(terminal);
    }
    if accepted.state == WorkerAttemptStateV1::Ambiguous {
      return Err(RemoteWorkerError::Ambiguous);
    }
    if prestarted {
      if accepted.state != WorkerAttemptStateV1::Started
        || !self
          .journal
          .was_started_by_session(&prepared.package.dispatch_id, &self.session_id)?
      {
        return Err(RemoteWorkerError::Ambiguous);
      }
    } else if accepted.state == WorkerAttemptStateV1::Started {
      return Err(RemoteWorkerError::Ambiguous);
    }
    if cancellation.is_cancelled() {
      self
        .journal
        .start(&prepared.package.dispatch_id, &self.session_id, now)?;
      let terminal = RemoteWorkerTerminalV1::new(
        &prepared.package.dispatch_id,
        &prepared.package.invocation_id,
        WorkerAttemptStateV1::Cancelled,
        json!({"code": "WOML_RUN_CANCELLED", "message": "Remote execution was cancelled before script execution."}),
        Utc::now(),
      )?;
      self
        .journal
        .terminal(&prepared.package.dispatch_id, &self.session_id, &terminal)?;
      return Ok(terminal);
    }
    let mut options = self.host_options.clone();
    options.module_artifacts.clear();
    let host = ScriptHostClient::spawn_with_authority(options, self.capability_authority.clone())
      .await
      .map_err(|_| RemoteWorkerError::HostUnavailable)?;
    let module_bindings: Vec<RuntimeModuleBinding> = Vec::new();
    let attempt = ScriptAttempt::new(
      prepared.package.attempt,
      maximum_attempts(&prepared.package),
      &prepared.package.package_digest,
    )
    .map_err(|_| RemoteWorkerError::InvalidPackage)?;
    // Everything above this point is preflight and may be retried safely. The
    // durable start fact is committed immediately before the first script-host
    // execute frame, which is the boundary after which effects are ambiguous.
    if !prestarted {
      if let Err(error) = self
        .journal
        .start(&prepared.package.dispatch_id, &self.session_id, now)
      {
        host.shutdown().await;
        return Err(error);
      }
    }
    let message = ExecuteMessage::runtime_script_with_modules(
      &prepared.package.invocation_id,
      &prepared.package.run_id,
      &prepared.package.node_id,
      attempt,
      timeout_ms,
      &source,
      &prepared.package.context,
      &secrets,
      &module_bindings,
    );
    let completion = {
      let execution = host.execute(&message);
      tokio::pin!(execution);
      loop {
        tokio::select! {
          completion = &mut execution => break completion,
          _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {
            if cancellation.is_cancelled() {
              if host.cancel_run(&prepared.package.invocation_id).await.is_err() {
                break Err(crate::host::ScriptHostClientError::HostCrashed(
                  "remote cancellation could not be delivered".to_string(),
                ));
              }
              break match tokio::time::timeout(
                std::time::Duration::from_millis(REMOTE_CANCELLATION_CONFIRMATION_TIMEOUT_MS),
                &mut execution,
              ).await {
                Ok(completion) => completion,
                Err(_) => Err(crate::host::ScriptHostClientError::HostCrashed(
                  "remote cancellation was not confirmed".to_string(),
                )),
              };
            }
          }
        }
      }
    };
    host.shutdown().await;
    let (state, mut outcome) = match completion {
      Ok(completed) => match completed.outcome {
        HostOutcome::Success { value } => (WorkerAttemptStateV1::Succeeded, value),
        HostOutcome::Failure { error } => (
          if error.kind == crate::protocol::HostReportedFailureKind::InvocationCancelled {
            WorkerAttemptStateV1::Cancelled
          } else {
            WorkerAttemptStateV1::Failed
          },
          json!({"code": error.code, "message": error.message, "kind": error.kind}),
        ),
      },
      Err(_) => return Err(RemoteWorkerError::Ambiguous),
    };
    redact_json_value(&mut outcome, &sensitive_values);
    let terminal = RemoteWorkerTerminalV1::new(
      &prepared.package.dispatch_id,
      &prepared.package.invocation_id,
      state,
      outcome,
      Utc::now(),
    )?;
    self
      .journal
      .terminal(&prepared.package.dispatch_id, &self.session_id, &terminal)?;
    Ok(terminal)
  }
}

pub(in crate::remote_worker_journal) struct ActiveSlot(
  pub(in crate::remote_worker_journal) Arc<AtomicUsize>,
);

impl Drop for ActiveSlot {
  fn drop(&mut self) {
    self.0.fetch_sub(1, Ordering::AcqRel);
  }
}
