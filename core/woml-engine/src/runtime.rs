//! Stable runtime entrypoints and engine adapters.
//!
//! Private scheduling/control-flow and lifecycle/correction modules preserve
//! one workflow authority, exact attempt identity, and existing boxed futures.

mod cancellation;
mod cleanup;
mod clock;
mod composition;
mod context;
mod control_flow;
mod correction;
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

use crate::event::ApprovalRequestedData;
use crate::event::ApprovalTimeoutPolicy;
use crate::event::ForEachIterationScope;
use crate::event::LifecycleFailure;
use crate::event::RunOutcomeDecidedData;
use crate::event::StepAttemptFailedData;
use crate::event::StepAttemptStartedData;
use crate::event::StepAttemptSucceededData;
use crate::projection::ApprovalRequestStatus;
use crate::run_event_schema_version_for_model;
use crate::ApprovalDecisionOutcome;
use crate::ApprovalTimeoutSettlement;
use crate::BoundRichApprovalReview;
use crate::CompiledWorkflowDefinition;
use crate::DurableDagEngine;
use crate::DurableEventStore;
use crate::DurableStoreError;
use crate::InMemoryDagEngine;
use crate::IssuedApprovalToken;
use crate::RecoveryReport;
use crate::ReviewerDirectoryAuthorityV1;
use crate::RichApprovalResolutionReason;
use crate::RunEvent;
use crate::RunEventPayload;
use crate::RunProjection;
use crate::RunStatus;
use crate::RunTimeoutSettlement;
use crate::StepFailureDisposition;
use crate::TriggerAdmissionRequest;
use cancellation::{cancelled_run_error, settle_cancelling_run};
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
pub(crate) use control_flow::for_each_effect_key;
use control_flow::{
  completed_terminal_approval_result, execute_bounded_for_each, execute_for_each_iteration_script,
  execute_fork, execute_parallel_group, fail_branch, fail_choice, for_each_digest,
  for_each_module_bindings, for_each_ready_nodes, for_each_selected_branch,
  for_each_selected_choice, invoke_parallel_child, next_parallel_completion, ready_fork,
  register_durable_wait_node, request_approval_wait, resolve_for_each_reference,
  rich_waiting_outcome, settle_open_for_each, settle_open_fork_cancellation, waiting_outcome,
  ActiveParallelInvocation, DurableWaitNodeProgress, ForEachTerminalDisposition,
  ForkRuntimeProgress, ParallelInvocationRequest, SharedForEachEngine,
};
pub(crate) use correction::resume_corrections_for_workflows_durable;
use correction::{
  append_step_attempt_success, append_step_attempt_success_payload,
  compensation_registration_for_iteration, correct_saga_failure_for_node,
  correction_iteration_identity, corrective_action_matches_runtime, execute_corrective_action,
  execute_saga_correction, CorrectiveActionOutcome,
};
pub use errors::BranchFailureSite;
pub use errors::CancelledRunDetails;
pub use errors::FailedApprovalDetails;
pub use errors::FailedBranchDetails;
pub use errors::FailedNotificationDetails;
pub use errors::FailedParallelDetails;
pub use errors::FailedRunDetails;
pub use errors::RuntimeExecutionError;
use failure::{
  attempt_run_failed_data, final_attempt_failure_in_nodes, final_workflow_attempt_failure,
  resumed_failure,
};
pub use lifecycle::execute_reusable_provider_lifecycle_durable;
use lifecycle::{
  drive_for_each_reusable_lifecycle, drive_for_each_step_lifecycle,
  drive_pending_reusable_step_lifecycles, drive_reusable_lifecycle, drive_reusable_step_lifecycle,
  drive_workflow_lifecycle, execute_lifecycle_notification, lifecycle_action_source,
  lifecycle_failure_from_attempt, render_template_scalar, resolved_lifecycle_secrets,
  reusable_provider_descriptor, step_lifecycle_outcome, step_lifecycle_request,
};
use modules::runtime_modules_from_store;
use modules::validate_runtime_modules;
pub use modules::RuntimeModuleArtifact;
pub use options::RuntimeExecutionOptions;
pub use outcomes::WaitingRichApprovalAuthority;
pub use outcomes::WaitingWorkflowApproval;
pub use outcomes::WaitingWorkflowSignal;
pub use outcomes::WaitingWorkflowTimer;
pub use outcomes::WorkflowExecutionResult;
pub use outcomes::WorkflowRuntimeOutcome;
pub use outcomes::RUNTIME_OUTCOME_CONTRACT;
pub use outcomes::RUNTIME_OUTCOME_VERSION;
use outcomes::{final_result, recorded_execution_order, succeeded_execution};
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
use scheduling::{
  acquire_policy_execution_lease_with_store, continue_runtime, ensure_held_locks_current,
  execute_policy_run_durable, policy_wakeup, resolve_for_each_lock_key, resolve_lock_key,
  runtime_result_node_id, schedule_workflow_timeout, settle_workflow_timeout_if_due,
  PolicyClaimAcquisition,
};
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

trait RuntimeDagEngine {
  fn workflow(&self) -> &CompiledWorkflowDefinition;
  fn definition_hash(&self) -> &str;
  fn start_run(
    &mut self,
    run_id: &str,
    trigger: Map<String, Value>,
  ) -> Result<(), RuntimeExecutionError>;
  fn append_payload(
    &mut self,
    run_id: &str,
    payload: RunEventPayload,
  ) -> Result<(), RuntimeExecutionError>;
  fn append_scoped_payload(
    &mut self,
    run_id: &str,
    iteration: ForEachIterationScope,
    payload: RunEventPayload,
  ) -> Result<(), RuntimeExecutionError>;
  fn append_scoped_payloads(
    &mut self,
    run_id: &str,
    iteration: ForEachIterationScope,
    payloads: Vec<RunEventPayload>,
  ) -> Result<(), RuntimeExecutionError> {
    for payload in payloads {
      self.append_scoped_payload(run_id, iteration.clone(), payload)?;
    }
    Ok(())
  }
  fn settle_workflow_timeout(
    &mut self,
    _run_id: &str,
    _occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<bool, RuntimeExecutionError> {
    Ok(false)
  }
  fn decide_run_cancelled(
    &mut self,
    run_id: &str,
    cancellation_request_id: String,
    _occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    self.append_payload(
      run_id,
      RunEventPayload::RunOutcomeDecided(RunOutcomeDecidedData::Cancelled {
        cancellation_request_id,
      }),
    )
  }
  fn decide_run_failed(
    &mut self,
    run_id: &str,
    failure: LifecycleFailure,
    _occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    self.append_payload(
      run_id,
      RunEventPayload::RunOutcomeDecided(RunOutcomeDecidedData::Failed { failure }),
    )
  }
  fn record_step_attempt_failure(
    &mut self,
    run_id: &str,
    _failed_at: chrono::DateTime<chrono::Utc>,
    failure: StepAttemptFailedData,
  ) -> Result<StepFailureDisposition, RuntimeExecutionError> {
    if self
      .workflow()
      .parallel_group_for_child(&failure.node_id)
      .is_some()
    {
      self.append_payload(run_id, RunEventPayload::StepAttemptFailed(failure))?;
      return Ok(StepFailureDisposition::StepFailed);
    }
    if self.event_schema_version() >= crate::RUN_EVENT_SCHEMA_VERSION_V6 {
      return Err(RuntimeExecutionError::Stalled(
        "Model v6 retry failures require the durable runtime".to_string(),
      ));
    }
    let run_failed = attempt_run_failed_data(self.event_schema_version(), &failure);
    self.append_payloads(
      run_id,
      vec![
        RunEventPayload::StepAttemptFailed(failure),
        RunEventPayload::RunFailed(run_failed),
      ],
    )?;
    Ok(StepFailureDisposition::RunFailed)
  }
  fn projection(&self, run_id: &str) -> Result<RunProjection, RuntimeExecutionError>;
  fn events(&self, run_id: &str) -> Result<Vec<RunEvent>, RuntimeExecutionError>;
  fn ready_node_ids(&self, run_id: &str) -> Result<Vec<String>, RuntimeExecutionError>;
  fn correction_action_externally_resolved(
    &self,
    _run_id: &str,
    _saga_id: &str,
    _action_id: &str,
  ) -> Result<bool, RuntimeExecutionError> {
    Ok(false)
  }
  fn request_approval(
    &mut self,
    _run_id: &str,
    _occurred_at: chrono::DateTime<chrono::Utc>,
    _request: ApprovalRequestedData,
  ) -> Result<IssuedApprovalToken, RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "human approval requires the durable runtime".to_string(),
    ))
  }
  fn request_rich_approval(
    &mut self,
    _run_id: &str,
    _occurred_at: chrono::DateTime<chrono::Utc>,
    _binding: &BoundRichApprovalReview,
  ) -> Result<(), RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "rich plan approval requires the durable runtime".to_string(),
    ))
  }
  fn settle_pending_rich_approval(
    &mut self,
    _run_id: &str,
    _reason: RichApprovalResolutionReason,
    _settled_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<bool, RuntimeExecutionError> {
    Ok(false)
  }
  fn register_durable_wait(
    &mut self,
    _run_id: &str,
    _occurred_at: chrono::DateTime<chrono::Utc>,
    _registration: crate::WaitRegisteredData,
  ) -> Result<(), RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "durable waits require the durable runtime".to_string(),
    ))
  }
  fn register_lock_waiter(
    &mut self,
    _run_id: &str,
    _lock_id: &str,
    _key: &str,
    _occurred_at: chrono::DateTime<chrono::Utc>,
    _deadline: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "durable locks require the durable runtime".to_string(),
    ))
  }
  fn release_lock(
    &mut self,
    _run_id: &str,
    _lock_id: &str,
    _result: Value,
    _occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "durable locks require the durable runtime".to_string(),
    ))
  }
  fn timeout_lock_waiter(
    &mut self,
    _run_id: &str,
    _lock_id: &str,
    _occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "durable locks require the durable runtime".to_string(),
    ))
  }
  fn renew_lock_lease(
    &mut self,
    _run_id: &str,
    _lock_id: &str,
    _now: chrono::DateTime<chrono::Utc>,
  ) -> Result<crate::LockLeaseMaintenance, RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "durable lock renewal requires the durable runtime".to_string(),
    ))
  }
  fn renew_iteration_lock_lease(
    &mut self,
    _run_id: &str,
    _lock_id: &str,
    _iteration: ForEachIterationScope,
    _now: chrono::DateTime<chrono::Utc>,
  ) -> Result<crate::LockLeaseMaintenance, RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "iteration lock renewal requires the durable runtime".to_string(),
    ))
  }
  fn cancel_lock_waiters(
    &mut self,
    _run_id: &str,
    _cancellation_id: &str,
    _now: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "durable lock cancellation requires the durable runtime".to_string(),
    ))
  }
  fn register_iteration_lock_waiter(
    &mut self,
    _run_id: &str,
    _lock_id: &str,
    _key: &str,
    _iteration: ForEachIterationScope,
    _now: chrono::DateTime<chrono::Utc>,
    _deadline: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "iteration lock acquisition requires the durable runtime".to_string(),
    ))
  }
  fn release_iteration_lock(
    &mut self,
    _run_id: &str,
    _lock_id: &str,
    _iteration: ForEachIterationScope,
    _result: Value,
    _now: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "iteration lock release requires the durable runtime".to_string(),
    ))
  }
  fn timeout_iteration_lock_waiter(
    &mut self,
    _run_id: &str,
    _lock_id: &str,
    _iteration: ForEachIterationScope,
    _now: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "iteration lock timeout requires the durable runtime".to_string(),
    ))
  }
  fn settle_timer_wait(
    &mut self,
    _run_id: &str,
    _wait_id: &str,
    _now: chrono::DateTime<chrono::Utc>,
  ) -> Result<crate::TimerWaitSettlement, RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "durable timer waits require the durable runtime".to_string(),
    ))
  }
  fn reissue_waiting_outcome(
    &mut self,
    _run_id: &str,
    _now: chrono::DateTime<chrono::Utc>,
  ) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
    Err(RuntimeExecutionError::Stalled(
      "human approval requires the durable runtime".to_string(),
    ))
  }
  fn append_payloads(
    &mut self,
    run_id: &str,
    payloads: Vec<RunEventPayload>,
  ) -> Result<(), RuntimeExecutionError> {
    for payload in payloads {
      self.append_payload(run_id, payload)?;
    }
    Ok(())
  }
  fn event_schema_version(&self) -> u32 {
    run_event_schema_version_for_model(self.workflow().schema_version)
  }
  fn publish_pure_result(
    &mut self,
    run_id: &str,
    node_id: &str,
    output: Value,
  ) -> Result<(), RuntimeExecutionError> {
    let invocation_id = generated_id("inv");
    let handler = self
      .workflow()
      .node(node_id)
      .map(|node| node.handler.clone())
      .ok_or_else(|| {
        RuntimeExecutionError::Stalled(format!("pure result node {node_id:?} disappeared"))
      })?;
    self.append_payload(
      run_id,
      RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
        node_id: node_id.to_string(),
        attempt: 1,
        invocation_id: invocation_id.clone(),
        handler,
        idempotency_key: None,
      }),
    )?;
    self.append_payload(
      run_id,
      RunEventPayload::StepAttemptSucceeded(StepAttemptSucceededData {
        node_id: node_id.to_string(),
        attempt: 1,
        invocation_id,
        output,
      }),
    )?;
    Ok(())
  }
}

impl RuntimeDagEngine for InMemoryDagEngine {
  fn workflow(&self) -> &CompiledWorkflowDefinition {
    self.workflow()
  }

  fn definition_hash(&self) -> &str {
    self.definition_hash()
  }

  fn start_run(
    &mut self,
    run_id: &str,
    trigger: Map<String, Value>,
  ) -> Result<(), RuntimeExecutionError> {
    if self.event_schema_version() >= crate::RUN_EVENT_SCHEMA_VERSION_V7 {
      let trigger_definition = self
        .workflow()
        .triggers
        .iter()
        .find(|candidate| candidate.handler == "trigger.manual")
        .ok_or_else(|| {
          RuntimeExecutionError::InvalidConfiguration(
            "a direct Model v7+ run requires a manual trigger".to_string(),
          )
        })?;
      let started = RunEventPayload::RunStarted(crate::RunStartedData {
        workflow_id: self.workflow().workflow_id.clone(),
        definition_hash: self.definition_hash().to_string(),
        trigger_id: Some(trigger_definition.id.clone()),
        trigger_handler: Some(trigger_definition.handler.clone()),
        trigger_occurrence_id: Some(generated_id("occ")),
        ingress: None,
        trigger,
      });
      let payloads = crate::durable::expand_model_v11_payload(self.workflow(), run_id, started)?;
      for payload in payloads {
        self.append_event(RunEvent {
          event_schema_version: self.event_schema_version(),
          event_id: generated_id("evt"),
          run_id: run_id.to_string(),
          sequence: self.events(run_id).len() as u64 + 1,
          occurred_at: chrono::Utc::now(),
          iteration: None,
          payload,
        })?;
      }
      return Ok(());
    }
    self.start_run(generated_id("evt"), run_id, chrono::Utc::now(), trigger)?;
    Ok(())
  }

  fn append_payload(
    &mut self,
    run_id: &str,
    payload: RunEventPayload,
  ) -> Result<(), RuntimeExecutionError> {
    let event_schema_version = self.event_schema_version();
    let payloads = if matches!(
      event_schema_version,
      crate::RUN_EVENT_SCHEMA_VERSION_V10
        | crate::RUN_EVENT_SCHEMA_VERSION_V11
        | crate::RUN_EVENT_SCHEMA_VERSION_V12
        | crate::RUN_EVENT_SCHEMA_VERSION_V13
        | crate::RUN_EVENT_SCHEMA_VERSION_V14
        | crate::RUN_EVENT_SCHEMA_VERSION_V15
        | crate::RUN_EVENT_SCHEMA_VERSION_V16
        | crate::RUN_EVENT_SCHEMA_VERSION_V17
        | crate::RUN_EVENT_SCHEMA_VERSION_V18
        | crate::RUN_EVENT_SCHEMA_VERSION_V19
        | crate::RUN_EVENT_SCHEMA_VERSION_V20
    ) {
      crate::durable::expand_model_v11_payload(self.workflow(), run_id, payload)?
    } else {
      vec![payload]
    };
    let occurred_at = chrono::Utc::now();
    for payload in payloads {
      let sequence = self.events(run_id).len() as u64 + 1;
      self.append_event(RunEvent {
        event_schema_version,
        event_id: generated_id("evt"),
        run_id: run_id.to_string(),
        sequence,
        occurred_at,
        iteration: None,
        payload,
      })?;
    }
    Ok(())
  }

  fn append_scoped_payload(
    &mut self,
    run_id: &str,
    iteration: ForEachIterationScope,
    payload: RunEventPayload,
  ) -> Result<(), RuntimeExecutionError> {
    self.append_event(RunEvent {
      event_schema_version: self.event_schema_version(),
      event_id: generated_id("evt"),
      run_id: run_id.to_string(),
      sequence: self.events(run_id).len() as u64 + 1,
      occurred_at: chrono::Utc::now(),
      iteration: Some(iteration),
      payload,
    })?;
    Ok(())
  }

  fn projection(&self, run_id: &str) -> Result<RunProjection, RuntimeExecutionError> {
    Ok(self.projection(run_id)?)
  }

  fn events(&self, run_id: &str) -> Result<Vec<RunEvent>, RuntimeExecutionError> {
    Ok(self.events(run_id).to_vec())
  }

  fn ready_node_ids(&self, run_id: &str) -> Result<Vec<String>, RuntimeExecutionError> {
    Ok(self.ready_node_ids(run_id)?)
  }

  fn correction_action_externally_resolved(
    &self,
    _run_id: &str,
    _saga_id: &str,
    _action_id: &str,
  ) -> Result<bool, RuntimeExecutionError> {
    Ok(false)
  }
}

impl RuntimeDagEngine for DurableDagEngine {
  fn workflow(&self) -> &CompiledWorkflowDefinition {
    self.workflow()
  }

  fn definition_hash(&self) -> &str {
    self.definition_hash()
  }

  fn start_run(
    &mut self,
    run_id: &str,
    trigger: Map<String, Value>,
  ) -> Result<(), RuntimeExecutionError> {
    if self.event_schema_version() >= crate::RUN_EVENT_SCHEMA_VERSION_V7 {
      let trigger_definition = self
        .workflow()
        .triggers
        .iter()
        .find(|candidate| candidate.handler == "trigger.manual")
        .ok_or_else(|| {
          RuntimeExecutionError::InvalidConfiguration(
            "a direct Model v7+ run requires a manual trigger".to_string(),
          )
        })?;
      DurableDagEngine::append_payload(
        self,
        generated_id("evt"),
        run_id,
        chrono::Utc::now(),
        RunEventPayload::RunStarted(crate::RunStartedData {
          workflow_id: self.workflow().workflow_id.clone(),
          definition_hash: self.definition_hash().to_string(),
          trigger_id: Some(trigger_definition.id.clone()),
          trigger_handler: Some(trigger_definition.handler.clone()),
          trigger_occurrence_id: Some(generated_id("occ")),
          ingress: None,
          trigger,
        }),
      )?;
      return Ok(());
    }
    self.start_run(generated_id("evt"), run_id, chrono::Utc::now(), trigger)?;
    Ok(())
  }

  fn append_payload(
    &mut self,
    run_id: &str,
    payload: RunEventPayload,
  ) -> Result<(), RuntimeExecutionError> {
    self.append_payload(generated_id("evt"), run_id, chrono::Utc::now(), payload)?;
    Ok(())
  }

  fn append_scoped_payload(
    &mut self,
    run_id: &str,
    iteration: ForEachIterationScope,
    payload: RunEventPayload,
  ) -> Result<(), RuntimeExecutionError> {
    self.append_scoped_payload(
      generated_id("evt"),
      run_id,
      chrono::Utc::now(),
      iteration,
      payload,
    )?;
    Ok(())
  }

  fn append_scoped_payloads(
    &mut self,
    run_id: &str,
    iteration: ForEachIterationScope,
    payloads: Vec<RunEventPayload>,
  ) -> Result<(), RuntimeExecutionError> {
    self.append_scoped_payloads_atomically(run_id, iteration, payloads)?;
    Ok(())
  }

  fn settle_workflow_timeout(
    &mut self,
    run_id: &str,
    occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<bool, RuntimeExecutionError> {
    Ok(matches!(
      DurableDagEngine::settle_run_timeout(self, run_id, occurred_at)?,
      RunTimeoutSettlement::TimedOut { .. }
    ))
  }

  fn decide_run_cancelled(
    &mut self,
    run_id: &str,
    cancellation_request_id: String,
    occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::decide_run_outcome(
      self,
      run_id,
      RunOutcomeDecidedData::Cancelled {
        cancellation_request_id,
      },
      occurred_at,
    )?;
    Ok(())
  }

  fn decide_run_failed(
    &mut self,
    run_id: &str,
    failure: LifecycleFailure,
    occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::decide_run_outcome(
      self,
      run_id,
      RunOutcomeDecidedData::Failed { failure },
      occurred_at,
    )?;
    Ok(())
  }

  fn record_step_attempt_failure(
    &mut self,
    run_id: &str,
    failed_at: chrono::DateTime<chrono::Utc>,
    failure: StepAttemptFailedData,
  ) -> Result<StepFailureDisposition, RuntimeExecutionError> {
    if self.event_schema_version() >= crate::RUN_EVENT_SCHEMA_VERSION_V6 {
      return Ok(
        DurableDagEngine::record_step_attempt_failure(self, run_id, failed_at, failure)?
          .disposition,
      );
    }
    if self
      .workflow()
      .parallel_group_for_child(&failure.node_id)
      .is_some()
    {
      DurableDagEngine::append_payload(
        self,
        generated_id("evt"),
        run_id,
        failed_at,
        RunEventPayload::StepAttemptFailed(failure),
      )?;
      return Ok(StepFailureDisposition::StepFailed);
    }
    let run_failed = attempt_run_failed_data(self.event_schema_version(), &failure);
    self.append_payloads(
      run_id,
      vec![
        RunEventPayload::StepAttemptFailed(failure),
        RunEventPayload::RunFailed(run_failed),
      ],
    )?;
    Ok(StepFailureDisposition::RunFailed)
  }

  fn projection(&self, run_id: &str) -> Result<RunProjection, RuntimeExecutionError> {
    Ok(self.projection(run_id)?)
  }

  fn events(&self, run_id: &str) -> Result<Vec<RunEvent>, RuntimeExecutionError> {
    Ok(self.events(run_id)?)
  }

  fn ready_node_ids(&self, run_id: &str) -> Result<Vec<String>, RuntimeExecutionError> {
    Ok(self.ready_node_ids(run_id)?)
  }

  fn correction_action_externally_resolved(
    &self,
    run_id: &str,
    saga_id: &str,
    action_id: &str,
  ) -> Result<bool, RuntimeExecutionError> {
    Ok(DurableDagEngine::correction_action_externally_resolved(
      self, run_id, saga_id, action_id,
    )?)
  }

  fn request_approval(
    &mut self,
    run_id: &str,
    occurred_at: chrono::DateTime<chrono::Utc>,
    request: ApprovalRequestedData,
  ) -> Result<IssuedApprovalToken, RuntimeExecutionError> {
    Ok(DurableDagEngine::request_approval(
      self,
      run_id,
      occurred_at,
      request,
    )?)
  }

  fn request_rich_approval(
    &mut self,
    run_id: &str,
    occurred_at: chrono::DateTime<chrono::Utc>,
    binding: &BoundRichApprovalReview,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::request_rich_approval(self, run_id, occurred_at, binding)?;
    Ok(())
  }

  fn settle_pending_rich_approval(
    &mut self,
    run_id: &str,
    reason: RichApprovalResolutionReason,
    settled_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<bool, RuntimeExecutionError> {
    Ok(DurableDagEngine::settle_pending_rich_approval(
      self, run_id, reason, settled_at,
    )?)
  }

  fn register_durable_wait(
    &mut self,
    run_id: &str,
    occurred_at: chrono::DateTime<chrono::Utc>,
    registration: crate::WaitRegisteredData,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::register_durable_wait(self, run_id, occurred_at, registration)?;
    Ok(())
  }

  fn register_lock_waiter(
    &mut self,
    run_id: &str,
    lock_id: &str,
    key: &str,
    occurred_at: chrono::DateTime<chrono::Utc>,
    deadline: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::register_lock_waiter(self, run_id, lock_id, key, occurred_at, deadline)?;
    Ok(())
  }

  fn release_lock(
    &mut self,
    run_id: &str,
    lock_id: &str,
    result: Value,
    occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::release_lock(self, run_id, lock_id, result, occurred_at)?;
    Ok(())
  }

  fn timeout_lock_waiter(
    &mut self,
    run_id: &str,
    lock_id: &str,
    occurred_at: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::timeout_lock_waiter(self, run_id, lock_id, occurred_at)?;
    Ok(())
  }

  fn renew_lock_lease(
    &mut self,
    run_id: &str,
    lock_id: &str,
    now: chrono::DateTime<chrono::Utc>,
  ) -> Result<crate::LockLeaseMaintenance, RuntimeExecutionError> {
    Ok(DurableDagEngine::renew_lock_lease(
      self, run_id, lock_id, now,
    )?)
  }

  fn renew_iteration_lock_lease(
    &mut self,
    run_id: &str,
    lock_id: &str,
    iteration: ForEachIterationScope,
    now: chrono::DateTime<chrono::Utc>,
  ) -> Result<crate::LockLeaseMaintenance, RuntimeExecutionError> {
    Ok(DurableDagEngine::renew_iteration_lock_lease(
      self, run_id, lock_id, iteration, now,
    )?)
  }

  fn cancel_lock_waiters(
    &mut self,
    run_id: &str,
    cancellation_id: &str,
    now: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::cancel_lock_waiters(self, run_id, cancellation_id, now)?;
    Ok(())
  }

  fn register_iteration_lock_waiter(
    &mut self,
    run_id: &str,
    lock_id: &str,
    key: &str,
    iteration: ForEachIterationScope,
    now: chrono::DateTime<chrono::Utc>,
    deadline: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::register_iteration_lock_waiter(
      self, run_id, lock_id, key, iteration, now, deadline,
    )?;
    Ok(())
  }

  fn release_iteration_lock(
    &mut self,
    run_id: &str,
    lock_id: &str,
    iteration: ForEachIterationScope,
    result: Value,
    now: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::release_iteration_lock(self, run_id, lock_id, iteration, result, now)?;
    Ok(())
  }

  fn timeout_iteration_lock_waiter(
    &mut self,
    run_id: &str,
    lock_id: &str,
    iteration: ForEachIterationScope,
    now: chrono::DateTime<chrono::Utc>,
  ) -> Result<(), RuntimeExecutionError> {
    DurableDagEngine::timeout_iteration_lock_waiter(self, run_id, lock_id, iteration, now)?;
    Ok(())
  }

  fn settle_timer_wait(
    &mut self,
    run_id: &str,
    wait_id: &str,
    now: chrono::DateTime<chrono::Utc>,
  ) -> Result<crate::TimerWaitSettlement, RuntimeExecutionError> {
    Ok(DurableDagEngine::settle_timer_wait(
      self, run_id, wait_id, now,
    )?)
  }

  fn reissue_waiting_outcome(
    &mut self,
    run_id: &str,
    now: chrono::DateTime<chrono::Utc>,
  ) -> Result<WorkflowRuntimeOutcome, RuntimeExecutionError> {
    let projection = self.projection(run_id)?;
    if let Some(request) = projection
      .rich_approval_requests
      .values()
      .find(|request| matches!(request.status, crate::RichApprovalRequestStatus::Waiting))
      .cloned()
    {
      let approval = self
        .workflow()
        .approval(&request.approval_id)
        .ok_or_else(|| {
          RuntimeExecutionError::Stalled(
            "waiting run references an unknown rich approval definition".to_string(),
          )
        })?;
      let inspection = DurableDagEngine::inspect_rich_approval(self, run_id, &request.request_id)?;
      let expires_at = approval
        .timeout_ms
        .map(|milliseconds| {
          i64::try_from(milliseconds)
            .map(chrono::Duration::milliseconds)
            .map(|duration| request.requested_at + duration)
        })
        .transpose()
        .map_err(|_| {
          RuntimeExecutionError::Stalled(
            "rich approval timeout exceeds the clock range".to_string(),
          )
        })?;
      let waiting_expires_at = match (expires_at, projection.timeout_at) {
        (Some(approval), Some(workflow)) => Some(approval.min(workflow)),
        (Some(approval), None) => Some(approval),
        (None, Some(workflow)) => Some(workflow),
        (None, None) => None,
      };
      if waiting_expires_at.is_some_and(|deadline| now >= deadline) {
        return Err(RuntimeExecutionError::Stalled(
          "rich approval deadline must be settled before wait reissue".to_string(),
        ));
      }
      let on_timeout = if projection
        .timeout_at
        .is_some_and(|workflow| expires_at.is_none_or(|approval| workflow <= approval))
      {
        ApprovalTimeoutPolicy::Fail
      } else if approval.on_timeout == "reject" {
        ApprovalTimeoutPolicy::Reject
      } else {
        ApprovalTimeoutPolicy::Fail
      };
      let workflow_version = self
        .workflow()
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.version.as_deref())
        .unwrap_or("0.0.0")
        .to_string();
      return Ok(rich_waiting_outcome(
        self.workflow().workflow_id.clone(),
        run_id.to_string(),
        approval,
        waiting_expires_at,
        on_timeout,
        BoundRichApprovalReview {
          profile: crate::RICH_PLAN_APPROVAL_PROFILE.to_string(),
          definition_hash: self.definition_hash().to_string(),
          workflow_id: self.workflow().workflow_id.clone(),
          workflow_version,
          run_id: run_id.to_string(),
          approval_id: request.approval_id,
          request_id: request.request_id,
          artifacts: inspection.review.artifacts.clone(),
          review: inspection.review,
          required_approvals: inspection.required,
          required_roles: inspection.required_roles,
          directory_version: inspection.directory_version,
          review_digest: inspection.review_digest,
        },
        request
          .decisions
          .values()
          .filter(|decision| decision.decision == crate::ApprovalDecision::Approved)
          .count() as u32,
        request.represented_roles,
      ));
    }
    if let Some(request) = projection
      .approval_requests
      .values()
      .find(|request| matches!(request.status, ApprovalRequestStatus::Waiting))
      .cloned()
    {
      let approval = self
        .workflow()
        .approval(&request.approval_id)
        .ok_or_else(|| {
          RuntimeExecutionError::Stalled(
            "waiting run references an unknown approval definition".to_string(),
          )
        })?;
      let token = DurableDagEngine::reissue_waiting_approval_token(
        self,
        run_id,
        &request.approval_id,
        &request.request_id,
        now,
      )?;
      return Ok(waiting_outcome(
        self.workflow().workflow_id.clone(),
        run_id.to_string(),
        approval,
        request.request_id,
        request.expires_at,
        request.on_timeout,
        token,
      ));
    }
    let wait = projection
      .durable_waits
      .values()
      .find(|wait| wait.status == crate::DurableWaitStatus::Registered)
      .ok_or_else(|| {
        RuntimeExecutionError::Stalled(
          "waiting run has no unresolved approval or durable wait".to_string(),
        )
      })?;
    if wait.kind == crate::DurableWaitKind::Signal {
      return Ok(WorkflowRuntimeOutcome::WaitingSignal {
        contract: RUNTIME_OUTCOME_CONTRACT,
        version: RUNTIME_OUTCOME_VERSION,
        workflow_id: self.workflow().workflow_id.clone(),
        run_id: run_id.to_string(),
        wait: WaitingWorkflowSignal {
          wait_id: wait.wait_id.clone(),
          node_id: wait.node_id.clone(),
          registered_at: wait.registered_at,
          signals: wait.signals.clone(),
          deadline: wait.deadline,
        },
      });
    }
    let deadline = wait.deadline.ok_or_else(|| {
      RuntimeExecutionError::Stalled("durable timer wait has no deadline".to_string())
    })?;
    Ok(WorkflowRuntimeOutcome::WaitingTime {
      contract: RUNTIME_OUTCOME_CONTRACT,
      version: RUNTIME_OUTCOME_VERSION,
      workflow_id: self.workflow().workflow_id.clone(),
      run_id: run_id.to_string(),
      wait: WaitingWorkflowTimer {
        wait_id: wait.wait_id.clone(),
        node_id: wait.node_id.clone(),
        kind: wait.kind,
        registered_at: wait.registered_at,
        deadline,
      },
    })
  }

  fn append_payloads(
    &mut self,
    run_id: &str,
    payloads: Vec<RunEventPayload>,
  ) -> Result<(), RuntimeExecutionError> {
    self.append_payloads_atomically(run_id, payloads)?;
    Ok(())
  }

  fn publish_pure_result(
    &mut self,
    run_id: &str,
    node_id: &str,
    output: Value,
  ) -> Result<(), RuntimeExecutionError> {
    let invocation_id = generated_id("inv");
    self.publish_pure_result(run_id, node_id, &invocation_id, output)?;
    Ok(())
  }
}

fn generated_id(prefix: &str) -> String {
  format!("{prefix}_{}", Uuid::new_v4().simple())
}
