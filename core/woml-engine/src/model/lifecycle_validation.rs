//! model lifecycle validation.

use super::{
  inspect_expression, issue, literal_string, object_fields, valid_discord_snowflake, valid_id,
  valid_reusable_alias, valid_secret_name, valid_whatsapp_language, valid_whatsapp_phone_number_id,
  valid_whatsapp_recipient, valid_whatsapp_template, CompiledReusableInvocation,
  CompiledWorkflowDefinition, LifecycleEventName, ModelIssue, ModelIssueCode, TemplatePart,
  ValueExpression,
};
use crate::COMPILED_MODEL_SCHEMA_VERSION_V11;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V12;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V13;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V14;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V15;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V16;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V17;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V18;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V19;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V20;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V21;
use std::collections::BTreeMap;
use std::collections::HashSet;

fn lifecycle_hook_id(event: LifecycleEventName) -> &'static str {
  match event {
    LifecycleEventName::RunStart => "lifecycle:run_start",
    LifecycleEventName::StepStart => "lifecycle:step_start",
    LifecycleEventName::StepSuccess => "lifecycle:step_success",
    LifecycleEventName::StepFailure => "lifecycle:step_failure",
    LifecycleEventName::StepComplete => "lifecycle:step_complete",
    LifecycleEventName::RunSuccess => "lifecycle:run_success",
    LifecycleEventName::RunFailure => "lifecycle:run_failure",
    LifecycleEventName::RunCancel => "lifecycle:run_cancel",
    LifecycleEventName::RunComplete => "lifecycle:run_complete",
  }
}

fn valid_lifecycle_script_inputs(inputs: &ValueExpression) -> bool {
  matches!(
    inputs,
    ValueExpression::Object { fields }
      if fields.len() == 1
        && matches!(fields.get("source"), Some(ValueExpression::Literal { value }) if value.as_str().is_some_and(|source| source.len() <= 1_048_576))
  )
}

pub(super) fn reusable_provider_ids(workflow: &CompiledWorkflowDefinition) -> HashSet<&str> {
  workflow
    .reusable_definitions
    .iter()
    .flatten()
    .filter_map(|definition| match definition {
      CompiledReusableInvocation::NotificationProvider { provider_id, .. } => {
        Some(provider_id.as_str())
      }
      CompiledReusableInvocation::Step { .. } => None,
    })
    .collect()
}

pub(super) fn valid_custom_provider_delivery(
  fields: &BTreeMap<String, ValueExpression>,
  expected_domain: &str,
  provider_ids: &HashSet<&str>,
  message_required: bool,
) -> bool {
  let required_fields = if message_required { 7 } else { 6 };
  if fields.len() < required_fields || fields.len() > 7 {
    return false;
  }
  if fields.keys().any(|key| {
    !matches!(
      key.as_str(),
      "deliveryId"
        | "provider"
        | "destination"
        | "credentials"
        | "providerId"
        | "domain"
        | "message"
    )
  }) {
    return false;
  }
  let delivery_id = literal_string(fields.get("deliveryId"));
  let provider_id = literal_string(fields.get("providerId"));
  let message_valid = fields.get("message").is_none_or(|message| {
    matches!(message, ValueExpression::Template { parts } if (1..=65).contains(&parts.len()) && parts.iter().all(|part| !matches!(part, TemplatePart::Text { text } if text.len() > 4096)))
  });
  literal_string(fields.get("provider")) == Some("custom")
    && literal_string(fields.get("domain")) == Some(expected_domain)
    && delivery_id.is_some_and(|value| valid_id(value) && Some(value) == provider_id)
    && provider_id.is_some_and(|value| provider_ids.contains(value))
    && literal_string(fields.get("destination")).is_some_and(valid_reusable_alias)
    && fields
      .get("credentials")
      .and_then(object_fields)
      .is_some_and(BTreeMap::is_empty)
    && (!message_required || fields.contains_key("message"))
    && message_valid
}

fn valid_lifecycle_notification_inputs(
  inputs: &ValueExpression,
  provider_ids: &HashSet<&str>,
) -> bool {
  let Some(fields) = object_fields(inputs) else {
    return false;
  };
  let Some(ValueExpression::Array { items }) = fields.get("deliveries") else {
    return false;
  };
  fields.len() == 1
    && (1..=256).contains(&items.len())
    && items.iter().all(|delivery| {
      let Some(fields) = object_fields(delivery) else {
        return false;
      };
      if literal_string(fields.get("provider")) == Some("custom") {
        return valid_custom_provider_delivery(fields, "informational", provider_ids, true);
      }
      let credentials = fields.get("credentials").and_then(object_fields);
      let provider = literal_string(fields.get("provider"));
      let built_in_credentials = match provider {
        Some("slack") => credentials.is_some_and(|credentials| {
          credentials.len() == 2
            && matches!(credentials.get("botToken"), Some(ValueExpression::SecretReference { name }) if valid_secret_name(name))
            && matches!(credentials.get("appToken"), Some(ValueExpression::SecretReference { name }) if valid_secret_name(name))
        }),
        Some("telegram") => credentials.is_some_and(|credentials| {
          credentials.len() == 1
            && matches!(credentials.get("botToken"), Some(ValueExpression::SecretReference { name }) if valid_secret_name(name))
        }),
        Some("discord") => credentials.is_some_and(|credentials| {
          credentials.len() == 1
            && matches!(credentials.get("botToken"), Some(ValueExpression::SecretReference { name }) if valid_secret_name(name))
        }),
        Some("whatsapp") => credentials.is_some_and(|credentials| {
          credentials.len() == 2
            && matches!(credentials.get("accessToken"), Some(ValueExpression::SecretReference { name }) if valid_secret_name(name))
            && literal_string(credentials.get("phoneNumberId")).is_some_and(valid_whatsapp_phone_number_id)
        }),
        _ => false,
      };
      let whatsapp = provider == Some("whatsapp");
      fields.len() == if whatsapp { 7 } else { 5 }
        && matches!(fields.get("deliveryId"), Some(ValueExpression::Literal { value }) if value.as_str().is_some_and(|value| !value.is_empty() && value.len() <= 512))
        && matches!(provider, Some("slack" | "telegram" | "discord" | "whatsapp"))
        && matches!(fields.get("destination"), Some(ValueExpression::Literal { value }) if value.as_str().is_some_and(|value| {
          if provider == Some("discord") {
            valid_discord_snowflake(value)
          } else if provider == Some("whatsapp") {
            valid_whatsapp_recipient(value)
          } else {
            !value.is_empty() && value.len() <= 512
          }
        }))
        && built_in_credentials
        && (!whatsapp
          || literal_string(fields.get("templateName")).is_some_and(valid_whatsapp_template))
        && (!whatsapp
          || literal_string(fields.get("language")).is_some_and(valid_whatsapp_language))
        && matches!(fields.get("message"), Some(ValueExpression::Template { parts }) if (1..=65).contains(&parts.len()) && parts.iter().all(|part| !matches!(part, TemplatePart::Text { text } if text.len() > 4096)))
    })
}

pub(super) fn inspect_lifecycle_contract(
  workflow: &CompiledWorkflowDefinition,
  issues: &mut Vec<ModelIssue>,
) {
  let Some(lifecycle) = &workflow.lifecycle else {
    return;
  };
  if !matches!(
    workflow.schema_version,
    COMPILED_MODEL_SCHEMA_VERSION_V11
      | COMPILED_MODEL_SCHEMA_VERSION_V12
      | COMPILED_MODEL_SCHEMA_VERSION_V13
      | COMPILED_MODEL_SCHEMA_VERSION_V14
      | COMPILED_MODEL_SCHEMA_VERSION_V15
      | COMPILED_MODEL_SCHEMA_VERSION_V16
      | COMPILED_MODEL_SCHEMA_VERSION_V17
      | COMPILED_MODEL_SCHEMA_VERSION_V18
      | COMPILED_MODEL_SCHEMA_VERSION_V19
      | COMPILED_MODEL_SCHEMA_VERSION_V20
      | COMPILED_MODEL_SCHEMA_VERSION_V21
  ) {
    issues.push(issue(
      ModelIssueCode::InvalidLifecycle,
      "lifecycle is available only in compiled Model v11 or later.",
    ));
    return;
  }
  if lifecycle.profile_version != 1 || lifecycle.hooks.is_empty() || lifecycle.hooks.len() > 9 {
    issues.push(issue(
      ModelIssueCode::InvalidLifecycle,
      "Model v11 lifecycle must use profileVersion 1 and contain 1 to 9 hooks.",
    ));
    return;
  }
  let node_ids = workflow
    .graph
    .nodes
    .iter()
    .chain(
      workflow
        .graph
        .for_each
        .iter()
        .flatten()
        .flat_map(|descriptor| descriptor.body.nodes.iter()),
    )
    .filter(|node| node.handler == "runtime.script")
    .map(|node| node.id.as_str())
    .collect::<HashSet<_>>();
  let mut events = HashSet::new();
  let mut action_ids = HashSet::new();
  let provider_ids = reusable_provider_ids(workflow);
  for hook in &lifecycle.hooks {
    let valid_steps = match (&hook.step_ids, hook.event.is_step()) {
      (None, true) => true,
      (Some(step_ids), true) => {
        !step_ids.is_empty()
          && step_ids.iter().all(|id| node_ids.contains(id.as_str()))
          && step_ids.iter().collect::<HashSet<_>>().len() == step_ids.len()
      }
      (None, false) => true,
      (Some(_), false) => false,
    };
    if hook.hook_id != lifecycle_hook_id(hook.event)
      || !events.insert(hook.event)
      || !valid_steps
      || hook.actions.is_empty()
      || hook.actions.len() > 64
    {
      issues.push(issue(
        ModelIssueCode::InvalidLifecycle,
        format!(
          "Lifecycle hook {:?} does not match the frozen Lifecycle Binding v1 contract.",
          hook.hook_id
        ),
      ));
      continue;
    }
    for (index, action) in hook.actions.iter().enumerate() {
      let expected_id = format!("{}:action:{index}", hook.hook_id);
      let valid_handler = match (action.handler.as_str(), &action.script_runtime) {
        ("runtime.lifecycle-script", Some(runtime)) => {
          runtime.binding_version == 2
            && runtime.bindings == ["context", "lifecycle", "attempt", "services", "secrets"]
            && runtime.required_secrets.len() <= 64
            && runtime
              .required_secrets
              .windows(2)
              .all(|pair| pair[0] < pair[1])
            && runtime
              .required_secrets
              .iter()
              .all(|name| valid_secret_name(name))
            && valid_lifecycle_script_inputs(&action.inputs)
        }
        ("notification.informational", None) => {
          valid_lifecycle_notification_inputs(&action.inputs, &provider_ids)
        }
        _ => false,
      };
      if action.action_id != expected_id
        || !action_ids.insert(action.action_id.as_str())
        || !valid_handler
      {
        issues.push(issue(
          ModelIssueCode::InvalidLifecycle,
          format!(
            "Lifecycle action {:?} does not match the frozen Lifecycle Binding v1 contract.",
            action.action_id
          ),
        ));
      }
      inspect_expression(
        &action.inputs,
        &format!("lifecycle[{}].action[{index}].inputs", hook.hook_id),
        issues,
      );
    }
  }
}
