//! Choice/branch evaluation failure settlement and primary diagnostics.

use crate::engine::BranchEvaluationError;
use crate::engine::BranchEvaluationErrorKind;
use crate::event::RunFailedData;
use crate::event::RunFailedDataV2;
use crate::runtime::errors::BranchFailureSite;
use crate::runtime::errors::FailedBranchDetails;
use crate::runtime::errors::FailedRunDetails;
use crate::runtime::errors::RuntimeExecutionError;
use crate::runtime::{generated_id, RuntimeDagEngine};
use crate::AttemptFailure;
use crate::AttemptFailureKind;
use crate::BranchFailure;
use crate::RunEventPayload;

pub(in crate::runtime) fn fail_choice<T, E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  error: BranchEvaluationError,
) -> Result<T, RuntimeExecutionError> {
  let BranchEvaluationError {
    branch_id,
    path,
    kind,
    ..
  } = error;
  let reference = path
    .as_ref()
    .map(|path| format!("context.{}", path.join(".")));
  let (code, message) = match kind {
    BranchEvaluationErrorKind::NotString(actual_type) => (
      "WOML_SWITCH_VALUE_INVALID".to_string(),
      format!(
        "Switch value {} must resolve to a JSON string; received {actual_type}.",
        reference.as_deref().unwrap_or("<unknown>")
      ),
    ),
    BranchEvaluationErrorKind::ReferenceNotAvailable => (
      "WOML_REFERENCE_NOT_AVAILABLE".to_string(),
      format!(
        "Switch reference {} is not available.",
        reference.as_deref().unwrap_or("<unknown>")
      ),
    ),
    BranchEvaluationErrorKind::SelectionInvalid => (
      "WOML_SWITCH_SELECTION_INVALID".to_string(),
      format!("Switch {branch_id:?} has no valid selectable case."),
    ),
    BranchEvaluationErrorKind::NotBoolean(_) => (
      "WOML_SWITCH_SELECTION_INVALID".to_string(),
      format!("Switch {branch_id:?} contains an invalid compiled selector."),
    ),
  };
  let node_id = engine
    .workflow()
    .graph
    .choices
    .as_deref()
    .unwrap_or_default()
    .iter()
    .find(|choice| choice.choice_id == branch_id)
    .map(|choice| choice.selector_node_id.clone())
    .unwrap_or(branch_id);
  let failure = AttemptFailure {
    kind: AttemptFailureKind::ServiceFailed,
    code: code.clone(),
    message: message.clone(),
    details: None,
    ..AttemptFailure::legacy_defaults()
  };
  engine.append_payload(
    run_id,
    RunEventPayload::RunFailed(RunFailedData::V2(RunFailedDataV2::Attempt {
      node_id: node_id.clone(),
      attempt: 1,
      invocation_id: generated_id("inv"),
      failure: failure.clone(),
    })),
  )?;
  Err(RuntimeExecutionError::RunFailed(Box::new(
    FailedRunDetails {
      code,
      message,
      node_id: Some(node_id),
      attempt: Some(1),
      max_attempts: Some(1),
      failure,
      events: engine.events(run_id)?,
    },
  )))
}

pub(in crate::runtime) fn fail_branch<T, E: RuntimeDagEngine>(
  engine: &mut E,
  run_id: &str,
  error: BranchEvaluationError,
  site: BranchFailureSite,
) -> Result<T, RuntimeExecutionError> {
  let BranchEvaluationError {
    branch_id,
    arm_id,
    path,
    kind,
  } = error;
  let reference = path
    .as_ref()
    .map(|path| format!("context.{}", path.join(".")));
  let failure = match kind {
    BranchEvaluationErrorKind::NotBoolean(actual_type) => BranchFailure::BranchTestNotBoolean {
      code: "WOML_BRANCH_TEST_NOT_BOOLEAN".to_string(),
      message: format!(
        "Branch test for {} must resolve to a JSON boolean.",
        arm_id.as_deref().unwrap_or(&branch_id)
      ),
      actual_type,
    },
    BranchEvaluationErrorKind::NotString(_) => BranchFailure::BranchSelectionInvalid {
      code: "WOML_BRANCH_SELECTION_INVALID".to_string(),
      message: "A string switch reached the conditional-branch failure path.".to_string(),
    },
    BranchEvaluationErrorKind::ReferenceNotAvailable => BranchFailure::ReferenceNotAvailable {
      code: "WOML_REFERENCE_NOT_AVAILABLE".to_string(),
      message: format!(
        "Reference {} is not available.",
        reference.as_deref().unwrap_or("<unknown>")
      ),
    },
    BranchEvaluationErrorKind::SelectionInvalid => BranchFailure::BranchSelectionInvalid {
      code: "WOML_BRANCH_SELECTION_INVALID".to_string(),
      message: format!("Branch {branch_id:?} has no valid selectable arm."),
    },
  };
  let code = failure.code().to_string();
  let message = match &failure {
    BranchFailure::BranchTestNotBoolean { message, .. }
    | BranchFailure::ReferenceNotAvailable { message, .. }
    | BranchFailure::BranchSelectionInvalid { message, .. } => message.clone(),
  };
  let details_branch_id = branch_id.clone();
  let details_arm_id = arm_id.clone();
  let details_path = path.clone();
  engine.append_payload(
    run_id,
    RunEventPayload::RunFailed(RunFailedData::V2(RunFailedDataV2::Branch {
      branch_id,
      arm_id,
      path,
      failure: failure.clone(),
    })),
  )?;
  Err(RuntimeExecutionError::BranchFailed(Box::new(
    FailedBranchDetails {
      code,
      message,
      branch_id: details_branch_id,
      arm_id: details_arm_id,
      path: details_path,
      site,
      failure,
      events: engine.events(run_id)?,
    },
  )))
}
