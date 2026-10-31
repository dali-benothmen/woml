//! runtime options.

use super::{
  register_builtin_capabilities, BuiltinCapabilityResources, EngineClock,
  ExecutionProgressReporter, LifecycleProgressReporter, PolicyExecutionCoordinator,
  PolicyExecutionRegistry, RuntimeModuleArtifact, RuntimePolicyProgressReporter,
  SharedScriptHostPool, SystemEngineClock,
};
use crate::interval::IntervalProgressReporter;
use crate::schedule::ScheduleClock;
use crate::schedule::ScheduleProgressReporter;
use crate::schedule::SystemScheduleClock;
use crate::workflow_calls::WorkflowCallProgressReporter;
use crate::CapabilityRegistry;
use crate::DurableCapabilityAuthority;
use crate::NotificationHostProcessOptions;
use crate::ReviewerDirectoryAuthorityV1;
use crate::ScriptHostModuleArtifact;
use crate::ScriptHostProcessOptions;
use crate::INFORMATIONAL_NOTIFICATION_PROVIDER_PROTOCOL_VERSION;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::RwLock;

#[derive(Clone)]
pub struct RuntimeExecutionOptions {
  pub script_host: ScriptHostProcessOptions,
  pub notification_host: Option<NotificationHostProcessOptions>,
  pub custom_notification_host_path: Option<PathBuf>,
  pub script_timeout_ms: u64,
  pub max_context_bytes: Option<usize>,
  pub clock: Arc<dyn EngineClock>,
  pub progress_reporter: Option<ExecutionProgressReporter>,
  pub lifecycle_progress_reporter: Option<LifecycleProgressReporter>,
  pub schedule_clock: Arc<dyn ScheduleClock>,
  pub schedule_progress_reporter: Option<ScheduleProgressReporter>,
  pub interval_progress_reporter: Option<IntervalProgressReporter>,
  pub workflow_call_progress_reporter: Option<WorkflowCallProgressReporter>,
  pub runtime_policy_progress_reporter: Option<RuntimePolicyProgressReporter>,
  pub operational_output_reporter: Option<crate::OperationalOutputReporter>,
  pub telemetry_reporter: Option<crate::TelemetrySignalReporter>,
  pub telemetry_context: Option<crate::TelemetrySignalContext>,
  pub telemetry_admission_link_reporter: Option<crate::TelemetryAdmissionLinkReporter>,
  pub telemetry_propagation: Arc<AtomicBool>,
  pub telemetry_sampling_ratio: Arc<RwLock<f64>>,
  pub resolved_secrets: Arc<BTreeMap<String, String>>,
  pub(crate) signal_ingress_token: Option<String>,
  pub capability_registry: Arc<CapabilityRegistry>,
  pub runtime_modules: Arc<Vec<RuntimeModuleArtifact>>,
  pub(super) shared_script_hosts: Option<Arc<SharedScriptHostPool>>,
  pub(super) capability_authority: Option<Arc<DurableCapabilityAuthority>>,
  pub(super) managed_database_pool: Option<Arc<crate::ManagedDatabasePool>>,
  pub(super) managed_storage_store: Option<Arc<crate::ManagedStorageStore>>,
  pub(super) managed_cache_store: Option<Arc<crate::ManagedCacheStore>>,
  pub(super) managed_durable_state_store: Option<Arc<crate::ManagedDurableStateStore>>,
  pub(super) managed_locks_service: Option<Arc<crate::ManagedLocksService>>,
  pub(super) managed_artifact_store: Option<Arc<crate::ManagedArtifactStore>>,
  pub(super) managed_artifact_put_handler: Option<Arc<crate::ManagedArtifactPutHandler>>,
  pub(super) managed_process_handler: Option<Arc<crate::ManagedProcessHandler>>,
  pub(super) operational_output_store: Option<Arc<crate::LazyOperationalOutputStore>>,
  pub(super) artifact_policy: Option<Arc<crate::artifacts::PreparedArtifactPolicy>>,
  pub(super) infrastructure_policy: Option<Arc<crate::PreparedInfrastructurePolicyV1>>,
  pub(super) emergency_deny_overlay: Option<Arc<crate::PreparedInfrastructurePolicyV1>>,
  pub(super) infrastructure_policy_required: bool,
  pub(super) reviewer_authority: Option<Arc<ReviewerDirectoryAuthorityV1>>,
  pub(super) policy_execution: Option<Arc<PolicyExecutionCoordinator>>,
  pub(super) policy_execution_registry: PolicyExecutionRegistry,
  pub(super) lock_renewals: Arc<std::sync::Mutex<HashMap<String, Arc<AtomicBool>>>>,
}

impl std::fmt::Debug for RuntimeExecutionOptions {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    formatter
      .debug_struct("RuntimeExecutionOptions")
      .field("script_host", &self.script_host)
      .field("script_timeout_ms", &self.script_timeout_ms)
      .field("notification_host", &self.notification_host)
      .field(
        "custom_notification_host_path",
        &self.custom_notification_host_path,
      )
      .field("max_context_bytes", &self.max_context_bytes)
      .field("clock", &"dyn EngineClock")
      .field("schedule_clock", &"dyn ScheduleClock")
      .field("resolved_secret_count", &self.resolved_secrets.len())
      .field("capability_registry", &"CapabilityRegistry")
      .field("runtime_module_count", &self.runtime_modules.len())
      .field(
        "infrastructure_policy_digest",
        &self
          .infrastructure_policy
          .as_ref()
          .map(|policy| &policy.digest),
      )
      .field(
        "infrastructure_policy_required",
        &self.infrastructure_policy_required,
      )
      .field(
        "emergency_deny_overlay_digest",
        &self
          .emergency_deny_overlay
          .as_ref()
          .map(|policy| &policy.digest),
      )
      .field(
        "shared_script_hosts",
        &self.shared_script_hosts.as_ref().map(|_| "enabled"),
      )
      .field(
        "lock_renewal_count",
        &self
          .lock_renewals
          .lock()
          .map(|items| items.len())
          .unwrap_or(0),
      )
      .field(
        "progress_reporter",
        &self.progress_reporter.as_ref().map(|_| "configured"),
      )
      .field(
        "lifecycle_progress_reporter",
        &self
          .lifecycle_progress_reporter
          .as_ref()
          .map(|_| "configured"),
      )
      .field(
        "schedule_progress_reporter",
        &self
          .schedule_progress_reporter
          .as_ref()
          .map(|_| "configured"),
      )
      .field(
        "interval_progress_reporter",
        &self
          .interval_progress_reporter
          .as_ref()
          .map(|_| "configured"),
      )
      .field(
        "workflow_call_progress_reporter",
        &self
          .workflow_call_progress_reporter
          .as_ref()
          .map(|_| "configured"),
      )
      .field(
        "telemetry_reporter",
        &self.telemetry_reporter.as_ref().map(|_| "configured"),
      )
      .field(
        "telemetry_context",
        &self.telemetry_context.as_ref().map(|_| "configured"),
      )
      .finish()
  }
}

impl RuntimeExecutionOptions {
  pub fn new(script_host: ScriptHostProcessOptions, script_timeout_ms: u64) -> Self {
    let notification_host_path = script_host.host_script_path.parent().map(|parent| {
      let extension = script_host
        .host_script_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("js");
      parent.join(format!("notification-provider-host.{extension}"))
    });
    let notification_host = notification_host_path.map(|path| {
      NotificationHostProcessOptions::new(script_host.bun_executable.clone(), path)
        .with_protocol_version(INFORMATIONAL_NOTIFICATION_PROVIDER_PROTOCOL_VERSION)
    });
    let custom_notification_host_path = script_host.host_script_path.parent().map(|parent| {
      let extension = script_host
        .host_script_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("js");
      parent.join(format!("custom-notification-provider-host.{extension}"))
    });
    let capability_registry = Arc::new(CapabilityRegistry::default());
    let managed_storage_store = Arc::new(crate::ManagedStorageStore::default());
    let telemetry_propagation = Arc::new(AtomicBool::new(false));
    let telemetry_sampling_ratio = Arc::new(RwLock::new(0.0));
    let BuiltinCapabilityResources {
      managed_database_pool,
      managed_cache_store,
      managed_durable_state_store,
      managed_locks_service,
    } = register_builtin_capabilities(
      &capability_registry,
      &managed_storage_store,
      &telemetry_propagation,
      &telemetry_sampling_ratio,
    );
    Self {
      script_host,
      notification_host,
      custom_notification_host_path,
      script_timeout_ms,
      max_context_bytes: None,
      clock: Arc::new(SystemEngineClock),
      progress_reporter: None,
      lifecycle_progress_reporter: None,
      schedule_clock: Arc::new(SystemScheduleClock),
      schedule_progress_reporter: None,
      interval_progress_reporter: None,
      workflow_call_progress_reporter: None,
      runtime_policy_progress_reporter: None,
      operational_output_reporter: None,
      telemetry_reporter: None,
      telemetry_context: None,
      telemetry_admission_link_reporter: None,
      telemetry_propagation,
      telemetry_sampling_ratio,
      resolved_secrets: Arc::new(BTreeMap::new()),
      signal_ingress_token: None,
      capability_registry,
      runtime_modules: Arc::new(Vec::new()),
      shared_script_hosts: None,
      capability_authority: None,
      managed_database_pool: Some(managed_database_pool),
      managed_storage_store: Some(managed_storage_store),
      managed_cache_store: Some(managed_cache_store),
      managed_durable_state_store: Some(managed_durable_state_store),
      managed_locks_service: Some(managed_locks_service),
      managed_artifact_store: None,
      managed_artifact_put_handler: None,
      managed_process_handler: None,
      operational_output_store: None,
      artifact_policy: None,
      infrastructure_policy: None,
      emergency_deny_overlay: None,
      infrastructure_policy_required: false,
      reviewer_authority: None,
      policy_execution: None,
      policy_execution_registry: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
      lock_renewals: Arc::new(std::sync::Mutex::new(HashMap::new())),
    }
  }
  pub fn with_clock(mut self, clock: Arc<dyn EngineClock>) -> Self {
    self.clock = clock;
    self
  }
  pub fn with_notification_host(mut self, host: NotificationHostProcessOptions) -> Self {
    self.notification_host =
      Some(host.with_protocol_version(INFORMATIONAL_NOTIFICATION_PROVIDER_PROTOCOL_VERSION));
    self
  }
  pub fn with_progress_reporter(mut self, reporter: ExecutionProgressReporter) -> Self {
    self.progress_reporter = Some(reporter);
    self
  }
  pub fn with_telemetry_reporter(
    mut self,
    context: crate::TelemetrySignalContext,
    reporter: crate::TelemetrySignalReporter,
  ) -> Self {
    self.telemetry_context = Some(context);
    self.telemetry_reporter = Some(reporter);
    self
  }
  pub fn with_telemetry_propagation(
    mut self,
    enabled: bool,
    sampling_ratio: f64,
    reporter: Option<crate::TelemetryAdmissionLinkReporter>,
  ) -> Self {
    self.telemetry_propagation.store(enabled, Ordering::Release);
    *self
      .telemetry_sampling_ratio
      .write()
      .expect("telemetry sampling ratio lock") = sampling_ratio.clamp(0.0, 1.0);
    self.telemetry_admission_link_reporter = reporter;
    self
  }
  pub fn with_lifecycle_progress_reporter(mut self, reporter: LifecycleProgressReporter) -> Self {
    self.lifecycle_progress_reporter = Some(reporter);
    self
  }
  pub fn with_schedule_clock(mut self, clock: Arc<dyn ScheduleClock>) -> Self {
    self.schedule_clock = clock;
    self
  }
  pub fn with_resolved_secrets(mut self, secrets: BTreeMap<String, String>) -> Self {
    self.resolved_secrets = Arc::new(secrets);
    self
  }
  pub fn with_signal_ingress_token(mut self, token: Option<String>) -> Self {
    self.signal_ingress_token = token.filter(|token| !token.is_empty());
    self
  }
  pub fn with_infrastructure_policy(
    mut self,
    policy: crate::PreparedInfrastructurePolicyV1,
  ) -> Self {
    self.infrastructure_policy = Some(Arc::new(policy));
    self.infrastructure_policy_required = true;
    self
  }
  pub fn require_infrastructure_policy(mut self) -> Self {
    self.infrastructure_policy_required = true;
    self
  }
  pub fn with_emergency_deny_overlay(
    mut self,
    policy: crate::PreparedInfrastructurePolicyV1,
  ) -> Self {
    self.emergency_deny_overlay = Some(Arc::new(policy));
    self.infrastructure_policy_required = true;
    self
  }
  pub fn with_runtime_modules(mut self, modules: Vec<RuntimeModuleArtifact>) -> Self {
    self.script_host.module_artifacts = modules
      .iter()
      .filter(|module| !module.name.starts_with("__woml_"))
      .map(|module| ScriptHostModuleArtifact {
        bundle_digest: module.bundle_digest.clone(),
        bundle: module.bundle.clone(),
        source_map_digest: module.source_map_digest.clone(),
        source_map: module.source_map.clone(),
      })
      .collect();
    self.runtime_modules = Arc::new(modules);
    self
  }
  pub fn with_reviewer_directory(mut self, authority: Arc<ReviewerDirectoryAuthorityV1>) -> Self {
    self.reviewer_authority = Some(authority);
    self
  }
  pub fn reviewer_authority(&self) -> Option<Arc<ReviewerDirectoryAuthorityV1>> {
    self.reviewer_authority.clone()
  }
  pub fn with_schedule_progress_reporter(mut self, reporter: ScheduleProgressReporter) -> Self {
    self.schedule_progress_reporter = Some(reporter);
    self
  }
  pub fn with_interval_progress_reporter(mut self, reporter: IntervalProgressReporter) -> Self {
    self.interval_progress_reporter = Some(reporter);
    self
  }
  pub fn with_workflow_call_progress_reporter(
    mut self,
    reporter: WorkflowCallProgressReporter,
  ) -> Self {
    self.workflow_call_progress_reporter = Some(reporter);
    self
  }
  pub fn with_runtime_policy_progress_reporter(
    mut self,
    reporter: RuntimePolicyProgressReporter,
  ) -> Self {
    self.runtime_policy_progress_reporter = Some(reporter);
    self
  }
  pub fn with_operational_output_reporter(
    mut self,
    reporter: crate::OperationalOutputReporter,
  ) -> Self {
    self.operational_output_reporter = Some(reporter);
    self
  }
}
