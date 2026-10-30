//! runtime errors.

use crate::event::ApprovalFailure;
use crate::event::ParallelFailure;
use crate::event::ParallelFailurePolicy;
use crate::AttemptFailure;
use crate::BranchFailure;
use crate::DurableEngineError;
use crate::DurableStoreError;
use crate::EngineError;
use crate::RunEvent;
use crate::ScriptHostClientError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RuntimeExecutionError {
  #[error(transparent)]
  Engine(#[from] EngineError),
  #[error(transparent)]
  DurableEngine(#[from] DurableEngineError),
  #[error(transparent)]
  DurableStore(#[from] DurableStoreError),
  #[error(transparent)]
  Host(#[from] ScriptHostClientError),
  #[error(transparent)]
  RunFailed(Box<FailedRunDetails>),
  #[error(transparent)]
  BranchFailed(Box<FailedBranchDetails>),
  #[error(transparent)]
  ParallelFailed(Box<FailedParallelDetails>),
  #[error(transparent)]
  ApprovalFailed(Box<FailedApprovalDetails>),
  #[error(transparent)]
  NotificationFailed(Box<FailedNotificationDetails>),
  #[error(transparent)]
  RunCancelled(Box<CancelledRunDetails>),
  #[error("workflow execution stalled: {0}")]
  Stalled(String),
  #[error("runtime configuration is invalid: {0}")]
  InvalidConfiguration(String),
}

#[derive(Debug, Error)]
#[error("workflow run {run_id:?} was cancelled [{code}]")]
pub struct CancelledRunDetails {
  pub code: String,
  pub run_id: String,
  pub cancellation_request_id: String,
  pub events: Vec<RunEvent>,
}

#[derive(Debug, Error)]
#[error("workflow execution failed [{code}]: {message}")]
pub struct FailedRunDetails {
  pub code: String,
  pub message: String,
  pub node_id: Option<String>,
  pub attempt: Option<u32>,
  pub max_attempts: Option<u32>,
  pub failure: AttemptFailure,
  pub events: Vec<RunEvent>,
}

#[derive(Debug, Error)]
#[error("workflow branch failed [{code}]: {message}")]
pub struct FailedBranchDetails {
  pub code: String,
  pub message: String,
  pub branch_id: String,
  pub arm_id: Option<String>,
  pub path: Option<Vec<String>>,
  pub site: BranchFailureSite,
  pub failure: BranchFailure,
  pub events: Vec<RunEvent>,
}

#[derive(Debug, Error)]
#[error("workflow parallel group failed [{code}]: {message}")]
pub struct FailedParallelDetails {
  pub code: String,
  pub message: String,
  pub parallel_id: String,
  pub policy: ParallelFailurePolicy,
  pub primary_node_id: String,
  pub failed_node_ids: Vec<String>,
  pub cancelled_node_ids: Vec<String>,
  pub failure: ParallelFailure,
  pub events: Vec<RunEvent>,
}

#[derive(Debug, Error)]
#[error("workflow approval failed [{code}]: {message}")]
pub struct FailedApprovalDetails {
  pub code: String,
  pub message: String,
  pub approval_id: String,
  pub request_id: String,
  pub failure: ApprovalFailure,
  pub events: Vec<RunEvent>,
}

#[derive(Debug, Error)]
#[error("workflow notification failed [{code}]: {message}")]
pub struct FailedNotificationDetails {
  pub code: String,
  pub message: String,
  pub approval_id: String,
  pub request_id: String,
  pub failed_delivery_ids: Vec<String>,
  pub failure: crate::event::NotificationRunFailure,
  pub events: Vec<RunEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchFailureSite {
  Test,
  Result,
  Selection,
}

impl BranchFailureSite {
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::Test => "test",
      Self::Result => "result",
      Self::Selection => "selection",
    }
  }
}
