//! Private webhook preparation.

use crate::model::ValueExpression;
use crate::webhook::compile_interval;
use crate::webhook::compile_schedule;
use crate::webhook::RunProgressIdentity;
use crate::webhook::StartupManualTrigger;
use crate::webhook::WebhookAuthentication;
use crate::webhook::WebhookDefinitionRegistration;
use crate::webhook::WebhookRuntimeError;
use crate::webhook::WebhookRuntimeState;
use crate::webhook::WhatsAppSubscriber;
use crate::webhook::WomlWebhookServerConfig;
use crate::webhook::EVENT_MAX_SUBSCRIBERS;
use crate::workflow_routing_session_credential;
use crate::DurableEventStore;
use crate::EventServiceAcceptedRun;
use crate::EventServiceSubscriber;
use crate::ManagedEventsHandler;
use crate::ManagedWorkflowCallsHandler;
use crate::WorkflowTargetRegistry;
use chrono::Utc;
use jsonschema::Validator;
use sha2::Digest;
use sha2::Sha256;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::sync::mpsc as tokio_mpsc;
use tokio::sync::Notify;
use uuid::Uuid;

pub(in crate::webhook) struct WebhookRoute {
  pub(in crate::webhook) workflow_id: String,
  pub(in crate::webhook) definition_hash: String,
  pub(in crate::webhook) trigger_id: String,
  pub(in crate::webhook) authentication: WebhookAuthentication,
  pub(in crate::webhook) schema: Option<Arc<Validator>>,
  pub(in crate::webhook) runtime_policy: bool,
}

pub(in crate::webhook) type EventSubscriber = EventServiceSubscriber;

pub(in crate::webhook) fn prepare_state(
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
  let mut whatsapp_subscribers = BTreeMap::<String, Vec<WhatsAppSubscriber>>::new();
  let mut whatsapp_verify_token: Option<String> = None;
  let mut whatsapp_app_secret: Option<String> = None;
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
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V16
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V17
    ) {
      return Err(WebhookRuntimeError::InvalidRegistration(format!(
        "workflow {:?} must use a supported compiled Model v7 through v17",
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
          | "trigger.whatsapp"
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
      if trigger.handler == "trigger.whatsapp" {
        let (phone_number_id, subscriber, verify_token, app_secret) =
          compile_whatsapp_subscriber(&registration, trigger)?;
        if whatsapp_verify_token
          .as_ref()
          .is_some_and(|existing| existing != &verify_token)
          || whatsapp_app_secret
            .as_ref()
            .is_some_and(|existing| existing != &app_secret)
        {
          return Err(WebhookRuntimeError::InvalidRegistration(
            "all WhatsApp triggers sharing one callback listener must use the same verify token and app secret"
              .to_string(),
          ));
        }
        whatsapp_verify_token.get_or_insert(verify_token);
        whatsapp_app_secret.get_or_insert(app_secret);
        whatsapp_subscribers
          .entry(phone_number_id)
          .or_default()
          .push(subscriber);
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
      whatsapp_subscribers,
      whatsapp_verify_token,
      whatsapp_app_secret,
      signal_token_digest: config
        .execution
        .signal_ingress_token
        .as_deref()
        .filter(|token| !token.is_empty())
        .map(|token| Sha256::digest(token.as_bytes()).into()),
      signal_rate_window: Mutex::new((Utc::now(), 0)),
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

fn compile_whatsapp_subscriber(
  registration: &WebhookDefinitionRegistration,
  trigger: &crate::model::CompiledTrigger,
) -> Result<(String, WhatsAppSubscriber, String, String), WebhookRuntimeError> {
  let fields = object_fields(&trigger.config).ok_or_else(|| {
    WebhookRuntimeError::InvalidRegistration(format!(
      "WhatsApp trigger {:?} config must be an object",
      trigger.id
    ))
  })?;
  let phone_number_id = literal_string(fields.get("phoneNumberId"))
    .filter(|value| {
      (6..=32).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_digit())
    })
    .ok_or_else(|| {
      WebhookRuntimeError::InvalidRegistration(format!(
        "WhatsApp trigger {:?} has an invalid phone number ID",
        trigger.id
      ))
    })?
    .to_string();
  let resolve_secret = |field: &str| -> Result<String, WebhookRuntimeError> {
    let Some(ValueExpression::SecretReference { name }) = fields.get(field) else {
      return Err(WebhookRuntimeError::InvalidRegistration(format!(
        "WhatsApp trigger {:?} requires symbolic {field}",
        trigger.id
      )));
    };
    registration
      .resolved_secrets
      .get(name)
      .filter(|value| !value.is_empty())
      .cloned()
      .ok_or_else(|| WebhookRuntimeError::SecretMissing(name.clone()))
  };
  Ok((
    phone_number_id,
    WhatsAppSubscriber {
      workflow_id: registration.workflow.workflow_id.clone(),
      definition_hash: registration.definition_hash.clone(),
      trigger_id: trigger.id.clone(),
    },
    resolve_secret("verifyToken")?,
    resolve_secret("appSecret")?,
  ))
}

pub(in crate::webhook) fn valid_event_name(value: &str) -> bool {
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

pub(in crate::webhook) fn object_fields(
  expression: &ValueExpression,
) -> Option<&BTreeMap<String, ValueExpression>> {
  match expression {
    ValueExpression::Object { fields } => Some(fields),
    _ => None,
  }
}

pub(in crate::webhook) fn literal_string(expression: Option<&ValueExpression>) -> Option<&str> {
  match expression {
    Some(ValueExpression::Literal { value }) => value.as_str(),
    _ => None,
  }
}

pub(in crate::webhook) fn literal_u64(expression: Option<&ValueExpression>) -> Option<u64> {
  match expression {
    Some(ValueExpression::Literal { value }) => value.as_u64(),
    _ => None,
  }
}

fn webhook_path(config: &ValueExpression) -> Option<&str> {
  literal_string(object_fields(config)?.get("path"))
}
