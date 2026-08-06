//! Minimal native boundary for the WOML Rust execution path.

use std::path::PathBuf;

use napi_derive::napi;
use serde_json::{Map, Value};
use woml_engine::{
  execute_workflow, execute_workflow_durable, recover_durable_runs, CompiledWorkflowDefinition,
  RuntimeExecutionOptions, ScriptHostProcessOptions,
};

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
    .map_err(|error| napi::Error::from_reason(error.to_string()))?;
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
  .map_err(|error| napi::Error::from_reason(error.to_string()))?;
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
