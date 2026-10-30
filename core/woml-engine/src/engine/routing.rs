//! engine routing.

use super::resolve_context_reference;
use crate::model::EdgeCondition;
use crate::model::ValueExpression;
use crate::CompiledWorkflowDefinition;
use crate::WorkflowContext;
use serde_json::Value;

pub(crate) fn selector_branch_id(selector_id: &str) -> Option<&str> {
  selector_id
    .strip_prefix("__woml_branch__")?
    .strip_suffix("__select")
}

pub(crate) fn selected_branch_arm(
  workflow: &CompiledWorkflowDefinition,
  selector_id: &str,
  context: &WorkflowContext,
) -> Result<String, BranchEvaluationError> {
  let branch_id = selector_branch_id(selector_id).ok_or_else(|| BranchEvaluationError {
    branch_id: selector_id.to_string(),
    arm_id: None,
    path: None,
    kind: BranchEvaluationErrorKind::SelectionInvalid,
  })?;
  for edge in workflow
    .graph
    .edges
    .iter()
    .filter(|edge| edge.from == selector_id && edge.branch_id.as_deref() == Some(branch_id))
  {
    match &edge.condition {
      EdgeCondition::Boolean { value: expression } => {
        let condition_path = match expression {
          ValueExpression::ContextReference { path } => Some(path.clone()),
          _ => None,
        };
        let resolved = resolve_context_reference(expression, context).map_err(|error| {
          BranchEvaluationError {
            branch_id: branch_id.to_string(),
            arm_id: Some(edge.id.clone()),
            path: Some(error.path),
            kind: BranchEvaluationErrorKind::ReferenceNotAvailable,
          }
        })?;
        match resolved {
          Value::Bool(true) => return Ok(edge.id.clone()),
          Value::Bool(false) => {}
          value => {
            return Err(BranchEvaluationError {
              branch_id: branch_id.to_string(),
              arm_id: Some(edge.id.clone()),
              path: condition_path,
              kind: BranchEvaluationErrorKind::NotBoolean(json_value_type(&value)),
            });
          }
        }
      }
      EdgeCondition::Always => return Ok(edge.id.clone()),
      _ => {
        return Err(BranchEvaluationError {
          branch_id: branch_id.to_string(),
          arm_id: Some(edge.id.clone()),
          path: None,
          kind: BranchEvaluationErrorKind::SelectionInvalid,
        });
      }
    }
  }
  Err(BranchEvaluationError {
    branch_id: branch_id.to_string(),
    arm_id: None,
    path: None,
    kind: BranchEvaluationErrorKind::SelectionInvalid,
  })
}

pub(crate) fn selected_choice_arm(
  workflow: &CompiledWorkflowDefinition,
  selector_id: &str,
  context: &WorkflowContext,
) -> Result<(String, String), BranchEvaluationError> {
  let choice = workflow
    .graph
    .choices
    .as_deref()
    .unwrap_or_default()
    .iter()
    .find(|choice| choice.selector_node_id == selector_id)
    .ok_or_else(|| BranchEvaluationError {
      branch_id: selector_id.to_string(),
      arm_id: None,
      path: None,
      kind: BranchEvaluationErrorKind::SelectionInvalid,
    })?;
  if let Some(selector) = &choice.string_selector {
    let path = match selector {
      ValueExpression::ContextReference { path } => Some(path.clone()),
      _ => None,
    };
    let value =
      resolve_context_reference(selector, context).map_err(|error| BranchEvaluationError {
        branch_id: choice.choice_id.clone(),
        arm_id: None,
        path: Some(error.path),
        kind: BranchEvaluationErrorKind::ReferenceNotAvailable,
      })?;
    let Value::String(value) = value else {
      return Err(BranchEvaluationError {
        branch_id: choice.choice_id.clone(),
        arm_id: None,
        path,
        kind: BranchEvaluationErrorKind::NotString(json_value_type_name(&value).to_string()),
      });
    };
    let arm_id = choice
      .string_cases
      .as_deref()
      .unwrap_or_default()
      .iter()
      .find(|case| case.value == value)
      .map(|case| case.arm_id.clone())
      .or_else(|| choice.default_arm_id.clone())
      .ok_or_else(|| BranchEvaluationError {
        branch_id: choice.choice_id.clone(),
        arm_id: None,
        path: None,
        kind: BranchEvaluationErrorKind::SelectionInvalid,
      })?;
    return Ok((choice.choice_id.clone(), arm_id));
  }
  for edge in workflow
    .graph
    .edges
    .iter()
    .filter(|edge| edge.from == selector_id && choice.arm_ids.contains(&edge.id))
  {
    match &edge.condition {
      EdgeCondition::Boolean { value: expression } => {
        let condition_path = match expression {
          ValueExpression::ContextReference { path } => Some(path.clone()),
          _ => None,
        };
        let resolved = resolve_context_reference(expression, context).map_err(|error| {
          BranchEvaluationError {
            branch_id: choice.choice_id.clone(),
            arm_id: Some(edge.id.clone()),
            path: Some(error.path),
            kind: BranchEvaluationErrorKind::ReferenceNotAvailable,
          }
        })?;
        match resolved {
          Value::Bool(true) => return Ok((choice.choice_id.clone(), edge.id.clone())),
          Value::Bool(false) => {}
          value => {
            return Err(BranchEvaluationError {
              branch_id: choice.choice_id.clone(),
              arm_id: Some(edge.id.clone()),
              path: condition_path,
              kind: BranchEvaluationErrorKind::NotBoolean(json_value_type(&value)),
            });
          }
        }
      }
      EdgeCondition::Always => return Ok((choice.choice_id.clone(), edge.id.clone())),
      _ => {
        return Err(BranchEvaluationError {
          branch_id: choice.choice_id.clone(),
          arm_id: Some(edge.id.clone()),
          path: None,
          kind: BranchEvaluationErrorKind::SelectionInvalid,
        });
      }
    }
  }
  Err(BranchEvaluationError {
    branch_id: choice.choice_id.clone(),
    arm_id: None,
    path: None,
    kind: BranchEvaluationErrorKind::SelectionInvalid,
  })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BranchEvaluationError {
  pub branch_id: String,
  pub arm_id: Option<String>,
  pub path: Option<Vec<String>>,
  pub kind: BranchEvaluationErrorKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BranchEvaluationErrorKind {
  NotBoolean(crate::JsonValueType),
  NotString(String),
  ReferenceNotAvailable,
  SelectionInvalid,
}

fn json_value_type(value: &Value) -> crate::JsonValueType {
  match value {
    Value::Null => crate::JsonValueType::Null,
    Value::Number(_) => crate::JsonValueType::Number,
    Value::String(_) => crate::JsonValueType::String,
    Value::Array(_) => crate::JsonValueType::Array,
    Value::Object(_) => crate::JsonValueType::Object,
    Value::Bool(_) => unreachable!("boolean values are handled before type classification"),
  }
}

fn json_value_type_name(value: &Value) -> &'static str {
  match value {
    Value::Null => "null",
    Value::Bool(_) => "boolean",
    Value::Number(_) => "number",
    Value::String(_) => "string",
    Value::Array(_) => "array",
    Value::Object(_) => "object",
  }
}
