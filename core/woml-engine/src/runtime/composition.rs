//! Explicit built-in capability composition and durable authority attachment.
//!
//! Registration order and resource ownership follow the runtime constructor.
//! Optional process/artifact authority is attached only by the reviewed options
//! methods; the generic capability registry does not choose runtime policy.

use super::{RuntimeExecutionError, RuntimeExecutionOptions, REVIEWER_AUTHORITIES};
use crate::CapabilityRegistry;
use crate::DurableCapabilityAuthority;
use crate::DurableEventStore;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::sync::RwLock;

impl RuntimeExecutionOptions {
  pub fn with_capability_registry(mut self, registry: Arc<CapabilityRegistry>) -> Self {
    self.capability_registry = registry;
    self.managed_database_pool = None;
    self.managed_storage_store = None;
    self.managed_cache_store = None;
    self.managed_durable_state_store = None;
    self.managed_locks_service = None;
    self.managed_artifact_store = None;
    self.managed_artifact_put_handler = None;
    self.managed_process_handler = None;
    self.operational_output_store = None;
    self.artifact_policy = None;
    self
  }
  /// Binds worker-side capability calls to a durable authority owned by the
  /// worker journal database. Remote attempt runtimes use this narrow accessor
  /// so they share the exact same policy, idempotency, and cancellation path as
  /// local script execution without gaining workflow scheduling authority.
  pub fn prepare_remote_worker_capabilities(
    self,
    database_path: &Path,
  ) -> Result<Self, RuntimeExecutionError> {
    attach_durable_capability_authority(self, database_path)
  }
  pub fn remote_worker_capability_authority(&self) -> Option<Arc<DurableCapabilityAuthority>> {
    self.capability_authority.clone()
  }
  /// Enables the managed process capability for this runtime only after the
  /// runtime-owned Process Policy v1 has been validated by the Rust core.
  pub fn with_process_policy(
    mut self,
    policy: crate::ProcessPolicyV1,
    mode: crate::ProcessRuntimeMode,
  ) -> Result<Self, crate::ProcessPolicyError> {
    let handler = Arc::new(crate::ManagedProcessHandler::new_with_secrets(
      policy,
      mode,
      Arc::clone(&self.resolved_secrets),
    )?);
    if let Some(artifacts) = &self.managed_artifact_put_handler {
      handler.configure_output_artifacts(Arc::clone(artifacts))?;
    }
    self
      .capability_registry
      .register(handler.clone())
      .map_err(|error| crate::ProcessPolicyError::Invalid(error.to_string()))?;
    self.managed_process_handler = Some(handler);
    Ok(self)
  }
  /// Enables durable artifacts only after Rust validates Artifact Policy v1.
  pub fn with_artifact_policy(
    mut self,
    policy: crate::ArtifactPolicyV1,
    mode: crate::ArtifactRuntimeMode,
  ) -> Result<Self, crate::ArtifactPolicyError> {
    let store = Arc::new(crate::ManagedArtifactStore::default());
    let handler = Arc::new(crate::ManagedArtifactPutHandler::new(
      Arc::clone(&store),
      policy,
      mode,
      &self.resolved_secrets,
    )?);
    let head_handler = handler.head_handler();
    let materialize_handler = handler.materialize_handler();
    self.artifact_policy = Some(handler.policy());
    if let Some(process) = &self.managed_process_handler {
      process
        .configure_output_artifacts(Arc::clone(&handler))
        .map_err(|error| crate::ArtifactPolicyError::Invalid(error.to_string()))?;
    }
    self
      .capability_registry
      .register(handler.clone())
      .map_err(|error| crate::ArtifactPolicyError::Invalid(error.to_string()))?;
    self
      .capability_registry
      .register(Arc::new(head_handler))
      .map_err(|error| crate::ArtifactPolicyError::Invalid(error.to_string()))?;
    self
      .capability_registry
      .register(Arc::new(materialize_handler))
      .map_err(|error| crate::ArtifactPolicyError::Invalid(error.to_string()))?;
    self.managed_artifact_store = Some(store);
    self.managed_artifact_put_handler = Some(handler);
    Ok(self)
  }
}

pub(super) fn attach_durable_capability_authority(
  mut options: RuntimeExecutionOptions,
  database_path: &std::path::Path,
) -> Result<RuntimeExecutionOptions, RuntimeExecutionError> {
  let operational_output = Arc::new(
    crate::operational_output_store_for_state(database_path)
      .map(|store| store.with_reporter(options.operational_output_reporter.clone()))
      .map_err(|error| {
        RuntimeExecutionError::InvalidConfiguration(format!(
          "{}: the operational-output path could not be derived",
          error.code()
        ))
      })?,
  );
  if let Some(process) = &options.managed_process_handler {
    process
      .configure_output_store(Arc::clone(&operational_output))
      .map_err(|error| RuntimeExecutionError::InvalidConfiguration(error.to_string()))?;
  }
  options.operational_output_store = Some(Arc::clone(&operational_output));
  if let Some(authority) = options.reviewer_authority() {
    REVIEWER_AUTHORITIES
      .get_or_init(|| RwLock::new(HashMap::new()))
      .write()
      .map_err(|_| {
        RuntimeExecutionError::InvalidConfiguration(
          "WOML_APPROVAL_AUTHORITY_UNAVAILABLE: reviewer authority registry is unavailable"
            .to_string(),
        )
      })?
      .insert(database_path.to_path_buf(), authority);
  }
  if let Some(storage) = &options.managed_storage_store {
    storage
      .configure_for_state(database_path)
      .map_err(|failure| RuntimeExecutionError::InvalidConfiguration(failure.message))?;
  }
  if let Some(cache) = &options.managed_cache_store {
    cache
      .configure_for_state(database_path)
      .map_err(|failure| RuntimeExecutionError::InvalidConfiguration(failure.message))?;
  }
  if let Some(state) = &options.managed_durable_state_store {
    state
      .configure_for_state(database_path)
      .map_err(|failure| RuntimeExecutionError::InvalidConfiguration(failure.message))?;
  }
  if let Some(locks) = &options.managed_locks_service {
    locks
      .configure_for_state(database_path)
      .map_err(|failure| RuntimeExecutionError::InvalidConfiguration(failure.message))?;
  }
  if let (Some(artifacts), Some(policy)) =
    (&options.managed_artifact_store, &options.artifact_policy)
  {
    artifacts
      .configure_for_state(database_path, Arc::clone(policy))
      .map_err(|failure| RuntimeExecutionError::InvalidConfiguration(failure.message))?;
  }
  if let Some(pool) = &options.managed_database_pool {
    pool
      .protect_path(database_path)
      .map_err(|failure| RuntimeExecutionError::InvalidConfiguration(failure.message))?;
  }
  let store = DurableEventStore::open(database_path.to_path_buf())?;
  if let Some(pool) = &options.managed_database_pool {
    // Protect the post-open canonical identity as well as the pre-open lexical
    // path so a state path created during this call cannot gain an alias.
    pool
      .protect_path(database_path)
      .map_err(|failure| RuntimeExecutionError::InvalidConfiguration(failure.message))?;
  }
  let locks = options
    .managed_locks_service
    .clone()
    .unwrap_or_else(|| Arc::new(crate::ManagedLocksService::default()));
  options.capability_authority = Some(Arc::new(
    DurableCapabilityAuthority::new_with_infrastructure_policy_authorities(
      Arc::clone(&options.capability_registry),
      Arc::new(tokio::sync::Mutex::new(store)),
      locks,
      options.infrastructure_policy_required || options.infrastructure_policy.is_some(),
      options.managed_artifact_store.clone(),
      options.emergency_deny_overlay.clone(),
    )
    .with_operational_output(operational_output, Arc::clone(&options.resolved_secrets)),
  ));
  Ok(options)
}

pub(super) fn register_builtin_capabilities(
  capability_registry: &Arc<CapabilityRegistry>,
  managed_storage_store: &Arc<crate::ManagedStorageStore>,
  telemetry_propagation: &Arc<AtomicBool>,
  telemetry_sampling_ratio: &Arc<RwLock<f64>>,
) -> BuiltinCapabilityResources {
  capability_registry
    .register(Arc::new(
      crate::ManagedHttpHandler::with_storage_and_propagation(
        Arc::clone(managed_storage_store),
        Arc::clone(telemetry_propagation),
        Arc::clone(telemetry_sampling_ratio),
      ),
    ))
    .expect("the production HTTP capability is registered exactly once");
  capability_registry
    .register(Arc::new(crate::ManagedTelegramHandler::default()))
    .expect("the production Telegram capability is registered exactly once");
  capability_registry
    .register(Arc::new(crate::ManagedDiscordHandler::default()))
    .expect("the production Discord capability is registered exactly once");
  capability_registry
    .register(Arc::new(crate::ManagedWhatsAppHandler::default()))
    .expect("the production WhatsApp capability is registered exactly once");
  let managed_database_pool = Arc::new(crate::ManagedDatabasePool::default());
  for handler in crate::ManagedDatabaseHandler::handlers(Arc::clone(&managed_database_pool)) {
    capability_registry
      .register(handler)
      .expect("each production database operation is registered exactly once");
  }
  for handler in crate::ManagedStorageHandler::handlers(Arc::clone(managed_storage_store)) {
    capability_registry
      .register(handler)
      .expect("each production storage operation is registered exactly once");
  }
  let managed_cache_store = Arc::new(crate::ManagedCacheStore::default());
  for handler in crate::ManagedCacheHandler::handlers(Arc::clone(&managed_cache_store)) {
    capability_registry
      .register(handler)
      .expect("each production cache operation is registered exactly once");
  }
  let managed_durable_state_store = Arc::new(crate::ManagedDurableStateStore::default());
  for handler in
    crate::ManagedDurableStateHandler::handlers(Arc::clone(&managed_durable_state_store))
  {
    capability_registry
      .register(handler)
      .expect("each production durable state operation is registered exactly once");
  }
  let managed_locks_service = Arc::new(crate::ManagedLocksService::default());
  for handler in crate::ManagedLocksService::handlers(Arc::clone(&managed_locks_service)) {
    capability_registry
      .register(handler)
      .expect("each production lock operation is registered exactly once");
  }
  BuiltinCapabilityResources {
    managed_database_pool,
    managed_cache_store,
    managed_durable_state_store,
    managed_locks_service,
  }
}

pub(super) struct BuiltinCapabilityResources {
  pub(super) managed_database_pool: Arc<crate::ManagedDatabasePool>,
  pub(super) managed_cache_store: Arc<crate::ManagedCacheStore>,
  pub(super) managed_durable_state_store: Arc<crate::ManagedDurableStateStore>,
  pub(super) managed_locks_service: Arc<crate::ManagedLocksService>,
}
