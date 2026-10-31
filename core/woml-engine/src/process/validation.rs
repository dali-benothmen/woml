//! Strict process payload, alias, cwd, environment-name, and resource-limit validation under the prepared policy.

use crate::process::invalid_input;
use crate::process::rejected;
use crate::process::PreparedProcessPolicy;
use crate::process::ProcessPolicyError;
use crate::process::ProcessPolicyLimits;
use crate::process::ProcessRunRequest;
use crate::process::MAX_PROCESS_STREAM_BYTES;
use crate::process::PROCESS_CONTRACT;
use crate::process::PROCESS_CONTRACT_VERSION;
use crate::process::PROCESS_CONTRACT_VERSION_V2;
use crate::CapabilityFailure;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

pub(in crate::process) fn parse_request(
  input: &Value,
) -> Result<ProcessRunRequest, CapabilityFailure> {
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

pub(in crate::process) fn authorize_cwd(
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

pub(in crate::process) fn validate_alias(alias: &str) -> Result<(), ProcessPolicyError> {
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

pub(in crate::process) fn validate_environment_name(name: &str) -> Result<(), String> {
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

pub(in crate::process) fn validate_limits(
  limits: &ProcessPolicyLimits,
) -> Result<(), ProcessPolicyError> {
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

pub(in crate::process) fn canonical_directory(
  path: &Path,
  label: &str,
) -> Result<PathBuf, ProcessPolicyError> {
  let canonical = std::fs::canonicalize(path)
    .map_err(|_| ProcessPolicyError::Invalid(format!("the {label} does not exist")))?;
  if !canonical.is_dir() {
    return Err(ProcessPolicyError::Invalid(format!(
      "the {label} is not a directory"
    )));
  }
  Ok(canonical)
}
