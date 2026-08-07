use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use thiserror::Error;

use crate::event::{
  ApprovalDecision, ApprovalDecisionSource, ApprovalFailure, ApprovalResolution,
  ApprovalTimeoutPolicy, AttemptFailure, BranchFailure, EventValidationError, ParallelFailure,
  ParallelFailurePolicy, ParallelGroupOutcome, RunEvent, RunEventPayload, RunFailedData,
  RunFailedDataV2, RunFailedDataV3, RunFailedDataV4, StepAttemptStartedData,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
  #[default]
  NotStarted,
  Running,
  Waiting,
  Succeeded,
  Failed,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WorkflowContext {
  pub trigger: Map<String, Value>,
  pub steps: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AttemptIdentity {
  pub node_id: String,
  pub attempt: u32,
  pub invocation_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AttemptStatus {
  Started,
  Succeeded { output: Value },
  Failed { failure: AttemptFailure },
}

#[derive(Debug, Clone, PartialEq)]
pub struct AttemptProjection {
  pub identity: AttemptIdentity,
  pub handler: String,
  pub status: AttemptStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParallelGroupStatus {
  Started,
  Completed {
    outcome: ParallelGroupOutcome,
    failed_node_ids: Vec<String>,
    cancelled_node_ids: Vec<String>,
  },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParallelGroupProjection {
  pub parallel_id: String,
  pub fork_context: WorkflowContext,
  pub status: ParallelGroupStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalRequestStatus {
  Waiting,
  Resolved {
    resolution: ApprovalResolution,
    resolved_at: DateTime<Utc>,
  },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequestProjection {
  pub approval_id: String,
  pub request_id: String,
  pub requested_at: DateTime<Utc>,
  pub expires_at: Option<DateTime<Utc>>,
  pub on_timeout: ApprovalTimeoutPolicy,
  pub status: ApprovalRequestStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunFailure {
  Attempt(AttemptFailure),
  Branch(BranchFailure),
  Parallel {
    parallel_id: String,
    policy: ParallelFailurePolicy,
    primary_node_id: String,
    failed_node_ids: Vec<String>,
    cancelled_node_ids: Vec<String>,
    failure: ParallelFailure,
  },
  Approval {
    approval_id: String,
    request_id: String,
    failure: ApprovalFailure,
  },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RunProjection {
  pub event_schema_version: Option<u32>,
  pub run_id: Option<String>,
  pub workflow_id: Option<String>,
  pub definition_hash: Option<String>,
  pub status: RunStatus,
  pub context: WorkflowContext,
  pub attempts: Vec<AttemptProjection>,
  pub branch_selections: BTreeMap<String, String>,
  pub parallel_groups: BTreeMap<String, ParallelGroupProjection>,
  pub approval_requests: BTreeMap<String, ApprovalRequestProjection>,
  pub terminal_node_id: Option<String>,
  pub result: Option<Value>,
  pub failure: Option<RunFailure>,
  pub last_sequence: u64,
}

impl RunProjection {
  pub fn completed_node_ids(&self) -> HashSet<&str> {
    self.context.steps.keys().map(String::as_str).collect()
  }

  pub fn attempted_node_ids(&self) -> HashSet<&str> {
    self
      .attempts
      .iter()
      .map(|attempt| attempt.identity.node_id.as_str())
      .collect()
  }
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum FoldError {
  #[error(transparent)]
  InvalidEvent(#[from] EventValidationError),
  #[error("{0}")]
  InvalidHistory(String),
}

fn identity(node_id: &str, attempt: u32, invocation_id: &str) -> AttemptIdentity {
  AttemptIdentity {
    node_id: node_id.to_string(),
    attempt,
    invocation_id: invocation_id.to_string(),
  }
}

fn started_attempt(data: &StepAttemptStartedData) -> AttemptProjection {
  AttemptProjection {
    identity: identity(&data.node_id, data.attempt, &data.invocation_id),
    handler: data.handler.clone(),
    status: AttemptStatus::Started,
  }
}

pub fn fold_events(events: &[RunEvent]) -> Result<RunProjection, FoldError> {
  let mut projection = RunProjection::default();
  let mut event_ids = HashSet::new();
  let mut attempt_indexes: HashMap<AttemptIdentity, usize> = HashMap::new();

  for (index, event) in events.iter().enumerate() {
    event.validate()?;
    let expected_sequence = index as u64 + 1;
    if event.sequence != expected_sequence {
      return Err(FoldError::InvalidHistory(format!(
        "Expected sequence {expected_sequence}, received {}.",
        event.sequence
      )));
    }
    if !event_ids.insert(event.event_id.as_str()) {
      return Err(FoldError::InvalidHistory(format!(
        "Event ID {:?} appears more than once.",
        event.event_id
      )));
    }
    if let Some(run_id) = &projection.run_id {
      if run_id != &event.run_id {
        return Err(FoldError::InvalidHistory(format!(
          "Run history mixes run IDs {:?} and {:?}.",
          run_id, event.run_id
        )));
      }
    }
    if let Some(event_schema_version) = projection.event_schema_version {
      if event_schema_version != event.event_schema_version {
        return Err(FoldError::InvalidHistory(format!(
          "Run history mixes event schema versions {event_schema_version} and {}.",
          event.event_schema_version
        )));
      }
    }
    if matches!(projection.status, RunStatus::Succeeded | RunStatus::Failed) {
      return Err(FoldError::InvalidHistory(
        "A terminal run event must be the final event.".to_string(),
      ));
    }

    match &event.payload {
      RunEventPayload::RunStarted(data) => {
        if index != 0 || projection.status != RunStatus::NotStarted {
          return Err(FoldError::InvalidHistory(
            "run_started must be the first and only start event.".to_string(),
          ));
        }
        projection.run_id = Some(event.run_id.clone());
        projection.event_schema_version = Some(event.event_schema_version);
        projection.workflow_id = Some(data.workflow_id.clone());
        projection.definition_hash = Some(data.definition_hash.clone());
        projection.context.trigger = data.trigger.clone();
        projection.status = RunStatus::Running;
      }
      RunEventPayload::StepAttemptStarted(data) => {
        require_running(&projection)?;
        let key = identity(&data.node_id, data.attempt, &data.invocation_id);
        if attempt_indexes.contains_key(&key) {
          return Err(FoldError::InvalidHistory(format!(
            "Attempt {:?} was started more than once.",
            key.invocation_id
          )));
        }
        let attempt_index = projection.attempts.len();
        projection.attempts.push(started_attempt(data));
        attempt_indexes.insert(key, attempt_index);
      }
      RunEventPayload::StepAttemptSucceeded(data) => {
        require_running(&projection)?;
        let key = identity(&data.node_id, data.attempt, &data.invocation_id);
        let attempt_index = *attempt_indexes.get(&key).ok_or_else(|| {
          FoldError::InvalidHistory(format!(
            "Successful attempt {:?} has no matching start event.",
            data.invocation_id
          ))
        })?;
        let attempt = &mut projection.attempts[attempt_index];
        if attempt.status != AttemptStatus::Started {
          return Err(FoldError::InvalidHistory(format!(
            "Attempt {:?} already has a terminal event.",
            data.invocation_id
          )));
        }
        if projection.context.steps.contains_key(&data.node_id) {
          return Err(FoldError::InvalidHistory(format!(
            "Node {:?} has already published a successful output.",
            data.node_id
          )));
        }
        attempt.status = AttemptStatus::Succeeded {
          output: data.output.clone(),
        };
        projection
          .context
          .steps
          .insert(data.node_id.clone(), data.output.clone());
      }
      RunEventPayload::StepAttemptFailed(data) => {
        require_running(&projection)?;
        let key = identity(&data.node_id, data.attempt, &data.invocation_id);
        let attempt_index = *attempt_indexes.get(&key).ok_or_else(|| {
          FoldError::InvalidHistory(format!(
            "Failed attempt {:?} has no matching start event.",
            data.invocation_id
          ))
        })?;
        let attempt = &mut projection.attempts[attempt_index];
        if attempt.status != AttemptStatus::Started {
          return Err(FoldError::InvalidHistory(format!(
            "Attempt {:?} already has a terminal event.",
            data.invocation_id
          )));
        }
        attempt.status = AttemptStatus::Failed {
          failure: data.failure.clone(),
        };
      }
      RunEventPayload::BranchSelected(data) => {
        require_running(&projection)?;
        if let Some(selected_arm) = projection.branch_selections.get(&data.branch_id) {
          return Err(FoldError::InvalidHistory(format!(
            "Branch {:?} was already selected as arm {:?}.",
            data.branch_id, selected_arm
          )));
        }
        projection
          .branch_selections
          .insert(data.branch_id.clone(), data.arm_id.clone());
      }
      RunEventPayload::ParallelGroupStarted(data) => {
        require_running(&projection)?;
        if projection.parallel_groups.contains_key(&data.parallel_id) {
          return Err(FoldError::InvalidHistory(format!(
            "Parallel group {:?} was started more than once.",
            data.parallel_id
          )));
        }
        projection.parallel_groups.insert(
          data.parallel_id.clone(),
          ParallelGroupProjection {
            parallel_id: data.parallel_id.clone(),
            fork_context: projection.context.clone(),
            status: ParallelGroupStatus::Started,
          },
        );
      }
      RunEventPayload::ParallelGroupCompleted(data) => {
        require_running(&projection)?;
        let group = projection
          .parallel_groups
          .get_mut(&data.parallel_id)
          .ok_or_else(|| {
            FoldError::InvalidHistory(format!(
              "Parallel group {:?} completed before it started.",
              data.parallel_id
            ))
          })?;
        if !matches!(group.status, ParallelGroupStatus::Started) {
          return Err(FoldError::InvalidHistory(format!(
            "Parallel group {:?} was completed more than once.",
            data.parallel_id
          )));
        }
        group.status = ParallelGroupStatus::Completed {
          outcome: data.outcome,
          failed_node_ids: data.failed_node_ids.clone(),
          cancelled_node_ids: data.cancelled_node_ids.clone(),
        };
      }
      RunEventPayload::ApprovalRequested(data) => {
        require_running(&projection)?;
        if projection.approval_requests.contains_key(&data.approval_id) {
          return Err(FoldError::InvalidHistory(format!(
            "Approval {:?} was requested more than once.",
            data.approval_id
          )));
        }
        if projection
          .approval_requests
          .values()
          .any(|request| request.request_id == data.request_id)
        {
          return Err(FoldError::InvalidHistory(format!(
            "Approval request ID {:?} appears more than once.",
            data.request_id
          )));
        }
        projection.approval_requests.insert(
          data.approval_id.clone(),
          ApprovalRequestProjection {
            approval_id: data.approval_id.clone(),
            request_id: data.request_id.clone(),
            requested_at: event.occurred_at,
            expires_at: data.expires_at,
            on_timeout: data.on_timeout,
            status: ApprovalRequestStatus::Waiting,
          },
        );
        projection.status = RunStatus::Waiting;
      }
      RunEventPayload::ApprovalResolved(data) => {
        if projection.status != RunStatus::Waiting {
          return Err(FoldError::InvalidHistory(
            "approval_resolved requires a waiting run.".to_string(),
          ));
        }
        let request = projection
          .approval_requests
          .get_mut(&data.approval_id)
          .ok_or_else(|| {
            FoldError::InvalidHistory(format!(
              "Approval {:?} resolved before it was requested.",
              data.approval_id
            ))
          })?;
        if request.request_id != data.request_id {
          return Err(FoldError::InvalidHistory(format!(
            "Approval {:?} resolved with a mismatched request ID.",
            data.approval_id
          )));
        }
        if !matches!(request.status, ApprovalRequestStatus::Waiting) {
          return Err(FoldError::InvalidHistory(format!(
            "Approval {:?} was resolved more than once.",
            data.approval_id
          )));
        }
        match &data.resolution {
          ApprovalResolution::Decision {
            source: ApprovalDecisionSource::Human,
            ..
          } if request
            .expires_at
            .is_some_and(|deadline| event.occurred_at >= deadline) =>
          {
            return Err(FoldError::InvalidHistory(format!(
              "Human decision for approval {:?} occurred at or after its deadline.",
              data.approval_id
            )));
          }
          ApprovalResolution::Decision {
            decision: ApprovalDecision::Rejected,
            source: ApprovalDecisionSource::Timeout,
          } if request.on_timeout == ApprovalTimeoutPolicy::Reject
            && request
              .expires_at
              .is_some_and(|deadline| event.occurred_at >= deadline) => {}
          ApprovalResolution::TimeoutFailure
            if request.on_timeout == ApprovalTimeoutPolicy::Fail
              && request
                .expires_at
                .is_some_and(|deadline| event.occurred_at >= deadline) => {}
          ApprovalResolution::Decision {
            source: ApprovalDecisionSource::Human,
            ..
          } => {}
          _ => {
            return Err(FoldError::InvalidHistory(format!(
              "Approval {:?} resolution does not match its deadline and timeout policy.",
              data.approval_id
            )));
          }
        }
        if let ApprovalResolution::Decision { decision, source } = data.resolution {
          projection.context.steps.insert(
            data.approval_id.clone(),
            json!({
              "decision": match decision {
                ApprovalDecision::Approved => "approved",
                ApprovalDecision::Rejected => "rejected",
              },
              "source": match source {
                ApprovalDecisionSource::Human => "human",
                ApprovalDecisionSource::Timeout => "timeout",
              },
              "decidedAt": event
                .occurred_at
                .to_rfc3339_opts(SecondsFormat::Millis, true),
            }),
          );
        }
        request.status = ApprovalRequestStatus::Resolved {
          resolution: data.resolution.clone(),
          resolved_at: event.occurred_at,
        };
        projection.status = RunStatus::Running;
      }
      RunEventPayload::RunSucceeded(data) => {
        require_running(&projection)?;
        let terminal_output_exists = projection
          .context
          .steps
          .contains_key(&data.terminal_node_id)
          || data
            .terminal_node_id
            .strip_prefix("__woml_approval__")
            .and_then(|value| value.strip_suffix("__join"))
            .is_some_and(|approval_id| projection.context.steps.contains_key(approval_id));
        if !terminal_output_exists {
          return Err(FoldError::InvalidHistory(format!(
            "Terminal node {:?} has not published a successful output.",
            data.terminal_node_id
          )));
        }
        projection.status = RunStatus::Succeeded;
        projection.terminal_node_id = Some(data.terminal_node_id.clone());
        projection.result = Some(data.result.clone());
      }
      RunEventPayload::RunFailed(data) => {
        require_running(&projection)?;
        let failure = match data {
          RunFailedData::V1(data) => {
            if let (Some(node_id), Some(attempt), Some(invocation_id)) =
              (&data.node_id, data.attempt, &data.invocation_id)
            {
              require_failed_attempt(
                &projection,
                &attempt_indexes,
                node_id,
                attempt,
                invocation_id,
              )?;
            }
            RunFailure::Attempt(data.failure.clone())
          }
          RunFailedData::V2(RunFailedDataV2::Attempt {
            node_id,
            attempt,
            invocation_id,
            failure,
          }) => {
            require_failed_attempt(
              &projection,
              &attempt_indexes,
              node_id,
              *attempt,
              invocation_id,
            )?;
            RunFailure::Attempt(failure.clone())
          }
          RunFailedData::V2(RunFailedDataV2::Branch { failure, .. }) => {
            RunFailure::Branch(failure.clone())
          }
          RunFailedData::V3(RunFailedDataV3::Parallel {
            parallel_id,
            policy,
            primary_node_id,
            failed_node_ids,
            cancelled_node_ids,
            failure,
          }) => {
            let group = projection.parallel_groups.get(parallel_id).ok_or_else(|| {
              FoldError::InvalidHistory(format!(
                "run_failed references parallel group {parallel_id:?} before it started."
              ))
            })?;
            match &group.status {
              ParallelGroupStatus::Completed {
                outcome: ParallelGroupOutcome::Failed,
                failed_node_ids: completed_failed,
                cancelled_node_ids: completed_cancelled,
              } if completed_failed == failed_node_ids
                && completed_cancelled == cancelled_node_ids => {}
              _ => {
                return Err(FoldError::InvalidHistory(format!(
                  "run_failed does not match the failed completion of parallel group {parallel_id:?}."
                )));
              }
            }
            RunFailure::Parallel {
              parallel_id: parallel_id.clone(),
              policy: *policy,
              primary_node_id: primary_node_id.clone(),
              failed_node_ids: failed_node_ids.clone(),
              cancelled_node_ids: cancelled_node_ids.clone(),
              failure: failure.clone(),
            }
          }
          RunFailedData::V4(RunFailedDataV4::Approval {
            approval_id,
            request_id,
            failure,
          }) => {
            let request = projection
              .approval_requests
              .get(approval_id)
              .ok_or_else(|| {
                FoldError::InvalidHistory(format!(
                  "run_failed references unknown approval {approval_id:?}."
                ))
              })?;
            if request.request_id != *request_id
              || !matches!(
                request.status,
                ApprovalRequestStatus::Resolved {
                  resolution: ApprovalResolution::TimeoutFailure,
                  ..
                }
              )
            {
              return Err(FoldError::InvalidHistory(format!(
                "run_failed does not match the timeout failure of approval {approval_id:?}."
              )));
            }
            RunFailure::Approval {
              approval_id: approval_id.clone(),
              request_id: request_id.clone(),
              failure: failure.clone(),
            }
          }
        };
        projection.status = RunStatus::Failed;
        projection.failure = Some(failure);
      }
    }
    projection.last_sequence = event.sequence;
  }
  Ok(projection)
}

fn require_failed_attempt(
  projection: &RunProjection,
  attempt_indexes: &HashMap<AttemptIdentity, usize>,
  node_id: &str,
  attempt: u32,
  invocation_id: &str,
) -> Result<(), FoldError> {
  let key = identity(node_id, attempt, invocation_id);
  let attempt_index = attempt_indexes.get(&key).ok_or_else(|| {
    FoldError::InvalidHistory(format!(
      "run_failed references unknown attempt {invocation_id:?}."
    ))
  })?;
  if !matches!(
    projection.attempts[*attempt_index].status,
    AttemptStatus::Failed { .. }
  ) {
    return Err(FoldError::InvalidHistory(format!(
      "run_failed references attempt {invocation_id:?} before it failed."
    )));
  }
  Ok(())
}

fn require_running(projection: &RunProjection) -> Result<(), FoldError> {
  if projection.status != RunStatus::Running {
    return Err(FoldError::InvalidHistory(
      "A run must be started and nonterminal before attempt or terminal events.".to_string(),
    ));
  }
  Ok(())
}
