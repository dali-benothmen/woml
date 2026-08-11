//! Durable workflow-to-workflow target selection and child-run admission.
//!
//! WC2 stops after admission. WC3 adds child dispatch, terminal observation,
//! and the author-facing JavaScript result without changing these identities.

use std::{
  collections::BTreeMap,
  fs::{self, OpenOptions},
  io::{Read, Write},
  path::Path,
  path::PathBuf,
  sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, Weak,
  },
  time::Duration,
};

use chrono::{DateTime, Utc};
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
  capability::CapabilityIdentityMode, event::is_definition_hash,
  execute_admitted_trigger_run_durable, AttemptFailureKind, CapabilityCallRequest,
  CapabilityCancellationToken, CapabilityDescriptor, CapabilityEffect, CapabilityFailure,
  CapabilityFailureKind, CapabilityHandler, CapabilityRegistry, CompiledWorkflowDefinition,
  DurableEventStore, DurableStoreError, EngineClock, ExecutionProgressReporter,
  IntervalProgressReporter, RunFailure, RunStatus, RuntimeExecutionOptions, ScheduleClock,
  ScheduleProgressReporter, ScriptHostProcessOptions, WorkflowRuntimeOutcome,
};

pub const WORKFLOW_CALL_CONTRACT: &str = "woml.workflow-call";
pub const WORKFLOW_CALL_CONTRACT_VERSION: u32 = 1;
pub const MAX_WORKFLOW_CALL_DEPTH: u32 = 32;
const MAX_WORKFLOW_CALL_PAYLOAD_BYTES: usize = 1_048_576;
const MAX_WORKFLOW_CALL_RESULT_BYTES: usize = 4_194_304;
pub const WORKFLOW_ROUTING_CONTRACT: &str = "woml.workflow-call-routing";
pub const WORKFLOW_ROUTING_CONTRACT_VERSION: u32 = 1;
pub const WORKFLOW_ROUTING_WAKE_PATH: &str = "/_woml/internal/workflow-calls/wake";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowTarget {
  pub runtime_id: String,
  pub workflow_id: String,
  pub definition_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRuntimeRoute {
  pub runtime_id: String,
  pub workflow_id: String,
  pub definition_hash: String,
  pub endpoint: String,
  pub session_credential_hash: String,
  pub lease_expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowRoutingWakeup {
  pub contract: String,
  pub contract_version: u32,
  pub kind: String,
  pub runtime_id: String,
  pub child_run_id: String,
  pub call_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowRoutingAcknowledgement {
  pub contract: String,
  pub contract_version: u32,
  pub kind: String,
  pub child_run_id: String,
  pub accepted: bool,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub code: Option<String>,
}

impl WorkflowRuntimeRoute {
  pub fn target(&self) -> WorkflowTarget {
    WorkflowTarget {
      runtime_id: self.runtime_id.clone(),
      workflow_id: self.workflow_id.clone(),
      definition_hash: self.definition_hash.clone(),
    }
  }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorkflowTargetRegistryError {
  #[error("workflow target registration is invalid")]
  InvalidTarget,
  #[error("workflow target registry is already sealed")]
  RegistrySealed,
  #[error("workflow target registry must be sealed before resolution")]
  RegistryNotSealed,
  #[error("workflow ID {0:?} has more than one live owner")]
  DuplicateWorkflowId(String),
  #[error("no active workflow owns ID {0:?}")]
  TargetNotFound(String),
}

#[derive(Debug)]
pub struct WorkflowTargetRegistry {
  runtime_id: String,
  targets: Mutex<BTreeMap<String, WorkflowTarget>>,
  sealed: AtomicBool,
}

impl WorkflowTargetRegistry {
  pub fn new(runtime_id: impl Into<String>) -> Result<Self, WorkflowTargetRegistryError> {
    let runtime_id = runtime_id.into();
    if runtime_id.is_empty() || runtime_id.len() > 256 {
      return Err(WorkflowTargetRegistryError::InvalidTarget);
    }
    Ok(Self {
      runtime_id,
      targets: Mutex::new(BTreeMap::new()),
      sealed: AtomicBool::new(false),
    })
  }

  pub fn register(
    &self,
    workflow: &CompiledWorkflowDefinition,
    definition_hash: &str,
  ) -> Result<WorkflowTarget, WorkflowTargetRegistryError> {
    if self.sealed.load(Ordering::SeqCst) {
      return Err(WorkflowTargetRegistryError::RegistrySealed);
    }
    if workflow.validate_structure().is_err()
      || !valid_workflow_id(&workflow.workflow_id)
      || !is_definition_hash(definition_hash)
    {
      return Err(WorkflowTargetRegistryError::InvalidTarget);
    }
    let target = WorkflowTarget {
      runtime_id: self.runtime_id.clone(),
      workflow_id: workflow.workflow_id.clone(),
      definition_hash: definition_hash.to_string(),
    };
    let mut targets = self.targets.lock().expect("workflow target registry lock");
    if targets.contains_key(&target.workflow_id) {
      return Err(WorkflowTargetRegistryError::DuplicateWorkflowId(
        target.workflow_id,
      ));
    }
    targets.insert(target.workflow_id.clone(), target.clone());
    Ok(target)
  }

  pub fn seal(&self) {
    self.sealed.store(true, Ordering::SeqCst);
  }

  pub fn runtime_id(&self) -> &str {
    &self.runtime_id
  }

  pub fn owns(&self, workflow_id: &str, definition_hash: &str) -> bool {
    self
      .targets
      .lock()
      .expect("workflow target registry lock")
      .get(workflow_id)
      .is_some_and(|target| target.definition_hash == definition_hash)
  }

  pub fn resolve(&self, workflow_id: &str) -> Result<WorkflowTarget, WorkflowTargetRegistryError> {
    if !self.sealed.load(Ordering::SeqCst) {
      return Err(WorkflowTargetRegistryError::RegistryNotSealed);
    }
    self
      .targets
      .lock()
      .expect("workflow target registry lock")
      .get(workflow_id)
      .cloned()
      .ok_or_else(|| WorkflowTargetRegistryError::TargetNotFound(workflow_id.to_string()))
  }

  pub fn targets(&self) -> Vec<WorkflowTarget> {
    self
      .targets
      .lock()
      .expect("workflow target registry lock")
      .values()
      .cloned()
      .collect()
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowCallIndexState {
  Admitted,
  Running,
  Succeeded,
  Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowCallAdmissionRequest {
  pub call_key: String,
  pub child_run_id: String,
  pub parent_run_id: String,
  pub parent_node_id: String,
  pub parent_attempt: u32,
  pub target_workflow_id: String,
  pub target_definition_hash: String,
  pub payload: Map<String, Value>,
  pub admitted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowCallAdmission {
  pub call_key: String,
  pub parent_run_id: String,
  pub parent_node_id: String,
  pub parent_attempt: u32,
  pub target_workflow_id: String,
  pub target_definition_hash: String,
  pub child_run_id: String,
  pub payload_digest: String,
  pub depth: u32,
  pub state: WorkflowCallIndexState,
  pub admitted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowCallAdmissionOutcome {
  #[serde(flatten)]
  pub admission: WorkflowCallAdmission,
  pub duplicate: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkflowCallRequest {
  contract: String,
  contract_version: u32,
  kind: String,
  workflow_id: String,
  payload: Map<String, Value>,
  options: WorkflowCallOptions,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkflowCallOptions {
  #[serde(default)]
  name: Option<String>,
  #[serde(default)]
  timeout_ms: Option<u64>,
}

#[derive(Clone)]
pub struct ManagedWorkflowCallsHandler {
  database_path: PathBuf,
  targets: Arc<WorkflowTargetRegistry>,
  execution: Option<WorkflowCallExecutionConfig>,
}

#[derive(Clone)]
struct WorkflowCallExecutionConfig {
  script_host: ScriptHostProcessOptions,
  script_timeout_ms: u64,
  max_context_bytes: Option<usize>,
  clock: Arc<dyn EngineClock>,
  progress_reporter: Option<ExecutionProgressReporter>,
  schedule_clock: Arc<dyn ScheduleClock>,
  schedule_progress_reporter: Option<ScheduleProgressReporter>,
  interval_progress_reporter: Option<IntervalProgressReporter>,
  resolved_secrets: Arc<BTreeMap<String, String>>,
  capability_registry: Weak<CapabilityRegistry>,
}

impl WorkflowCallExecutionConfig {
  fn from_options(options: &RuntimeExecutionOptions) -> Self {
    Self {
      script_host: options.script_host.clone(),
      script_timeout_ms: options.script_timeout_ms,
      max_context_bytes: options.max_context_bytes,
      clock: Arc::clone(&options.clock),
      progress_reporter: options.progress_reporter.clone(),
      schedule_clock: Arc::clone(&options.schedule_clock),
      schedule_progress_reporter: options.schedule_progress_reporter.clone(),
      interval_progress_reporter: options.interval_progress_reporter.clone(),
      resolved_secrets: Arc::clone(&options.resolved_secrets),
      capability_registry: Arc::downgrade(&options.capability_registry),
    }
  }

  fn runtime_options(&self) -> Result<RuntimeExecutionOptions, CapabilityFailure> {
    let registry = self
      .capability_registry
      .upgrade()
      .ok_or_else(workflow_call_unavailable)?;
    let mut options =
      RuntimeExecutionOptions::new(self.script_host.clone(), self.script_timeout_ms)
        .with_capability_registry(registry);
    options.max_context_bytes = self.max_context_bytes;
    options.clock = Arc::clone(&self.clock);
    options.progress_reporter = self.progress_reporter.clone();
    options.schedule_clock = Arc::clone(&self.schedule_clock);
    options.schedule_progress_reporter = self.schedule_progress_reporter.clone();
    options.interval_progress_reporter = self.interval_progress_reporter.clone();
    options.resolved_secrets = Arc::clone(&self.resolved_secrets);
    Ok(options)
  }
}

#[derive(Clone)]
struct PreparedWorkflowCall {
  admission: WorkflowCallAdmissionRequest,
  timeout_ms: u64,
  route: PreparedWorkflowCallRoute,
}

#[derive(Clone)]
enum PreparedWorkflowCallRoute {
  Local,
  Remote(WorkflowRuntimeRoute),
}

impl ManagedWorkflowCallsHandler {
  pub fn new(database_path: PathBuf, targets: Arc<WorkflowTargetRegistry>) -> Self {
    Self {
      database_path,
      targets,
      execution: None,
    }
  }

  pub fn with_execution(mut self, options: &RuntimeExecutionOptions) -> Self {
    self.execution = Some(WorkflowCallExecutionConfig::from_options(options));
    self
  }

  fn prepare(
    &self,
    call: &CapabilityCallRequest,
  ) -> Result<PreparedWorkflowCall, CapabilityFailure> {
    let request = parse_request(&call.input)?;
    let identity_matches = match (&request.options.name, call.identity.mode) {
      (Some(name), CapabilityIdentityMode::Named) => name == &call.identity.operation_name,
      (None, CapabilityIdentityMode::Automatic) => true,
      _ => false,
    };
    if !identity_matches {
      return Err(workflow_call_failure(
        CapabilityFailureKind::InvalidInput,
        "WOML_WORKFLOW_CALL_IDENTITY_INVALID",
        "Workflow Call options.name does not match the engine-owned operation identity.",
        false,
      ));
    }
    let (target, route) = match self.targets.resolve(&request.workflow_id) {
      Ok(target) => (target, PreparedWorkflowCallRoute::Local),
      Err(WorkflowTargetRegistryError::TargetNotFound(_)) => {
        let store = DurableEventStore::open(&self.database_path).map_err(store_failure)?;
        let route = store
          .workflow_runtime_route(&request.workflow_id, Utc::now())
          .map_err(store_failure)?
          .ok_or_else(|| {
            target_failure(WorkflowTargetRegistryError::TargetNotFound(
              request.workflow_id.clone(),
            ))
          })?;
        (route.target(), PreparedWorkflowCallRoute::Remote(route))
      }
      Err(error) => return Err(target_failure(error)),
    };
    let call_key = derive_workflow_call_key(
      &call.identity.step_idempotency_key,
      &target.workflow_id,
      &call.identity.operation_name,
    );
    Ok(PreparedWorkflowCall {
      admission: WorkflowCallAdmissionRequest {
        child_run_id: child_run_id(&call_key),
        call_key,
        parent_run_id: call.run_id.clone(),
        parent_node_id: call.node_id.clone(),
        parent_attempt: call.attempt_number,
        target_workflow_id: target.workflow_id,
        target_definition_hash: target.definition_hash,
        payload: request.payload,
        admitted_at: Utc::now(),
      },
      timeout_ms: request.options.timeout_ms.unwrap_or(30_000),
      route,
    })
  }
}

impl CapabilityHandler for ManagedWorkflowCallsHandler {
  fn descriptor(&self) -> CapabilityDescriptor {
    CapabilityDescriptor {
      capability: "workflows".to_string(),
      operation: "call".to_string(),
      input_contract_version: WORKFLOW_CALL_CONTRACT_VERSION,
      result_contract_version: WORKFLOW_CALL_CONTRACT_VERSION,
      effect: CapabilityEffect::IdempotentWrite,
      supports_cancellation: true,
      supports_provider_idempotency: false,
    }
  }

  fn validate_request(&self, request: &CapabilityCallRequest) -> Result<(), CapabilityFailure> {
    parse_request(&request.input).map(|_| ())
  }

  fn safe_request_metadata(
    &self,
    call: &CapabilityCallRequest,
  ) -> Result<Map<String, Value>, CapabilityFailure> {
    let admission = self.prepare(call)?.admission;
    let store = DurableEventStore::open(&self.database_path).map_err(store_failure)?;
    let depth = store
      .workflow_call_depth_for_parent(&admission.parent_run_id)
      .map_err(store_failure)?
      .saturating_add(1);
    let payload_digest = workflow_call_payload_digest(&admission.payload)?;
    Ok(Map::from_iter([
      (
        "targetWorkflowId".to_string(),
        Value::String(admission.target_workflow_id),
      ),
      (
        "targetDefinitionHash".to_string(),
        Value::String(admission.target_definition_hash),
      ),
      (
        "childRunId".to_string(),
        Value::String(admission.child_run_id),
      ),
      ("payloadDigest".to_string(), Value::String(payload_digest)),
      ("lineageDepth".to_string(), Value::from(depth)),
    ]))
  }

  fn safe_result_metadata(&self, result: &Value) -> Map<String, Value> {
    let Some(child_run_id) = result.get("childRunId").and_then(Value::as_str) else {
      return Map::new();
    };
    let Some(digest) = child_run_id.strip_prefix("run_call_") else {
      return Map::new();
    };
    let call_key = format!("sha256:{digest}");
    let admission = DurableEventStore::open(&self.database_path)
      .and_then(|store| store.workflow_call(&call_key))
      .ok()
      .flatten();
    let Some(admission) = admission else {
      return Map::new();
    };
    Map::from_iter([
      (
        "targetWorkflowId".to_string(),
        Value::String(admission.target_workflow_id),
      ),
      (
        "targetDefinitionHash".to_string(),
        Value::String(admission.target_definition_hash),
      ),
      (
        "childRunId".to_string(),
        Value::String(admission.child_run_id),
      ),
      (
        "payloadDigest".to_string(),
        Value::String(admission.payload_digest),
      ),
      ("lineageDepth".to_string(), Value::from(admission.depth)),
    ])
  }

  fn execute(
    &self,
    _input: Value,
    _cancellation: CapabilityCancellationToken,
  ) -> BoxFuture<'static, Result<Value, CapabilityFailure>> {
    Box::pin(async { Err(workflow_call_unavailable()) })
  }

  fn execute_request_scoped(
    &self,
    call: &CapabilityCallRequest,
    _workflow_scope: Option<String>,
    cancellation: CapabilityCancellationToken,
  ) -> BoxFuture<'static, Result<Value, CapabilityFailure>> {
    let prepared = match self.prepare(call) {
      Ok(prepared) => prepared,
      Err(error) => return Box::pin(async move { Err(error) }),
    };
    let database_path = self.database_path.clone();
    let execution = self.execution.clone();
    Box::pin(async move {
      if cancellation.is_cancelled() {
        return Err(workflow_call_cancelled());
      }
      let admission_request = prepared.admission;
      let admission_database_path = database_path.clone();
      let outcome = tokio::task::spawn_blocking(move || {
        DurableEventStore::open(admission_database_path)?.admit_workflow_call(admission_request)
      })
      .await
      .map_err(|_| workflow_call_unavailable())?
      .map_err(store_failure)?;
      let Some(execution) = execution else {
        return Ok(admission_result(&outcome));
      };
      execute_child_and_wait(
        database_path,
        outcome,
        execution,
        prepared.timeout_ms,
        cancellation,
        prepared.route,
      )
      .await
    })
  }
}

fn admission_result(outcome: &WorkflowCallAdmissionOutcome) -> Value {
  json!({
    "contract": "woml.workflow-call-admission",
    "contractVersion": 1,
    "kind": "admitted",
    "data": {
      "targetWorkflowId": outcome.admission.target_workflow_id,
      "targetDefinitionHash": outcome.admission.target_definition_hash,
      "childRunId": outcome.admission.child_run_id,
      "payloadDigest": outcome.admission.payload_digest,
      "lineageDepth": outcome.admission.depth,
      "duplicate": outcome.duplicate,
    }
  })
}

async fn execute_child_and_wait(
  database_path: PathBuf,
  outcome: WorkflowCallAdmissionOutcome,
  execution: WorkflowCallExecutionConfig,
  timeout_ms: u64,
  cancellation: CapabilityCancellationToken,
  route: PreparedWorkflowCallRoute,
) -> Result<Value, CapabilityFailure> {
  let admission = outcome.admission;
  match route {
    PreparedWorkflowCallRoute::Local => {
      dispatch_admitted_workflow_call(
        database_path.clone(),
        admission.clone(),
        execution.runtime_options()?,
      )
      .await?;
    }
    PreparedWorkflowCallRoute::Remote(route) => {
      wake_remote_workflow_call(&database_path, &route, &admission).await?;
    }
  }
  observe_existing_child(&database_path, &admission, timeout_ms, &cancellation).await
}

/// Claims and starts one already-admitted child. Returning `false` means
/// another executor already owns it or the child is terminal.
pub async fn dispatch_admitted_workflow_call(
  database_path: PathBuf,
  admission: WorkflowCallAdmission,
  runtime_options: RuntimeExecutionOptions,
) -> Result<bool, CapabilityFailure> {
  let mut store = DurableEventStore::open(&database_path).map_err(store_failure)?;
  let claimed = store
    .claim_workflow_call_execution(&admission.call_key)
    .map_err(store_failure)?;
  drop(store);
  if !claimed {
    return Ok(false);
  }
  tokio::spawn(async move {
    let result = execute_admitted_trigger_run_durable(
      database_path.clone(),
      &admission.child_run_id,
      runtime_options,
    )
    .await;
    let terminal_state = if matches!(result, Ok(WorkflowRuntimeOutcome::Succeeded { .. })) {
      WorkflowCallIndexState::Succeeded
    } else {
      WorkflowCallIndexState::Failed
    };
    let _ = DurableEventStore::open(&database_path).and_then(|mut store| {
      store.transition_workflow_call_state(
        &admission.call_key,
        WorkflowCallIndexState::Running,
        terminal_state,
      )
    });
  });
  Ok(true)
}

async fn wake_remote_workflow_call(
  database_path: &Path,
  route: &WorkflowRuntimeRoute,
  admission: &WorkflowCallAdmission,
) -> Result<(), CapabilityFailure> {
  let credential = workflow_routing_session_credential(database_path, &route.runtime_id)
    .map_err(|_| workflow_call_unavailable())?;
  if workflow_routing_credential_hash(&credential) != route.session_credential_hash {
    return Err(workflow_call_unavailable());
  }
  let wakeup = WorkflowRoutingWakeup {
    contract: WORKFLOW_ROUTING_CONTRACT.to_string(),
    contract_version: WORKFLOW_ROUTING_CONTRACT_VERSION,
    kind: "wakeup".to_string(),
    runtime_id: route.runtime_id.clone(),
    child_run_id: admission.child_run_id.clone(),
    call_key: admission.call_key.clone(),
  };
  let response = reqwest::Client::new()
    .post(format!("{}{}", route.endpoint, WORKFLOW_ROUTING_WAKE_PATH))
    .bearer_auth(&credential)
    .timeout(Duration::from_secs(2))
    .json(&wakeup)
    .send()
    .await;
  let Ok(response) = response else {
    // The target's durable pending-call scanner is the lost-wake-up fallback.
    return Ok(());
  };
  let status = response.status();
  let acknowledgement = response
    .json::<WorkflowRoutingAcknowledgement>()
    .await
    .map_err(|_| workflow_call_unavailable())?;
  if status.is_success()
    && acknowledgement.contract == WORKFLOW_ROUTING_CONTRACT
    && acknowledgement.contract_version == WORKFLOW_ROUTING_CONTRACT_VERSION
    && acknowledgement.kind == "acknowledgement"
    && acknowledgement.child_run_id == admission.child_run_id
    && acknowledgement.accepted
  {
    return Ok(());
  }
  let code = acknowledgement.code.as_deref().unwrap_or_default();
  if code == "WOML_WORKFLOW_DEFINITION_MISMATCH" {
    return Err(workflow_call_failure_with_child(
      CapabilityFailureKind::ServiceRejected,
      "WOML_WORKFLOW_DEFINITION_MISMATCH",
      "The selected target no longer matches its registered definition.",
      false,
      admission,
    ));
  }
  Err(workflow_call_unavailable())
}

pub fn workflow_routing_session_credential(
  database_path: &Path,
  runtime_id: &str,
) -> std::io::Result<String> {
  let key = load_or_create_workflow_routing_key(database_path)?;
  let mut digest = Sha256::new();
  digest.update(b"woml.workflow-call-routing\0v1\0");
  digest.update(&key);
  digest.update(b"\0");
  digest.update(runtime_id.as_bytes());
  Ok(hex::encode(digest.finalize()))
}

pub fn workflow_routing_credential_hash(credential: &str) -> String {
  format!(
    "sha256:{}",
    hex::encode(Sha256::digest(credential.as_bytes()))
  )
}

fn load_or_create_workflow_routing_key(database_path: &Path) -> std::io::Result<[u8; 32]> {
  let key_path = workflow_routing_key_path(database_path);
  if let Ok(mut file) = OpenOptions::new().read(true).open(&key_path) {
    let mut key = [0_u8; 32];
    file.read_exact(&mut key)?;
    let mut extra = [0_u8; 1];
    if file.read(&mut extra)? != 0 {
      return Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "WOML workflow routing key has an invalid size",
      ));
    }
    return Ok(key);
  }
  if let Some(parent) = key_path.parent() {
    fs::create_dir_all(parent)?;
  }
  let mut key = [0_u8; 32];
  getrandom::getrandom(&mut key)
    .map_err(|error| std::io::Error::other(format!("routing key generation failed: {error}")))?;
  let mut options = OpenOptions::new();
  options.write(true).create_new(true);
  #[cfg(unix)]
  {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
  }
  match options.open(&key_path) {
    Ok(mut file) => {
      file.write_all(&key)?;
      file.sync_all()?;
      Ok(key)
    }
    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
      load_or_create_workflow_routing_key(database_path)
    }
    Err(error) => Err(error),
  }
}

fn workflow_routing_key_path(database_path: &Path) -> PathBuf {
  let mut value = database_path.as_os_str().to_os_string();
  value.push(".workflow-routing-v1.key");
  PathBuf::from(value)
}

async fn observe_existing_child(
  database_path: &std::path::Path,
  admission: &WorkflowCallAdmission,
  timeout_ms: u64,
  cancellation: &CapabilityCancellationToken,
) -> Result<Value, CapabilityFailure> {
  let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
  loop {
    let store = DurableEventStore::open(database_path.to_path_buf()).map_err(store_failure)?;
    let projection = store
      .projection(&admission.child_run_id)
      .map_err(store_failure)?;
    match projection.status {
      RunStatus::Succeeded => {
        let result = projection
          .result
          .ok_or_else(|| workflow_call_result_missing(admission))?;
        return workflow_call_success(admission, result);
      }
      RunStatus::Failed => {
        return if matches!(
          projection.failure,
          Some(RunFailure::Attempt(ref failure))
            if failure.kind == AttemptFailureKind::InvalidScriptResult
        ) {
          Err(workflow_call_result_missing(admission))
        } else {
          Err(workflow_call_child_failed(admission))
        };
      }
      RunStatus::Waiting => {
        return Err(workflow_call_wait_unsupported(admission));
      }
      RunStatus::NotStarted | RunStatus::Running => {}
    }
    tokio::select! {
      _ = cancellation.cancelled() => {
        return Err(workflow_call_cancelled_after_admission(admission));
      }
      _ = tokio::time::sleep_until(deadline) => {
        return Err(workflow_call_timed_out(admission));
      }
      _ = tokio::time::sleep(Duration::from_millis(10)) => {}
    }
  }
}

fn workflow_call_success(
  admission: &WorkflowCallAdmission,
  result: Value,
) -> Result<Value, CapabilityFailure> {
  let result_bytes = serde_json::to_vec(&result)
    .map_err(|_| workflow_call_result_missing(admission))?
    .len();
  if result_bytes > MAX_WORKFLOW_CALL_RESULT_BYTES {
    return Err(workflow_call_failure_with_child(
      CapabilityFailureKind::ResultTooLarge,
      "WOML_WORKFLOW_RESULT_TOO_LARGE",
      "The called workflow result exceeds the 4 MiB limit.",
      false,
      admission,
    ));
  }
  Ok(json!({
    "contract": WORKFLOW_CALL_CONTRACT,
    "contractVersion": WORKFLOW_CALL_CONTRACT_VERSION,
    "kind": "succeeded",
    "workflowId": admission.target_workflow_id,
    "definitionHash": admission.target_definition_hash,
    "childRunId": admission.child_run_id,
    "result": result,
  }))
}

pub fn derive_workflow_call_key(
  step_idempotency_key: &str,
  target_workflow_id: &str,
  operation_name: &str,
) -> String {
  let mut digest = Sha256::new();
  digest.update(b"woml.workflow-call\0v1\0");
  digest.update(step_idempotency_key.as_bytes());
  digest.update(b"\0");
  digest.update(target_workflow_id.as_bytes());
  digest.update(b"\0");
  digest.update(operation_name.as_bytes());
  format!("sha256:{}", hex::encode(digest.finalize()))
}

pub(crate) fn workflow_call_payload_digest(
  payload: &Map<String, Value>,
) -> Result<String, CapabilityFailure> {
  let encoded = serde_json_canonicalizer::to_vec(payload).map_err(|_| {
    workflow_call_failure(
      CapabilityFailureKind::InvalidInput,
      "WOML_WORKFLOW_PAYLOAD_INVALID",
      "The workflow call payload is not canonical JSON.",
      false,
    )
  })?;
  if encoded.len() > MAX_WORKFLOW_CALL_PAYLOAD_BYTES {
    return Err(workflow_call_failure(
      CapabilityFailureKind::InputTooLarge,
      "WOML_WORKFLOW_PAYLOAD_TOO_LARGE",
      "The workflow call payload exceeds the 1 MiB limit.",
      false,
    ));
  }
  Ok(format!("sha256:{}", hex::encode(Sha256::digest(encoded))))
}

fn child_run_id(call_key: &str) -> String {
  format!(
    "run_call_{}",
    call_key.strip_prefix("sha256:").unwrap_or(call_key)
  )
}

fn parse_request(input: &Value) -> Result<WorkflowCallRequest, CapabilityFailure> {
  let request: WorkflowCallRequest = serde_json::from_value(input.clone()).map_err(|_| {
    workflow_call_failure(
      CapabilityFailureKind::InvalidInput,
      "WOML_WORKFLOW_TARGET_INVALID",
      "Workflow Call request is invalid.",
      false,
    )
  })?;
  let valid_options = request
    .options
    .name
    .as_deref()
    .is_none_or(valid_operation_name)
    && request
      .options
      .timeout_ms
      .is_none_or(|value| (1..=86_400_000).contains(&value));
  if request.contract != WORKFLOW_CALL_CONTRACT
    || request.contract_version != WORKFLOW_CALL_CONTRACT_VERSION
    || request.kind != "request"
    || !valid_workflow_id(&request.workflow_id)
    || !valid_options
  {
    return Err(workflow_call_failure(
      CapabilityFailureKind::InvalidInput,
      "WOML_WORKFLOW_TARGET_INVALID",
      "Workflow Call request does not match Workflow Call v1.",
      false,
    ));
  }
  workflow_call_payload_digest(&request.payload)?;
  Ok(request)
}

fn valid_workflow_id(value: &str) -> bool {
  value.len() <= 256
    && value.split('-').all(|segment| {
      let mut characters = segment.chars();
      matches!(characters.next(), Some(first) if first.is_ascii_lowercase())
        && characters.all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
    })
}

fn valid_operation_name(value: &str) -> bool {
  !value.is_empty()
    && value.len() <= 128
    && value.split(['.', '_', '-']).all(|segment| {
      let mut characters = segment.chars();
      matches!(characters.next(), Some(first) if first.is_ascii_lowercase())
        && characters.all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
    })
}

fn target_failure(error: WorkflowTargetRegistryError) -> CapabilityFailure {
  match error {
    WorkflowTargetRegistryError::TargetNotFound(_) => workflow_call_failure(
      CapabilityFailureKind::ServiceRejected,
      "WOML_WORKFLOW_TARGET_NOT_FOUND",
      "No active workflow owns the requested workflow ID.",
      false,
    ),
    WorkflowTargetRegistryError::DuplicateWorkflowId(_) => workflow_call_failure(
      CapabilityFailureKind::ServiceRejected,
      "WOML_WORKFLOW_TARGET_AMBIGUOUS",
      "More than one active runtime owns the requested workflow ID.",
      false,
    ),
    _ => workflow_call_failure(
      CapabilityFailureKind::ServiceRejected,
      "WOML_WORKFLOW_TARGET_UNAVAILABLE",
      "The workflow target registry is unavailable.",
      true,
    ),
  }
}

fn store_failure(error: DurableStoreError) -> CapabilityFailure {
  let (kind, code, message, retryable) = match error {
    DurableStoreError::WorkflowCallIdempotencyConflict => (
      CapabilityFailureKind::ServiceRejected,
      "WOML_WORKFLOW_CALL_IDEMPOTENCY_CONFLICT",
      "The stable workflow call identity is already bound to different input.",
      false,
    ),
    DurableStoreError::WorkflowCallDepthExceeded => (
      CapabilityFailureKind::ServiceRejected,
      "WOML_WORKFLOW_CALL_DEPTH_EXCEEDED",
      "The workflow call exceeds the maximum lineage depth.",
      false,
    ),
    DurableStoreError::WorkflowCallCycle => (
      CapabilityFailureKind::ServiceRejected,
      "WOML_WORKFLOW_CALL_CYCLE",
      "The workflow call would repeat a workflow already present in its lineage.",
      false,
    ),
    DurableStoreError::WorkflowCallDefinitionMismatch => (
      CapabilityFailureKind::ServiceRejected,
      "WOML_WORKFLOW_DEFINITION_MISMATCH",
      "The selected target no longer matches its registered definition.",
      false,
    ),
    DurableStoreError::WorkflowCallHistoryInvalid(_) => (
      CapabilityFailureKind::ServiceRejected,
      "WOML_WORKFLOW_CALL_FAILED",
      "The durable workflow call history is contradictory.",
      false,
    ),
    _ => (
      CapabilityFailureKind::TransportFailed,
      "WOML_WORKFLOW_TARGET_UNAVAILABLE",
      "The durable workflow call authority is unavailable.",
      true,
    ),
  };
  workflow_call_failure(kind, code, message, retryable)
}

fn workflow_call_unavailable() -> CapabilityFailure {
  workflow_call_failure(
    CapabilityFailureKind::ServiceRejected,
    "WOML_WORKFLOW_TARGET_UNAVAILABLE",
    "The durable workflow call authority is unavailable.",
    true,
  )
}

fn workflow_call_cancelled() -> CapabilityFailure {
  workflow_call_failure(
    CapabilityFailureKind::Cancelled,
    "WOML_WORKFLOW_CALL_CANCELLED",
    "The workflow call was cancelled before child admission.",
    false,
  )
}

fn workflow_call_cancelled_after_admission(admission: &WorkflowCallAdmission) -> CapabilityFailure {
  workflow_call_failure_with_child(
    CapabilityFailureKind::Cancelled,
    "WOML_WORKFLOW_CALL_CANCELLED",
    "The parent stopped waiting; the admitted child workflow remains independent.",
    false,
    admission,
  )
}

fn workflow_call_timed_out(admission: &WorkflowCallAdmission) -> CapabilityFailure {
  workflow_call_failure_with_child(
    CapabilityFailureKind::TimedOut,
    "WOML_WORKFLOW_CALL_TIMED_OUT",
    "The parent stopped waiting before the child workflow finished.",
    false,
    admission,
  )
}

fn workflow_call_result_missing(admission: &WorkflowCallAdmission) -> CapabilityFailure {
  workflow_call_failure_with_child(
    CapabilityFailureKind::InvalidResult,
    "WOML_WORKFLOW_RESULT_MISSING",
    "The called workflow reached success without a JSON result.",
    false,
    admission,
  )
}

fn workflow_call_wait_unsupported(admission: &WorkflowCallAdmission) -> CapabilityFailure {
  workflow_call_failure_with_child(
    CapabilityFailureKind::ServiceRejected,
    "WOML_WORKFLOW_CALL_WAIT_UNSUPPORTED",
    "Workflow Calls v1 cannot wait for a child Human Approval.",
    false,
    admission,
  )
}

fn workflow_call_child_failed(admission: &WorkflowCallAdmission) -> CapabilityFailure {
  workflow_call_failure_with_child(
    CapabilityFailureKind::ServiceRejected,
    "WOML_WORKFLOW_CALL_FAILED",
    "The called workflow failed. Inspect its child run for details.",
    false,
    admission,
  )
}

fn workflow_call_failure_with_child(
  kind: CapabilityFailureKind,
  code: &str,
  message: &str,
  retryable: bool,
  admission: &WorkflowCallAdmission,
) -> CapabilityFailure {
  let mut failure = workflow_call_failure(kind, code, message, retryable);
  failure.details = Some(Map::from_iter([
    (
      "workflowId".to_string(),
      Value::String(admission.target_workflow_id.clone()),
    ),
    (
      "childRunId".to_string(),
      Value::String(admission.child_run_id.clone()),
    ),
  ]));
  failure
}

fn workflow_call_failure(
  kind: CapabilityFailureKind,
  code: &str,
  message: &str,
  retryable: bool,
) -> CapabilityFailure {
  CapabilityFailure {
    kind,
    code: code.to_string(),
    message: message.to_string(),
    retryable,
    ambiguous: false,
    details: None,
  }
}
