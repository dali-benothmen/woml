use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};

use chrono::{Duration, Utc};
use rusqlite::Connection;
use serde_json::Map;
use uuid::Uuid;
use woml_engine::{
  fold_events, ApprovalDecision, ApprovalDecisionOutcomeStatus, ApprovalRequestedData,
  ApprovalTimeoutPolicy, CompiledWorkflowDefinition, DurableDagEngine, DurableEventStore,
  DurableStoreError, NotificationDeliveryStatus, NotificationDeliveryWork,
  NotificationMessageUpdateStatus, NotificationProviderAdapter, NotificationProviderDeliveryResult,
  NotificationProviderUpdateResult, NotificationSafeFailure, NotificationUpdateWork,
  ProviderMessageIdentity, RunEvent, RunEventPayload, RunFailure, RunStatus,
  SlackProviderMessageIdentity, COMPILED_MODEL_SCHEMA_VERSION_V5, RUN_EVENT_SCHEMA_VERSION_V5,
};

const MODEL: &str = include_str!("../../../woml/tests/fixtures/approval-slack.compiled.v5.json");
const PARTIAL: &str = include_str!(
  "../../../woml/tests/fixtures/run-events/approval-slack-partial-delivery.events.v5.json"
);
const ALL_FAILED: &str =
  include_str!("../../../woml/tests/fixtures/run-events/approval-slack-all-failed.events.v5.json");
const APPROVED: &str =
  include_str!("../../../woml/tests/fixtures/run-events/approval-slack-approved.events.v5.json");
const MODEL_HASH: &str = "sha256:a02f094f7200f0e7e33bef7de2aba9b52638ac24adb9f017fd292764fbcb6988";
const APPROVAL_ID: &str = "releaseApproval";
const REQUEST_ID: &str = "aprreq_release_n3";
const DELIVERY_0: &str = "releaseApproval:notify:0:channel:0";
const DELIVERY_1: &str = "releaseApproval:notify:0:channel:1";

fn model() -> CompiledWorkflowDefinition {
  serde_json::from_str(MODEL).unwrap()
}

fn waiting_store(run_id: &str, now: chrono::DateTime<Utc>) -> DurableEventStore {
  waiting_store_with(run_id, now, DurableEventStore::open_in_memory().unwrap())
}

fn waiting_store_with(
  run_id: &str,
  now: chrono::DateTime<Utc>,
  store: DurableEventStore,
) -> DurableEventStore {
  let mut engine = DurableDagEngine::new(model(), MODEL_HASH, store).unwrap();
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
  engine.into_store()
}

struct TemporaryDatabase {
  path: PathBuf,
}

impl TemporaryDatabase {
  fn new() -> Self {
    Self {
      path: std::env::temp_dir().join(format!(
        "woml-n3-notifications-{}.sqlite",
        Uuid::new_v4().simple()
      )),
    }
  }

  fn path(&self) -> &Path {
    &self.path
  }
}

impl Drop for TemporaryDatabase {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.path);
  }
}

fn safe_failure(kind: &str, retryable: bool) -> NotificationSafeFailure {
  NotificationSafeFailure {
    kind: kind.to_string(),
    code: "WOML_NOTIFICATION_PROVIDER_UNAVAILABLE".to_string(),
    message: "The fake provider could not deliver this notification.".to_string(),
    retryable,
    retry_after_ms: None,
  }
}

fn provider_message(index: usize) -> ProviderMessageIdentity {
  ProviderMessageIdentity::Slack(SlackProviderMessageIdentity {
    workspace_id: "T12345678".to_string(),
    channel_id: format!("C1234567{index}"),
    message_id: format!("1723024800.{index:06}"),
  })
}

#[derive(Default)]
struct FakeSlack {
  deliveries: Vec<NotificationDeliveryWork>,
  updates: Vec<NotificationUpdateWork>,
  delivery_results: BTreeMap<String, VecDeque<NotificationProviderDeliveryResult>>,
  update_results: BTreeMap<String, VecDeque<NotificationProviderUpdateResult>>,
}

impl FakeSlack {
  fn delivery_result(&mut self, delivery_id: &str, result: NotificationProviderDeliveryResult) {
    self
      .delivery_results
      .entry(delivery_id.to_string())
      .or_default()
      .push_back(result);
  }

  fn update_result(&mut self, delivery_id: &str, result: NotificationProviderUpdateResult) {
    self
      .update_results
      .entry(delivery_id.to_string())
      .or_default()
      .push_back(result);
  }
}

impl NotificationProviderAdapter for FakeSlack {
  fn deliver(&mut self, work: &NotificationDeliveryWork) -> NotificationProviderDeliveryResult {
    self.deliveries.push(work.clone());
    self
      .delivery_results
      .get_mut(&work.delivery_id)
      .and_then(VecDeque::pop_front)
      .unwrap_or_else(|| {
        NotificationProviderDeliveryResult::Succeeded(provider_message(self.deliveries.len()))
      })
  }

  fn update(&mut self, work: &NotificationUpdateWork) -> NotificationProviderUpdateResult {
    self.updates.push(work.clone());
    self
      .update_results
      .get_mut(&work.delivery_id)
      .and_then(VecDeque::pop_front)
      .unwrap_or(NotificationProviderUpdateResult::Succeeded)
  }
}

#[test]
fn model_v5_and_all_frozen_notification_histories_are_native_rust_contracts() {
  let model: CompiledWorkflowDefinition = serde_json::from_str(MODEL).unwrap();
  assert_eq!(model.schema_version, COMPILED_MODEL_SCHEMA_VERSION_V5);
  model.validate_for_durable_execution().unwrap();

  let partial: Vec<RunEvent> = serde_json::from_str(PARTIAL).unwrap();
  let partial_projection = fold_events(&partial).unwrap();
  assert_eq!(partial_projection.status, RunStatus::Waiting);
  assert_eq!(partial_projection.notification_deliveries.len(), 2);

  let failed: Vec<RunEvent> = serde_json::from_str(ALL_FAILED).unwrap();
  assert!(failed
    .iter()
    .all(|event| event.event_schema_version == RUN_EVENT_SCHEMA_VERSION_V5));
  let failed_projection = fold_events(&failed).unwrap();
  assert_eq!(failed_projection.status, RunStatus::Failed);
  assert!(matches!(
    failed_projection.failure,
    Some(RunFailure::Notification { .. })
  ));

  let approved: Vec<RunEvent> = serde_json::from_str(APPROVED).unwrap();
  let approved_projection = fold_events(&approved).unwrap();
  assert_eq!(approved_projection.status, RunStatus::Succeeded);
  assert_eq!(approved_projection.notification_decisions.len(), 1);
  assert_eq!(approved_projection.notification_updates.len(), 2);
}

#[test]
fn durable_outbox_delivers_every_channel_and_one_decision_updates_every_message() {
  let now = Utc::now();
  let mut store = waiting_store("run_n3_shared", now);
  let requested = store.projection("run_n3_shared").unwrap();
  assert_eq!(requested.status, RunStatus::Waiting);
  assert_eq!(requested.notification_deliveries.len(), 2);
  assert!(requested.context.steps.is_empty());
  assert!(requested
    .notification_deliveries
    .values()
    .all(|delivery| matches!(delivery.status, NotificationDeliveryStatus::Requested)));

  let mut slack = FakeSlack::default();
  let report = store
    .dispatch_ready_notifications("run_n3_shared", now, &mut slack)
    .unwrap();
  assert_eq!(
    (report.attempted, report.succeeded, report.failed),
    (2, 2, 0)
  );
  assert_eq!(slack.deliveries.len(), 2);
  assert_ne!(
    slack.deliveries[0].decision_capability,
    slack.deliveries[1].decision_capability
  );
  assert_eq!(
    slack.deliveries[0].credentials,
    BTreeMap::from([
      ("appToken".to_string(), "SLACK_APP_TOKEN".to_string()),
      ("botToken".to_string(), "SLACK_BOT_TOKEN".to_string()),
    ])
  );

  let outcome = store
    .resolve_notification_approval(
      &slack.deliveries[1].decision_capability,
      "U12345678",
      ApprovalDecision::Approved,
      now + Duration::seconds(1),
    )
    .unwrap();
  assert_eq!(outcome.status, ApprovalDecisionOutcomeStatus::Accepted);
  let repeated = store
    .resolve_notification_approval(
      &slack.deliveries[0].decision_capability,
      "U87654321",
      ApprovalDecision::Approved,
      now + Duration::seconds(2),
    )
    .unwrap();
  assert_eq!(
    repeated.status,
    ApprovalDecisionOutcomeStatus::AlreadyResolved
  );
  assert!(matches!(
    store.resolve_notification_approval(
      &slack.deliveries[0].decision_capability,
      "U87654321",
      ApprovalDecision::Rejected,
      now + Duration::seconds(2),
    ),
    Err(DurableStoreError::ApprovalDecisionConflict)
  ));

  let decided = store.projection("run_n3_shared").unwrap();
  assert_eq!(decided.status, RunStatus::Running);
  assert_eq!(decided.notification_decisions.len(), 1);
  assert_eq!(decided.notification_updates.len(), 2);
  assert_eq!(decided.context.steps.len(), 1);
  assert!(!decided.context.steps.contains_key("finalStatus"));

  let update_report = store
    .dispatch_ready_notification_updates("run_n3_shared", now + Duration::seconds(2), &mut slack)
    .unwrap();
  assert_eq!(
    (
      update_report.updates_attempted,
      update_report.updates_succeeded
    ),
    (2, 2)
  );
  assert_eq!(slack.updates.len(), 2);
  assert!(store
    .events("run_n3_shared")
    .unwrap()
    .iter()
    .all(|event| !matches!(event.payload, RunEventPayload::StepAttemptStarted(_))));
}

#[test]
fn retries_reuse_the_effect_identity_and_partial_failure_keeps_the_approval_waiting() {
  let now = Utc::now();
  let mut store = waiting_store("run_n3_retry", now);
  let first = store
    .begin_notification_delivery("run_n3_retry", DELIVERY_0, now)
    .unwrap();
  store
    .complete_notification_delivery(
      &first,
      NotificationProviderDeliveryResult::Failed(safe_failure("provider_unavailable", true)),
      now,
    )
    .unwrap();
  assert!(store
    .begin_notification_delivery(
      "run_n3_retry",
      DELIVERY_0,
      now + Duration::milliseconds(999)
    )
    .is_err());
  let second = store
    .begin_notification_delivery("run_n3_retry", DELIVERY_0, now + Duration::seconds(1))
    .unwrap();
  assert_eq!(second.attempt, 2);
  assert_eq!(second.idempotency_key, first.idempotency_key);
  assert_ne!(second.attempt_id, first.attempt_id);
  assert_ne!(second.decision_capability, first.decision_capability);
  store
    .complete_notification_delivery(
      &second,
      NotificationProviderDeliveryResult::Succeeded(provider_message(1)),
      now + Duration::seconds(1),
    )
    .unwrap();

  let other = store
    .begin_notification_delivery("run_n3_retry", DELIVERY_1, now + Duration::seconds(1))
    .unwrap();
  let projection = store
    .complete_notification_delivery(
      &other,
      NotificationProviderDeliveryResult::Failed(safe_failure("destination_invalid", false)),
      now + Duration::seconds(1),
    )
    .unwrap();
  assert_eq!(projection.status, RunStatus::Waiting);
  assert!(projection.notification_decisions.is_empty());
  assert!(projection.context.steps.is_empty());
}

#[test]
fn all_final_delivery_failures_fail_the_run_without_inventing_a_decision() {
  let now = Utc::now();
  let mut store = waiting_store("run_n3_all_failed", now);
  let mut slack = FakeSlack::default();
  for delivery_id in [DELIVERY_0, DELIVERY_1] {
    slack.delivery_result(
      delivery_id,
      NotificationProviderDeliveryResult::Failed(safe_failure("destination_invalid", false)),
    );
  }
  let report = store
    .dispatch_ready_notifications("run_n3_all_failed", now, &mut slack)
    .unwrap();
  assert_eq!((report.attempted, report.failed), (2, 2));
  assert!(report.run_failed);
  let projection = store.projection("run_n3_all_failed").unwrap();
  assert_eq!(projection.status, RunStatus::Failed);
  assert!(projection.notification_decisions.is_empty());
  assert!(projection.context.steps.is_empty());
  assert!(matches!(
    projection.failure,
    Some(RunFailure::Notification { .. })
  ));
}

#[test]
fn local_http_fallback_resolution_also_queues_updates_for_delivered_messages() {
  let now = Utc::now();
  let mut store = waiting_store("run_n3_fallback", now);
  let mut slack = FakeSlack::default();
  store
    .dispatch_ready_notifications("run_n3_fallback", now, &mut slack)
    .unwrap();
  let fallback = store
    .reissue_approval_token("run_n3_fallback", APPROVAL_ID, REQUEST_ID, now)
    .unwrap();
  let outcome = store
    .resolve_human_approval(
      &fallback.token,
      ApprovalDecision::Rejected,
      now + Duration::seconds(1),
    )
    .unwrap();
  assert_eq!(outcome.status, ApprovalDecisionOutcomeStatus::Accepted);

  let projection = store.projection("run_n3_fallback").unwrap();
  assert_eq!(projection.notification_updates.len(), 2);
  assert!(projection.notification_decisions.is_empty());
  assert_eq!(projection.status, RunStatus::Running);
}

#[test]
fn recovery_never_loses_unsent_work_or_replays_an_ambiguous_send() {
  let now = Utc::now();
  let mut store = waiting_store("run_n3_recovery", now);

  let before_send = store.recover_interrupted_runs().unwrap();
  assert_eq!(before_send.recovered_runs, 0);
  assert!(store
    .projection("run_n3_recovery")
    .unwrap()
    .notification_deliveries
    .values()
    .all(|delivery| matches!(delivery.status, NotificationDeliveryStatus::Requested)));

  let uncertain = store
    .begin_notification_delivery("run_n3_recovery", DELIVERY_0, now)
    .unwrap();
  let recovery = store.recover_interrupted_runs().unwrap();
  assert_eq!(recovery.recovered_runs, 1);
  let recovered = store.projection("run_n3_recovery").unwrap();
  assert!(matches!(
    recovered.notification_deliveries[DELIVERY_0].status,
    NotificationDeliveryStatus::Failed { final_: true, .. }
  ));

  let mut slack = FakeSlack::default();
  let report = store
    .dispatch_ready_notifications("run_n3_recovery", Utc::now(), &mut slack)
    .unwrap();
  assert_eq!((report.attempted, report.succeeded), (1, 1));
  assert_eq!(slack.deliveries[0].delivery_id, DELIVERY_1);
  assert_ne!(slack.deliveries[0].attempt_id, uncertain.attempt_id);
  assert_eq!(
    store.projection("run_n3_recovery").unwrap().status,
    RunStatus::Waiting
  );
}

#[test]
fn committed_intents_survive_restart_and_capability_secrets_never_enter_events() {
  let database = TemporaryDatabase::new();
  let now = Utc::now();
  {
    let store = DurableEventStore::open(database.path()).unwrap();
    drop(waiting_store_with("run_n3_restart", now, store));
  }

  let mut reopened = DurableEventStore::open(database.path()).unwrap();
  let mut slack = FakeSlack::default();
  let report = reopened
    .dispatch_ready_notifications("run_n3_restart", now, &mut slack)
    .unwrap();
  assert_eq!((report.attempted, report.succeeded), (2, 2));
  let event_json = serde_json::to_string(&reopened.events("run_n3_restart").unwrap()).unwrap();
  for work in &slack.deliveries {
    assert!(!event_json.contains(&work.decision_capability));
    let (_, secret) = work.decision_capability.split_once('.').unwrap();
    assert!(!event_json.contains(secret));
  }
  drop(reopened);

  let connection = Connection::open(database.path()).unwrap();
  let hash_lengths = connection
    .prepare("SELECT length(secret_hash) FROM woml_notification_capabilities ORDER BY attempt_id")
    .unwrap()
    .query_map([], |row| row.get::<_, i64>(0))
    .unwrap()
    .collect::<Result<Vec<_>, _>>()
    .unwrap();
  assert_eq!(hash_lengths, [32, 32]);
}

#[test]
fn interrupted_message_updates_are_retried_without_reopening_the_decision() {
  let now = Utc::now();
  let mut store = waiting_store("run_n3_update_recovery", now);
  let mut slack = FakeSlack::default();
  store
    .dispatch_ready_notifications("run_n3_update_recovery", now, &mut slack)
    .unwrap();
  store
    .resolve_notification_approval(
      &slack.deliveries[0].decision_capability,
      "U12345678",
      ApprovalDecision::Rejected,
      now + Duration::seconds(1),
    )
    .unwrap();
  let interrupted_update = store
    .begin_notification_update(
      "run_n3_update_recovery",
      DELIVERY_0,
      now + Duration::seconds(1),
    )
    .unwrap();

  let recovery = store.recover_interrupted_runs().unwrap();
  assert_eq!(recovery.recovered_runs, 1);
  let recovered = store.projection("run_n3_update_recovery").unwrap();
  let NotificationMessageUpdateStatus::Failed {
    final_: false,
    failed_at,
    ..
  } = recovered.notification_updates[DELIVERY_0].status
  else {
    panic!("an interrupted message update must become retryable");
  };
  assert_eq!(recovered.notification_decisions.len(), 1);

  slack.update_result(
    DELIVERY_1,
    NotificationProviderUpdateResult::Failed(safe_failure("provider_unavailable", true)),
  );
  let report = store
    .dispatch_ready_notification_updates(
      "run_n3_update_recovery",
      failed_at + Duration::seconds(1),
      &mut slack,
    )
    .unwrap();
  assert_eq!(report.updates_attempted, 2);
  let retried_update = slack
    .updates
    .iter()
    .find(|update| update.delivery_id == DELIVERY_0)
    .unwrap();
  assert_eq!(retried_update.attempt, 2);
  assert_eq!(
    retried_update.idempotency_key,
    interrupted_update.idempotency_key
  );
  assert_eq!(
    store
      .projection("run_n3_update_recovery")
      .unwrap()
      .notification_decisions
      .len(),
    1
  );
}
