//! Registered handler lookup, contract/result bounds, safe metadata, and bounded execution supervision. Durable permission is checked separately by authority.

use crate::capability::duration_ms;
use crate::capability::failed_fields;
use crate::capability::failed_result;
use crate::capability::failure;
use crate::capability::failure_with_size;
use crate::capability::serialized_bytes;
use crate::capability::validate_safe_metadata;
use crate::capability::CapabilityCallRequest;
use crate::capability::CapabilityCallResult;
use crate::capability::CapabilityCallSucceeded;
use crate::capability::CapabilityCancellationToken;
use crate::capability::CapabilityDescriptor;
use crate::capability::CapabilityEffect;
use crate::capability::CapabilityFailure;
use crate::capability::CapabilityFailureKind;
use crate::capability::CapabilityHandler;
use crate::capability::CAPABILITY_CALL_CONTRACT;
use crate::capability::CAPABILITY_CALL_CONTRACT_VERSION;
use crate::capability::DEFAULT_CAPABILITY_IN_FLIGHT_GLOBAL;
use crate::capability::DEFAULT_CAPABILITY_IN_FLIGHT_PER_INVOCATION;
use crate::infrastructure_policy_runtime::InfrastructurePolicyDescriptorContextV1;
use crate::infrastructure_policy_runtime::InfrastructurePolicyOperationDescriptorV1;
use futures_util::FutureExt;
use serde_json::Map;
use serde_json::Value;
use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;
use thiserror::Error;
use tokio::time;

const CAPABILITY_CANCELLATION_CLEANUP_MS: u64 = 3_000;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CapabilityRegistryError {
  #[error("capability handler {capability}.{operation} is already registered")]
  DuplicateHandler {
    capability: String,
    operation: String,
  },
}

pub struct CapabilityRegistry {
  pub(in crate::capability) handlers: Mutex<HashMap<(String, String), Arc<dyn CapabilityHandler>>>,
  pub(in crate::capability) in_flight_by_invocation: Mutex<HashMap<String, usize>>,
  pub(in crate::capability) in_flight_global: AtomicUsize,
  pub(in crate::capability) max_per_invocation: usize,
  pub(in crate::capability) max_global: usize,
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
    let metadata = handler.safe_request_metadata(request)?;
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

  /// Builds a trusted operation descriptor after both the generic capability
  /// contract and the concrete registered handler have accepted the request.
  pub fn infrastructure_policy_descriptor(
    &self,
    request: &CapabilityCallRequest,
    context: InfrastructurePolicyDescriptorContextV1,
  ) -> Result<InfrastructurePolicyOperationDescriptorV1, CapabilityFailure> {
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
    let metadata = handler.infrastructure_policy_metadata(request)?;
    InfrastructurePolicyOperationDescriptorV1::build(
      request,
      &handler.descriptor(),
      handler.effect(&request.input),
      context,
      metadata,
    )
  }

  pub async fn execute(
    &self,
    request: CapabilityCallRequest,
    cancellation: CapabilityCancellationToken,
  ) -> CapabilityCallResult {
    self.execute_scoped(request, None, cancellation).await
  }

  pub(crate) async fn execute_scoped(
    &self,
    request: CapabilityCallRequest,
    workflow_scope: Option<String>,
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
    let effect = handler.effect(&request.input);
    let supports_cancellation = handler.descriptor().supports_cancellation;
    let may_have_effect = effect != CapabilityEffect::Read;
    let safe_to_retry = effect != CapabilityEffect::UnsafeWrite;

    let handler_future = AssertUnwindSafe(handler.execute_request_scoped(
      &request,
      workflow_scope,
      cancellation.clone(),
    ))
    .catch_unwind();
    let timeout = time::sleep(time::Duration::from_millis(request.limits.timeout_ms));
    tokio::pin!(handler_future);
    tokio::pin!(timeout);
    let outcome = tokio::select! {
      biased;
      _ = cancellation.cancelled() => {
        let fallback = failure(
          CapabilityFailureKind::Cancelled,
          "WOML_CAPABILITY_CANCELLED",
          "The capability call was cancelled.",
          false,
          may_have_effect,
        );
        if supports_cancellation {
          match time::timeout(
            time::Duration::from_millis(CAPABILITY_CANCELLATION_CLEANUP_MS),
            &mut handler_future,
          ).await {
            Ok(Ok(Err(error))) if error.kind == CapabilityFailureKind::Cancelled => Err(error),
            _ => Err(fallback),
          }
        } else {
          Err(fallback)
        }
      },
      _ = &mut timeout => {
        let fallback = failure(
          CapabilityFailureKind::TimedOut,
          "WOML_CAPABILITY_TIMED_OUT",
          "The capability call exceeded its deadline.",
          safe_to_retry,
          may_have_effect,
        );
        if supports_cancellation {
          cancellation.cancel();
          match time::timeout(
            time::Duration::from_millis(CAPABILITY_CANCELLATION_CLEANUP_MS),
            &mut handler_future,
          ).await {
            Ok(Ok(Err(error))) if error.kind == CapabilityFailureKind::TimedOut => Err(error),
            _ => Err(fallback),
          }
        } else {
          Err(fallback)
        }
      },
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

  pub(in crate::capability) fn lookup_and_validate(
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
    handler.validate_request(request)?;
    Ok(handler)
  }

  pub fn safe_result_metadata(
    &self,
    request: &CapabilityCallRequest,
    result: &Value,
  ) -> Result<Map<String, Value>, CapabilityFailure> {
    let handler = self.lookup_and_validate(request)?;
    let metadata = handler.safe_result_metadata(result);
    validate_safe_metadata(&metadata).map_err(|message| {
      failure(
        CapabilityFailureKind::InvalidResult,
        "WOML_CAPABILITY_UNSAFE_RESULT_METADATA",
        message,
        false,
        false,
      )
    })?;
    Ok(metadata)
  }

  pub fn safe_failure_metadata(
    &self,
    request: &CapabilityCallRequest,
    failure: &CapabilityFailure,
  ) -> Result<Map<String, Value>, CapabilityFailure> {
    let handler = self.lookup_and_validate(request)?;
    let metadata = handler.safe_failure_metadata(failure);
    validate_safe_metadata(&metadata).map_err(|message| {
      crate::capability::failure(
        CapabilityFailureKind::InvalidResult,
        "WOML_CAPABILITY_UNSAFE_FAILURE_METADATA",
        message,
        false,
        false,
      )
    })?;
    Ok(metadata)
  }
}
