//! event identities.

use super::{EventValidationError, LifecycleFailure, ProviderMessageIdentity, RunEvent};
use crate::capability::validate_safe_metadata;
use crate::RUN_EVENT_SCHEMA_VERSION_V10;
use crate::RUN_EVENT_SCHEMA_VERSION_V11;
use crate::RUN_EVENT_SCHEMA_VERSION_V15;
use crate::RUN_EVENT_SCHEMA_VERSION_V16;
use crate::RUN_EVENT_SCHEMA_VERSION_V8;
use crate::RUN_EVENT_SCHEMA_VERSION_V9;
use serde_json::Map;
use serde_json::Value;

pub(super) fn valid_id(value: &str) -> bool {
  !value.is_empty() && value.chars().count() <= 256
}

pub(super) fn valid_public_structural_id(value: &str) -> bool {
  let mut characters = value.chars();
  matches!(characters.next(), Some(first) if first.is_ascii_lowercase())
    && characters.all(|character| character.is_ascii_alphanumeric())
    && value.chars().count() <= 256
}

pub(super) fn valid_remote_public_id(value: &str) -> bool {
  let bytes = value.as_bytes();
  !bytes.is_empty()
    && bytes.len() <= 128
    && bytes[0].is_ascii_lowercase()
    && bytes
      .iter()
      .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
    && bytes.last() != Some(&b'-')
    && !bytes.windows(2).any(|pair| pair == b"--")
}

pub(super) fn valid_prefixed_id(value: &str, prefix: &str, minimum: usize) -> bool {
  value.len() >= minimum
    && value.len() <= 256
    && value.strip_prefix(prefix).is_some_and(|suffix| {
      !suffix.is_empty()
        && suffix
          .bytes()
          .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    })
}

pub fn is_notification_delivery_id(value: &str, approval_id: &str) -> bool {
  let Some(suffix) = value.strip_prefix(&format!("{approval_id}:notify:")) else {
    return false;
  };
  let Some((tag, destination)) = suffix
    .split_once(":channel:")
    .or_else(|| suffix.split_once(":chat:"))
  else {
    return false;
  };
  [tag, destination].iter().all(|part| {
    !part.is_empty()
      && part.bytes().all(|byte| byte.is_ascii_digit())
      && (*part == "0" || !part.starts_with('0'))
  })
}

pub(super) fn valid_sha256(value: &str) -> bool {
  value.strip_prefix("sha256:").is_some_and(|digest| {
    digest.len() == 64
      && digest
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
  })
}

pub(super) fn valid_rich_approval_identity(
  approval_id: &str,
  request_id: &str,
  digest: &str,
) -> bool {
  valid_public_structural_id(approval_id)
    && is_approval_request_id(request_id)
    && valid_sha256(digest)
}

pub(super) fn valid_rich_approval_roles(roles: &[String]) -> bool {
  roles.len() <= crate::RICH_APPROVAL_MAX_ROLES
    && roles.windows(2).all(|pair| pair[0] < pair[1])
    && roles.iter().all(|role| {
      if role.is_empty() || role.len() > 64 {
        return false;
      }
      let mut segments = role.split(['-', '_', '.']);
      let Some(first) = segments.next() else {
        return false;
      };
      !first.is_empty()
        && first.as_bytes()[0].is_ascii_lowercase()
        && first
          .bytes()
          .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && segments.all(|segment| {
          !segment.is_empty()
            && segment
              .bytes()
              .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
    })
}

pub(super) fn valid_lifecycle_failure(failure: &LifecycleFailure) -> bool {
  !failure.message.is_empty()
    && failure.message.len() <= 1024
    && failure.code.len() <= 128
    && failure.code.starts_with("WOML_")
    && failure
      .code
      .bytes()
      .all(|byte| byte == b'_' || byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

pub(super) fn valid_lifecycle_action_identity(
  hook_invocation_id: &str,
  action_id: &str,
  attempt: u32,
) -> bool {
  valid_sha256(hook_invocation_id) && valid_id(action_id) && attempt == 1
}

pub(super) fn valid_capability_name(value: &str) -> bool {
  value.len() <= 128
    && value.split(['.', '_', '-']).all(|segment| {
      let mut characters = segment.chars();
      matches!(characters.next(), Some(first) if first.is_ascii_lowercase())
        && characters.all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
    })
}

pub(super) fn valid_signal_name(value: &str) -> bool {
  !value.is_empty()
    && value.len() <= 128
    && value.split('.').all(|segment| {
      let mut characters = segment.chars();
      matches!(characters.next(), Some(first) if first.is_ascii_lowercase())
        && characters.all(|character| {
          character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
    })
}

pub(super) fn valid_lock_key(value: &str) -> bool {
  !value.is_empty()
    && value.len() <= 256
    && value.trim() == value
    && !value.chars().any(char::is_control)
    && !value.split('/').any(str::is_empty)
}

pub(super) fn valid_fencing_token(value: &str) -> bool {
  !value.is_empty()
    && value.len() <= 20
    && value.bytes().all(|byte| byte.is_ascii_digit())
    && (value == "0" || !value.starts_with('0'))
    && value.parse::<u64>().is_ok_and(|token| token > 0)
}

pub(super) fn validate_operation_base(
  event_schema_version: u32,
  node_id: &str,
  attempt_number: u32,
  invocation_id: &str,
  call_id: &str,
  operation_key: &str,
  capability: &str,
  operation: &str,
  metadata: &Map<String, Value>,
) -> Result<(), EventValidationError> {
  if !matches!(
    event_schema_version,
    RUN_EVENT_SCHEMA_VERSION_V8
      | RUN_EVENT_SCHEMA_VERSION_V9
      | RUN_EVENT_SCHEMA_VERSION_V10
      | RUN_EVENT_SCHEMA_VERSION_V11
  ) {
    return Err(EventValidationError::Invalid(
      "Operation events are available only in run-event schema v8 or later.".to_string(),
    ));
  }
  validate_identity(node_id, attempt_number, invocation_id)?;
  if attempt_number > 10
    || !valid_id(call_id)
    || !valid_sha256(operation_key)
    || !valid_capability_name(capability)
    || !valid_capability_name(operation)
  {
    return Err(EventValidationError::Invalid(
      "Operation event contains an invalid correlation or logical operation identity.".to_string(),
    ));
  }
  validate_safe_metadata(metadata).map_err(EventValidationError::Invalid)
}

pub(super) fn valid_trigger_handler(value: &str) -> bool {
  matches!(
    value,
    "trigger.manual"
      | "trigger.webhook"
      | "trigger.slack"
      | "trigger.telegram"
      | "trigger.discord"
      | "trigger.whatsapp"
      | "trigger.schedule"
      | "trigger.interval"
      | "trigger.event"
  )
}

pub(crate) fn valid_provider_message(message: &ProviderMessageIdentity) -> bool {
  match message {
    ProviderMessageIdentity::Slack(message) => {
      valid_prefixed_id(&message.workspace_id, "T", 9)
        && matches!(
          message.channel_id.as_bytes().first(),
          Some(b'C' | b'G' | b'D')
        )
        && message.channel_id.len() >= 9
        && message.channel_id.len() <= 32
        && message.channel_id[1..]
          .bytes()
          .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        && message
          .message_id
          .split_once('.')
          .is_some_and(|(seconds, micros)| {
            seconds.len() >= 10
              && seconds.bytes().all(|byte| byte.is_ascii_digit())
              && micros.len() == 6
              && micros.bytes().all(|byte| byte.is_ascii_digit())
          })
    }
    ProviderMessageIdentity::Communication(message) => {
      matches!(
        message.provider.as_str(),
        "telegram" | "discord" | "whatsapp"
      ) && valid_id(&message.account_id)
        && valid_id(&message.conversation_id)
        && valid_id(&message.message_id)
        && message.thread_id.as_deref().is_none_or(valid_id)
    }
  }
}

pub fn is_approval_request_id(value: &str) -> bool {
  let Some(suffix) = value.strip_prefix("aprreq_") else {
    return false;
  };
  value.len() >= 10
    && value.len() <= 256
    && !suffix.is_empty()
    && suffix
      .bytes()
      .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

pub(super) fn valid_branch_arm_id(branch_id: &str, arm_id: &str) -> bool {
  let Some(suffix) = arm_id.strip_prefix(&format!("{branch_id}:")) else {
    return false;
  };
  if suffix == "otherwise" {
    return true;
  }
  let Some(index) = suffix.strip_prefix("when:") else {
    return false;
  };
  !index.is_empty()
    && index.bytes().all(|byte| byte.is_ascii_digit())
    && (index == "0" || !index.starts_with('0'))
    && arm_id.chars().count() <= 256
}

pub(super) fn valid_ordered_id_lists(first: &[String], second: &[String]) -> bool {
  let mut seen = std::collections::HashSet::new();
  first
    .iter()
    .chain(second)
    .all(|node_id| valid_public_structural_id(node_id) && seen.insert(node_id))
}

pub(super) fn validate_identity(
  node_id: &str,
  attempt: u32,
  invocation_id: &str,
) -> Result<(), EventValidationError> {
  if !valid_id(node_id) || !valid_id(invocation_id) || attempt == 0 {
    return Err(EventValidationError::Invalid(
      "Attempt identity requires valid nodeId, invocationId, and attempt >= 1.".to_string(),
    ));
  }
  Ok(())
}

pub(super) fn validate_for_each_iteration_event(
  event: &RunEvent,
  for_each_id: &str,
) -> Result<(), EventValidationError> {
  let scope = event.iteration.as_ref().ok_or_else(|| {
    EventValidationError::Invalid(
      "Iteration Event v15 records require an explicit iteration scope.".to_string(),
    )
  })?;
  if !matches!(
    event.event_schema_version,
    RUN_EVENT_SCHEMA_VERSION_V15 | RUN_EVENT_SCHEMA_VERSION_V16
  ) || !valid_public_structural_id(for_each_id)
    || scope.for_each_id != for_each_id
    || scope.index >= 10_000
  {
    return Err(EventValidationError::Invalid(
      "Iteration Event v15 identity does not match its payload.".to_string(),
    ));
  }
  Ok(())
}

pub(super) fn validate_notification_identity(
  approval_id: &str,
  request_id: &str,
  delivery_id: &str,
) -> Result<(), EventValidationError> {
  if !valid_public_structural_id(approval_id)
    || !is_approval_request_id(request_id)
    || !is_notification_delivery_id(delivery_id, approval_id)
  {
    return Err(EventValidationError::Invalid(
      "Notification event contains invalid approval, request, or delivery identity.".to_string(),
    ));
  }
  Ok(())
}

pub fn is_definition_hash(value: &str) -> bool {
  let Some(hex) = value.strip_prefix("sha256:") else {
    return false;
  };
  hex.len() == 64
    && hex
      .bytes()
      .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
