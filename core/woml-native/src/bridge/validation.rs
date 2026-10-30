//! Private bridge validation.

use super::native_execution_error;
use super::NativeManualAdmissionRejected;
use std::path::PathBuf;
use woml_engine::CompiledWorkflowDefinition;
use woml_engine::DurableEventStore;
use woml_engine::RuntimeExecutionError;

pub(super) fn encode_manual_rejection(
  request_id: String,
  code: &'static str,
  message: &'static str,
) -> napi::Result<String> {
  serde_json::to_string(&NativeManualAdmissionRejected {
    profile: "woml.manual-trigger-admission/v1",
    message_type: "rejected",
    request_id,
    code,
    message,
  })
  .map_err(|error| {
    napi::Error::from_reason(format!(
      "Could not encode manual admission rejection: {error}"
    ))
  })
}

pub(super) fn verify_durable_run_definition(
  event_store_path: &str,
  run_id: &str,
  definition_hash: &str,
  workflow: &CompiledWorkflowDefinition,
) -> napi::Result<()> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_execution_error(RuntimeExecutionError::DurableStore(error)))?;
  let binding = store
    .run_binding(run_id)
    .map_err(|error| native_execution_error(RuntimeExecutionError::DurableStore(error)))?;
  let stored_workflow = store
    .definition(&binding.definition_hash)
    .map_err(|error| native_execution_error(RuntimeExecutionError::DurableStore(error)))?;
  if binding.definition_hash != definition_hash || stored_workflow != *workflow {
    return Err(native_execution_error(
      RuntimeExecutionError::InvalidConfiguration(
        "the supplied WOML definition does not match the durable run definition".to_string(),
      ),
    ));
  }
  Ok(())
}

pub(super) fn operational_cursor(value: f64) -> napi::Result<u64> {
  if !value.is_finite() || value < 0.0 || value.fract() != 0.0 || value > 9_007_199_254_740_991.0 {
    return Err(napi::Error::from_reason(
      "WOML_OUTPUT_CURSOR_INVALID: the operational-output cursor is invalid".to_string(),
    ));
  }
  Ok(value as u64)
}
