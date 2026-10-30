//! model issues.

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ModelIssueCode {
  UnsupportedSchemaVersion,
  InvalidIdentifier,
  InvalidMetadata,
  MissingTrigger,
  DuplicateTriggerId,
  MissingNode,
  MissingEntryNode,
  DuplicateNodeId,
  DuplicateEdgeId,
  DuplicateEntryNodeId,
  UnknownEdgeEndpoint,
  InvalidEntryNode,
  UnreachableNode,
  CyclicGraph,
  TerminalNodeCount,
  UnknownHandler,
  UnsupportedTrigger,
  UnsupportedNodeInputs,
  UnsupportedEdgeCondition,
  UnsupportedBranch,
  UnsupportedRetry,
  UnsupportedTimeout,
  UnsupportedNonSequentialDag,
  InvalidValueExpression,
  InvalidBranchSelector,
  InvalidBranchGroup,
  InvalidBranchResult,
  InvalidParallelGroup,
  UnsupportedParallelExecution,
  InvalidApprovalGroup,
  InvalidNotificationGroup,
  InvalidScriptRuntime,
  InvalidModuleRuntime,
  InvalidLifecycle,
  InvalidRuntimePolicy,
  UnsupportedRuntimePolicyExecution,
  InvalidForkGraph,
  InvalidControlChoice,
  InvalidContextVisibility,
  InvalidWorkflowSettlement,
  UnsupportedForkExecution,
  InvalidReusableDefinition,
  UnsupportedReusableExecution,
  InvalidForEach,
  UnsupportedForEachExecution,
  InvalidDurableWait,
  UnsupportedDurableWaitExecution,
  InvalidLock,
  UnsupportedLockExecution,
  InvalidSaga,
  UnsupportedSagaExecution,
  UnsupportedRichApprovalExecution,
  InvalidExecutionPlacement,
  UnsupportedRemoteExecution,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelIssue {
  pub code: ModelIssueCode,
  pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("compiled workflow model validation failed: {first_message}")]
pub struct ModelValidationError {
  pub issues: Vec<ModelIssue>,
  first_message: String,
}

impl ModelValidationError {
  pub(super) fn new(issues: Vec<ModelIssue>) -> Self {
    let first_message = issues
      .first()
      .map(|issue| issue.message.clone())
      .unwrap_or_else(|| "unknown validation error".to_string());
    Self {
      issues,
      first_message,
    }
  }
}
