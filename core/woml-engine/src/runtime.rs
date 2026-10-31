//! Stable runtime entrypoints and orchestration wiring.
//!
//! Private engine-adapter, scheduling/control-flow, and lifecycle/correction modules preserve
//! one workflow authority, exact attempt identity, and existing boxed futures.

mod cancellation;
mod cleanup;
mod clock;
mod composition;
mod context;
mod control_flow;
mod correction;
mod engine_adapter;
mod errors;
mod failure;
mod lifecycle;
mod modules;
mod options;
mod outcomes;
mod progress;
mod scheduling;
mod script_execution;
mod script_host_pool;
#[cfg(test)]
mod tests;

use crate::ApprovalDecisionOutcome;
use crate::ApprovalTimeoutSettlement;

use crate::CompiledWorkflowDefinition;
use crate::DurableDagEngine;
use crate::DurableEventStore;
use crate::DurableStoreError;
use crate::InMemoryDagEngine;

use crate::RecoveryReport;
use crate::ReviewerDirectoryAuthorityV1;
use crate::RichApprovalResolutionReason;
use crate::RunEvent;
#[cfg(test)]
use crate::RunProjection;

use crate::RunStatus;
use crate::RunTimeoutSettlement;

use crate::TriggerAdmissionRequest;
use cancellation::cancelled_run_error;
use cancellation::settle_cancelling_run;
use cleanup::ArtifactInvocationCleanup;
pub use clock::EngineClock;
pub use clock::FixedEngineClock;
pub use clock::SystemEngineClock;
use composition::attach_durable_capability_authority;
use composition::register_builtin_capabilities;
use composition::BuiltinCapabilityResources;
use context::lock_descriptor_for_body_node;
use context::resolved_script_secrets;
use context::reusable_step_binding;
use context::reusable_step_descriptor;
use context::runtime_context_for_node;
use control_flow::completed_terminal_approval_result;
use control_flow::execute_bounded_for_each;
use control_flow::execute_for_each_iteration_script;
use control_flow::execute_fork;
use control_flow::execute_parallel_group;
use control_flow::fail_branch;
use control_flow::fail_choice;
use control_flow::for_each_digest;
pub(crate) use control_flow::for_each_effect_key;
use control_flow::for_each_module_bindings;
use control_flow::for_each_ready_nodes;
use control_flow::for_each_selected_branch;
use control_flow::for_each_selected_choice;
use control_flow::invoke_parallel_child;
use control_flow::next_parallel_completion;
use control_flow::ready_fork;
use control_flow::register_durable_wait_node;
use control_flow::request_approval_wait;
use control_flow::resolve_for_each_reference;
use control_flow::settle_open_for_each;
use control_flow::settle_open_fork_cancellation;
use control_flow::ActiveParallelInvocation;
use control_flow::DurableWaitNodeProgress;
use control_flow::ForEachTerminalDisposition;
use control_flow::ForkRuntimeProgress;
use control_flow::ParallelInvocationRequest;
use control_flow::SharedForEachEngine;
use correction::append_step_attempt_success;
use correction::append_step_attempt_success_payload;
use correction::compensation_registration_for_iteration;
use correction::correct_saga_failure_for_node;
use correction::correction_iteration_identity;
use correction::corrective_action_matches_runtime;
use correction::execute_corrective_action;
use correction::execute_saga_correction;
pub(crate) use correction::resume_corrections_for_workflows_durable;
use correction::CorrectiveActionOutcome;
pub use errors::BranchFailureSite;
pub use errors::CancelledRunDetails;
pub use errors::FailedApprovalDetails;
pub use errors::FailedBranchDetails;
pub use errors::FailedNotificationDetails;
pub use errors::FailedParallelDetails;
pub use errors::FailedRunDetails;
pub use errors::RuntimeExecutionError;
use failure::attempt_run_failed_data;
use failure::final_attempt_failure_in_nodes;
use failure::final_workflow_attempt_failure;
use failure::resumed_failure;
use lifecycle::drive_for_each_reusable_lifecycle;
use lifecycle::drive_for_each_step_lifecycle;
use lifecycle::drive_pending_reusable_step_lifecycles;
use lifecycle::drive_reusable_lifecycle;
use lifecycle::drive_reusable_step_lifecycle;
use lifecycle::drive_workflow_lifecycle;
use lifecycle::execute_lifecycle_notification;
pub use lifecycle::execute_reusable_provider_lifecycle_durable;
use lifecycle::lifecycle_action_source;
use lifecycle::lifecycle_failure_from_attempt;
use lifecycle::render_template_scalar;
use lifecycle::resolved_lifecycle_secrets;
use lifecycle::reusable_provider_descriptor;
use lifecycle::step_lifecycle_outcome;
use lifecycle::step_lifecycle_request;
use modules::runtime_modules_from_store;
use modules::validate_runtime_modules;
pub use modules::RuntimeModuleArtifact;
pub use options::RuntimeExecutionOptions;
use outcomes::final_result;
use outcomes::recorded_execution_order;
use outcomes::succeeded_execution;
pub use outcomes::WaitingRichApprovalAuthority;
pub use outcomes::WaitingWorkflowApproval;
pub use outcomes::WaitingWorkflowSignal;
pub use outcomes::WaitingWorkflowTimer;
pub use outcomes::WorkflowExecutionResult;
pub use outcomes::WorkflowRuntimeOutcome;
pub use outcomes::RUNTIME_OUTCOME_CONTRACT;
pub use outcomes::RUNTIME_OUTCOME_VERSION;
use progress::report_attempt_failed;
use progress::report_attempt_succeeded;
use progress::report_retry_scheduled;
use progress::retry_max_attempts;
pub use progress::ExecutionProgress;
pub use progress::ExecutionProgressReporter;
pub use progress::LifecycleProgress;
pub use progress::LifecycleProgressPhase;
pub use progress::LifecycleProgressReporter;
pub use progress::RuntimePolicyProgress;
pub use progress::RuntimePolicyProgressPhase;
pub use progress::RuntimePolicyProgressReporter;
pub use progress::EXECUTION_PROGRESS_CONTRACT;
pub use progress::EXECUTION_PROGRESS_VERSION;
pub use progress::LIFECYCLE_PROGRESS_PROFILE;
pub use progress::RUNTIME_POLICY_PROGRESS_PROFILE;
use scheduling::acquire_policy_execution_lease_with_store;
use scheduling::continue_runtime;
use scheduling::ensure_held_locks_current;
use scheduling::execute_policy_run_durable;
use scheduling::policy_wakeup;
use scheduling::resolve_for_each_lock_key;
use scheduling::resolve_lock_key;
use scheduling::runtime_result_node_id;
use scheduling::schedule_workflow_timeout;
use scheduling::settle_workflow_timeout_if_due;
use scheduling::PolicyClaimAcquisition;
pub(crate) use scheduling::{PolicyExecutionCoordinator, PolicyExecutionRegistry};
#[cfg(test)]
use script_execution::ensure_local_script_execution;
use script_host_pool::SharedScriptHostPool;
use serde_json::Map;
use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::RwLock;
use uuid::Uuid;

use engine_adapter::RuntimeDagEngine;

static REVIEWER_AUTHORITIES: OnceLock<RwLock<HashMap<PathBuf, Arc<ReviewerDirectoryAuthorityV1>>>> =
  OnceLock::new();

pub(crate) struct CorrectionRecoveryResume {
  pub run_id: String,
  pub outcome: Result<WorkflowRuntimeOutcome, RuntimeExecutionError>,
}

pub async fn execute_workflow(
  workflow: CompiledWorkflowDefinition,
  definition_hash: String,
  trigger: Map<String, Value>,
  options: RuntimeExecutionOptions,
) -> Result<WorkflowExecutionResult, RuntimeExecutionError> {
  let mut engine = InMemoryDagEngine::new(workflow, definition_hash)?;
  succeeded_execution(execute_with_engine(&mut engine, trigger, options).await?)
}

pub async fn execute_workflow_durable(
  workflow: CompiledWorkflowDefinition,
  definition_hash: String,
  trigger: Map<String, Value>,
  options: RuntimeExecutionOptions,
  database_path: PathBuf,
) -> Result<WorkflowExecutionResult, RuntimeExecutionError> {
  if workflow_has_approval(&workflow)
    || workflow_has_durable_wait(&workflow)
    || workflow_has_durable_lock(&workflow)
  {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "durably waiting workflows require execute_workflow_durable_outcome".to_string(),
    ));
  }
  succeeded_execution(
    execute_workflow_durable_internal(workflow, definition_hash, trigger, options, database_path)
      .await?,
  )
}

pub async fn execute_workflow_durable_outcome(
  workflow: CompiledWorkflowDefinition,
  definition_hash: String,
  trigger: Map<String, Value>,
  options: RuntimeExecutionOptions,
  database_path: PathBuf,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  if !workflow_has_approval(&workflow)
    && !workflow_has_durable_wait(&workflow)
    && !workflow_has_durable_lock(&workflow)
  {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "the runtime outcome API requires an approval, durable-wait, or lock workflow".to_string(),
    ));
  }
  execute_workflow_durable_internal(workflow, definition_hash, trigger, options, database_path)
    .await
}

async fn execute_workflow_durable_internal(
  workflow: CompiledWorkflowDefinition,
  definition_hash: String,
  trigger: Map<String, Value>,
  options: RuntimeExecutionOptions,
  database_path: PathBuf,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  let mut span = crate::performance::PerformanceSpan::new("runtime", "runtime.execute_durable");
  span.count("nodes", workflow.graph.nodes.len());
  span.count("modules", options.runtime_modules.len());
  let result = execute_workflow_durable_internal_unprofiled(
    workflow,
    definition_hash,
    trigger,
    options,
    database_path,
  )
  .await;
  if let Ok(outcome) = &result {
    span.run_id(runtime_outcome_run_id(outcome).to_string());
    span.succeed();
  }
  result
}

async fn execute_workflow_durable_internal_unprofiled(
  workflow: CompiledWorkflowDefinition,
  definition_hash: String,
  trigger: Map<String, Value>,
  options: RuntimeExecutionOptions,
  database_path: PathBuf,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  validate_rich_approval_authority(&workflow, &options)?;
  let options = attach_durable_capability_authority(options, &database_path)?;
  let mut store = DurableEventStore::open(database_path.clone())?;
  if workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V12 {
    store.register_definition_module_artifacts(
      &workflow,
      &definition_hash,
      options.runtime_modules.as_ref(),
    )?;
    let trigger_definition = workflow
      .triggers
      .iter()
      .find(|candidate| candidate.handler == "trigger.manual")
      .ok_or_else(|| {
        RuntimeExecutionError::InvalidConfiguration(
          "a direct Model v12+ run requires a manual trigger".to_string(),
        )
      })?;
    let admission = store.admit_trigger_occurrence(TriggerAdmissionRequest {
      workflow_id: workflow.workflow_id.clone(),
      definition_hash,
      trigger_id: trigger_definition.id.clone(),
      trigger_handler: trigger_definition.handler.clone(),
      source_identity: format!("manual:{}", Uuid::new_v4().simple()),
      payload: trigger,
      received_at: options.clock.now(),
    })?;
    bind_run_infrastructure_policy(&mut store, &admission.run_id, &options)?;
    return execute_policy_run_durable(database_path, &admission.run_id, options, store).await;
  }
  recover_operational_output(&options)?;
  store.recover_interrupted_runs()?;
  store.register_definition_module_artifacts(
    &workflow,
    &definition_hash,
    options.runtime_modules.as_ref(),
  )?;
  let mut engine = DurableDagEngine::new(workflow, definition_hash, store)?;
  execute_with_engine(&mut engine, trigger, options).await
}

pub async fn resume_workflow_durable(
  database_path: PathBuf,
  run_id: &str,
  options: RuntimeExecutionOptions,
) -> Result<WorkflowExecutionResult, RuntimeExecutionError> {
  succeeded_execution(
    resume_workflow_durable_internal(database_path, run_id, options, Some(false)).await?,
  )
}

pub async fn resume_workflow_durable_outcome(
  database_path: PathBuf,
  run_id: &str,
  options: RuntimeExecutionOptions,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  resume_workflow_durable_internal(database_path, run_id, options, Some(true)).await
}

pub async fn resume_workflow_durable_any_outcome(
  database_path: PathBuf,
  run_id: &str,
  options: RuntimeExecutionOptions,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  resume_workflow_durable_internal(database_path, run_id, options, None).await
}

/// Continues one run that was already atomically admitted by a production
/// trigger. Unlike the one-shot CLI resume APIs, this does not run global crash
/// recovery before dispatch: a long-lived trigger server may have other runs
/// executing concurrently, and their active attempts must not be mistaken for
/// leftovers from a dead process.
pub async fn execute_admitted_trigger_run_durable(
  database_path: PathBuf,
  run_id: &str,
  options: RuntimeExecutionOptions,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  let mut span =
    crate::performance::PerformanceSpan::new("runtime", "runtime.execute_admitted_run");
  span.run_id(run_id.to_string());
  let result =
    execute_admitted_trigger_run_durable_unprofiled(database_path, run_id, options).await;
  if result.is_ok() {
    span.succeed();
  }
  result
}

async fn execute_admitted_trigger_run_durable_unprofiled(
  database_path: PathBuf,
  run_id: &str,
  options: RuntimeExecutionOptions,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  let options = attach_durable_capability_authority(options, &database_path)?;
  let mut store = DurableEventStore::open(database_path.clone())?;
  bind_run_infrastructure_policy(&mut store, run_id, &options)?;
  let binding = store.run_binding(run_id)?;
  let workflow = store.definition(&binding.definition_hash)?;
  validate_rich_approval_authority(&workflow, &options)?;
  if workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V12 {
    return execute_policy_run_durable(database_path, run_id, options, store).await;
  }
  let options = runtime_modules_from_store(options, &store, &binding.definition_hash)?;
  let mut engine = DurableDagEngine::resume(store, run_id)?;
  resume_with_engine(&mut engine, run_id, options).await
}

async fn resume_workflow_durable_internal(
  database_path: PathBuf,
  run_id: &str,
  options: RuntimeExecutionOptions,
  approval_outcome_api: Option<bool>,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  let options = attach_durable_capability_authority(options, &database_path)?;
  let mut store = DurableEventStore::open(database_path.clone())?;
  bind_run_infrastructure_policy(&mut store, run_id, &options)?;
  let binding = store.run_binding(run_id)?;
  let workflow = store.definition(&binding.definition_hash)?;
  validate_rich_approval_authority(&workflow, &options)?;
  let has_approval = workflow_has_approval(&workflow);
  let has_durable_wait = workflow_has_durable_wait(&workflow);
  let has_durable_lock = workflow_has_durable_lock(&workflow);
  if approval_outcome_api == Some(true) && !has_approval && !has_durable_wait && !has_durable_lock {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "the runtime outcome API requires an approval, durable-wait, or lock workflow".to_string(),
    ));
  }
  if approval_outcome_api == Some(false) && (has_approval || has_durable_wait || has_durable_lock) {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "durably waiting workflows require resume_workflow_durable_outcome".to_string(),
    ));
  }
  if workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V12 {
    return execute_policy_run_durable(database_path, run_id, options, store).await;
  }
  recover_operational_output(&options)?;
  store.recover_interrupted_runs()?;
  let options = runtime_modules_from_store(options, &store, &binding.definition_hash)?;
  let mut engine = DurableDagEngine::resume(store, run_id)?;
  resume_with_engine(&mut engine, run_id, options).await
}

fn recover_operational_output(
  options: &RuntimeExecutionOptions,
) -> Result<(), RuntimeExecutionError> {
  let Some(lazy) = options.operational_output_store.as_ref() else {
    return Ok(());
  };
  let Some(store) = lazy.activate_if_exists().map_err(|error| {
    RuntimeExecutionError::InvalidConfiguration(format!("{}: {error}", error.code()))
  })?
  else {
    return Ok(());
  };
  store.recover_interrupted_streams().map_err(|error| {
    RuntimeExecutionError::InvalidConfiguration(format!("{}: {error}", error.code()))
  })?;
  Ok(())
}

fn recover_operational_run(
  options: &RuntimeExecutionOptions,
  run_id: &str,
) -> Result<(), RuntimeExecutionError> {
  let Some(lazy) = options.operational_output_store.as_ref() else {
    return Ok(());
  };
  let Some(store) = lazy.activate_if_exists().map_err(|error| {
    RuntimeExecutionError::InvalidConfiguration(format!("{}: {error}", error.code()))
  })?
  else {
    return Ok(());
  };
  store.recover_interrupted_run(run_id).map_err(|error| {
    RuntimeExecutionError::InvalidConfiguration(format!("{}: {error}", error.code()))
  })?;
  Ok(())
}

fn bind_run_infrastructure_policy(
  store: &mut DurableEventStore,
  run_id: &str,
  options: &RuntimeExecutionOptions,
) -> Result<(), RuntimeExecutionError> {
  if let Some(policy) = options.infrastructure_policy.as_ref() {
    store.bind_infrastructure_policy(run_id, policy, options.clock.now())?;
    return Ok(());
  }
  if options.infrastructure_policy_required
    && store.infrastructure_policy_binding(run_id)?.is_none()
  {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "WOML_POLICY_REQUIRED: this runtime requires an infrastructure policy, but the run has no admitted policy bundle."
        .to_string(),
    ));
  }
  Ok(())
}

fn workflow_has_approval(workflow: &CompiledWorkflowDefinition) -> bool {
  workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V4
    && workflow
      .graph
      .nodes
      .iter()
      .any(|node| node.handler == "engine.approval-wait")
}

fn validate_rich_approval_authority(
  workflow: &CompiledWorkflowDefinition,
  options: &RuntimeExecutionOptions,
) -> Result<(), RuntimeExecutionError> {
  if workflow.schema_version != crate::COMPILED_MODEL_SCHEMA_VERSION_V20 {
    return Ok(());
  }
  let authority = options.reviewer_authority.as_ref().ok_or_else(|| {
    RuntimeExecutionError::InvalidConfiguration(
      "WOML_APPROVAL_AUTHORITY_UNAVAILABLE: Model v20 requires an operator-owned Reviewer Directory v1".to_string(),
    )
  })?;
  let version = authority.active_version().map_err(|error| {
    RuntimeExecutionError::InvalidConfiguration(format!("{}: {error}", error.code()))
  })?;
  for node in workflow
    .graph
    .nodes
    .iter()
    .filter(|node| node.handler == "engine.approval-wait")
  {
    let approval = workflow.approval(&node.id).ok_or_else(|| {
      RuntimeExecutionError::InvalidConfiguration(
        "Model v20 contains an invalid approval descriptor".to_string(),
      )
    })?;
    let Some(required) = approval.required_approvals else {
      continue;
    };
    authority
      .validate_requirements(&version, required, &approval.required_roles)
      .map_err(|error| {
        RuntimeExecutionError::InvalidConfiguration(format!("{}: {error}", error.code()))
      })?;
  }
  Ok(())
}

fn workflow_has_durable_wait(workflow: &CompiledWorkflowDefinition) -> bool {
  workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V17
    && workflow
      .graph
      .nodes
      .iter()
      .any(|node| node.handler == "engine.durable-wait")
}

fn workflow_has_durable_lock(workflow: &CompiledWorkflowDefinition) -> bool {
  workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V18
    && workflow
      .graph
      .locks
      .as_deref()
      .is_some_and(|locks| !locks.is_empty())
}

pub fn recover_durable_runs(
  database_path: PathBuf,
) -> Result<RecoveryReport, RuntimeExecutionError> {
  let mut store = DurableEventStore::open(database_path)?;
  Ok(store.recover_interrupted_runs()?)
}

/// Atomically resolves every due timer wait in the bounded batch and resumes
/// only the runs whose wait this caller won. The caller supplies the clock via
/// RuntimeExecutionOptions, so restart recovery and tests do not depend on an
/// in-memory timer.
pub async fn resume_due_timer_waits_durable(
  database_path: PathBuf,
  options: RuntimeExecutionOptions,
  limit: usize,
) -> Result<Vec<WorkflowRuntimeOutcome>, RuntimeExecutionError> {
  resume_due_timer_waits_for_workflows_durable(database_path, options, limit, None).await
}

/// Resumes due timer waits only for workflow IDs owned by the calling runtime.
/// A shared durable store can serve multiple runtime processes, so the
/// production scheduler must never claim a wait merely because it can see it.
pub(crate) async fn resume_due_timer_waits_for_workflows_durable(
  database_path: PathBuf,
  options: RuntimeExecutionOptions,
  limit: usize,
  workflow_ids: Option<&HashSet<String>>,
) -> Result<Vec<WorkflowRuntimeOutcome>, RuntimeExecutionError> {
  let now = options.clock.now();
  let due = match workflow_ids {
    Some(workflow_ids) => DurableEventStore::open(database_path.clone())?
      .due_timer_waits_for_workflows(now, limit, workflow_ids)?,
    None => DurableEventStore::open(database_path.clone())?.due_timer_waits(now, limit)?,
  };
  let mut outcomes = Vec::with_capacity(due.len());
  for wait in due {
    let mut store = DurableEventStore::open(database_path.clone())?;
    let settlement = store.settle_timer_wait(&wait.run_id, &wait.wait_id, now)?;
    if !matches!(settlement, crate::TimerWaitSettlement::Resolved { .. }) {
      continue;
    }
    outcomes.push(
      execute_policy_run_durable(database_path.clone(), &wait.run_id, options.clone(), store)
        .await?,
    );
  }
  Ok(outcomes)
}

pub struct ExternalSignalRuntimeOutcome {
  pub admission: crate::ExternalSignalAdmissionOutcome,
  pub continuation: Option<Result<WorkflowRuntimeOutcome, RuntimeExecutionError>>,
}

pub async fn admit_external_signal_durable(
  database_path: PathBuf,
  request: crate::ExternalSignalAdmissionRequest,
  options: RuntimeExecutionOptions,
) -> Result<ExternalSignalRuntimeOutcome, RuntimeExecutionError> {
  let mut known_secrets = options
    .resolved_secrets
    .values()
    .cloned()
    .collect::<Vec<_>>();
  if let Some(token) = options.signal_ingress_token.as_ref() {
    known_secrets.push(token.clone());
  }
  let mut store = DurableEventStore::open(database_path.clone())?;
  let admission = store.admit_external_signal_atomically(request, &known_secrets)?;
  let continuation = if admission.resumes_run() {
    Some(execute_admitted_trigger_run_durable(database_path, &admission.run_id, options).await)
  } else {
    None
  };
  Ok(ExternalSignalRuntimeOutcome {
    admission,
    continuation,
  })
}

pub(crate) struct DueSignalWaitResume {
  pub run_id: String,
  pub outcome: Result<WorkflowRuntimeOutcome, RuntimeExecutionError>,
}

pub(crate) async fn resume_due_signal_waits_for_workflows_durable(
  database_path: PathBuf,
  options: RuntimeExecutionOptions,
  limit: usize,
  workflow_ids: &HashSet<String>,
) -> Result<Vec<DueSignalWaitResume>, RuntimeExecutionError> {
  let now = options.clock.now();
  let due = DurableEventStore::open(database_path.clone())?.due_signal_waits_for_workflows(
    now,
    limit,
    workflow_ids,
  )?;
  let mut resumed = Vec::with_capacity(due.len());
  for wait in due {
    let mut store = DurableEventStore::open(database_path.clone())?;
    let settlement = store.settle_signal_wait_timeout(&wait.run_id, &wait.wait_id, now)?;
    if !matches!(
      settlement,
      crate::SignalWaitTimeoutSettlement::Continued { .. }
        | crate::SignalWaitTimeoutSettlement::Failed { .. }
    ) {
      continue;
    }
    resumed.push(DueSignalWaitResume {
      run_id: wait.run_id.clone(),
      outcome: execute_policy_run_durable(
        database_path.clone(),
        &wait.run_id,
        options.clone(),
        store,
      )
      .await,
    });
  }
  Ok(resumed)
}

pub fn resolve_human_approval_durable(
  database_path: PathBuf,
  token: &str,
  decision: crate::ApprovalDecision,
  clock: &dyn EngineClock,
) -> Result<ApprovalDecisionOutcome, RuntimeExecutionError> {
  let mut store = DurableEventStore::open(database_path)?;
  Ok(store.resolve_human_approval(token, decision, clock.now())?)
}

pub fn resolve_rich_approval_durable(
  database_path: PathBuf,
  decision: crate::RichApprovalIdentityDecision,
) -> Result<crate::RichApprovalDecisionOutcome, RuntimeExecutionError> {
  let authority = REVIEWER_AUTHORITIES
    .get_or_init(|| RwLock::new(HashMap::new()))
    .read()
    .map_err(|_| {
      RuntimeExecutionError::InvalidConfiguration(
        "WOML_APPROVAL_AUTHORITY_UNAVAILABLE: reviewer authority is unavailable".to_string(),
      )
    })?
    .get(&database_path)
    .cloned()
    .ok_or_else(|| {
      RuntimeExecutionError::InvalidConfiguration(
        "WOML_APPROVAL_AUTHORITY_UNAVAILABLE: no reviewer authority is active for this durable store".to_string(),
      )
    })?;
  let mut store = DurableEventStore::open(database_path)?;
  Ok(store.record_rich_approval_identity_decision(&authority, decision)?)
}

pub fn settle_approval_timeout_durable(
  database_path: PathBuf,
  run_id: &str,
  approval_id: &str,
  clock: &dyn EngineClock,
) -> Result<ApprovalTimeoutSettlement, RuntimeExecutionError> {
  let mut store = DurableEventStore::open(database_path)?;
  let now = clock.now();
  let projection = store.projection(run_id)?;
  if let Some(request) = projection.rich_approval_requests.get(approval_id) {
    let binding = store.run_binding(run_id)?;
    let workflow = store.definition(&binding.definition_hash)?;
    let approval = workflow.approval(approval_id).ok_or_else(|| {
      RuntimeExecutionError::Stalled(
        "rich approval timeout references an unknown approval definition".to_string(),
      )
    })?;
    let deadline = approval
      .timeout_ms
      .ok_or(DurableStoreError::ApprovalHasNoDeadline)
      .and_then(|milliseconds| {
        i64::try_from(milliseconds)
          .map(chrono::Duration::milliseconds)
          .map(|duration| request.requested_at + duration)
          .map_err(|_| {
            DurableStoreError::Contract(
              "Rich approval timeout exceeds the durable clock range.".to_string(),
            )
          })
      })?;
    if request.status != crate::RichApprovalRequestStatus::Waiting {
      return Ok(ApprovalTimeoutSettlement {
        status: crate::ApprovalTimeoutSettlementStatus::AlreadyResolved,
        run_id: run_id.to_string(),
        approval_id: approval_id.to_string(),
        request_id: request.request_id.clone(),
        resolution: None,
        settled_at: request.resolved_at,
      });
    }
    if now < deadline {
      return Ok(ApprovalTimeoutSettlement {
        status: crate::ApprovalTimeoutSettlementStatus::NotDue,
        run_id: run_id.to_string(),
        approval_id: approval_id.to_string(),
        request_id: request.request_id.clone(),
        resolution: None,
        settled_at: None,
      });
    }
    let outcome = store.settle_rich_approval_without_vote(
      run_id,
      &request.request_id,
      &request.review_digest,
      RichApprovalResolutionReason::Timeout,
      now,
    )?;
    return Ok(ApprovalTimeoutSettlement {
      status: if outcome.status == crate::RichApprovalDecisionStatus::AlreadyResolved {
        crate::ApprovalTimeoutSettlementStatus::AlreadyResolved
      } else {
        crate::ApprovalTimeoutSettlementStatus::Settled
      },
      run_id: run_id.to_string(),
      approval_id: approval_id.to_string(),
      request_id: request.request_id.clone(),
      resolution: None,
      settled_at: Some(now),
    });
  }
  if projection.timeout_reached_at.is_some() {
    let request = projection
      .approval_requests
      .get(approval_id)
      .ok_or_else(|| {
        RuntimeExecutionError::Stalled(
          "workflow timeout references an unknown approval request".to_string(),
        )
      })?;
    return Ok(ApprovalTimeoutSettlement {
      status: crate::ApprovalTimeoutSettlementStatus::Settled,
      run_id: run_id.to_string(),
      approval_id: approval_id.to_string(),
      request_id: request.request_id.clone(),
      resolution: None,
      settled_at: projection.timeout_reached_at,
    });
  }
  if projection
    .timeout_at
    .is_some_and(|deadline_at| now >= deadline_at)
    && projection.business_outcome.is_none()
    && projection.cancellation_request_id.is_none()
  {
    let settlement = store.settle_run_timeout(run_id, now)?;
    let timed_out = match settlement {
      RunTimeoutSettlement::TimedOut { projection }
      | RunTimeoutSettlement::LostRace { projection } => projection
        .timeout_reached_at
        .is_some()
        .then_some(projection),
      RunTimeoutSettlement::NotConfigured | RunTimeoutSettlement::NotDue { .. } => None,
    };
    if let Some(projection) = timed_out {
      let request = projection
        .approval_requests
        .get(approval_id)
        .ok_or_else(|| {
          RuntimeExecutionError::Stalled(
            "workflow timeout references an unknown approval request".to_string(),
          )
        })?;
      return Ok(ApprovalTimeoutSettlement {
        status: crate::ApprovalTimeoutSettlementStatus::Settled,
        run_id: run_id.to_string(),
        approval_id: approval_id.to_string(),
        request_id: request.request_id.clone(),
        resolution: None,
        settled_at: projection.timeout_reached_at,
      });
    }
  }
  Ok(store.settle_approval_timeout(run_id, approval_id, now)?)
}

async fn execute_with_engine<E: RuntimeDagEngine>(
  engine: &mut E,
  trigger: Map<String, Value>,
  options: RuntimeExecutionOptions,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  let mut span = crate::performance::PerformanceSpan::new("runtime", "runtime.run_engine");
  span.count("nodes", engine.workflow().graph.nodes.len());
  let result = execute_with_engine_unprofiled(engine, trigger, options.clone()).await;
  match &result {
    Ok(outcome) => {
      let run_id = runtime_outcome_run_id(outcome);
      report_telemetry_history(engine, run_id, &options);
      span.run_id(run_id.to_string());
      span.succeed();
    }
    Err(error) => {
      if let Some(events) = runtime_error_events(error) {
        report_telemetry_events(&engine.workflow().workflow_id, events, &options);
      }
    }
  }
  result
}

async fn execute_with_engine_unprofiled<E: RuntimeDagEngine>(
  engine: &mut E,
  trigger: Map<String, Value>,
  options: RuntimeExecutionOptions,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  if options.script_timeout_ms == 0 {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "script_timeout_ms must be greater than zero".to_string(),
    ));
  }
  validate_runtime_modules(engine.workflow(), &options)?;

  execute_runtime(engine, trigger, &options).await
}

fn sha256_identity(content: &str) -> String {
  format!("sha256:{:x}", Sha256::digest(content.as_bytes()))
}

async fn resume_with_engine<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  options: RuntimeExecutionOptions,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  let mut span = crate::performance::PerformanceSpan::new("runtime", "runtime.resume_engine");
  span.run_id(run_id.to_string());
  span.count("nodes", engine.workflow().graph.nodes.len());
  let result = resume_with_engine_unprofiled(engine, run_id, options.clone()).await;
  report_telemetry_history(engine, run_id, &options);
  settle_operational_run(engine, run_id, &options);
  if result.is_ok() {
    span.succeed();
  }
  result
}

fn report_telemetry_history<E: RuntimeDagEngine>(
  engine: &E,
  run_id: &str,
  options: &RuntimeExecutionOptions,
) {
  if options.telemetry_reporter.is_none() || options.telemetry_context.is_none() {
    return;
  }
  let Ok(events) = engine.events(run_id) else {
    return;
  };
  report_telemetry_events(&engine.workflow().workflow_id, &events, options);
}

fn report_telemetry_events(
  workflow_id: &str,
  events: &[RunEvent],
  options: &RuntimeExecutionOptions,
) {
  let (Some(reporter), Some(context)) = (
    options.telemetry_reporter.as_ref(),
    options.telemetry_context.as_ref(),
  ) else {
    return;
  };
  for signal in crate::project_telemetry_signals(events, workflow_id, context) {
    reporter(signal);
  }
}

fn runtime_error_events(error: &RuntimeExecutionError) -> Option<&[RunEvent]> {
  match error {
    RuntimeExecutionError::RunFailed(details) => Some(&details.events),
    RuntimeExecutionError::BranchFailed(details) => Some(&details.events),
    RuntimeExecutionError::ParallelFailed(details) => Some(&details.events),
    RuntimeExecutionError::ApprovalFailed(details) => Some(&details.events),
    RuntimeExecutionError::NotificationFailed(details) => Some(&details.events),
    RuntimeExecutionError::RunCancelled(details) => Some(&details.events),
    _ => None,
  }
}

fn settle_operational_run<E: RuntimeDagEngine>(
  engine: &E,
  run_id: &str,
  options: &RuntimeExecutionOptions,
) {
  let Some(store) = options
    .operational_output_store
    .as_ref()
    .and_then(|store| store.active_store())
  else {
    return;
  };
  let Ok(projection) = engine.projection(run_id) else {
    return;
  };
  let status = match projection.status {
    RunStatus::Succeeded => crate::OperationalCloseStatusV1::Succeeded,
    RunStatus::Failed => crate::OperationalCloseStatusV1::Failed,
    RunStatus::Cancelled => crate::OperationalCloseStatusV1::Cancelled,
    _ => return,
  };
  let _ = store.append(crate::OperationalRecordDraftV1 {
    identity: crate::OperationalIdentityV1 {
      workflow_id: engine.workflow().workflow_id.clone(),
      run_id: run_id.to_string(),
      node_id: None,
      attempt: None,
      invocation_id: None,
      phase: crate::OperationalExecutionPhaseV1::Forward,
    },
    body: crate::OperationalBodyV1::Closed {
      status,
      reason_code: None,
    },
  });
}

fn runtime_outcome_run_id(outcome: &WorkflowRuntimeOutcome) -> &str {
  match outcome {
    WorkflowRuntimeOutcome::Succeeded { execution, .. } => &execution.run_id,
    WorkflowRuntimeOutcome::Waiting { run_id, .. } => run_id,
    WorkflowRuntimeOutcome::WaitingTime { run_id, .. } => run_id,
    WorkflowRuntimeOutcome::WaitingSignal { run_id, .. } => run_id,
  }
}

async fn resume_with_engine_unprofiled<E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  options: RuntimeExecutionOptions,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  if options.script_timeout_ms == 0 {
    return Err(RuntimeExecutionError::InvalidConfiguration(
      "script_timeout_ms must be greater than zero".to_string(),
    ));
  }
  validate_runtime_modules(engine.workflow(), &options)?;
  ensure_held_locks_current(engine, run_id, &options)?;
  let projection = engine.projection(run_id)?;
  let execution_order = recorded_execution_order(&engine.events(run_id)?);
  match projection.status {
    RunStatus::Succeeded => {
      return Ok(WorkflowRuntimeOutcome::succeeded(final_result(
        engine,
        run_id,
        execution_order,
      )?));
    }
    RunStatus::Failed => return Err(resumed_failure(engine, run_id, projection)?),
    RunStatus::Cancelled => {
      return Err(cancelled_run_error(engine, run_id)?);
    }
    RunStatus::Finalizing => {
      let mut host = None;
      drive_pending_reusable_step_lifecycles(engine, run_id, &options, &mut host).await?;
      drive_workflow_lifecycle(engine, run_id, &options, &mut host).await?;
      if let Some(host) = host {
        options.release_script_host(host).await;
      }
      let projection = engine.projection(run_id)?;
      return match projection.status {
        RunStatus::Succeeded => Ok(WorkflowRuntimeOutcome::succeeded(final_result(
          engine,
          run_id,
          execution_order,
        )?)),
        RunStatus::Failed => Err(resumed_failure(engine, run_id, projection)?),
        RunStatus::Cancelled => Err(cancelled_run_error(engine, run_id)?),
        _ => Err(RuntimeExecutionError::Stalled(
          "stored run did not finish lifecycle continuation".to_string(),
        )),
      };
    }
    RunStatus::Cancelling | RunStatus::Running => {}
    RunStatus::Waiting => {
      return engine.reissue_waiting_outcome(run_id, options.clock.now());
    }
    RunStatus::NotStarted | RunStatus::Queued => {
      return Err(RuntimeExecutionError::Stalled(
        "stored run has not entered execution".to_string(),
      ));
    }
  }
  let terminal_node_id = runtime_result_node_id(engine.workflow())
    .ok_or_else(|| RuntimeExecutionError::Stalled("no terminal node exists".to_string()))?
    .to_string();
  continue_runtime(engine, run_id, terminal_node_id, execution_order, &options).await
}

async fn execute_runtime<E: RuntimeDagEngine>(
  engine: &mut E,
  trigger: Map<String, Value>,
  options: &RuntimeExecutionOptions,
) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
  let terminal_node_id = runtime_result_node_id(engine.workflow())
    .ok_or_else(|| RuntimeExecutionError::Stalled("no terminal node exists".to_string()))?
    .to_string();
  let run_id = generated_id("run");
  engine.start_run(&run_id, trigger)?;
  continue_runtime(engine, &run_id, terminal_node_id, Vec::new(), options).await
}

fn generated_id(prefix: &str) -> String {
  format!("{prefix}_{}", Uuid::new_v4().simple())
}
