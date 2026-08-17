use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};

use chrono::{Duration, Utc};
use serde_json::{json, Map, Value};
use uuid::Uuid;
use woml_engine::{
  ApprovalDecision, ApprovalDecisionOutcomeStatus, ApprovalRequestStatus, ApprovalRequestedData,
  ApprovalResolution, ApprovalTimeoutPolicy, CommunicationProviderMessageIdentity,
  CompiledWorkflowDefinition, DurableDagEngine, DurableEventStore, DurableStoreError,
  NotificationDeliveryWork, NotificationMessageUpdateStatus, NotificationProviderAdapter,
  NotificationProviderDeliveryResult, NotificationProviderUpdateResult, NotificationSafeFailure,
  NotificationUpdateWork, ProviderMessageIdentity, RunStatus, SlackProviderMessageIdentity,
};

const BASE_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/approval-slack.compiled.v5.json");
const MODEL_HASH: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const APPROVAL_ID: &str = "releaseApproval";
const REQUEST_ID: &str = "aprreq_cross_provider";
const SLACK_DELIVERY: &str = "releaseApproval:notify:0:channel:0";
const TELEGRAM_DELIVERY: &str = "releaseApproval:notify:1:chat:0";

struct TemporaryDatabase(PathBuf);

impl TemporaryDatabase {
  fn new() -> Self {
    Self(std::env::temp_dir().join(format!(
      "woml-cross-provider-{}.sqlite",
      Uuid::new_v4().simple()
    )))
  }

  fn path(&self) -> &Path {
    &self.0
  }
}

impl Drop for TemporaryDatabase {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.0);
    let _ = std::fs::remove_file(self.0.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(self.0.with_extension("sqlite-shm"));
  }
}

fn mixed_model() -> CompiledWorkflowDefinition {
  let mut value: Value = serde_json::from_str(BASE_MODEL).unwrap();
  let wait = value["graph"]["nodes"]
    .as_array_mut()
    .unwrap()
    .iter_mut()
    .find(|node| node["handler"] == "engine.approval-wait")
    .unwrap();
  let second = &mut wait["inputs"]["fields"]["notifications"]["items"]
    .as_array_mut()
    .unwrap()[1];
  second["fields"]["deliveryId"]["value"] = json!(TELEGRAM_DELIVERY);
  second["fields"]["provider"]["value"] = json!("telegram");
  second["fields"]["destination"]["value"] = json!("-1001234567890");
  second["fields"]["credentials"] = json!({
    "kind": "object",
    "fields": {
      "botToken": {
        "kind": "secretReference",
        "name": "TELEGRAM_BOT_TOKEN"
      }
    }
  });
  let model: CompiledWorkflowDefinition = serde_json::from_value(value).unwrap();
  model.validate_for_durable_execution().unwrap();
  model
}

fn waiting_store(path: &Path, run_id: &str) -> (String, String) {
  let now = Utc::now();
  let store = DurableEventStore::open(path).unwrap();
  let mut engine = DurableDagEngine::new(mixed_model(), MODEL_HASH, store).unwrap();
  engine
    .start_run(format!("evt_{run_id}_started"), run_id, now, Map::new())
    .unwrap();
  engine
    .request_approval(
      run_id,
      now,
      ApprovalRequestedData {
        approval_id: APPROVAL_ID.to_string(),
        request_id: REQUEST_ID.to_string(),
        expires_at: Some(now + Duration::hours(24)),
        on_timeout: ApprovalTimeoutPolicy::Reject,
      },
    )
    .unwrap();
  let mut store = engine.into_store();

  let slack = store
    .begin_notification_delivery(run_id, SLACK_DELIVERY, now)
    .unwrap();
  let slack_capability = slack.decision_capability.clone();
  store
    .complete_notification_delivery(
      &slack,
      NotificationProviderDeliveryResult::Succeeded(ProviderMessageIdentity::Slack(
        SlackProviderMessageIdentity {
          workspace_id: "T12345678".to_string(),
          channel_id: "C12345678".to_string(),
          message_id: "1723024800.000001".to_string(),
        },
      )),
      now,
    )
    .unwrap();

  let telegram = store
    .begin_notification_delivery(run_id, TELEGRAM_DELIVERY, now)
    .unwrap();
  let telegram_capability = telegram.decision_capability.clone();
  store
    .complete_notification_delivery(
      &telegram,
      NotificationProviderDeliveryResult::Succeeded(ProviderMessageIdentity::Communication(
        CommunicationProviderMessageIdentity {
          provider: "telegram".to_string(),
          account_id: "bot123".to_string(),
          conversation_id: "chat123".to_string(),
          message_id: "message123".to_string(),
          thread_id: None,
        },
      )),
      now,
    )
    .unwrap();
  (slack_capability, telegram_capability)
}

#[derive(Default)]
struct MixedUpdateAdapter;

impl NotificationProviderAdapter for MixedUpdateAdapter {
  fn deliver(&mut self, _work: &NotificationDeliveryWork) -> NotificationProviderDeliveryResult {
    panic!("deliveries are prepared by the convergence fixture")
  }

  fn update(&mut self, work: &NotificationUpdateWork) -> NotificationProviderUpdateResult {
    if work.provider == "telegram" {
      NotificationProviderUpdateResult::Failed(NotificationSafeFailure {
        kind: "update_failed".to_string(),
        code: "WOML_TELEGRAM_UPDATE_FAILED".to_string(),
        message: "The delivered Telegram message could not be updated.".to_string(),
        retryable: false,
        retry_after_ms: None,
      })
    } else {
      NotificationProviderUpdateResult::Succeeded
    }
  }
}

#[test]
fn simultaneous_provider_decisions_converge_and_update_failures_do_not_change_the_winner() {
  let database = TemporaryDatabase::new();
  let run_id = "run_cross_provider";
  let (slack_capability, telegram_capability) = waiting_store(database.path(), run_id);
  let barrier = Arc::new(Barrier::new(2));

  let first_path = database.path().to_path_buf();
  let first_barrier = Arc::clone(&barrier);
  let first = std::thread::spawn(move || {
    let mut store = DurableEventStore::open(first_path).unwrap();
    first_barrier.wait();
    store.resolve_notification_approval_from_provider(
      &slack_capability,
      SLACK_DELIVERY,
      "slack",
      "U12345678",
      ApprovalDecision::Approved,
      Utc::now(),
    )
  });

  let second_path = database.path().to_path_buf();
  let second_barrier = Arc::clone(&barrier);
  let second = std::thread::spawn(move || {
    let mut store = DurableEventStore::open(second_path).unwrap();
    second_barrier.wait();
    store.resolve_notification_approval_from_provider(
      &telegram_capability,
      TELEGRAM_DELIVERY,
      "telegram",
      "telegram:123456789",
      ApprovalDecision::Rejected,
      Utc::now(),
    )
  });

  let outcomes = [first.join().unwrap(), second.join().unwrap()];
  assert_eq!(
    outcomes
      .iter()
      .filter(|outcome| matches!(outcome, Ok(value) if value.status == ApprovalDecisionOutcomeStatus::Accepted))
      .count(),
    1
  );
  assert_eq!(
    outcomes
      .iter()
      .filter(|outcome| matches!(outcome, Err(DurableStoreError::ApprovalDecisionConflict)))
      .count(),
    1
  );

  let accepted_decision = outcomes
    .iter()
    .find_map(|outcome| outcome.as_ref().ok().map(|value| value.decision))
    .unwrap();
  let mut store = DurableEventStore::open(database.path()).unwrap();
  let decided = store.projection(run_id).unwrap();
  assert_eq!(decided.status, RunStatus::Running);
  assert_eq!(decided.notification_decisions.len(), 1);
  assert_eq!(decided.notification_updates.len(), 2);
  assert!(matches!(
    &decided.approval_requests[APPROVAL_ID].status,
    ApprovalRequestStatus::Resolved {
      resolution: ApprovalResolution::Decision { decision, .. },
      ..
    } if *decision == accepted_decision
  ));

  let report = store
    .dispatch_ready_notification_updates(run_id, Utc::now(), &mut MixedUpdateAdapter)
    .unwrap();
  assert_eq!(report.updates_attempted, 2);
  assert_eq!(report.updates_succeeded, 1);
  assert_eq!(report.updates_failed, 1);

  let updated = store.projection(run_id).unwrap();
  assert_eq!(updated.status, RunStatus::Running);
  assert!(matches!(
    &updated.approval_requests[APPROVAL_ID].status,
    ApprovalRequestStatus::Resolved {
      resolution: ApprovalResolution::Decision { decision, .. },
      ..
    } if *decision == accepted_decision
  ));
  assert!(updated.notification_updates.values().any(|update| matches!(
    update.status,
    NotificationMessageUpdateStatus::Failed { final_: true, .. }
  )));
}

#[test]
fn repeated_same_decision_is_idempotent_across_provider_capabilities() {
  let database = TemporaryDatabase::new();
  let run_id = "run_cross_provider_duplicate";
  let (slack_capability, telegram_capability) = waiting_store(database.path(), run_id);
  let mut store = DurableEventStore::open(database.path()).unwrap();
  let accepted = store
    .resolve_notification_approval_from_provider(
      &slack_capability,
      SLACK_DELIVERY,
      "slack",
      "U12345678",
      ApprovalDecision::Approved,
      Utc::now(),
    )
    .unwrap();
  let repeated = store
    .resolve_notification_approval_from_provider(
      &telegram_capability,
      TELEGRAM_DELIVERY,
      "telegram",
      "telegram:123456789",
      ApprovalDecision::Approved,
      Utc::now(),
    )
    .unwrap();
  assert_eq!(accepted.status, ApprovalDecisionOutcomeStatus::Accepted);
  assert_eq!(
    repeated.status,
    ApprovalDecisionOutcomeStatus::AlreadyResolved
  );
  assert_eq!(
    store
      .projection(run_id)
      .unwrap()
      .notification_decisions
      .len(),
    1
  );
}
