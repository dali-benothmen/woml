//! runtime progress.

use super::{RuntimeDagEngine, RuntimeExecutionError, RuntimeExecutionOptions};
use crate::interval::IntervalProgress;
use crate::projection::ForEachIterationStatus;
use crate::projection::ForEachStatus;
use crate::schedule::ScheduleProgress;
use crate::PolicyWaitingFor;
use serde::Serialize;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub const EXECUTION_PROGRESS_CONTRACT: &str = "woml.execution-progress";

pub const EXECUTION_PROGRESS_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExecutionProgress {
  ForEachProgress {
    contract: &'static str,
    version: u32,
    #[serde(rename = "runId")]
    run_id: String,
    #[serde(rename = "forEachId")]
    for_each_id: String,
    status: &'static str,
    total: u32,
    succeeded: u32,
    failed: u32,
    skipped: u32,
    active: u32,
    pending: u32,
    concurrency: u32,
  },
  StepAttemptFailed {
    contract: &'static str,
    version: u32,
    #[serde(rename = "runId")]
    run_id: String,
    #[serde(rename = "nodeId")]
    node_id: String,
    attempt: u32,
    #[serde(rename = "maxAttempts")]
    max_attempts: u32,
    #[serde(rename = "failureCode")]
    failure_code: String,
  },
  StepRetryScheduled {
    contract: &'static str,
    version: u32,
    #[serde(rename = "runId")]
    run_id: String,
    #[serde(rename = "nodeId")]
    node_id: String,
    #[serde(rename = "nextAttempt")]
    next_attempt: u32,
    #[serde(rename = "maxAttempts")]
    max_attempts: u32,
    #[serde(rename = "scheduledAt")]
    scheduled_at: chrono::DateTime<chrono::Utc>,
  },
  StepAttemptSucceeded {
    contract: &'static str,
    version: u32,
    #[serde(rename = "runId")]
    run_id: String,
    #[serde(rename = "nodeId")]
    node_id: String,
    attempt: u32,
    #[serde(rename = "maxAttempts")]
    max_attempts: u32,
  },
}

pub type ExecutionProgressReporter = Arc<dyn Fn(ExecutionProgress) + Send + Sync>;

pub const LIFECYCLE_PROGRESS_PROFILE: &str = "woml.lifecycle-progress/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleProgressPhase {
  HookRequested,
  ActionStarted,
  ActionSucceeded,
  ActionFailed,
  HookCompleted,
  RunFinalizing,
  RunFinalized,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleProgress {
  pub profile: &'static str,
  pub run_id: String,
  pub workflow_id: String,
  pub phase: LifecycleProgressPhase,
  pub hook_id: String,
  pub action_id: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub step_id: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub code: Option<String>,
}

pub type LifecycleProgressReporter = Arc<dyn Fn(LifecycleProgress) + Send + Sync>;

pub const RUNTIME_POLICY_PROGRESS_PROFILE: &str = "woml.runtime-policy-progress/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimePolicyProgressPhase {
  Queued,
  Eligible,
  Started,
  TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimePolicyProgress {
  pub profile: &'static str,
  pub run_id: String,
  pub workflow_id: String,
  pub phase: RuntimePolicyProgressPhase,
  pub queue: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub waiting_for: Option<PolicyWaitingFor>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub eligible_at: Option<chrono::DateTime<chrono::Utc>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub code: Option<String>,
}

pub type RuntimePolicyProgressReporter = Arc<dyn Fn(RuntimePolicyProgress) + Send + Sync>;

impl RuntimeExecutionOptions {
  pub fn report_telemetry_admission_context(
    &self,
    run_id: &str,
    context: crate::InboundTraceContext,
    occurred_at: chrono::DateTime<chrono::Utc>,
  ) {
    if !self.telemetry_propagation.load(Ordering::Acquire) {
      return;
    }
    let (Some(reporter), Some(signal_context)) = (
      self.telemetry_admission_link_reporter.as_ref(),
      self.telemetry_context.as_ref(),
    ) else {
      return;
    };
    let (status, trace_id, span_id, sampled) = match context {
      crate::InboundTraceContext::Absent => return,
      crate::InboundTraceContext::Invalid => (
        crate::TelemetryAdmissionLinkStatus::Invalid,
        None,
        None,
        None,
      ),
      crate::InboundTraceContext::Valid(context) => (
        crate::TelemetryAdmissionLinkStatus::Linked,
        Some(context.trace_id),
        Some(context.span_id),
        Some(context.sampled),
      ),
    };
    reporter(crate::TelemetryAdmissionLinkV1 {
      profile: crate::TELEMETRY_ADMISSION_LINK_PROFILE,
      occurred_at,
      runtime_instance_id: signal_context.runtime_instance_id.clone(),
      deployment_id: signal_context.deployment_id.clone(),
      run_id: run_id.to_string(),
      status,
      trace_id,
      span_id,
      sampled,
    });
  }
  pub(crate) fn report_schedule(&self, progress: ScheduleProgress) {
    if let Some(reporter) = &self.schedule_progress_reporter {
      reporter(progress);
    }
  }
  pub(crate) fn report_interval(&self, progress: IntervalProgress) {
    if let Some(reporter) = &self.interval_progress_reporter {
      reporter(progress);
    }
  }
  pub(super) fn report_runtime_policy(&self, progress: RuntimePolicyProgress) {
    if let Some(reporter) = &self.runtime_policy_progress_reporter {
      reporter(progress);
    }
  }
  pub(super) fn report(&self, progress: ExecutionProgress) {
    if let Some(reporter) = &self.progress_reporter {
      reporter(progress);
    }
  }
  pub(super) fn report_lifecycle(&self, progress: LifecycleProgress) {
    if let Some(reporter) = &self.lifecycle_progress_reporter {
      reporter(progress);
    }
  }
}

pub(super) fn retry_max_attempts<E: RuntimeDagEngine>(engine: &E, node_id: &str) -> u32 {
  engine
    .workflow()
    .node(node_id)
    .and_then(|node| node.retry_policy.as_ref())
    .map_or(1, |policy| policy.max_attempts)
}

pub(super) fn report_attempt_failed<E: RuntimeDagEngine>(
  engine: &E,
  options: &RuntimeExecutionOptions,
  run_id: &str,
  node_id: &str,
  attempt: u32,
  failure_code: String,
) {
  let max_attempts = retry_max_attempts(engine, node_id);
  if max_attempts > 1 {
    options.report(ExecutionProgress::StepAttemptFailed {
      contract: EXECUTION_PROGRESS_CONTRACT,
      version: EXECUTION_PROGRESS_VERSION,
      run_id: run_id.to_string(),
      node_id: node_id.to_string(),
      attempt,
      max_attempts,
      failure_code,
    });
  }
}

pub(super) fn report_retry_scheduled<E: RuntimeDagEngine>(
  engine: &E,
  options: &RuntimeExecutionOptions,
  run_id: &str,
  node_id: &str,
  next_attempt: u32,
  scheduled_at: chrono::DateTime<chrono::Utc>,
) {
  options.report(ExecutionProgress::StepRetryScheduled {
    contract: EXECUTION_PROGRESS_CONTRACT,
    version: EXECUTION_PROGRESS_VERSION,
    run_id: run_id.to_string(),
    node_id: node_id.to_string(),
    next_attempt,
    max_attempts: retry_max_attempts(engine, node_id),
    scheduled_at,
  });
}

pub(super) fn report_attempt_succeeded<E: RuntimeDagEngine>(
  engine: &E,
  options: &RuntimeExecutionOptions,
  run_id: &str,
  node_id: &str,
  attempt: u32,
) {
  let max_attempts = retry_max_attempts(engine, node_id);
  if max_attempts > 1 && attempt > 1 {
    options.report(ExecutionProgress::StepAttemptSucceeded {
      contract: EXECUTION_PROGRESS_CONTRACT,
      version: EXECUTION_PROGRESS_VERSION,
      run_id: run_id.to_string(),
      node_id: node_id.to_string(),
      attempt,
      max_attempts,
    });
  }
}

pub(super) fn report_for_each_progress<E: RuntimeDagEngine>(
  engine: &E,
  options: &RuntimeExecutionOptions,
  run_id: &str,
  for_each_id: &str,
) -> Result<(), RuntimeExecutionError> {
  let projection = engine.projection(run_id)?;
  let Some(state) = projection.for_each.get(for_each_id) else {
    return Ok(());
  };
  let succeeded = state
    .iterations
    .values()
    .filter(|status| matches!(status, ForEachIterationStatus::Succeeded { .. }))
    .count() as u32;
  let failed = state
    .iterations
    .values()
    .filter(|status| matches!(status, ForEachIterationStatus::Failed { .. }))
    .count() as u32;
  let skipped = state
    .iterations
    .values()
    .filter(|status| matches!(status, ForEachIterationStatus::Skipped))
    .count() as u32;
  let active = state
    .iterations
    .values()
    .filter(|status| matches!(status, ForEachIterationStatus::Started))
    .count() as u32;
  let pending = state
    .total
    .saturating_sub(succeeded + failed + skipped + active);
  let status = match state.status {
    ForEachStatus::Open => "running",
    ForEachStatus::Succeeded => "succeeded",
    ForEachStatus::Failed => "failed",
    ForEachStatus::Cancelled => "cancelled",
  };
  options.report(ExecutionProgress::ForEachProgress {
    contract: EXECUTION_PROGRESS_CONTRACT,
    version: EXECUTION_PROGRESS_VERSION,
    run_id: run_id.to_string(),
    for_each_id: for_each_id.to_string(),
    status,
    total: state.total,
    succeeded,
    failed,
    skipped,
    active,
    pending,
    concurrency: state.concurrency,
  });
  Ok(())
}
