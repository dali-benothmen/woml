//! Private bridge configuration.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use woml_engine::prepare_emergency_deny_overlay;
use woml_engine::prepare_infrastructure_policy;
use woml_engine::ArtifactPolicyV1;
use woml_engine::ArtifactRuntimeMode;
use woml_engine::InfrastructurePolicyBundleV1;
use woml_engine::InfrastructurePolicyTrustV1;
use woml_engine::ProcessPolicyV1;
use woml_engine::ProcessRuntimeMode;
use woml_engine::ReviewerDirectoryAuthorityV1;
use woml_engine::ReviewerDirectoryV1;
use woml_engine::RuntimeExecutionOptions;
use woml_engine::RuntimeModuleArtifact;
use woml_engine::ScriptHostProcessOptions;

pub(super) fn runtime_options(
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
) -> RuntimeExecutionOptions {
  RuntimeExecutionOptions::new(
    ScriptHostProcessOptions::new(
      PathBuf::from(bun_executable),
      PathBuf::from(script_host_path),
    ),
    u64::from(script_timeout_ms),
  )
}

pub(super) fn runtime_options_with_secrets(
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  resolved_secrets_json: String,
) -> napi::Result<RuntimeExecutionOptions> {
  let secrets: BTreeMap<String, String> = serde_json::from_str(&resolved_secrets_json)
    .map_err(|error| napi::Error::from_reason(format!("Invalid resolved secrets JSON: {error}")))?;
  Ok(
    runtime_options(bun_executable, script_host_path, script_timeout_ms)
      .with_resolved_secrets(secrets),
  )
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NativeRuntimeCapabilities {
  #[serde(default)]
  pub(super) process: Option<NativeProcessRuntime>,
  #[serde(default)]
  pub(super) artifacts: Option<NativeArtifactRuntime>,
  #[serde(default)]
  pub(super) reviewers: Option<NativeReviewerRuntime>,
  #[serde(default)]
  pub(super) infrastructure_policy: Option<NativeInfrastructurePolicyRuntime>,
  #[serde(default)]
  pub(super) emergency_deny: Option<NativeInfrastructurePolicyRuntime>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct NativeInfrastructurePolicyTrust {
  #[serde(default)]
  pub(super) keys: BTreeMap<String, String>,
  #[serde(default)]
  pub(super) revoked_key_ids: BTreeSet<String>,
  #[serde(default)]
  pub(super) require_signature: bool,
}

impl From<NativeInfrastructurePolicyTrust> for InfrastructurePolicyTrustV1 {
  fn from(value: NativeInfrastructurePolicyTrust) -> Self {
    Self {
      keys: value.keys,
      revoked_key_ids: value.revoked_key_ids,
      require_signature: value.require_signature,
    }
  }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct NativeInfrastructurePolicyRuntime {
  pub(super) policy: InfrastructurePolicyBundleV1,
  #[serde(default)]
  pub(super) trust: NativeInfrastructurePolicyTrust,
  #[serde(default)]
  pub(super) required: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NativeProcessRuntime {
  pub(super) mode: ProcessRuntimeMode,
  pub(super) policy: ProcessPolicyV1,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NativeArtifactRuntime {
  pub(super) mode: ArtifactRuntimeMode,
  pub(super) policy: ArtifactPolicyV1,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NativeReviewerRuntime {
  pub(super) directory: ReviewerDirectoryV1,
}

pub(super) fn apply_runtime_capabilities(
  options: RuntimeExecutionOptions,
  runtime_capabilities_json: Option<String>,
) -> napi::Result<RuntimeExecutionOptions> {
  let Some(source) = runtime_capabilities_json else {
    return Ok(options);
  };
  let capabilities: NativeRuntimeCapabilities = serde_json::from_str(&source).map_err(|error| {
    napi::Error::from_reason(format!("Invalid runtime capabilities JSON: {error}"))
  })?;
  let options = match capabilities.process {
    Some(process) => options
      .with_process_policy(process.policy, process.mode)
      .map_err(|error| napi::Error::from_reason(error.to_string())),
    None => Ok(options),
  }?;
  let options = match capabilities.artifacts {
    Some(artifacts) => options
      .with_artifact_policy(artifacts.policy, artifacts.mode)
      .map_err(|error| napi::Error::from_reason(error.to_string())),
    None => Ok(options),
  }?;
  let options = match capabilities.reviewers {
    Some(reviewers) => ReviewerDirectoryAuthorityV1::from_document(reviewers.directory)
      .map(|authority| options.with_reviewer_directory(Arc::new(authority)))
      .map_err(|error| napi::Error::from_reason(format!("{}: {error}", error.code()))),
    None => Ok(options),
  }?;
  let mut base_policy_environment = None;
  let options = if let Some(policy) = capabilities.infrastructure_policy {
    let required = policy.required;
    let prepared = prepare_infrastructure_policy(policy.policy, &policy.trust.into())
      .map_err(|error| napi::Error::from_reason(format!("{}: {error}", error.code())))?;
    base_policy_environment = Some(prepared.bundle.environment.clone());
    let options = options.with_infrastructure_policy(prepared);
    if required {
      options.require_infrastructure_policy()
    } else {
      options
    }
  } else {
    options
  };
  match capabilities.emergency_deny {
    Some(policy) => {
      let prepared = prepare_emergency_deny_overlay(policy.policy, &policy.trust.into())
        .map_err(|error| napi::Error::from_reason(format!("{}: {error}", error.code())))?;
      if base_policy_environment.as_deref() != Some(prepared.bundle.environment.as_str()) {
        return Err(napi::Error::from_reason(
          "WOML_POLICY_INVALID: an emergency deny overlay requires a base policy in the same environment."
            .to_string(),
        ));
      }
      Ok(options.with_emergency_deny_overlay(prepared))
    }
    None => Ok(options),
  }
}

pub(super) fn runtime_options_with_modules(
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  resolved_secrets_json: String,
  runtime_modules_json: Option<String>,
  runtime_capabilities_json: Option<String>,
) -> napi::Result<RuntimeExecutionOptions> {
  let modules: Vec<RuntimeModuleArtifact> =
    serde_json::from_str(runtime_modules_json.as_deref().unwrap_or("[]")).map_err(|error| {
      napi::Error::from_reason(format!("Invalid runtime modules JSON: {error}"))
    })?;
  apply_runtime_capabilities(
    runtime_options_with_secrets(
      bun_executable,
      script_host_path,
      script_timeout_ms,
      resolved_secrets_json,
    )?
    .with_runtime_modules(modules),
    runtime_capabilities_json,
  )
}

pub(super) fn decode_artifact_maintenance_authority(
  policy_json: &str,
  mode_json: &str,
  resolved_secrets_json: &str,
) -> napi::Result<(
  ArtifactPolicyV1,
  ArtifactRuntimeMode,
  BTreeMap<String, String>,
)> {
  let policy = serde_json::from_str(policy_json)
    .map_err(|_| napi::Error::from_reason("Artifact Policy v1 is invalid.".to_string()))?;
  let mode = serde_json::from_str(mode_json)
    .map_err(|_| napi::Error::from_reason("Artifact runtime mode is invalid.".to_string()))?;
  let secrets = serde_json::from_str(resolved_secrets_json)
    .map_err(|_| napi::Error::from_reason("Resolved artifact secrets are invalid.".to_string()))?;
  Ok((policy, mode, secrets))
}
