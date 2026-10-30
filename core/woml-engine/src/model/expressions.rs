//! model expressions.

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ValueExpression {
  #[serde(rename = "literal")]
  Literal { value: Value },
  #[serde(rename = "contextReference")]
  ContextReference { path: Vec<String> },
  #[serde(rename = "lifecycleReference")]
  LifecycleReference { path: Vec<String> },
  #[serde(rename = "secretReference")]
  SecretReference { name: String },
  #[serde(rename = "object")]
  Object {
    fields: BTreeMap<String, ValueExpression>,
  },
  #[serde(rename = "array")]
  Array { items: Vec<ValueExpression> },
  #[serde(rename = "template")]
  Template { parts: Vec<TemplatePart> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum TemplatePart {
  #[serde(rename = "text")]
  Text { text: String },
  #[serde(rename = "contextReference")]
  ContextReference { path: Vec<String> },
  #[serde(rename = "lifecycleReference")]
  LifecycleReference { path: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum EdgeCondition {
  #[serde(rename = "always")]
  Always,
  #[serde(rename = "boolean")]
  Boolean { value: ValueExpression },
  #[serde(rename = "truthy")]
  Truthy { value: ValueExpression },
  #[serde(rename = "equals")]
  Equals {
    left: ValueExpression,
    right: ValueExpression,
  },
}
