//! model approval validation.

use super::{
  issue, literal_string, reusable_provider_ids, valid_custom_provider_delivery,
  valid_discord_snowflake, valid_public_structural_id, valid_secret_name, valid_slack_destination,
  valid_whatsapp_language, valid_whatsapp_phone_number_id, valid_whatsapp_recipient,
  valid_whatsapp_template, CompiledWorkflowDefinition, CompiledWorkflowEdge, CompiledWorkflowNode,
  EdgeCondition, ModelIssue, ModelIssueCode, NotificationDefinition, ValueExpression,
};
use crate::COMPILED_MODEL_SCHEMA_VERSION_V20;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V21;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V4;
use crate::COMPILED_MODEL_SCHEMA_VERSION_V5;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;

fn approval_join_id(value: &str) -> Option<&str> {
  value
    .strip_prefix("__woml_approval__")?
    .strip_suffix("__join")
    .filter(|approval_id| valid_public_structural_id(approval_id))
}

pub(super) fn approval_notifications(
  approval_id: &str,
  fields: &BTreeMap<String, ValueExpression>,
  provider_ids: &HashSet<&str>,
) -> Option<Vec<NotificationDefinition>> {
  let Some(expression) = fields.get("notifications") else {
    return Some(Vec::new());
  };
  let ValueExpression::Array { items } = expression else {
    return None;
  };
  if items.is_empty() {
    return None;
  }
  let mut notifications = Vec::with_capacity(items.len());
  let mut previous_tag = None;
  let mut previous_channel = None;
  let mut duplicate_keys = HashSet::new();
  for item in items {
    let ValueExpression::Object { fields } = item else {
      return None;
    };
    let ValueExpression::Literal { value: delivery } = fields.get("deliveryId")? else {
      return None;
    };
    let ValueExpression::Literal { value: provider } = fields.get("provider")? else {
      return None;
    };
    let ValueExpression::Literal { value: destination } = fields.get("destination")? else {
      return None;
    };
    let ValueExpression::Object {
      fields: credentials,
    } = fields.get("credentials")?
    else {
      return None;
    };
    let delivery_id = delivery.as_str()?;
    let provider = provider.as_str()?;
    let destination = destination.as_str()?;
    let prefix = format!("{approval_id}:notify:");
    let suffix = delivery_id.strip_prefix(&prefix)?;
    let destination_marker = if provider == "telegram" {
      ":chat:"
    } else if provider == "whatsapp" {
      ":recipient:"
    } else {
      ":channel:"
    };
    let (tag, channel) = suffix.split_once(destination_marker)?;
    let tag = tag.parse::<usize>().ok()?;
    let channel = channel.parse::<usize>().ok()?;
    let ordered = match (previous_tag, previous_channel) {
      (None, None) => tag == 0 && channel == 0,
      (Some(previous_tag), Some(previous_channel)) if tag == previous_tag => {
        channel == previous_channel + 1
      }
      (Some(previous_tag), Some(_)) => tag == previous_tag + 1 && channel == 0,
      _ => false,
    };
    if !ordered {
      return None;
    }
    let (names, provider_id, message, template_name, language, duplicate_key) =
      if provider == "slack" {
        if fields.len() != 4
          || !["deliveryId", "provider", "destination", "credentials"]
            .iter()
            .all(|key| fields.contains_key(*key))
          || !valid_slack_destination(destination)
          || credentials.len() != 2
        {
          return None;
        }
        let mut names = BTreeMap::new();
        for key in ["botToken", "appToken"] {
          let ValueExpression::SecretReference { name } = credentials.get(key)? else {
            return None;
          };
          if !valid_secret_name(name) {
            return None;
          }
          names.insert(key.to_string(), name.clone());
        }
        let duplicate_key = format!(
          "slack\0{}\0{}\0{}",
          names["botToken"], names["appToken"], destination
        );
        (names, None, None, None, None, duplicate_key)
      } else if provider == "telegram" {
        if fields.len() != 4
          || !["deliveryId", "provider", "destination", "credentials"]
            .iter()
            .all(|key| fields.contains_key(*key))
          || destination.is_empty()
          || destination.len() > 20
          || destination
            .strip_prefix('-')
            .unwrap_or(destination)
            .bytes()
            .any(|byte| !byte.is_ascii_digit())
          || credentials.len() != 1
        {
          return None;
        }
        let ValueExpression::SecretReference { name } = credentials.get("botToken")? else {
          return None;
        };
        if !valid_secret_name(name) {
          return None;
        }
        let names = BTreeMap::from_iter([("botToken".to_string(), name.clone())]);
        let duplicate_key = format!("telegram\0{}\0{}", names["botToken"], destination);
        (names, None, None, None, None, duplicate_key)
      } else if provider == "discord" {
        if fields.len() != 4
          || !["deliveryId", "provider", "destination", "credentials"]
            .iter()
            .all(|key| fields.contains_key(*key))
          || !valid_discord_snowflake(destination)
          || credentials.len() != 1
        {
          return None;
        }
        let ValueExpression::SecretReference { name } = credentials.get("botToken")? else {
          return None;
        };
        if !valid_secret_name(name) {
          return None;
        }
        let names = BTreeMap::from_iter([("botToken".to_string(), name.clone())]);
        let duplicate_key = format!("discord\0{}\0{}", names["botToken"], destination);
        (names, None, None, None, None, duplicate_key)
      } else if provider == "whatsapp" {
        if fields.len() != 6
          || ![
            "deliveryId",
            "provider",
            "destination",
            "credentials",
            "templateName",
            "language",
          ]
          .iter()
          .all(|key| fields.contains_key(*key))
          || !valid_whatsapp_recipient(destination)
          || credentials.len() != 2
        {
          return None;
        }
        let ValueExpression::SecretReference { name: access_token } =
          credentials.get("accessToken")?
        else {
          return None;
        };
        let phone_number_id = literal_string(credentials.get("phoneNumberId"))?;
        let template_name = literal_string(fields.get("templateName"))?;
        let language = literal_string(fields.get("language"))?;
        if !valid_secret_name(access_token)
          || !valid_whatsapp_phone_number_id(phone_number_id)
          || !valid_whatsapp_template(template_name)
          || !valid_whatsapp_language(language)
        {
          return None;
        }
        let names = BTreeMap::from_iter([
          ("accessToken".to_string(), access_token.clone()),
          ("phoneNumberId".to_string(), phone_number_id.to_string()),
        ]);
        let duplicate_key = format!(
          "whatsapp\0{}\0{}\0{}",
          access_token, phone_number_id, destination
        );
        (
          names,
          None,
          None,
          Some(template_name.to_string()),
          Some(language.to_string()),
          duplicate_key,
        )
      } else if provider == "custom"
        && valid_custom_provider_delivery(fields, "approval", provider_ids, false)
      {
        let provider_id = literal_string(fields.get("providerId"))?.to_string();
        (
          BTreeMap::new(),
          Some(provider_id.clone()),
          fields.get("message").cloned(),
          None,
          None,
          format!("custom\0{provider_id}"),
        )
      } else {
        return None;
      };
    if !duplicate_keys.insert(duplicate_key) {
      return None;
    }
    notifications.push(NotificationDefinition {
      delivery_id: delivery_id.to_string(),
      provider: provider.to_string(),
      destination: destination.to_string(),
      credentials: names,
      provider_id,
      message,
      template_name,
      language,
    });
    previous_tag = Some(tag);
    previous_channel = Some(channel);
  }
  Some(notifications)
}

fn valid_reviewer_role(role: &str) -> bool {
  if role.is_empty() || role.len() > 64 {
    return false;
  }
  let mut segments = role.split(['-', '_', '.']);
  let Some(first) = segments.next() else {
    return false;
  };
  let valid_segment = |segment: &str, require_letter: bool| {
    !segment.is_empty()
      && (!require_letter || segment.as_bytes()[0].is_ascii_lowercase())
      && segment
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
  };
  valid_segment(first, true) && segments.all(|segment| valid_segment(segment, false))
}

pub(super) fn approval_wait_inputs(
  node: Option<&CompiledWorkflowNode>,
  provider_ids: &HashSet<&str>,
  schema_version: u32,
) -> bool {
  let Some(node) = node else {
    return false;
  };
  if node.handler != "engine.approval-wait"
    || node.timeout_ms.is_some()
    || node.retry_policy.is_some()
  {
    return false;
  }
  let ValueExpression::Object { fields } = &node.inputs else {
    return false;
  };
  if !(1..=6).contains(&fields.len()) {
    return false;
  }
  if fields.keys().any(|key| {
    key != "timeoutMs"
      && key != "onTimeout"
      && key != "notifications"
      && key != "review"
      && key != "requiredApprovals"
      && key != "requiredRoles"
  }) {
    return false;
  }
  let valid_timeout = fields.get("timeoutMs").is_none_or(|value| {
    matches!(
      value,
      ValueExpression::Literal { value }
        if value.as_u64().is_some_and(|milliseconds| {
          (1..=9_007_199_254_740_991).contains(&milliseconds)
        })
    )
  });
  let valid_policy = matches!(
    fields.get("onTimeout"),
    Some(ValueExpression::Literal { value })
      if matches!(value.as_str(), Some("reject" | "fail"))
  );
  let valid_metadata = node.metadata.as_ref().is_none_or(|metadata| {
    metadata
      .keys()
      .all(|key| key == "name" || key == "description")
      && metadata
        .values()
        .all(|value| value.as_str().is_some_and(|text| !text.is_empty()))
  });
  let valid_notifications = approval_notifications(&node.id, fields, provider_ids).is_some();
  let rich_count = usize::from(fields.contains_key("review"))
    + usize::from(fields.contains_key("requiredApprovals"))
    + usize::from(fields.contains_key("requiredRoles"));
  let valid_rich_review = rich_count == 0
    || rich_count == 3
      && matches!(
        schema_version,
        COMPILED_MODEL_SCHEMA_VERSION_V20 | COMPILED_MODEL_SCHEMA_VERSION_V21
      )
      && matches!(
        fields.get("review"),
        Some(ValueExpression::ContextReference { path })
          if path.len() >= 2 && path[0] == "steps" && valid_public_structural_id(&path[1])
      )
      && matches!(
        fields.get("requiredApprovals"),
        Some(ValueExpression::Literal { value })
          if value.as_u64().is_some_and(|count| (1..=20).contains(&count))
      )
      && matches!(
        fields.get("requiredRoles"),
        Some(ValueExpression::Array { items })
          if items.len() <= 16
            && items.iter().all(|item| matches!(item, ValueExpression::Literal { value } if value.as_str().is_some_and(valid_reviewer_role)))
            && items.windows(2).all(|pair| {
              let (ValueExpression::Literal { value: left }, ValueExpression::Literal { value: right }) = (&pair[0], &pair[1]) else { return false; };
              left.as_str().zip(right.as_str()).is_some_and(|(left, right)| left < right)
            })
      );
  valid_timeout && valid_policy && valid_metadata && valid_notifications && valid_rich_review
}

fn approval_join_inputs(node: Option<&CompiledWorkflowNode>) -> bool {
  matches!(
    node,
    Some(CompiledWorkflowNode {
      handler,
      inputs: ValueExpression::Object { fields },
      timeout_ms: None,
      retry_policy: None,
      metadata: None,
      ..
    }) if handler == "engine.approval-join" && fields.is_empty()
  )
}

fn is_approval_decision_condition(
  condition: &EdgeCondition,
  approval_id: &str,
  decision: &str,
) -> bool {
  matches!(
    condition,
    EdgeCondition::Equals {
      left: ValueExpression::ContextReference { path },
      right: ValueExpression::Literal { value },
    } if path == &["steps", approval_id, "decision"] && value.as_str() == Some(decision)
  )
}

pub(super) fn inspect_approval_contract(
  workflow: &CompiledWorkflowDefinition,
  issues: &mut Vec<ModelIssue>,
) {
  let provider_ids = reusable_provider_ids(workflow);
  let nodes: HashMap<&str, &CompiledWorkflowNode> = workflow
    .graph
    .nodes
    .iter()
    .map(|node| (node.id.as_str(), node))
    .collect();
  let mut groups: BTreeMap<&str, Vec<&CompiledWorkflowEdge>> = BTreeMap::new();
  let mut rich_approval_count = 0usize;

  for edge in &workflow.graph.edges {
    if let Some(approval_id) = edge.approval_id.as_deref() {
      groups.entry(approval_id).or_default().push(edge);
      if edge.branch_id.is_some() || edge.parallel_id.is_some() {
        issues.push(issue(
          ModelIssueCode::InvalidApprovalGroup,
          format!(
            "Approval edge {:?} cannot also belong to a branch or parallel group.",
            edge.id
          ),
        ));
      }
    } else if matches!(edge.condition, EdgeCondition::Equals { .. }) {
      issues.push(issue(
        ModelIssueCode::InvalidApprovalGroup,
        format!("Equals edge {:?} must carry an approvalId.", edge.id),
      ));
    }
  }

  if workflow.schema_version < COMPILED_MODEL_SCHEMA_VERSION_V4 && !groups.is_empty() {
    issues.push(issue(
      ModelIssueCode::InvalidApprovalGroup,
      "Compiled model v1-v3 cannot contain model-v4 approval groups.",
    ));
  }

  for (approval_id, edges) in &groups {
    let join_id = format!("__woml_approval__{approval_id}__join");
    let wait = nodes.get(*approval_id).copied();
    let join = nodes.get(join_id.as_str()).copied();
    let approved_route_id = format!("{approval_id}:approved");
    let rejected_route_id = format!("{approval_id}:rejected");
    let approved_join_id = format!("{approval_id}:approved:join");
    let rejected_join_id = format!("{approval_id}:rejected:join");
    let approved_route = edges
      .iter()
      .copied()
      .find(|edge| edge.id == approved_route_id);
    let rejected_route = edges
      .iter()
      .copied()
      .find(|edge| edge.id == rejected_route_id);
    let approved_join = edges
      .iter()
      .copied()
      .find(|edge| edge.id == approved_join_id);
    let rejected_join = edges
      .iter()
      .copied()
      .find(|edge| edge.id == rejected_join_id);

    let route_is_valid = |route: Option<&CompiledWorkflowEdge>,
                          join: Option<&CompiledWorkflowEdge>,
                          decision: &str| {
      let Some(route) = route else {
        return false;
      };
      if route.from != **approval_id
        || route.approval_id.as_deref() != Some(*approval_id)
        || !is_approval_decision_condition(&route.condition, approval_id, decision)
      {
        return false;
      }
      if route.to == join_id {
        return join.is_none();
      }
      matches!(
        join,
        Some(join_edge)
          if join_edge.from != **approval_id
            && join_edge.to == join_id
            && join_edge.approval_id.as_deref() == Some(*approval_id)
            && matches!(join_edge.condition, EdgeCondition::Always)
      )
    };
    let routes_are_valid = route_is_valid(approved_route, approved_join, "approved")
      && route_is_valid(rejected_route, rejected_join, "rejected");
    let expected_edges = 2
      + usize::from(approved_route.is_some_and(|edge| edge.to != join_id))
      + usize::from(rejected_route.is_some_and(|edge| edge.to != join_id));
    let wait_outgoing: Vec<_> = workflow
      .graph
      .edges
      .iter()
      .filter(|edge| edge.from.as_str() == *approval_id)
      .collect();
    let join_incoming: Vec<_> = workflow
      .graph
      .edges
      .iter()
      .filter(|edge| edge.to == join_id)
      .collect();
    let boundaries_are_closed = wait_outgoing.len() == 2
      && wait_outgoing
        .iter()
        .all(|edge| edge.approval_id.as_deref() == Some(*approval_id))
      && join_incoming
        .iter()
        .all(|edge| edge.approval_id.as_deref() == Some(*approval_id));
    let has_notifications = wait.is_some_and(|node| {
      matches!(&node.inputs, ValueExpression::Object { fields } if fields.contains_key("notifications"))
    });
    if wait.is_some_and(|node| {
      matches!(&node.inputs, ValueExpression::Object { fields } if fields.contains_key("review"))
    }) {
      rich_approval_count += 1;
    }
    if has_notifications && workflow.schema_version < COMPILED_MODEL_SCHEMA_VERSION_V5 {
      issues.push(issue(
        ModelIssueCode::InvalidNotificationGroup,
        format!("Approval {approval_id:?} notifications require compiled model v5 or later."),
      ));
    }

    if !valid_public_structural_id(approval_id)
      || !approval_wait_inputs(wait, &provider_ids, workflow.schema_version)
      || !approval_join_inputs(join)
      || !routes_are_valid
      || edges.len() != expected_edges
      || !boundaries_are_closed
    {
      issues.push(issue(
        ModelIssueCode::InvalidApprovalGroup,
        format!(
          "Approval group {approval_id:?} does not match the frozen wait, decision-route, empty-arm, and join contract."
        ),
      ));
    }
  }

  if workflow.schema_version == COMPILED_MODEL_SCHEMA_VERSION_V20 && rich_approval_count == 0 {
    issues.push(issue(
      ModelIssueCode::InvalidApprovalGroup,
      "Compiled Model v20 requires at least one complete rich approval descriptor.",
    ));
  }

  for node in &workflow.graph.nodes {
    if node.handler == "engine.approval-wait" && !groups.contains_key(node.id.as_str()) {
      issues.push(issue(
        ModelIssueCode::InvalidApprovalGroup,
        format!("Approval wait {:?} has no matching edge group.", node.id),
      ));
    } else if node.handler == "engine.approval-join"
      && approval_join_id(&node.id).is_none_or(|approval_id| !groups.contains_key(approval_id))
    {
      issues.push(issue(
        ModelIssueCode::InvalidApprovalGroup,
        format!("Approval join {:?} has no matching edge group.", node.id),
      ));
    }
  }
}
