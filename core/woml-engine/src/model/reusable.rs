//! model reusable.

use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum CompiledReusableInvocation {
  Step {
    #[serde(rename = "invocationId")]
    invocation_id: String,
    #[serde(rename = "nodeId")]
    node_id: String,
    alias: String,
    #[serde(rename = "definitionDigest")]
    definition_digest: String,
    source: String,
    #[serde(rename = "scriptArtifactId")]
    script_artifact_id: String,
    props: Vec<CompiledReusableBoundProp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lifecycle: Option<CompiledReusableLifecycle>,
  },
  NotificationProvider {
    #[serde(rename = "providerId")]
    provider_id: String,
    alias: String,
    #[serde(rename = "definitionDigest")]
    definition_digest: String,
    source: String,
    #[serde(rename = "scriptArtifactId")]
    script_artifact_id: String,
    props: Vec<CompiledReusableBoundProp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lifecycle: Option<CompiledReusableLifecycle>,
  },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledReusableBoundProp {
  pub name: String,
  pub binding_name: String,
  pub secret: bool,
  pub expression: CompiledReusablePropExpression,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum CompiledReusablePropExpression {
  Literal { value: String },
  Context { path: String },
  Secret { name: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledReusableLifecycle {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub on_success: Option<Vec<String>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub on_error: Option<Vec<String>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub on_complete: Option<Vec<String>>,
}
