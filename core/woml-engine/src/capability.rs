//! Provider-neutral capability execution contracts.
//!
//! This module owns generic operation validation, dispatch, limits, cancellation,
//! and safe failures. Capability-specific policy belongs in registered handlers,
//! never in the workflow DAG traversal.

use std::{
  collections::HashMap,
  panic::AssertUnwindSafe,
  sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
  },
  time::Instant,
};

use chrono::Utc;
use futures_util::{future::BoxFuture, FutureExt};
use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::{
  sync::{Mutex as AsyncMutex, Notify},
  time,
};
use uuid::Uuid;

use crate::{
  DurableEventStore, DurableStoreError, OperationExecutionMode, OperationFailedData,
  OperationStartedData, OperationSucceededData, RunEventPayload,
};

pub const CAPABILITY_CALL_CONTRACT: &str = "woml.capability-call";
pub const CAPABILITY_CALL_CONTRACT_VERSION: u32 = 1;
pub const DEFAULT_CAPABILITY_INPUT_BYTES: u64 = 1_048_576;
pub const DEFAULT_CAPABILITY_RESULT_BYTES: u64 = 4_194_304;
pub const DEFAULT_CAPABILITY_FRAME_BYTES: u64 = 8_388_608;
pub const DEFAULT_CAPABILITY_TIMEOUT_MS: u64 = 30_000;
pub const MAX_CAPABILITY_TIMEOUT_MS: u64 = 86_400_000;
pub const DEFAULT_CAPABILITY_IN_FLIGHT_PER_INVOCATION: usize = 32;
pub const DEFAULT_CAPABILITY_IN_FLIGHT_GLOBAL: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityIdentityMode {
  Automatic,
  Named,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityCallIdentity {
  pub mode: CapabilityIdentityMode,
  pub step_idempotency_key: String,
  pub operation_name: String,
  pub operation_key: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub provider_idempotency_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityCallLimits {
  pub input_bytes: u64,
  pub result_bytes: u64,
  pub timeout_ms: u64,
}

impl Default for CapabilityCallLimits {
  fn default() -> Self {
    Self {
      input_bytes: DEFAULT_CAPABILITY_INPUT_BYTES,
      result_bytes: DEFAULT_CAPABILITY_RESULT_BYTES,
      timeout_ms: DEFAULT_CAPABILITY_TIMEOUT_MS,
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityCallRequest {
  pub contract: String,
  pub contract_version: u32,
  pub message_type: String,
  pub invocation_id: String,
  pub call_id: String,
  pub run_id: String,
  pub node_id: String,
  pub attempt_number: u32,
  pub capability: String,
  pub operation: String,
  pub input_contract_version: u32,
  pub result_contract_version: u32,
  pub identity: CapabilityCallIdentity,
  pub limits: CapabilityCallLimits,
  pub input: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityFailureKind {
  InvalidInput,
  InvalidResult,
  UnsupportedCapability,
  UnsupportedOperation,
  UnsupportedVersion,
  InputTooLarge,
  ResultTooLarge,
  FrameTooLarge,
  TimedOut,
  Cancelled,
  HandlerCrashed,
  WorkerCrashed,
  HostCrashed,
  TransportFailed,
  ServiceRejected,
  Interrupted,
  Ambiguous,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityFailure {
  pub kind: CapabilityFailureKind,
  pub code: String,
  pub message: String,
  pub retryable: bool,
  pub ambiguous: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub details: Option<Map<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityCallSucceeded {
  pub contract: String,
  pub contract_version: u32,
  pub message_type: String,
  pub invocation_id: String,
  pub call_id: String,
  pub outcome: String,
  pub result_contract_version: u32,
  pub result_bytes: u64,
  pub duration_ms: f64,
  pub result: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityCallFailed {
  pub contract: String,
  pub contract_version: u32,
  pub message_type: String,
  pub invocation_id: String,
  pub call_id: String,
  pub outcome: String,
  pub duration_ms: f64,
  pub error: CapabilityFailure,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum CapabilityCallResult {
  Succeeded(CapabilityCallSucceeded),
  Failed(CapabilityCallFailed),
  Cancelled(CapabilityCallFailed),
}

impl<'de> Deserialize<'de> for CapabilityCallResult {
  fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
  where
    D: Deserializer<'de>,
  {
    let value = Value::deserialize(deserializer)?;
    match value.get("outcome").and_then(Value::as_str) {
      Some("succeeded") => serde_json::from_value(value)
        .map(Self::Succeeded)
        .map_err(D::Error::custom),
      Some("failed") => serde_json::from_value(value)
        .map(Self::Failed)
        .map_err(D::Error::custom),
      Some("cancelled") => serde_json::from_value(value)
        .map(Self::Cancelled)
        .map_err(D::Error::custom),
      _ => Err(D::Error::custom(
        "Capability Call v1 result has an invalid outcome.",
      )),
    }
  }
}

impl CapabilityCallResult {
  pub fn invocation_id(&self) -> &str {
    match self {
      Self::Succeeded(result) => &result.invocation_id,
      Self::Failed(result) | Self::Cancelled(result) => &result.invocation_id,
    }
  }

  pub fn call_id(&self) -> &str {
    match self {
      Self::Succeeded(result) => &result.call_id,
      Self::Failed(result) | Self::Cancelled(result) => &result.call_id,
    }
  }

  pub fn validate(&self) -> Result<(), String> {
    match self {
      Self::Succeeded(result) => {
        validate_result_base(
          &result.contract,
          result.contract_version,
          &result.message_type,
          &result.invocation_id,
          &result.call_id,
          result.duration_ms,
        )?;
        if result.outcome != "succeeded"
          || result.result_contract_version == 0
          || result.result_bytes > DEFAULT_CAPABILITY_FRAME_BYTES
        {
          return Err("Capability success does not match Capability Call v1.".to_string());
        }
      }
      Self::Failed(result) | Self::Cancelled(result) => {
        validate_result_base(
          &result.contract,
          result.contract_version,
          &result.message_type,
          &result.invocation_id,
          &result.call_id,
          result.duration_ms,
        )?;
        let expected = if matches!(self, Self::Cancelled(_)) {
          "cancelled"
        } else {
          "failed"
        };
        if result.outcome != expected {
          return Err("Capability failure outcome does not match its result variant.".to_string());
        }
        result.error.validate()?;
      }
    }
    Ok(())
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityEffect {
  Read,
  IdempotentWrite,
  UnsafeWrite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityDescriptor {
  pub capability: String,
  pub operation: String,
  pub input_contract_version: u32,
  pub result_contract_version: u32,
  pub effect: CapabilityEffect,
  pub supports_cancellation: bool,
  pub supports_provider_idempotency: bool,
}

#[derive(Debug, Clone, Default)]
pub struct CapabilityCancellationToken {
  state: Arc<CancellationState>,
}

#[derive(Debug, Default)]
struct CancellationState {
  cancelled: AtomicBool,
  notify: Notify,
}

impl CapabilityCancellationToken {
  pub fn cancel(&self) {
    if !self.state.cancelled.swap(true, Ordering::SeqCst) {
      self.state.notify.notify_waiters();
    }
  }

  pub fn is_cancelled(&self) -> bool {
    self.state.cancelled.load(Ordering::SeqCst)
  }

  pub async fn cancelled(&self) {
    while !self.is_cancelled() {
      self.state.notify.notified().await;
    }
  }
}

pub trait CapabilityHandler: Send + Sync + 'static {
  fn descriptor(&self) -> CapabilityDescriptor;

  fn safe_metadata(&self, _input: &Value) -> Map<String, Value> {
    Map::new()
  }

  fn execute(
    &self,
    input: Value,
    cancellation: CapabilityCancellationToken,
  ) -> BoxFuture<'static, Result<Value, CapabilityFailure>>;
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CapabilityRegistryError {
  #[error("capability handler {capability}.{operation} is already registered")]
  DuplicateHandler {
    capability: String,
    operation: String,
  },
}

pub struct CapabilityRegistry {
  handlers: Mutex<HashMap<(String, String), Arc<dyn CapabilityHandler>>>,
  in_flight_by_invocation: Mutex<HashMap<String, usize>>,
  in_flight_global: AtomicUsize,
  max_per_invocation: usize,
  max_global: usize,
}

#[derive(Debug, Error)]
pub enum DurableCapabilityAuthorityError {
  #[error(transparent)]
  Store(#[from] DurableStoreError),
}

/// Rust's durable authority for managed capability operations. It records a
/// start before dispatch and a terminal event before exposing the result.
pub struct DurableCapabilityAuthority {
  registry: Arc<CapabilityRegistry>,
  store: Arc<AsyncMutex<DurableEventStore>>,
}

impl DurableCapabilityAuthority {
  pub fn new(registry: Arc<CapabilityRegistry>, store: Arc<AsyncMutex<DurableEventStore>>) -> Self {
    Self { registry, store }
  }

  pub fn registry(&self) -> &Arc<CapabilityRegistry> {
    &self.registry
  }

  pub fn store(&self) -> &Arc<AsyncMutex<DurableEventStore>> {
    &self.store
  }

  pub async fn execute(
    &self,
    request: CapabilityCallRequest,
    cancellation: CapabilityCancellationToken,
  ) -> Result<CapabilityCallResult, DurableCapabilityAuthorityError> {
    {
      let store = self.store.lock().await;
      let projection = store.projection(&request.run_id)?;
      let attempt = projection.attempts.iter().find(|attempt| {
        attempt.identity.node_id == request.node_id
          && attempt.identity.attempt == request.attempt_number
          && attempt.identity.invocation_id == request.invocation_id
          && attempt.status == crate::projection::AttemptStatus::Started
      });
      if attempt.and_then(|attempt| attempt.idempotency_key.as_deref())
        != Some(request.identity.step_idempotency_key.as_str())
      {
        return Err(
          DurableStoreError::Contract(
            "Capability call step identity does not match its active durable attempt.".to_string(),
          )
          .into(),
        );
      }
    }
    let metadata = match self.registry.safe_metadata(&request) {
      Ok(metadata) => metadata,
      Err(error) => {
        return Ok(CapabilityCallResult::Failed(failed_fields(
          &request,
          Instant::now(),
          "failed",
          error,
        )))
      }
    };
    {
      let mut store = self.store.lock().await;
      store.append_payload(
        request.run_id.clone(),
        generated_event_id(),
        Utc::now(),
        RunEventPayload::OperationStarted(OperationStartedData {
          node_id: request.node_id.clone(),
          attempt_number: request.attempt_number,
          invocation_id: request.invocation_id.clone(),
          call_id: request.call_id.clone(),
          operation_key: request.identity.operation_key.clone(),
          capability: request.capability.clone(),
          operation: request.operation.clone(),
          execution_mode: OperationExecutionMode::Managed,
          metadata: metadata.clone(),
        }),
      )?;
    }

    let result = self.registry.execute(request.clone(), cancellation).await;
    let payload = match &result {
      CapabilityCallResult::Succeeded(success) => {
        let encoded = serde_json::to_vec(&success.result).unwrap_or_default();
        RunEventPayload::OperationSucceeded(OperationSucceededData {
          node_id: request.node_id.clone(),
          attempt_number: request.attempt_number,
          invocation_id: request.invocation_id.clone(),
          call_id: request.call_id.clone(),
          operation_key: request.identity.operation_key.clone(),
          capability: request.capability.clone(),
          operation: request.operation.clone(),
          execution_mode: OperationExecutionMode::Managed,
          metadata,
          duration_ms: success.duration_ms,
          result_bytes: success.result_bytes,
          result_digest: format!("sha256:{}", hex::encode(Sha256::digest(&encoded))),
        })
      }
      CapabilityCallResult::Failed(failed) | CapabilityCallResult::Cancelled(failed) => {
        RunEventPayload::OperationFailed(OperationFailedData {
          node_id: request.node_id.clone(),
          attempt_number: request.attempt_number,
          invocation_id: request.invocation_id.clone(),
          call_id: request.call_id.clone(),
          operation_key: request.identity.operation_key.clone(),
          capability: request.capability.clone(),
          operation: request.operation.clone(),
          execution_mode: OperationExecutionMode::Managed,
          metadata,
          duration_ms: failed.duration_ms,
          failure: failed.error.clone(),
        })
      }
    };
    {
      let mut store = self.store.lock().await;
      store.append_payload(
        request.run_id.clone(),
        generated_event_id(),
        Utc::now(),
        payload,
      )?;
    }
    Ok(result)
  }
}

impl Default for CapabilityRegistry {
  fn default() -> Self {
    Self::new(
      DEFAULT_CAPABILITY_IN_FLIGHT_PER_INVOCATION,
      DEFAULT_CAPABILITY_IN_FLIGHT_GLOBAL,
    )
  }
}

impl CapabilityRegistry {
  pub fn new(max_per_invocation: usize, max_global: usize) -> Self {
    Self {
      handlers: Mutex::new(HashMap::new()),
      in_flight_by_invocation: Mutex::new(HashMap::new()),
      in_flight_global: AtomicUsize::new(0),
      max_per_invocation: max_per_invocation.max(1),
      max_global: max_global.max(1),
    }
  }

  pub fn register(
    &self,
    handler: Arc<dyn CapabilityHandler>,
  ) -> Result<(), CapabilityRegistryError> {
    let descriptor = handler.descriptor();
    let key = (descriptor.capability.clone(), descriptor.operation.clone());
    let mut handlers = self.handlers.lock().expect("capability registry lock");
    if handlers.contains_key(&key) {
      return Err(CapabilityRegistryError::DuplicateHandler {
        capability: key.0,
        operation: key.1,
      });
    }
    handlers.insert(key, handler);
    Ok(())
  }

  pub fn descriptor(&self, capability: &str, operation: &str) -> Option<CapabilityDescriptor> {
    self
      .handlers
      .lock()
      .expect("capability registry lock")
      .get(&(capability.to_string(), operation.to_string()))
      .map(|handler| handler.descriptor())
  }

  pub fn safe_metadata(
    &self,
    request: &CapabilityCallRequest,
  ) -> Result<Map<String, Value>, CapabilityFailure> {
    request.validate()?;
    let input_bytes = serialized_bytes(&request.input);
    if input_bytes > request.limits.input_bytes {
      return Err(failure_with_size(
        CapabilityFailureKind::InputTooLarge,
        "WOML_CAPABILITY_INPUT_TOO_LARGE",
        "The capability input exceeds its configured byte limit.",
        input_bytes,
        request.limits.input_bytes,
      ));
    }
    let handler = self.lookup_and_validate(request)?;
    let metadata = handler.safe_metadata(&request.input);
    validate_safe_metadata(&metadata).map_err(|message| {
      failure(
        CapabilityFailureKind::InvalidInput,
        "WOML_CAPABILITY_UNSAFE_METADATA",
        message,
        false,
        false,
      )
    })?;
    Ok(metadata)
  }

  pub async fn execute(
    &self,
    request: CapabilityCallRequest,
    cancellation: CapabilityCancellationToken,
  ) -> CapabilityCallResult {
    let started = Instant::now();
    let handler = match self.lookup_and_validate(&request) {
      Ok(handler) => handler,
      Err(error) => return failed_result(&request, started, error),
    };

    let input_bytes = serialized_bytes(&request.input);
    if input_bytes > request.limits.input_bytes {
      return failed_result(
        &request,
        started,
        failure_with_size(
          CapabilityFailureKind::InputTooLarge,
          "WOML_CAPABILITY_INPUT_TOO_LARGE",
          "The capability input exceeds its configured byte limit.",
          input_bytes,
          request.limits.input_bytes,
        ),
      );
    }
    let _permit = match self.acquire(&request.invocation_id) {
      Some(permit) => permit,
      None => {
        return failed_result(
          &request,
          started,
          failure(
            CapabilityFailureKind::ServiceRejected,
            "WOML_CAPABILITY_BACKPRESSURE",
            "The capability concurrency limit is currently full.",
            true,
            false,
          ),
        )
      }
    };
    let descriptor = handler.descriptor();
    let may_have_effect = descriptor.effect != CapabilityEffect::Read;
    let safe_to_retry = descriptor.effect != CapabilityEffect::UnsafeWrite;

    let handler_future =
      AssertUnwindSafe(handler.execute(request.input.clone(), cancellation.clone())).catch_unwind();
    let timeout = time::sleep(time::Duration::from_millis(request.limits.timeout_ms));
    tokio::pin!(handler_future);
    tokio::pin!(timeout);
    let outcome = tokio::select! {
      biased;
      _ = cancellation.cancelled() => Err(failure(
        CapabilityFailureKind::Cancelled,
        "WOML_CAPABILITY_CANCELLED",
        "The capability call was cancelled.",
        false,
        may_have_effect,
      )),
      _ = &mut timeout => Err(failure(
        CapabilityFailureKind::TimedOut,
        "WOML_CAPABILITY_TIMED_OUT",
        "The capability call exceeded its deadline.",
        safe_to_retry,
        may_have_effect,
      )),
      result = &mut handler_future => match result {
        Ok(result) => result,
        Err(_) => Err(failure(
          CapabilityFailureKind::HandlerCrashed,
          "WOML_CAPABILITY_HANDLER_CRASHED",
          "The capability handler crashed.",
          false,
          may_have_effect,
        )),
      }
    };

    match outcome {
      Ok(result) => {
        let result_bytes = serialized_bytes(&result);
        if result_bytes > request.limits.result_bytes {
          failed_result(
            &request,
            started,
            failure_with_size(
              CapabilityFailureKind::ResultTooLarge,
              "WOML_CAPABILITY_RESULT_TOO_LARGE",
              "The capability result exceeds its configured byte limit.",
              result_bytes,
              request.limits.result_bytes,
            ),
          )
        } else {
          CapabilityCallResult::Succeeded(CapabilityCallSucceeded {
            contract: CAPABILITY_CALL_CONTRACT.to_string(),
            contract_version: CAPABILITY_CALL_CONTRACT_VERSION,
            message_type: "result".to_string(),
            invocation_id: request.invocation_id,
            call_id: request.call_id,
            outcome: "succeeded".to_string(),
            result_contract_version: request.result_contract_version,
            result_bytes,
            duration_ms: duration_ms(started),
            result,
          })
        }
      }
      Err(error) if error.kind == CapabilityFailureKind::Cancelled => {
        CapabilityCallResult::Cancelled(failed_fields(&request, started, "cancelled", error))
      }
      Err(error) => failed_result(&request, started, error),
    }
  }

  fn lookup_and_validate(
    &self,
    request: &CapabilityCallRequest,
  ) -> Result<Arc<dyn CapabilityHandler>, CapabilityFailure> {
    request.validate()?;
    let handlers = self.handlers.lock().expect("capability registry lock");
    let handler = handlers
      .get(&(request.capability.clone(), request.operation.clone()))
      .cloned()
      .ok_or_else(|| {
        let kind = if handlers
          .keys()
          .any(|(capability, _)| capability == &request.capability)
        {
          CapabilityFailureKind::UnsupportedOperation
        } else {
          CapabilityFailureKind::UnsupportedCapability
        };
        failure(
          kind,
          if kind == CapabilityFailureKind::UnsupportedCapability {
            "WOML_CAPABILITY_UNSUPPORTED"
          } else {
            "WOML_CAPABILITY_OPERATION_UNSUPPORTED"
          },
          "No matching capability handler is registered.",
          false,
          false,
        )
      })?;
    let descriptor = handler.descriptor();
    if descriptor.input_contract_version != request.input_contract_version
      || descriptor.result_contract_version != request.result_contract_version
    {
      return Err(failure(
        CapabilityFailureKind::UnsupportedVersion,
        "WOML_CAPABILITY_VERSION_UNSUPPORTED",
        "The requested capability contract version is not supported.",
        false,
        false,
      ));
    }
    if request.identity.provider_idempotency_key.is_some()
      && !descriptor.supports_provider_idempotency
    {
      return Err(failure(
        CapabilityFailureKind::InvalidInput,
        "WOML_CAPABILITY_IDEMPOTENCY_UNSUPPORTED",
        "This capability does not support provider idempotency keys.",
        false,
        false,
      ));
    }
    Ok(handler)
  }

  fn acquire(&self, invocation_id: &str) -> Option<CapabilityPermit<'_>> {
    let mut per_invocation = self
      .in_flight_by_invocation
      .lock()
      .expect("capability in-flight lock");
    let current = per_invocation.get(invocation_id).copied().unwrap_or(0);
    if current >= self.max_per_invocation
      || self.in_flight_global.load(Ordering::SeqCst) >= self.max_global
    {
      return None;
    }
    per_invocation.insert(invocation_id.to_string(), current + 1);
    self.in_flight_global.fetch_add(1, Ordering::SeqCst);
    Some(CapabilityPermit {
      registry: self,
      invocation_id: invocation_id.to_string(),
    })
  }
}

struct CapabilityPermit<'a> {
  registry: &'a CapabilityRegistry,
  invocation_id: String,
}

impl Drop for CapabilityPermit<'_> {
  fn drop(&mut self) {
    let mut per_invocation = self
      .registry
      .in_flight_by_invocation
      .lock()
      .expect("capability in-flight lock");
    if let Some(count) = per_invocation.get_mut(&self.invocation_id) {
      *count -= 1;
      if *count == 0 {
        per_invocation.remove(&self.invocation_id);
      }
    }
    self
      .registry
      .in_flight_global
      .fetch_sub(1, Ordering::SeqCst);
  }
}

impl CapabilityCallRequest {
  pub fn validate(&self) -> Result<(), CapabilityFailure> {
    let valid = self.contract == CAPABILITY_CALL_CONTRACT
      && self.contract_version == CAPABILITY_CALL_CONTRACT_VERSION
      && self.message_type == "request"
      && [
        &self.invocation_id,
        &self.call_id,
        &self.run_id,
        &self.node_id,
      ]
      .into_iter()
      .all(|value| !value.is_empty() && value.len() <= 256)
      && (1..=10).contains(&self.attempt_number)
      && valid_name(&self.capability)
      && valid_name(&self.operation)
      && self.input_contract_version > 0
      && self.result_contract_version > 0
      && valid_digest(&self.identity.step_idempotency_key)
      && valid_digest(&self.identity.operation_key)
      && !self.identity.operation_name.is_empty()
      && self.identity.operation_name.len() <= 256
      && self
        .identity
        .provider_idempotency_key
        .as_ref()
        .is_none_or(|value| !value.is_empty() && value.len() <= 256)
      && (1..=DEFAULT_CAPABILITY_FRAME_BYTES).contains(&self.limits.input_bytes)
      && (1..=DEFAULT_CAPABILITY_FRAME_BYTES).contains(&self.limits.result_bytes)
      && (1..=MAX_CAPABILITY_TIMEOUT_MS).contains(&self.limits.timeout_ms);
    if valid {
      Ok(())
    } else {
      Err(failure(
        CapabilityFailureKind::InvalidInput,
        "WOML_CAPABILITY_REQUEST_INVALID",
        "The capability call does not match Capability Call v1.",
        false,
        false,
      ))
    }
  }
}

impl CapabilityFailure {
  pub fn validate(&self) -> Result<(), String> {
    let code_valid = !self.code.is_empty()
      && self.code.len() <= 128
      && self
        .code
        .starts_with(|character: char| character.is_ascii_uppercase())
      && self
        .code
        .bytes()
        .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_');
    if !code_valid || self.message.is_empty() || self.message.len() > 2048 {
      return Err("Capability failure has an invalid code or message.".to_string());
    }
    if let Some(details) = &self.details {
      validate_safe_metadata(details)?;
    }
    Ok(())
  }
}

pub fn derive_operation_key(step_idempotency_key: &str, operation_name: &str) -> String {
  let mut digest = Sha256::new();
  digest.update(b"woml.capability-operation\0v1\0");
  digest.update(step_idempotency_key.as_bytes());
  digest.update(b"\0");
  digest.update(operation_name.as_bytes());
  format!("sha256:{}", hex::encode(digest.finalize()))
}

pub fn validate_safe_metadata(metadata: &Map<String, Value>) -> Result<(), String> {
  const FORBIDDEN: &[&str] = &[
    "authorization",
    "cookie",
    "set-cookie",
    "token",
    "secret",
    "password",
    "body",
    "query",
    "url",
    "headers",
    "input",
    "output",
    "data",
    "value",
  ];
  if metadata.len() > 32 {
    return Err("Safe metadata may contain at most 32 fields.".to_string());
  }
  for (key, value) in metadata {
    let valid_key = key.len() <= 64
      && key
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic())
      && key
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "_.-".contains(character))
      && !FORBIDDEN.contains(&key.to_ascii_lowercase().as_str());
    if !valid_key
      || !matches!(
        value,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
      )
    {
      return Err(
        "Capability metadata must be bounded, scalar, and free of sensitive field names."
          .to_string(),
      );
    }
  }
  Ok(())
}

fn valid_name(value: &str) -> bool {
  value.len() <= 128
    && value.split(['.', '_', '-']).all(|segment| {
      let mut chars = segment.chars();
      matches!(chars.next(), Some(first) if first.is_ascii_lowercase())
        && chars.all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
    })
}

fn valid_digest(value: &str) -> bool {
  value.strip_prefix("sha256:").is_some_and(|digest| {
    digest.len() == 64
      && digest
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
  })
}

fn serialized_bytes(value: &Value) -> u64 {
  serde_json::to_vec(value)
    .map(|bytes| bytes.len() as u64)
    .unwrap_or(u64::MAX)
}

fn validate_result_base(
  contract: &str,
  contract_version: u32,
  message_type: &str,
  invocation_id: &str,
  call_id: &str,
  duration_ms: f64,
) -> Result<(), String> {
  if contract != CAPABILITY_CALL_CONTRACT
    || contract_version != CAPABILITY_CALL_CONTRACT_VERSION
    || message_type != "result"
    || invocation_id.is_empty()
    || invocation_id.len() > 256
    || call_id.is_empty()
    || call_id.len() > 256
    || !duration_ms.is_finite()
    || duration_ms < 0.0
  {
    return Err(
      "Capability result does not match the Capability Call v1 base contract.".to_string(),
    );
  }
  Ok(())
}

fn generated_event_id() -> String {
  format!("event_{}", Uuid::new_v4().simple())
}

fn duration_ms(started: Instant) -> f64 {
  started.elapsed().as_secs_f64() * 1_000.0
}

fn failure(
  kind: CapabilityFailureKind,
  code: impl Into<String>,
  message: impl Into<String>,
  retryable: bool,
  ambiguous: bool,
) -> CapabilityFailure {
  CapabilityFailure {
    kind,
    code: code.into(),
    message: message.into(),
    retryable,
    ambiguous,
    details: None,
  }
}

fn failure_with_size(
  kind: CapabilityFailureKind,
  code: &str,
  message: &str,
  actual_bytes: u64,
  limit_bytes: u64,
) -> CapabilityFailure {
  let mut details = Map::new();
  details.insert("actualBytes".to_string(), Value::from(actual_bytes));
  details.insert("limitBytes".to_string(), Value::from(limit_bytes));
  CapabilityFailure {
    kind,
    code: code.to_string(),
    message: message.to_string(),
    retryable: false,
    ambiguous: false,
    details: Some(details),
  }
}

fn failed_fields(
  request: &CapabilityCallRequest,
  started: Instant,
  outcome: &str,
  error: CapabilityFailure,
) -> CapabilityCallFailed {
  CapabilityCallFailed {
    contract: CAPABILITY_CALL_CONTRACT.to_string(),
    contract_version: CAPABILITY_CALL_CONTRACT_VERSION,
    message_type: "result".to_string(),
    invocation_id: request.invocation_id.clone(),
    call_id: request.call_id.clone(),
    outcome: outcome.to_string(),
    duration_ms: duration_ms(started),
    error,
  }
}

fn failed_result(
  request: &CapabilityCallRequest,
  started: Instant,
  error: CapabilityFailure,
) -> CapabilityCallResult {
  CapabilityCallResult::Failed(failed_fields(request, started, "failed", error))
}

/// Builds a safe correlated failure when a capability request cannot reach a
/// durable authority. The original input is deliberately excluded.
pub fn capability_transport_failure(
  request: &CapabilityCallRequest,
  code: &str,
  message: &str,
  retryable: bool,
  ambiguous: bool,
) -> CapabilityCallResult {
  failed_result(
    request,
    Instant::now(),
    failure(
      CapabilityFailureKind::TransportFailed,
      code,
      message,
      retryable,
      ambiguous,
    ),
  )
}

/// Deterministic fake handler used by Rust conformance tests. It is never
/// registered in a production registry automatically.
#[derive(Debug, Default)]
pub struct TestCapabilityHandler;

impl CapabilityHandler for TestCapabilityHandler {
  fn descriptor(&self) -> CapabilityDescriptor {
    CapabilityDescriptor {
      capability: "test".to_string(),
      operation: "control".to_string(),
      input_contract_version: 1,
      result_contract_version: 1,
      effect: CapabilityEffect::Read,
      supports_cancellation: true,
      supports_provider_idempotency: false,
    }
  }

  fn safe_metadata(&self, input: &Value) -> Map<String, Value> {
    let mut metadata = Map::new();
    if let Some(mode) = input.get("mode").and_then(Value::as_str) {
      metadata.insert("mode".to_string(), Value::String(mode.to_string()));
    }
    metadata
  }

  fn execute(
    &self,
    input: Value,
    cancellation: CapabilityCancellationToken,
  ) -> BoxFuture<'static, Result<Value, CapabilityFailure>> {
    Box::pin(async move {
      match input.get("mode").and_then(Value::as_str).unwrap_or("echo") {
        "echo" => Ok(input.get("value").cloned().unwrap_or(Value::Null)),
        "delay" => {
          let delay_ms = input.get("delayMs").and_then(Value::as_u64).unwrap_or(1);
          tokio::select! {
            _ = time::sleep(time::Duration::from_millis(delay_ms)) => Ok(input.get("value").cloned().unwrap_or(Value::Null)),
            _ = cancellation.cancelled() => Err(failure(CapabilityFailureKind::Cancelled, "WOML_TEST_CANCELLED", "The test operation was cancelled.", false, false)),
          }
        }
        "fail" => Err(failure(
          CapabilityFailureKind::ServiceRejected,
          "WOML_TEST_FAILURE",
          "The test capability was instructed to fail.",
          false,
          false,
        )),
        "panic" => panic!("test capability panic"),
        _ => Err(failure(
          CapabilityFailureKind::InvalidInput,
          "WOML_TEST_MODE_INVALID",
          "The test capability mode is invalid.",
          false,
          false,
        )),
      }
    })
  }
}
