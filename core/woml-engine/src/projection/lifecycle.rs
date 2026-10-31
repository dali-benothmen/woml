//! projection lifecycle.

use crate::event::LifecycleFailure;
use crate::event::LifecycleSubject;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleStatus {
  #[default]
  Idle,
  Running,
  Finalizing,
  CompletedWithWarnings,
  Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleActionStatus {
  Started,
  Succeeded,
  Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleActionProjection {
  pub action_id: String,
  pub attempt: u32,
  pub status: LifecycleActionStatus,
  pub failure: Option<LifecycleFailure>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleHookStatus {
  Requested,
  Running,
  Completed,
  CompletedWithWarnings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleHookProjection {
  pub hook_invocation_id: String,
  pub hook_id: String,
  pub event: crate::model::LifecycleEventName,
  pub subject: LifecycleSubject,
  pub status: LifecycleHookStatus,
  pub actions: BTreeMap<String, LifecycleActionProjection>,
  pub failed_actions: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReusableLifecycleStatus {
  Requested,
  Running,
  Completed,
  CompletedWithWarnings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReusableLifecycleProjection {
  pub invocation_id: String,
  pub definition_digest: String,
  pub hook: crate::ReusableLifecycleHook,
  pub status: ReusableLifecycleStatus,
  pub active_action_id: Option<String>,
  pub completed_action_ids: Vec<String>,
  pub warning_codes: Vec<String>,
}
