//! engine context.

use super::ContextReferenceError;
use crate::model::ValueExpression;
use crate::WorkflowContext;
use serde_json::Value;

pub(crate) fn resolve_context_reference(
  expression: &ValueExpression,
  context: &WorkflowContext,
) -> Result<Value, ContextReferenceError> {
  let ValueExpression::ContextReference { path } = expression else {
    return Err(ContextReferenceError { path: Vec::new() });
  };
  let missing = || ContextReferenceError { path: path.clone() };
  let Some(root) = path.first().map(String::as_str) else {
    return Err(missing());
  };
  let (mut value, remaining): (Value, &[String]) = match root {
    "trigger" => (Value::Object(context.trigger.clone()), &path[1..]),
    "steps" => (Value::Object(context.steps.clone()), &path[1..]),
    _ => return Err(missing()),
  };
  for segment in remaining {
    value = match value {
      Value::Object(object) => object.get(segment).cloned().ok_or_else(missing)?,
      _ => return Err(missing()),
    };
  }
  Ok(value)
}
