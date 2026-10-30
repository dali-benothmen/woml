//! Artifact operator policy, bounded limits, prepared local/S3 configuration, symbolic credential resolution, and redacted debugging.

use crate::artifacts::canonical_directory;
use crate::artifacts::ARTIFACT_POLICY_CONTRACT;
use crate::artifacts::ARTIFACT_POLICY_CONTRACT_VERSION;
use reqwest::Url;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use thiserror::Error;

pub(in crate::artifacts) const MAX_ARTIFACT_BYTES: u64 = 1_099_511_627_776;

const MAX_RUN_BYTES: u64 = 4_398_046_511_104;

const MAX_STORE_BYTES: u64 = 1_125_899_906_842_624;

const MAX_TRANSFER_TIMEOUT_MS: u64 = 86_400_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactRuntimeMode {
  Foreground,
  Background,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactPolicyV1 {
  pub contract: String,
  pub contract_version: u32,
  pub enabled: bool,
  pub modes: ArtifactPolicyModes,
  pub project_root: String,
  pub read_roots: Vec<String>,
  pub materialization: ArtifactMaterializationPolicy,
  pub limits: ArtifactLimits,
  pub retention: ArtifactRetentionPolicy,
  pub secret_scanning: ArtifactSecretScanningPolicy,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub operational_output: Option<ArtifactOperationalOutputPolicy>,
  pub backend: ArtifactBackendPolicy,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactOperationalOutputPolicy {
  pub enabled: bool,
  pub retention_ms: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactPolicyModes {
  pub foreground: bool,
  pub background: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactMaterializationPolicy {
  pub directory: String,
  pub cleanup_on_start: bool,
  pub max_bytes_per_attempt: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactLimits {
  pub max_artifact_bytes: u64,
  pub max_run_bytes: u64,
  pub max_store_bytes: u64,
  pub max_concurrent_transfers: usize,
  pub transfer_timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactRetentionPolicy {
  pub default_ms: u64,
  pub minimum_ms: u64,
  pub maximum_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum KnownSecretAction {
  Reject,
  Quarantine,
  Allow,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactSecretScanningPolicy {
  pub known_secret_action: KnownSecretAction,
  pub minimum_secret_bytes: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ArtifactBackendPolicy {
  Local {
    contract: String,
    #[serde(rename = "contractVersion")]
    contract_version: u32,
    directory: String,
  },
  S3 {
    contract: String,
    #[serde(rename = "contractVersion")]
    contract_version: u32,
    bucket: String,
    region: String,
    prefix: String,
    #[serde(default)]
    endpoint: Option<String>,
    #[serde(rename = "forcePathStyle")]
    force_path_style: bool,
    authentication: Box<Value>,
    #[serde(rename = "serverSideEncryption")]
    server_side_encryption: Box<Value>,
  },
}

#[derive(Debug, Error)]
pub enum ArtifactPolicyError {
  #[error("WOML_ARTIFACT_SERVICE_DISABLED: durable artifacts are disabled by runtime policy. Corrective action: provide a reviewed Artifact Policy v1 document")]
  Disabled,
  #[error("WOML_ARTIFACT_SERVICE_DISABLED: durable artifacts are not allowed in {0} mode. Corrective action: enable that mode in Artifact Policy v1")]
  ModeDenied(&'static str),
  #[error("WOML_ARTIFACT_POLICY_INVALID: {0}. Corrective action: repair Artifact Policy v1 and run WOML again")]
  Invalid(String),
}

#[derive(Clone)]
pub(crate) struct PreparedArtifactPolicy {
  pub(in crate::artifacts) project_root: PathBuf,
  pub(in crate::artifacts) read_roots: Vec<PathBuf>,
  pub(in crate::artifacts) backend: PreparedArtifactBackend,
  pub(in crate::artifacts) materialization_directory: PathBuf,
  pub(in crate::artifacts) max_materialization_bytes_per_attempt: u64,
  pub(in crate::artifacts) limits: ArtifactLimits,
  pub(in crate::artifacts) retention: ArtifactRetentionPolicy,
  pub(in crate::artifacts) secret_action: KnownSecretAction,
  pub(in crate::artifacts) minimum_secret_bytes: usize,
  pub(in crate::artifacts) operational_output: Option<ArtifactOperationalOutputPolicy>,
}

impl std::fmt::Debug for PreparedArtifactPolicy {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    formatter
      .debug_struct("PreparedArtifactPolicy")
      .field("project_root", &self.project_root)
      .field("read_roots", &self.read_roots)
      .field("backend", &self.backend)
      .field("materialization_directory", &self.materialization_directory)
      .field("limits", &self.limits)
      .field("retention", &self.retention)
      .field("secret_action", &self.secret_action)
      .field("operational_output", &self.operational_output)
      .finish()
  }
}

#[derive(Clone)]
pub(in crate::artifacts) enum PreparedArtifactBackend {
  Local { directory: PathBuf },
  S3(Box<PreparedS3Backend>),
}

impl std::fmt::Debug for PreparedArtifactBackend {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::Local { directory } => formatter
        .debug_struct("Local")
        .field("directory", directory)
        .finish(),
      Self::S3(config) => formatter
        .debug_struct("S3")
        .field("bucket", &config.bucket)
        .field("region", &config.region)
        .field("prefix", &config.prefix)
        .field("endpoint", &config.endpoint)
        .field("force_path_style", &config.force_path_style)
        .field("server_side_encryption", &config.server_side_encryption)
        .finish_non_exhaustive(),
    }
  }
}

#[derive(Clone)]
pub(in crate::artifacts) struct PreparedS3Backend {
  pub(in crate::artifacts) bucket: String,
  pub(in crate::artifacts) region: String,
  pub(in crate::artifacts) prefix: String,
  pub(in crate::artifacts) endpoint: Url,
  pub(in crate::artifacts) force_path_style: bool,
  pub(in crate::artifacts) credentials: S3Credentials,
  pub(in crate::artifacts) server_side_encryption: S3ServerSideEncryption,
}

#[derive(Clone)]
pub(in crate::artifacts) struct S3Credentials {
  pub(in crate::artifacts) access_key: String,
  pub(in crate::artifacts) secret_key: String,
  pub(in crate::artifacts) session_token: Option<String>,
}

#[derive(Debug, Clone)]
pub(in crate::artifacts) enum S3ServerSideEncryption {
  Aes256,
  AwsKms { key_id: String },
}

impl PreparedArtifactPolicy {
  pub(in crate::artifacts) fn prepare(
    policy: ArtifactPolicyV1,
    mode: ArtifactRuntimeMode,
    resolved_secrets: &BTreeMap<String, String>,
  ) -> Result<Self, ArtifactPolicyError> {
    if policy.contract != ARTIFACT_POLICY_CONTRACT
      || policy.contract_version != ARTIFACT_POLICY_CONTRACT_VERSION
    {
      return Err(ArtifactPolicyError::Invalid(
        "the document does not match Artifact Policy v1".to_string(),
      ));
    }
    if !policy.enabled {
      return Err(ArtifactPolicyError::Disabled);
    }
    let allowed = match mode {
      ArtifactRuntimeMode::Foreground => policy.modes.foreground,
      ArtifactRuntimeMode::Background => policy.modes.background,
    };
    if !allowed {
      return Err(ArtifactPolicyError::ModeDenied(match mode {
        ArtifactRuntimeMode::Foreground => "foreground",
        ArtifactRuntimeMode::Background => "background",
      }));
    }
    validate_policy_limits(&policy)?;
    let project_root = canonical_directory(Path::new(&policy.project_root), "project root")?;
    let mut read_roots = Vec::new();
    for root in &policy.read_roots {
      let candidate = Path::new(root);
      let path = if candidate.is_absolute() {
        candidate.to_path_buf()
      } else {
        project_root.join(candidate)
      };
      let canonical = canonical_directory(&path, "read root")?;
      if !canonical.starts_with(&project_root) {
        return Err(ArtifactPolicyError::Invalid(
          "every read root must remain beneath the project root".to_string(),
        ));
      }
      if !read_roots.contains(&canonical) {
        read_roots.push(canonical);
      }
    }
    let backend = match &policy.backend {
      ArtifactBackendPolicy::Local {
        contract,
        contract_version,
        directory,
      } if contract == "woml.artifact-backend" && *contract_version == 1 => {
        let authored = Path::new(directory);
        let directory = if authored.is_absolute() {
          authored.to_path_buf()
        } else {
          project_root.join(authored)
        };
        PreparedArtifactBackend::Local { directory }
      }
      ArtifactBackendPolicy::Local { .. } => {
        return Err(ArtifactPolicyError::Invalid(
          "the backend does not match Artifact Backend v1".to_string(),
        ))
      }
      ArtifactBackendPolicy::S3 {
        contract,
        contract_version,
        bucket,
        region,
        prefix,
        endpoint,
        force_path_style,
        authentication,
        server_side_encryption,
      } if contract == "woml.artifact-backend" && *contract_version == 1 => {
        PreparedArtifactBackend::S3(Box::new(prepare_s3_backend(
          bucket,
          region,
          prefix,
          endpoint.as_deref(),
          *force_path_style,
          authentication,
          server_side_encryption,
          resolved_secrets,
        )?))
      }
      ArtifactBackendPolicy::S3 { .. } => {
        return Err(ArtifactPolicyError::Invalid(
          "the S3 backend does not match Artifact Backend v1".to_string(),
        ))
      }
    };
    let materialization_path = Path::new(&policy.materialization.directory);
    let materialization_directory = if materialization_path.is_absolute() {
      materialization_path.to_path_buf()
    } else {
      project_root.join(materialization_path)
    };
    Ok(Self {
      project_root,
      read_roots,
      backend,
      materialization_directory,
      max_materialization_bytes_per_attempt: policy.materialization.max_bytes_per_attempt,
      limits: policy.limits,
      retention: policy.retention,
      secret_action: policy.secret_scanning.known_secret_action,
      minimum_secret_bytes: policy.secret_scanning.minimum_secret_bytes,
      operational_output: policy.operational_output,
    })
  }
}

#[derive(Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase", deny_unknown_fields)]
pub(in crate::artifacts) enum AuthoredS3Authentication {
  Ambient,
  Secrets {
    #[serde(rename = "accessKeySecret")]
    access_key_secret: String,
    #[serde(rename = "secretKeySecret")]
    secret_key_secret: String,
    #[serde(rename = "sessionTokenSecret")]
    session_token_secret: Option<String>,
  },
}

#[derive(Deserialize)]
#[serde(tag = "mode", deny_unknown_fields)]
pub(in crate::artifacts) enum AuthoredS3Encryption {
  #[serde(rename = "aes256")]
  Aes256,
  #[serde(rename = "aws-kms")]
  AwsKms {
    #[serde(rename = "keyId")]
    key_id: String,
  },
}

#[allow(clippy::too_many_arguments)]
fn prepare_s3_backend(
  bucket: &str,
  region: &str,
  prefix: &str,
  endpoint: Option<&str>,
  force_path_style: bool,
  authentication: &Value,
  server_side_encryption: &Value,
  resolved_secrets: &BTreeMap<String, String>,
) -> Result<PreparedS3Backend, ArtifactPolicyError> {
  if bucket.len() < 3
    || bucket.len() > 63
    || !bucket.bytes().all(|byte| {
      byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'.' || byte == b'-'
    })
    || !bucket
      .as_bytes()
      .first()
      .is_some_and(u8::is_ascii_alphanumeric)
    || !bucket
      .as_bytes()
      .last()
      .is_some_and(u8::is_ascii_alphanumeric)
    || region.is_empty()
    || region.len() > 128
    || prefix.len() > 512
    || !prefix
      .bytes()
      .all(|byte| byte.is_ascii_alphanumeric() || b"!_.*'()/-".contains(&byte))
  {
    return Err(ArtifactPolicyError::Invalid(
      "the S3 bucket, region, or prefix is invalid".to_string(),
    ));
  }
  let endpoint = endpoint
    .map(str::to_string)
    .unwrap_or_else(|| format!("https://s3.{region}.amazonaws.com"));
  let endpoint = Url::parse(&endpoint)
    .map_err(|_| ArtifactPolicyError::Invalid("the S3 endpoint is not a valid URL".to_string()))?;
  if endpoint.scheme() != "https"
    || endpoint.username() != ""
    || endpoint.password().is_some()
    || endpoint.query().is_some()
    || endpoint.fragment().is_some()
  {
    return Err(ArtifactPolicyError::Invalid(
      "the S3 endpoint must be a credential-free HTTPS URL".to_string(),
    ));
  }
  let authentication: AuthoredS3Authentication = serde_json::from_value(authentication.clone())
    .map_err(|_| {
      ArtifactPolicyError::Invalid("the S3 authentication contract is invalid".to_string())
    })?;
  let secret = |name: &str| {
    resolved_secrets
      .get(name)
      .filter(|value| !value.is_empty() && value.len() <= 16_384)
      .cloned()
      .ok_or_else(|| {
        ArtifactPolicyError::Invalid(format!("the S3 credential secret {name} is unavailable"))
      })
  };
  let credentials = match authentication {
    AuthoredS3Authentication::Ambient => S3Credentials {
      access_key: std::env::var("AWS_ACCESS_KEY_ID").map_err(|_| {
        ArtifactPolicyError::Invalid("ambient AWS_ACCESS_KEY_ID is unavailable".to_string())
      })?,
      secret_key: std::env::var("AWS_SECRET_ACCESS_KEY").map_err(|_| {
        ArtifactPolicyError::Invalid("ambient AWS_SECRET_ACCESS_KEY is unavailable".to_string())
      })?,
      session_token: std::env::var("AWS_SESSION_TOKEN").ok(),
    },
    AuthoredS3Authentication::Secrets {
      access_key_secret,
      secret_key_secret,
      session_token_secret,
    } => S3Credentials {
      access_key: secret(&access_key_secret)?,
      secret_key: secret(&secret_key_secret)?,
      session_token: session_token_secret.as_deref().map(secret).transpose()?,
    },
  };
  if credentials.access_key.is_empty() || credentials.secret_key.is_empty() {
    return Err(ArtifactPolicyError::Invalid(
      "the S3 credentials are empty".to_string(),
    ));
  }
  let encryption: AuthoredS3Encryption = serde_json::from_value(server_side_encryption.clone())
    .map_err(|_| {
      ArtifactPolicyError::Invalid("the S3 encryption contract is invalid".to_string())
    })?;
  let server_side_encryption = match encryption {
    AuthoredS3Encryption::Aes256 => S3ServerSideEncryption::Aes256,
    AuthoredS3Encryption::AwsKms { key_id } if !key_id.is_empty() && key_id.len() <= 2_048 => {
      S3ServerSideEncryption::AwsKms { key_id }
    }
    AuthoredS3Encryption::AwsKms { .. } => {
      return Err(ArtifactPolicyError::Invalid(
        "the S3 KMS key ID is invalid".to_string(),
      ))
    }
  };
  Ok(PreparedS3Backend {
    bucket: bucket.to_string(),
    region: region.to_string(),
    prefix: prefix.to_string(),
    endpoint,
    force_path_style,
    credentials,
    server_side_encryption,
  })
}

fn validate_policy_limits(policy: &ArtifactPolicyV1) -> Result<(), ArtifactPolicyError> {
  let limits = &policy.limits;
  let retention = &policy.retention;
  if policy.read_roots.is_empty()
    || policy.read_roots.len() > 128
    || limits.max_artifact_bytes == 0
    || limits.max_artifact_bytes > MAX_ARTIFACT_BYTES
    || limits.max_run_bytes < limits.max_artifact_bytes
    || limits.max_run_bytes > MAX_RUN_BYTES
    || limits.max_store_bytes < limits.max_artifact_bytes
    || limits.max_store_bytes > MAX_STORE_BYTES
    || !(1..=64).contains(&limits.max_concurrent_transfers)
    || !(1_000..=MAX_TRANSFER_TIMEOUT_MS).contains(&limits.transfer_timeout_ms)
    || retention.minimum_ms < 3_600_000
    || retention.minimum_ms > retention.default_ms
    || retention.default_ms > retention.maximum_ms
    || retention.maximum_ms > 31_536_000_000
    || policy.operational_output.as_ref().is_some_and(|output| {
      !output.enabled
        || output.retention_ms < retention.minimum_ms
        || output.retention_ms > retention.maximum_ms
    })
    || policy.secret_scanning.minimum_secret_bytes != 8
    || !policy.materialization.cleanup_on_start
    || policy.materialization.max_bytes_per_attempt == 0
    || policy.materialization.max_bytes_per_attempt > MAX_RUN_BYTES
  {
    return Err(ArtifactPolicyError::Invalid(
      "the policy contains inconsistent or out-of-range limits".to_string(),
    ));
  }
  Ok(())
}
