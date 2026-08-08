use std::collections::{BTreeMap, HashMap};
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::Arc;

use actix_web::http::{header, Method, StatusCode};
use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer};
use chrono::{SecondsFormat, Utc};
use futures_util::StreamExt;
use jsonschema::error::ValidationErrorKind;
use jsonschema::Validator;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use thiserror::Error;
use tokio::sync::{mpsc as tokio_mpsc, oneshot};
use uuid::Uuid;

use crate::model::ValueExpression;
use crate::schedule::{
  ScheduleMisfirePolicy, ScheduleProgress, ScheduleProgressReason, WomlSchedule,
  SCHEDULE_PROGRESS_CONTRACT, SCHEDULE_PROGRESS_CONTRACT_VERSION,
};
use crate::{
  execute_admitted_trigger_run_durable, CompiledWorkflowDefinition, DurableEventStore,
  DurableStoreError, ModelValidationError, RuntimeExecutionOptions, ScheduleCursorRegistration,
  TriggerAdmissionRequest,
};

pub const WEBHOOK_MAX_BODY_BYTES: usize = 1024 * 1024;
pub const TRIGGER_PROGRESS_CONTRACT: &str = "woml.trigger-progress";
pub const TRIGGER_PROGRESS_CONTRACT_VERSION: u32 = 1;

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
}

impl WebhookDefinitionRegistration {
  pub fn new(workflow: CompiledWorkflowDefinition, definition_hash: impl Into<String>) -> Self {
    Self {
      workflow,
      definition_hash: definition_hash.into(),
      resolved_secrets: BTreeMap::new(),
    }
  }

  pub fn with_secret(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
    self.resolved_secrets.insert(name.into(), value.into());
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
  schedule_tasks: Vec<actix_web::rt::task::JoinHandle<()>>,
}

impl WomlWebhookServer {
  pub async fn start(config: WomlWebhookServerConfig) -> Result<Self, WebhookRuntimeError> {
    Self::start_with_external_ingress(config, None).await
  }

  pub async fn start_with_external_ingress(
    config: WomlWebhookServerConfig,
    external_ingress: Option<ExternalTriggerAdmissionReceiver>,
  ) -> Result<Self, WebhookRuntimeError> {
    let (state, recovery_runs, startup_manual_runs) = prepare_state(config)?;
    let listener = if state.routes.is_empty() {
      None
    } else {
      Some(TcpListener::bind(state.bind_address)?)
    };
    let local_address = listener
      .as_ref()
      .map(TcpListener::local_addr)
      .transpose()?
      .unwrap_or_else(|| "127.0.0.1:0".parse().expect("valid internal address"));
    let app_state = web::Data::new(state);
    let recovery_state = app_state.clone();
    let (schedule_tasks, recovered_schedule_runs) = initialize_schedules(recovery_state.clone())?;
    let handle = if let Some(listener) = listener {
      let server = HttpServer::new(move || {
        App::new()
          .app_data(app_state.clone())
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

    if let Some(receiver) = external_ingress {
      let external_state = recovery_state.clone();
      actix_web::rt::spawn(run_external_ingress(external_state, receiver));
    }

    recovery_state.report(TriggerProgress::Ready {
      contract: TRIGGER_PROGRESS_CONTRACT,
      contract_version: TRIGGER_PROGRESS_CONTRACT_VERSION,
      registration_count: recovery_state.registration_count,
      occurred_at: Utc::now(),
    });

    for identity in recovery_runs {
      recovery_state.report_run_started(&identity);
      dispatch_run(recovery_state.get_ref(), identity);
    }
    for (identity, duplicate) in recovered_schedule_runs {
      recovery_state.report_accepted(&identity, duplicate);
      if !duplicate {
        recovery_state.report_run_started(&identity);
        dispatch_run(recovery_state.get_ref(), identity);
      }
    }
    for startup in startup_manual_runs {
      admit_startup_manual(recovery_state.get_ref(), startup);
    }

    Ok(Self {
      local_address,
      handle,
      schedule_tasks,
    })
  }

  pub const fn local_address(&self) -> SocketAddr {
    self.local_address
  }

  pub async fn stop(self) {
    for task in self.schedule_tasks {
      task.abort();
    }
    if let Some(handle) = self.handle {
      handle.stop(true).await;
    }
  }
}

#[derive(Debug, Error)]
pub enum WebhookRuntimeError {
  #[error("invalid WOML webhook registration: {0}")]
  InvalidRegistration(String),
  #[error("compiled WOML webhook definition is invalid: {0}")]
  Model(#[from] ModelValidationError),
  #[error("webhook route {0:?} is registered more than once")]
  RouteConflict(String),
  #[error("webhook secret {0:?} is missing or empty")]
  SecretMissing(String),
  #[error("webhook JSON Schema is invalid for trigger {trigger_id:?}: {message}")]
  InvalidSchema { trigger_id: String, message: String },
  #[error("durable webhook storage is unavailable: {0}")]
  DurableStore(#[from] DurableStoreError),
  #[error("the webhook listener could not bind: {0}")]
  Io(#[from] std::io::Error),
}

struct WebhookRuntimeState {
  bind_address: SocketAddr,
  database_path: PathBuf,
  routes: HashMap<String, Arc<WebhookRoute>>,
  registration_count: usize,
  execution: RuntimeExecutionOptions,
  progress_reporter: Option<TriggerProgressReporter>,
  schedules: Vec<ScheduleRuntimeRegistration>,
}

#[derive(Clone)]
struct ScheduleRuntimeRegistration {
  workflow_id: String,
  definition_hash: String,
  trigger_id: String,
  schedule: WomlSchedule,
  on_missed: ScheduleMisfirePolicy,
}

struct WebhookRoute {
  workflow_id: String,
  definition_hash: String,
  trigger_id: String,
  authentication: WebhookAuthentication,
  schema: Option<Arc<Validator>>,
}

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

fn prepare_state(
  config: WomlWebhookServerConfig,
) -> Result<
  (
    WebhookRuntimeState,
    Vec<RunProgressIdentity>,
    Vec<StartupManualTrigger>,
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
  let mut registration_count = 0;
  for registration in config.registrations {
    registration.workflow.validate_for_durable_execution()?;
    if registration.workflow.schema_version != crate::COMPILED_MODEL_SCHEMA_VERSION_V7 {
      return Err(WebhookRuntimeError::InvalidRegistration(format!(
        "workflow {:?} must use compiled Model v7",
        registration.workflow.workflow_id
      )));
    }
    if registration.workflow.triggers.iter().any(|trigger| {
      !matches!(
        trigger.handler.as_str(),
        "trigger.manual" | "trigger.webhook" | "trigger.slack" | "trigger.schedule"
      )
    }) {
      return Err(WebhookRuntimeError::InvalidRegistration(format!(
        "workflow {:?} contains a production trigger that is not active in T9",
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
    }
    definitions.push((registration.workflow, registration.definition_hash));
  }

  for workflow_id in config.startup_manual_triggers.keys() {
    if !definitions
      .iter()
      .any(|(workflow, _)| &workflow.workflow_id == workflow_id)
    {
      return Err(WebhookRuntimeError::InvalidRegistration(format!(
        "startup manual trigger names unknown workflow {workflow_id:?}"
      )));
    }
  }

  let mut store = DurableEventStore::open(&config.database_path)?;
  store.recover_interrupted_runs()?;
  for (workflow, definition_hash) in &definitions {
    store.register_definition(workflow, definition_hash)?;
  }
  let recovery_runs = store
    .recover_undispatched_trigger_runs()?
    .into_iter()
    .map(|work| RunProgressIdentity {
      workflow_id: work.occurrence.workflow_id,
      trigger_id: work.occurrence.trigger_id,
      trigger_handler: work.occurrence.trigger_handler,
      occurrence_id: work.occurrence.occurrence_id,
      run_id: work.occurrence.run_id,
    })
    .collect();

  Ok((
    WebhookRuntimeState {
      bind_address: config.bind_address,
      database_path: config.database_path,
      routes,
      registration_count,
      execution: config.execution,
      progress_reporter: config.progress_reporter,
      schedules,
    },
    recovery_runs,
    startup_manual_runs,
  ))
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
    let mut store = DurableEventStore::open(&state.database_path)?;
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
    let cursor = match DurableEventStore::open(&state.database_path)
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
    let mut store = match DurableEventStore::open(&state.database_path) {
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

async fn handle_webhook(
  request: HttpRequest,
  mut body: web::Payload,
  state: web::Data<WebhookRuntimeState>,
) -> HttpResponse {
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
    let mut store = DurableEventStore::open(database_path)?;
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
    status: "accepted",
    duplicate: outcome.duplicate,
  })
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
  let admitted = DurableEventStore::open(&state.database_path)
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
    Err(_) => state.report(TriggerProgress::OccurrenceRejected {
      contract: TRIGGER_PROGRESS_CONTRACT,
      contract_version: TRIGGER_PROGRESS_CONTRACT_VERSION,
      workflow_id: Some(startup.workflow_id),
      trigger_id: Some(startup.trigger_id),
      trigger_handler: "trigger.manual".to_string(),
      code: "WOML_TRIGGER_UNAVAILABLE".to_string(),
      message: "The durable WOML trigger authority is unavailable.".to_string(),
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
      let mut store = DurableEventStore::open(database_path)?;
      store.admit_trigger_occurrence(request)
    })
    .await;

    let outcome = match admitted {
      Ok(Ok(outcome)) => outcome,
      Ok(Err(error)) => {
        let (code, message) = if matches!(error, DurableStoreError::TriggerIdempotencyConflict) {
          (
            "WOML_TRIGGER_IDEMPOTENCY_CONFLICT",
            "This provider event is already bound to a different payload.",
          )
        } else {
          (
            "WOML_TRIGGER_UNAVAILABLE",
            "The durable WOML trigger authority rejected the provider event.",
          )
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

fn dispatch_run(state: &WebhookRuntimeState, identity: RunProgressIdentity) {
  let database_path = state.database_path.clone();
  let execution = state.execution.clone();
  let reporter = state.progress_reporter.clone();
  actix_web::rt::spawn(async move {
    let result =
      execute_admitted_trigger_run_durable(database_path, &identity.run_id, execution).await;
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
  });
}

fn runtime_failure_code(error: &crate::RuntimeExecutionError) -> String {
  match error {
    crate::RuntimeExecutionError::RunFailed(details) => details.code.clone(),
    crate::RuntimeExecutionError::BranchFailed(details) => details.code.clone(),
    crate::RuntimeExecutionError::ParallelFailed(details) => details.code.clone(),
    crate::RuntimeExecutionError::ApprovalFailed(details) => details.code.clone(),
    crate::RuntimeExecutionError::NotificationFailed(details) => details.code.clone(),
    _ => "WOML_TRIGGER_EXECUTION_FAILED".to_string(),
  }
}

fn authorized(request: &HttpRequest, authentication: &WebhookAuthentication) -> bool {
  match authentication {
    WebhookAuthentication::None => true,
    WebhookAuthentication::Bearer { token_digest } => {
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
      if !scheme.eq_ignore_ascii_case("bearer") || presented.is_empty() {
        return false;
      }
      bearer_token_matches(token_digest, presented)
    }
  }
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
