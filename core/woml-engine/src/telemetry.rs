use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::event::{
  BusinessOutcome, CorrectiveActionKind, DurableWaitKind, RunEvent, RunEventPayload,
};

pub const TELEMETRY_SIGNAL_PROFILE: &str = "woml.telemetry-signal/v1";
const MAX_ID_BYTES: usize = 320;
const MAX_CODE_BYTES: usize = 128;
const MAX_DURATION_MS: f64 = 315_576_000_000.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetrySignalContext {
  pub runtime_instance_id: String,
  pub deployment_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TelemetrySignalKind {
  Runtime,
  Run,
  Attempt,
  Retry,
  Wait,
  Approval,
  Lock,
  Capability,
  WorkflowCall,
  Compensation,
  Cleanup,
  Recovery,
  RemoteDispatch,
}

impl TelemetrySignalKind {
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::Runtime => "runtime",
      Self::Run => "run",
      Self::Attempt => "attempt",
      Self::Retry => "retry",
      Self::Wait => "wait",
      Self::Approval => "approval",
      Self::Lock => "lock",
      Self::Capability => "capability",
      Self::WorkflowCall => "workflow_call",
      Self::Compensation => "compensation",
      Self::Cleanup => "cleanup",
      Self::Recovery => "recovery",
      Self::RemoteDispatch => "remote_dispatch",
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TelemetrySignalStatus {
  Admitted,
  Started,
  Succeeded,
  Failed,
  Cancelled,
  TimedOut,
  Waiting,
  Resolved,
  Released,
  Degraded,
  Stopped,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum TelemetryAttributeValue {
  String(String),
  Boolean(bool),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetrySignalV1 {
  pub profile: &'static str,
  pub signal_id: String,
  pub kind: TelemetrySignalKind,
  pub occurred_at: DateTime<Utc>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub started_at: Option<DateTime<Utc>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub ended_at: Option<DateTime<Utc>>,
  pub runtime_instance_id: String,
  pub deployment_id: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub workflow_id: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub run_id: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub node_id: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub attempt: Option<u32>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub invocation_id: Option<String>,
  pub status: TelemetrySignalStatus,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub code: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub duration_ms: Option<f64>,
  pub attributes: BTreeMap<String, TelemetryAttributeValue>,
}

impl TelemetrySignalV1 {
  pub fn validate(&self) -> Result<(), &'static str> {
    if self.profile != TELEMETRY_SIGNAL_PROFILE {
      return Err("invalid telemetry profile");
    }
    for value in [
      Some(self.signal_id.as_str()),
      Some(self.runtime_instance_id.as_str()),
      Some(self.deployment_id.as_str()),
      self.workflow_id.as_deref(),
      self.run_id.as_deref(),
      self.node_id.as_deref(),
      self.invocation_id.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
      if !safe_id(value) {
        return Err("invalid telemetry identity");
      }
    }
    if self
      .attempt
      .is_some_and(|attempt| attempt == 0 || attempt > 1_000_000)
    {
      return Err("invalid telemetry attempt");
    }
    if self
      .duration_ms
      .is_some_and(|duration| !duration.is_finite() || !(0.0..=MAX_DURATION_MS).contains(&duration))
    {
      return Err("invalid telemetry duration");
    }
    if matches!((self.started_at, self.ended_at), (Some(start), Some(end)) if end < start) {
      return Err("invalid telemetry interval");
    }
    if self.code.as_deref().is_some_and(|code| !safe_code(code)) {
      return Err("invalid telemetry code");
    }
    if self.attributes.len() > 16
      || self
        .attributes
        .iter()
        .any(|(key, value)| !safe_attribute(key, value))
    {
      return Err("invalid telemetry attributes");
    }
    Ok(())
  }
}

pub type TelemetrySignalReporter = Arc<dyn Fn(TelemetrySignalV1) + Send + Sync>;

/// Isolates execution from telemetry consumers. A full queue drops the newest
/// best-effort record; a panicking or disconnected sink only stops its worker.
pub fn bounded_telemetry_reporter(
  capacity: usize,
  sink: TelemetrySignalReporter,
) -> TelemetrySignalReporter {
  let (sender, receiver) = std::sync::mpsc::sync_channel::<TelemetrySignalV1>(capacity.max(1));
  let _ = std::thread::Builder::new()
    .name("woml-telemetry".to_string())
    .spawn(move || {
      while let Ok(signal) = receiver.recv() {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sink(signal)));
      }
    });
  Arc::new(move |signal| {
    let _ = sender.try_send(signal);
  })
}

#[derive(Debug, Clone)]
struct IntervalIdentity {
  started_at: DateTime<Utc>,
  node_id: Option<String>,
  attempt: Option<u32>,
  invocation_id: Option<String>,
  category: Option<String>,
}

pub fn project_telemetry_signals(
  events: &[RunEvent],
  workflow_id: &str,
  context: &TelemetrySignalContext,
) -> Vec<TelemetrySignalV1> {
  let Some(first) = events.first() else {
    return Vec::new();
  };
  let mut signals = Vec::new();
  let mut run_started_at = first.occurred_at;
  let mut run_trigger = None;
  let mut run_code = None;
  let mut workflow_call = false;
  let mut attempts = HashMap::<String, IntervalIdentity>::new();
  let mut capabilities = HashMap::<String, IntervalIdentity>::new();
  let mut waits = HashMap::<String, IntervalIdentity>::new();
  let mut approvals = HashMap::<String, IntervalIdentity>::new();
  let mut locks = HashMap::<String, IntervalIdentity>::new();
  let mut corrections = HashMap::<String, IntervalIdentity>::new();
  let mut lifecycle_actions = HashMap::<String, IntervalIdentity>::new();
  let mut for_each_groups = HashMap::<String, IntervalIdentity>::new();
  let mut remote_dispatches = HashMap::<String, IntervalIdentity>::new();

  for event in events {
    // Iteration-local events can be arbitrarily high-cardinality. The durable
    // for-each open/settled pair below is the reviewed aggregate trace unit.
    if event.iteration.is_some()
      && matches!(
        &event.payload,
        RunEventPayload::StepAttemptStarted(_)
          | RunEventPayload::StepAttemptSucceeded(_)
          | RunEventPayload::StepAttemptFailed(_)
          | RunEventPayload::StepRetryScheduled(_)
          | RunEventPayload::OperationStarted(_)
          | RunEventPayload::OperationSucceeded(_)
          | RunEventPayload::OperationFailed(_)
      )
    {
      continue;
    }
    match &event.payload {
      RunEventPayload::RemoteDispatchTransitioned(data) => {
        use crate::RemoteDispatchStateV1;
        match data.state {
          RemoteDispatchStateV1::Requested => {
            remote_dispatches.insert(
              data.dispatch_id.clone(),
              IntervalIdentity {
                started_at: event.occurred_at,
                node_id: Some(data.node_id.clone()),
                attempt: Some(data.attempt),
                invocation_id: Some(data.invocation_id.clone()),
                category: Some("queue".to_string()),
              },
            );
          }
          RemoteDispatchStateV1::Started => {
            if let Some(queued) = remote_dispatches.remove(&data.dispatch_id) {
              signals.push(interval_signal(
                event,
                workflow_id,
                context,
                TelemetrySignalKind::RemoteDispatch,
                TelemetrySignalStatus::Started,
                queued,
                None,
                [("remote.phase", string_attribute("queue"))],
              ));
            }
            remote_dispatches.insert(
              data.dispatch_id.clone(),
              IntervalIdentity {
                started_at: event.occurred_at,
                node_id: Some(data.node_id.clone()),
                attempt: Some(data.attempt),
                invocation_id: Some(data.invocation_id.clone()),
                category: Some("execution".to_string()),
              },
            );
          }
          RemoteDispatchStateV1::Succeeded
          | RemoteDispatchStateV1::Failed
          | RemoteDispatchStateV1::Cancelled
          | RemoteDispatchStateV1::Ambiguous => {
            if let Some(started) = remote_dispatches.remove(&data.dispatch_id) {
              let (status, code, outcome) = match data.state {
                RemoteDispatchStateV1::Succeeded => {
                  (TelemetrySignalStatus::Succeeded, None, "succeeded")
                }
                RemoteDispatchStateV1::Failed => (
                  TelemetrySignalStatus::Failed,
                  Some("WOML_REMOTE_EXECUTION_FAILED".to_string()),
                  "failed",
                ),
                RemoteDispatchStateV1::Cancelled => {
                  (TelemetrySignalStatus::Cancelled, None, "cancelled")
                }
                RemoteDispatchStateV1::Ambiguous => (
                  TelemetrySignalStatus::Degraded,
                  Some("WOML_REMOTE_DISPATCH_AMBIGUOUS".to_string()),
                  "ambiguous",
                ),
                _ => unreachable!(),
              };
              signals.push(interval_signal(
                event,
                workflow_id,
                context,
                TelemetrySignalKind::RemoteDispatch,
                status,
                started,
                code,
                [
                  ("remote.phase", string_attribute("execution")),
                  ("remote.outcome", string_attribute(outcome)),
                ],
              ));
            }
          }
          RemoteDispatchStateV1::Offered
          | RemoteDispatchStateV1::Accepted
          | RemoteDispatchStateV1::CancelRequested
          | RemoteDispatchStateV1::Abandoned => {}
        }
      }
      RunEventPayload::RunAdmitted(data) => {
        run_started_at = data.admitted_at;
        run_trigger = Some(data.trigger.handler.clone());
      }
      RunEventPayload::RunExecutionStarted(data) => {
        run_started_at = data.started_at;
        signals.push(point_signal(
          event,
          workflow_id,
          context,
          TelemetrySignalKind::Run,
          TelemetrySignalStatus::Started,
          None,
          None,
          None,
          None,
          [(
            "trigger.type",
            run_trigger
              .as_deref()
              .and_then(safe_category)
              .map(TelemetryAttributeValue::String)
              .unwrap_or_else(|| string_attribute("unknown")),
          )],
        ));
      }
      RunEventPayload::RunTimeoutReached(data) => run_code = Some(data.code.clone()),
      RunEventPayload::RunOutcomeDecided(crate::RunOutcomeDecidedData::Failed { failure }) => {
        run_code = Some(failure.code.clone());
      }
      RunEventPayload::RunStarted(data) => {
        run_started_at = event.occurred_at;
        run_trigger = data.trigger_handler.clone();
        workflow_call = data.ingress.is_some();
        signals.push(point_signal(
          event,
          workflow_id,
          context,
          TelemetrySignalKind::Run,
          TelemetrySignalStatus::Started,
          None,
          None,
          None,
          None,
          [(
            "trigger.type",
            data
              .trigger_handler
              .as_deref()
              .and_then(safe_category)
              .map(TelemetryAttributeValue::String)
              .unwrap_or_else(|| string_attribute("unknown")),
          )],
        ));
      }
      RunEventPayload::StepAttemptStarted(data) => {
        attempts.insert(
          data.invocation_id.clone(),
          IntervalIdentity {
            started_at: event.occurred_at,
            node_id: Some(data.node_id.clone()),
            attempt: Some(data.attempt),
            invocation_id: Some(data.invocation_id.clone()),
            category: safe_category(&data.handler),
          },
        );
      }
      RunEventPayload::StepAttemptSucceeded(data) => {
        if let Some(started) = attempts.remove(&data.invocation_id) {
          signals.push(interval_signal(
            event,
            workflow_id,
            context,
            TelemetrySignalKind::Attempt,
            TelemetrySignalStatus::Succeeded,
            started,
            None,
            [("node.kind", string_attribute("step"))],
          ));
        }
      }
      RunEventPayload::StepAttemptFailed(data) => {
        if let Some(started) = attempts.remove(&data.invocation_id) {
          signals.push(interval_signal(
            event,
            workflow_id,
            context,
            TelemetrySignalKind::Attempt,
            TelemetrySignalStatus::Failed,
            started,
            Some(data.failure.code.clone()),
            [("node.kind", string_attribute("step"))],
          ));
        }
      }
      RunEventPayload::StepRetryScheduled(data) => signals.push(point_signal(
        event,
        workflow_id,
        context,
        TelemetrySignalKind::Retry,
        TelemetrySignalStatus::Waiting,
        Some(data.node_id.clone()),
        Some(data.next_attempt),
        None,
        None,
        [("retry.scheduled", TelemetryAttributeValue::Boolean(true))],
      )),
      RunEventPayload::ForEachOpened(data) => {
        for_each_groups.insert(
          data.for_each_id.clone(),
          IntervalIdentity {
            started_at: event.occurred_at,
            node_id: Some(data.for_each_id.clone()),
            attempt: Some(1),
            invocation_id: Some(data.for_each_id.clone()),
            category: None,
          },
        );
      }
      RunEventPayload::ForEachSucceeded(data)
      | RunEventPayload::ForEachFailed(data)
      | RunEventPayload::ForEachCancelled(data) => {
        if let Some(started) = for_each_groups.remove(&data.for_each_id) {
          let status = match &event.payload {
            RunEventPayload::ForEachSucceeded(_) => TelemetrySignalStatus::Succeeded,
            RunEventPayload::ForEachFailed(_) => TelemetrySignalStatus::Failed,
            RunEventPayload::ForEachCancelled(_) => TelemetrySignalStatus::Cancelled,
            _ => unreachable!(),
          };
          signals.push(interval_signal(
            event,
            workflow_id,
            context,
            TelemetrySignalKind::Attempt,
            status,
            started,
            None,
            [("node.kind", string_attribute("for_each"))],
          ));
        }
      }
      RunEventPayload::WaitRegistered(data) => {
        waits.insert(
          data.wait_id.clone(),
          IntervalIdentity {
            started_at: data.registered_at,
            node_id: Some(data.node_id.clone()),
            attempt: None,
            invocation_id: Some(data.wait_id.clone()),
            category: Some(wait_kind(data.kind).to_string()),
          },
        );
      }
      RunEventPayload::WaitSatisfied(data) => {
        if let Some(started) = waits.remove(&data.wait_id) {
          signals.push(interval_signal(
            event,
            workflow_id,
            context,
            TelemetrySignalKind::Wait,
            TelemetrySignalStatus::Resolved,
            started,
            None,
            [],
          ));
        }
      }
      RunEventPayload::WaitTimedOut(data) => {
        if let Some(started) = waits.remove(&data.wait_id) {
          signals.push(interval_signal(
            event,
            workflow_id,
            context,
            TelemetrySignalKind::Wait,
            TelemetrySignalStatus::TimedOut,
            started,
            Some("WOML_WAIT_TIMED_OUT".to_string()),
            [],
          ));
        }
      }
      RunEventPayload::WaitCancelled(data) => {
        if let Some(started) = waits.remove(&data.wait_id) {
          signals.push(interval_signal(
            event,
            workflow_id,
            context,
            TelemetrySignalKind::Wait,
            TelemetrySignalStatus::Cancelled,
            started,
            None,
            [],
          ));
        }
      }
      RunEventPayload::ApprovalRequested(data) => {
        approvals.insert(
          data.request_id.clone(),
          IntervalIdentity {
            started_at: event.occurred_at,
            node_id: Some(data.approval_id.clone()),
            attempt: None,
            invocation_id: Some(data.request_id.clone()),
            category: None,
          },
        );
      }
      RunEventPayload::ApprovalResolved(data) => {
        if let Some(started) = approvals.remove(&data.request_id) {
          let timed_out = matches!(
            data.resolution.as_ref(),
            Some(crate::event::ApprovalResolution::TimeoutFailure)
          );
          signals.push(interval_signal(
            event,
            workflow_id,
            context,
            TelemetrySignalKind::Approval,
            if timed_out {
              TelemetrySignalStatus::TimedOut
            } else {
              TelemetrySignalStatus::Resolved
            },
            started,
            timed_out.then(|| "WOML_APPROVAL_TIMED_OUT".to_string()),
            [],
          ));
        }
      }
      RunEventPayload::LockWaitRegistered(data) => {
        locks.insert(
          data.lock_id.clone(),
          IntervalIdentity {
            started_at: data.registered_at,
            node_id: None,
            attempt: None,
            invocation_id: Some(data.lock_id.clone()),
            category: None,
          },
        );
      }
      RunEventPayload::LockAcquired(data) => {
        if let Some(started) = locks.remove(&data.lock_id) {
          signals.push(interval_signal(
            event,
            workflow_id,
            context,
            TelemetrySignalKind::Lock,
            TelemetrySignalStatus::Resolved,
            started,
            None,
            [("lock.outcome", string_attribute("acquired"))],
          ));
        }
        locks.insert(
          data.lock_id.clone(),
          IntervalIdentity {
            started_at: data.acquired_at,
            node_id: None,
            attempt: None,
            invocation_id: Some(data.lock_id.clone()),
            category: None,
          },
        );
      }
      RunEventPayload::LockReleased(data) => {
        if let Some(started) = locks.remove(&data.lock_id) {
          signals.push(interval_signal(
            event,
            workflow_id,
            context,
            TelemetrySignalKind::Lock,
            TelemetrySignalStatus::Released,
            started,
            None,
            [("lock.outcome", string_attribute("released"))],
          ));
        }
      }
      RunEventPayload::LockAcquireTimedOut(data) => {
        if let Some(started) = locks.remove(&data.lock_id) {
          signals.push(interval_signal(
            event,
            workflow_id,
            context,
            TelemetrySignalKind::Lock,
            TelemetrySignalStatus::TimedOut,
            started,
            Some("WOML_LOCK_ACQUIRE_TIMED_OUT".to_string()),
            [("lock.outcome", string_attribute("timed_out"))],
          ));
        }
      }
      RunEventPayload::OperationStarted(data) => {
        capabilities.insert(
          data.call_id.clone(),
          IntervalIdentity {
            started_at: event.occurred_at,
            node_id: Some(data.node_id.clone()),
            attempt: Some(data.attempt_number),
            invocation_id: Some(data.invocation_id.clone()),
            category: safe_category(&data.capability),
          },
        );
      }
      RunEventPayload::OperationSucceeded(data) => {
        if let Some(started) = capabilities.remove(&data.call_id) {
          signals.push(interval_signal(
            event,
            workflow_id,
            context,
            TelemetrySignalKind::Capability,
            TelemetrySignalStatus::Succeeded,
            started,
            None,
            [],
          ));
        }
      }
      RunEventPayload::OperationFailed(data) => {
        if let Some(started) = capabilities.remove(&data.call_id) {
          signals.push(interval_signal(
            event,
            workflow_id,
            context,
            TelemetrySignalKind::Capability,
            TelemetrySignalStatus::Failed,
            started,
            Some(data.failure.code.clone()),
            [],
          ));
        }
      }
      RunEventPayload::LifecycleActionAttemptStarted(data) => {
        lifecycle_actions.insert(
          lifecycle_key(&data.hook_invocation_id, &data.action_id, data.attempt),
          IntervalIdentity {
            started_at: event.occurred_at,
            node_id: Some(data.action_id.clone()),
            attempt: Some(data.attempt),
            invocation_id: Some(data.hook_invocation_id.clone()),
            category: Some("lifecycle".to_string()),
          },
        );
      }
      RunEventPayload::LifecycleActionSucceeded(data) => {
        if let Some(started) = lifecycle_actions.remove(&lifecycle_key(
          &data.hook_invocation_id,
          &data.action_id,
          data.attempt,
        )) {
          signals.push(interval_signal(
            event,
            workflow_id,
            context,
            TelemetrySignalKind::Capability,
            TelemetrySignalStatus::Succeeded,
            started,
            None,
            [],
          ));
        }
      }
      RunEventPayload::LifecycleActionFailed(data) => {
        if let Some(started) = lifecycle_actions.remove(&lifecycle_key(
          &data.hook_invocation_id,
          &data.action_id,
          data.attempt,
        )) {
          signals.push(interval_signal(
            event,
            workflow_id,
            context,
            TelemetrySignalKind::Capability,
            TelemetrySignalStatus::Failed,
            started,
            Some(data.failure.code.clone()),
            [],
          ));
        }
      }
      RunEventPayload::ReusableLifecycleActionStarted(data) => {
        lifecycle_actions.insert(
          reusable_lifecycle_key(&data.invocation_id, &data.action_id),
          IntervalIdentity {
            started_at: event.occurred_at,
            node_id: Some(data.action_id.clone()),
            attempt: Some(1),
            invocation_id: Some(data.invocation_id.clone()),
            category: Some("lifecycle".to_string()),
          },
        );
      }
      RunEventPayload::ReusableLifecycleActionSucceeded(data) => {
        if let Some(started) = lifecycle_actions.remove(&reusable_lifecycle_key(
          &data.invocation_id,
          &data.action_id,
        )) {
          signals.push(interval_signal(
            event,
            workflow_id,
            context,
            TelemetrySignalKind::Capability,
            TelemetrySignalStatus::Succeeded,
            started,
            None,
            [],
          ));
        }
      }
      RunEventPayload::ReusableLifecycleActionFailed(data) => {
        if let Some(started) = lifecycle_actions.remove(&reusable_lifecycle_key(
          &data.invocation_id,
          &data.action_id,
        )) {
          signals.push(interval_signal(
            event,
            workflow_id,
            context,
            TelemetrySignalKind::Capability,
            TelemetrySignalStatus::Failed,
            started,
            Some(data.warning_code.clone()),
            [],
          ));
        }
      }
      RunEventPayload::CorrectionAttemptStarted(data) => {
        corrections.insert(
          correction_key(&data.action_id, data.attempt),
          IntervalIdentity {
            started_at: event.occurred_at,
            node_id: Some(data.action_id.clone()),
            attempt: Some(data.attempt),
            invocation_id: None,
            category: Some(corrective_kind(data.action_kind).to_string()),
          },
        );
      }
      RunEventPayload::CorrectionAttemptSucceeded(data) => {
        if let Some(started) = corrections.remove(&correction_key(&data.action_id, data.attempt)) {
          signals.push(corrective_signal(
            event,
            workflow_id,
            context,
            data.action_kind,
            started,
            TelemetrySignalStatus::Succeeded,
            None,
          ));
        }
      }
      RunEventPayload::CorrectionAttemptFailed(data) => {
        if let Some(started) = corrections.remove(&correction_key(&data.action_id, data.attempt)) {
          signals.push(corrective_signal(
            event,
            workflow_id,
            context,
            data.action_kind,
            started,
            TelemetrySignalStatus::Failed,
            Some(data.code.clone()),
          ));
        }
      }
      RunEventPayload::CorrectionInterrupted(data) => signals.push(point_signal(
        event,
        workflow_id,
        context,
        TelemetrySignalKind::Recovery,
        TelemetrySignalStatus::Degraded,
        Some(data.action_id.clone()),
        Some(data.attempt),
        None,
        Some("WOML_CORRECTION_INTERRUPTED".to_string()),
        [("recovered", TelemetryAttributeValue::Boolean(true))],
      )),
      RunEventPayload::RunFinalized(data) => {
        signals.push(run_signal(
          event,
          workflow_id,
          context,
          run_started_at,
          data.outcome,
          run_trigger.as_deref(),
          run_code.clone(),
        ));
        if workflow_call {
          let mut call = run_signal(
            event,
            workflow_id,
            context,
            run_started_at,
            data.outcome,
            run_trigger.as_deref(),
            run_code.clone(),
          );
          call.kind = TelemetrySignalKind::WorkflowCall;
          call.signal_id = signal_id(event, TelemetrySignalKind::WorkflowCall);
          signals.push(call);
        }
      }
      _ => {}
    }
  }

  for (wait_id, started) in waits {
    let event = events.last().unwrap_or(first);
    let mut signal = interval_signal(
      event,
      workflow_id,
      context,
      TelemetrySignalKind::Wait,
      TelemetrySignalStatus::Waiting,
      started,
      None,
      [],
    );
    signal.signal_id = format!("telemetry_{wait_id}_waiting");
    signal.ended_at = None;
    signal.duration_ms = None;
    signals.push(signal);
  }

  signals.retain(|signal| signal.validate().is_ok());
  signals
}

pub fn trace_id_v1(run_id: &str) -> String {
  digest_prefix(&[b"woml:trace:v1", run_id.as_bytes()], 16)
}

pub fn span_id_v1(run_id: &str, kind: &str, stable_operation_identity: &str) -> String {
  digest_prefix(
    &[
      b"woml:span:v1",
      run_id.as_bytes(),
      kind.as_bytes(),
      stable_operation_identity.as_bytes(),
    ],
    8,
  )
}

fn digest_prefix(parts: &[&[u8]], bytes: usize) -> String {
  let mut digest = Sha256::new();
  for (index, part) in parts.iter().enumerate() {
    if index > 0 {
      digest.update([0]);
    }
    digest.update(part);
  }
  let mut result = hex::encode(&digest.finalize()[..bytes]);
  if result.bytes().all(|byte| byte == b'0') {
    result.replace_range(result.len() - 1.., "1");
  }
  result
}

#[allow(clippy::too_many_arguments)]
fn interval_signal<const N: usize>(
  event: &RunEvent,
  workflow_id: &str,
  context: &TelemetrySignalContext,
  kind: TelemetrySignalKind,
  status: TelemetrySignalStatus,
  started: IntervalIdentity,
  code: Option<String>,
  extra_attributes: [(&str, TelemetryAttributeValue); N],
) -> TelemetrySignalV1 {
  let mut attributes = BTreeMap::new();
  if let Some(category) = started.category {
    let key = match kind {
      TelemetrySignalKind::Capability => "capability.category",
      TelemetrySignalKind::Wait => "wait.kind",
      TelemetrySignalKind::Compensation | TelemetrySignalKind::Cleanup => "corrective.phase",
      TelemetrySignalKind::RemoteDispatch => "remote.phase",
      _ => "handler.category",
    };
    attributes.insert(key.to_string(), TelemetryAttributeValue::String(category));
  }
  for (key, value) in extra_attributes {
    attributes.insert(key.to_string(), value);
  }
  TelemetrySignalV1 {
    profile: TELEMETRY_SIGNAL_PROFILE,
    signal_id: signal_id(event, kind),
    kind,
    occurred_at: event.occurred_at,
    started_at: Some(started.started_at),
    ended_at: Some(event.occurred_at),
    runtime_instance_id: context.runtime_instance_id.clone(),
    deployment_id: context.deployment_id.clone(),
    workflow_id: Some(workflow_id.to_string()),
    run_id: Some(event.run_id.clone()),
    node_id: started.node_id,
    attempt: started.attempt,
    invocation_id: started.invocation_id,
    status,
    code,
    duration_ms: duration_ms(started.started_at, event.occurred_at),
    attributes,
  }
}

#[allow(clippy::too_many_arguments)]
fn point_signal<const N: usize>(
  event: &RunEvent,
  workflow_id: &str,
  context: &TelemetrySignalContext,
  kind: TelemetrySignalKind,
  status: TelemetrySignalStatus,
  node_id: Option<String>,
  attempt: Option<u32>,
  invocation_id: Option<String>,
  code: Option<String>,
  attributes: [(&str, TelemetryAttributeValue); N],
) -> TelemetrySignalV1 {
  TelemetrySignalV1 {
    profile: TELEMETRY_SIGNAL_PROFILE,
    signal_id: signal_id(event, kind),
    kind,
    occurred_at: event.occurred_at,
    started_at: Some(event.occurred_at),
    ended_at: Some(event.occurred_at),
    runtime_instance_id: context.runtime_instance_id.clone(),
    deployment_id: context.deployment_id.clone(),
    workflow_id: Some(workflow_id.to_string()),
    run_id: Some(event.run_id.clone()),
    node_id,
    attempt,
    invocation_id,
    status,
    code,
    duration_ms: Some(0.0),
    attributes: attributes
      .into_iter()
      .map(|(key, value)| (key.to_string(), value))
      .collect(),
  }
}

fn corrective_signal(
  event: &RunEvent,
  workflow_id: &str,
  context: &TelemetrySignalContext,
  action_kind: CorrectiveActionKind,
  started: IntervalIdentity,
  status: TelemetrySignalStatus,
  code: Option<String>,
) -> TelemetrySignalV1 {
  let kind = match action_kind {
    CorrectiveActionKind::Compensation => TelemetrySignalKind::Compensation,
    CorrectiveActionKind::Cleanup => TelemetrySignalKind::Cleanup,
  };
  interval_signal(event, workflow_id, context, kind, status, started, code, [])
}

fn run_signal(
  event: &RunEvent,
  workflow_id: &str,
  context: &TelemetrySignalContext,
  started_at: DateTime<Utc>,
  outcome: BusinessOutcome,
  trigger: Option<&str>,
  code: Option<String>,
) -> TelemetrySignalV1 {
  let attributes = trigger
    .and_then(safe_category)
    .map(|trigger| [("trigger.type", TelemetryAttributeValue::String(trigger))])
    .unwrap_or_else(|| [("trigger.type", string_attribute("unknown"))]);
  interval_signal(
    event,
    workflow_id,
    context,
    TelemetrySignalKind::Run,
    match outcome {
      BusinessOutcome::Succeeded => TelemetrySignalStatus::Succeeded,
      BusinessOutcome::Failed => TelemetrySignalStatus::Failed,
      BusinessOutcome::Cancelled => TelemetrySignalStatus::Cancelled,
    },
    IntervalIdentity {
      started_at,
      node_id: None,
      attempt: None,
      invocation_id: None,
      category: None,
    },
    code,
    attributes,
  )
}

fn signal_id(event: &RunEvent, kind: TelemetrySignalKind) -> String {
  format!("telemetry_{}_{}", event.event_id, kind.as_str())
}

fn duration_ms(started_at: DateTime<Utc>, ended_at: DateTime<Utc>) -> Option<f64> {
  let micros = (ended_at - started_at).num_microseconds()?.max(0);
  Some(micros as f64 / 1_000.0)
}

fn wait_kind(kind: DurableWaitKind) -> &'static str {
  match kind {
    DurableWaitKind::Duration => "duration",
    DurableWaitKind::Until => "until",
    DurableWaitKind::Signal => "signal",
  }
}

fn corrective_kind(kind: CorrectiveActionKind) -> &'static str {
  match kind {
    CorrectiveActionKind::Compensation => "compensation",
    CorrectiveActionKind::Cleanup => "cleanup",
  }
}

fn correction_key(action_id: &str, attempt: u32) -> String {
  format!("{action_id}:{attempt}")
}

fn lifecycle_key(hook_invocation_id: &str, action_id: &str, attempt: u32) -> String {
  format!("{hook_invocation_id}:{action_id}:{attempt}")
}

fn reusable_lifecycle_key(invocation_id: &str, action_id: &str) -> String {
  format!("{invocation_id}:{action_id}")
}

fn string_attribute(value: &str) -> TelemetryAttributeValue {
  TelemetryAttributeValue::String(value.to_string())
}

fn safe_id(value: &str) -> bool {
  !value.is_empty()
    && value.len() <= MAX_ID_BYTES
    && !value.chars().any(|character| character.is_control())
}

fn safe_code(value: &str) -> bool {
  value.len() > 5
    && value.len() <= MAX_CODE_BYTES
    && value.starts_with("WOML_")
    && value[5..]
      .bytes()
      .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

fn safe_category(value: &str) -> Option<String> {
  (value.len() <= 64
    && !value.is_empty()
    && value.bytes().all(|byte| {
      byte.is_ascii_lowercase()
        || byte.is_ascii_digit()
        || matches!(byte, b'.' | b'_' | b'/' | b'-')
    }))
  .then(|| value.to_string())
}

fn safe_attribute(key: &str, value: &TelemetryAttributeValue) -> bool {
  const KEYS: &[&str] = &[
    "trigger.type",
    "handler.category",
    "node.kind",
    "capability.category",
    "corrective.phase",
    "wait.kind",
    "lock.outcome",
    "retry.scheduled",
    "recovered",
    "remote.phase",
    "remote.outcome",
  ];
  KEYS.contains(&key)
    && match value {
      TelemetryAttributeValue::String(value) => safe_category(value).is_some(),
      TelemetryAttributeValue::Boolean(_) => matches!(key, "retry.scheduled" | "recovered"),
    }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::event::{
    FinalLifecycleStatus, ForEachIterationScope, ForEachOpenedData, ForEachSettledData,
    RunExecutionStartedData, RunFinalizedData, StepAttemptStartedData, StepAttemptSucceededData,
  };
  use serde_json::json;

  fn event(sequence: u64, occurred_at: DateTime<Utc>, payload: RunEventPayload) -> RunEvent {
    RunEvent {
      event_schema_version: crate::RUN_EVENT_SCHEMA_VERSION_V19,
      event_id: format!("evt_{sequence}"),
      run_id: "run_safe".to_string(),
      sequence,
      occurred_at,
      iteration: None,
      payload,
    }
  }

  #[test]
  fn deterministic_identity_matches_the_frozen_cross_language_fixture() {
    assert_eq!(
      trace_id_v1("run_01HXTRACE"),
      "71bbd5822afe17d9d00bf8eaa62854b3"
    );
    assert_eq!(
      span_id_v1("run_01HXTRACE", "run", "run_01HXTRACE"),
      "0868430ecec5cefe"
    );
    assert_eq!(
      span_id_v1("run_01HXTRACE", "attempt", "prepare:1:inv_01"),
      "8a857c39a28e38ba"
    );
  }

  #[test]
  fn projection_uses_durable_times_and_never_copies_workflow_values() {
    let started_at = "2026-10-23T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
    let attempt_at = started_at + chrono::Duration::milliseconds(10);
    let ended_at = started_at + chrono::Duration::milliseconds(125);
    let events = vec![
      event(
        1,
        started_at,
        RunEventPayload::RunExecutionStarted(RunExecutionStartedData {
          started_at,
          timeout_at: None,
        }),
      ),
      event(
        2,
        attempt_at,
        RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
          node_id: "prepare".to_string(),
          attempt: 1,
          invocation_id: "inv_01".to_string(),
          handler: "runtime.script".to_string(),
          idempotency_key: Some("secret-idempotency-value".to_string()),
        }),
      ),
      event(
        3,
        ended_at,
        RunEventPayload::StepAttemptSucceeded(StepAttemptSucceededData {
          node_id: "prepare".to_string(),
          attempt: 1,
          invocation_id: "inv_01".to_string(),
          output: json!({ "token": "never-export-this" }),
        }),
      ),
      event(
        4,
        ended_at,
        RunEventPayload::RunFinalized(RunFinalizedData {
          outcome: BusinessOutcome::Succeeded,
          lifecycle_status: FinalLifecycleStatus::Completed,
          warnings: Vec::new(),
        }),
      ),
    ];
    let signals = project_telemetry_signals(
      &events,
      "checkout",
      &TelemetrySignalContext {
        runtime_instance_id: "runtime_01".to_string(),
        deployment_id: "deployment_01".to_string(),
      },
    );
    assert_eq!(signals.len(), 3);
    assert_eq!(signals[0].status, TelemetrySignalStatus::Started);
    assert_eq!(
      signals
        .iter()
        .find(|signal| signal.kind == TelemetrySignalKind::Attempt)
        .and_then(|signal| signal.duration_ms),
      Some(115.0)
    );
    let encoded = serde_json::to_string(&signals).unwrap();
    assert!(!encoded.contains("never-export-this"));
    assert!(!encoded.contains("secret-idempotency-value"));
  }

  #[test]
  fn remote_projection_exports_queue_and_ambiguity_without_target_or_worker_dimensions() {
    let queued_at = "2026-10-24T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
    let started_at = queued_at + chrono::Duration::milliseconds(40);
    let lost_at = started_at + chrono::Duration::milliseconds(125);
    let remote = |sequence, occurred_at, state| RunEvent {
      event_schema_version: crate::RUN_EVENT_SCHEMA_VERSION_V20,
      event_id: format!("evt_remote_{sequence}"),
      run_id: "run_remote_safe".to_string(),
      sequence,
      occurred_at,
      iteration: None,
      payload: RunEventPayload::RemoteDispatchTransitioned(crate::RemoteDispatchTransitionedData {
        dispatch_id: format!("dispatch_{}", "a".repeat(64)),
        node_id: "deploy".to_string(),
        attempt: 1,
        invocation_id: "inv_remote_safe".to_string(),
        target: "production-private".to_string(),
        state,
        worker_id: (!matches!(state, crate::RemoteDispatchStateV1::Requested))
          .then(|| "production-runner-01".to_string()),
        terminal_digest: None,
      }),
    };
    let signals = project_telemetry_signals(
      &[
        remote(1, queued_at, crate::RemoteDispatchStateV1::Requested),
        remote(2, started_at, crate::RemoteDispatchStateV1::Started),
        remote(3, lost_at, crate::RemoteDispatchStateV1::Ambiguous),
      ],
      "remote-workflow",
      &TelemetrySignalContext {
        runtime_instance_id: "runtime_01".to_string(),
        deployment_id: "deployment_01".to_string(),
      },
    );
    assert_eq!(signals.len(), 2);
    assert_eq!(signals[0].duration_ms, Some(40.0));
    assert_eq!(signals[1].duration_ms, Some(125.0));
    assert_eq!(signals[1].status, TelemetrySignalStatus::Degraded);
    let encoded = serde_json::to_string(&signals).unwrap();
    assert!(!encoded.contains("production-private"));
    assert!(!encoded.contains("production-runner-01"));
  }

  #[test]
  fn bounded_dispatch_never_waits_for_a_slow_or_panicking_sink() {
    let sink: TelemetrySignalReporter = Arc::new(|_| {
      std::thread::sleep(std::time::Duration::from_millis(100));
      panic!("telemetry sink failure");
    });
    let reporter = bounded_telemetry_reporter(1, sink);
    let now = Utc::now();
    let signal = TelemetrySignalV1 {
      profile: TELEMETRY_SIGNAL_PROFILE,
      signal_id: "telemetry_test".to_string(),
      kind: TelemetrySignalKind::Runtime,
      occurred_at: now,
      started_at: Some(now),
      ended_at: Some(now),
      runtime_instance_id: "runtime_01".to_string(),
      deployment_id: "deployment_01".to_string(),
      workflow_id: None,
      run_id: None,
      node_id: None,
      attempt: None,
      invocation_id: None,
      status: TelemetrySignalStatus::Started,
      code: None,
      duration_ms: Some(0.0),
      attributes: BTreeMap::new(),
    };
    let started = std::time::Instant::now();
    for _ in 0..100 {
      reporter(signal.clone());
    }
    assert!(started.elapsed() < std::time::Duration::from_millis(25));
  }

  #[test]
  fn for_each_iterations_coalesce_to_one_bounded_span() {
    let started_at = "2026-10-23T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
    let mut events = vec![event(
      1,
      started_at,
      RunEventPayload::ForEachOpened(ForEachOpenedData {
        for_each_id: "items".to_string(),
        total: 100,
        items_digest: "sha256:items".to_string(),
        concurrency: 10,
      }),
    )];
    for index in 0..100 {
      let mut started = event(
        index as u64 + 2,
        started_at + chrono::Duration::milliseconds(i64::from(index)),
        RunEventPayload::StepAttemptStarted(StepAttemptStartedData {
          node_id: "transform".to_string(),
          attempt: 1,
          invocation_id: format!("inv_{index}"),
          handler: "runtime.script".to_string(),
          idempotency_key: None,
        }),
      );
      started.iteration = Some(ForEachIterationScope {
        for_each_id: "items".to_string(),
        index,
      });
      events.push(started);
    }
    events.push(event(
      102,
      started_at + chrono::Duration::milliseconds(100),
      RunEventPayload::ForEachSucceeded(ForEachSettledData {
        for_each_id: "items".to_string(),
        total: 100,
        succeeded: 100,
        failed: 0,
        skipped: 0,
        aggregate_digest: "sha256:aggregate".to_string(),
      }),
    ));
    let signals = project_telemetry_signals(
      &events,
      "batch",
      &TelemetrySignalContext {
        runtime_instance_id: "runtime_01".to_string(),
        deployment_id: "deployment_01".to_string(),
      },
    );
    assert_eq!(signals.len(), 1);
    assert_eq!(
      signals[0].attributes.get("node.kind"),
      Some(&string_attribute("for_each"))
    );
  }
}
