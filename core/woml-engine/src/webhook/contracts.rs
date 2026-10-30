//! Private webhook contracts.

use crate::CompiledWorkflowDefinition;
use crate::DurableStoreError;
use crate::ModelValidationError;
use crate::RuntimeExecutionOptions;
use crate::RuntimeModuleArtifact;
use crate::TriggerAdmissionRequest;
use chrono::Utc;
use serde::Serialize;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::mpsc as tokio_mpsc;
use tokio::sync::oneshot;

pub const WEBHOOK_MAX_BODY_BYTES: usize = 1024 * 1024;

pub const WHATSAPP_CALLBACK_PATH: &str = "/callbacks/whatsapp";

pub const COMMUNICATION_PROVIDER_MAX_BATCH_ITEMS: usize = 100;

pub const COMMUNICATION_PROVIDER_MAX_MESSAGE_BYTES: usize = 40_000;

pub const EVENT_MAX_SUBSCRIBERS: usize = 1_000;

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
