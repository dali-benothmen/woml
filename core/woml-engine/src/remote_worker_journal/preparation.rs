//! Private remote worker journal preparation.

use crate::derive_remote_dispatch_id;
use crate::event::is_definition_hash;
use crate::model::ValueExpression;
use crate::protocol::ScriptAttempt;
use crate::remote_dispatch::RemoteDispatchRequestV1;
use crate::remote_protocol::RemoteCoordinatorAuthorityV1;
use crate::remote_protocol::RemoteDispatchPackageV1;
use crate::remote_protocol::RemoteEffectiveAuthorityV1;
use crate::remote_protocol::REMOTE_DISPATCH_PACKAGE_PROFILE;
use crate::remote_protocol::REMOTE_DISPATCH_PACKAGE_VERSION;
use crate::remote_worker_journal::artifact_grant_digest;
use crate::remote_worker_journal::valid_secret_name;
use crate::remote_worker_journal::validate_artifact_grants;
use crate::remote_worker_journal::PreparedRemoteDispatchV1;
use crate::remote_worker_journal::RemoteWorkerAuthorityV1;
use crate::remote_worker_journal::RemoteWorkerError;
use chrono::DateTime;
use chrono::Utc;
use serde_json_canonicalizer::to_vec as canonical_json;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

/// Validates package integrity and computes the authority intersection that is
/// persisted by `WorkerJournalV1::accept` before any script-host process starts.
pub fn prepare_remote_dispatch(
  package: RemoteDispatchPackageV1,
  coordinator: &RemoteCoordinatorAuthorityV1,
  worker_id: &str,
  worker_target: &str,
  worker: &RemoteWorkerAuthorityV1,
  now: DateTime<Utc>,
) -> Result<PreparedRemoteDispatchV1, RemoteWorkerError> {
  let encoded = canonical_json(&package).map_err(|_| RemoteWorkerError::InvalidPackage)?;
  if encoded.len() > crate::WORKER_PROTOCOL_MAX_MESSAGE_BYTES
    || package.profile != REMOTE_DISPATCH_PACKAGE_PROFILE
    || package.version != REMOTE_DISPATCH_PACKAGE_VERSION
    || package.node_id != package.node.id
    || package.node.handler != "runtime.script"
    || package.target != worker_target
    || package.deadline <= now
    || !is_definition_hash(&package.definition_package_root_hash)
    || !is_definition_hash(&package.target_registry_digest)
    || !is_definition_hash(&worker.policy_digest)
    || !is_definition_hash(&coordinator.policy_digest)
    || package
      .infrastructure_policy_digest
      .as_ref()
      .is_some_and(|digest| !is_definition_hash(digest) || digest != &coordinator.policy_digest)
    || package.package_digest
      != package
        .computed_digest()
        .map_err(|_| RemoteWorkerError::InvalidPackage)?
  {
    return Err(if package.deadline <= now {
      RemoteWorkerError::Expired
    } else {
      RemoteWorkerError::InvalidPackage
    });
  }
  let request = RemoteDispatchRequestV1 {
    run_id: package.run_id.clone(),
    node_id: package.node_id.clone(),
    iteration_identity: package.iteration_key.clone().unwrap_or_default(),
    attempt: package.attempt,
    invocation_id: package.invocation_id.clone(),
    target: package.target.clone(),
    target_digest: package.target_registry_digest.clone(),
    package_digest: package.package_digest.clone(),
    requested_at: now,
  };
  if derive_remote_dispatch_id(&request).map_err(|_| RemoteWorkerError::InvalidPackage)?
    != package.dispatch_id
  {
    return Err(RemoteWorkerError::InvalidPackage);
  }
  ScriptAttempt::new(
    package.attempt,
    maximum_attempts(&package),
    &package.package_digest,
  )
  .map_err(|_| RemoteWorkerError::InvalidPackage)?;
  let execution = package
    .node
    .execution
    .as_ref()
    .ok_or(RemoteWorkerError::InvalidPackage)?;
  if execution.profile_version != 1
    || execution.target != package.target
    || execution.required_capabilities != package.required_capabilities
    || package.required_capabilities.first().map(String::as_str) != Some("runtime.script")
  {
    return Err(RemoteWorkerError::InvalidPackage);
  }
  if script_source(&package.node).is_none() {
    return Err(RemoteWorkerError::InvalidPackage);
  }
  let script_runtime = package
    .node
    .script_runtime
    .as_ref()
    .ok_or(RemoteWorkerError::InvalidPackage)?;
  if script_runtime.binding_version != 1
    || script_runtime
      .bindings
      .iter()
      .map(String::as_str)
      .ne(["context", "attempt", "services", "secrets"])
  {
    return Err(RemoteWorkerError::InvalidPackage);
  }
  let requested = package
    .required_capabilities
    .iter()
    .map(|id| {
      capability_version(id)
        .map(|version| (id.clone(), version))
        .ok_or(RemoteWorkerError::CapabilityUnavailable)
    })
    .collect::<Result<BTreeMap<_, _>, _>>()?;
  if requested.len() != package.required_capabilities.len()
    || requested.get("runtime.script") != Some(&1)
  {
    return Err(RemoteWorkerError::CapabilityUnavailable);
  }
  let coordinator_capabilities: BTreeMap<_, _> = coordinator
    .allowed_capabilities
    .iter()
    .map(|capability| (capability.id.clone(), capability.version))
    .collect();
  if coordinator_capabilities.len() != coordinator.allowed_capabilities.len() {
    return Err(RemoteWorkerError::CapabilityUnavailable);
  }
  for (id, version) in &requested {
    if coordinator_capabilities.get(id) != Some(version)
      || worker.allowed_capabilities.get(id) != Some(version)
    {
      return Err(RemoteWorkerError::CapabilityUnavailable);
    }
  }
  let required_secrets: BTreeSet<_> = package.required_secrets.iter().cloned().collect();
  let coordinator_secrets: BTreeSet<_> = coordinator.allowed_secret_names.iter().cloned().collect();
  if required_secrets.len() != package.required_secrets.len()
    || coordinator_secrets.len() != coordinator.allowed_secret_names.len()
    || required_secrets.iter().any(|name| !valid_secret_name(name))
    || !required_secrets.is_subset(&coordinator_secrets)
    || !required_secrets.is_subset(&worker.allowed_secret_names)
    || script_runtime
      .required_secrets
      .iter()
      .cloned()
      .collect::<BTreeSet<_>>()
      != required_secrets
  {
    return Err(RemoteWorkerError::SecretUnavailable);
  }
  validate_artifact_grants(&package.artifact_grants, &package, now)?;
  let mut effective = RemoteEffectiveAuthorityV1 {
    coordinator_policy_digest: coordinator.policy_digest.clone(),
    worker_policy_digest: worker.policy_digest.clone(),
    capabilities: requested
      .into_iter()
      .map(|(id, version)| crate::WorkerCapabilityV1 { id, version })
      .collect(),
    secret_names: package.required_secrets.clone(),
    artifact_grant_ids: package
      .artifact_grants
      .iter()
      .map(artifact_grant_digest)
      .collect::<Result<Vec<_>, _>>()?,
    authority_digest: String::new(),
  };
  effective
    .capabilities
    .sort_by(|left, right| left.id.cmp(&right.id));
  effective.secret_names.sort();
  effective.artifact_grant_ids.sort();
  effective.authority_digest = effective
    .computed_digest()
    .map_err(|_| RemoteWorkerError::InvalidPackage)?;
  Ok(PreparedRemoteDispatchV1 {
    package,
    effective_authority: effective,
    worker_id: worker_id.to_string(),
  })
}

pub(in crate::remote_worker_journal) fn script_source(
  node: &crate::CompiledWorkflowNode,
) -> Option<&str> {
  let ValueExpression::Object { fields } = &node.inputs else {
    return None;
  };
  let ValueExpression::Literal { value } = fields.get("source")? else {
    return None;
  };
  value.as_str()
}

pub(in crate::remote_worker_journal) fn valid_bound_id(value: &str) -> bool {
  !value.is_empty()
    && value.len() <= 256
    && value
      .bytes()
      .next()
      .is_some_and(|byte| byte.is_ascii_alphanumeric())
    && value
      .bytes()
      .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'-'))
}

fn capability_version(id: &str) -> Option<u32> {
  match id {
    "runtime.script" | "services.artifacts" | "operational.output" => Some(1),
    "services.process" => Some(2),
    _ => None,
  }
}

pub(in crate::remote_worker_journal) fn maximum_attempts(package: &RemoteDispatchPackageV1) -> u32 {
  package
    .node
    .retry_policy
    .as_ref()
    .map_or(1, |policy| policy.max_attempts)
}
