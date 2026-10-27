//! Managed Process v1 capability executed and authorized by the Rust core.

use std::{
  collections::{BTreeMap, BTreeSet, HashMap},
  io,
  path::{Component, Path, PathBuf},
  process::{ExitStatus, Stdio},
  sync::{Arc, Mutex},
  time::{Duration, Instant},
};

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use thiserror::Error;
use tokio::{
  io::{AsyncRead, AsyncReadExt},
  process::{Child, Command},
  sync::{Notify, OwnedSemaphorePermit, Semaphore},
  task::JoinHandle,
};

const PROCESS_TERMINATION_GRACE: Duration = Duration::from_millis(250);
const PROCESS_PIPE_DRAIN_GRACE: Duration = Duration::from_secs(1);

use crate::{
  CapabilityCallRequest, CapabilityCancellationToken, CapabilityDescriptor, CapabilityEffect,
  CapabilityFailure, CapabilityFailureKind, CapabilityHandler, LazyOperationalOutputStore,
  OperationalBodyV1, OperationalCloseStatusV1, OperationalIdentityV1, OperationalOutputStore,
  OperationalRecordDraftV1, OperationalSourceV1,
};

pub const PROCESS_CONTRACT: &str = "woml.process";
pub const PROCESS_CONTRACT_VERSION: u32 = 1;
pub const PROCESS_CONTRACT_VERSION_V2: u32 = 2;
pub const PROCESS_POLICY_CONTRACT: &str = "woml.process-policy";
pub const PROCESS_POLICY_CONTRACT_VERSION: u32 = 1;
const MAX_PROCESS_STREAM_BYTES: usize = 256 * 1024 * 1024;

fn default_process_stream_bytes() -> usize {
  crate::DEFAULT_OPERATIONAL_INLINE_RUN_BYTES as usize
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProcessRuntimeMode {
  Foreground,
  Background,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessPolicyV1 {
  pub contract: String,
  pub contract_version: u32,
  pub enabled: bool,
  pub modes: ProcessPolicyModes,
  pub project_root: String,
  pub executables: Vec<ProcessPolicyExecutable>,
  pub working_directories: Vec<String>,
  pub environment: ProcessPolicyEnvironment,
  pub limits: ProcessPolicyLimits,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessPolicyModes {
  pub foreground: bool,
  pub background: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessPolicyExecutable {
  pub alias: String,
  pub path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessPolicyEnvironment {
  pub inherit: Vec<String>,
  pub allow: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessPolicyLimits {
  pub max_timeout_ms: u64,
  pub max_output_bytes: usize,
  #[serde(default = "default_process_stream_bytes")]
  pub max_stream_bytes: usize,
  pub max_concurrent: usize,
}

#[derive(Debug, Error)]
pub enum ProcessPolicyError {
  #[error("WOML_PROCESS_DISABLED: process execution is disabled by runtime policy. Corrective action: grant a reviewed foreground --allow-process list or configure Process Policy v1")]
  Disabled,
  #[error("WOML_PROCESS_MODE_DENIED: process execution is not allowed in {0} mode. Corrective action: enable that mode in the operator-owned Process Policy v1 document")]
  ModeDenied(&'static str),
  #[error("WOML_PROCESS_POLICY_INVALID: {0}. Corrective action: repair the Process Policy v1 document and run WOML again")]
  Invalid(String),
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum OutputMode {
  Capture,
  Discard,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProcessOutputRequest {
  stdout: OutputMode,
  stderr: OutputMode,
  max_bytes: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProcessRunRequest {
  contract: String,
  contract_version: u32,
  kind: String,
  executable: String,
  args: Vec<String>,
  cwd: String,
  env: BTreeMap<String, String>,
  timeout_ms: u64,
  accept_exit_codes: Vec<i32>,
  output: ProcessOutputRequest,
  #[serde(default)]
  stream: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProcessRunResultV1 {
  contract: &'static str,
  contract_version: u32,
  kind: &'static str,
  exit_code: Option<i32>,
  signal: Option<String>,
  stdout: String,
  stderr: String,
  stdout_bytes: u64,
  stderr_bytes: u64,
  stdout_truncated: bool,
  stderr_truncated: bool,
  duration_ms: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProcessRunResultV2 {
  contract: &'static str,
  contract_version: u32,
  kind: &'static str,
  exit_code: Option<i32>,
  signal: Option<String>,
  stdout: String,
  stderr: String,
  stdout_bytes: u64,
  stderr_bytes: u64,
  stdout_truncated: bool,
  stderr_truncated: bool,
  duration_ms: f64,
  streamed: bool,
  stream_start_sequence: Option<u64>,
  stream_end_sequence: Option<u64>,
  stream_truncated: bool,
  stream_artifact: Option<Value>,
}

#[derive(Debug)]
struct PreparedProcessPolicy {
  project_root: PathBuf,
  executables: HashMap<String, ApprovedExecutable>,
  working_directories: Vec<PathBuf>,
  inherited_environment: BTreeMap<String, String>,
  allowed_environment: BTreeSet<String>,
  limits: ProcessPolicyLimits,
}

#[derive(Debug)]
struct ApprovedExecutable {
  path: PathBuf,
  identity: ExecutableIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExecutableIdentity {
  length: u64,
  modified: Option<std::time::SystemTime>,
  #[cfg(unix)]
  device: u64,
  #[cfg(unix)]
  inode: u64,
  #[cfg(windows)]
  volume_serial_number: Option<u32>,
  #[cfg(windows)]
  file_index: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ManagedProcessHandler {
  policy: Arc<PreparedProcessPolicy>,
  concurrency: Arc<Semaphore>,
  output_store: Arc<Mutex<Option<Arc<LazyOperationalOutputStore>>>>,
  output_artifacts: Arc<Mutex<Option<Arc<crate::ManagedArtifactPutHandler>>>>,
  resolved_secrets: Arc<BTreeMap<String, String>>,
}

impl ManagedProcessHandler {
  pub fn new(
    policy: ProcessPolicyV1,
    mode: ProcessRuntimeMode,
  ) -> Result<Self, ProcessPolicyError> {
    let prepared = PreparedProcessPolicy::prepare(policy, mode)?;
    let max_concurrent = prepared.limits.max_concurrent;
    Ok(Self {
      policy: Arc::new(prepared),
      concurrency: Arc::new(Semaphore::new(max_concurrent)),
      output_store: Arc::new(Mutex::new(None)),
      output_artifacts: Arc::new(Mutex::new(None)),
      resolved_secrets: Arc::new(BTreeMap::new()),
    })
  }

  pub fn new_with_secrets(
    policy: ProcessPolicyV1,
    mode: ProcessRuntimeMode,
    resolved_secrets: Arc<BTreeMap<String, String>>,
  ) -> Result<Self, ProcessPolicyError> {
    let mut handler = Self::new(policy, mode)?;
    handler.resolved_secrets = resolved_secrets;
    Ok(handler)
  }

  pub fn configure_output_store(
    &self,
    store: Arc<LazyOperationalOutputStore>,
  ) -> Result<(), ProcessPolicyError> {
    let mut configured = self.output_store.lock().map_err(|_| {
      ProcessPolicyError::Invalid("the operational-output authority is unavailable".to_string())
    })?;
    if let Some(existing) = configured.as_ref() {
      if !Arc::ptr_eq(existing, &store) {
        return Err(ProcessPolicyError::Invalid(
          "the process handler is already bound to another operational-output authority"
            .to_string(),
        ));
      }
      return Ok(());
    }
    *configured = Some(store);
    Ok(())
  }

  pub(crate) fn configure_output_artifacts(
    &self,
    handler: Arc<crate::ManagedArtifactPutHandler>,
  ) -> Result<(), ProcessPolicyError> {
    let mut configured = self.output_artifacts.lock().map_err(|_| {
      ProcessPolicyError::Invalid("the operational artifact authority is unavailable".to_string())
    })?;
    if let Some(existing) = configured.as_ref() {
      if !Arc::ptr_eq(existing, &handler) {
        return Err(ProcessPolicyError::Invalid(
          "the process handler is already bound to another artifact authority".to_string(),
        ));
      }
      return Ok(());
    }
    *configured = Some(handler);
    Ok(())
  }

  fn parse_and_authorize(&self, input: &Value) -> Result<AuthorizedProcess, CapabilityFailure> {
    let mut request = parse_request(input)?;
    request.timeout_ms = request.timeout_ms.min(self.policy.limits.max_timeout_ms);
    if request.output.max_bytes > self.policy.limits.max_output_bytes {
      return Err(rejected(
        "WOML_PROCESS_OUTPUT_LIMIT_DENIED",
        "The requested process output limit exceeds the runtime policy limit.",
      ));
    }
    let executable = self
      .policy
      .executables
      .get(&request.executable)
      .ok_or_else(|| {
        rejected(
          "WOML_PROCESS_EXECUTABLE_DENIED",
          "The requested executable alias is not allowed by runtime policy.",
        )
      })?;
    verify_executable(executable)?;
    for name in request.env.keys() {
      if !self.policy.allowed_environment.contains(name) {
        return Err(rejected(
          "WOML_PROCESS_ENV_DENIED",
          "The process request contains an environment variable not allowed by runtime policy.",
        ));
      }
    }
    let cwd = authorize_cwd(&self.policy, &request.cwd)?;
    Ok(AuthorizedProcess {
      request,
      executable: executable.path.clone(),
      cwd,
    })
  }
}

impl PreparedProcessPolicy {
  fn prepare(
    policy: ProcessPolicyV1,
    mode: ProcessRuntimeMode,
  ) -> Result<Self, ProcessPolicyError> {
    if policy.contract != PROCESS_POLICY_CONTRACT
      || policy.contract_version != PROCESS_POLICY_CONTRACT_VERSION
    {
      return Err(ProcessPolicyError::Invalid(
        "the policy does not match Process Policy v1".to_string(),
      ));
    }
    if !policy.enabled {
      return Err(ProcessPolicyError::Disabled);
    }
    let allowed = match mode {
      ProcessRuntimeMode::Foreground => policy.modes.foreground,
      ProcessRuntimeMode::Background => policy.modes.background,
    };
    if !allowed {
      return Err(ProcessPolicyError::ModeDenied(match mode {
        ProcessRuntimeMode::Foreground => "foreground",
        ProcessRuntimeMode::Background => "background",
      }));
    }
    validate_limits(&policy.limits)?;
    if policy.project_root.is_empty()
      || policy.project_root.len() > 4096
      || policy.executables.len() > 128
      || policy.working_directories.len() > 128
      || policy.environment.inherit.len() > 128
      || policy.environment.allow.len() > 128
    {
      return Err(ProcessPolicyError::Invalid(
        "the policy exceeds Process Policy v1 collection or path bounds".to_string(),
      ));
    }
    let project_root = canonical_directory(Path::new(&policy.project_root), "project root")?;
    if policy.working_directories.is_empty() {
      return Err(ProcessPolicyError::Invalid(
        "at least one working-directory root is required".to_string(),
      ));
    }
    let mut working_directories = Vec::with_capacity(policy.working_directories.len());
    let mut authored_working_directories = BTreeSet::new();
    for value in &policy.working_directories {
      if value.is_empty()
        || value.len() > 4096
        || !authored_working_directories.insert(value.clone())
      {
        return Err(ProcessPolicyError::Invalid(
          "working-directory roots must be unique non-empty paths within the v1 size bound"
            .to_string(),
        ));
      }
      let candidate = Path::new(value);
      let path = if candidate.is_absolute() {
        candidate.to_path_buf()
      } else {
        project_root.join(candidate)
      };
      let canonical = canonical_directory(&path, "working-directory root")?;
      if !canonical.starts_with(&project_root) {
        return Err(ProcessPolicyError::Invalid(
          "a working-directory root escapes the project root".to_string(),
        ));
      }
      if !working_directories.contains(&canonical) {
        working_directories.push(canonical);
      }
    }
    let mut executables = HashMap::new();
    for entry in &policy.executables {
      validate_alias(&entry.alias)?;
      if entry.path.is_empty() || entry.path.len() > 4096 {
        return Err(ProcessPolicyError::Invalid(format!(
          "executable alias '{}' has an invalid path",
          entry.alias
        )));
      }
      let canonical = std::fs::canonicalize(&entry.path).map_err(|_| {
        ProcessPolicyError::Invalid(format!(
          "executable alias '{}' points to a missing path",
          entry.alias
        ))
      })?;
      let identity = verify_executable_policy(&canonical, &entry.alias)?;
      if executables
        .insert(
          entry.alias.clone(),
          ApprovedExecutable {
            path: canonical,
            identity,
          },
        )
        .is_some()
      {
        return Err(ProcessPolicyError::Invalid(format!(
          "executable alias '{}' is duplicated",
          entry.alias
        )));
      }
    }
    let mut inherited_environment = BTreeMap::new();
    let mut inherited_names = BTreeSet::new();
    for name in &policy.environment.inherit {
      validate_environment_name(name).map_err(ProcessPolicyError::Invalid)?;
      if !inherited_names.insert(name.clone()) {
        return Err(ProcessPolicyError::Invalid(
          "inherited environment-variable names must be unique".to_string(),
        ));
      }
      if let Ok(value) = std::env::var(name) {
        inherited_environment.insert(name.clone(), value);
      }
    }
    let mut allowed_environment = BTreeSet::new();
    for name in &policy.environment.allow {
      validate_environment_name(name).map_err(ProcessPolicyError::Invalid)?;
      if !allowed_environment.insert(name.clone()) {
        return Err(ProcessPolicyError::Invalid(
          "explicit environment-variable names must be unique".to_string(),
        ));
      }
    }
    Ok(Self {
      project_root,
      executables,
      working_directories,
      inherited_environment,
      allowed_environment,
      limits: policy.limits,
    })
  }
}

impl CapabilityHandler for ManagedProcessHandler {
  fn descriptor(&self) -> CapabilityDescriptor {
    CapabilityDescriptor {
      capability: "process".to_string(),
      operation: "run".to_string(),
      input_contract_version: PROCESS_CONTRACT_VERSION,
      result_contract_version: PROCESS_CONTRACT_VERSION,
      effect: CapabilityEffect::UnsafeWrite,
      supports_cancellation: true,
      supports_provider_idempotency: false,
    }
  }

  fn validate_request(&self, request: &CapabilityCallRequest) -> Result<(), CapabilityFailure> {
    self.parse_and_authorize(&request.input).map(|_| ())
  }

  fn safe_metadata(&self, input: &Value) -> Map<String, Value> {
    let Ok(request) = parse_request(input) else {
      return Map::new();
    };
    Map::from_iter([
      ("executable".to_string(), Value::String(request.executable)),
      ("argumentCount".to_string(), json!(request.args.len())),
      (
        "workingDirectory".to_string(),
        Value::String("policy-relative".to_string()),
      ),
      ("timeoutMs".to_string(), json!(request.timeout_ms)),
      (
        "acceptedExitCodeCount".to_string(),
        json!(request.accept_exit_codes.len()),
      ),
      (
        "outputMaxBytes".to_string(),
        json!(request.output.max_bytes),
      ),
      (
        "stdoutMode".to_string(),
        Value::String(output_mode_name(request.output.stdout).to_string()),
      ),
      (
        "stderrMode".to_string(),
        Value::String(output_mode_name(request.output.stderr).to_string()),
      ),
      ("stream".to_string(), Value::Bool(request.stream)),
    ])
  }

  fn safe_result_metadata(&self, result: &Value) -> Map<String, Value> {
    let mut safe = Map::from_iter([(
      "outcome".to_string(),
      Value::String("succeeded".to_string()),
    )]);
    for field in [
      "exitCode",
      "signal",
      "durationMs",
      "stdoutBytes",
      "stderrBytes",
      "stdoutTruncated",
      "stderrTruncated",
    ] {
      if let Some(value) = result.get(field) {
        safe.insert(field.to_string(), value.clone());
      }
    }
    safe
  }

  fn safe_failure_metadata(&self, error: &CapabilityFailure) -> Map<String, Value> {
    let outcome = match error.kind {
      CapabilityFailureKind::Cancelled => "cancelled",
      CapabilityFailureKind::TimedOut => "timed_out",
      CapabilityFailureKind::Interrupted => "interrupted",
      _ => "failed",
    };
    let mut safe = Map::from_iter([("outcome".to_string(), Value::String(outcome.to_string()))]);
    if let Some(details) = error.details.as_ref() {
      for field in [
        "exitCode",
        "signal",
        "durationMs",
        "stdoutBytes",
        "stderrBytes",
        "stdoutTruncated",
        "stderrTruncated",
      ] {
        if let Some(value) = details.get(field) {
          safe.insert(field.to_string(), value.clone());
        }
      }
    }
    safe
  }

  fn execute(
    &self,
    input: Value,
    cancellation: CapabilityCancellationToken,
  ) -> BoxFuture<'static, Result<Value, CapabilityFailure>> {
    let authorized = match self.parse_and_authorize(&input) {
      Ok(authorized) => authorized,
      Err(error) => return Box::pin(async move { Err(error) }),
    };
    let policy = Arc::clone(&self.policy);
    let concurrency = Arc::clone(&self.concurrency);
    Box::pin(async move {
      let permit = concurrency.try_acquire_owned().map_err(|_| {
        rejected(
          "WOML_PROCESS_CONCURRENCY_LIMIT",
          "The runtime process concurrency limit has been reached.",
        )
      })?;
      execute_process(authorized, policy, cancellation, permit, None).await
    })
  }

  fn execute_request_scoped(
    &self,
    request: &CapabilityCallRequest,
    workflow_scope: Option<String>,
    cancellation: CapabilityCancellationToken,
  ) -> BoxFuture<'static, Result<Value, CapabilityFailure>> {
    let authorized = match self.parse_and_authorize(&request.input) {
      Ok(authorized) => authorized,
      Err(error) => return Box::pin(async move { Err(error) }),
    };
    let stream = if authorized.request.stream {
      let Some(workflow_id) = workflow_scope else {
        return Box::pin(async move { Err(output_authority_failure(false)) });
      };
      let store = match self.output_store.lock() {
        Ok(configured) => configured.clone(),
        Err(_) => None,
      };
      let Some(store) = store else {
        return Box::pin(async move { Err(output_authority_failure(false)) });
      };
      Some(ProcessStreamTarget {
        store,
        identity: OperationalIdentityV1 {
          workflow_id,
          run_id: request.run_id.clone(),
          node_id: Some(request.node_id.clone()),
          attempt: Some(request.attempt_number),
          invocation_id: Some(request.invocation_id.clone()),
          phase: if request.invocation_id.starts_with("corr_") {
            crate::OperationalExecutionPhaseV1::Corrective
          } else {
            crate::OperationalExecutionPhaseV1::Forward
          },
        },
        resolved_secrets: Arc::clone(&self.resolved_secrets),
        artifact_handler: self
          .output_artifacts
          .lock()
          .ok()
          .and_then(|value| value.clone()),
      })
    } else {
      None
    };
    let policy = Arc::clone(&self.policy);
    let concurrency = Arc::clone(&self.concurrency);
    Box::pin(async move {
      let permit = concurrency.try_acquire_owned().map_err(|_| {
        rejected(
          "WOML_PROCESS_CONCURRENCY_LIMIT",
          "The runtime process concurrency limit has been reached.",
        )
      })?;
      execute_process(authorized, policy, cancellation, permit, stream).await
    })
  }
}

#[derive(Debug)]
struct AuthorizedProcess {
  request: ProcessRunRequest,
  executable: PathBuf,
  cwd: PathBuf,
}

#[derive(Debug)]
struct CapturedOutput {
  text: String,
  bytes: u64,
  truncated: bool,
}

#[derive(Debug, Default)]
struct ProcessStreamState {
  start_sequence: Option<u64>,
  end_sequence: Option<u64>,
  truncated: bool,
  retained_bytes: u64,
  artifact_writer: Option<crate::artifacts::OperationalArtifactWriter>,
  artifact: Option<crate::ArtifactReferenceV1>,
}

#[derive(Debug, Default)]
struct ProcessStreamFailure {
  failure: Mutex<Option<CapabilityFailure>>,
  notify: Notify,
}

impl ProcessStreamFailure {
  fn report(&self, failure: CapabilityFailure) {
    if let Ok(mut stored) = self.failure.lock() {
      if stored.is_none() {
        *stored = Some(failure);
        self.notify.notify_one();
      }
    }
  }

  fn take(&self) -> Option<CapabilityFailure> {
    self.failure.lock().ok()?.take()
  }
}

#[derive(Debug, Clone)]
struct ProcessStreamTarget {
  store: Arc<LazyOperationalOutputStore>,
  identity: OperationalIdentityV1,
  resolved_secrets: Arc<BTreeMap<String, String>>,
  artifact_handler: Option<Arc<crate::ManagedArtifactPutHandler>>,
}

#[derive(Debug, Clone)]
struct ActiveProcessStream {
  store: Arc<OperationalOutputStore>,
  identity: OperationalIdentityV1,
  state: Arc<Mutex<ProcessStreamState>>,
  failure: Arc<ProcessStreamFailure>,
  max_retained_bytes: u64,
}

async fn execute_process(
  authorized: AuthorizedProcess,
  policy: Arc<PreparedProcessPolicy>,
  cancellation: CapabilityCancellationToken,
  _permit: OwnedSemaphorePermit,
  stream_target: Option<ProcessStreamTarget>,
) -> Result<Value, CapabilityFailure> {
  let started = Instant::now();
  let request = authorized.request;
  // Process output is ephemeral, but it is still user-visible. Operator-
  // inherited values are sensitive at this boundary. The Bun worker separately
  // redacts resolved WOML secrets because secret provenance intentionally does
  // not cross the Process v1 contract.
  let sensitive_values = sensitive_environment_values(
    &policy,
    &request.env,
    stream_target
      .as_ref()
      .map(|target| target.resolved_secrets.as_ref()),
  );
  let redaction_lookahead = sensitive_values
    .iter()
    .map(String::len)
    .max()
    .unwrap_or(0)
    .saturating_sub(1);
  let stream = match stream_target {
    Some(target) => {
      let artifact_writer = target.artifact_handler.as_ref().and_then(|handler| {
        handler
          .begin_operational_output(
            &target.identity.workflow_id,
            &target.identity.run_id,
            target.identity.node_id.as_deref()?,
            target.identity.attempt?,
            target.identity.invocation_id.as_deref()?,
          )
          .ok()
          .flatten()
      });
      Some(ActiveProcessStream {
        store: target
          .store
          .activate()
          .map_err(|_| output_authority_failure(false))?,
        identity: target.identity,
        state: Arc::new(Mutex::new(ProcessStreamState {
          artifact_writer,
          ..ProcessStreamState::default()
        })),
        failure: Arc::new(ProcessStreamFailure::default()),
        max_retained_bytes: policy.limits.max_stream_bytes as u64,
      })
    }
    None => None,
  };
  let mut command = Command::new(&authorized.executable);
  prepare_process_environment(&mut command).map_err(|_| {
    failure(
      CapabilityFailureKind::ServiceRejected,
      "WOML_PROCESS_SPAWN_FAILED",
      "The approved executable could not be started.",
      true,
      false,
      None,
    )
  })?;
  command
    .args(&request.args)
    .current_dir(&authorized.cwd)
    .envs(&policy.inherited_environment)
    .envs(&request.env)
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(true);
  prepare_process_containment(&mut command);
  let mut child = command.spawn().map_err(|_| {
    failure(
      CapabilityFailureKind::ServiceRejected,
      "WOML_PROCESS_SPAWN_FAILED",
      "The approved executable could not be started.",
      true,
      false,
      None,
    )
  })?;
  let mut containment = match ProcessContainment::attach(&child) {
    Ok(containment) => containment,
    Err(_) => {
      let _ = child.kill().await;
      let _ = child.wait().await;
      return Err(failure(
        CapabilityFailureKind::HandlerCrashed,
        "WOML_PROCESS_CONTAINMENT_FAILED",
        "The process could not be attached to its operating-system containment boundary.",
        false,
        true,
        None,
      ));
    }
  };
  let stdout = child.stdout.take().ok_or_else(|| {
    failure(
      CapabilityFailureKind::HandlerCrashed,
      "WOML_PROCESS_PIPE_UNAVAILABLE",
      "The process stdout pipe could not be initialized.",
      false,
      true,
      None,
    )
  })?;
  let stderr = child.stderr.take().ok_or_else(|| {
    failure(
      CapabilityFailureKind::HandlerCrashed,
      "WOML_PROCESS_PIPE_UNAVAILABLE",
      "The process stderr pipe could not be initialized.",
      false,
      true,
      None,
    )
  })?;
  let stdout_task = tokio::spawn(drain_output(
    stdout,
    request.output.stdout,
    request.output.max_bytes,
    redaction_lookahead,
    sensitive_values.clone(),
    stream
      .clone()
      .map(|target| (target, OperationalSourceV1::Stdout)),
  ));
  let stderr_task = tokio::spawn(drain_output(
    stderr,
    request.output.stderr,
    request.output.max_bytes,
    redaction_lookahead,
    sensitive_values.clone(),
    stream
      .clone()
      .map(|target| (target, OperationalSourceV1::Stderr)),
  ));
  let timeout = tokio::time::sleep(Duration::from_millis(request.timeout_ms));
  tokio::pin!(timeout);
  let status = tokio::select! {
    biased;
    _ = cancellation.cancelled() => {
      let _ = terminate_process_tree(&mut child, &mut containment).await;
      let _ = collect_output_task(stdout_task).await;
      let _ = collect_output_task(stderr_task).await;
      close_process_stream(stream.as_ref(), OperationalCloseStatusV1::Cancelled, Some("WOML_PROCESS_CANCELLED"))?;
      return Err(failure(
        CapabilityFailureKind::Cancelled,
        "WOML_PROCESS_CANCELLED",
        "The process was cancelled.",
        false,
        true,
        Some(safe_duration(started)),
      ));
    }
    _ = &mut timeout => {
      let _ = terminate_process_tree(&mut child, &mut containment).await;
      let _ = collect_output_task(stdout_task).await;
      let _ = collect_output_task(stderr_task).await;
      close_process_stream(stream.as_ref(), OperationalCloseStatusV1::TimedOut, Some("WOML_PROCESS_TIMED_OUT"))?;
      return Err(failure(
        CapabilityFailureKind::TimedOut,
        "WOML_PROCESS_TIMED_OUT",
        "The process exceeded its approved timeout.",
        false,
        true,
        Some(safe_duration(started)),
      ));
    }
    failure = wait_for_process_stream_failure(stream.as_ref()), if stream.is_some() => {
      let _ = terminate_process_tree(&mut child, &mut containment).await;
      let _ = collect_output_task(stdout_task).await;
      let _ = collect_output_task(stderr_task).await;
      let _ = close_process_stream(
        stream.as_ref(),
        OperationalCloseStatusV1::Interrupted,
        Some("WOML_PROCESS_OUTPUT_AUTHORITY_FAILED"),
      );
      return Err(failure);
    }
    status = child.wait() => status.map_err(|_| failure(
      CapabilityFailureKind::HandlerCrashed,
      "WOML_PROCESS_WAIT_FAILED",
      "The runtime could not observe the process exit status.",
      false,
      true,
      Some(safe_duration(started)),
    ))?,
  };
  // Process v1 never permits detached work. If the leader exited after
  // starting descendants, close the containment boundary before waiting for
  // inherited stdout/stderr handles to reach EOF.
  containment.close_remaining_tree().await;
  let stdout = redact_and_bound_output(
    collect_output_task(stdout_task).await?,
    &sensitive_values,
    request.output.max_bytes,
  );
  let stderr = redact_and_bound_output(
    collect_output_task(stderr_task).await?,
    &sensitive_values,
    request.output.max_bytes,
  );
  let exit_code = status.code();
  let signal = exit_signal(&status);
  let duration_ms = started.elapsed().as_secs_f64() * 1_000.0;
  let stream_status = if exit_code.is_some_and(|code| request.accept_exit_codes.contains(&code)) {
    OperationalCloseStatusV1::Succeeded
  } else {
    OperationalCloseStatusV1::Failed
  };
  close_process_stream(
    stream.as_ref(),
    stream_status,
    if matches!(stream_status, OperationalCloseStatusV1::Succeeded) {
      None
    } else if exit_code.is_none() {
      Some("WOML_PROCESS_SIGNALLED")
    } else {
      Some("WOML_PROCESS_EXIT_CODE")
    },
  )?;
  let stream_state = stream.as_ref().and_then(|stream| {
    stream.state.lock().ok().map(|state| {
      (
        state.start_sequence,
        state.end_sequence,
        state.truncated,
        state.artifact.clone(),
      )
    })
  });
  let result = ProcessRunResultV1 {
    contract: PROCESS_CONTRACT,
    contract_version: PROCESS_CONTRACT_VERSION,
    kind: "result",
    exit_code,
    signal: signal.clone(),
    stdout: stdout.text,
    stderr: stderr.text,
    stdout_bytes: stdout.bytes,
    stderr_bytes: stderr.bytes,
    stdout_truncated: stdout.truncated,
    stderr_truncated: stderr.truncated,
    duration_ms,
  };
  if exit_code.is_none() || !request.accept_exit_codes.contains(&exit_code.unwrap_or(-1)) {
    let details = Map::from_iter([
      (
        "exitCode".to_string(),
        exit_code.map_or(Value::Null, Value::from),
      ),
      (
        "signal".to_string(),
        signal.map_or(Value::Null, Value::String),
      ),
      ("durationMs".to_string(), json!(duration_ms)),
      ("stdoutBytes".to_string(), json!(stdout.bytes)),
      ("stderrBytes".to_string(), json!(stderr.bytes)),
      ("stdoutTruncated".to_string(), json!(stdout.truncated)),
      ("stderrTruncated".to_string(), json!(stderr.truncated)),
    ]);
    let (code, message, ambiguous) = if exit_code.is_none() {
      (
        "WOML_PROCESS_SIGNALLED",
        "The process was terminated by a signal or platform-equivalent condition.",
        true,
      )
    } else {
      (
        "WOML_PROCESS_EXIT_CODE",
        "The process exited with a status not accepted by the workflow.",
        false,
      )
    };
    return Err(failure(
      CapabilityFailureKind::ServiceRejected,
      code,
      message,
      false,
      ambiguous,
      Some(details),
    ));
  }
  if request.contract_version == PROCESS_CONTRACT_VERSION_V2 {
    let (stream_start_sequence, stream_end_sequence, stream_truncated, stream_artifact) =
      stream_state.unwrap_or((None, None, false, None));
    serde_json::to_value(ProcessRunResultV2 {
      contract: PROCESS_CONTRACT,
      contract_version: PROCESS_CONTRACT_VERSION_V2,
      kind: "result",
      exit_code: result.exit_code,
      signal: result.signal,
      stdout: result.stdout,
      stderr: result.stderr,
      stdout_bytes: result.stdout_bytes,
      stderr_bytes: result.stderr_bytes,
      stdout_truncated: result.stdout_truncated,
      stderr_truncated: result.stderr_truncated,
      duration_ms: result.duration_ms,
      streamed: request.stream,
      stream_start_sequence,
      stream_end_sequence,
      stream_truncated,
      stream_artifact: stream_artifact
        .map(|artifact| serde_json::to_value(artifact).map_err(|_| output_failure()))
        .transpose()?,
    })
    .map_err(|_| output_failure())
  } else {
    serde_json::to_value(result).map_err(|_| output_failure())
  }
}

async fn collect_output_task(
  mut task: JoinHandle<Result<CapturedOutput, CapabilityFailure>>,
) -> Result<CapturedOutput, CapabilityFailure> {
  match tokio::time::timeout(PROCESS_PIPE_DRAIN_GRACE, &mut task).await {
    Ok(joined) => joined.map_err(|_| output_failure())?,
    Err(_) => {
      task.abort();
      let _ = task.await;
      Err(failure(
        CapabilityFailureKind::HandlerCrashed,
        "WOML_PROCESS_PIPE_DRAIN_TIMED_OUT",
        "The process output pipes did not close after the containment boundary terminated.",
        false,
        true,
        None,
      ))
    }
  }
}

async fn wait_for_process_stream_failure(
  stream: Option<&ActiveProcessStream>,
) -> CapabilityFailure {
  let Some(stream) = stream else {
    return std::future::pending().await;
  };
  loop {
    if let Some(failure) = stream.failure.take() {
      return failure;
    }
    stream.failure.notify.notified().await;
  }
}

async fn terminate_process_tree(
  child: &mut Child,
  containment: &mut ProcessContainment,
) -> io::Result<ExitStatus> {
  containment.graceful_terminate()?;
  tokio::time::sleep(PROCESS_TERMINATION_GRACE).await;
  containment.force_terminate()?;
  child.wait().await
}

fn prepare_process_environment(command: &mut Command) -> io::Result<()> {
  command.env_clear();
  #[cfg(windows)]
  {
    // Windows runtime initialization (including Node's crypto and Winsock)
    // requires SystemRoot. Derive this OS bootstrap value from the kernel,
    // not the ambient environment; application variables remain policy-only.
    command.env("SystemRoot", windows_system_root()?);
  }
  Ok(())
}

#[cfg(windows)]
fn windows_system_root() -> io::Result<std::ffi::OsString> {
  use std::os::windows::ffi::OsStringExt;
  let mut buffer = vec![0u16; 32_768];
  // SAFETY: buffer is writable for the supplied number of UTF-16 code units.
  let length = unsafe {
    windows_sys::Win32::System::SystemInformation::GetSystemWindowsDirectoryW(
      buffer.as_mut_ptr(),
      buffer.len() as u32,
    )
  } as usize;
  if length == 0 {
    return Err(io::Error::last_os_error());
  }
  if length >= buffer.len() {
    return Err(io::Error::new(
      io::ErrorKind::InvalidData,
      "the Windows system directory exceeds the supported path bound",
    ));
  }
  Ok(std::ffi::OsString::from_wide(&buffer[..length]))
}

#[cfg(unix)]
fn prepare_process_containment(command: &mut Command) {
  use std::os::unix::process::CommandExt;
  command.as_std_mut().process_group(0);
}

#[cfg(windows)]
fn prepare_process_containment(_command: &mut Command) {}

#[cfg(not(any(unix, windows)))]
fn prepare_process_containment(_command: &mut Command) {}

#[derive(Debug)]
struct ProcessContainment {
  #[cfg(unix)]
  process_group_id: i32,
  #[cfg(windows)]
  job: WindowsJob,
}

impl ProcessContainment {
  fn attach(child: &Child) -> io::Result<Self> {
    #[cfg(unix)]
    {
      let process_group_id = child
        .id()
        .and_then(|id| i32::try_from(id).ok())
        .ok_or_else(|| io::Error::other("spawned process has no valid process-group id"))?;
      Ok(Self { process_group_id })
    }
    #[cfg(windows)]
    {
      Ok(Self {
        job: WindowsJob::attach(child)?,
      })
    }
    #[cfg(not(any(unix, windows)))]
    {
      let _ = child;
      Ok(Self {})
    }
  }

  fn graceful_terminate(&mut self) -> io::Result<()> {
    #[cfg(unix)]
    {
      signal_process_group(self.process_group_id, libc::SIGTERM)
    }
    #[cfg(windows)]
    {
      // Windows has no portable graceful signal for an arbitrary non-console
      // process. Job termination is therefore the reviewed forced boundary.
      self.job.terminate()
    }
    #[cfg(not(any(unix, windows)))]
    {
      Ok(())
    }
  }

  fn force_terminate(&mut self) -> io::Result<()> {
    #[cfg(unix)]
    {
      signal_process_group(self.process_group_id, libc::SIGKILL)
    }
    #[cfg(windows)]
    {
      self.job.terminate()
    }
    #[cfg(not(any(unix, windows)))]
    {
      Ok(())
    }
  }

  async fn close_remaining_tree(&mut self) {
    #[cfg(unix)]
    {
      if process_group_exists(self.process_group_id) {
        let _ = self.graceful_terminate();
        tokio::time::sleep(PROCESS_TERMINATION_GRACE).await;
        let _ = self.force_terminate();
      }
    }
    #[cfg(windows)]
    {
      // Closing a kill-on-close Job Object is the final descendant boundary.
      let _ = self.job.terminate();
    }
  }
}

impl Drop for ProcessContainment {
  fn drop(&mut self) {
    #[cfg(unix)]
    {
      // A dropped handler future (for example, host shutdown) must not bypass
      // containment cleanup. SIGKILL is the only synchronous fail-closed
      // action available during Drop.
      let _ = signal_process_group(self.process_group_id, libc::SIGKILL);
    }
    // On Windows, dropping the WindowsJob field closes its kill-on-close job.
  }
}

#[cfg(unix)]
fn signal_process_group(process_group_id: i32, signal: i32) -> io::Result<()> {
  // SAFETY: kill is called with a positive, engine-owned process-group id and
  // a platform signal constant. A negative pid targets only that group.
  let result = unsafe { libc::kill(-process_group_id, signal) };
  if result == 0 {
    return Ok(());
  }
  let error = io::Error::last_os_error();
  if error.raw_os_error() == Some(libc::ESRCH) {
    Ok(())
  } else {
    Err(error)
  }
}

#[cfg(unix)]
fn process_group_exists(process_group_id: i32) -> bool {
  // SAFETY: signal 0 performs existence/permission checking without sending a
  // signal. The id was captured from the child created by this handler.
  let result = unsafe { libc::kill(-process_group_id, 0) };
  result == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(windows)]
#[derive(Debug)]
struct WindowsJob {
  handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl WindowsJob {
  fn attach(child: &Child) -> io::Result<Self> {
    use std::{mem::size_of, ptr};
    use windows_sys::Win32::{
      Foundation::CloseHandle,
      System::JobObjects::{
        AssignProcessToJobObject, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
      },
    };
    // SAFETY: every handle is checked, the info pointer and byte size match,
    // and ownership of the created job is retained by WindowsJob.
    unsafe {
      let child_handle = child.raw_handle().ok_or_else(|| {
        io::Error::new(
          io::ErrorKind::InvalidInput,
          "the child process exited before containment was attached",
        )
      })?;
      let handle =
        windows_sys::Win32::System::JobObjects::CreateJobObjectW(ptr::null(), ptr::null());
      if handle.is_null() {
        return Err(io::Error::last_os_error());
      }
      let mut information: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
      information.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
      if SetInformationJobObject(
        handle,
        JobObjectExtendedLimitInformation,
        &information as *const _ as *const _,
        size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
      ) == 0
        || AssignProcessToJobObject(handle, child_handle as _) == 0
      {
        let error = io::Error::last_os_error();
        CloseHandle(handle);
        return Err(error);
      }
      Ok(Self { handle })
    }
  }

  fn terminate(&self) -> io::Result<()> {
    // SAFETY: handle is a live Job Object owned by self.
    let result =
      unsafe { windows_sys::Win32::System::JobObjects::TerminateJobObject(self.handle, 1) };
    if result == 0 {
      Err(io::Error::last_os_error())
    } else {
      Ok(())
    }
  }
}

#[cfg(windows)]
// SAFETY: WindowsJob exclusively owns a kernel handle. Moving that owned
// handle between executor threads does not change its validity or ownership.
unsafe impl Send for WindowsJob {}

#[cfg(windows)]
impl Drop for WindowsJob {
  fn drop(&mut self) {
    // SAFETY: handle is owned by self and closed exactly once.
    unsafe {
      windows_sys::Win32::Foundation::CloseHandle(self.handle);
    }
  }
}

async fn drain_output<R: AsyncRead + Unpin>(
  mut reader: R,
  mode: OutputMode,
  max_bytes: usize,
  redaction_lookahead: usize,
  sensitive_values: Vec<String>,
  stream: Option<(ActiveProcessStream, OperationalSourceV1)>,
) -> Result<CapturedOutput, CapabilityFailure> {
  let capture_limit = max_bytes.saturating_add(redaction_lookahead);
  let mut captured = Vec::with_capacity(match mode {
    OutputMode::Capture => capture_limit.min(64 * 1024),
    OutputMode::Discard => 0,
  });
  let mut total = 0_u64;
  let mut buffer = [0_u8; 8192];
  let mut stream_decoder = stream
    .as_ref()
    .map(|_| IncrementalTextRedactor::new(&sensitive_values));
  loop {
    let read = reader
      .read(&mut buffer)
      .await
      .map_err(|_| output_failure())?;
    if read == 0 {
      break;
    }
    total = total.saturating_add(read as u64);
    if matches!(mode, OutputMode::Capture) && captured.len() < capture_limit {
      let remaining = capture_limit - captured.len();
      captured.extend_from_slice(&buffer[..read.min(remaining)]);
    }
    if let (Some((target, source)), Some(decoder)) = (&stream, &mut stream_decoder) {
      if let Some(text) = decoder.push(&buffer[..read], false) {
        if let Err(error) = append_process_text(target, *source, &text) {
          target.failure.report(error.clone());
          return Err(error);
        }
      }
    }
  }
  if let (Some((target, source)), Some(decoder)) = (&stream, &mut stream_decoder) {
    if let Some(text) = decoder.push(&[], true) {
      if let Err(error) = append_process_text(target, *source, &text) {
        target.failure.report(error.clone());
        return Err(error);
      }
    }
  }
  Ok(CapturedOutput {
    text: String::from_utf8_lossy(&captured).into_owned(),
    bytes: total,
    truncated: matches!(mode, OutputMode::Capture) && total > max_bytes as u64,
  })
}

fn sensitive_environment_values(
  policy: &PreparedProcessPolicy,
  request_environment: &BTreeMap<String, String>,
  resolved_secrets: Option<&BTreeMap<String, String>>,
) -> Vec<String> {
  let mut values = policy
    .inherited_environment
    .values()
    .filter(|value| !value.is_empty())
    .cloned()
    .collect::<Vec<_>>();
  values.extend(
    request_environment
      .values()
      .filter(|value| !value.is_empty())
      .cloned(),
  );
  if let Some(resolved_secrets) = resolved_secrets {
    values.extend(
      resolved_secrets
        .values()
        .filter(|value| !value.is_empty())
        .cloned(),
    );
  }
  values.sort_by_key(|value| std::cmp::Reverse(value.len()));
  values.dedup();
  values
}

#[derive(Debug)]
struct IncrementalTextRedactor {
  pending_bytes: Vec<u8>,
  pending_text: String,
  secrets: Vec<String>,
  lookahead: usize,
}

impl IncrementalTextRedactor {
  fn new(secrets: &[String]) -> Self {
    Self {
      pending_bytes: Vec::new(),
      pending_text: String::new(),
      secrets: secrets.to_vec(),
      lookahead: secrets
        .iter()
        .map(String::len)
        .max()
        .unwrap_or(1)
        .saturating_sub(1),
    }
  }

  fn push(&mut self, bytes: &[u8], eof: bool) -> Option<String> {
    self.pending_bytes.extend_from_slice(bytes);
    loop {
      match std::str::from_utf8(&self.pending_bytes) {
        Ok(value) => {
          self.pending_text.push_str(value);
          self.pending_bytes.clear();
          break;
        }
        Err(error) => {
          let valid = error.valid_up_to();
          if valid > 0 {
            let prefix = String::from_utf8_lossy(&self.pending_bytes[..valid]);
            self.pending_text.push_str(&prefix);
            self.pending_bytes.drain(..valid);
          }
          match error.error_len() {
            Some(length) => {
              self.pending_text.push('\u{fffd}');
              self
                .pending_bytes
                .drain(..length.min(self.pending_bytes.len()));
            }
            None => break,
          }
        }
      }
    }
    if eof && !self.pending_bytes.is_empty() {
      self
        .pending_text
        .push_str(&String::from_utf8_lossy(&self.pending_bytes));
      self.pending_bytes.clear();
    }
    for secret in &self.secrets {
      self.pending_text = self.pending_text.replace(secret, "[redacted]");
    }
    let emit_bytes = if eof {
      self.pending_text.len()
    } else {
      self.pending_text.len().saturating_sub(self.lookahead)
    };
    let mut boundary = emit_bytes;
    while boundary > 0 && !self.pending_text.is_char_boundary(boundary) {
      boundary -= 1;
    }
    if boundary == 0 {
      return None;
    }
    let emitted = self.pending_text[..boundary].to_string();
    self.pending_text.drain(..boundary);
    (!emitted.is_empty()).then_some(emitted)
  }
}

fn append_process_text(
  target: &ActiveProcessStream,
  source: OperationalSourceV1,
  text: &str,
) -> Result<(), CapabilityFailure> {
  let mut remaining = text;
  while !remaining.is_empty() {
    let mut end = remaining.len().min(crate::MAX_OPERATIONAL_CHUNK_BYTES);
    while end > 0 && !remaining.is_char_boundary(end) {
      end -= 1;
    }
    if end == 0 {
      return Err(output_authority_failure(true));
    }
    append_process_output(target, source, remaining[..end].to_string())?;
    remaining = &remaining[end..];
  }
  Ok(())
}

fn append_process_output(
  target: &ActiveProcessStream,
  source: OperationalSourceV1,
  text: String,
) -> Result<(), CapabilityFailure> {
  let byte_count = text.len() as u64;
  let mut state = target
    .state
    .lock()
    .map_err(|_| output_authority_failure(true))?;
  if let Some(writer) = state.artifact_writer.as_mut() {
    let source_name = match source {
      OperationalSourceV1::Stdout => "stdout",
      OperationalSourceV1::Stderr => "stderr",
      OperationalSourceV1::Script => "script",
      OperationalSourceV1::Runtime => "runtime",
    };
    let _ = writer.append_redacted(source_name, &text);
  }
  if state.truncated {
    return Ok(());
  }
  if state.retained_bytes.saturating_add(byte_count) > target.max_retained_bytes {
    let record = target
      .store
      .append(OperationalRecordDraftV1 {
        identity: target.identity.clone(),
        body: OperationalBodyV1::Truncated {
          source,
          dropped_bytes: byte_count,
          reason: "inline_run_budget".to_string(),
        },
      })
      .map_err(|_| output_authority_failure(true))?;
    state.start_sequence.get_or_insert(record.sequence);
    state.end_sequence = Some(record.sequence);
    state.truncated = true;
    return Ok(());
  }
  let record = target
    .store
    .append(OperationalRecordDraftV1 {
      identity: target.identity.clone(),
      body: OperationalBodyV1::Output {
        source,
        level: None,
        text,
        byte_count,
      },
    })
    .map_err(|_| output_authority_failure(true))?;
  state.retained_bytes = state.retained_bytes.saturating_add(byte_count);
  state.start_sequence.get_or_insert(record.sequence);
  state.end_sequence = Some(record.sequence);
  Ok(())
}

fn close_process_stream(
  stream: Option<&ActiveProcessStream>,
  status: OperationalCloseStatusV1,
  reason_code: Option<&str>,
) -> Result<(), CapabilityFailure> {
  let Some(stream) = stream else {
    return Ok(());
  };
  let (writer, publish) = {
    let mut state = stream
      .state
      .lock()
      .map_err(|_| output_authority_failure(true))?;
    (state.artifact_writer.take(), state.truncated)
  };
  if let Some(writer) = writer {
    if let Ok(Some(artifact)) = writer.finish(publish) {
      let record = stream
        .store
        .append(OperationalRecordDraftV1 {
          identity: stream.identity.clone(),
          body: OperationalBodyV1::Artifact {
            artifact_id: artifact.id.clone(),
            name: artifact.name.clone(),
            size: artifact.size,
            sha256: artifact.sha256.clone(),
            content_type: artifact.content_type.clone(),
          },
        })
        .map_err(|_| output_authority_failure(true))?;
      let mut state = stream
        .state
        .lock()
        .map_err(|_| output_authority_failure(true))?;
      state.start_sequence.get_or_insert(record.sequence);
      state.end_sequence = Some(record.sequence);
      state.artifact = Some(artifact);
    }
  }
  let record = stream
    .store
    .append(OperationalRecordDraftV1 {
      identity: stream.identity.clone(),
      body: OperationalBodyV1::Closed {
        status,
        reason_code: reason_code.map(str::to_string),
      },
    })
    .map_err(|_| output_authority_failure(true))?;
  let mut state = stream
    .state
    .lock()
    .map_err(|_| output_authority_failure(true))?;
  state.start_sequence.get_or_insert(record.sequence);
  state.end_sequence = Some(record.sequence);
  Ok(())
}

fn output_authority_failure(ambiguous: bool) -> CapabilityFailure {
  failure(
    CapabilityFailureKind::TransportFailed,
    "WOML_PROCESS_OUTPUT_AUTHORITY_FAILED",
    "The operational-output authority could not accept streamed process output.",
    false,
    ambiguous,
    None,
  )
}

fn redact_and_bound_output(
  mut output: CapturedOutput,
  sensitive_values: &[String],
  max_bytes: usize,
) -> CapturedOutput {
  for value in sensitive_values {
    output.text = output.text.replace(value, "[redacted]");
  }
  if output.text.len() > max_bytes {
    let mut end = max_bytes;
    while !output.text.is_char_boundary(end) {
      end = end.saturating_sub(1);
    }
    output.text.truncate(end);
    output.truncated = true;
  }
  output
}

fn parse_request(input: &Value) -> Result<ProcessRunRequest, CapabilityFailure> {
  let request: ProcessRunRequest = serde_json::from_value(input.clone()).map_err(|_| {
    invalid_input(
      "Process input does not match the frozen Process v1 or Process v2 request contract.",
    )
  })?;
  let has_stream = input
    .as_object()
    .is_some_and(|object| object.contains_key("stream"));
  let version_shape_valid = match request.contract_version {
    PROCESS_CONTRACT_VERSION => !has_stream,
    PROCESS_CONTRACT_VERSION_V2 => has_stream,
    _ => false,
  };
  if request.contract != PROCESS_CONTRACT
    || !version_shape_valid
    || request.kind != "request"
    || request.args.len() > 256
    || request.env.len() > 128
    || request.accept_exit_codes.is_empty()
    || request.accept_exit_codes.len() > 32
    || request.timeout_ms == 0
    || request.timeout_ms > 86_400_000
    || request.output.max_bytes == 0
    || request.output.max_bytes > 262_144
  {
    return Err(invalid_input(
      "Process input does not match the frozen Process v1 or Process v2 request contract.",
    ));
  }
  validate_alias_failure(&request.executable)?;
  validate_relative_cwd(&request.cwd)?;
  if request
    .args
    .iter()
    .any(|value| value.contains('\0') || value.len() > 32_768)
    || request.env.iter().any(|(name, value)| {
      validate_environment_name(name).is_err() || value.contains('\0') || value.len() > 32_768
    })
    || request
      .accept_exit_codes
      .iter()
      .any(|code| !(0..=255).contains(code))
  {
    return Err(invalid_input(
      "Process input contains an invalid argument, environment value, or exit code.",
    ));
  }
  let unique: BTreeSet<_> = request.accept_exit_codes.iter().collect();
  if unique.len() != request.accept_exit_codes.len() {
    return Err(invalid_input("Process accepted exit codes must be unique."));
  }
  Ok(request)
}

fn authorize_cwd(
  policy: &PreparedProcessPolicy,
  requested: &str,
) -> Result<PathBuf, CapabilityFailure> {
  validate_relative_cwd(requested)?;
  let candidate = policy.project_root.join(requested);
  let canonical = std::fs::canonicalize(&candidate).map_err(|_| {
    rejected(
      "WOML_PROCESS_CWD_NOT_FOUND",
      "The requested process working directory does not exist.",
    )
  })?;
  if !canonical.is_dir()
    || !policy
      .working_directories
      .iter()
      .any(|root| canonical.starts_with(root))
  {
    return Err(rejected(
      "WOML_PROCESS_CWD_DENIED",
      "The requested process working directory is not allowed by runtime policy.",
    ));
  }
  Ok(canonical)
}

fn validate_relative_cwd(value: &str) -> Result<(), CapabilityFailure> {
  if value.is_empty() || value.len() > 4096 || value.contains('\0') {
    return Err(invalid_input(
      "Process cwd must be a non-empty relative path.",
    ));
  }
  let path = Path::new(value);
  if path.is_absolute()
    || value.starts_with(['/', '\\'])
    || (value.len() >= 3
      && value.as_bytes()[0].is_ascii_alphabetic()
      && value.as_bytes()[1] == b':'
      && matches!(value.as_bytes()[2], b'/' | b'\\'))
    || value
      .split(['/', '\\'])
      .any(|component| component == ".." || invalid_portable_path_component(component))
    || path.components().any(|component| {
      matches!(
        component,
        Component::ParentDir | Component::RootDir | Component::Prefix(_)
      )
    })
  {
    return Err(invalid_input(
      "Process cwd must be relative and must not contain parent traversal.",
    ));
  }
  Ok(())
}

fn invalid_portable_path_component(component: &str) -> bool {
  if component.is_empty() || component == "." {
    return false;
  }
  if component.contains(':') || component.ends_with(' ') || component.ends_with('.') {
    return true;
  }
  let stem = component
    .split('.')
    .next()
    .unwrap_or(component)
    .to_ascii_uppercase();
  matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
    || stem
      .strip_prefix("COM")
      .or_else(|| stem.strip_prefix("LPT"))
      .is_some_and(|suffix| suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9'))
}

fn validate_alias(alias: &str) -> Result<(), ProcessPolicyError> {
  validate_alias_text(alias).map_err(ProcessPolicyError::Invalid)
}

fn validate_alias_failure(alias: &str) -> Result<(), CapabilityFailure> {
  validate_alias_text(alias).map_err(|_| invalid_input("Process executable alias is invalid."))
}

fn validate_alias_text(alias: &str) -> Result<(), String> {
  if alias.is_empty()
    || alias.len() > 128
    || !alias
      .bytes()
      .enumerate()
      .all(|(index, byte)| byte.is_ascii_alphanumeric() || (index > 0 && b"._-".contains(&byte)))
  {
    return Err("an executable alias is invalid".to_string());
  }
  Ok(())
}

fn validate_environment_name(name: &str) -> Result<(), String> {
  if name.is_empty()
    || name.len() > 128
    || !name.bytes().enumerate().all(|(index, byte)| {
      byte == b'_' || byte.is_ascii_alphabetic() || (index > 0 && byte.is_ascii_digit())
    })
  {
    return Err("an environment-variable name is invalid".to_string());
  }
  Ok(())
}

fn validate_limits(limits: &ProcessPolicyLimits) -> Result<(), ProcessPolicyError> {
  if limits.max_timeout_ms == 0
    || limits.max_timeout_ms > 86_400_000
    || limits.max_output_bytes == 0
    || limits.max_output_bytes > 262_144
    || limits.max_stream_bytes == 0
    || limits.max_stream_bytes > MAX_PROCESS_STREAM_BYTES
    || limits.max_concurrent == 0
    || limits.max_concurrent > 64
  {
    return Err(ProcessPolicyError::Invalid(
      "timeout, captured-output, streamed-output, or concurrency limits are outside Process Policy v1 bounds"
        .to_string(),
    ));
  }
  Ok(())
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf, ProcessPolicyError> {
  let canonical = std::fs::canonicalize(path)
    .map_err(|_| ProcessPolicyError::Invalid(format!("the {label} does not exist")))?;
  if !canonical.is_dir() {
    return Err(ProcessPolicyError::Invalid(format!(
      "the {label} is not a directory"
    )));
  }
  Ok(canonical)
}

fn verify_executable_policy(
  path: &Path,
  alias: &str,
) -> Result<ExecutableIdentity, ProcessPolicyError> {
  let metadata = std::fs::metadata(path).map_err(|_| {
    ProcessPolicyError::Invalid(format!("executable alias '{alias}' is unavailable"))
  })?;
  if !metadata.is_file() || !platform_executable(&metadata) {
    return Err(ProcessPolicyError::Invalid(format!(
      "executable alias '{alias}' does not point to an executable file"
    )));
  }
  executable_identity(path, &metadata).map_err(|_| {
    ProcessPolicyError::Invalid(format!(
      "executable alias '{alias}' identity could not be inspected"
    ))
  })
}

fn verify_executable(executable: &ApprovedExecutable) -> Result<(), CapabilityFailure> {
  let canonical = std::fs::canonicalize(&executable.path).map_err(|_| {
    rejected(
      "WOML_PROCESS_EXECUTABLE_NOT_FOUND",
      "The approved executable is no longer available.",
    )
  })?;
  let metadata = std::fs::metadata(&canonical).map_err(|_| {
    rejected(
      "WOML_PROCESS_EXECUTABLE_NOT_FOUND",
      "The approved executable is no longer available.",
    )
  })?;
  let identity = executable_identity(&canonical, &metadata).map_err(|_| {
    rejected(
      "WOML_PROCESS_EXECUTABLE_CHANGED",
      "The approved executable identity can no longer be verified.",
    )
  })?;
  if canonical != executable.path
    || !metadata.is_file()
    || !platform_executable(&metadata)
    || identity != executable.identity
  {
    return Err(rejected(
      "WOML_PROCESS_EXECUTABLE_CHANGED",
      "The approved executable changed after runtime preflight.",
    ));
  }
  Ok(())
}

fn executable_identity(
  path: &Path,
  metadata: &std::fs::Metadata,
) -> io::Result<ExecutableIdentity> {
  #[cfg(unix)]
  use std::os::unix::fs::MetadataExt;
  #[cfg(unix)]
  let _ = path;
  #[cfg(windows)]
  let (volume_serial_number, file_index) = windows_file_identity(path)?;
  Ok(ExecutableIdentity {
    length: metadata.len(),
    modified: metadata.modified().ok(),
    #[cfg(unix)]
    device: metadata.dev(),
    #[cfg(unix)]
    inode: metadata.ino(),
    #[cfg(windows)]
    volume_serial_number: Some(volume_serial_number),
    #[cfg(windows)]
    file_index: Some(file_index),
  })
}

#[cfg(windows)]
fn windows_file_identity(path: &Path) -> io::Result<(u32, u64)> {
  use std::os::windows::io::AsRawHandle;
  use windows_sys::Win32::Storage::FileSystem::{
    GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
  };

  let file = std::fs::File::open(path)?;
  let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
  // SAFETY: file owns a valid readable handle for the duration of the call and
  // information points to the correctly sized output structure.
  if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut information) } == 0 {
    return Err(io::Error::last_os_error());
  }
  let file_index = ((information.nFileIndexHigh as u64) << 32) | information.nFileIndexLow as u64;
  Ok((information.dwVolumeSerialNumber, file_index))
}

#[cfg(unix)]
fn platform_executable(metadata: &std::fs::Metadata) -> bool {
  use std::os::unix::fs::PermissionsExt;
  metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn platform_executable(_metadata: &std::fs::Metadata) -> bool {
  true
}

#[cfg(unix)]
fn exit_signal(status: &std::process::ExitStatus) -> Option<String> {
  use std::os::unix::process::ExitStatusExt;
  status.signal().map(|signal| signal.to_string())
}

#[cfg(not(unix))]
fn exit_signal(_status: &std::process::ExitStatus) -> Option<String> {
  None
}

fn safe_duration(started: Instant) -> Map<String, Value> {
  Map::from_iter([(
    "durationMs".to_string(),
    json!(started.elapsed().as_secs_f64() * 1_000.0),
  )])
}

fn output_mode_name(mode: OutputMode) -> &'static str {
  match mode {
    OutputMode::Capture => "capture",
    OutputMode::Discard => "discard",
  }
}

fn invalid_input(message: &str) -> CapabilityFailure {
  failure(
    CapabilityFailureKind::InvalidInput,
    "WOML_PROCESS_INPUT_INVALID",
    message,
    false,
    false,
    None,
  )
}

fn rejected(code: &str, message: &str) -> CapabilityFailure {
  failure(
    CapabilityFailureKind::ServiceRejected,
    code,
    message,
    false,
    false,
    None,
  )
}

fn output_failure() -> CapabilityFailure {
  failure(
    CapabilityFailureKind::HandlerCrashed,
    "WOML_PROCESS_OUTPUT_FAILED",
    "The runtime could not safely capture process output.",
    false,
    true,
    None,
  )
}

fn failure(
  kind: CapabilityFailureKind,
  code: &str,
  message: &str,
  retryable: bool,
  ambiguous: bool,
  details: Option<Map<String, Value>>,
) -> CapabilityFailure {
  CapabilityFailure {
    kind,
    code: code.to_string(),
    message: format!("{message} Corrective action: {}", corrective_action(code)),
    retryable,
    ambiguous,
    details,
  }
}

fn corrective_action(code: &str) -> &'static str {
  match code {
    "WOML_PROCESS_EXECUTABLE_DENIED" => {
      "add the intended executable alias to the operator-owned process policy"
    }
    "WOML_PROCESS_EXECUTABLE_NOT_FOUND" | "WOML_PROCESS_EXECUTABLE_CHANGED" => {
      "restore the reviewed executable and restart the WOML runtime so it can preflight it again"
    }
    "WOML_PROCESS_CWD_NOT_FOUND" | "WOML_PROCESS_CWD_DENIED" => {
      "use an existing project-relative directory allowed by the process policy"
    }
    "WOML_PROCESS_ENV_DENIED" => {
      "remove the variable or allow its name in the operator-owned process policy"
    }
    "WOML_PROCESS_OUTPUT_LIMIT_DENIED" => {
      "lower output.maxBytes or raise the policy ceiling after review"
    }
    "WOML_PROCESS_CONCURRENCY_LIMIT" => {
      "wait for another managed process to finish or raise the reviewed concurrency limit"
    }
    "WOML_PROCESS_SPAWN_FAILED" => {
      "verify executable permissions and host resources, then start a new run"
    }
    "WOML_PROCESS_TIMED_OUT" => {
      "optimize the command or increase its timeout within the policy ceiling"
    }
    "WOML_PROCESS_CANCELLED" => {
      "inspect possible partial external effects before starting a new run"
    }
    "WOML_PROCESS_EXIT_CODE" => {
      "inspect the tool result or explicitly accept that exit code when it represents success"
    }
    "WOML_PROCESS_SIGNALLED" => {
      "inspect host resource pressure and possible partial effects before retrying"
    }
    "WOML_PROCESS_INPUT_INVALID" => "correct the Process v1 request fields and bounds",
    _ => "inspect the runtime diagnostics and start a new run only when the cause is understood",
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn typescript_and_rust_share_the_frozen_process_v1_payloads() {
    let request: Value = serde_json::from_str(include_str!(
      "../../../woml/tests/fixtures/services-contracts/process-request.v1.json"
    ))
    .expect("standalone Process v1 request fixture");
    let capability_request: Value = serde_json::from_str(include_str!(
      "../../../woml/tests/fixtures/services-contracts/capability-process-request.v1.json"
    ))
    .expect("Capability Call v1 Process request fixture");
    assert_eq!(capability_request["input"], request);

    let parsed = parse_request(&request).expect("Rust accepts the TypeScript request fixture");
    assert_eq!(parsed.contract, PROCESS_CONTRACT);
    assert_eq!(parsed.contract_version, PROCESS_CONTRACT_VERSION);
    assert_eq!(parsed.kind, "request");
    assert_eq!(parsed.executable, "bun");
    assert_eq!(parsed.args, ["test", "--timeout", "5000"]);
    assert_eq!(parsed.cwd, ".");
    assert_eq!(parsed.env.get("CI").map(String::as_str), Some("true"));
    assert_eq!(parsed.timeout_ms, 300_000);
    assert_eq!(parsed.accept_exit_codes, [0]);

    let result: Value = serde_json::from_str(include_str!(
      "../../../woml/tests/fixtures/services-contracts/process-result.v1.json"
    ))
    .expect("standalone Process v1 result fixture");
    let capability_result: Value = serde_json::from_str(include_str!(
      "../../../woml/tests/fixtures/services-contracts/capability-process-succeeded.v1.json"
    ))
    .expect("Capability Call v1 Process result fixture");
    assert_eq!(capability_result["result"], result);
  }

  fn policy() -> ProcessPolicyV1 {
    ProcessPolicyV1 {
      contract: PROCESS_POLICY_CONTRACT.to_string(),
      contract_version: PROCESS_POLICY_CONTRACT_VERSION,
      enabled: true,
      modes: ProcessPolicyModes {
        foreground: true,
        background: false,
      },
      project_root: std::env::current_dir()
        .expect("current directory")
        .to_string_lossy()
        .into_owned(),
      executables: vec![ProcessPolicyExecutable {
        alias: "fixture".to_string(),
        path: std::env::current_exe()
          .expect("current test executable")
          .to_string_lossy()
          .into_owned(),
      }],
      working_directories: vec![".".to_string()],
      environment: ProcessPolicyEnvironment {
        inherit: Vec::new(),
        allow: vec![
          "WOML_PROCESS_FIXTURE".to_string(),
          "WOML_PROCESS_FIXTURE_TEXT".to_string(),
          "WOML_PROCESS_FIXTURE_EXIT".to_string(),
          "WOML_PROCESS_FIXTURE_TREE".to_string(),
          "WOML_PROCESS_FIXTURE_HEARTBEAT".to_string(),
          "WOML_PROCESS_FIXTURE_CWD".to_string(),
          "WOML_PROCESS_FIXTURE_ENV_NAME".to_string(),
          "WOML_PROCESS_FIXTURE_PRINT_ENV".to_string(),
          "PATH".to_string(),
        ],
      },
      limits: ProcessPolicyLimits {
        max_timeout_ms: 5_000,
        max_output_bytes: 1_024,
        max_stream_bytes: crate::DEFAULT_OPERATIONAL_INLINE_RUN_BYTES as usize,
        max_concurrent: 1,
      },
    }
  }

  fn request(environment: Value, max_bytes: usize, accepted: Vec<i32>) -> Value {
    json!({
      "contract": PROCESS_CONTRACT,
      "contractVersion": PROCESS_CONTRACT_VERSION,
      "kind": "request",
      "executable": "fixture",
      "args": ["--exact", "process::tests::fixture_child", "--nocapture"],
      "cwd": ".",
      "env": environment,
      "timeoutMs": 5_000,
      "acceptExitCodes": accepted,
      "output": { "stdout": "capture", "stderr": "capture", "maxBytes": max_bytes }
    })
  }

  fn with_cwd(mut request: Value, cwd: &str) -> Value {
    request["cwd"] = json!(cwd);
    request
  }

  fn temporary_directory(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
      "woml-process-{label}-{}",
      uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir(&path).expect("create temporary directory");
    path
  }

  #[test]
  fn fixture_child() {
    if std::env::var("WOML_PROCESS_FIXTURE").as_deref() != Ok("woml-process-fixture-active") {
      return;
    }
    if let Ok(role) = std::env::var("WOML_PROCESS_FIXTURE_TREE") {
      let heartbeat =
        std::env::var("WOML_PROCESS_FIXTURE_HEARTBEAT").expect("tree fixture heartbeat path");
      if role == "parent" {
        std::process::Command::new(std::env::current_exe().expect("fixture executable"))
          .args(["--exact", "process::tests::fixture_child", "--nocapture"])
          .env_clear()
          .env("WOML_PROCESS_FIXTURE", "woml-process-fixture-active")
          .env("WOML_PROCESS_FIXTURE_TREE", "grandchild")
          .env("WOML_PROCESS_FIXTURE_HEARTBEAT", &heartbeat)
          .stdin(Stdio::null())
          .stdout(Stdio::null())
          .stderr(Stdio::null())
          .spawn()
          .expect("spawn grandchild fixture");
      }
      let marker = if role == "parent" { b'P' } else { b'G' };
      loop {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
          .create(true)
          .append(true)
          .open(&heartbeat)
          .expect("open heartbeat");
        file.write_all(&[marker]).expect("write heartbeat");
        file.sync_all().expect("sync heartbeat");
        std::thread::sleep(Duration::from_millis(10));
      }
    }
    if std::env::var("WOML_PROCESS_FIXTURE_CWD").as_deref() == Ok("report-current-directory") {
      print!(
        "{}",
        std::env::current_dir()
          .expect("fixture cwd")
          .to_string_lossy()
      );
      return;
    }
    if let Ok(name) = std::env::var("WOML_PROCESS_FIXTURE_ENV_NAME") {
      print!(
        "{}",
        if std::env::var_os(name).is_some() {
          "present"
        } else {
          "absent"
        }
      );
      return;
    }
    if let Ok(name) = std::env::var("WOML_PROCESS_FIXTURE_PRINT_ENV") {
      print!("{}", std::env::var(name).unwrap_or_default());
      return;
    }
    let text = std::env::var("WOML_PROCESS_FIXTURE_TEXT").unwrap_or_default();
    print!("{text}");
    if let Ok(code) = std::env::var("WOML_PROCESS_FIXTURE_EXIT") {
      std::process::exit(code.parse().expect("fixture exit code"));
    }
  }

  #[tokio::test]
  async fn approved_process_returns_bounded_output() {
    let handler =
      ManagedProcessHandler::new(policy(), ProcessRuntimeMode::Foreground).expect("valid policy");
    let value = handler
      .execute(
        request(
          json!({
            "WOML_PROCESS_FIXTURE": "woml-process-fixture-active",
            "WOML_PROCESS_FIXTURE_TEXT": "abcdefghijklmnopqrstuvwxyz"
          }),
          10,
          vec![0],
        ),
        CapabilityCancellationToken::default(),
      )
      .await
      .expect("approved process succeeds");
    assert_eq!(value["contract"], PROCESS_CONTRACT);
    assert_eq!(value["exitCode"], 0);
    assert_eq!(value["stdoutTruncated"], true);
    assert!(value["stdoutBytes"].as_u64().expect("byte count") > 10);
    assert!(value["stdout"].as_str().expect("stdout").len() <= 10);
  }

  #[tokio::test]
  async fn process_v2_streams_redacted_output_and_preserves_bounded_capture() {
    let directory = temporary_directory("stream-v2");
    let output_path = directory.join("output.sqlite");
    let secrets = Arc::new(BTreeMap::from([(
      "token".to_string(),
      "stream-secret-42".to_string(),
    )]));
    let handler =
      ManagedProcessHandler::new_with_secrets(policy(), ProcessRuntimeMode::Foreground, secrets)
        .expect("valid Process policy");
    handler
      .configure_output_store(Arc::new(LazyOperationalOutputStore::new(
        &output_path,
        "deployment-process-v2",
      )))
      .expect("configure output store");
    let mut input = request(
      json!({
        "WOML_PROCESS_FIXTURE": "woml-process-fixture-active",
        "WOML_PROCESS_FIXTURE_TEXT": "before-stream-secret-42-after"
      }),
      128,
      vec![0],
    );
    input["contractVersion"] = json!(PROCESS_CONTRACT_VERSION_V2);
    input["stream"] = json!(true);
    let call = CapabilityCallRequest {
      contract: crate::CAPABILITY_CALL_CONTRACT.to_string(),
      contract_version: crate::CAPABILITY_CALL_CONTRACT_VERSION,
      message_type: "request".to_string(),
      invocation_id: "inv-process-v2".to_string(),
      call_id: "call-process-v2".to_string(),
      run_id: "run-process-v2".to_string(),
      node_id: "runCommand".to_string(),
      attempt_number: 1,
      capability: "process".to_string(),
      operation: "run".to_string(),
      input_contract_version: 1,
      result_contract_version: 1,
      identity: crate::CapabilityCallIdentity {
        mode: crate::capability::CapabilityIdentityMode::Named,
        step_idempotency_key: format!("sha256:{}", "1".repeat(64)),
        operation_name: "process.run.fixture".to_string(),
        operation_key: format!("sha256:{}", "2".repeat(64)),
        provider_idempotency_key: None,
      },
      limits: crate::CapabilityCallLimits::default(),
      input,
    };
    let result = handler
      .execute_request_scoped(
        &call,
        Some("process-stream-workflow".to_string()),
        CapabilityCancellationToken::default(),
      )
      .await
      .expect("Process v2 succeeds");
    assert_eq!(result["contractVersion"], PROCESS_CONTRACT_VERSION_V2);
    assert_eq!(result["streamed"], true);
    assert!(result["stdout"].as_str().unwrap().contains("[redacted]"));
    assert!(!result["stdout"]
      .as_str()
      .unwrap()
      .contains("stream-secret-42"));

    let store = OperationalOutputStore::open(&output_path, "deployment-process-v2")
      .expect("reopen output store");
    let records = store
      .read_run_page("run-process-v2", 0, 100)
      .expect("stream replay")
      .records;
    let encoded = serde_json::to_string(&records).expect("record JSON");
    assert!(encoded.contains("[redacted]"));
    assert!(!encoded.contains("stream-secret-42"));
    assert!(matches!(
      records.last().map(|record| &record.body),
      Some(OperationalBodyV1::Closed {
        status: OperationalCloseStatusV1::Succeeded,
        ..
      })
    ));
    let _ = std::fs::remove_dir_all(directory);
  }

  #[tokio::test]
  async fn inherited_environment_values_are_redacted_from_captured_output() {
    let mut configured = policy();
    configured.environment.inherit = vec!["PATH".to_string()];
    let handler = ManagedProcessHandler::new(configured, ProcessRuntimeMode::Foreground)
      .expect("valid inherited environment policy");
    let value = handler
      .execute(
        request(
          json!({
            "WOML_PROCESS_FIXTURE": "woml-process-fixture-active",
            "WOML_PROCESS_FIXTURE_PRINT_ENV": "PATH"
          }),
          1_024,
          vec![0],
        ),
        CapabilityCancellationToken::default(),
      )
      .await
      .expect("approved process succeeds");
    assert!(value["stdout"]
      .as_str()
      .expect("stdout")
      .contains("[redacted]"));
    assert!(!value
      .to_string()
      .contains(&std::env::var("PATH").expect("host PATH")));
  }

  #[tokio::test]
  async fn ambient_environment_is_cleared_unless_explicitly_inherited() {
    let handler =
      ManagedProcessHandler::new(policy(), ProcessRuntimeMode::Foreground).expect("valid policy");
    let value = handler
      .execute(
        request(
          json!({
            "WOML_PROCESS_FIXTURE": "woml-process-fixture-active",
            "WOML_PROCESS_FIXTURE_ENV_NAME": "HOME"
          }),
          128,
          vec![0],
        ),
        CapabilityCancellationToken::default(),
      )
      .await
      .expect("approved process succeeds");
    assert!(value["stdout"].as_str().expect("stdout").contains("absent"));
  }

  #[tokio::test]
  async fn policy_inherits_only_names_the_operator_approved() {
    let mut configured = policy();
    configured.environment.inherit = vec!["PATH".to_string()];
    let handler = ManagedProcessHandler::new(configured, ProcessRuntimeMode::Foreground)
      .expect("valid inherited environment policy");
    let value = handler
      .execute(
        request(
          json!({
            "WOML_PROCESS_FIXTURE": "woml-process-fixture-active",
            "WOML_PROCESS_FIXTURE_ENV_NAME": "PATH"
          }),
          128,
          vec![0],
        ),
        CapabilityCancellationToken::default(),
      )
      .await
      .expect("approved inherited variable is available");
    assert!(value["stdout"]
      .as_str()
      .expect("stdout")
      .contains("present"));
  }

  #[tokio::test]
  async fn path_shadowing_cannot_change_the_preflighted_executable() {
    let handler =
      ManagedProcessHandler::new(policy(), ProcessRuntimeMode::Foreground).expect("valid policy");
    let value = handler
      .execute(
        request(
          json!({
            "WOML_PROCESS_FIXTURE": "woml-process-fixture-active",
            "WOML_PROCESS_FIXTURE_TEXT": "approved-binary-ran",
            "PATH": "/definitely/not/the/operator/path"
          }),
          128,
          vec![0],
        ),
        CapabilityCancellationToken::default(),
      )
      .await
      .expect("canonical executable is used directly");
    assert_eq!(value["exitCode"], 0);
  }

  #[tokio::test]
  async fn cwd_is_resolved_from_the_policy_project_root() {
    let root = temporary_directory("project-root");
    let mut configured = policy();
    configured.project_root = root.to_string_lossy().into_owned();
    configured.working_directories = vec![".".to_string()];
    let handler = ManagedProcessHandler::new(configured, ProcessRuntimeMode::Foreground)
      .expect("valid project policy");
    let value = handler
      .execute(
        request(
          json!({
            "WOML_PROCESS_FIXTURE": "woml-process-fixture-active",
            "WOML_PROCESS_FIXTURE_CWD": "report-current-directory"
          }),
          512,
          vec![0],
        ),
        CapabilityCancellationToken::default(),
      )
      .await
      .expect("approved process succeeds");
    let stdout = value["stdout"].as_str().expect("stdout");
    let canonical_root = root.canonicalize().expect("canonical policy project root");
    #[cfg(windows)]
    {
      let reported = stdout
        .replace(r"\\?\", "")
        .replace('\\', "/")
        .to_ascii_lowercase();
      let expected = canonical_root
        .to_string_lossy()
        .replace(r"\\?\", "")
        .replace('\\', "/")
        .to_ascii_lowercase();
      assert!(reported.contains(&expected));
    }
    #[cfg(not(windows))]
    assert!(stdout.contains(canonical_root.to_string_lossy().as_ref()));
    let _ = std::fs::remove_dir(root);
  }

  #[cfg(windows)]
  #[tokio::test]
  async fn windows_child_receives_system_root_without_inheriting_the_application_environment() {
    let handler =
      ManagedProcessHandler::new(policy(), ProcessRuntimeMode::Foreground).expect("valid policy");
    let value = handler
      .execute(
        request(
          json!({
            "WOML_PROCESS_FIXTURE": "woml-process-fixture-active",
            "WOML_PROCESS_FIXTURE_PRINT_ENV": "SystemRoot"
          }),
          1_024,
          vec![0],
        ),
        CapabilityCancellationToken::default(),
      )
      .await
      .expect("child starts with the Windows bootstrap environment");
    assert!(value["stdout"].as_str().expect("stdout").contains(
      windows_system_root()
        .expect("Windows system directory")
        .to_string_lossy()
        .as_ref()
    ));
    let value = handler
      .execute(
        request(
          json!({
            "WOML_PROCESS_FIXTURE": "woml-process-fixture-active",
            "WOML_PROCESS_FIXTURE_ENV_NAME": "PATH"
          }),
          128,
          vec![0],
        ),
        CapabilityCancellationToken::default(),
      )
      .await
      .expect("child starts without an ambient application PATH");
    assert!(value["stdout"].as_str().expect("stdout").contains("absent"));
  }

  #[cfg(windows)]
  #[tokio::test]
  #[ignore = "requires WOML_PROCESS_TEST_NODE; run explicitly by Windows CI"]
  async fn windows_node_executes_javascript_with_only_the_os_bootstrap_environment() {
    let mut configured = policy();
    configured.limits.max_timeout_ms = 10_000;
    configured.executables = vec![ProcessPolicyExecutable {
      alias: "node".to_string(),
      path: std::env::var("WOML_PROCESS_TEST_NODE").expect("CI-provided Node executable"),
    }];
    let handler = ManagedProcessHandler::new(configured, ProcessRuntimeMode::Foreground)
      .expect("valid Node process policy");
    let arguments = ["space value", "café 雪", "\"quoted\"", "; echo not-a-shell"];
    let script = r#"
      const assert = require('node:assert/strict');
      assert.ok(process.env.SystemRoot);
      assert.equal(process.env.PATH, undefined);
      assert.equal(process.env.HOME, undefined);
      assert.equal(require('node:crypto').randomBytes(16).length, 16);
      process.stdout.write(JSON.stringify(process.argv.slice(1)));
    "#;
    let mut args = vec!["-e", script];
    args.extend(arguments);
    let mut input = request(json!({}), 1_024, vec![0]);
    input["executable"] = json!("node");
    input["args"] = json!(args);
    input["timeoutMs"] = json!(10_000);
    let value = handler
      .execute(input, CapabilityCancellationToken::default())
      .await
      .expect("Node executes JavaScript and crypto under Rust process containment");
    assert_eq!(value["exitCode"], 0);
    assert_eq!(value["stdout"], serde_json::to_string(&arguments).unwrap());
    assert_eq!(value["stderr"], "");
  }

  #[cfg(unix)]
  #[test]
  fn symlink_cannot_escape_an_allowed_working_directory() {
    use std::os::unix::fs::symlink;
    let root = temporary_directory("symlink-root");
    let outside = temporary_directory("symlink-outside");
    symlink(&outside, root.join("escape")).expect("create symlink");
    let mut configured = policy();
    configured.project_root = root.to_string_lossy().into_owned();
    configured.working_directories = vec![".".to_string()];
    let handler = ManagedProcessHandler::new(configured, ProcessRuntimeMode::Foreground)
      .expect("valid project policy");
    let error = handler
      .parse_and_authorize(&with_cwd(request(json!({}), 128, vec![0]), "escape"))
      .expect_err("symlink escape denied");
    assert_eq!(error.code, "WOML_PROCESS_CWD_DENIED");
    let _ = std::fs::remove_file(root.join("escape"));
    let _ = std::fs::remove_dir(root);
    let _ = std::fs::remove_dir(outside);
  }

  #[cfg(unix)]
  #[test]
  fn executable_replacement_is_detected_after_preflight() {
    let root = temporary_directory("executable-identity");
    let executable = root.join("fixture");
    std::fs::copy(
      std::env::current_exe().expect("current executable"),
      &executable,
    )
    .expect("copy executable");
    let mut configured = policy();
    configured.executables[0].path = executable.to_string_lossy().into_owned();
    let handler = ManagedProcessHandler::new(configured, ProcessRuntimeMode::Foreground)
      .expect("valid executable policy");
    let replacement = root.join("replacement");
    std::fs::copy(
      std::env::current_exe().expect("current executable"),
      &replacement,
    )
    .expect("copy replacement");
    std::fs::rename(&replacement, &executable).expect("replace executable");
    let error = handler
      .parse_and_authorize(&request(json!({}), 128, vec![0]))
      .expect_err("replacement denied");
    assert_eq!(error.code, "WOML_PROCESS_EXECUTABLE_CHANGED");
    let _ = std::fs::remove_file(executable);
    let _ = std::fs::remove_dir(root);
  }

  #[test]
  fn validation_rejects_device_paths_and_resource_exhaustion() {
    for cwd in ["NUL", "con.txt", "COM1", "LPT9.log", "folder. "] {
      let error = parse_request(&with_cwd(request(json!({}), 128, vec![0]), cwd))
        .expect_err("portable device path denied");
      assert_eq!(error.code, "WOML_PROCESS_INPUT_INVALID");
    }
    let too_many_args = (0..257).map(|_| "x").collect::<Vec<_>>();
    let mut value = request(json!({}), 128, vec![0]);
    value["args"] = json!(too_many_args);
    assert_eq!(
      parse_request(&value)
        .expect_err("argument count bounded")
        .code,
      "WOML_PROCESS_INPUT_INVALID"
    );
    value = request(json!({}), 128, vec![0]);
    value["args"] = json!(["x".repeat(32_769)]);
    assert_eq!(
      parse_request(&value)
        .expect_err("argument size bounded")
        .code,
      "WOML_PROCESS_INPUT_INVALID"
    );
    let environment = (0..129)
      .map(|index| (format!("SAFE_{index}"), "x"))
      .collect::<BTreeMap<_, _>>();
    assert_eq!(
      parse_request(&request(json!(environment), 128, vec![0]))
        .expect_err("environment count bounded")
        .code,
      "WOML_PROCESS_INPUT_INVALID"
    );
    assert_eq!(
      parse_request(&request(
        json!({ "SAFE": "x".repeat(32_769) }),
        128,
        vec![0]
      ))
      .expect_err("environment value bounded")
      .code,
      "WOML_PROCESS_INPUT_INVALID"
    );
  }

  #[test]
  fn generated_process_requests_are_validated_without_panics() {
    let alphabet = [b'a', b'Z', b'0', b'.', b'/', b'\\', b':', b' ', 0, 0xff];
    let mut state = 0x5eed_u64;
    for length in 0..256 {
      let mut bytes = Vec::with_capacity(length);
      for _ in 0..length {
        state = state
          .wrapping_mul(6_364_136_223_846_793_005)
          .wrapping_add(1);
        bytes.push(alphabet[(state as usize) % alphabet.len()]);
      }
      let candidate = String::from_utf8_lossy(&bytes).into_owned();
      let value = with_cwd(request(json!({}), 128, vec![0]), &candidate);
      let _ = parse_request(&value);
    }
  }

  #[test]
  fn every_process_failure_has_an_author_facing_corrective_action() {
    for code in [
      "WOML_PROCESS_EXECUTABLE_DENIED",
      "WOML_PROCESS_EXECUTABLE_NOT_FOUND",
      "WOML_PROCESS_EXECUTABLE_CHANGED",
      "WOML_PROCESS_CWD_DENIED",
      "WOML_PROCESS_ENV_DENIED",
      "WOML_PROCESS_SPAWN_FAILED",
      "WOML_PROCESS_TIMED_OUT",
      "WOML_PROCESS_CANCELLED",
      "WOML_PROCESS_EXIT_CODE",
      "WOML_PROCESS_INPUT_INVALID",
    ] {
      let error = failure(
        CapabilityFailureKind::ServiceRejected,
        code,
        "Safe process failure.",
        false,
        false,
        None,
      );
      assert!(error.message.contains("Corrective action:"), "{code}");
      assert!(!error.message.contains('/') && !error.message.contains('\\'));
    }
  }

  #[tokio::test]
  async fn nonaccepted_exit_code_has_only_safe_failure_metadata() {
    let handler =
      ManagedProcessHandler::new(policy(), ProcessRuntimeMode::Foreground).expect("valid policy");
    let error = handler
      .execute(
        request(
          json!({
            "WOML_PROCESS_FIXTURE": "woml-process-fixture-active",
            "WOML_PROCESS_FIXTURE_TEXT": "sensitive output",
            "WOML_PROCESS_FIXTURE_EXIT": "7"
          }),
          128,
          vec![0],
        ),
        CapabilityCancellationToken::default(),
      )
      .await
      .expect_err("exit code is rejected");
    assert_eq!(error.code, "WOML_PROCESS_EXIT_CODE");
    let details = error.details.expect("safe details");
    assert_eq!(details.get("exitCode"), Some(&json!(7)));
    assert!(!details.contains_key("stdout"));
    assert!(!details.contains_key("stderr"));
  }

  #[tokio::test]
  async fn request_environment_is_enforced_again_in_rust() {
    let handler =
      ManagedProcessHandler::new(policy(), ProcessRuntimeMode::Foreground).expect("valid policy");
    let error = handler
      .execute(
        request(json!({ "UNAPPROVED_SECRET": "no" }), 128, vec![0]),
        CapabilityCancellationToken::default(),
      )
      .await
      .expect_err("environment is denied");
    assert_eq!(error.code, "WOML_PROCESS_ENV_DENIED");
  }

  #[test]
  fn runtime_policy_caps_the_effective_process_timeout() {
    let handler =
      ManagedProcessHandler::new(policy(), ProcessRuntimeMode::Foreground).expect("valid policy");
    let mut input = request(
      json!({ "WOML_PROCESS_FIXTURE": "woml-process-fixture-active" }),
      128,
      vec![0],
    );
    input["timeoutMs"] = json!(10_000);
    let authorized = handler
      .parse_and_authorize(&input)
      .expect("timeout is bounded by policy");
    assert_eq!(authorized.request.timeout_ms, 5_000);
  }

  #[test]
  fn process_versions_keep_v1_exact_and_accept_the_explicit_v2_shape() {
    let v1 = request(
      json!({ "WOML_PROCESS_FIXTURE": "woml-process-fixture-active" }),
      128,
      vec![0],
    );
    assert_eq!(
      parse_request(&v1)
        .expect("exact Process v1")
        .contract_version,
      PROCESS_CONTRACT_VERSION
    );

    let mut invalid_v1 = v1.clone();
    invalid_v1["stream"] = json!(false);
    assert!(parse_request(&invalid_v1).is_err());

    let mut v2_disabled = v1.clone();
    v2_disabled["contractVersion"] = json!(PROCESS_CONTRACT_VERSION_V2);
    v2_disabled["stream"] = json!(false);
    let parsed = parse_request(&v2_disabled).expect("explicit Process v2 without streaming");
    assert_eq!(parsed.contract_version, PROCESS_CONTRACT_VERSION_V2);
    assert!(!parsed.stream);

    let mut v2_streaming = v2_disabled;
    v2_streaming["stream"] = json!(true);
    assert!(
      parse_request(&v2_streaming)
        .expect("explicit Process v2 streaming")
        .stream
    );
  }

  #[test]
  fn incremental_stream_redaction_covers_every_secret_split_and_invalid_utf8() {
    let secret = "token-秘密-42";
    let expected = "before-[redacted]-after";
    let source = format!("before-{secret}-after");
    for split in 0..=source.len() {
      let mut redactor = IncrementalTextRedactor::new(&[secret.to_string()]);
      let mut output = String::new();
      if let Some(value) = redactor.push(&source.as_bytes()[..split], false) {
        output.push_str(&value);
      }
      if let Some(value) = redactor.push(&source.as_bytes()[split..], true) {
        output.push_str(&value);
      }
      assert_eq!(output, expected, "secret split at byte {split}");
      assert!(!output.contains(secret));
    }

    let multibyte = "ready 🚀 完了";
    let mut redactor = IncrementalTextRedactor::new(&[]);
    let mut output = String::new();
    for byte in multibyte.as_bytes() {
      if let Some(value) = redactor.push(&[*byte], false) {
        output.push_str(&value);
      }
    }
    if let Some(value) = redactor.push(&[], true) {
      output.push_str(&value);
    }
    assert_eq!(output, multibyte);

    let mut invalid = IncrementalTextRedactor::new(&[]);
    let repaired = invalid
      .push(&[b'a', 0xff, b'b'], true)
      .expect("invalid bytes produce safe text");
    assert_eq!(repaired, "a\u{fffd}b");
  }

  #[test]
  fn durable_metadata_excludes_arguments_environment_and_output() {
    let handler =
      ManagedProcessHandler::new(policy(), ProcessRuntimeMode::Foreground).expect("valid policy");
    let input = request(
      json!({
        "WOML_PROCESS_FIXTURE": "woml-process-fixture-active",
        "WOML_PROCESS_FIXTURE_TEXT": "never-persist-this-secret"
      }),
      128,
      vec![0],
    );
    let metadata = handler.safe_metadata(&input);
    let encoded = serde_json::to_string(&metadata).expect("metadata JSON");
    assert_eq!(metadata.get("executable"), Some(&json!("fixture")));
    assert_eq!(metadata.get("argumentCount"), Some(&json!(3)));
    assert!(!encoded.contains("never-persist-this-secret"));
    assert!(!encoded.contains("--exact"));
    assert!(!metadata.contains_key("args"));
    assert!(!metadata.contains_key("env"));
    assert!(!metadata.contains_key("stdout"));
    assert!(!metadata.contains_key("stderr"));
  }

  async fn assert_heartbeat_stops(path: &Path) {
    tokio::time::sleep(Duration::from_millis(150)).await;
    let first = std::fs::metadata(path).expect("heartbeat exists").len();
    tokio::time::sleep(Duration::from_millis(150)).await;
    let second = std::fs::metadata(path).expect("heartbeat remains").len();
    assert_eq!(first, second, "a descendant survived containment shutdown");
  }

  fn tree_request(path: &Path, timeout_ms: u64) -> Value {
    let mut value = request(
      json!({
        "WOML_PROCESS_FIXTURE": "woml-process-fixture-active",
        "WOML_PROCESS_FIXTURE_TREE": "parent",
        "WOML_PROCESS_FIXTURE_HEARTBEAT": path.to_string_lossy()
      }),
      128,
      vec![0],
    );
    value["timeoutMs"] = json!(timeout_ms);
    value
  }

  #[cfg(any(unix, windows))]
  #[tokio::test]
  async fn timeout_terminates_child_and_grandchild_tree() {
    let heartbeat = std::env::temp_dir().join(format!(
      "woml-process-timeout-heartbeat-{}",
      uuid::Uuid::new_v4()
    ));
    let handler =
      ManagedProcessHandler::new(policy(), ProcessRuntimeMode::Foreground).expect("valid policy");
    let error = handler
      .execute(
        tree_request(&heartbeat, 250),
        CapabilityCancellationToken::default(),
      )
      .await
      .expect_err("tree times out");
    assert_eq!(error.kind, CapabilityFailureKind::TimedOut);
    assert_eq!(error.code, "WOML_PROCESS_TIMED_OUT");
    assert!(error.ambiguous);
    assert_heartbeat_stops(&heartbeat).await;
    let _ = std::fs::remove_file(heartbeat);
  }

  #[cfg(any(unix, windows))]
  #[tokio::test]
  async fn cancellation_terminates_tree_and_cannot_report_success() {
    let heartbeat = std::env::temp_dir().join(format!(
      "woml-process-cancel-heartbeat-{}",
      uuid::Uuid::new_v4()
    ));
    let handler =
      ManagedProcessHandler::new(policy(), ProcessRuntimeMode::Foreground).expect("valid policy");
    let cancellation = CapabilityCancellationToken::default();
    let cancel = cancellation.clone();
    let request = tree_request(&heartbeat, 5_000);
    let execution = tokio::spawn(async move { handler.execute(request, cancellation).await });
    tokio::time::sleep(Duration::from_millis(150)).await;
    cancel.cancel();
    let error = execution
      .await
      .expect("execution task")
      .expect_err("tree is cancelled");
    assert_eq!(error.kind, CapabilityFailureKind::Cancelled);
    assert_eq!(error.code, "WOML_PROCESS_CANCELLED");
    assert!(error.ambiguous);
    assert_heartbeat_stops(&heartbeat).await;
    let _ = std::fs::remove_file(heartbeat);
  }

  #[cfg(any(unix, windows))]
  #[tokio::test]
  async fn concurrency_ceiling_rejects_excess_processes_without_queueing() {
    let heartbeat = std::env::temp_dir().join(format!(
      "woml-process-concurrency-heartbeat-{}",
      uuid::Uuid::new_v4()
    ));
    let handler =
      ManagedProcessHandler::new(policy(), ProcessRuntimeMode::Foreground).expect("valid policy");
    let cancellation = CapabilityCancellationToken::default();
    let running_handler = handler.clone();
    let running_cancellation = cancellation.clone();
    let running_request = tree_request(&heartbeat, 5_000);
    let running = tokio::spawn(async move {
      running_handler
        .execute(running_request, running_cancellation)
        .await
    });
    for _ in 0..100 {
      if heartbeat.exists() {
        break;
      }
      tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(heartbeat.exists(), "first process did not start");
    let error = handler
      .execute(
        request(
          json!({ "WOML_PROCESS_FIXTURE": "woml-process-fixture-active" }),
          128,
          vec![0],
        ),
        CapabilityCancellationToken::default(),
      )
      .await
      .expect_err("second process exceeds the policy ceiling");
    assert_eq!(error.code, "WOML_PROCESS_CONCURRENCY_LIMIT");
    cancellation.cancel();
    let _ = running.await;
    assert_heartbeat_stops(&heartbeat).await;
    let _ = std::fs::remove_file(heartbeat);
  }

  #[cfg(any(unix, windows))]
  #[tokio::test]
  async fn dropped_handler_future_closes_the_process_tree() {
    let heartbeat = std::env::temp_dir().join(format!(
      "woml-process-host-stop-heartbeat-{}",
      uuid::Uuid::new_v4()
    ));
    let handler =
      ManagedProcessHandler::new(policy(), ProcessRuntimeMode::Foreground).expect("valid policy");
    let request = tree_request(&heartbeat, 5_000);
    let execution = tokio::spawn(async move {
      handler
        .execute(request, CapabilityCancellationToken::default())
        .await
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    execution.abort();
    let _ = execution.await;
    assert_heartbeat_stops(&heartbeat).await;
    let _ = std::fs::remove_file(heartbeat);
  }
}
