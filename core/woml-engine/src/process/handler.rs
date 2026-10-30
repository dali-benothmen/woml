//! Process capability handler, request authorization, shared concurrency, and output-authority binding. No shell command-string execution.

use crate::process::authorize_cwd;
use crate::process::execute_process;
use crate::process::output_authority_failure;
use crate::process::output_mode_name;
use crate::process::parse_request;
use crate::process::rejected;
use crate::process::verify_executable;
use crate::process::PreparedProcessPolicy;
use crate::process::ProcessPolicyError;
use crate::process::ProcessPolicyV1;
use crate::process::ProcessRunRequest;
use crate::process::ProcessRuntimeMode;
use crate::process::ProcessStreamTarget;
use crate::process::PROCESS_CONTRACT_VERSION;
use crate::CapabilityCallRequest;
use crate::CapabilityCancellationToken;
use crate::CapabilityDescriptor;
use crate::CapabilityEffect;
use crate::CapabilityFailure;
use crate::CapabilityFailureKind;
use crate::CapabilityHandler;
use crate::LazyOperationalOutputStore;
use crate::OperationalIdentityV1;
use futures_util::future::BoxFuture;
use serde_json::json;
use serde_json::Map;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;
use tokio::sync::Semaphore;

#[derive(Debug, Clone)]
pub struct ManagedProcessHandler {
  pub(in crate::process) policy: Arc<PreparedProcessPolicy>,
  pub(in crate::process) concurrency: Arc<Semaphore>,
  pub(in crate::process) output_store: Arc<Mutex<Option<Arc<LazyOperationalOutputStore>>>>,
  pub(in crate::process) output_artifacts:
    Arc<Mutex<Option<Arc<crate::ManagedArtifactPutHandler>>>>,
  pub(in crate::process) resolved_secrets: Arc<BTreeMap<String, String>>,
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

  pub(in crate::process) fn parse_and_authorize(
    &self,
    input: &Value,
  ) -> Result<AuthorizedProcess, CapabilityFailure> {
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
pub(in crate::process) struct AuthorizedProcess {
  pub(in crate::process) request: ProcessRunRequest,
  pub(in crate::process) executable: PathBuf,
  pub(in crate::process) cwd: PathBuf,
}

pub(in crate::process) fn safe_duration(started: Instant) -> Map<String, Value> {
  Map::from_iter([(
    "durationMs".to_string(),
    json!(started.elapsed().as_secs_f64() * 1_000.0),
  )])
}
