//! Frozen safe capability failure/result construction and monotonic duration accounting.

use crate::capability::CapabilityCallFailed;
use crate::capability::CapabilityCallRequest;
use crate::capability::CapabilityCallResult;
use crate::capability::CapabilityFailure;
use crate::capability::CapabilityFailureKind;
use crate::capability::CAPABILITY_CALL_CONTRACT;
use crate::capability::CAPABILITY_CALL_CONTRACT_VERSION;
use serde_json::Map;
use serde_json::Value;
use std::time::Instant;

pub(in crate::capability) fn duration_ms(started: Instant) -> f64 {
  started.elapsed().as_secs_f64() * 1_000.0
}

pub(in crate::capability) fn failure(
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

pub(in crate::capability) fn failure_with_size(
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

pub(in crate::capability) fn failed_fields(
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

pub(in crate::capability) fn failed_result(
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
