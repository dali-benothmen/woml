use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

use crate::event::{
  AttemptFailure, BranchFailure, EventValidationError, RunEvent, RunEventPayload, RunFailedData,
  RunFailedDataV2, StepAttemptStartedData,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
  #[default]
  NotStarted,
  Running,
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
pub enum RunFailure {
  Attempt(AttemptFailure),
  Branch(BranchFailure),
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
      RunEventPayload::RunSucceeded(data) => {
        require_running(&projection)?;
        if !projection
          .context
          .steps
          .contains_key(&data.terminal_node_id)
        {
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
