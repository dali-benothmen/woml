//! Minimal native boundary for the WOML Rust execution path.

use std::path::PathBuf;

use napi_derive::napi;
use serde::Serialize;
use serde_json::{Map, Value};
use woml_engine::{
  execute_workflow, execute_workflow_durable, recover_durable_runs, CompiledWorkflowDefinition,
  RunEventPayload, RunFailedData, RunFailedDataV2, RuntimeExecutionError, RuntimeExecutionOptions,
  ScriptHostProcessOptions,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeExecutionError {
  kind: &'static str,
  code: String,
  message: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  node_id: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  branch_id: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  arm_id: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  reference_path: Option<Vec<String>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  branch_site: Option<&'static str>,
}

fn native_execution_error(error: RuntimeExecutionError) -> napi::Error {
  let envelope = match error {
    RuntimeExecutionError::RunFailed(details) => {
      let node_id = details.events.iter().rev().find_map(|event| {
        if let RunEventPayload::RunFailed(data) = &event.payload {
          match data {
            RunFailedData::V1(data) => data.node_id.clone(),
            RunFailedData::V2(RunFailedDataV2::Attempt { node_id, .. }) => Some(node_id.clone()),
            RunFailedData::V2(RunFailedDataV2::Branch { .. }) => None,
          }
        } else {
          None
        }
      });
      NativeExecutionError {
        kind: "woml_execution_error",
        code: details.code.clone(),
        message: details.message.clone(),
        node_id,
        branch_id: None,
        arm_id: None,
        reference_path: None,
        branch_site: None,
      }
    }
    RuntimeExecutionError::BranchFailed(details) => NativeExecutionError {
      kind: "woml_execution_error",
      code: details.code.clone(),
      message: details.message.clone(),
      node_id: None,
      branch_id: Some(details.branch_id.clone()),
      arm_id: details.arm_id.clone(),
      reference_path: details.path.clone(),
      branch_site: Some(details.site.as_str()),
    },
    error => NativeExecutionError {
      kind: "woml_execution_error",
      code: "WOML_RUST_EXECUTION_FAILED".to_string(),
      message: error.to_string(),
      node_id: None,
      branch_id: None,
      arm_id: None,
      reference_path: None,
      branch_site: None,
    },
  };
  let reason = serde_json::to_string(&envelope).unwrap_or_else(|_| {
    "WOML Rust execution failed and its error could not be encoded.".to_string()
  });
  napi::Error::from_reason(reason)
}

#[napi(ts_return_type = "Promise<string>")]
pub async fn execute_woml_workflow(
  compiled_model_json: String,
  definition_hash: String,
  trigger_json: String,
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
) -> napi::Result<String> {
  let workflow: CompiledWorkflowDefinition =
    serde_json::from_str(&compiled_model_json).map_err(|error| {
      napi::Error::from_reason(format!("Invalid compiled workflow JSON: {error}"))
    })?;
  let trigger: Map<String, Value> = serde_json::from_str(&trigger_json)
    .map_err(|error| napi::Error::from_reason(format!("Invalid trigger JSON: {error}")))?;
  let host_options = ScriptHostProcessOptions::new(
    PathBuf::from(bun_executable),
    PathBuf::from(script_host_path),
  );
  let options = RuntimeExecutionOptions::new(host_options, u64::from(script_timeout_ms));
  let result = execute_workflow(workflow, definition_hash, trigger, options)
    .await
    .map_err(native_execution_error)?;
  serde_json::to_string(&result)
    .map_err(|error| napi::Error::from_reason(format!("Could not encode WOML result: {error}")))
}

#[napi(ts_return_type = "Promise<string>")]
pub async fn execute_woml_workflow_durable(
  compiled_model_json: String,
  definition_hash: String,
  trigger_json: String,
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  event_store_path: String,
) -> napi::Result<String> {
  let workflow: CompiledWorkflowDefinition =
    serde_json::from_str(&compiled_model_json).map_err(|error| {
      napi::Error::from_reason(format!("Invalid compiled workflow JSON: {error}"))
    })?;
  let trigger: Map<String, Value> = serde_json::from_str(&trigger_json)
    .map_err(|error| napi::Error::from_reason(format!("Invalid trigger JSON: {error}")))?;
  let host_options = ScriptHostProcessOptions::new(
    PathBuf::from(bun_executable),
    PathBuf::from(script_host_path),
  );
  let options = RuntimeExecutionOptions::new(host_options, u64::from(script_timeout_ms));
  let result = execute_workflow_durable(
    workflow,
    definition_hash,
    trigger,
    options,
    PathBuf::from(event_store_path),
  )
  .await
  .map_err(native_execution_error)?;
  serde_json::to_string(&result)
    .map_err(|error| napi::Error::from_reason(format!("Could not encode WOML result: {error}")))
}

#[napi]
pub fn recover_woml_runs(event_store_path: String) -> napi::Result<String> {
  let report = recover_durable_runs(PathBuf::from(event_store_path))
    .map_err(|error| napi::Error::from_reason(error.to_string()))?;
  serde_json::to_string(&report)
    .map_err(|error| napi::Error::from_reason(format!("Could not encode recovery report: {error}")))
}
