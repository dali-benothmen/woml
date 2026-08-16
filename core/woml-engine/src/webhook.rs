use std::collections::{BTreeMap, HashMap};
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use actix_web::http::{header, Method, StatusCode};
use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer};
use chrono::{Duration as ChronoDuration, SecondsFormat, Utc};
use futures_util::StreamExt;
use jsonschema::error::ValidationErrorKind;
use jsonschema::Validator;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use thiserror::Error;
use tokio::sync::{mpsc as tokio_mpsc, oneshot, Notify};
use uuid::Uuid;

use crate::interval::{
  IntervalProgress, IntervalProgressReason, WomlInterval, INTERVAL_PROGRESS_CONTRACT,
  INTERVAL_PROGRESS_CONTRACT_VERSION,
};
use crate::model::ValueExpression;
use crate::schedule::{
  ScheduleMisfirePolicy, ScheduleProgress, ScheduleProgressReason, WomlSchedule,
  SCHEDULE_PROGRESS_CONTRACT, SCHEDULE_PROGRESS_CONTRACT_VERSION,
};
use crate::{
  dispatch_admitted_workflow_call, execute_admitted_trigger_run_durable,
  workflow_routing_credential_hash, workflow_routing_session_credential,
  CompiledWorkflowDefinition, DurableEventStore, DurableStoreError, EventServiceAcceptedRun,
  EventServiceSubscriber, IntervalCursorRegistration, ManagedEventsHandler,
  ManagedWorkflowCallsHandler, ModelValidationError, RuntimeExecutionOptions,
  RuntimeModuleArtifact, ScheduleCursorRegistration, TriggerAdmissionRequest,
  WorkflowRoutingAcknowledgement, WorkflowRoutingWakeup, WorkflowTargetRegistry,
  WORKFLOW_ROUTING_CONTRACT, WORKFLOW_ROUTING_CONTRACT_VERSION, WORKFLOW_ROUTING_WAKE_PATH,
};

pub const WEBHOOK_MAX_BODY_BYTES: usize = 1024 * 1024;
pub const EVENT_MAX_SUBSCRIBERS: usize = 1_000;
pub const TRIGGER_PROGRESS_CONTRACT: &str = "woml.trigger-progress";
pub const TRIGGER_PROGRESS_CONTRACT_VERSION: u32 = 1;
const WORKFLOW_RUNTIME_LEASE_SECONDS: i64 = 10;
const WORKFLOW_RUNTIME_RENEW_SECONDS: u64 = 2;
const WORKFLOW_PENDING_SCAN_MILLISECONDS: u64 = 250;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type")]
pub enum TriggerProgress {
  #[serde(rename = "ready", rename_all = "camelCase")]
  Ready {
    contract: &'static str,
    contract_version: u32,
    registration_count: usize,
    occurred_at: chrono::DateTime<Utc>,
  },
  #[serde(rename = "occurrence_accepted", rename_all = "camelCase")]
  OccurrenceAccepted {
    contract: &'static str,
    contract_version: u32,
    workflow_id: String,
    trigger_id: String,
    trigger_handler: String,
    occurrence_id: String,
    run_id: String,
    duplicate: bool,
    occurred_at: chrono::DateTime<Utc>,
  },
  #[serde(rename = "run_started", rename_all = "camelCase")]
  RunStarted {
    contract: &'static str,
    contract_version: u32,
    workflow_id: String,
    trigger_id: String,
    trigger_handler: String,
    occurrence_id: String,
    run_id: String,
    occurred_at: chrono::DateTime<Utc>,
  },
  #[serde(rename = "run_terminal", rename_all = "camelCase")]
  RunTerminal {
    contract: &'static str,
    contract_version: u32,
    workflow_id: String,
    run_id: String,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure_code: Option<String>,
    occurred_at: chrono::DateTime<Utc>,
  },
  #[serde(rename = "occurrence_rejected", rename_all = "camelCase")]
  OccurrenceRejected {
    contract: &'static str,
    contract_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    workflow_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    trigger_id: Option<String>,
    trigger_handler: String,
    code: String,
    message: String,
    occurred_at: chrono::DateTime<Utc>,
  },
}

pub type TriggerProgressReporter = Arc<dyn Fn(TriggerProgress) + Send + Sync>;

pub struct WebhookDefinitionRegistration {
  pub workflow: CompiledWorkflowDefinition,
  pub definition_hash: String,
  pub resolved_secrets: BTreeMap<String, String>,
  pub runtime_modules: Vec<RuntimeModuleArtifact>,
}

impl WebhookDefinitionRegistration {
  pub fn new(workflow: CompiledWorkflowDefinition, definition_hash: impl Into<String>) -> Self {
    Self {
      workflow,
      definition_hash: definition_hash.into(),
      resolved_secrets: BTreeMap::new(),
      runtime_modules: Vec::new(),
    }
  }

  pub fn with_secret(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
    self.resolved_secrets.insert(name.into(), value.into());
    self
  }

  pub fn with_runtime_modules(mut self, modules: Vec<RuntimeModuleArtifact>) -> Self {
    self.runtime_modules = modules;
    self
  }
}

pub struct WomlWebhookServerConfig {
  pub bind_address: SocketAddr,
  pub database_path: PathBuf,
  pub registrations: Vec<WebhookDefinitionRegistration>,
  pub startup_manual_triggers: BTreeMap<String, String>,
  pub execution: RuntimeExecutionOptions,
  pub progress_reporter: Option<TriggerProgressReporter>,
}

pub struct ExternalTriggerAdmissionCommand {
  pub request: TriggerAdmissionRequest,
  pub response: oneshot::Sender<Result<crate::TriggerAdmissionOutcome, DurableStoreError>>,
}

pub type ExternalTriggerAdmissionReceiver =
  tokio_mpsc::UnboundedReceiver<ExternalTriggerAdmissionCommand>;

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
    let (state, recovery_runs, startup_manual_runs, internal_event_dispatch) =
      prepare_state(config)?;
    let listener = if state.routes.is_empty() && state.event_token_digests.is_empty() {
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
    let _ = DurableEventStore::open_ready(&self.database_path).and_then(|mut store| {
      store
        .unregister_workflow_runtime_routes(&self.workflow_runtime_id)
        .map(|_| ())
    });
  }
}

#[derive(Debug, Error)]
pub enum WebhookRuntimeError {
  #[error("invalid WOML trigger registration: {0}")]
  InvalidRegistration(String),
  #[error("compiled WOML trigger definition is invalid: {0}")]
  Model(#[from] ModelValidationError),
  #[error("webhook route {0:?} is registered more than once")]
  RouteConflict(String),
  #[error("trigger secret {0:?} is missing or empty")]
  SecretMissing(String),
  #[error("trigger JSON Schema is invalid for trigger {trigger_id:?}: {message}")]
  InvalidSchema { trigger_id: String, message: String },
  #[error("durable trigger storage is unavailable: {0}")]
  DurableStore(#[from] DurableStoreError),
  #[error("the trigger listener could not bind: {0}")]
  Io(#[from] std::io::Error),
}

struct WebhookRuntimeState {
  bind_address: SocketAddr,
  database_path: PathBuf,
  routes: HashMap<String, Arc<WebhookRoute>>,
  event_subscribers: BTreeMap<String, Vec<EventServiceSubscriber>>,
  event_token_digests: BTreeMap<String, [u8; 32]>,
  registration_count: usize,
  execution: RuntimeExecutionOptions,
  progress_reporter: Option<TriggerProgressReporter>,
  schedules: Vec<ScheduleRuntimeRegistration>,
  intervals: Vec<IntervalRuntimeRegistration>,
  workflow_targets: Arc<WorkflowTargetRegistry>,
  workflow_routing_credential: String,
  admission_open: AtomicBool,
  active_runs: Arc<AtomicUsize>,
  active_runs_changed: Arc<Notify>,
}

async fn handle_workflow_call_wakeup(
  state: web::Data<WebhookRuntimeState>,
  request: HttpRequest,
  wakeup: web::Json<WorkflowRoutingWakeup>,
) -> HttpResponse {
  if !state.admission_open.load(Ordering::Acquire) {
    return workflow_routing_acknowledgement(
      StatusCode::SERVICE_UNAVAILABLE,
      &wakeup.child_run_id,
      false,
      Some("WOML_RUNTIME_NOT_READY"),
    );
  }
  let supplied = request
    .headers()
    .get(header::AUTHORIZATION)
    .and_then(|value| value.to_str().ok())
    .and_then(|value| value.strip_prefix("Bearer "))
    .unwrap_or_default();
  let expected = state.workflow_routing_credential.as_bytes();
  let authorized =
    supplied.len() == expected.len() && bool::from(supplied.as_bytes().ct_eq(expected));
  if !authorized {
    return workflow_routing_acknowledgement(
      StatusCode::UNAUTHORIZED,
      &wakeup.child_run_id,
      false,
      Some("WOML_WORKFLOW_TARGET_UNAVAILABLE"),
    );
  }
  if wakeup.contract != WORKFLOW_ROUTING_CONTRACT
    || wakeup.contract_version != WORKFLOW_ROUTING_CONTRACT_VERSION
    || wakeup.kind != "wakeup"
    || wakeup.runtime_id != state.workflow_targets.runtime_id()
  {
    return workflow_routing_acknowledgement(
      StatusCode::BAD_REQUEST,
      &wakeup.child_run_id,
      false,
      Some("WOML_WORKFLOW_TARGET_UNAVAILABLE"),
    );
  }
  let admission = DurableEventStore::open_ready(&state.database_path)
    .and_then(|store| store.workflow_call_for_child(&wakeup.child_run_id));
  let Ok(Some(admission)) = admission else {
    return workflow_routing_acknowledgement(
      StatusCode::NOT_FOUND,
      &wakeup.child_run_id,
      false,
      Some("WOML_WORKFLOW_TARGET_UNAVAILABLE"),
    );
  };
  if admission.call_key != wakeup.call_key
    || !state.workflow_targets.owns(
      &admission.target_workflow_id,
      &admission.target_definition_hash,
    )
  {
    return workflow_routing_acknowledgement(
      StatusCode::CONFLICT,
      &wakeup.child_run_id,
      false,
      Some("WOML_WORKFLOW_DEFINITION_MISMATCH"),
    );
  }
  if dispatch_admitted_workflow_call(
    state.database_path.clone(),
    admission,
    state.execution.clone(),
  )
  .await
  .is_err()
  {
    return workflow_routing_acknowledgement(
      StatusCode::SERVICE_UNAVAILABLE,
      &wakeup.child_run_id,
      false,
      Some("WOML_WORKFLOW_TARGET_UNAVAILABLE"),
    );
  }
  workflow_routing_acknowledgement(StatusCode::OK, &wakeup.child_run_id, true, None)
}

fn workflow_routing_acknowledgement(
  status: StatusCode,
  child_run_id: &str,
  accepted: bool,
  code: Option<&str>,
) -> HttpResponse {
  HttpResponse::build(status).json(WorkflowRoutingAcknowledgement {
    contract: WORKFLOW_ROUTING_CONTRACT.to_string(),
    contract_version: WORKFLOW_ROUTING_CONTRACT_VERSION,
    kind: "acknowledgement".to_string(),
    child_run_id: child_run_id.to_string(),
    accepted,
    code: code.map(str::to_string),
  })
}

async fn run_workflow_routing_maintenance(state: web::Data<WebhookRuntimeState>) {
  let mut next_renewal = tokio::time::Instant::now();
  loop {
    let now = Utc::now();
    let targets = state.workflow_targets.targets();
    if tokio::time::Instant::now() >= next_renewal {
      let renewed = DurableEventStore::open_ready(&state.database_path).and_then(|mut store| {
        store.renew_workflow_runtime_routes(
          state.workflow_targets.runtime_id(),
          now,
          now + ChronoDuration::seconds(WORKFLOW_RUNTIME_LEASE_SECONDS),
        )
      });
      if renewed.is_ok_and(|count| count != targets.len()) {
        return;
      }
      next_renewal = tokio::time::Instant::now()
        + std::time::Duration::from_secs(WORKFLOW_RUNTIME_RENEW_SECONDS);
    }
    for target in targets {
      let pending = DurableEventStore::open_ready(&state.database_path)
        .and_then(|store| store.admitted_workflow_calls_for_target(&target));
      if let Ok(pending) = pending {
        for admission in pending {
          let _ = dispatch_admitted_workflow_call(
            state.database_path.clone(),
            admission,
            state.execution.clone(),
          )
          .await;
        }
      }
    }
    tokio::time::sleep(std::time::Duration::from_millis(
      WORKFLOW_PENDING_SCAN_MILLISECONDS,
    ))
    .await;
  }
}

#[derive(Clone)]
struct ScheduleRuntimeRegistration {
  workflow_id: String,
  definition_hash: String,
  trigger_id: String,
  schedule: WomlSchedule,
  on_missed: ScheduleMisfirePolicy,
}

#[derive(Clone)]
struct IntervalRuntimeRegistration {
  workflow_id: String,
  definition_hash: String,
  trigger_id: String,
  interval: WomlInterval,
  on_missed: ScheduleMisfirePolicy,
}

struct WebhookRoute {
  workflow_id: String,
  definition_hash: String,
  trigger_id: String,
  authentication: WebhookAuthentication,
  schema: Option<Arc<Validator>>,
  runtime_policy: bool,
}

type EventSubscriber = EventServiceSubscriber;

#[derive(Clone)]
struct RunProgressIdentity {
  workflow_id: String,
  trigger_id: String,
  trigger_handler: String,
  occurrence_id: String,
  run_id: String,
}

struct StartupManualTrigger {
  workflow_id: String,
  definition_hash: String,
  trigger_id: String,
}

impl WebhookRuntimeState {
  fn report(&self, progress: TriggerProgress) {
    if let Some(reporter) = &self.progress_reporter {
      reporter(progress);
    }
  }

  fn report_accepted(&self, identity: &RunProgressIdentity, duplicate: bool) {
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

  fn report_run_started(&self, identity: &RunProgressIdentity) {
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

  fn report_rejected(
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

  fn report_event_rejected(
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

enum WebhookAuthentication {
  None,
  Bearer { token_digest: [u8; 32] },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebhookAcceptedResponse {
  run_id: String,
  status: &'static str,
  duplicate: bool,
}

#[derive(Debug, Serialize)]
struct WebhookErrorResponse {
  error: WebhookErrorBody,
}

#[derive(Debug, Serialize)]
struct WebhookErrorBody {
  code: &'static str,
  message: &'static str,
  #[serde(skip_serializing_if = "Option::is_none")]
  issues: Option<Vec<WebhookSchemaIssue>>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct WebhookSchemaIssue {
  path: String,
  message: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all_fields = "camelCase")]
enum EventDeliveryResponse {
  #[serde(rename = "accepted")]
  Accepted {
    workflow_id: String,
    trigger_id: String,
    run_id: String,
    duplicate: bool,
  },
  #[serde(rename = "rejected")]
  Rejected {
    workflow_id: String,
    trigger_id: String,
    code: &'static str,
    message: &'static str,
    retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    issues: Option<Vec<WebhookSchemaIssue>>,
  },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EventPublishedResponse {
  event_id: String,
  event_name: String,
  status: &'static str,
  deliveries: Vec<EventDeliveryResponse>,
}

fn prepare_state(
  config: WomlWebhookServerConfig,
) -> Result<
  (
    WebhookRuntimeState,
    Vec<RunProgressIdentity>,
    Vec<StartupManualTrigger>,
    tokio_mpsc::UnboundedReceiver<EventServiceAcceptedRun>,
  ),
  WebhookRuntimeError,
> {
  if config.registrations.is_empty() {
    return Err(WebhookRuntimeError::InvalidRegistration(
      "at least one compiled workflow is required".to_string(),
    ));
  }

  let mut routes = HashMap::new();
  let mut definitions = Vec::new();
  let mut startup_manual_runs = Vec::new();
  let mut schedules = Vec::new();
  let mut intervals = Vec::new();
  let mut event_subscribers = BTreeMap::<String, Vec<EventSubscriber>>::new();
  let mut event_token_digests = BTreeMap::<String, [u8; 32]>::new();
  let mut registration_count = 0;
  let workflow_targets = Arc::new(
    WorkflowTargetRegistry::new(format!("runtime_{}", Uuid::new_v4().simple()))
      .map_err(|error| WebhookRuntimeError::InvalidRegistration(error.to_string()))?,
  );
  let workflow_routing_credential =
    workflow_routing_session_credential(&config.database_path, workflow_targets.runtime_id())?;
  for registration in config.registrations {
    registration.workflow.validate_for_durable_execution()?;
    if !matches!(
      registration.workflow.schema_version,
      crate::COMPILED_MODEL_SCHEMA_VERSION_V7
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V8
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V9
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V10
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V11
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V12
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V13
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V14
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V15
    ) {
      return Err(WebhookRuntimeError::InvalidRegistration(format!(
        "workflow {:?} must use a supported compiled Model v7 through v15",
        registration.workflow.workflow_id
      )));
    }
    workflow_targets
      .register(&registration.workflow, &registration.definition_hash)
      .map_err(|error| WebhookRuntimeError::InvalidRegistration(error.to_string()))?;
    if registration.workflow.triggers.iter().any(|trigger| {
      !matches!(
        trigger.handler.as_str(),
        "trigger.manual"
          | "trigger.webhook"
          | "trigger.slack"
          | "trigger.telegram"
          | "trigger.discord"
          | "trigger.schedule"
          | "trigger.interval"
          | "trigger.event"
      )
    }) {
      return Err(WebhookRuntimeError::InvalidRegistration(format!(
        "workflow {:?} contains an unsupported production trigger",
        registration.workflow.workflow_id
      )));
    }
    registration_count += registration.workflow.triggers.len();

    if let Some(trigger_id) = config
      .startup_manual_triggers
      .get(&registration.workflow.workflow_id)
    {
      let trigger = registration.workflow.trigger(trigger_id).ok_or_else(|| {
        WebhookRuntimeError::InvalidRegistration(format!(
          "workflow {:?} has no manual trigger {:?}",
          registration.workflow.workflow_id, trigger_id
        ))
      })?;
      if trigger.handler != "trigger.manual" {
        return Err(WebhookRuntimeError::InvalidRegistration(format!(
          "trigger {:?} in workflow {:?} is not manual",
          trigger_id, registration.workflow.workflow_id
        )));
      }
      startup_manual_runs.push(StartupManualTrigger {
        workflow_id: registration.workflow.workflow_id.clone(),
        definition_hash: registration.definition_hash.clone(),
        trigger_id: trigger_id.clone(),
      });
    }

    for trigger in &registration.workflow.triggers {
      if trigger.handler == "trigger.schedule" {
        schedules.push(compile_schedule(&registration, trigger)?);
      }
      if trigger.handler == "trigger.interval" {
        intervals.push(compile_interval(&registration, trigger)?);
      }
      if trigger.handler == "trigger.webhook" {
        let route = compile_route(&registration, trigger)?;
        let path = webhook_path(&trigger.config).ok_or_else(|| {
          WebhookRuntimeError::InvalidRegistration(format!(
            "trigger {:?} has no valid static path",
            trigger.id
          ))
        })?;
        if routes.insert(path.to_string(), Arc::new(route)).is_some() {
          return Err(WebhookRuntimeError::RouteConflict(path.to_string()));
        }
      }
      if trigger.handler == "trigger.event" {
        let (event_name, subscriber, token_digest) =
          compile_event_subscriber(&registration, trigger)?;
        if let Some(token_digest) = token_digest {
          if event_token_digests
            .get(&event_name)
            .is_some_and(|existing| existing != &token_digest)
          {
            return Err(WebhookRuntimeError::InvalidRegistration(format!(
              "all subscribers to event {event_name:?} must resolve to the same secret"
            )));
          }
          event_token_digests.insert(event_name.clone(), token_digest);
        }
        let subscribers = event_subscribers.entry(event_name).or_default();
        if subscribers.len() >= EVENT_MAX_SUBSCRIBERS {
          return Err(WebhookRuntimeError::InvalidRegistration(format!(
            "an event name cannot have more than {EVENT_MAX_SUBSCRIBERS} subscribers"
          )));
        }
        subscribers.push(subscriber);
      }
    }
    definitions.push((
      registration.workflow,
      registration.definition_hash,
      registration.runtime_modules,
    ));
  }
  workflow_targets.seal();

  for workflow_id in config.startup_manual_triggers.keys() {
    if !definitions
      .iter()
      .any(|(workflow, _, _)| &workflow.workflow_id == workflow_id)
    {
      return Err(WebhookRuntimeError::InvalidRegistration(format!(
        "startup manual trigger names unknown workflow {workflow_id:?}"
      )));
    }
  }

  let mut store = DurableEventStore::open(&config.database_path)?;
  let has_live_workflow_runtime = store.has_live_workflow_runtime_routes(Utc::now())?;
  if !has_live_workflow_runtime {
    store.recover_interrupted_runs()?;
  }
  for (workflow, definition_hash, runtime_modules) in &definitions {
    store.validate_runtime_policy_activation(workflow, definition_hash)?;
    store.register_definition_module_artifacts(workflow, definition_hash, runtime_modules)?;
  }
  if has_live_workflow_runtime {
    store.recover_workflow_call_children_for_targets(&workflow_targets.targets())?;
  }
  let recovery_runs = store
    .recover_undispatched_trigger_runs()?
    .into_iter()
    .filter(|work| {
      workflow_targets.owns(
        &work.occurrence.workflow_id,
        &work.occurrence.definition_hash,
      )
    })
    .map(|work| RunProgressIdentity {
      workflow_id: work.occurrence.workflow_id,
      trigger_id: work.occurrence.trigger_id,
      trigger_handler: work.occurrence.trigger_handler,
      occurrence_id: work.occurrence.occurrence_id,
      run_id: work.occurrence.run_id,
    })
    .collect();

  let (internal_event_sender, internal_event_receiver) = tokio_mpsc::unbounded_channel();
  let dispatcher = Arc::new(move |accepted: EventServiceAcceptedRun| {
    let _ = internal_event_sender.send(accepted);
  });
  config
    .execution
    .capability_registry
    .register(Arc::new(ManagedEventsHandler::new(
      config.database_path.clone(),
      event_subscribers.clone(),
      dispatcher,
    )))
    .map_err(|error| WebhookRuntimeError::InvalidRegistration(error.to_string()))?;
  config
    .execution
    .capability_registry
    .register(Arc::new(
      ManagedWorkflowCallsHandler::new(config.database_path.clone(), Arc::clone(&workflow_targets))
        .with_execution(&config.execution),
    ))
    .map_err(|error| WebhookRuntimeError::InvalidRegistration(error.to_string()))?;
  config
    .execution
    .capability_registry
    .register(Arc::new(
      ManagedWorkflowCallsHandler::for_start(
        config.database_path.clone(),
        Arc::clone(&workflow_targets),
      )
      .with_execution(&config.execution),
    ))
    .map_err(|error| WebhookRuntimeError::InvalidRegistration(error.to_string()))?;

  Ok((
    WebhookRuntimeState {
      bind_address: config.bind_address,
      database_path: config.database_path,
      routes,
      event_subscribers,
      event_token_digests,
      registration_count,
      execution: config.execution,
      progress_reporter: config.progress_reporter,
      schedules,
      intervals,
      workflow_targets,
      workflow_routing_credential,
      admission_open: AtomicBool::new(false),
      active_runs: Arc::new(AtomicUsize::new(0)),
      active_runs_changed: Arc::new(Notify::new()),
    },
    recovery_runs,
    startup_manual_runs,
    internal_event_receiver,
  ))
}

fn compile_event_subscriber(
  registration: &WebhookDefinitionRegistration,
  trigger: &crate::model::CompiledTrigger,
) -> Result<(String, EventSubscriber, Option<[u8; 32]>), WebhookRuntimeError> {
  let fields = object_fields(&trigger.config).ok_or_else(|| {
    WebhookRuntimeError::InvalidRegistration(format!(
      "event trigger {:?} config must be an object",
      trigger.id
    ))
  })?;
  let event_name = literal_string(fields.get("name"))
    .filter(|value| valid_event_name(value))
    .ok_or_else(|| {
      WebhookRuntimeError::InvalidRegistration(format!(
        "event trigger {:?} has an invalid name",
        trigger.id
      ))
    })?
    .to_string();
  let token_digest = match fields.get("secret") {
    None => None,
    Some(ValueExpression::SecretReference { name }) => {
      let token = registration
        .resolved_secrets
        .get(name)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| WebhookRuntimeError::SecretMissing(name.clone()))?;
      Some(Sha256::digest(token.as_bytes()).into())
    }
    Some(_) => {
      return Err(WebhookRuntimeError::InvalidRegistration(format!(
        "event trigger {:?} publisher secret must be symbolic",
        trigger.id
      )))
    }
  };
  let schema = match fields.get("schema") {
    None => None,
    Some(ValueExpression::Literal { value }) => {
      jsonschema::draft202012::meta::validate(value).map_err(|error| {
        WebhookRuntimeError::InvalidSchema {
          trigger_id: trigger.id.clone(),
          message: error.to_string(),
        }
      })?;
      Some(Arc::new(
        jsonschema::draft202012::options()
          .should_validate_formats(true)
          .build(value)
          .map_err(|error| WebhookRuntimeError::InvalidSchema {
            trigger_id: trigger.id.clone(),
            message: error.to_string(),
          })?,
      ))
    }
    Some(_) => {
      return Err(WebhookRuntimeError::InvalidRegistration(format!(
        "event trigger {:?} schema must be a literal object",
        trigger.id
      )))
    }
  };
  Ok((
    event_name,
    EventSubscriber {
      workflow_id: registration.workflow.workflow_id.clone(),
      definition_hash: registration.definition_hash.clone(),
      trigger_id: trigger.id.clone(),
      schema,
    },
    token_digest,
  ))
}

fn valid_event_name(value: &str) -> bool {
  if value.len() > 256
    || !value
      .bytes()
      .next()
      .is_some_and(|byte| byte.is_ascii_lowercase())
  {
    return false;
  }
  let mut has_separator = false;
  let mut previous_separator = false;
  for byte in value.bytes() {
    let separator = matches!(byte, b'.' | b'_' | b'-');
    if separator {
      if previous_separator {
        return false;
      }
      has_separator = true;
    } else if !byte.is_ascii_lowercase() && !byte.is_ascii_digit() {
      return false;
    }
    previous_separator = separator;
  }
  has_separator && !previous_separator
}

fn compile_route(
  registration: &WebhookDefinitionRegistration,
  trigger: &crate::model::CompiledTrigger,
) -> Result<WebhookRoute, WebhookRuntimeError> {
  let fields = object_fields(&trigger.config).ok_or_else(|| {
    WebhookRuntimeError::InvalidRegistration(format!(
      "trigger {:?} config must be an object",
      trigger.id
    ))
  })?;
  let authentication = object_fields(fields.get("authentication").ok_or_else(|| {
    WebhookRuntimeError::InvalidRegistration(format!(
      "trigger {:?} is missing authentication",
      trigger.id
    ))
  })?)
  .ok_or_else(|| {
    WebhookRuntimeError::InvalidRegistration(format!(
      "trigger {:?} authentication must be an object",
      trigger.id
    ))
  })?;
  let authentication = match literal_string(authentication.get("kind")) {
    Some("none") => WebhookAuthentication::None,
    Some("bearer") => {
      let Some(ValueExpression::SecretReference { name }) = authentication.get("secret") else {
        return Err(WebhookRuntimeError::InvalidRegistration(format!(
          "trigger {:?} bearer authentication requires a symbolic secret",
          trigger.id
        )));
      };
      let token = registration
        .resolved_secrets
        .get(name)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| WebhookRuntimeError::SecretMissing(name.clone()))?;
      WebhookAuthentication::Bearer {
        token_digest: Sha256::digest(token.as_bytes()).into(),
      }
    }
    _ => {
      return Err(WebhookRuntimeError::InvalidRegistration(format!(
        "trigger {:?} has unsupported authentication",
        trigger.id
      )))
    }
  };

  let schema = match fields.get("schema") {
    None => None,
    Some(ValueExpression::Literal { value }) => {
      jsonschema::draft202012::meta::validate(value).map_err(|error| {
        WebhookRuntimeError::InvalidSchema {
          trigger_id: trigger.id.clone(),
          message: error.to_string(),
        }
      })?;
      let validator = jsonschema::draft202012::options()
        .should_validate_formats(true)
        .build(value)
        .map_err(|error| WebhookRuntimeError::InvalidSchema {
          trigger_id: trigger.id.clone(),
          message: error.to_string(),
        })?;
      Some(Arc::new(validator))
    }
    Some(_) => {
      return Err(WebhookRuntimeError::InvalidRegistration(format!(
        "trigger {:?} schema must be a literal object",
        trigger.id
      )))
    }
  };

  Ok(WebhookRoute {
    workflow_id: registration.workflow.workflow_id.clone(),
    definition_hash: registration.definition_hash.clone(),
    trigger_id: trigger.id.clone(),
    authentication,
    schema,
    runtime_policy: registration.workflow.schema_version
      >= crate::COMPILED_MODEL_SCHEMA_VERSION_V12,
  })
}

fn object_fields(expression: &ValueExpression) -> Option<&BTreeMap<String, ValueExpression>> {
  match expression {
    ValueExpression::Object { fields } => Some(fields),
    _ => None,
  }
}

fn literal_string(expression: Option<&ValueExpression>) -> Option<&str> {
  match expression {
    Some(ValueExpression::Literal { value }) => value.as_str(),
    _ => None,
  }
}

fn literal_u64(expression: Option<&ValueExpression>) -> Option<u64> {
  match expression {
    Some(ValueExpression::Literal { value }) => value.as_u64(),
    _ => None,
  }
}

fn webhook_path(config: &ValueExpression) -> Option<&str> {
  literal_string(object_fields(config)?.get("path"))
}

fn compile_schedule(
  registration: &WebhookDefinitionRegistration,
  trigger: &crate::model::CompiledTrigger,
) -> Result<ScheduleRuntimeRegistration, WebhookRuntimeError> {
  let fields = object_fields(&trigger.config).ok_or_else(|| {
    WebhookRuntimeError::InvalidRegistration(format!(
      "schedule trigger {:?} config must be an object",
      trigger.id
    ))
  })?;
  let cron = literal_string(fields.get("cron")).ok_or_else(|| {
    WebhookRuntimeError::InvalidRegistration(format!(
      "schedule trigger {:?} is missing cron",
      trigger.id
    ))
  })?;
  let timezone = literal_string(fields.get("timezone")).ok_or_else(|| {
    WebhookRuntimeError::InvalidRegistration(format!(
      "schedule trigger {:?} is missing timezone",
      trigger.id
    ))
  })?;
  let on_missed = literal_string(fields.get("onMissed")).ok_or_else(|| {
    WebhookRuntimeError::InvalidRegistration(format!(
      "schedule trigger {:?} is missing onMissed",
      trigger.id
    ))
  })?;
  let schedule = WomlSchedule::parse(cron, timezone).map_err(|error| {
    WebhookRuntimeError::InvalidRegistration(format!(
      "schedule trigger {:?} is invalid: {error}",
      trigger.id
    ))
  })?;
  let on_missed = ScheduleMisfirePolicy::parse(on_missed).map_err(|error| {
    WebhookRuntimeError::InvalidRegistration(format!(
      "schedule trigger {:?} is invalid: {error}",
      trigger.id
    ))
  })?;
  Ok(ScheduleRuntimeRegistration {
    workflow_id: registration.workflow.workflow_id.clone(),
    definition_hash: registration.definition_hash.clone(),
    trigger_id: trigger.id.clone(),
    schedule,
    on_missed,
  })
}

fn compile_interval(
  registration: &WebhookDefinitionRegistration,
  trigger: &crate::model::CompiledTrigger,
) -> Result<IntervalRuntimeRegistration, WebhookRuntimeError> {
  let fields = object_fields(&trigger.config).ok_or_else(|| {
    WebhookRuntimeError::InvalidRegistration(format!(
      "interval trigger {:?} config must be an object",
      trigger.id
    ))
  })?;
  let every_ms = literal_u64(fields.get("everyMs")).ok_or_else(|| {
    WebhookRuntimeError::InvalidRegistration(format!(
      "interval trigger {:?} is missing everyMs",
      trigger.id
    ))
  })?;
  let on_missed = literal_string(fields.get("onMissed")).ok_or_else(|| {
    WebhookRuntimeError::InvalidRegistration(format!(
      "interval trigger {:?} is missing onMissed",
      trigger.id
    ))
  })?;
  let interval = WomlInterval::new(every_ms).map_err(|error| {
    WebhookRuntimeError::InvalidRegistration(format!(
      "interval trigger {:?} is invalid: {error}",
      trigger.id
    ))
  })?;
  let on_missed = ScheduleMisfirePolicy::parse(on_missed).map_err(|error| {
    WebhookRuntimeError::InvalidRegistration(format!(
      "interval trigger {:?} is invalid: {error}",
      trigger.id
    ))
  })?;
  Ok(IntervalRuntimeRegistration {
    workflow_id: registration.workflow.workflow_id.clone(),
    definition_hash: registration.definition_hash.clone(),
    trigger_id: trigger.id.clone(),
    interval,
    on_missed,
  })
}

fn initialize_schedules(
  state: web::Data<WebhookRuntimeState>,
) -> Result<
  (
    Vec<actix_web::rt::task::JoinHandle<()>>,
    Vec<(RunProgressIdentity, bool)>,
  ),
  WebhookRuntimeError,
> {
  let now = state.execution.schedule_clock.now();
  let mut recovered_runs = Vec::new();
  let mut prepared = Vec::new();
  for registration in state.schedules.clone() {
    let initial_next = registration
      .schedule
      .next_at_or_after(now)
      .map_err(|error| {
        WebhookRuntimeError::InvalidRegistration(format!(
          "schedule trigger {:?} could not compute its first occurrence: {error}",
          registration.trigger_id
        ))
      })?;
    let mut store = DurableEventStore::open_ready(&state.database_path)?;
    let registered = store.register_schedule_cursor(
      &ScheduleCursorRegistration {
        workflow_id: registration.workflow_id.clone(),
        trigger_id: registration.trigger_id.clone(),
        definition_hash: registration.definition_hash.clone(),
        cron: registration.schedule.cron().to_string(),
        timezone: registration.schedule.timezone().to_string(),
        on_missed: registration.on_missed.as_str().to_string(),
      },
      initial_next,
      now,
    )?;
    let mut cursor = registered.cursor;
    let mut reason = if registered.initialized {
      ScheduleProgressReason::Initialized
    } else {
      ScheduleProgressReason::Restarted
    };

    if !registered.initialized && cursor.next_scheduled_at < now {
      let next = registration.schedule.next_after(now).map_err(|error| {
        WebhookRuntimeError::InvalidRegistration(format!(
          "schedule trigger {:?} could not recover its cursor: {error}",
          registration.trigger_id
        ))
      })?;
      match registration.on_missed {
        ScheduleMisfirePolicy::Skip => {
          cursor = store.advance_schedule_cursor(
            &registration.workflow_id,
            &registration.trigger_id,
            cursor.next_scheduled_at,
            next,
            now,
          )?;
          reason = ScheduleProgressReason::MisfireSkipped;
        }
        ScheduleMisfirePolicy::RunOnce => {
          let planned = registration
            .schedule
            .latest_at_or_before(now)
            .map_err(|error| {
              WebhookRuntimeError::InvalidRegistration(format!(
                "schedule trigger {:?} could not recover its latest occurrence: {error}",
                registration.trigger_id
              ))
            })?;
          let (outcome, advanced) = store.claim_schedule_occurrence(
            cursor.next_scheduled_at,
            next,
            schedule_admission(&registration, planned, now),
          )?;
          cursor = advanced;
          recovered_runs.push((
            schedule_identity(&registration, &outcome),
            outcome.duplicate,
          ));
          reason = ScheduleProgressReason::MisfireRunOnce;
        }
      }
    }
    report_schedule_next_due(state.get_ref(), &registration, &cursor, reason, now);
    prepared.push(registration);
  }

  let tasks = prepared
    .into_iter()
    .map(|registration| {
      let task_state = state.clone();
      actix_web::rt::spawn(async move {
        run_schedule_loop(task_state, registration).await;
      })
    })
    .collect();
  Ok((tasks, recovered_runs))
}

async fn run_schedule_loop(
  state: web::Data<WebhookRuntimeState>,
  registration: ScheduleRuntimeRegistration,
) {
  loop {
    let cursor = match DurableEventStore::open_ready(&state.database_path)
      .and_then(|store| store.schedule_cursor(&registration.workflow_id, &registration.trigger_id))
    {
      Ok(cursor) => cursor,
      Err(error) => {
        report_schedule_error(state.get_ref(), &registration, &error.to_string());
        return;
      }
    };
    state
      .execution
      .schedule_clock
      .sleep_until(cursor.next_scheduled_at)
      .await;
    let now = state.execution.schedule_clock.now();
    if now < cursor.next_scheduled_at {
      continue;
    }

    let following = match registration.schedule.next_after(cursor.next_scheduled_at) {
      Ok(value) => value,
      Err(error) => {
        report_schedule_error(state.get_ref(), &registration, &error.to_string());
        return;
      }
    };
    let multiple_elapsed = following <= now;
    let mut store = match DurableEventStore::open_ready(&state.database_path) {
      Ok(store) => store,
      Err(error) => {
        report_schedule_error(state.get_ref(), &registration, &error.to_string());
        return;
      }
    };

    if multiple_elapsed && registration.on_missed == ScheduleMisfirePolicy::Skip {
      let next = match registration.schedule.next_after(now) {
        Ok(value) => value,
        Err(error) => {
          report_schedule_error(state.get_ref(), &registration, &error.to_string());
          return;
        }
      };
      match store.advance_schedule_cursor(
        &registration.workflow_id,
        &registration.trigger_id,
        cursor.next_scheduled_at,
        next,
        now,
      ) {
        Ok(advanced) => report_schedule_next_due(
          state.get_ref(),
          &registration,
          &advanced,
          ScheduleProgressReason::MisfireSkipped,
          now,
        ),
        Err(DurableStoreError::ScheduleCursorConflict) => continue,
        Err(error) => {
          report_schedule_error(state.get_ref(), &registration, &error.to_string());
          return;
        }
      }
      continue;
    }

    let (planned, next, reason) = if multiple_elapsed {
      let planned = match registration.schedule.latest_at_or_before(now) {
        Ok(value) => value,
        Err(error) => {
          report_schedule_error(state.get_ref(), &registration, &error.to_string());
          return;
        }
      };
      let next = match registration.schedule.next_after(now) {
        Ok(value) => value,
        Err(error) => {
          report_schedule_error(state.get_ref(), &registration, &error.to_string());
          return;
        }
      };
      (planned, next, ScheduleProgressReason::MisfireRunOnce)
    } else {
      (
        cursor.next_scheduled_at,
        following,
        ScheduleProgressReason::Advanced,
      )
    };
    let (outcome, advanced) = match store.claim_schedule_occurrence(
      cursor.next_scheduled_at,
      next,
      schedule_admission(&registration, planned, now),
    ) {
      Ok(value) => value,
      Err(DurableStoreError::ScheduleCursorConflict) => continue,
      Err(DurableStoreError::RuntimePolicyQueueFull) => {
        report_schedule_policy_queue_full(state.get_ref(), &registration);
        return;
      }
      Err(error) => {
        report_schedule_error(state.get_ref(), &registration, &error.to_string());
        return;
      }
    };
    let identity = schedule_identity(&registration, &outcome);
    state.report_accepted(&identity, outcome.duplicate);
    if !outcome.duplicate {
      state.report_run_started(&identity);
      dispatch_run(state.get_ref(), identity);
    }
    report_schedule_next_due(state.get_ref(), &registration, &advanced, reason, now);
  }
}

fn schedule_admission(
  registration: &ScheduleRuntimeRegistration,
  planned: chrono::DateTime<Utc>,
  triggered_at: chrono::DateTime<Utc>,
) -> TriggerAdmissionRequest {
  let planned_text = planned.to_rfc3339_opts(SecondsFormat::Millis, true);
  let triggered_text = triggered_at.to_rfc3339_opts(SecondsFormat::Millis, true);
  TriggerAdmissionRequest {
    workflow_id: registration.workflow_id.clone(),
    definition_hash: registration.definition_hash.clone(),
    trigger_id: registration.trigger_id.clone(),
    trigger_handler: "trigger.schedule".to_string(),
    source_identity: format!(
      "{}:{}:{planned_text}",
      registration.workflow_id, registration.trigger_id
    ),
    payload: serde_json::Map::from_iter([
      ("scheduledAt".to_string(), Value::String(planned_text)),
      ("triggeredAt".to_string(), Value::String(triggered_text)),
    ]),
    received_at: triggered_at,
  }
}

fn schedule_identity(
  registration: &ScheduleRuntimeRegistration,
  outcome: &crate::TriggerAdmissionOutcome,
) -> RunProgressIdentity {
  RunProgressIdentity {
    workflow_id: registration.workflow_id.clone(),
    trigger_id: registration.trigger_id.clone(),
    trigger_handler: "trigger.schedule".to_string(),
    occurrence_id: outcome.occurrence_id.clone(),
    run_id: outcome.run_id.clone(),
  }
}

fn report_schedule_next_due(
  state: &WebhookRuntimeState,
  registration: &ScheduleRuntimeRegistration,
  cursor: &crate::ScheduleCursor,
  reason: ScheduleProgressReason,
  occurred_at: chrono::DateTime<Utc>,
) {
  state.execution.report_schedule(ScheduleProgress::NextDue {
    contract: SCHEDULE_PROGRESS_CONTRACT,
    contract_version: SCHEDULE_PROGRESS_CONTRACT_VERSION,
    workflow_id: registration.workflow_id.clone(),
    trigger_id: registration.trigger_id.clone(),
    timezone: registration.schedule.timezone().to_string(),
    next_scheduled_at: cursor.next_scheduled_at,
    reason,
    occurred_at,
  });
}

fn report_schedule_error(
  state: &WebhookRuntimeState,
  registration: &ScheduleRuntimeRegistration,
  message: &str,
) {
  state
    .execution
    .report_schedule(ScheduleProgress::SchedulerError {
      contract: SCHEDULE_PROGRESS_CONTRACT,
      contract_version: SCHEDULE_PROGRESS_CONTRACT_VERSION,
      workflow_id: registration.workflow_id.clone(),
      trigger_id: registration.trigger_id.clone(),
      code: "WOML_SCHEDULE_RUNTIME_FAILED".to_string(),
      message: message.to_string(),
      occurred_at: state.execution.schedule_clock.now(),
    });
}

fn report_schedule_policy_queue_full(
  state: &WebhookRuntimeState,
  registration: &ScheduleRuntimeRegistration,
) {
  state
    .execution
    .report_schedule(ScheduleProgress::SchedulerError {
      contract: SCHEDULE_PROGRESS_CONTRACT,
      contract_version: SCHEDULE_PROGRESS_CONTRACT_VERSION,
      workflow_id: registration.workflow_id.clone(),
      trigger_id: registration.trigger_id.clone(),
      code: "WOML_POLICY_QUEUE_FULL".to_string(),
      message: "The durable WOML policy queue is full; the schedule cursor was not advanced."
        .to_string(),
      occurred_at: state.execution.schedule_clock.now(),
    });
}

fn initialize_intervals(
  state: web::Data<WebhookRuntimeState>,
) -> Result<
  (
    Vec<actix_web::rt::task::JoinHandle<()>>,
    Vec<(RunProgressIdentity, bool)>,
  ),
  WebhookRuntimeError,
> {
  let now = state.execution.schedule_clock.now();
  let mut recovered_runs = Vec::new();
  let mut prepared = Vec::new();
  for registration in state.intervals.clone() {
    let mut store = DurableEventStore::open_ready(&state.database_path)?;
    let registered = store.register_interval_cursor(
      &IntervalCursorRegistration {
        workflow_id: registration.workflow_id.clone(),
        trigger_id: registration.trigger_id.clone(),
        definition_hash: registration.definition_hash.clone(),
        every_ms: registration.interval.every_ms(),
        on_missed: registration.on_missed.as_str().to_string(),
      },
      now,
      now,
    )?;
    let mut cursor = registered.cursor;
    let mut reason = if registered.initialized {
      IntervalProgressReason::Initialized
    } else {
      IntervalProgressReason::Restarted
    };

    if !registered.initialized && cursor.next_scheduled_at < now {
      let next_sequence = registration
        .interval
        .next_sequence_after(cursor.anchor_at, now)
        .map_err(|error| interval_registration_error(&registration, "recover its cursor", error))?;
      let next_scheduled_at = registration
        .interval
        .planned_at(cursor.anchor_at, next_sequence)
        .map_err(|error| interval_registration_error(&registration, "recover its cursor", error))?;
      match registration.on_missed {
        ScheduleMisfirePolicy::Skip => {
          cursor = store.advance_interval_cursor(
            &registration.workflow_id,
            &registration.trigger_id,
            cursor.next_sequence,
            cursor.next_scheduled_at,
            next_sequence,
            next_scheduled_at,
            now,
          )?;
          reason = IntervalProgressReason::MisfireSkipped;
        }
        ScheduleMisfirePolicy::RunOnce => {
          let planned_sequence = registration
            .interval
            .latest_sequence_at_or_before(cursor.anchor_at, now)
            .map_err(|error| {
              interval_registration_error(&registration, "recover its latest occurrence", error)
            })?;
          let planned_at = registration
            .interval
            .planned_at(cursor.anchor_at, planned_sequence)
            .map_err(|error| {
              interval_registration_error(&registration, "recover its latest occurrence", error)
            })?;
          let (outcome, advanced) = store.claim_interval_occurrence(
            cursor.next_sequence,
            cursor.next_scheduled_at,
            next_sequence,
            next_scheduled_at,
            interval_admission(
              &registration,
              cursor.anchor_at,
              planned_sequence,
              planned_at,
              now,
            ),
          )?;
          cursor = advanced;
          recovered_runs.push((
            interval_identity(&registration, &outcome),
            outcome.duplicate,
          ));
          reason = IntervalProgressReason::MisfireRunOnce;
        }
      }
    }
    report_interval_next_due(state.get_ref(), &registration, &cursor, reason, now);
    prepared.push(registration);
  }

  let tasks = prepared
    .into_iter()
    .map(|registration| {
      let task_state = state.clone();
      actix_web::rt::spawn(async move {
        run_interval_loop(task_state, registration).await;
      })
    })
    .collect();
  Ok((tasks, recovered_runs))
}

fn interval_registration_error(
  registration: &IntervalRuntimeRegistration,
  operation: &str,
  error: crate::IntervalError,
) -> WebhookRuntimeError {
  WebhookRuntimeError::InvalidRegistration(format!(
    "interval trigger {:?} could not {operation}: {error}",
    registration.trigger_id
  ))
}

async fn run_interval_loop(
  state: web::Data<WebhookRuntimeState>,
  registration: IntervalRuntimeRegistration,
) {
  loop {
    let cursor = match DurableEventStore::open_ready(&state.database_path)
      .and_then(|store| store.interval_cursor(&registration.workflow_id, &registration.trigger_id))
    {
      Ok(cursor) => cursor,
      Err(error) => {
        report_interval_error(state.get_ref(), &registration, &error.to_string());
        return;
      }
    };
    state
      .execution
      .schedule_clock
      .sleep_until(cursor.next_scheduled_at)
      .await;
    let now = state.execution.schedule_clock.now();
    if now < cursor.next_scheduled_at {
      continue;
    }

    let following_sequence = match cursor.next_sequence.checked_add(1) {
      Some(value) => value,
      None => {
        report_interval_error(
          state.get_ref(),
          &registration,
          "interval sequence exceeds the supported range",
        );
        return;
      }
    };
    let following_at = match registration
      .interval
      .planned_at(cursor.anchor_at, following_sequence)
    {
      Ok(value) => value,
      Err(error) => {
        report_interval_error(state.get_ref(), &registration, &error.to_string());
        return;
      }
    };
    let multiple_elapsed = following_at <= now;
    let mut store = match DurableEventStore::open_ready(&state.database_path) {
      Ok(store) => store,
      Err(error) => {
        report_interval_error(state.get_ref(), &registration, &error.to_string());
        return;
      }
    };

    if multiple_elapsed && registration.on_missed == ScheduleMisfirePolicy::Skip {
      let next_sequence = match registration
        .interval
        .next_sequence_after(cursor.anchor_at, now)
      {
        Ok(value) => value,
        Err(error) => {
          report_interval_error(state.get_ref(), &registration, &error.to_string());
          return;
        }
      };
      let next_scheduled_at = match registration
        .interval
        .planned_at(cursor.anchor_at, next_sequence)
      {
        Ok(value) => value,
        Err(error) => {
          report_interval_error(state.get_ref(), &registration, &error.to_string());
          return;
        }
      };
      match store.advance_interval_cursor(
        &registration.workflow_id,
        &registration.trigger_id,
        cursor.next_sequence,
        cursor.next_scheduled_at,
        next_sequence,
        next_scheduled_at,
        now,
      ) {
        Ok(advanced) => report_interval_next_due(
          state.get_ref(),
          &registration,
          &advanced,
          IntervalProgressReason::MisfireSkipped,
          now,
        ),
        Err(DurableStoreError::IntervalCursorConflict) => continue,
        Err(error) => {
          report_interval_error(state.get_ref(), &registration, &error.to_string());
          return;
        }
      }
      continue;
    }

    let (planned_sequence, planned_at, next_sequence, next_scheduled_at, reason) =
      if multiple_elapsed {
        let planned_sequence = match registration
          .interval
          .latest_sequence_at_or_before(cursor.anchor_at, now)
        {
          Ok(value) => value,
          Err(error) => {
            report_interval_error(state.get_ref(), &registration, &error.to_string());
            return;
          }
        };
        let planned_at = match registration
          .interval
          .planned_at(cursor.anchor_at, planned_sequence)
        {
          Ok(value) => value,
          Err(error) => {
            report_interval_error(state.get_ref(), &registration, &error.to_string());
            return;
          }
        };
        let next_sequence = match registration
          .interval
          .next_sequence_after(cursor.anchor_at, now)
        {
          Ok(value) => value,
          Err(error) => {
            report_interval_error(state.get_ref(), &registration, &error.to_string());
            return;
          }
        };
        let next_at = match registration
          .interval
          .planned_at(cursor.anchor_at, next_sequence)
        {
          Ok(value) => value,
          Err(error) => {
            report_interval_error(state.get_ref(), &registration, &error.to_string());
            return;
          }
        };
        (
          planned_sequence,
          planned_at,
          next_sequence,
          next_at,
          IntervalProgressReason::MisfireRunOnce,
        )
      } else {
        (
          cursor.next_sequence,
          cursor.next_scheduled_at,
          following_sequence,
          following_at,
          IntervalProgressReason::Advanced,
        )
      };

    let (outcome, advanced) = match store.claim_interval_occurrence(
      cursor.next_sequence,
      cursor.next_scheduled_at,
      next_sequence,
      next_scheduled_at,
      interval_admission(
        &registration,
        cursor.anchor_at,
        planned_sequence,
        planned_at,
        now,
      ),
    ) {
      Ok(value) => value,
      Err(DurableStoreError::IntervalCursorConflict) => continue,
      Err(DurableStoreError::RuntimePolicyQueueFull) => {
        report_interval_policy_queue_full(state.get_ref(), &registration);
        return;
      }
      Err(error) => {
        report_interval_error(state.get_ref(), &registration, &error.to_string());
        return;
      }
    };
    let identity = interval_identity(&registration, &outcome);
    state.report_accepted(&identity, outcome.duplicate);
    if !outcome.duplicate {
      state.report_run_started(&identity);
      dispatch_run(state.get_ref(), identity);
    }
    report_interval_next_due(state.get_ref(), &registration, &advanced, reason, now);
  }
}

fn interval_admission(
  registration: &IntervalRuntimeRegistration,
  anchor_at: chrono::DateTime<Utc>,
  sequence: u64,
  planned_at: chrono::DateTime<Utc>,
  triggered_at: chrono::DateTime<Utc>,
) -> TriggerAdmissionRequest {
  let anchor_text = anchor_at.to_rfc3339_opts(SecondsFormat::Millis, true);
  let planned_text = planned_at.to_rfc3339_opts(SecondsFormat::Millis, true);
  let triggered_text = triggered_at.to_rfc3339_opts(SecondsFormat::Millis, true);
  TriggerAdmissionRequest {
    workflow_id: registration.workflow_id.clone(),
    definition_hash: registration.definition_hash.clone(),
    trigger_id: registration.trigger_id.clone(),
    trigger_handler: "trigger.interval".to_string(),
    source_identity: format!(
      "{}:{}:{anchor_text}:{sequence}",
      registration.workflow_id, registration.trigger_id
    ),
    payload: serde_json::Map::from_iter([
      ("scheduledAt".to_string(), Value::String(planned_text)),
      ("triggeredAt".to_string(), Value::String(triggered_text)),
    ]),
    received_at: triggered_at,
  }
}

fn interval_identity(
  registration: &IntervalRuntimeRegistration,
  outcome: &crate::TriggerAdmissionOutcome,
) -> RunProgressIdentity {
  RunProgressIdentity {
    workflow_id: registration.workflow_id.clone(),
    trigger_id: registration.trigger_id.clone(),
    trigger_handler: "trigger.interval".to_string(),
    occurrence_id: outcome.occurrence_id.clone(),
    run_id: outcome.run_id.clone(),
  }
}

fn report_interval_next_due(
  state: &WebhookRuntimeState,
  registration: &IntervalRuntimeRegistration,
  cursor: &crate::IntervalCursor,
  reason: IntervalProgressReason,
  occurred_at: chrono::DateTime<Utc>,
) {
  state.execution.report_interval(IntervalProgress::NextDue {
    contract: INTERVAL_PROGRESS_CONTRACT,
    contract_version: INTERVAL_PROGRESS_CONTRACT_VERSION,
    workflow_id: registration.workflow_id.clone(),
    trigger_id: registration.trigger_id.clone(),
    every_ms: registration.interval.every_ms(),
    anchor_at: cursor.anchor_at,
    next_sequence: cursor.next_sequence,
    next_scheduled_at: cursor.next_scheduled_at,
    reason,
    occurred_at,
  });
}

fn report_interval_error(
  state: &WebhookRuntimeState,
  registration: &IntervalRuntimeRegistration,
  message: &str,
) {
  state
    .execution
    .report_interval(IntervalProgress::SchedulerError {
      contract: INTERVAL_PROGRESS_CONTRACT,
      contract_version: INTERVAL_PROGRESS_CONTRACT_VERSION,
      workflow_id: registration.workflow_id.clone(),
      trigger_id: registration.trigger_id.clone(),
      code: "WOML_INTERVAL_RUNTIME_FAILED".to_string(),
      message: message.to_string(),
      occurred_at: state.execution.schedule_clock.now(),
    });
}

fn report_interval_policy_queue_full(
  state: &WebhookRuntimeState,
  registration: &IntervalRuntimeRegistration,
) {
  state
    .execution
    .report_interval(IntervalProgress::SchedulerError {
      contract: INTERVAL_PROGRESS_CONTRACT,
      contract_version: INTERVAL_PROGRESS_CONTRACT_VERSION,
      workflow_id: registration.workflow_id.clone(),
      trigger_id: registration.trigger_id.clone(),
      code: "WOML_POLICY_QUEUE_FULL".to_string(),
      message: "The durable WOML policy queue is full; the interval cursor was not advanced."
        .to_string(),
      occurred_at: state.execution.schedule_clock.now(),
    });
}

async fn handle_webhook(
  request: HttpRequest,
  mut body: web::Payload,
  state: web::Data<WebhookRuntimeState>,
) -> HttpResponse {
  if !state.admission_open.load(Ordering::Acquire) {
    let mut response = HttpResponse::ServiceUnavailable();
    response.insert_header((header::CACHE_CONTROL, "no-store"));
    response.insert_header((header::RETRY_AFTER, "1"));
    return response.json(WebhookErrorResponse {
      error: WebhookErrorBody {
        code: "WOML_RUNTIME_NOT_READY",
        message: "The WOML deployment is still activating; retry this request.",
        issues: None,
      },
    });
  }
  let event_name = request
    .path()
    .strip_prefix("/_woml/events/")
    .map(str::to_string);
  if let Some(event_name) = event_name {
    return handle_event_publication(request, body, state, event_name).await;
  }
  let Some(route) = state.routes.get(request.path()).cloned() else {
    return rejected_response(
      state.get_ref(),
      None,
      StatusCode::NOT_FOUND,
      "WOML_TRIGGER_NOT_FOUND",
      "No WOML webhook is registered for this route.",
      None,
    );
  };
  if request.method() != Method::POST {
    return rejected_response(
      state.get_ref(),
      Some(route.as_ref()),
      StatusCode::METHOD_NOT_ALLOWED,
      "WOML_TRIGGER_METHOD_NOT_ALLOWED",
      "This WOML webhook accepts POST requests only.",
      None,
    );
  }
  if !authorized(&request, &route.authentication) {
    return rejected_response(
      state.get_ref(),
      Some(route.as_ref()),
      StatusCode::UNAUTHORIZED,
      "WOML_TRIGGER_UNAUTHORIZED",
      "Webhook authentication failed.",
      None,
    );
  }
  if !has_json_content_type(&request) {
    return rejected_response(
      state.get_ref(),
      Some(route.as_ref()),
      StatusCode::BAD_REQUEST,
      "WOML_TRIGGER_PAYLOAD_INVALID",
      "Webhook payload must use application/json.",
      None,
    );
  }
  if request
    .headers()
    .get(header::CONTENT_LENGTH)
    .and_then(|value| value.to_str().ok())
    .and_then(|value| value.parse::<u64>().ok())
    .is_some_and(|length| length > WEBHOOK_MAX_BODY_BYTES as u64)
  {
    return rejected_response(
      state.get_ref(),
      Some(route.as_ref()),
      StatusCode::PAYLOAD_TOO_LARGE,
      "WOML_TRIGGER_PAYLOAD_TOO_LARGE",
      "Webhook payload exceeds the 1 MiB limit.",
      None,
    );
  }

  let mut bytes = Vec::new();
  while let Some(chunk) = body.next().await {
    let Ok(chunk) = chunk else {
      return rejected_response(
        state.get_ref(),
        Some(route.as_ref()),
        StatusCode::BAD_REQUEST,
        "WOML_TRIGGER_PAYLOAD_INVALID",
        "Webhook payload must be a valid JSON object.",
        None,
      );
    };
    if bytes.len().saturating_add(chunk.len()) > WEBHOOK_MAX_BODY_BYTES {
      return rejected_response(
        state.get_ref(),
        Some(route.as_ref()),
        StatusCode::PAYLOAD_TOO_LARGE,
        "WOML_TRIGGER_PAYLOAD_TOO_LARGE",
        "Webhook payload exceeds the 1 MiB limit.",
        None,
      );
    }
    bytes.extend_from_slice(&chunk);
  }
  let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
    return rejected_response(
      state.get_ref(),
      Some(route.as_ref()),
      StatusCode::BAD_REQUEST,
      "WOML_TRIGGER_PAYLOAD_INVALID",
      "Webhook payload must be a valid JSON object.",
      None,
    );
  };
  let Value::Object(payload) = value else {
    return rejected_response(
      state.get_ref(),
      Some(route.as_ref()),
      StatusCode::BAD_REQUEST,
      "WOML_TRIGGER_PAYLOAD_INVALID",
      "Webhook payload must be a valid JSON object.",
      None,
    );
  };

  if let Some(validator) = &route.schema {
    let value = Value::Object(payload.clone());
    let mut issues = validator
      .iter_errors(&value)
      .take(100)
      .map(schema_issue)
      .collect::<Vec<_>>();
    if !issues.is_empty() {
      issues.sort_by(|left, right| {
        left
          .path
          .cmp(&right.path)
          .then(left.message.cmp(right.message))
      });
      return rejected_response(
        state.get_ref(),
        Some(route.as_ref()),
        StatusCode::BAD_REQUEST,
        "WOML_TRIGGER_SCHEMA_INVALID",
        "Webhook payload does not match the declared schema.",
        Some(issues),
      );
    }
  }

  let source_identity = match request.headers().get("Idempotency-Key") {
    Some(value) => match value.to_str() {
      Ok(value) if !value.is_empty() && value.len() <= 512 => value.to_string(),
      _ => {
        return rejected_response(
          state.get_ref(),
          Some(route.as_ref()),
          StatusCode::BAD_REQUEST,
          "WOML_TRIGGER_PAYLOAD_INVALID",
          "Webhook payload must be a valid JSON object.",
          None,
        )
      }
    },
    None => format!("webhook_{}", Uuid::new_v4().simple()),
  };
  let admission = TriggerAdmissionRequest {
    workflow_id: route.workflow_id.clone(),
    definition_hash: route.definition_hash.clone(),
    trigger_id: route.trigger_id.clone(),
    trigger_handler: "trigger.webhook".to_string(),
    source_identity,
    payload,
    received_at: Utc::now(),
  };
  let database_path = state.database_path.clone();
  let admitted = web::block(move || {
    let mut store = DurableEventStore::open_ready(database_path)?;
    store.admit_trigger_occurrence(admission)
  })
  .await;

  let outcome = match admitted {
    Ok(Ok(outcome)) => outcome,
    Ok(Err(DurableStoreError::TriggerIdempotencyConflict)) => {
      return rejected_response(
        state.get_ref(),
        Some(route.as_ref()),
        StatusCode::CONFLICT,
        "WOML_TRIGGER_IDEMPOTENCY_CONFLICT",
        "This idempotency key is already bound to a different payload.",
        None,
      )
    }
    Ok(Err(DurableStoreError::RuntimePolicyQueueFull)) => {
      state.report_rejected(
        Some(route.as_ref()),
        "WOML_POLICY_QUEUE_FULL",
        "The durable WOML policy queue is full; retry this request.",
      );
      let mut response = HttpResponse::ServiceUnavailable();
      response.insert_header((header::CACHE_CONTROL, "no-store"));
      response.insert_header((header::RETRY_AFTER, "1"));
      return response.json(WebhookErrorResponse {
        error: WebhookErrorBody {
          code: "WOML_POLICY_QUEUE_FULL",
          message: "The durable WOML policy queue is full; retry this request.",
          issues: None,
        },
      });
    }
    Ok(Err(_)) | Err(_) => {
      return rejected_response(
        state.get_ref(),
        Some(route.as_ref()),
        StatusCode::SERVICE_UNAVAILABLE,
        "WOML_TRIGGER_UNAVAILABLE",
        "The durable WOML trigger authority is unavailable.",
        None,
      )
    }
  };

  let identity = RunProgressIdentity {
    workflow_id: route.workflow_id.clone(),
    trigger_id: route.trigger_id.clone(),
    trigger_handler: "trigger.webhook".to_string(),
    occurrence_id: outcome.occurrence_id,
    run_id: outcome.run_id.clone(),
  };
  state.report_accepted(&identity, outcome.duplicate);
  if !outcome.duplicate {
    state.report_run_started(&identity);
    dispatch_run(state.get_ref(), identity);
  }
  HttpResponse::Accepted().json(WebhookAcceptedResponse {
    run_id: outcome.run_id,
    status: if route.runtime_policy {
      "queued"
    } else {
      "accepted"
    },
    duplicate: outcome.duplicate,
  })
}

async fn handle_event_publication(
  request: HttpRequest,
  mut body: web::Payload,
  state: web::Data<WebhookRuntimeState>,
  event_name: String,
) -> HttpResponse {
  if request.method() != Method::POST {
    return event_rejected_response(
      state.get_ref(),
      StatusCode::METHOD_NOT_ALLOWED,
      "WOML_EVENT_METHOD_NOT_ALLOWED",
      "The WOML event publisher accepts POST requests only.",
    );
  }
  if !valid_event_name(&event_name) {
    return event_rejected_response(
      state.get_ref(),
      StatusCode::BAD_REQUEST,
      "WOML_EVENT_NAME_INVALID",
      "Event name is invalid.",
    );
  }
  let Some(subscribers) = state.event_subscribers.get(&event_name).cloned() else {
    return event_rejected_response(
      state.get_ref(),
      StatusCode::NOT_FOUND,
      "WOML_EVENT_NOT_FOUND",
      "No loaded WOML workflow subscribes to this event name.",
    );
  };
  let Some(token_digest) = state.event_token_digests.get(&event_name) else {
    return event_rejected_response(
      state.get_ref(),
      StatusCode::SERVICE_UNAVAILABLE,
      "WOML_EVENT_UNAVAILABLE",
      "The WOML event publisher is unavailable.",
    );
  };
  if !authorized_bearer(&request, token_digest) {
    return event_rejected_response(
      state.get_ref(),
      StatusCode::UNAUTHORIZED,
      "WOML_EVENT_UNAUTHORIZED",
      "Event publisher authentication failed.",
    );
  }
  let event_id = match request
    .headers()
    .get("Event-ID")
    .and_then(|value| value.to_str().ok())
  {
    Some(value) if valid_event_id(value) => value.to_string(),
    _ => {
      return event_rejected_response(
        state.get_ref(),
        StatusCode::BAD_REQUEST,
        "WOML_EVENT_ID_INVALID",
        "Event-ID must contain 1 to 256 URL-safe identifier characters.",
      )
    }
  };
  if !has_json_content_type(&request) {
    return event_rejected_response(
      state.get_ref(),
      StatusCode::BAD_REQUEST,
      "WOML_EVENT_PAYLOAD_INVALID",
      "Event payload must use application/json.",
    );
  }
  if request
    .headers()
    .get(header::CONTENT_LENGTH)
    .and_then(|value| value.to_str().ok())
    .and_then(|value| value.parse::<u64>().ok())
    .is_some_and(|length| length > WEBHOOK_MAX_BODY_BYTES as u64)
  {
    return event_rejected_response(
      state.get_ref(),
      StatusCode::PAYLOAD_TOO_LARGE,
      "WOML_EVENT_PAYLOAD_TOO_LARGE",
      "Event payload exceeds the 1 MiB limit.",
    );
  }

  let mut bytes = Vec::new();
  while let Some(chunk) = body.next().await {
    let Ok(chunk) = chunk else {
      return event_rejected_response(
        state.get_ref(),
        StatusCode::BAD_REQUEST,
        "WOML_EVENT_PAYLOAD_INVALID",
        "Event payload must be a valid JSON object.",
      );
    };
    if bytes.len().saturating_add(chunk.len()) > WEBHOOK_MAX_BODY_BYTES {
      return event_rejected_response(
        state.get_ref(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "WOML_EVENT_PAYLOAD_TOO_LARGE",
        "Event payload exceeds the 1 MiB limit.",
      );
    }
    bytes.extend_from_slice(&chunk);
  }
  let payload = match serde_json::from_slice::<Value>(&bytes) {
    Ok(Value::Object(payload)) => payload,
    _ => {
      return event_rejected_response(
        state.get_ref(),
        StatusCode::BAD_REQUEST,
        "WOML_EVENT_PAYLOAD_INVALID",
        "Event payload must be a valid JSON object.",
      )
    }
  };

  let received_at = Utc::now();
  let mut deliveries = Vec::with_capacity(subscribers.len());
  for subscriber in subscribers {
    if let Some(validator) = &subscriber.schema {
      let value = Value::Object(payload.clone());
      let mut issues = validator
        .iter_errors(&value)
        .take(100)
        .map(schema_issue)
        .collect::<Vec<_>>();
      if !issues.is_empty() {
        issues.sort_by(|left, right| {
          left
            .path
            .cmp(&right.path)
            .then(left.message.cmp(right.message))
        });
        let code = "WOML_TRIGGER_SCHEMA_INVALID";
        let message = "Event payload does not match this subscriber schema.";
        state.report_event_rejected(Some(&subscriber), code, message);
        deliveries.push(EventDeliveryResponse::Rejected {
          workflow_id: subscriber.workflow_id.clone(),
          trigger_id: subscriber.trigger_id.clone(),
          code,
          message,
          retryable: false,
          issues: Some(issues),
        });
        continue;
      }
    }

    let admission = TriggerAdmissionRequest {
      workflow_id: subscriber.workflow_id.clone(),
      definition_hash: subscriber.definition_hash.clone(),
      trigger_id: subscriber.trigger_id.clone(),
      trigger_handler: "trigger.event".to_string(),
      source_identity: event_source_identity(
        &event_id,
        &subscriber.workflow_id,
        &subscriber.trigger_id,
      ),
      payload: payload.clone(),
      received_at,
    };
    let database_path = state.database_path.clone();
    let admitted = web::block(move || {
      let mut store = DurableEventStore::open_ready(database_path)?;
      store.admit_trigger_occurrence(admission)
    })
    .await;
    match admitted {
      Ok(Ok(outcome)) => {
        let identity = RunProgressIdentity {
          workflow_id: subscriber.workflow_id.clone(),
          trigger_id: subscriber.trigger_id.clone(),
          trigger_handler: "trigger.event".to_string(),
          occurrence_id: outcome.occurrence_id,
          run_id: outcome.run_id.clone(),
        };
        state.report_accepted(&identity, outcome.duplicate);
        if !outcome.duplicate {
          state.report_run_started(&identity);
          dispatch_run(state.get_ref(), identity);
        }
        deliveries.push(EventDeliveryResponse::Accepted {
          workflow_id: subscriber.workflow_id.clone(),
          trigger_id: subscriber.trigger_id.clone(),
          run_id: outcome.run_id,
          duplicate: outcome.duplicate,
        });
      }
      Ok(Err(error)) => {
        let (code, message, retryable) = event_admission_failure(&error);
        state.report_event_rejected(Some(&subscriber), code, message);
        deliveries.push(EventDeliveryResponse::Rejected {
          workflow_id: subscriber.workflow_id.clone(),
          trigger_id: subscriber.trigger_id.clone(),
          code,
          message,
          retryable,
          issues: None,
        });
      }
      Err(_) => {
        let code = "WOML_TRIGGER_UNAVAILABLE";
        let message = "The durable WOML trigger authority is unavailable.";
        state.report_event_rejected(Some(&subscriber), code, message);
        deliveries.push(EventDeliveryResponse::Rejected {
          workflow_id: subscriber.workflow_id.clone(),
          trigger_id: subscriber.trigger_id.clone(),
          code,
          message,
          retryable: true,
          issues: None,
        });
      }
    }
  }

  let accepted = deliveries
    .iter()
    .filter(|delivery| matches!(delivery, EventDeliveryResponse::Accepted { .. }))
    .count();
  let status = if accepted == deliveries.len() {
    "accepted"
  } else if accepted == 0 {
    "rejected"
  } else {
    "partial"
  };
  let mut response = HttpResponse::Ok();
  response.insert_header((header::CACHE_CONTROL, "no-store"));
  response.json(EventPublishedResponse {
    event_id,
    event_name,
    status,
    deliveries,
  })
}

fn valid_event_id(value: &str) -> bool {
  let bytes = value.as_bytes();
  !bytes.is_empty()
    && bytes.len() <= 256
    && bytes[0].is_ascii_alphanumeric()
    && bytes[1..]
      .iter()
      .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}

fn event_source_identity(event_id: &str, workflow_id: &str, trigger_id: &str) -> String {
  let mut hasher = Sha256::new();
  hasher.update(event_id.as_bytes());
  hasher.update([0]);
  hasher.update(workflow_id.as_bytes());
  hasher.update([0]);
  hasher.update(trigger_id.as_bytes());
  format!("event:v1:sha256:{}", hex::encode(hasher.finalize()))
}

fn event_admission_failure(error: &DurableStoreError) -> (&'static str, &'static str, bool) {
  match error {
    DurableStoreError::RuntimePolicyQueueFull => (
      "WOML_POLICY_QUEUE_FULL",
      "The durable WOML policy queue is full; retry this subscriber delivery.",
      true,
    ),
    DurableStoreError::TriggerIdempotencyConflict => (
      "WOML_TRIGGER_IDEMPOTENCY_CONFLICT",
      "This event ID is already bound to a different payload.",
      false,
    ),
    DurableStoreError::TriggerDefinitionMismatch
    | DurableStoreError::TriggerHandlerMismatch
    | DurableStoreError::DefinitionConflict(_) => (
      "WOML_TRIGGER_DEFINITION_MISMATCH",
      "The event subscriber no longer matches its registered workflow definition.",
      false,
    ),
    DurableStoreError::TriggerHistoryInvalid(_) | DurableStoreError::InvalidStoredEvent(_) => (
      "WOML_TRIGGER_HISTORY_INVALID",
      "The durable event subscriber history is contradictory.",
      false,
    ),
    _ => (
      "WOML_TRIGGER_UNAVAILABLE",
      "The durable WOML trigger authority is unavailable.",
      true,
    ),
  }
}

fn admit_startup_manual(state: &WebhookRuntimeState, startup: StartupManualTrigger) {
  let request = TriggerAdmissionRequest {
    workflow_id: startup.workflow_id.clone(),
    definition_hash: startup.definition_hash,
    trigger_id: startup.trigger_id.clone(),
    trigger_handler: "trigger.manual".to_string(),
    source_identity: format!("manual_{}", Uuid::new_v4().simple()),
    payload: serde_json::Map::new(),
    received_at: Utc::now(),
  };
  let admitted = DurableEventStore::open_ready(&state.database_path)
    .and_then(|mut store| store.admit_trigger_occurrence(request));
  match admitted {
    Ok(outcome) => {
      let identity = RunProgressIdentity {
        workflow_id: startup.workflow_id,
        trigger_id: startup.trigger_id,
        trigger_handler: "trigger.manual".to_string(),
        occurrence_id: outcome.occurrence_id,
        run_id: outcome.run_id,
      };
      state.report_accepted(&identity, false);
      state.report_run_started(&identity);
      dispatch_run(state, identity);
    }
    Err(error) => state.report(TriggerProgress::OccurrenceRejected {
      contract: TRIGGER_PROGRESS_CONTRACT,
      contract_version: TRIGGER_PROGRESS_CONTRACT_VERSION,
      workflow_id: Some(startup.workflow_id),
      trigger_id: Some(startup.trigger_id),
      trigger_handler: "trigger.manual".to_string(),
      code: if matches!(error, DurableStoreError::RuntimePolicyQueueFull) {
        "WOML_POLICY_QUEUE_FULL"
      } else {
        "WOML_TRIGGER_UNAVAILABLE"
      }
      .to_string(),
      message: if matches!(error, DurableStoreError::RuntimePolicyQueueFull) {
        "The durable WOML policy queue is full; retry the manual run."
      } else {
        "The durable WOML trigger authority is unavailable."
      }
      .to_string(),
      occurred_at: Utc::now(),
    }),
  }
}

async fn run_external_ingress(
  state: web::Data<WebhookRuntimeState>,
  mut receiver: ExternalTriggerAdmissionReceiver,
) {
  while let Some(command) = receiver.recv().await {
    let request = command.request;
    let workflow_id = request.workflow_id.clone();
    let trigger_id = request.trigger_id.clone();
    let trigger_handler = request.trigger_handler.clone();
    let database_path = state.database_path.clone();
    let admitted = web::block(move || {
      let mut store = DurableEventStore::open_ready(database_path)?;
      store.admit_trigger_occurrence(request)
    })
    .await;

    let outcome = match admitted {
      Ok(Ok(outcome)) => outcome,
      Ok(Err(error)) => {
        let manual = trigger_handler == "trigger.manual";
        let (code, message) = match &error {
          DurableStoreError::TriggerIdempotencyConflict => (
            "WOML_TRIGGER_IDEMPOTENCY_CONFLICT",
            if manual {
              "This manual request identity is already bound to another run."
            } else {
              "This provider event is already bound to a different payload."
            },
          ),
          DurableStoreError::RuntimePolicyQueueFull => (
            "WOML_POLICY_QUEUE_FULL",
            if manual {
              "The durable WOML policy queue is full; try the manual trigger again later."
            } else {
              "The durable WOML policy queue is full; the provider may retry this event."
            },
          ),
          DurableStoreError::DefinitionNotFound(_)
          | DurableStoreError::TriggerDefinitionMismatch => (
            "WOML_TRIGGER_DEFINITION_MISMATCH",
            "The provider event does not match the active workflow definition; restart the WOML runtime.",
          ),
          DurableStoreError::TriggerNotFound { .. } => (
            "WOML_TRIGGER_NOT_FOUND",
            "The provider event references a trigger that is not registered in the active workflow.",
          ),
          DurableStoreError::TriggerHandlerMismatch => (
            "WOML_TRIGGER_HANDLER_MISMATCH",
            "The provider event type does not match the compiled workflow trigger.",
          ),
          DurableStoreError::Contract(_) | DurableStoreError::InvalidModel(_) => (
            "WOML_TRIGGER_CONTRACT_INVALID",
            "The compiled workflow and durable trigger contracts are incompatible; update or rebuild WOML.",
          ),
          _ => (
            "WOML_TRIGGER_UNAVAILABLE",
            if manual {
              "The durable WOML trigger authority is temporarily unavailable; retry the manual request."
            } else {
              "The durable WOML trigger authority is temporarily unavailable; the provider may retry this event."
            },
          ),
        };
        state.report(TriggerProgress::OccurrenceRejected {
          contract: TRIGGER_PROGRESS_CONTRACT,
          contract_version: TRIGGER_PROGRESS_CONTRACT_VERSION,
          workflow_id: Some(workflow_id),
          trigger_id: Some(trigger_id),
          trigger_handler,
          code: code.to_string(),
          message: message.to_string(),
          occurred_at: Utc::now(),
        });
        let _ = command.response.send(Err(error));
        continue;
      }
      Err(_) => {
        state.report(TriggerProgress::OccurrenceRejected {
          contract: TRIGGER_PROGRESS_CONTRACT,
          contract_version: TRIGGER_PROGRESS_CONTRACT_VERSION,
          workflow_id: Some(workflow_id),
          trigger_id: Some(trigger_id),
          trigger_handler,
          code: "WOML_TRIGGER_UNAVAILABLE".to_string(),
          message: "The durable WOML trigger authority is unavailable.".to_string(),
          occurred_at: Utc::now(),
        });
        let _ = command.response.send(Err(DurableStoreError::Contract(
          "The external trigger admission task failed.".to_string(),
        )));
        continue;
      }
    };

    let identity = RunProgressIdentity {
      workflow_id,
      trigger_id,
      trigger_handler,
      occurrence_id: outcome.occurrence_id.clone(),
      run_id: outcome.run_id.clone(),
    };
    state.report_accepted(&identity, outcome.duplicate);
    if !outcome.duplicate {
      state.report_run_started(&identity);
      dispatch_run(state.get_ref(), identity);
    }
    let _ = command.response.send(Ok(outcome));
  }
}

async fn run_internal_event_dispatch(
  state: web::Data<WebhookRuntimeState>,
  mut receiver: tokio_mpsc::UnboundedReceiver<EventServiceAcceptedRun>,
) {
  while let Some(accepted) = receiver.recv().await {
    let identity = RunProgressIdentity {
      workflow_id: accepted.workflow_id,
      trigger_id: accepted.trigger_id,
      trigger_handler: "trigger.event".to_string(),
      occurrence_id: accepted.occurrence_id,
      run_id: accepted.run_id,
    };
    state.report_accepted(&identity, accepted.duplicate);
    if !accepted.duplicate {
      state.report_run_started(&identity);
      dispatch_run(state.get_ref(), identity);
    }
  }
}

fn dispatch_run(state: &WebhookRuntimeState, identity: RunProgressIdentity) {
  let database_path = state.database_path.clone();
  let execution = state.execution.clone();
  let reporter = state.progress_reporter.clone();
  let active_runs = Arc::clone(&state.active_runs);
  let active_runs_changed = Arc::clone(&state.active_runs_changed);
  active_runs.fetch_add(1, Ordering::AcqRel);
  actix_web::rt::spawn(async move {
    let result =
      execute_trigger_run_with_builtin_notifications(database_path, &identity.run_id, execution)
        .await;
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
      Ok(crate::WorkflowRuntimeOutcome::Waiting { .. }) => None,
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
      || definition
        .notifications
        .iter()
        .any(|delivery| !matches!(delivery.provider.as_str(), "slack" | "telegram" | "discord"))
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

async fn wait_for_active_runs(state: &WebhookRuntimeState, deadline: std::time::Duration) {
  let wait = async {
    while state.active_runs.load(Ordering::Acquire) > 0 {
      state.active_runs_changed.notified().await;
    }
  };
  let _ = tokio::time::timeout(deadline, wait).await;
}

fn runtime_failure_code(error: &crate::RuntimeExecutionError) -> String {
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

fn authorized(request: &HttpRequest, authentication: &WebhookAuthentication) -> bool {
  match authentication {
    WebhookAuthentication::None => true,
    WebhookAuthentication::Bearer { token_digest } => authorized_bearer(request, token_digest),
  }
}

fn authorized_bearer(request: &HttpRequest, token_digest: &[u8; 32]) -> bool {
  let Some(value) = request
    .headers()
    .get(header::AUTHORIZATION)
    .and_then(|value| value.to_str().ok())
  else {
    return false;
  };
  let Some((scheme, presented)) = value.split_once(' ') else {
    return false;
  };
  scheme.eq_ignore_ascii_case("bearer")
    && !presented.is_empty()
    && bearer_token_matches(token_digest, presented)
}

fn bearer_token_matches(expected_digest: &[u8; 32], presented: &str) -> bool {
  let presented_digest: [u8; 32] = Sha256::digest(presented.as_bytes()).into();
  bool::from(expected_digest.ct_eq(&presented_digest))
}

fn has_json_content_type(request: &HttpRequest) -> bool {
  request
    .headers()
    .get(header::CONTENT_TYPE)
    .and_then(|value| value.to_str().ok())
    .and_then(|value| value.split(';').next())
    .is_some_and(|media_type| media_type.trim().eq_ignore_ascii_case("application/json"))
}

fn schema_issue(error: jsonschema::ValidationError<'_>) -> WebhookSchemaIssue {
  let mut path = error.instance_path().to_string();
  let message = match error.kind() {
    ValidationErrorKind::Required { property } => {
      if let Some(property) = property.as_str() {
        path.push('/');
        path.push_str(&escape_json_pointer(property));
      }
      "Required property is missing."
    }
    ValidationErrorKind::AdditionalProperties { .. }
    | ValidationErrorKind::UnevaluatedProperties { .. } => "Unexpected property is not allowed.",
    ValidationErrorKind::Type { .. } => "Value has the wrong JSON type.",
    ValidationErrorKind::Format { .. } => "String does not match the required format.",
    ValidationErrorKind::Pattern { .. } => "String does not match the required pattern.",
    ValidationErrorKind::MinLength { .. } => "String is shorter than allowed.",
    ValidationErrorKind::MaxLength { .. } => "String is longer than allowed.",
    ValidationErrorKind::Minimum { .. } | ValidationErrorKind::ExclusiveMinimum { .. } => {
      "Number is smaller than allowed."
    }
    ValidationErrorKind::Maximum { .. } | ValidationErrorKind::ExclusiveMaximum { .. } => {
      "Number is larger than allowed."
    }
    _ => "Value does not satisfy the declared schema.",
  };
  WebhookSchemaIssue {
    path: truncate_utf8(path, 2048),
    message,
  }
}

fn escape_json_pointer(value: &str) -> String {
  value.replace('~', "~0").replace('/', "~1")
}

fn truncate_utf8(mut value: String, max_bytes: usize) -> String {
  if value.len() <= max_bytes {
    return value;
  }
  let mut boundary = max_bytes;
  while !value.is_char_boundary(boundary) {
    boundary -= 1;
  }
  value.truncate(boundary);
  value
}

fn rejected_response(
  state: &WebhookRuntimeState,
  route: Option<&WebhookRoute>,
  status: StatusCode,
  code: &'static str,
  message: &'static str,
  issues: Option<Vec<WebhookSchemaIssue>>,
) -> HttpResponse {
  state.report_rejected(route, code, message);
  error_response(status, code, message, issues)
}

fn event_rejected_response(
  state: &WebhookRuntimeState,
  status: StatusCode,
  code: &'static str,
  message: &'static str,
) -> HttpResponse {
  state.report_event_rejected(None, code, message);
  error_response(status, code, message, None)
}

fn error_response(
  status: StatusCode,
  code: &'static str,
  message: &'static str,
  issues: Option<Vec<WebhookSchemaIssue>>,
) -> HttpResponse {
  let mut response = HttpResponse::build(status);
  response.insert_header((header::CACHE_CONTROL, "no-store"));
  if status == StatusCode::METHOD_NOT_ALLOWED {
    response.insert_header((header::ALLOW, "POST"));
  }
  response.json(WebhookErrorResponse {
    error: WebhookErrorBody {
      code,
      message,
      issues,
    },
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn bearer_comparison_hashes_every_candidate_to_the_same_fixed_width() {
    let expected: [u8; 32] = Sha256::digest(b"correct-token").into();
    assert!(bearer_token_matches(&expected, "correct-token"));
    assert!(!bearer_token_matches(&expected, "x"));
    assert!(!bearer_token_matches(&expected, &"x".repeat(8_192)));
  }
}
