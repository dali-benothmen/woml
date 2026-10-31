//! Operator policy preparation, approved roots/environment/executables, mode checks, and shared prepared authority.

use crate::process::canonical_directory;
use crate::process::validate_alias;
use crate::process::validate_environment_name;
use crate::process::validate_limits;
use crate::process::verify_executable_policy;
use crate::process::ExecutableIdentity;
use crate::process::ProcessPolicyError;
use crate::process::ProcessPolicyLimits;
use crate::process::ProcessPolicyV1;
use crate::process::ProcessRuntimeMode;
use crate::process::PROCESS_POLICY_CONTRACT;
use crate::process::PROCESS_POLICY_CONTRACT_VERSION;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug)]
pub(in crate::process) struct PreparedProcessPolicy {
  pub(in crate::process) project_root: PathBuf,
  pub(in crate::process) executables: HashMap<String, ApprovedExecutable>,
  pub(in crate::process) working_directories: Vec<PathBuf>,
  pub(in crate::process) inherited_environment: BTreeMap<String, String>,
  pub(in crate::process) allowed_environment: BTreeSet<String>,
  pub(in crate::process) limits: ProcessPolicyLimits,
}

#[derive(Debug)]
pub(in crate::process) struct ApprovedExecutable {
  pub(in crate::process) path: PathBuf,
  pub(in crate::process) identity: ExecutableIdentity,
}

impl PreparedProcessPolicy {
  pub(in crate::process) fn prepare(
    policy: ProcessPolicyV1,
    mode: ProcessRuntimeMode,
  ) -> Result<Self, ProcessPolicyError> {
    if policy.contract != PROCESS_POLICY_CONTRACT
      || policy.contract_version != PROCESS_POLICY_CONTRACT_VERSION
    {
      return Err(ProcessPolicyError::Invalid(
        "the policy does not match Process Policy v1".to_string(),
      ));
    }
    if !policy.enabled {
      return Err(ProcessPolicyError::Disabled);
    }
    let allowed = match mode {
      ProcessRuntimeMode::Foreground => policy.modes.foreground,
      ProcessRuntimeMode::Background => policy.modes.background,
    };
    if !allowed {
      return Err(ProcessPolicyError::ModeDenied(match mode {
        ProcessRuntimeMode::Foreground => "foreground",
        ProcessRuntimeMode::Background => "background",
      }));
    }
    validate_limits(&policy.limits)?;
    if policy.project_root.is_empty()
      || policy.project_root.len() > 4096
      || policy.executables.len() > 128
      || policy.working_directories.len() > 128
      || policy.environment.inherit.len() > 128
      || policy.environment.allow.len() > 128
    {
      return Err(ProcessPolicyError::Invalid(
        "the policy exceeds Process Policy v1 collection or path bounds".to_string(),
      ));
    }
    let project_root = canonical_directory(Path::new(&policy.project_root), "project root")?;
    if policy.working_directories.is_empty() {
      return Err(ProcessPolicyError::Invalid(
        "at least one working-directory root is required".to_string(),
      ));
    }
    let mut working_directories = Vec::with_capacity(policy.working_directories.len());
    let mut authored_working_directories = BTreeSet::new();
    for value in &policy.working_directories {
      if value.is_empty()
        || value.len() > 4096
        || !authored_working_directories.insert(value.clone())
      {
        return Err(ProcessPolicyError::Invalid(
          "working-directory roots must be unique non-empty paths within the v1 size bound"
            .to_string(),
        ));
      }
      let candidate = Path::new(value);
      let path = if candidate.is_absolute() {
        candidate.to_path_buf()
      } else {
        project_root.join(candidate)
      };
      let canonical = canonical_directory(&path, "working-directory root")?;
      if !canonical.starts_with(&project_root) {
        return Err(ProcessPolicyError::Invalid(
          "a working-directory root escapes the project root".to_string(),
        ));
      }
      if !working_directories.contains(&canonical) {
        working_directories.push(canonical);
      }
    }
    let mut executables = HashMap::new();
    for entry in &policy.executables {
      validate_alias(&entry.alias)?;
      if entry.path.is_empty() || entry.path.len() > 4096 {
        return Err(ProcessPolicyError::Invalid(format!(
          "executable alias '{}' has an invalid path",
          entry.alias
        )));
      }
      let canonical = std::fs::canonicalize(&entry.path).map_err(|_| {
        ProcessPolicyError::Invalid(format!(
          "executable alias '{}' points to a missing path",
          entry.alias
        ))
      })?;
      let identity = verify_executable_policy(&canonical, &entry.alias)?;
      if executables
        .insert(
          entry.alias.clone(),
          ApprovedExecutable {
            path: canonical,
            identity,
          },
        )
        .is_some()
      {
        return Err(ProcessPolicyError::Invalid(format!(
          "executable alias '{}' is duplicated",
          entry.alias
        )));
      }
    }
    let mut inherited_environment = BTreeMap::new();
    let mut inherited_names = BTreeSet::new();
    for name in &policy.environment.inherit {
      validate_environment_name(name).map_err(ProcessPolicyError::Invalid)?;
      if !inherited_names.insert(name.clone()) {
        return Err(ProcessPolicyError::Invalid(
          "inherited environment-variable names must be unique".to_string(),
        ));
      }
      if let Ok(value) = std::env::var(name) {
        inherited_environment.insert(name.clone(), value);
      }
    }
    let mut allowed_environment = BTreeSet::new();
    for name in &policy.environment.allow {
      validate_environment_name(name).map_err(ProcessPolicyError::Invalid)?;
      if !allowed_environment.insert(name.clone()) {
        return Err(ProcessPolicyError::Invalid(
          "explicit environment-variable names must be unique".to_string(),
        ));
      }
    }
    Ok(Self {
      project_root,
      executables,
      working_directories,
      inherited_environment,
      allowed_environment,
      limits: policy.limits,
    })
  }
}
