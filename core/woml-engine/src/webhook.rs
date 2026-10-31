//! Trigger-host preparation, atomic activation and bounded shutdown.
//!
//! Private ingress and clock producers submit to shared durable admission;
//! dispatch delegates to the workflow runtime rather than scheduling a new DAG.

mod admission;
mod authentication;
mod contracts;
mod dispatch;
mod events;
mod intervals;
mod preparation;
mod responses;
mod routing;
mod schedules;
mod signals;
mod state;
#[cfg(test)]
mod tests;
mod waits;
mod whatsapp;

pub(in crate::webhook) use admission::{
  admit_startup_manual, handle_webhook, run_external_ingress, run_internal_event_dispatch,
};
pub(in crate::webhook) use authentication::{authorized, authorized_bearer, WebhookAuthentication};
pub use contracts::{
  ExternalTriggerAdmissionCommand, ExternalTriggerAdmissionReceiver, TriggerProgress,
  TriggerProgressReporter, WebhookDefinitionRegistration, WebhookRuntimeError,
  WomlWebhookServerConfig, COMMUNICATION_PROVIDER_MAX_BATCH_ITEMS,
  COMMUNICATION_PROVIDER_MAX_MESSAGE_BYTES, EVENT_MAX_SUBSCRIBERS, TRIGGER_PROGRESS_CONTRACT,
  TRIGGER_PROGRESS_CONTRACT_VERSION, WEBHOOK_MAX_BODY_BYTES, WHATSAPP_CALLBACK_PATH,
};
pub(in crate::webhook) use dispatch::{dispatch_run, runtime_failure_code, wait_for_active_runs};
pub(in crate::webhook) use events::handle_event_publication;
pub(in crate::webhook) use intervals::{
  compile_interval, initialize_intervals, IntervalRuntimeRegistration,
};
pub(in crate::webhook) use preparation::{
  literal_string, literal_u64, object_fields, prepare_state, valid_event_name, EventSubscriber,
  WebhookRoute,
};
pub(in crate::webhook) use responses::{
  error_response, has_json_content_type, inbound_trace_context, rejected_response, schema_issue,
  WebhookAcceptedResponse, WebhookErrorBody, WebhookErrorResponse, WebhookSchemaIssue,
};
pub(in crate::webhook) use routing::{
  handle_workflow_call_wakeup, run_workflow_routing_maintenance, WORKFLOW_RUNTIME_LEASE_SECONDS,
};
pub(in crate::webhook) use schedules::{
  compile_schedule, initialize_schedules, ScheduleRuntimeRegistration,
};
pub(in crate::webhook) use signals::handle_external_signal;
pub(in crate::webhook) use state::{
  ActiveRunGuard, RunProgressIdentity, StartupManualTrigger, WebhookRuntimeState,
};
pub(in crate::webhook) use waits::run_durable_wait_scheduler;
pub(in crate::webhook) use whatsapp::{handle_whatsapp_callback, WhatsAppSubscriber};

use crate::workflow_routing_credential_hash;
use crate::DurableEventStore;
use crate::EventServiceAcceptedRun;
use crate::WORKFLOW_ROUTING_WAKE_PATH;
use actix_web::web;
use actix_web::App;
use actix_web::HttpServer;
use chrono::Duration as ChronoDuration;
use chrono::Utc;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use tokio::sync::mpsc as tokio_mpsc;

pub struct WomlWebhookServer {
  local_address: SocketAddr,
  handle: Option<actix_web::dev::ServerHandle>,
  internal_handle: actix_web::dev::ServerHandle,
  database_path: PathBuf,
  workflow_runtime_id: String,
  timed_trigger_tasks: Vec<actix_web::rt::task::JoinHandle<()>>,
  pending_activation: Option<PendingActivation>,
  runtime_state: web::Data<WebhookRuntimeState>,
}

struct PendingActivation {
  state: web::Data<WebhookRuntimeState>,
  recovery_runs: Vec<RunProgressIdentity>,
  startup_manual_runs: Vec<StartupManualTrigger>,
  external_ingress: Option<ExternalTriggerAdmissionReceiver>,
  internal_event_dispatch: tokio_mpsc::UnboundedReceiver<EventServiceAcceptedRun>,
  internal_address: SocketAddr,
}

impl WomlWebhookServer {
  pub async fn start(config: WomlWebhookServerConfig) -> Result<Self, WebhookRuntimeError> {
    Self::start_with_external_ingress(config, None).await
  }

  pub async fn start_with_external_ingress(
    config: WomlWebhookServerConfig,
    external_ingress: Option<ExternalTriggerAdmissionReceiver>,
  ) -> Result<Self, WebhookRuntimeError> {
    let mut server = Self::prepare_with_external_ingress(config, external_ingress).await?;
    server.activate().await?;
    Ok(server)
  }

  /// Prepares the complete runtime without admitting trigger occurrences.
  /// Definitions and module artifacts are pinned, listeners are bound, and
  /// routes are compiled, but every ingress surface remains closed until
  /// `activate` succeeds.
  pub async fn prepare_with_external_ingress(
    config: WomlWebhookServerConfig,
    external_ingress: Option<ExternalTriggerAdmissionReceiver>,
  ) -> Result<Self, WebhookRuntimeError> {
    let mut span =
      crate::performance::PerformanceSpan::new("runtime", "runtime.prepare_trigger_host");
    span.count("workflows", config.registrations.len());
    let result = Self::prepare_with_external_ingress_unprofiled(config, external_ingress).await;
    if result.is_ok() {
      span.succeed();
    }
    result
  }

  async fn prepare_with_external_ingress_unprofiled(
    config: WomlWebhookServerConfig,
    external_ingress: Option<ExternalTriggerAdmissionReceiver>,
  ) -> Result<Self, WebhookRuntimeError> {
    let (state, recovery_runs, startup_manual_runs, internal_event_dispatch) =
      prepare_state(config)?;
    let listener = if state.routes.is_empty()
      && state.event_token_digests.is_empty()
      && state.whatsapp_subscribers.is_empty()
      && state.signal_token_digest.is_none()
    {
      None
    } else {
      Some(TcpListener::bind(state.bind_address)?)
    };
    let local_address = listener
      .as_ref()
      .map(TcpListener::local_addr)
      .transpose()?
      .unwrap_or_else(|| "127.0.0.1:0".parse().expect("valid internal address"));
    let internal_listener = TcpListener::bind("127.0.0.1:0")?;
    let internal_address = internal_listener.local_addr()?;
    let workflow_runtime_id = state.workflow_targets.runtime_id().to_string();
    let database_path = state.database_path.clone();
    let app_state = web::Data::new(state);
    let public_app_state = app_state.clone();
    let handle = if let Some(listener) = listener {
      let server = HttpServer::new(move || {
        App::new()
          .app_data(public_app_state.clone())
          .default_service(web::to(handle_webhook))
      })
      .listen(listener)?
      .run();
      let handle = server.handle();
      actix_web::rt::spawn(server);
      Some(handle)
    } else {
      None
    };
    let internal_app_state = app_state.clone();
    let internal_server = HttpServer::new(move || {
      App::new()
        .app_data(internal_app_state.clone())
        .app_data(web::JsonConfig::default().limit(16 * 1024))
        .route(
          WORKFLOW_ROUTING_WAKE_PATH,
          web::post().to(handle_workflow_call_wakeup),
        )
    })
    .listen(internal_listener)?
    .run();
    let internal_handle = internal_server.handle();
    actix_web::rt::spawn(internal_server);

    Ok(Self {
      local_address,
      handle,
      internal_handle,
      database_path,
      workflow_runtime_id,
      timed_trigger_tasks: Vec::new(),
      runtime_state: app_state.clone(),
      pending_activation: Some(PendingActivation {
        state: app_state,
        recovery_runs,
        startup_manual_runs,
        external_ingress,
        internal_event_dispatch,
        internal_address,
      }),
    })
  }

  pub async fn activate(&mut self) -> Result<(), WebhookRuntimeError> {
    let mut span =
      crate::performance::PerformanceSpan::new("runtime", "runtime.activate_trigger_host");
    let result = self.activate_unprofiled().await;
    if result.is_ok() {
      span.succeed();
    }
    result
  }

  async fn activate_unprofiled(&mut self) -> Result<(), WebhookRuntimeError> {
    let Some(pending) = self.pending_activation.take() else {
      return Ok(());
    };
    let state = pending.state;
    let now = Utc::now();
    if let Err(error) = DurableEventStore::open_ready(&self.database_path)?
      .register_workflow_runtime_routes(
        &self.workflow_runtime_id,
        &state.workflow_targets.targets(),
        &format!("http://{}", pending.internal_address),
        &workflow_routing_credential_hash(&state.workflow_routing_credential),
        now,
        now + ChronoDuration::seconds(WORKFLOW_RUNTIME_LEASE_SECONDS),
      )
    {
      return Err(error.into());
    }

    let activation = (|| {
      let (mut tasks, recovered_schedule_runs) = initialize_schedules(state.clone())?;
      let (interval_tasks, recovered_interval_runs) = initialize_intervals(state.clone())?;
      tasks.extend(interval_tasks);
      Ok::<_, WebhookRuntimeError>((tasks, recovered_schedule_runs, recovered_interval_runs))
    })();
    let (mut tasks, recovered_schedule_runs, recovered_interval_runs) = match activation {
      Ok(value) => value,
      Err(error) => {
        let _ = DurableEventStore::open_ready(&self.database_path).and_then(|mut store| {
          store
            .unregister_workflow_runtime_routes(&self.workflow_runtime_id)
            .map(|_| ())
        });
        return Err(error);
      }
    };

    let routing_state = state.clone();
    tasks.push(actix_web::rt::spawn(async move {
      run_workflow_routing_maintenance(routing_state).await;
    }));
    let wait_state = state.clone();
    tasks.push(actix_web::rt::spawn(async move {
      run_durable_wait_scheduler(wait_state).await;
    }));
    if let Some(receiver) = pending.external_ingress {
      let external_state = state.clone();
      tasks.push(actix_web::rt::spawn(run_external_ingress(
        external_state,
        receiver,
      )));
    }
    let internal_state = state.clone();
    tasks.push(actix_web::rt::spawn(run_internal_event_dispatch(
      internal_state,
      pending.internal_event_dispatch,
    )));

    state.admission_open.store(true, Ordering::Release);
    state.report(TriggerProgress::Ready {
      contract: TRIGGER_PROGRESS_CONTRACT,
      contract_version: TRIGGER_PROGRESS_CONTRACT_VERSION,
      registration_count: state.registration_count,
      occurred_at: Utc::now(),
    });
    for identity in pending.recovery_runs {
      state.report_run_started(&identity);
      dispatch_run(state.get_ref(), identity);
    }
    for (identity, duplicate) in recovered_schedule_runs {
      state.report_accepted(&identity, duplicate);
      if !duplicate {
        state.report_run_started(&identity);
        dispatch_run(state.get_ref(), identity);
      }
    }
    for (identity, duplicate) in recovered_interval_runs {
      state.report_accepted(&identity, duplicate);
      if !duplicate {
        state.report_run_started(&identity);
        dispatch_run(state.get_ref(), identity);
      }
    }
    for startup in pending.startup_manual_runs {
      admit_startup_manual(state.get_ref(), startup);
    }
    self.timed_trigger_tasks = tasks;
    Ok(())
  }

  pub const fn local_address(&self) -> SocketAddr {
    self.local_address
  }

  pub async fn stop(self) {
    self
      .stop_with_deadline(std::time::Duration::from_secs(30))
      .await;
  }

  pub async fn stop_with_deadline(self, deadline: std::time::Duration) {
    self
      .runtime_state
      .admission_open
      .store(false, Ordering::Release);
    for task in self.timed_trigger_tasks {
      task.abort();
    }
    if let Some(handle) = self.handle {
      handle.stop(true).await;
    }
    self.internal_handle.stop(true).await;
    wait_for_active_runs(&self.runtime_state, deadline).await;
    self
      .runtime_state
      .execution
      .shutdown_shared_script_hosts()
      .await;
    let _ = DurableEventStore::open_ready(&self.database_path).and_then(|mut store| {
      store
        .unregister_workflow_runtime_routes(&self.workflow_runtime_id)
        .map(|_| ())
    });
  }
}
