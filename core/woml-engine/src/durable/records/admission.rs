//! Durable records / admission.

use chrono::DateTime;
use chrono::Utc;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunDefinitionBinding {
  pub run_id: String,
  pub workflow_id: String,
  pub definition_hash: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TriggerAdmissionRequest {
  pub workflow_id: String,
  pub definition_hash: String,
  pub trigger_id: String,
  pub trigger_handler: String,
  pub source_identity: String,
  pub payload: Map<String, Value>,
  pub received_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerOccurrence {
  pub occurrence_schema_version: u32,
  pub occurrence_id: String,
  pub workflow_id: String,
  pub trigger_id: String,
  pub trigger_handler: String,
  pub definition_hash: String,
  pub source_identity_hash: String,
  pub payload_hash: String,
  pub received_at: DateTime<Utc>,
  pub run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerAdmissionOutcome {
  pub occurrence_id: String,
  pub run_id: String,
  pub duplicate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleCursorRegistration {
  pub workflow_id: String,
  pub trigger_id: String,
  pub definition_hash: String,
  pub cron: String,
  pub timezone: String,
  pub on_missed: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleCursor {
  pub workflow_id: String,
  pub trigger_id: String,
  pub definition_hash: String,
  pub cron: String,
  pub timezone: String,
  pub on_missed: String,
  pub next_scheduled_at: DateTime<Utc>,
  pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleCursorRegistrationOutcome {
  pub cursor: ScheduleCursor,
  pub initialized: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntervalCursorRegistration {
  pub workflow_id: String,
  pub trigger_id: String,
  pub definition_hash: String,
  pub every_ms: u64,
  pub on_missed: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntervalCursor {
  pub workflow_id: String,
  pub trigger_id: String,
  pub definition_hash: String,
  pub every_ms: u64,
  pub on_missed: String,
  pub anchor_at: DateTime<Utc>,
  pub next_sequence: u64,
  pub next_scheduled_at: DateTime<Utc>,
  pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntervalCursorRegistrationOutcome {
  pub cursor: IntervalCursor,
  pub initialized: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerRecoveryWork {
  pub occurrence: TriggerOccurrence,
  pub trigger: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InternalEventAdmissionRequest {
  pub publication_id: String,
  pub parent_run_id: String,
  pub event_name: String,
  pub trigger: TriggerAdmissionRequest,
  pub emitted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InternalEventAdmissionOutcome {
  pub depth: u32,
  pub occurrence: TriggerAdmissionOutcome,
}
