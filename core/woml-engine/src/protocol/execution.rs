//! protocol execution.

use super::{
  CorrectionBindingRef, LifecycleBindingV1, ReusableLifecycleBindingV1, ReusableScriptBindingV3,
  RuntimeModuleBinding, ScriptAttempt, ScriptBindings, ScriptExecutionContext, ScriptExecutionMode,
  SCRIPT_HOST_PROTOCOL, SCRIPT_HOST_PROTOCOL_VERSION,
};
use crate::WorkflowContext;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteMessage<'a> {
  pub protocol: &'static str,
  pub protocol_version: u32,
  pub message_type: &'static str,
  pub invocation_id: &'a str,
  pub run_id: &'a str,
  pub node_id: &'a str,
  pub attempt: ScriptAttempt<'a>,
  pub mode: ScriptExecutionMode,
  pub handler: &'static str,
  pub timeout_ms: u64,
  pub source: &'a str,
  pub context: ScriptExecutionContext<'a>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub lifecycle: Option<&'a LifecycleBindingV1>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub reusable: Option<&'a ReusableScriptBindingV3>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub reusable_lifecycle: Option<&'a ReusableLifecycleBindingV1>,
  pub bindings: ScriptBindings<'a>,
  pub modules: &'a [RuntimeModuleBinding],
}

impl<'a> ExecuteMessage<'a> {
  pub fn runtime_script(
    invocation_id: &'a str,
    run_id: &'a str,
    node_id: &'a str,
    attempt: ScriptAttempt<'a>,
    timeout_ms: u64,
    source: &'a str,
    context: &'a WorkflowContext,
  ) -> Self {
    static EMPTY_SECRETS: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    Self::runtime_script_with_secrets(
      invocation_id,
      run_id,
      node_id,
      attempt,
      timeout_ms,
      source,
      context,
      EMPTY_SECRETS.get_or_init(BTreeMap::new),
    )
  }

  #[allow(clippy::too_many_arguments)]
  pub fn runtime_script_with_secrets(
    invocation_id: &'a str,
    run_id: &'a str,
    node_id: &'a str,
    attempt: ScriptAttempt<'a>,
    timeout_ms: u64,
    source: &'a str,
    context: &'a WorkflowContext,
    secrets: &'a BTreeMap<String, String>,
  ) -> Self {
    Self::runtime_script_with_modules(
      invocation_id,
      run_id,
      node_id,
      attempt,
      timeout_ms,
      source,
      context,
      secrets,
      &[],
    )
  }

  #[allow(clippy::too_many_arguments)]
  pub fn runtime_script_with_modules(
    invocation_id: &'a str,
    run_id: &'a str,
    node_id: &'a str,
    attempt: ScriptAttempt<'a>,
    timeout_ms: u64,
    source: &'a str,
    context: &'a WorkflowContext,
    secrets: &'a BTreeMap<String, String>,
    modules: &'a [RuntimeModuleBinding],
  ) -> Self {
    Self {
      protocol: SCRIPT_HOST_PROTOCOL,
      protocol_version: SCRIPT_HOST_PROTOCOL_VERSION,
      message_type: "execute",
      invocation_id,
      run_id,
      node_id,
      attempt,
      mode: ScriptExecutionMode::Step,
      handler: "runtime.script",
      timeout_ms,
      source,
      context: ScriptExecutionContext::root(context),
      lifecycle: None,
      reusable: None,
      reusable_lifecycle: None,
      bindings: ScriptBindings {
        binding_version: 1,
        services_version: 1,
        secrets,
      },
      modules,
    }
  }

  #[allow(clippy::too_many_arguments)]
  pub fn corrective_script_with_modules(
    invocation_id: &'a str,
    run_id: &'a str,
    action_id: &'a str,
    attempt: ScriptAttempt<'a>,
    timeout_ms: u64,
    source: &'a str,
    context: &'a WorkflowContext,
    correction: CorrectionBindingRef<'a>,
    secrets: &'a BTreeMap<String, String>,
    modules: &'a [RuntimeModuleBinding],
  ) -> Self {
    Self {
      protocol: SCRIPT_HOST_PROTOCOL,
      protocol_version: SCRIPT_HOST_PROTOCOL_VERSION,
      message_type: "execute",
      invocation_id,
      run_id,
      node_id: action_id,
      attempt,
      mode: ScriptExecutionMode::Step,
      handler: "runtime.script",
      timeout_ms,
      source,
      context: ScriptExecutionContext::correction(context, correction),
      lifecycle: None,
      reusable: None,
      reusable_lifecycle: None,
      bindings: ScriptBindings {
        binding_version: 1,
        services_version: 1,
        secrets,
      },
      modules,
    }
  }

  #[allow(clippy::too_many_arguments)]
  pub fn lifecycle_script_with_modules(
    invocation_id: &'a str,
    run_id: &'a str,
    action_id: &'a str,
    attempt: ScriptAttempt<'a>,
    timeout_ms: u64,
    source: &'a str,
    context: &'a WorkflowContext,
    lifecycle: &'a LifecycleBindingV1,
    secrets: &'a BTreeMap<String, String>,
    modules: &'a [RuntimeModuleBinding],
  ) -> Self {
    Self {
      protocol: SCRIPT_HOST_PROTOCOL,
      protocol_version: SCRIPT_HOST_PROTOCOL_VERSION,
      message_type: "execute",
      invocation_id,
      run_id,
      node_id: action_id,
      attempt,
      mode: ScriptExecutionMode::Lifecycle,
      handler: "runtime.lifecycle-script",
      timeout_ms,
      source,
      context: ScriptExecutionContext::root(context),
      lifecycle: Some(lifecycle),
      reusable: None,
      reusable_lifecycle: None,
      bindings: ScriptBindings {
        binding_version: 1,
        services_version: 1,
        secrets,
      },
      modules,
    }
  }

  #[allow(clippy::too_many_arguments)]
  pub fn lifecycle_script_with_iteration_modules(
    invocation_id: &'a str,
    run_id: &'a str,
    action_id: &'a str,
    attempt: ScriptAttempt<'a>,
    timeout_ms: u64,
    source: &'a str,
    context: &'a WorkflowContext,
    item: &'a Value,
    index: u32,
    total: u32,
    lifecycle: &'a LifecycleBindingV1,
    secrets: &'a BTreeMap<String, String>,
    modules: &'a [RuntimeModuleBinding],
  ) -> Result<Self, String> {
    if total == 0 || total > 10_000 || index >= total {
      return Err("Script iteration bindings require index < total <= 10000.".to_string());
    }
    Ok(Self {
      protocol: SCRIPT_HOST_PROTOCOL,
      protocol_version: SCRIPT_HOST_PROTOCOL_VERSION,
      message_type: "execute",
      invocation_id,
      run_id,
      node_id: action_id,
      attempt,
      mode: ScriptExecutionMode::Lifecycle,
      handler: "runtime.lifecycle-script",
      timeout_ms,
      source,
      context: ScriptExecutionContext::iteration(context, item, index, total),
      lifecycle: Some(lifecycle),
      reusable: None,
      reusable_lifecycle: None,
      bindings: ScriptBindings {
        binding_version: 1,
        services_version: 1,
        secrets,
      },
      modules,
    })
  }

  #[allow(clippy::too_many_arguments)]
  pub fn runtime_script_with_iteration(
    invocation_id: &'a str,
    run_id: &'a str,
    node_id: &'a str,
    attempt: ScriptAttempt<'a>,
    timeout_ms: u64,
    source: &'a str,
    context: &'a WorkflowContext,
    item: &'a Value,
    index: u32,
    total: u32,
    secrets: &'a BTreeMap<String, String>,
  ) -> Result<Self, String> {
    Self::runtime_script_with_iteration_modules(
      invocation_id,
      run_id,
      node_id,
      attempt,
      timeout_ms,
      source,
      context,
      item,
      index,
      total,
      secrets,
      &[],
    )
  }

  #[allow(clippy::too_many_arguments)]
  pub fn runtime_script_with_iteration_modules(
    invocation_id: &'a str,
    run_id: &'a str,
    node_id: &'a str,
    attempt: ScriptAttempt<'a>,
    timeout_ms: u64,
    source: &'a str,
    context: &'a WorkflowContext,
    item: &'a Value,
    index: u32,
    total: u32,
    secrets: &'a BTreeMap<String, String>,
    modules: &'a [RuntimeModuleBinding],
  ) -> Result<Self, String> {
    Self::script_with_iteration_modules_and_reusable(
      invocation_id,
      run_id,
      node_id,
      attempt,
      timeout_ms,
      source,
      context,
      item,
      index,
      total,
      secrets,
      modules,
      None,
    )
  }

  #[allow(clippy::too_many_arguments)]
  pub fn reusable_script_with_iteration_modules(
    invocation_id: &'a str,
    run_id: &'a str,
    node_id: &'a str,
    attempt: ScriptAttempt<'a>,
    timeout_ms: u64,
    source: &'a str,
    context: &'a WorkflowContext,
    item: &'a Value,
    index: u32,
    total: u32,
    secrets: &'a BTreeMap<String, String>,
    modules: &'a [RuntimeModuleBinding],
    reusable: &'a ReusableScriptBindingV3,
  ) -> Result<Self, String> {
    Self::script_with_iteration_modules_and_reusable(
      invocation_id,
      run_id,
      node_id,
      attempt,
      timeout_ms,
      source,
      context,
      item,
      index,
      total,
      secrets,
      modules,
      Some(reusable),
    )
  }

  #[allow(clippy::too_many_arguments)]
  fn script_with_iteration_modules_and_reusable(
    invocation_id: &'a str,
    run_id: &'a str,
    node_id: &'a str,
    attempt: ScriptAttempt<'a>,
    timeout_ms: u64,
    source: &'a str,
    context: &'a WorkflowContext,
    item: &'a Value,
    index: u32,
    total: u32,
    secrets: &'a BTreeMap<String, String>,
    modules: &'a [RuntimeModuleBinding],
    reusable: Option<&'a ReusableScriptBindingV3>,
  ) -> Result<Self, String> {
    if total == 0 || total > 10_000 || index >= total {
      return Err("Script iteration bindings require index < total <= 10000.".to_string());
    }
    Ok(Self {
      protocol: SCRIPT_HOST_PROTOCOL,
      protocol_version: SCRIPT_HOST_PROTOCOL_VERSION,
      message_type: "execute",
      invocation_id,
      run_id,
      node_id,
      attempt,
      mode: ScriptExecutionMode::Step,
      handler: "runtime.script",
      timeout_ms,
      source,
      context: ScriptExecutionContext::iteration(context, item, index, total),
      lifecycle: None,
      reusable,
      reusable_lifecycle: None,
      bindings: ScriptBindings {
        binding_version: 1,
        services_version: 1,
        secrets,
      },
      modules,
    })
  }

  #[allow(clippy::too_many_arguments)]
  pub fn reusable_script_with_modules(
    invocation_id: &'a str,
    run_id: &'a str,
    node_id: &'a str,
    attempt: ScriptAttempt<'a>,
    timeout_ms: u64,
    source: &'a str,
    context: &'a WorkflowContext,
    secrets: &'a BTreeMap<String, String>,
    modules: &'a [RuntimeModuleBinding],
    reusable: &'a ReusableScriptBindingV3,
  ) -> Self {
    Self {
      protocol: SCRIPT_HOST_PROTOCOL,
      protocol_version: SCRIPT_HOST_PROTOCOL_VERSION,
      message_type: "execute",
      invocation_id,
      run_id,
      node_id,
      attempt,
      mode: ScriptExecutionMode::Step,
      handler: "runtime.script",
      timeout_ms,
      source,
      context: ScriptExecutionContext::root(context),
      lifecycle: None,
      reusable: Some(reusable),
      reusable_lifecycle: None,
      bindings: ScriptBindings {
        binding_version: 1,
        services_version: 1,
        secrets,
      },
      modules,
    }
  }

  #[allow(clippy::too_many_arguments)]
  pub fn reusable_lifecycle_script_with_modules(
    invocation_id: &'a str,
    run_id: &'a str,
    action_id: &'a str,
    attempt: ScriptAttempt<'a>,
    timeout_ms: u64,
    source: &'a str,
    context: &'a WorkflowContext,
    secrets: &'a BTreeMap<String, String>,
    modules: &'a [RuntimeModuleBinding],
    reusable: &'a ReusableScriptBindingV3,
    reusable_lifecycle: &'a ReusableLifecycleBindingV1,
  ) -> Self {
    Self {
      protocol: SCRIPT_HOST_PROTOCOL,
      protocol_version: SCRIPT_HOST_PROTOCOL_VERSION,
      message_type: "execute",
      invocation_id,
      run_id,
      node_id: action_id,
      attempt,
      mode: ScriptExecutionMode::Lifecycle,
      handler: "runtime.lifecycle-script",
      timeout_ms,
      source,
      context: ScriptExecutionContext::root(context),
      lifecycle: None,
      reusable: Some(reusable),
      reusable_lifecycle: Some(reusable_lifecycle),
      bindings: ScriptBindings {
        binding_version: 1,
        services_version: 1,
        secrets,
      },
      modules,
    }
  }

  #[allow(clippy::too_many_arguments)]
  pub fn reusable_lifecycle_script_with_iteration_modules(
    invocation_id: &'a str,
    run_id: &'a str,
    action_id: &'a str,
    attempt: ScriptAttempt<'a>,
    timeout_ms: u64,
    source: &'a str,
    context: &'a WorkflowContext,
    item: &'a Value,
    index: u32,
    total: u32,
    secrets: &'a BTreeMap<String, String>,
    modules: &'a [RuntimeModuleBinding],
    reusable: &'a ReusableScriptBindingV3,
    reusable_lifecycle: &'a ReusableLifecycleBindingV1,
  ) -> Result<Self, String> {
    if total == 0 || total > 10_000 || index >= total {
      return Err("Script iteration bindings require index < total <= 10000.".to_string());
    }
    Ok(Self {
      protocol: SCRIPT_HOST_PROTOCOL,
      protocol_version: SCRIPT_HOST_PROTOCOL_VERSION,
      message_type: "execute",
      invocation_id,
      run_id,
      node_id: action_id,
      attempt,
      mode: ScriptExecutionMode::Lifecycle,
      handler: "runtime.lifecycle-script",
      timeout_ms,
      source,
      context: ScriptExecutionContext::iteration(context, item, index, total),
      lifecycle: None,
      reusable: Some(reusable),
      reusable_lifecycle: Some(reusable_lifecycle),
      bindings: ScriptBindings {
        binding_version: 1,
        services_version: 1,
        secrets,
      },
      modules,
    })
  }
}
