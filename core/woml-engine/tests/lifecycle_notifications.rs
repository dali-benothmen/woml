use std::{collections::BTreeMap, path::PathBuf};

use serde_json::{json, Map};
use woml_engine::{
  execute_workflow_durable, CompiledWorkflowDefinition, LifecycleEventName,
  NotificationHostProcessOptions, RunEventPayload, RuntimeExecutionOptions,
  ScriptHostProcessOptions,
};

const HASH: &str = "sha256:5555555555555555555555555555555555555555555555555555555555555555";

fn hosts() -> Option<(ScriptHostProcessOptions, NotificationHostProcessOptions)> {
  let bun = std::process::Command::new("bun")
    .arg("--version")
    .output()
    .ok()?
    .status
    .success()
    .then_some(PathBuf::from("bun"))?;
  let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../woml-cli");
  let script = root.join("src/script-host.ts");
  let notification = root.join("tests/fixtures/informational-notification-provider-host.ts");
  (script.exists() && notification.exists()).then(|| {
    (
      ScriptHostProcessOptions::new(bun.clone(), script),
      NotificationHostProcessOptions::new(bun, notification).with_environment(BTreeMap::from([
        ("WOML_SECRETS_PROVIDER".to_string(), "env".to_string()),
        (
          "WOML_SECRET_SLACK_BOT_TOKEN".to_string(),
          "xoxb-lec5-test".to_string(),
        ),
        (
          "WOML_SECRET_SLACK_APP_TOKEN".to_string(),
          "xapp-lec5-test".to_string(),
        ),
      ])),
    )
  })
}

fn notification_workflow() -> CompiledWorkflowDefinition {
  let mut workflow = CompiledWorkflowDefinition::from_json(include_str!(
    "../../../woml/tests/fixtures/lifecycle/lifecycle.compiled.v11.json"
  ))
  .unwrap();
  let lifecycle = workflow.lifecycle.as_mut().unwrap();
  let mut action = lifecycle
    .hooks
    .iter()
    .find(|hook| hook.event == LifecycleEventName::StepFailure)
    .unwrap()
    .actions[0]
    .clone();
  action.action_id = "lifecycle:run_success:action:0".to_string();
  let woml_engine::model::ValueExpression::Object { fields } = &mut action.inputs else {
    unreachable!()
  };
  let woml_engine::model::ValueExpression::Array { items } = fields.get_mut("deliveries").unwrap()
  else {
    unreachable!()
  };
  for item in items {
    let woml_engine::model::ValueExpression::Object { fields } = item else {
      unreachable!()
    };
    fields.insert(
      "message".to_string(),
      woml_engine::model::ValueExpression::Template {
        parts: vec![
          woml_engine::model::TemplatePart::Text {
            text: "Order ".to_string(),
          },
          woml_engine::model::TemplatePart::ContextReference {
            path: vec!["trigger".to_string(), "orderId".to_string()],
          },
          woml_engine::model::TemplatePart::Text {
            text: " completed in ".to_string(),
          },
          woml_engine::model::TemplatePart::LifecycleReference {
            path: vec!["workflow".to_string(), "id".to_string()],
          },
        ],
      },
    );
  }
  lifecycle
    .hooks
    .retain(|hook| hook.event == LifecycleEventName::RunSuccess);
  lifecycle.hooks[0].actions = vec![action];
  workflow.validate_structure().unwrap();
  workflow
}

#[tokio::test]
async fn informational_slack_deliveries_are_durable_and_do_not_gain_approval_authority() {
  let Some((script_host, notification_host)) = hosts() else {
    return;
  };
  let database = std::env::temp_dir().join(format!(
    "woml-lec5-notification-{}.sqlite",
    uuid::Uuid::new_v4().simple()
  ));
  let execution = execute_workflow_durable(
    notification_workflow(),
    HASH.to_string(),
    Map::from_iter([("orderId".to_string(), json!("order-42"))]),
    RuntimeExecutionOptions::new(script_host, 2_000).with_notification_host(notification_host),
    database.clone(),
  )
  .await
  .unwrap();

  assert_eq!(execution.result, json!({ "done": true }));
  let deliveries = execution
    .events
    .iter()
    .filter_map(|event| match &event.payload {
      RunEventPayload::OperationSucceeded(data)
        if data.capability == "notifications" && data.operation == "deliver" =>
      {
        Some(data)
      }
      _ => None,
    })
    .collect::<Vec<_>>();
  assert_eq!(deliveries.len(), 2, "events: {:#?}", execution.events);
  assert!(deliveries.iter().all(|delivery| {
    delivery.metadata.contains_key("workspaceId")
      && delivery.metadata.contains_key("channelId")
      && delivery.metadata.contains_key("providerMessageId")
  }));
  let serialized = serde_json::to_string(&execution.events).unwrap();
  assert!(!serialized.contains("Order order-42 completed"));
  assert!(!serialized.contains("xoxb-lec5-test"));
  assert!(!serialized.contains("decisionCapability"));
  assert!(!serialized.contains("approvalId"));
  let finalized = execution
    .events
    .iter()
    .find_map(|event| match &event.payload {
      RunEventPayload::RunFinalized(data) => Some(data),
      _ => None,
    })
    .unwrap();
  assert!(finalized.warnings.is_empty());
  let _ = std::fs::remove_file(database);
}

#[tokio::test]
async fn one_failed_channel_is_a_warning_and_does_not_rewrite_business_success() {
  let Some((script_host, mut notification_host)) = hosts() else {
    return;
  };
  notification_host.environment.insert(
    "WOML_FAKE_SLACK_FAILED_DESTINATIONS".to_string(),
    "C0123456789".to_string(),
  );
  let database = std::env::temp_dir().join(format!(
    "woml-lec5-partial-{}.sqlite",
    uuid::Uuid::new_v4().simple()
  ));
  let execution = execute_workflow_durable(
    notification_workflow(),
    HASH.to_string(),
    Map::from_iter([("orderId".to_string(), json!("order-partial"))]),
    RuntimeExecutionOptions::new(script_host, 2_000).with_notification_host(notification_host),
    database.clone(),
  )
  .await
  .unwrap();

  assert_eq!(execution.result, json!({ "done": true }));
  assert_eq!(
    execution
      .events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::OperationSucceeded(_)))
      .count(),
    1
  );
  assert_eq!(
    execution
      .events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::OperationFailed(_)))
      .count(),
    1
  );
  let finalized = execution
    .events
    .iter()
    .find_map(|event| match &event.payload {
      RunEventPayload::RunFinalized(data) => Some(data),
      _ => None,
    })
    .unwrap();
  assert_eq!(finalized.outcome, woml_engine::BusinessOutcome::Succeeded);
  assert_eq!(finalized.warnings.len(), 1);
  assert_eq!(finalized.warnings[0].code, "WOML_SLACK_DESTINATION_INVALID");
  let _ = std::fs::remove_file(database);
}
