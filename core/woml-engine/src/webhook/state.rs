//! Private webhook state.

use crate::webhook::EventSubscriber;
use crate::webhook::IntervalRuntimeRegistration;
use crate::webhook::ScheduleRuntimeRegistration;
use crate::webhook::TriggerProgress;
use crate::webhook::TriggerProgressReporter;
use crate::webhook::WebhookRoute;
use crate::webhook::WhatsAppSubscriber;
use crate::webhook::TRIGGER_PROGRESS_CONTRACT;
use crate::webhook::TRIGGER_PROGRESS_CONTRACT_VERSION;
use crate::DurableEventStore;
use crate::EventServiceSubscriber;
use crate::RuntimeExecutionOptions;
use crate::WorkflowTargetRegistry;
use chrono::DateTime;
use chrono::Utc;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::sync::Notify;

pub(in crate::webhook) struct WebhookRuntimeState {
  pub(in crate::webhook) bind_address: SocketAddr,
  pub(in crate::webhook) database_path: PathBuf,
  pub(in crate::webhook) routes: HashMap<String, Arc<WebhookRoute>>,
  pub(in crate::webhook) event_subscribers: BTreeMap<String, Vec<EventServiceSubscriber>>,
  pub(in crate::webhook) event_token_digests: BTreeMap<String, [u8; 32]>,
  pub(in crate::webhook) whatsapp_subscribers: BTreeMap<String, Vec<WhatsAppSubscriber>>,
  pub(in crate::webhook) whatsapp_verify_token: Option<String>,
  pub(in crate::webhook) whatsapp_app_secret: Option<String>,
  pub(in crate::webhook) signal_token_digest: Option<[u8; 32]>,
  pub(in crate::webhook) signal_rate_window: Mutex<(DateTime<Utc>, u32)>,
  pub(in crate::webhook) registration_count: usize,
  pub(in crate::webhook) execution: RuntimeExecutionOptions,
  pub(in crate::webhook) progress_reporter: Option<TriggerProgressReporter>,
  pub(in crate::webhook) schedules: Vec<ScheduleRuntimeRegistration>,
  pub(in crate::webhook) intervals: Vec<IntervalRuntimeRegistration>,
  pub(in crate::webhook) workflow_targets: Arc<WorkflowTargetRegistry>,
  pub(in crate::webhook) workflow_routing_credential: String,
  pub(in crate::webhook) admission_open: AtomicBool,
  pub(in crate::webhook) active_runs: Arc<AtomicUsize>,
  pub(in crate::webhook) active_runs_changed: Arc<Notify>,
}

#[derive(Clone)]
pub(in crate::webhook) struct RunProgressIdentity {
  pub(in crate::webhook) workflow_id: String,
  pub(in crate::webhook) trigger_id: String,
  pub(in crate::webhook) trigger_handler: String,
  pub(in crate::webhook) occurrence_id: String,
  pub(in crate::webhook) run_id: String,
}

pub(in crate::webhook) struct ActiveRunGuard {
  pub(in crate::webhook) active_runs: Arc<AtomicUsize>,
  pub(in crate::webhook) active_runs_changed: Arc<Notify>,
}

impl ActiveRunGuard {
  pub(in crate::webhook) fn acquire(state: &WebhookRuntimeState) -> Self {
    state.active_runs.fetch_add(1, Ordering::AcqRel);
    Self {
      active_runs: Arc::clone(&state.active_runs),
      active_runs_changed: Arc::clone(&state.active_runs_changed),
    }
  }
}

impl Drop for ActiveRunGuard {
  fn drop(&mut self) {
    self.active_runs.fetch_sub(1, Ordering::AcqRel);
    self.active_runs_changed.notify_waiters();
  }
}

pub(in crate::webhook) struct StartupManualTrigger {
  pub(in crate::webhook) workflow_id: String,
  pub(in crate::webhook) definition_hash: String,
  pub(in crate::webhook) trigger_id: String,
}

impl WebhookRuntimeState {
  pub(in crate::webhook) fn report(&self, progress: TriggerProgress) {
    if let Some(reporter) = &self.progress_reporter {
      reporter(progress);
    }
  }

  pub(in crate::webhook) fn report_accepted(
    &self,
    identity: &RunProgressIdentity,
    duplicate: bool,
  ) {
    self.report(TriggerProgress::OccurrenceAccepted {
      contract: TRIGGER_PROGRESS_CONTRACT,
      contract_version: TRIGGER_PROGRESS_CONTRACT_VERSION,
      workflow_id: identity.workflow_id.clone(),
      trigger_id: identity.trigger_id.clone(),
      trigger_handler: identity.trigger_handler.clone(),
      occurrence_id: identity.occurrence_id.clone(),
      run_id: identity.run_id.clone(),
      duplicate,
      occurred_at: Utc::now(),
    });
  }

  pub(in crate::webhook) fn report_run_started(&self, identity: &RunProgressIdentity) {
    // Event v11 reports the real scheduler start through Runtime Policy
    // Progress. Do not claim that a newly admitted queued run has started.
    if DurableEventStore::open_ready(&self.database_path)
      .and_then(|store| store.projection(&identity.run_id))
      .is_ok_and(|projection| projection.status == crate::RunStatus::Queued)
    {
      return;
    }
    self.report(TriggerProgress::RunStarted {
      contract: TRIGGER_PROGRESS_CONTRACT,
      contract_version: TRIGGER_PROGRESS_CONTRACT_VERSION,
      workflow_id: identity.workflow_id.clone(),
      trigger_id: identity.trigger_id.clone(),
      trigger_handler: identity.trigger_handler.clone(),
      occurrence_id: identity.occurrence_id.clone(),
      run_id: identity.run_id.clone(),
      occurred_at: Utc::now(),
    });
  }

  pub(in crate::webhook) fn report_rejected(
    &self,
    route: Option<&WebhookRoute>,
    code: &'static str,
    message: &'static str,
  ) {
    self.report(TriggerProgress::OccurrenceRejected {
      contract: TRIGGER_PROGRESS_CONTRACT,
      contract_version: TRIGGER_PROGRESS_CONTRACT_VERSION,
      workflow_id: route.map(|route| route.workflow_id.clone()),
      trigger_id: route.map(|route| route.trigger_id.clone()),
      trigger_handler: "trigger.webhook".to_string(),
      code: code.to_string(),
      message: message.to_string(),
      occurred_at: Utc::now(),
    });
  }

  pub(in crate::webhook) fn report_event_rejected(
    &self,
    subscriber: Option<&EventSubscriber>,
    code: &'static str,
    message: &'static str,
  ) {
    self.report(TriggerProgress::OccurrenceRejected {
      contract: TRIGGER_PROGRESS_CONTRACT,
      contract_version: TRIGGER_PROGRESS_CONTRACT_VERSION,
      workflow_id: subscriber.map(|subscriber| subscriber.workflow_id.clone()),
      trigger_id: subscriber.map(|subscriber| subscriber.trigger_id.clone()),
      trigger_handler: "trigger.event".to_string(),
      code: code.to_string(),
      message: message.to_string(),
      occurred_at: Utc::now(),
    });
  }
}
