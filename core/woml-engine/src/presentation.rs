use std::collections::{BTreeSet, HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

use crate::{
  durable::{DurableEventStore, DurableStoreError},
  event::{AttemptFailure, AttemptFailureKind, BranchFailure, RunEvent, RunEventPayload},
  model::{
    CompiledReusableInvocation, CompiledTrigger, CompiledWorkflowDefinition, LifecycleEventName,
    ValueExpression,
  },
  projection::{
    fold_events, FoldError, LifecycleActionStatus, LifecycleHookStatus, RunFailure, RunProjection,
    RunStatus,
  },
};

pub const RUN_PRESENTATION_PROFILE: &str = "woml.run-presentation/v1";
pub const RUN_PRESENTATION_LIST_PROFILE: &str = "woml.run-presentation-list/v1";
pub const RUN_PRESENTATION_MAX_BYTES: usize = 2 * 1024 * 1024;
pub const RUN_PRESENTATION_MAX_STEPS: usize = 10_000;
pub const RUN_PRESENTATION_MAX_LIFECYCLE: usize = 1_000;
pub const RUN_PRESENTATION_MAX_WARNINGS: usize = 1_000;
pub const RUN_PRESENTATION_RECENT_LIMIT: usize = 10;
pub const RUN_PRESENTATION_MAX_EVENTS: usize = 100_000;

const MAX_SHORT_TEXT: usize = 2_048;
const MAX_MESSAGE: usize = 8_192;
const MAX_VALUE_DEPTH: usize = 5;
const MAX_VALUE_PROPERTIES: usize = 20;
const MAX_VALUE_ITEMS: usize = 20;
const MAX_VALUE_STRING: usize = 500;
const MAX_VALUE_NODES: usize = 2_000;
const MAX_DURATION_MS: u64 = 31_536_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentationTriggerType {
  Manual,
  Webhook,
  Slack,
  Telegram,
  Schedule,
  Interval,
  Event,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerPresentationV1 {
  pub id: String,
  #[serde(rename = "type")]
  pub kind: PresentationTriggerType,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub label: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub method: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub url: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub schedule: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub timezone: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub interval: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub event: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub workspace: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub scope: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowPresentationV1 {
  pub id: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub name: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub description: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub version: Option<String>,
  pub definition_hash: String,
  pub triggers: Vec<TriggerPresentationV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentationRunStatus {
  Queued,
  Running,
  Waiting,
  Retrying,
  Cancelling,
  Finalizing,
  Succeeded,
  Failed,
  Cancelled,
  TimedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentationStepStatus {
  Queued,
  Running,
  Waiting,
  Retrying,
  Succeeded,
  Failed,
  Cancelled,
  TimedOut,
  Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentationStepKind {
  Step,
  Script,
  CustomStep,
  Switch,
  Choose,
  Parallel,
  Fork,
  Branch,
  Approval,
  WorkflowCall,
  WorkflowStart,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationFailureV1 {
  pub code: String,
  pub message: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub kind: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub retryable: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepPresentationV1 {
  pub id: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub name: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub description: Option<String>,
  pub kind: PresentationStepKind,
  pub status: PresentationStepStatus,
  pub depth: u32,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub started_at: Option<DateTime<Utc>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub completed_at: Option<DateTime<Utc>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub duration_ms: Option<u64>,
  pub attempts: u32,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub detail: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub result: Option<Value>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub result_truncated: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub failure: Option<PresentationFailureV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PresentationLifecycleHook {
  OnStart,
  OnSuccess,
  OnError,
  OnCancel,
  OnComplete,
  OnStepStart,
  OnStepSuccess,
  OnStepFailure,
  OnStepComplete,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecyclePresentationV1 {
  pub hook: PresentationLifecycleHook,
  pub status: PresentationStepStatus,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub duration_ms: Option<u64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub provider: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub detail: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub failure: Option<PresentationFailureV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunPresentationSummaryV1 {
  pub succeeded: u32,
  pub failed: u32,
  pub skipped: u32,
  pub cancelled: u32,
  pub total: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunPresentationV1 {
  pub profile: &'static str,
  pub workflow: WorkflowPresentationV1,
  pub run_id: String,
  pub trigger: RunPresentationTriggerV1,
  pub status: PresentationRunStatus,
  pub admitted_at: DateTime<Utc>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub started_at: Option<DateTime<Utc>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub completed_at: Option<DateTime<Utc>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub duration_ms: Option<u64>,
  pub steps: Vec<StepPresentationV1>,
  pub summary: RunPresentationSummaryV1,
  pub lifecycle: Vec<LifecyclePresentationV1>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub result: Option<Value>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub result_truncated: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub failure: Option<PresentationFailureV1>,
  pub warnings: Vec<PresentationFailureV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunPresentationTriggerV1 {
  pub id: String,
  #[serde(rename = "type")]
  pub kind: PresentationTriggerType,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunPresentationListV1 {
  pub profile: &'static str,
  pub workflow_id: String,
  pub runs: Vec<RunPresentationV1>,
}

#[derive(Debug, Error)]
pub enum RunPresentationError {
  #[error(transparent)]
  Store(#[from] DurableStoreError),
  #[error(transparent)]
  Fold(#[from] FoldError),
  #[error("run history is empty")]
  EmptyHistory,
  #[error("run history does not match its frozen workflow definition")]
  DefinitionMismatch,
  #[error("unsupported presentation trigger handler {0:?}")]
  UnsupportedTrigger(String),
  #[error("run presentation has too many {0}")]
  TooMany(&'static str),
  #[error("run presentation exceeds the 2 MiB encoded limit")]
  TooLarge,
  #[error("recent presentation limit must be between 1 and 10")]
  InvalidRecentLimit,
  #[error("run presentation could not be encoded: {0}")]
  Encoding(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Default)]
struct AttemptTimeline {
  started_at: Option<DateTime<Utc>>,
  completed_at: Option<DateTime<Utc>>,
  attempts: u32,
  latest: Option<TimelineStatus>,
}

#[derive(Debug, Clone, Copy, Default)]
struct ControlTimeline {
  started_at: Option<DateTime<Utc>>,
  completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default)]
struct ControlTimelines {
  choice: HashMap<String, ControlTimeline>,
  parallel: HashMap<String, ControlTimeline>,
  fork: HashMap<String, ControlTimeline>,
  branch: HashMap<(String, String), ControlTimeline>,
  approval: HashMap<String, ControlTimeline>,
}

#[derive(Debug, Clone)]
enum TimelineStatus {
  Started,
  Succeeded,
  Failed(AttemptFailure),
}

fn redact_marker_value(value: &str, marker: &str) -> String {
  let mut output = value.to_string();
  let mut search_from = 0;
  loop {
    let lowered = output.to_ascii_lowercase();
    let Some(relative) = lowered[search_from..].find(marker) else {
      break;
    };
    let start = search_from + relative + marker.len();
    let end = output[start..]
      .char_indices()
      .find_map(|(offset, character)| {
        (character.is_whitespace() || matches!(character, '&' | '"' | '\'' | ']' | '}' | ',' | ';'))
          .then_some(start + offset)
      })
      .unwrap_or(output.len());
    if start == end {
      search_from = start;
      continue;
    }
    output.replace_range(start..end, "[redacted]");
    search_from = start + "[redacted]".len();
  }
  output
}

fn redact_secret_fragments(value: &str) -> String {
  let mut output = value.to_string();
  for marker in [
    "token=",
    "secret=",
    "password=",
    "passwd=",
    "api_key=",
    "apikey=",
    "access_key=",
    "private_key=",
    "authorization=",
    "idempotency_key=",
    "idempotencykey=",
  ] {
    output = redact_marker_value(&output, marker);
  }
  for marker in ["xoxb-", "xoxa-", "xoxp-", "xoxr-", "xoxs-", "xapp-"] {
    output = redact_marker_value(&output, marker);
  }

  let mut words = output.split_whitespace().peekable();
  let mut rebuilt = Vec::new();
  while let Some(word) = words.next() {
    rebuilt.push(word.to_string());
    if word.eq_ignore_ascii_case("bearer") || word.eq_ignore_ascii_case("basic") {
      if words.next().is_some() {
        rebuilt.push("[redacted]".to_string());
      }
    }
  }
  if rebuilt.len() > 1 && value.contains(char::is_whitespace) {
    rebuilt.join(" ")
  } else {
    output
  }
}

fn bounded(value: &str, maximum: usize) -> String {
  let redacted = redact_secret_fragments(value);
  let mut characters = redacted.chars();
  let truncated = characters.by_ref().take(maximum).collect::<String>();
  if characters.next().is_some() && maximum > 0 {
    let mut value = truncated
      .chars()
      .take(maximum.saturating_sub(1))
      .collect::<String>();
    value.push('…');
    value
  } else {
    truncated
  }
}

fn metadata_string(
  metadata: Option<&serde_json::Map<String, Value>>,
  key: &str,
  maximum: usize,
) -> Option<String> {
  metadata
    .and_then(|value| value.get(key))
    .and_then(Value::as_str)
    .map(|value| bounded(value, maximum))
}

fn literal<'a>(trigger: &'a CompiledTrigger, key: &str) -> Option<&'a Value> {
  let ValueExpression::Object { fields } = &trigger.config else {
    return None;
  };
  let ValueExpression::Literal { value } = fields.get(key)? else {
    return None;
  };
  Some(value)
}

fn literal_string(trigger: &CompiledTrigger, key: &str) -> Option<String> {
  literal(trigger, key)
    .and_then(Value::as_str)
    .map(|value| bounded(value, MAX_SHORT_TEXT))
}

fn literal_strings(trigger: &CompiledTrigger, key: &str) -> Vec<String> {
  let ValueExpression::Object { fields } = &trigger.config else {
    return Vec::new();
  };
  let Some(ValueExpression::Array { items }) = fields.get(key) else {
    return Vec::new();
  };
  items
    .iter()
    .filter_map(|item| match item {
      ValueExpression::Literal { value } => value.as_str(),
      _ => None,
    })
    .map(|value| bounded(value, MAX_SHORT_TEXT))
    .collect()
}

fn trigger_type(handler: &str) -> Result<PresentationTriggerType, RunPresentationError> {
  Ok(match handler {
    "trigger.manual" => PresentationTriggerType::Manual,
    "trigger.webhook" => PresentationTriggerType::Webhook,
    "trigger.slack" => PresentationTriggerType::Slack,
    "trigger.telegram" => PresentationTriggerType::Telegram,
    "trigger.schedule" => PresentationTriggerType::Schedule,
    "trigger.interval" => PresentationTriggerType::Interval,
    "trigger.event" => PresentationTriggerType::Event,
    _ => {
      return Err(RunPresentationError::UnsupportedTrigger(
        handler.to_string(),
      ))
    }
  })
}

fn trigger_presentation(
  trigger: &CompiledTrigger,
) -> Result<TriggerPresentationV1, RunPresentationError> {
  let kind = trigger_type(&trigger.handler)?;
  let scope = if matches!(
    kind,
    PresentationTriggerType::Slack | PresentationTriggerType::Telegram
  ) {
    let channels = literal_strings(trigger, "channels");
    let events = literal_strings(trigger, "events");
    let mut parts = Vec::new();
    if !channels.is_empty() {
      parts.push(
        channels
          .into_iter()
          .map(|channel| format!("#{channel}"))
          .collect::<Vec<_>>()
          .join(", "),
      );
    }
    if !events.is_empty() {
      parts.push(events.join(", "));
    }
    (!parts.is_empty()).then(|| bounded(&parts.join(" · "), MAX_SHORT_TEXT))
  } else {
    None
  };
  let interval = literal(trigger, "everyMs")
    .and_then(Value::as_u64)
    .map(|milliseconds| format!("{milliseconds}ms"));
  Ok(TriggerPresentationV1 {
    id: bounded(&trigger.id, 256),
    kind,
    label: None,
    method: literal_string(trigger, "method"),
    url: literal_string(trigger, "path"),
    schedule: literal_string(trigger, "cron"),
    timezone: literal_string(trigger, "timezone"),
    interval,
    event: literal_string(trigger, "name"),
    workspace: None,
    scope,
  })
}

fn workflow_presentation(
  workflow: &CompiledWorkflowDefinition,
  definition_hash: &str,
) -> Result<WorkflowPresentationV1, RunPresentationError> {
  let metadata = workflow.metadata.as_ref();
  Ok(WorkflowPresentationV1 {
    id: bounded(&workflow.workflow_id, 256),
    name: metadata
      .and_then(|value| value.name.as_deref())
      .map(|value| bounded(value, MAX_SHORT_TEXT)),
    description: metadata
      .and_then(|value| value.description.as_deref())
      .map(|value| bounded(value, MAX_MESSAGE)),
    version: metadata
      .and_then(|value| value.version.as_deref())
      .map(|value| bounded(value, 128)),
    definition_hash: definition_hash.to_string(),
    triggers: workflow
      .triggers
      .iter()
      .map(trigger_presentation)
      .collect::<Result<Vec<_>, _>>()?,
  })
}

fn is_sensitive_key(key: &str) -> bool {
  let normalized = key
    .chars()
    .filter(|character| !matches!(character, '-' | '_'))
    .flat_map(char::to_lowercase)
    .collect::<String>();
  [
    "authorization",
    "cookie",
    "setcookie",
    "password",
    "passwd",
    "secret",
    "token",
    "apikey",
    "accesskey",
    "privatekey",
    "credential",
    "idempotencykey",
    "capability",
    "approvalurl",
    "resumeurl",
  ]
  .iter()
  .any(|sensitive| normalized == *sensitive || normalized.ends_with(sensitive))
}

fn bounded_json_inner(value: &Value, key: &str, depth: usize, nodes: &mut usize) -> (Value, bool) {
  *nodes += 1;
  if *nodes > MAX_VALUE_NODES {
    return (Value::String("[preview limit reached]".to_string()), true);
  }
  if is_sensitive_key(key) {
    return (Value::String("[redacted]".to_string()), true);
  }
  match value {
    Value::Null | Value::Bool(_) | Value::Number(_) => (value.clone(), false),
    Value::String(value) => {
      let bounded = bounded(value, MAX_VALUE_STRING);
      let changed = bounded != *value;
      (Value::String(bounded), changed)
    }
    Value::Array(_) if depth >= MAX_VALUE_DEPTH => {
      (Value::String("[maximum depth reached]".to_string()), true)
    }
    Value::Array(items) => {
      let mut changed = items.len() > MAX_VALUE_ITEMS;
      let mut output = Vec::new();
      for item in items.iter().take(MAX_VALUE_ITEMS) {
        let (item, item_changed) = bounded_json_inner(item, "", depth + 1, nodes);
        changed |= item_changed;
        output.push(item);
      }
      if items.len() > MAX_VALUE_ITEMS {
        output.push(Value::String(format!(
          "[{} more items]",
          items.len() - MAX_VALUE_ITEMS
        )));
      }
      (Value::Array(output), changed)
    }
    Value::Object(_) if depth >= MAX_VALUE_DEPTH => {
      (Value::String("[maximum depth reached]".to_string()), true)
    }
    Value::Object(fields) => {
      let mut changed = fields.len() > MAX_VALUE_PROPERTIES;
      let mut output = serde_json::Map::new();
      for (name, item) in fields.iter().take(MAX_VALUE_PROPERTIES) {
        let (item, item_changed) = bounded_json_inner(item, name, depth + 1, nodes);
        changed |= item_changed;
        output.insert(bounded(name, MAX_SHORT_TEXT), item);
      }
      if fields.len() > MAX_VALUE_PROPERTIES {
        output.insert(
          "…".to_string(),
          Value::String(format!(
            "[{} more properties]",
            fields.len() - MAX_VALUE_PROPERTIES
          )),
        );
      }
      (Value::Object(output), changed)
    }
  }
}

fn bounded_json(value: &Value) -> (Value, bool) {
  bounded_json_inner(value, "", 0, &mut 0)
}

fn duration_between(start: Option<DateTime<Utc>>, end: Option<DateTime<Utc>>) -> Option<u64> {
  let milliseconds = end?.signed_duration_since(start?).num_milliseconds();
  Some((milliseconds.max(0) as u64).min(MAX_DURATION_MS))
}

fn timelines(events: &[RunEvent]) -> HashMap<String, AttemptTimeline> {
  let mut timelines = HashMap::<String, AttemptTimeline>::new();
  for event in events {
    match &event.payload {
      RunEventPayload::StepAttemptStarted(data) => {
        let timeline = timelines.entry(data.node_id.clone()).or_default();
        timeline.started_at.get_or_insert(event.occurred_at);
        timeline.attempts = timeline.attempts.max(data.attempt);
        timeline.latest = Some(TimelineStatus::Started);
      }
      RunEventPayload::StepAttemptSucceeded(data) => {
        let timeline = timelines.entry(data.node_id.clone()).or_default();
        timeline.completed_at = Some(event.occurred_at);
        timeline.attempts = timeline.attempts.max(data.attempt);
        timeline.latest = Some(TimelineStatus::Succeeded);
      }
      RunEventPayload::StepAttemptFailed(data) => {
        let timeline = timelines.entry(data.node_id.clone()).or_default();
        timeline.completed_at = Some(event.occurred_at);
        timeline.attempts = timeline.attempts.max(data.attempt);
        timeline.latest = Some(TimelineStatus::Failed(data.failure.clone()));
      }
      _ => {}
    }
  }
  timelines
}

fn control_timelines(events: &[RunEvent]) -> ControlTimelines {
  let mut value = ControlTimelines::default();
  for event in events {
    match &event.payload {
      RunEventPayload::ParallelGroupStarted(data) => {
        value
          .parallel
          .entry(data.parallel_id.clone())
          .or_default()
          .started_at = Some(event.occurred_at);
      }
      RunEventPayload::ChoiceSelected(data) => {
        let timeline = value.choice.entry(data.choice_id.clone()).or_default();
        timeline.started_at = Some(event.occurred_at);
        timeline.completed_at = Some(event.occurred_at);
      }
      RunEventPayload::ParallelGroupCompleted(data) => {
        value
          .parallel
          .entry(data.parallel_id.clone())
          .or_default()
          .completed_at = Some(event.occurred_at);
      }
      RunEventPayload::ForkOpened(data) => {
        value
          .fork
          .entry(data.fork_id.clone())
          .or_default()
          .started_at = Some(event.occurred_at);
      }
      RunEventPayload::ForkBranchSettled(data) => {
        value
          .branch
          .entry((data.fork_id.clone(), data.branch_id.clone()))
          .or_default()
          .completed_at = Some(event.occurred_at);
      }
      RunEventPayload::ForkJoinSettled(data) => {
        value
          .fork
          .entry(data.fork_id.clone())
          .or_default()
          .completed_at = Some(event.occurred_at);
      }
      RunEventPayload::ApprovalRequested(data) => {
        value
          .approval
          .entry(data.approval_id.clone())
          .or_default()
          .started_at = Some(event.occurred_at);
      }
      RunEventPayload::ApprovalResolved(data) => {
        value
          .approval
          .entry(data.approval_id.clone())
          .or_default()
          .completed_at = Some(event.occurred_at);
      }
      _ => {}
    }
  }
  value
}

fn attempt_failure(failure: &AttemptFailure) -> PresentationFailureV1 {
  PresentationFailureV1 {
    code: bounded(&failure.code, 124),
    message: bounded(&failure.message, MAX_MESSAGE),
    kind: Some(format!("{:?}", failure.kind).to_ascii_lowercase()),
    retryable: failure.retryable,
  }
}

fn branch_failure(failure: &BranchFailure) -> PresentationFailureV1 {
  let message = match failure {
    BranchFailure::BranchTestNotBoolean { message, .. }
    | BranchFailure::ReferenceNotAvailable { message, .. }
    | BranchFailure::BranchSelectionInvalid { message, .. } => message,
  };
  PresentationFailureV1 {
    code: failure.code().to_string(),
    message: bounded(message, MAX_MESSAGE),
    kind: Some("branch".to_string()),
    retryable: Some(false),
  }
}

fn run_failure(failure: &RunFailure) -> PresentationFailureV1 {
  match failure {
    RunFailure::Attempt(failure) => attempt_failure(failure),
    RunFailure::Branch(failure) => branch_failure(failure),
    RunFailure::Parallel { failure, .. } => PresentationFailureV1 {
      code: failure.code.clone(),
      message: bounded(&failure.message, MAX_MESSAGE),
      kind: Some(failure.kind.clone()),
      retryable: Some(false),
    },
    RunFailure::Approval { failure, .. } => PresentationFailureV1 {
      code: failure.code.clone(),
      message: bounded(&failure.message, MAX_MESSAGE),
      kind: Some(failure.kind.clone()),
      retryable: Some(false),
    },
    RunFailure::Notification { failure, .. } => PresentationFailureV1 {
      code: failure.code.clone(),
      message: bounded(&failure.message, MAX_MESSAGE),
      kind: Some(failure.kind.clone()),
      retryable: Some(false),
    },
  }
}

fn presentation_run_status(projection: &RunProjection) -> PresentationRunStatus {
  if !projection.pending_retries.is_empty() {
    return PresentationRunStatus::Retrying;
  }
  if projection.timeout_reached_at.is_some() && projection.status == RunStatus::Failed {
    return PresentationRunStatus::TimedOut;
  }
  match projection.status {
    RunStatus::NotStarted | RunStatus::Queued => PresentationRunStatus::Queued,
    RunStatus::Running => PresentationRunStatus::Running,
    RunStatus::Waiting => PresentationRunStatus::Waiting,
    RunStatus::Cancelling => PresentationRunStatus::Cancelling,
    RunStatus::Finalizing => PresentationRunStatus::Finalizing,
    RunStatus::Succeeded => PresentationRunStatus::Succeeded,
    RunStatus::Failed => PresentationRunStatus::Failed,
    RunStatus::Cancelled => PresentationRunStatus::Cancelled,
  }
}

fn reusable_aliases(workflow: &CompiledWorkflowDefinition) -> HashMap<&str, &str> {
  workflow
    .reusable_definitions
    .as_deref()
    .unwrap_or_default()
    .iter()
    .filter_map(|definition| match definition {
      CompiledReusableInvocation::Step { node_id, alias, .. } => {
        Some((node_id.as_str(), alias.as_str()))
      }
      CompiledReusableInvocation::NotificationProvider { .. } => None,
    })
    .collect()
}

fn selected_choice(
  workflow: &CompiledWorkflowDefinition,
  node_id: &str,
  projection: &RunProjection,
) -> Option<String> {
  if let Some(choice) = workflow
    .graph
    .choices
    .as_deref()
    .unwrap_or_default()
    .iter()
    .find(|choice| {
      choice.result_node_id.as_deref() == Some(node_id) || choice.selector_node_id == node_id
    })
  {
    let arm = projection.choice_selections.get(&choice.choice_id)?;
    if choice.string_selector.is_some() {
      if let Some(value) = choice
        .string_cases
        .as_deref()
        .unwrap_or_default()
        .iter()
        .find(|case| case.arm_id == *arm)
        .map(|case| case.value.as_str())
      {
        return Some(bounded(&format!("Selected \"{value}\"."), MAX_MESSAGE));
      }
      return Some("Selected default.".to_string());
    }
    if choice.default_arm_id.as_deref() == Some(arm) || arm.ends_with(":otherwise") {
      return Some("Selected otherwise.".to_string());
    }
    let index = choice
      .arm_ids
      .iter()
      .position(|candidate| candidate == arm)
      .unwrap_or(0)
      + 1;
    return Some(format!("Selected condition {index}."));
  }
  projection.branch_selections.get(node_id).map(|arm| {
    if arm.ends_with(":otherwise") {
      "Selected otherwise.".to_string()
    } else if let Some(index) = arm
      .rsplit(':')
      .next()
      .and_then(|value| value.parse::<usize>().ok())
    {
      format!("Selected condition {}.", index + 1)
    } else {
      bounded(&format!("Selected case \"{arm}\"."), MAX_MESSAGE)
    }
  })
}

fn combine_details(parts: impl IntoIterator<Item = Option<String>>) -> Option<String> {
  let joined = parts.into_iter().flatten().collect::<Vec<_>>().join(" · ");
  (!joined.is_empty()).then(|| bounded(&joined, MAX_MESSAGE))
}

fn retry_detail(
  node_id: &str,
  timeline: Option<&AttemptTimeline>,
  status: PresentationStepStatus,
  projection: &RunProjection,
) -> Option<String> {
  if let Some(retry) = projection.pending_retries.get(node_id) {
    return Some(format!(
      "Attempt {} scheduled for {}.",
      retry.next_attempt,
      retry.scheduled_at.to_rfc3339()
    ));
  }
  let attempts = timeline.map_or(1, |timeline| timeline.attempts.max(1));
  if attempts <= 1 {
    return None;
  }
  Some(match status {
    PresentationStepStatus::Succeeded => format!("Succeeded after {attempts} attempts."),
    PresentationStepStatus::Failed | PresentationStepStatus::TimedOut => {
      format!("Retry exhausted after {attempts} attempts.")
    }
    _ => format!("Attempt {attempts} in progress."),
  })
}

fn workflow_operation(
  node_id: &str,
  projection: &RunProjection,
) -> Option<(PresentationStepKind, String)> {
  let operations = projection
    .operations
    .values()
    .filter(|operation| {
      operation.node_id == node_id
        && operation.capability == "workflows"
        && matches!(operation.operation.as_str(), "call" | "start")
    })
    .collect::<Vec<_>>();
  if operations.is_empty() {
    return None;
  }
  let kind = if operations
    .iter()
    .all(|operation| operation.operation == "call")
  {
    PresentationStepKind::WorkflowCall
  } else if operations
    .iter()
    .all(|operation| operation.operation == "start")
  {
    PresentationStepKind::WorkflowStart
  } else {
    PresentationStepKind::Script
  };
  let details = operations
    .iter()
    .map(|operation| {
      let target = operation
        .metadata
        .get("targetWorkflowId")
        .and_then(Value::as_str)
        .unwrap_or("unknown workflow");
      let child = operation
        .metadata
        .get("childRunId")
        .and_then(Value::as_str)
        .unwrap_or("child run pending");
      match (&operation.operation[..], &operation.status) {
        ("call", crate::projection::OperationStatus::Started) => {
          format!("Waiting for {target} · {child}")
        }
        ("call", crate::projection::OperationStatus::Succeeded { .. }) => {
          format!("{target} completed · {child}")
        }
        ("call", crate::projection::OperationStatus::Failed { .. }) => {
          format!("{target} failed · {child}")
        }
        ("start", crate::projection::OperationStatus::Started) => {
          format!("Starting {target} · {child}")
        }
        ("start", crate::projection::OperationStatus::Succeeded { .. }) => {
          format!("Started {target} · {child} · detached")
        }
        ("start", crate::projection::OperationStatus::Failed { .. }) => {
          format!("Could not start {target} · {child}")
        }
        _ => format!("{target} · {child}"),
      }
    })
    .collect::<Vec<_>>()
    .join("; ");
  Some((kind, bounded(&details, MAX_MESSAGE)))
}

fn runtime_policy_detail(
  workflow: &CompiledWorkflowDefinition,
  projection: &RunProjection,
  node_id: &str,
) -> Option<String> {
  if !workflow
    .graph
    .entry_node_ids
    .iter()
    .any(|entry| entry == node_id)
  {
    return None;
  }
  let queue = projection.queue.as_deref().unwrap_or("workflow queue");
  if projection.status == RunStatus::Queued {
    let mut gates = Vec::new();
    if workflow
      .runtime_policy
      .as_ref()
      .and_then(|policy| policy.concurrency)
      .is_some()
    {
      gates.push("concurrency");
    }
    if workflow
      .runtime_policy
      .as_ref()
      .and_then(|policy| policy.rate_limit.as_ref())
      .is_some()
    {
      gates.push("rate limit");
    }
    return Some(if gates.is_empty() {
      format!("Waiting in queue {queue}.")
    } else {
      format!(
        "Waiting in queue {queue} for {} capacity.",
        gates.join(" and ")
      )
    });
  }
  let wait_ms = duration_between(projection.admitted_at, projection.started_at)?;
  (wait_ms > 0).then(|| format!("Started after {wait_ms} ms in queue {queue}."))
}

fn control_metadata<'a>(
  workflow: &'a CompiledWorkflowDefinition,
  node: &'a crate::model::CompiledWorkflowNode,
) -> Option<&'a serde_json::Map<String, Value>> {
  if node.handler == "engine.branch-result" {
    let selector = workflow
      .graph
      .choices
      .as_deref()
      .unwrap_or_default()
      .iter()
      .find(|choice| choice.result_node_id.as_deref() == Some(node.id.as_str()))
      .map(|choice| choice.selector_node_id.as_str());
    if let Some(selector) = selector {
      return workflow
        .graph
        .nodes
        .iter()
        .find(|candidate| candidate.id == selector)
        .and_then(|candidate| candidate.metadata.as_ref());
    }
  }
  if node.handler == "engine.parallel-join" {
    let start_id = format!("__woml_parallel__{}__start", node.id);
    return workflow
      .graph
      .nodes
      .iter()
      .find(|candidate| candidate.id == start_id)
      .and_then(|candidate| candidate.metadata.as_ref());
  }
  node.metadata.as_ref()
}

fn presentable_kind(
  workflow: &CompiledWorkflowDefinition,
  node: &crate::model::CompiledWorkflowNode,
  reusable: &HashMap<&str, &str>,
) -> Option<PresentationStepKind> {
  if reusable.contains_key(node.id.as_str()) {
    return Some(PresentationStepKind::CustomStep);
  }
  Some(match node.handler.as_str() {
    "runtime.script" => PresentationStepKind::Script,
    "engine.branch-result" => {
      if workflow
        .graph
        .choices
        .as_deref()
        .unwrap_or_default()
        .iter()
        .any(|choice| {
          choice.result_node_id.as_deref() == Some(node.id.as_str())
            && choice.string_selector.is_some()
        })
      {
        PresentationStepKind::Switch
      } else {
        PresentationStepKind::Choose
      }
    }
    "engine.choice-select" => {
      let choice = workflow
        .graph
        .choices
        .as_deref()
        .unwrap_or_default()
        .iter()
        .find(|choice| choice.selector_node_id == node.id && choice.result_node_id.is_none())?;
      if choice.string_selector.is_some() {
        PresentationStepKind::Switch
      } else {
        PresentationStepKind::Choose
      }
    }
    "engine.approval-wait" => PresentationStepKind::Approval,
    _ => return None,
  })
}

fn presentation_node_id(
  workflow: &CompiledWorkflowDefinition,
  node: &crate::model::CompiledWorkflowNode,
) -> String {
  if node.handler != "engine.choice-select" {
    return bounded(&node.id, 256);
  }
  let choices = workflow.graph.choices.as_deref().unwrap_or_default();
  let Some((index, choice)) = choices
    .iter()
    .enumerate()
    .find(|(_, choice)| choice.selector_node_id == node.id)
  else {
    return "choice".to_string();
  };
  let label = if choice.string_selector.is_some() {
    "switch"
  } else {
    "choose"
  };
  format!("{label}-{}", index + 1)
}

fn choice_status(
  workflow: &CompiledWorkflowDefinition,
  node_id: &str,
  projection: &RunProjection,
) -> PresentationStepStatus {
  let selected = workflow
    .graph
    .choices
    .as_deref()
    .unwrap_or_default()
    .iter()
    .find(|choice| choice.selector_node_id == node_id)
    .is_some_and(|choice| projection.choice_selections.contains_key(&choice.choice_id));
  if selected {
    PresentationStepStatus::Succeeded
  } else {
    inactive_status(projection)
  }
}

fn inactive_status(projection: &RunProjection) -> PresentationStepStatus {
  if matches!(
    projection.status,
    RunStatus::Succeeded | RunStatus::Failed | RunStatus::Cancelled
  ) {
    PresentationStepStatus::Skipped
  } else {
    PresentationStepStatus::Queued
  }
}

fn parallel_status(parallel_id: &str, projection: &RunProjection) -> PresentationStepStatus {
  match projection
    .parallel_groups
    .get(parallel_id)
    .map(|group| &group.status)
  {
    Some(crate::projection::ParallelGroupStatus::Started) => PresentationStepStatus::Running,
    Some(crate::projection::ParallelGroupStatus::Completed { outcome, .. }) => match outcome {
      crate::event::ParallelGroupOutcome::Succeeded => PresentationStepStatus::Succeeded,
      crate::event::ParallelGroupOutcome::Failed => PresentationStepStatus::Failed,
    },
    None => inactive_status(projection),
  }
}

fn parallel_detail(
  definition: &crate::model::ParallelGroupDefinition,
  projection: &RunProjection,
) -> String {
  let policy = if definition.on_error == "fail-fast" {
    "fail fast"
  } else {
    "wait for all"
  };
  let outcome = match projection
    .parallel_groups
    .get(&definition.parallel_id)
    .map(|group| &group.status)
  {
    Some(crate::projection::ParallelGroupStatus::Started) => "running".to_string(),
    Some(crate::projection::ParallelGroupStatus::Completed {
      outcome: crate::event::ParallelGroupOutcome::Succeeded,
      ..
    }) => "all children completed".to_string(),
    Some(crate::projection::ParallelGroupStatus::Completed {
      failed_node_ids,
      cancelled_node_ids,
      ..
    }) => format!(
      "{} failed, {} cancelled",
      failed_node_ids.len(),
      cancelled_node_ids.len()
    ),
    None => "not started".to_string(),
  };
  bounded(
    &format!(
      "{} children · up to {} at once · {policy} · {outcome}.",
      definition.child_node_ids.len(),
      definition.concurrency
    ),
    MAX_MESSAGE,
  )
}

fn parallel_failure(
  parallel_id: &str,
  projection: &RunProjection,
) -> Option<PresentationFailureV1> {
  match projection.failure.as_ref() {
    Some(RunFailure::Parallel {
      parallel_id: failed_parallel,
      failure,
      ..
    }) if failed_parallel == parallel_id => Some(PresentationFailureV1 {
      code: bounded(&failure.code, 124),
      message: bounded(&failure.message, MAX_MESSAGE),
      kind: Some(failure.kind.clone()),
      retryable: Some(false),
    }),
    _ => None,
  }
}

fn approval_status(approval_id: &str, projection: &RunProjection) -> PresentationStepStatus {
  match projection
    .approval_requests
    .get(approval_id)
    .map(|request| &request.status)
  {
    Some(crate::projection::ApprovalRequestStatus::Waiting) => PresentationStepStatus::Waiting,
    Some(crate::projection::ApprovalRequestStatus::Resolved { resolution, .. }) => match resolution
    {
      crate::event::ApprovalResolution::TimeoutFailure => PresentationStepStatus::Failed,
      crate::event::ApprovalResolution::Decision { .. } => PresentationStepStatus::Succeeded,
    },
    None => inactive_status(projection),
  }
}

fn approval_detail(approval_id: &str, projection: &RunProjection) -> Option<String> {
  let request = projection.approval_requests.get(approval_id)?;
  let deliveries = projection
    .notification_deliveries
    .values()
    .filter(|delivery| {
      delivery.approval_id == approval_id && delivery.request_id == request.request_id
    })
    .collect::<Vec<_>>();
  let delivered = deliveries
    .iter()
    .filter(|delivery| {
      matches!(
        delivery.status,
        crate::projection::NotificationDeliveryStatus::Succeeded { .. }
      )
    })
    .count();
  let failed = deliveries
    .iter()
    .filter(|delivery| {
      matches!(
        delivery.status,
        crate::projection::NotificationDeliveryStatus::Failed { final_: true, .. }
      )
    })
    .count();
  let providers = deliveries
    .iter()
    .map(|delivery| delivery.provider.as_str())
    .collect::<BTreeSet<_>>()
    .into_iter()
    .collect::<Vec<_>>()
    .join(", ");
  let notification = if deliveries.is_empty() {
    None
  } else {
    Some(format!(
      "Notifications {delivered}/{} delivered{}{}",
      deliveries.len(),
      if failed == 0 {
        String::new()
      } else {
        format!(", {failed} failed")
      },
      if providers.is_empty() {
        String::new()
      } else {
        format!(" via {providers}")
      }
    ))
  };
  let decision = match &request.status {
    crate::projection::ApprovalRequestStatus::Waiting => Some(match request.expires_at {
      Some(deadline) => format!("Waiting for decision until {}", deadline.to_rfc3339()),
      None => "Waiting for decision".to_string(),
    }),
    crate::projection::ApprovalRequestStatus::Resolved { resolution, .. } => {
      Some(match resolution {
        crate::event::ApprovalResolution::Decision { decision, source } => {
          let decision = format!("{decision:?}").to_ascii_lowercase();
          let source = match source {
            crate::event::ApprovalDecisionSource::Human => projection
              .notification_decisions
              .iter()
              .rev()
              .find(|accepted| {
                accepted.approval_id == approval_id && accepted.request_id == request.request_id
              })
              .map(|accepted| format!(" by {}", accepted.provider))
              .unwrap_or_else(|| " by human".to_string()),
            crate::event::ApprovalDecisionSource::Timeout => " at timeout".to_string(),
          };
          format!("Decision {decision}{source}")
        }
        crate::event::ApprovalResolution::TimeoutFailure => {
          "Approval failed at timeout".to_string()
        }
      })
    }
  };
  combine_details([decision, notification])
}

fn approval_failure(
  approval_id: &str,
  projection: &RunProjection,
) -> Option<PresentationFailureV1> {
  match projection.failure.as_ref() {
    Some(RunFailure::Approval {
      approval_id: failed_approval,
      failure,
      ..
    }) if failed_approval == approval_id => Some(PresentationFailureV1 {
      code: bounded(&failure.code, 124),
      message: bounded(&failure.message, MAX_MESSAGE),
      kind: Some(failure.kind.clone()),
      retryable: Some(false),
    }),
    Some(RunFailure::Notification {
      approval_id: failed_approval,
      failure,
      ..
    }) if failed_approval == approval_id => Some(PresentationFailureV1 {
      code: bounded(&failure.code, 124),
      message: bounded(&failure.message, MAX_MESSAGE),
      kind: Some(failure.kind.clone()),
      retryable: Some(false),
    }),
    _ => None,
  }
}

fn fork_status(fork_id: &str, projection: &RunProjection) -> PresentationStepStatus {
  match projection.forks.get(fork_id).map(|fork| fork.join_status) {
    Some(crate::projection::ForkJoinStatus::Pending) => PresentationStepStatus::Running,
    Some(crate::projection::ForkJoinStatus::Succeeded) => PresentationStepStatus::Succeeded,
    Some(crate::projection::ForkJoinStatus::Failed) => PresentationStepStatus::Failed,
    Some(crate::projection::ForkJoinStatus::Cancelled) => PresentationStepStatus::Cancelled,
    None => inactive_status(projection),
  }
}

fn fork_detail(fork: &crate::model::CompiledFork, projection: &RunProjection) -> String {
  let joined = fork.joined_branch_ids.len();
  let join_profile = if joined == 0 {
    "join none".to_string()
  } else if joined == fork.branches.len() {
    "join all".to_string()
  } else {
    format!("join {}", fork.joined_branch_ids.join(", "))
  };
  let settlement = match projection.forks.get(&fork.fork_id) {
    Some(value) => match value.join_status {
      crate::projection::ForkJoinStatus::Pending => {
        format!("waiting for {joined} joined branches")
      }
      crate::projection::ForkJoinStatus::Succeeded => {
        format!("continuation released after {joined} branches")
      }
      crate::projection::ForkJoinStatus::Failed => value
        .blocking_branch_id
        .as_deref()
        .map(|branch| format!("join blocked by {branch}"))
        .unwrap_or_else(|| "join failed".to_string()),
      crate::projection::ForkJoinStatus::Cancelled => "join cancelled".to_string(),
    },
    None => "not started".to_string(),
  };
  bounded(
    &format!(
      "{} branches · {join_profile} · {settlement}.",
      fork.branches.len()
    ),
    MAX_MESSAGE,
  )
}

fn fork_failure(fork_id: &str, projection: &RunProjection) -> Option<PresentationFailureV1> {
  (fork_status(fork_id, projection) == PresentationStepStatus::Failed)
    .then(|| projection.failure.as_ref().map(run_failure))
    .flatten()
}

fn fork_branch_status(
  fork_id: &str,
  branch_id: &str,
  projection: &RunProjection,
) -> PresentationStepStatus {
  match projection
    .forks
    .get(fork_id)
    .and_then(|fork| fork.branches.get(branch_id))
    .and_then(|branch| branch.outcome)
  {
    Some(crate::event::ForkBranchOutcome::Succeeded) => PresentationStepStatus::Succeeded,
    Some(crate::event::ForkBranchOutcome::Failed) => PresentationStepStatus::Failed,
    Some(crate::event::ForkBranchOutcome::Cancelled) => PresentationStepStatus::Cancelled,
    None if projection.forks.contains_key(fork_id) => PresentationStepStatus::Running,
    None => inactive_status(projection),
  }
}

fn cancelled_nodes(projection: &RunProjection) -> HashSet<&str> {
  projection
    .parallel_groups
    .values()
    .flat_map(|group| match &group.status {
      crate::projection::ParallelGroupStatus::Completed {
        cancelled_node_ids, ..
      } => cancelled_node_ids
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>(),
      crate::projection::ParallelGroupStatus::Started => Vec::new(),
    })
    .collect()
}

fn step_status(
  node_id: &str,
  timeline: Option<&AttemptTimeline>,
  projection: &RunProjection,
  cancelled: &HashSet<&str>,
) -> PresentationStepStatus {
  if projection.pending_retries.contains_key(node_id) {
    return PresentationStepStatus::Retrying;
  }
  if cancelled.contains(node_id) {
    return PresentationStepStatus::Cancelled;
  }
  match timeline.and_then(|timeline| timeline.latest.as_ref()) {
    Some(TimelineStatus::Started) => {
      if projection.status == RunStatus::Waiting
        && projection.approval_requests.contains_key(node_id)
      {
        PresentationStepStatus::Waiting
      } else {
        PresentationStepStatus::Running
      }
    }
    Some(TimelineStatus::Succeeded) => PresentationStepStatus::Succeeded,
    Some(TimelineStatus::Failed(failure)) if failure.kind == AttemptFailureKind::ScriptTimedOut => {
      PresentationStepStatus::TimedOut
    }
    Some(TimelineStatus::Failed(failure))
      if failure.kind == AttemptFailureKind::InvocationCancelled =>
    {
      if projection.timeout_reached_at.is_some() {
        PresentationStepStatus::TimedOut
      } else {
        PresentationStepStatus::Cancelled
      }
    }
    Some(TimelineStatus::Failed(_)) => PresentationStepStatus::Failed,
    None
      if matches!(
        projection.status,
        RunStatus::Succeeded | RunStatus::Failed | RunStatus::Cancelled
      ) =>
    {
      PresentationStepStatus::Skipped
    }
    None => PresentationStepStatus::Queued,
  }
}

fn steps(
  workflow: &CompiledWorkflowDefinition,
  projection: &RunProjection,
  events: &[RunEvent],
) -> Result<Vec<StepPresentationV1>, RunPresentationError> {
  let timelines = timelines(events);
  let control_times = control_timelines(events);
  let reusable = reusable_aliases(workflow);
  let cancelled = cancelled_nodes(projection);
  let mut steps = Vec::new();
  for node in &workflow.graph.nodes {
    if node.handler == "engine.parallel-start" {
      let Some(parallel_id) = node
        .id
        .strip_prefix("__woml_parallel__")
        .and_then(|value| value.strip_suffix("__start"))
      else {
        continue;
      };
      let definition = workflow.parallel_group(parallel_id);
      let timeline = control_times
        .parallel
        .get(parallel_id)
        .copied()
        .unwrap_or_default();
      let (result, result_truncated) = definition
        .as_ref()
        .and_then(|definition| {
          let fields = definition
            .child_node_ids
            .iter()
            .filter_map(|id| {
              projection
                .context
                .steps
                .get(id)
                .map(|value| (id.clone(), value.clone()))
            })
            .collect::<serde_json::Map<_, _>>();
          (!fields.is_empty()).then_some(Value::Object(fields))
        })
        .map(|value| bounded_json(&value))
        .map_or((None, None), |(value, truncated)| {
          (Some(value), truncated.then_some(true))
        });
      let metadata = node.metadata.as_ref();
      if steps.len() >= RUN_PRESENTATION_MAX_STEPS {
        return Err(RunPresentationError::TooMany("steps"));
      }
      steps.push(StepPresentationV1 {
        id: bounded(parallel_id, 256),
        name: metadata_string(metadata, "name", MAX_SHORT_TEXT),
        description: metadata_string(metadata, "description", MAX_MESSAGE),
        kind: PresentationStepKind::Parallel,
        status: parallel_status(parallel_id, projection),
        depth: 0,
        started_at: timeline.started_at,
        completed_at: timeline.completed_at,
        duration_ms: duration_between(timeline.started_at, timeline.completed_at),
        attempts: 1,
        detail: combine_details([
          runtime_policy_detail(workflow, projection, &node.id),
          definition
            .as_ref()
            .map(|definition| parallel_detail(definition, projection)),
        ]),
        result,
        result_truncated,
        failure: parallel_failure(parallel_id, projection),
      });
      continue;
    }
    if node.handler == "engine.fork-open" {
      let Some(fork) = workflow
        .graph
        .forks
        .as_deref()
        .unwrap_or_default()
        .iter()
        .find(|fork| fork.open_node_id == node.id)
      else {
        continue;
      };
      let timeline = control_times
        .fork
        .get(&fork.fork_id)
        .copied()
        .unwrap_or_default();
      let metadata = node.metadata.as_ref();
      if steps.len() >= RUN_PRESENTATION_MAX_STEPS {
        return Err(RunPresentationError::TooMany("steps"));
      }
      steps.push(StepPresentationV1 {
        id: bounded(&fork.fork_id, 256),
        name: metadata_string(metadata, "name", MAX_SHORT_TEXT),
        description: metadata_string(metadata, "description", MAX_MESSAGE),
        kind: PresentationStepKind::Fork,
        status: fork_status(&fork.fork_id, projection),
        depth: 0,
        started_at: timeline.started_at,
        completed_at: timeline.completed_at,
        duration_ms: duration_between(timeline.started_at, timeline.completed_at),
        attempts: 1,
        detail: combine_details([
          runtime_policy_detail(workflow, projection, &node.id),
          Some(fork_detail(fork, projection)),
        ]),
        result: None,
        result_truncated: None,
        failure: fork_failure(&fork.fork_id, projection),
      });
      continue;
    }
    if matches!(
      node.handler.as_str(),
      "engine.parallel-join" | "engine.fork-join"
    ) {
      continue;
    }
    let fork_owner = workflow.fork_branch_owner(&node.id);
    if let Some((fork, branch)) = fork_owner {
      if branch.entry_node_id == node.id {
        let timeline = control_times
          .branch
          .get(&(fork.fork_id.clone(), branch.branch_id.clone()))
          .copied()
          .unwrap_or_default();
        let started_at = control_times
          .fork
          .get(&fork.fork_id)
          .and_then(|timeline| timeline.started_at);
        if steps.len() >= RUN_PRESENTATION_MAX_STEPS {
          return Err(RunPresentationError::TooMany("steps"));
        }
        steps.push(StepPresentationV1 {
          id: bounded(&branch.branch_id, 256),
          name: None,
          description: None,
          kind: PresentationStepKind::Branch,
          status: fork_branch_status(&fork.fork_id, &branch.branch_id, projection),
          depth: 1,
          started_at,
          completed_at: timeline.completed_at,
          duration_ms: duration_between(started_at, timeline.completed_at),
          attempts: 1,
          detail: Some(
            if fork.joined_branch_ids.contains(&branch.branch_id) {
              "Joins the continuation; the fork waits for this branch."
            } else {
              "Runs independently; this branch does not delay the continuation."
            }
            .to_string(),
          ),
          result: None,
          result_truncated: None,
          failure: None,
        });
      }
    }
    let Some(mut kind) = presentable_kind(workflow, node, &reusable) else {
      continue;
    };
    if steps.len() >= RUN_PRESENTATION_MAX_STEPS {
      return Err(RunPresentationError::TooMany("steps"));
    }
    let metadata = control_metadata(workflow, node);
    let timeline = timelines.get(&node.id);
    let status = match node.handler.as_str() {
      "engine.approval-wait" => approval_status(&node.id, projection),
      "engine.choice-select" => choice_status(workflow, &node.id, projection),
      _ => step_status(&node.id, timeline, projection, &cancelled),
    };
    let (result, result_truncated) = projection
      .context
      .steps
      .get(&node.id)
      .map(bounded_json)
      .map_or((None, None), |(value, truncated)| {
        (Some(value), truncated.then_some(true))
      });
    let mut failure = timeline.and_then(|timeline| match timeline.latest.as_ref() {
      Some(TimelineStatus::Failed(failure)) => Some(attempt_failure(failure)),
      _ => None,
    });
    let workflow_operation = workflow_operation(&node.id, projection);
    if let Some((operation_kind, _)) = workflow_operation.as_ref() {
      kind = *operation_kind;
    }
    let mut detail = combine_details([
      runtime_policy_detail(workflow, projection, &node.id),
      selected_choice(workflow, &node.id, projection),
      workflow_operation.map(|(_, detail)| detail),
      retry_detail(&node.id, timeline, status, projection),
    ]);
    if node.handler == "engine.approval-wait" {
      detail = approval_detail(&node.id, projection);
      failure = approval_failure(&node.id, projection).or(failure);
    }
    let name = metadata_string(metadata, "name", MAX_SHORT_TEXT).or_else(|| {
      reusable
        .get(node.id.as_str())
        .map(|alias| bounded(alias, MAX_SHORT_TEXT))
    });
    let control_timeline = if node.handler == "engine.approval-wait" {
      Some(
        control_times
          .approval
          .get(&node.id)
          .copied()
          .unwrap_or_default(),
      )
    } else if node.handler == "engine.choice-select" {
      workflow
        .graph
        .choices
        .as_deref()
        .unwrap_or_default()
        .iter()
        .find(|choice| choice.selector_node_id == node.id)
        .map(|choice| {
          control_times
            .choice
            .get(&choice.choice_id)
            .copied()
            .unwrap_or_default()
        })
    } else {
      None
    };
    let started_at = control_timeline
      .and_then(|timeline| timeline.started_at)
      .or_else(|| timeline.and_then(|timeline| timeline.started_at));
    let completed_at = control_timeline
      .and_then(|timeline| timeline.completed_at)
      .or_else(|| timeline.and_then(|timeline| timeline.completed_at));
    let depth = if fork_owner.is_some() {
      2
    } else if workflow.parallel_group_for_child(&node.id).is_some() {
      1
    } else {
      0
    };
    steps.push(StepPresentationV1 {
      id: presentation_node_id(workflow, node),
      name,
      description: metadata_string(metadata, "description", MAX_MESSAGE),
      kind,
      status,
      depth,
      started_at,
      completed_at,
      duration_ms: duration_between(started_at, completed_at),
      attempts: timeline.map_or(1, |timeline| timeline.attempts.max(1)),
      detail,
      result,
      result_truncated,
      failure,
    });
  }
  Ok(steps)
}

fn lifecycle_hook(event: LifecycleEventName) -> PresentationLifecycleHook {
  match event {
    LifecycleEventName::RunStart => PresentationLifecycleHook::OnStart,
    LifecycleEventName::StepStart => PresentationLifecycleHook::OnStepStart,
    LifecycleEventName::StepSuccess => PresentationLifecycleHook::OnStepSuccess,
    LifecycleEventName::StepFailure => PresentationLifecycleHook::OnStepFailure,
    LifecycleEventName::StepComplete => PresentationLifecycleHook::OnStepComplete,
    LifecycleEventName::RunSuccess => PresentationLifecycleHook::OnSuccess,
    LifecycleEventName::RunFailure => PresentationLifecycleHook::OnError,
    LifecycleEventName::RunCancel => PresentationLifecycleHook::OnCancel,
    LifecycleEventName::RunComplete => PresentationLifecycleHook::OnComplete,
  }
}

fn provider_label(handler: &str) -> String {
  let value = handler
    .strip_prefix("notification.")
    .or_else(|| handler.strip_prefix("runtime."))
    .unwrap_or(handler)
    .replace(['_', '-'], " ");
  bounded(&value, MAX_SHORT_TEXT)
}

fn reusable_lifecycle_hook(hook: crate::event::ReusableLifecycleHook) -> PresentationLifecycleHook {
  match hook {
    crate::event::ReusableLifecycleHook::OnSuccess => PresentationLifecycleHook::OnSuccess,
    crate::event::ReusableLifecycleHook::OnError => PresentationLifecycleHook::OnError,
    crate::event::ReusableLifecycleHook::OnComplete => PresentationLifecycleHook::OnComplete,
  }
}

fn lifecycle(
  workflow: &CompiledWorkflowDefinition,
  projection: &RunProjection,
  events: &[RunEvent],
) -> Result<Vec<LifecyclePresentationV1>, RunPresentationError> {
  let mut order = Vec::new();
  let mut times = HashMap::<String, (DateTime<Utc>, Option<DateTime<Utc>>)>::new();
  for event in events {
    match &event.payload {
      RunEventPayload::LifecycleHookRequested(data) => {
        order.push(data.hook_invocation_id.clone());
        times.insert(data.hook_invocation_id.clone(), (event.occurred_at, None));
      }
      RunEventPayload::LifecycleHookCompleted(data) => {
        if let Some((_, completed)) = times.get_mut(&data.hook_invocation_id) {
          *completed = Some(event.occurred_at);
        }
      }
      _ => {}
    }
  }
  let compiled = workflow.lifecycle.as_ref();
  let mut output = Vec::new();
  for invocation_id in order {
    if output.len() >= RUN_PRESENTATION_MAX_LIFECYCLE {
      return Err(RunPresentationError::TooMany("lifecycle items"));
    }
    let Some(hook) = projection.lifecycle_hooks.get(&invocation_id) else {
      continue;
    };
    let status = match hook.status {
      LifecycleHookStatus::Requested => PresentationStepStatus::Queued,
      LifecycleHookStatus::Running => PresentationStepStatus::Running,
      LifecycleHookStatus::Completed | LifecycleHookStatus::CompletedWithWarnings => {
        PresentationStepStatus::Succeeded
      }
    };
    let failure = hook.actions.values().find_map(|action| {
      (action.status == LifecycleActionStatus::Failed)
        .then(|| action.failure.as_ref())
        .flatten()
        .map(|failure| PresentationFailureV1 {
          code: bounded(&failure.code, 124),
          message: bounded(&failure.message, MAX_MESSAGE),
          kind: Some(format!("{:?}", failure.kind).to_ascii_lowercase()),
          retryable: Some(false),
        })
    });
    let definition = compiled.and_then(|lifecycle| {
      lifecycle
        .hooks
        .iter()
        .find(|item| item.hook_id == hook.hook_id)
    });
    let providers = definition
      .into_iter()
      .flat_map(|definition| definition.actions.iter())
      .map(|action| provider_label(&action.handler))
      .collect::<BTreeSet<_>>()
      .into_iter()
      .collect::<Vec<_>>();
    let provider = (!providers.is_empty()).then(|| providers.join(", "));
    let action_count = definition.map_or(hook.actions.len(), |item| item.actions.len());
    let subject = match hook.subject.kind {
      crate::event::LifecycleSubjectKind::Workflow => "Workflow".to_string(),
      crate::event::LifecycleSubjectKind::Step => format!("Step {}", hook.subject.id),
    };
    let detail = Some(format!(
      "{subject} · {action_count} action{}{}",
      if action_count == 1 { "" } else { "s" },
      if hook.failed_actions == 0 {
        String::new()
      } else {
        format!(
          " · {} warning{}",
          hook.failed_actions,
          if hook.failed_actions == 1 { "" } else { "s" }
        )
      }
    ));
    let (started, completed) = times
      .get(&invocation_id)
      .copied()
      .unwrap_or((events[0].occurred_at, None));
    output.push(LifecyclePresentationV1 {
      hook: lifecycle_hook(hook.event),
      status,
      duration_ms: duration_between(Some(started), completed),
      provider,
      detail,
      failure,
    });
  }

  let mut reusable_times = HashMap::<
    (String, crate::event::ReusableLifecycleHook),
    (DateTime<Utc>, Option<DateTime<Utc>>),
  >::new();
  let mut reusable_order = Vec::new();
  for event in events {
    match &event.payload {
      RunEventPayload::ReusableLifecycleRequested(data) => {
        let key = (data.invocation_id.clone(), data.hook);
        reusable_order.push(key.clone());
        reusable_times.insert(key, (event.occurred_at, None));
      }
      RunEventPayload::ReusableLifecycleCompleted(data) => {
        if let Some((_, completed)) =
          reusable_times.get_mut(&(data.invocation_id.clone(), data.hook))
        {
          *completed = Some(event.occurred_at);
        }
      }
      _ => {}
    }
  }
  for (invocation_id, reusable_hook) in reusable_order {
    if output.len() >= RUN_PRESENTATION_MAX_LIFECYCLE {
      return Err(RunPresentationError::TooMany("lifecycle items"));
    }
    let key = format!("{}:{:?}", invocation_id, reusable_hook);
    let Some(hook) = projection.reusable_lifecycle_hooks.get(&key) else {
      continue;
    };
    let status = match hook.status {
      crate::projection::ReusableLifecycleStatus::Requested => PresentationStepStatus::Queued,
      crate::projection::ReusableLifecycleStatus::Running => PresentationStepStatus::Running,
      crate::projection::ReusableLifecycleStatus::Completed
      | crate::projection::ReusableLifecycleStatus::CompletedWithWarnings => {
        PresentationStepStatus::Succeeded
      }
    };
    let alias = workflow
      .reusable_definitions
      .as_deref()
      .unwrap_or_default()
      .iter()
      .find_map(|definition| match definition {
        CompiledReusableInvocation::Step {
          invocation_id: candidate,
          alias,
          ..
        } if candidate == &invocation_id => Some(alias.as_str()),
        CompiledReusableInvocation::NotificationProvider {
          provider_id, alias, ..
        } if provider_id == &invocation_id => Some(alias.as_str()),
        _ => None,
      })
      .unwrap_or(invocation_id.as_str());
    let (started, completed) = reusable_times
      .get(&(invocation_id.clone(), reusable_hook))
      .copied()
      .unwrap_or((events[0].occurred_at, None));
    let hook_name = match reusable_hook {
      crate::event::ReusableLifecycleHook::OnSuccess => "on-success",
      crate::event::ReusableLifecycleHook::OnError => "on-error",
      crate::event::ReusableLifecycleHook::OnComplete => "on-complete",
    };
    output.push(LifecyclePresentationV1 {
      hook: reusable_lifecycle_hook(reusable_hook),
      status,
      duration_ms: duration_between(Some(started), completed),
      provider: Some(bounded(alias, MAX_SHORT_TEXT)),
      detail: Some(bounded(
        &format!(
          "Reusable {hook_name} · {} action{}{}",
          hook.completed_action_ids.len(),
          if hook.completed_action_ids.len() == 1 {
            ""
          } else {
            "s"
          },
          if hook.warning_codes.is_empty() {
            String::new()
          } else {
            format!(
              " · {} warning{}",
              hook.warning_codes.len(),
              if hook.warning_codes.len() == 1 {
                ""
              } else {
                "s"
              }
            )
          }
        ),
        MAX_MESSAGE,
      )),
      failure: None,
    });
  }
  Ok(output)
}

fn summary(steps: &[StepPresentationV1]) -> RunPresentationSummaryV1 {
  let mut value = RunPresentationSummaryV1 {
    succeeded: 0,
    failed: 0,
    skipped: 0,
    cancelled: 0,
    total: steps.len() as u32,
  };
  for step in steps {
    match step.status {
      PresentationStepStatus::Succeeded => value.succeeded += 1,
      PresentationStepStatus::Failed | PresentationStepStatus::TimedOut => value.failed += 1,
      PresentationStepStatus::Skipped => value.skipped += 1,
      PresentationStepStatus::Cancelled => value.cancelled += 1,
      _ => {}
    }
  }
  value
}

pub fn project_run_presentation_v1(
  workflow: &CompiledWorkflowDefinition,
  definition_hash: &str,
  events: &[RunEvent],
) -> Result<RunPresentationV1, RunPresentationError> {
  if events.len() > RUN_PRESENTATION_MAX_EVENTS {
    return Err(RunPresentationError::TooMany("events"));
  }
  let first = events.first().ok_or(RunPresentationError::EmptyHistory)?;
  let projection = fold_events(events)?;
  if projection.definition_hash.as_deref() != Some(definition_hash)
    || projection
      .workflow_id
      .as_deref()
      .is_some_and(|workflow_id| workflow_id != workflow.workflow_id)
  {
    return Err(RunPresentationError::DefinitionMismatch);
  }
  let admitted_at = projection.admitted_at.unwrap_or(first.occurred_at);
  let terminal = matches!(
    projection.status,
    RunStatus::Succeeded | RunStatus::Failed | RunStatus::Cancelled
  );
  let completed_at = terminal.then(|| events.last().expect("non-empty history").occurred_at);
  let steps = steps(workflow, &projection, events)?;
  let lifecycle = lifecycle(workflow, &projection, events)?;
  let (result, result_truncated) = projection
    .result
    .as_ref()
    .map(bounded_json)
    .map_or((None, None), |(value, truncated)| {
      (Some(value), truncated.then_some(true))
    });
  let mut warnings = projection
    .lifecycle_warnings
    .iter()
    .take(RUN_PRESENTATION_MAX_WARNINGS)
    .map(|warning| PresentationFailureV1 {
      code: bounded(&warning.code, 124),
      message: bounded(
        &format!(
          "Lifecycle action \"{}\" completed with a warning.",
          warning.action_id
        ),
        MAX_MESSAGE,
      ),
      kind: Some("lifecycle_warning".to_string()),
      retryable: Some(false),
    })
    .collect::<Vec<_>>();
  if projection.lifecycle_warnings.len() > RUN_PRESENTATION_MAX_WARNINGS {
    return Err(RunPresentationError::TooMany("warnings"));
  }
  for reusable in projection.reusable_lifecycle_hooks.values() {
    for code in &reusable.warning_codes {
      if warnings.len() >= RUN_PRESENTATION_MAX_WARNINGS {
        return Err(RunPresentationError::TooMany("warnings"));
      }
      warnings.push(PresentationFailureV1 {
        code: bounded(code, 124),
        message: bounded(
          &format!(
            "Reusable lifecycle action for \"{}\" completed with a warning.",
            reusable.invocation_id
          ),
          MAX_MESSAGE,
        ),
        kind: Some("reusable_lifecycle_warning".to_string()),
        retryable: Some(false),
      });
    }
  }
  let failure = projection.failure.as_ref().map(run_failure).or_else(|| {
    projection
      .lifecycle_failure
      .as_ref()
      .map(|failure| PresentationFailureV1 {
        code: bounded(&failure.code, 124),
        message: bounded(&failure.message, MAX_MESSAGE),
        kind: Some(format!("{:?}", failure.kind).to_ascii_lowercase()),
        retryable: Some(false),
      })
  });
  warnings.shrink_to_fit();
  let legacy_trigger = (projection.ingress.is_none()
    && projection.trigger_id.is_none()
    && projection.trigger_handler.is_none())
  .then(|| workflow.triggers.first())
  .flatten();
  let trigger_handler = projection
    .trigger_handler
    .as_deref()
    .or_else(|| legacy_trigger.map(|trigger| trigger.handler.as_str()))
    .unwrap_or("trigger.event");
  let presentation = RunPresentationV1 {
    profile: RUN_PRESENTATION_PROFILE,
    workflow: workflow_presentation(workflow, definition_hash)?,
    run_id: projection
      .run_id
      .clone()
      .unwrap_or_else(|| first.run_id.clone()),
    trigger: RunPresentationTriggerV1 {
      id: projection
        .trigger_id
        .clone()
        .or_else(|| legacy_trigger.map(|trigger| trigger.id.clone()))
        .unwrap_or_else(|| "workflow-call".to_string()),
      kind: trigger_type(trigger_handler)?,
    },
    status: presentation_run_status(&projection),
    admitted_at,
    started_at: projection.started_at,
    completed_at,
    duration_ms: duration_between(projection.started_at.or(Some(admitted_at)), completed_at),
    summary: summary(&steps),
    steps,
    lifecycle,
    result,
    result_truncated,
    failure,
    warnings,
  };
  if serde_json::to_vec(&presentation)?.len() > RUN_PRESENTATION_MAX_BYTES {
    return Err(RunPresentationError::TooLarge);
  }
  Ok(presentation)
}

pub fn run_presentation_from_store_v1(
  store: &DurableEventStore,
  run_id: &str,
) -> Result<RunPresentationV1, RunPresentationError> {
  if store.event_count(run_id)? > RUN_PRESENTATION_MAX_EVENTS {
    return Err(RunPresentationError::TooMany("events"));
  }
  let binding = store.run_binding(run_id)?;
  let workflow = store.definition(&binding.definition_hash)?;
  let events = store.events(run_id)?;
  project_run_presentation_v1(&workflow, &binding.definition_hash, &events)
}

pub fn recent_run_presentations_from_store_v1(
  store: &DurableEventStore,
  workflow_id: &str,
  limit: usize,
) -> Result<RunPresentationListV1, RunPresentationError> {
  if !(1..=RUN_PRESENTATION_RECENT_LIMIT).contains(&limit) {
    return Err(RunPresentationError::InvalidRecentLimit);
  }
  let listed = store.list_runs_v2_filtered(limit, Some(workflow_id), None)?;
  let runs = listed
    .runs
    .iter()
    .map(|run| run_presentation_from_store_v1(store, &run.run_id))
    .collect::<Result<Vec<_>, _>>()?;
  let list = RunPresentationListV1 {
    profile: RUN_PRESENTATION_LIST_PROFILE,
    workflow_id: workflow_id.to_string(),
    runs,
  };
  if serde_json::to_vec(&list)?.len() > RUN_PRESENTATION_MAX_BYTES {
    return Err(RunPresentationError::TooLarge);
  }
  Ok(list)
}
