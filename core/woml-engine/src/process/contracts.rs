//! Frozen Process v1/v2 requests/results and operator-policy shapes. Omitted streaming retains v1 semantics.

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use thiserror::Error;

pub const PROCESS_CONTRACT: &str = "woml.process";

pub const PROCESS_CONTRACT_VERSION: u32 = 1;

pub const PROCESS_CONTRACT_VERSION_V2: u32 = 2;

pub const PROCESS_POLICY_CONTRACT: &str = "woml.process-policy";

pub const PROCESS_POLICY_CONTRACT_VERSION: u32 = 1;

pub(in crate::process) const MAX_PROCESS_STREAM_BYTES: usize = 256 * 1024 * 1024;

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
pub(in crate::process) enum OutputMode {
  Capture,
  Discard,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(in crate::process) struct ProcessOutputRequest {
  pub(in crate::process) stdout: OutputMode,
  pub(in crate::process) stderr: OutputMode,
  pub(in crate::process) max_bytes: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(in crate::process) struct ProcessRunRequest {
  pub(in crate::process) contract: String,
  pub(in crate::process) contract_version: u32,
  pub(in crate::process) kind: String,
  pub(in crate::process) executable: String,
  pub(in crate::process) args: Vec<String>,
  pub(in crate::process) cwd: String,
  pub(in crate::process) env: BTreeMap<String, String>,
  pub(in crate::process) timeout_ms: u64,
  pub(in crate::process) accept_exit_codes: Vec<i32>,
  pub(in crate::process) output: ProcessOutputRequest,
  #[serde(default)]
  pub(in crate::process) stream: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::process) struct ProcessRunResultV1 {
  pub(in crate::process) contract: &'static str,
  pub(in crate::process) contract_version: u32,
  pub(in crate::process) kind: &'static str,
  pub(in crate::process) exit_code: Option<i32>,
  pub(in crate::process) signal: Option<String>,
  pub(in crate::process) stdout: String,
  pub(in crate::process) stderr: String,
  pub(in crate::process) stdout_bytes: u64,
  pub(in crate::process) stderr_bytes: u64,
  pub(in crate::process) stdout_truncated: bool,
  pub(in crate::process) stderr_truncated: bool,
  pub(in crate::process) duration_ms: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::process) struct ProcessRunResultV2 {
  pub(in crate::process) contract: &'static str,
  pub(in crate::process) contract_version: u32,
  pub(in crate::process) kind: &'static str,
  pub(in crate::process) exit_code: Option<i32>,
  pub(in crate::process) signal: Option<String>,
  pub(in crate::process) stdout: String,
  pub(in crate::process) stderr: String,
  pub(in crate::process) stdout_bytes: u64,
  pub(in crate::process) stderr_bytes: u64,
  pub(in crate::process) stdout_truncated: bool,
  pub(in crate::process) stderr_truncated: bool,
  pub(in crate::process) duration_ms: f64,
  pub(in crate::process) streamed: bool,
  pub(in crate::process) stream_start_sequence: Option<u64>,
  pub(in crate::process) stream_end_sequence: Option<u64>,
  pub(in crate::process) stream_truncated: bool,
  pub(in crate::process) stream_artifact: Option<Value>,
}
