//! Existing deterministic test capability, retaining its original exported identity and behavior.

use crate::capability::failure;
use crate::capability::CapabilityCancellationToken;
use crate::capability::CapabilityDescriptor;
use crate::capability::CapabilityEffect;
use crate::capability::CapabilityFailure;
use crate::capability::CapabilityFailureKind;
use crate::capability::CapabilityHandler;
use futures_util::future::BoxFuture;
use serde_json::Map;
use serde_json::Value;
use tokio::time;

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
