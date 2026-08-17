use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::Connection;
use serde_json::{json, Map, Value};
use tokio::sync::Notify;
use uuid::Uuid;
use woml_engine::{
  CompiledWorkflowDefinition, DurableEventStore, DurableStoreError, RunEventPayload,
  RuntimeExecutionOptions, ScheduleClock, ScheduleCursorRegistration, ScriptHostProcessOptions,
  TriggerAdmissionRequest, TriggerProgress, WebhookDefinitionRegistration, WomlSchedule,
  WomlWebhookServer, WomlWebhookServerConfig,
};

const SCHEDULE_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/triggers-schedule.compiled.v7.json");
const SCHEDULE_HASH: &str =
  "sha256:5ebaa8eb530be54477cdcbfc8aa71657126cd629a8a09f8c070039705446fa68";

fn model() -> CompiledWorkflowDefinition {
  serde_json::from_str(SCHEDULE_MODEL).unwrap()
}

fn model_with(workflow_id: &str, on_missed: &str) -> CompiledWorkflowDefinition {
  let mut value: Value = serde_json::from_str(SCHEDULE_MODEL).unwrap();
  value["workflowId"] = json!(workflow_id);
  value["triggers"][0]["config"]["fields"]["onMissed"]["value"] = json!(on_missed);
  serde_json::from_value(value).unwrap()
}

fn timestamp(value: &str) -> DateTime<Utc> {
  DateTime::parse_from_rfc3339(value)
    .unwrap()
    .with_timezone(&Utc)
}

struct TemporaryDatabase {
  path: PathBuf,
}

impl TemporaryDatabase {
  fn new(label: &str) -> Self {
    Self {
      path: std::env::temp_dir().join(format!(
        "woml-t9-{label}-{}.sqlite",
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
    let _ = std::fs::remove_file(format!("{}-wal", self.path.display()));
    let _ = std::fs::remove_file(format!("{}-shm", self.path.display()));
  }
}

#[derive(Default)]
struct FakeScheduleClock {
  now: Mutex<DateTime<Utc>>,
  changed: Notify,
}

impl FakeScheduleClock {
  fn new(now: DateTime<Utc>) -> Self {
    Self {
      now: Mutex::new(now),
      changed: Notify::new(),
    }
  }

  fn advance_to(&self, now: DateTime<Utc>) {
    *self.now.lock().unwrap() = now;
    self.changed.notify_waiters();
  }
}

impl ScheduleClock for FakeScheduleClock {
  fn now(&self) -> DateTime<Utc> {
    *self.now.lock().unwrap()
  }

  fn sleep_until(&self, deadline: DateTime<Utc>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
    Box::pin(async move {
      loop {
        let notified = self.changed.notified();
        if self.now() >= deadline {
          return;
        }
        notified.await;
      }
    })
  }
}

fn placeholder_host() -> ScriptHostProcessOptions {
  ScriptHostProcessOptions::new(
    "bun",
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../woml-cli/src/script-host.ts"),
  )
}

fn occurrence_count(path: &Path) -> i64 {
  Connection::open(path)
    .unwrap()
    .query_row("SELECT COUNT(*) FROM woml_trigger_occurrences", [], |row| {
      row.get(0)
    })
    .unwrap()
}

#[test]
fn rust_matches_the_frozen_schedule_semantics_table() {
  let fixture: Value = serde_json::from_str(include_str!(
    "../../../woml/tests/fixtures/schedule-semantics.v1.json"
  ))
  .unwrap();
  for case in fixture["occurrenceCases"].as_array().unwrap() {
    let schedule = WomlSchedule::parse(
      case["cron"].as_str().unwrap(),
      case["timezone"].as_str().unwrap(),
    )
    .unwrap();
    let actual = schedule
      .occurrences_between(
        timestamp(case["startInclusive"].as_str().unwrap()),
        timestamp(case["endExclusive"].as_str().unwrap()),
      )
      .unwrap()
      .into_iter()
      .map(|value| value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
      .collect::<Vec<_>>();
    let expected = case["expected"]
      .as_array()
      .unwrap()
      .iter()
      .map(|value| value.as_str().unwrap().to_string())
      .collect::<Vec<_>>();
    assert_eq!(actual, expected, "schedule case {:?}", case["name"]);
  }
  for cron in fixture["invalidCron"].as_array().unwrap() {
    assert!(WomlSchedule::parse(cron.as_str().unwrap(), "UTC").is_err());
  }
  for timezone in fixture["invalidTimezones"].as_array().unwrap() {
    assert!(WomlSchedule::parse("0 9 * * *", timezone.as_str().unwrap()).is_err());
  }
}

#[test]
fn cursor_and_occurrence_commit_atomically_with_exact_context() {
  let database = TemporaryDatabase::new("atomic");
  let mut store = DurableEventStore::open(database.path()).unwrap();
  let workflow = model();
  store.register_definition(&workflow, SCHEDULE_HASH).unwrap();
  let planned = Utc.with_ymd_and_hms(2026, 8, 8, 7, 0, 0).unwrap();
  let next = Utc.with_ymd_and_hms(2026, 8, 9, 7, 0, 0).unwrap();
  store
    .register_schedule_cursor(
      &ScheduleCursorRegistration {
        workflow_id: workflow.workflow_id.clone(),
        trigger_id: "dailyReport".to_string(),
        definition_hash: SCHEDULE_HASH.to_string(),
        cron: "0 9 * * *".to_string(),
        timezone: "Europe/Berlin".to_string(),
        on_missed: "skip".to_string(),
      },
      planned,
      planned,
    )
    .unwrap();
  let triggered = planned + chrono::Duration::seconds(2);
  let payload = Map::from_iter([
    (
      "scheduledAt".to_string(),
      json!(planned.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
    ),
    (
      "triggeredAt".to_string(),
      json!(triggered.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
    ),
  ]);
  let (admitted, cursor) = store
    .claim_schedule_occurrence(
      planned,
      next,
      TriggerAdmissionRequest {
        workflow_id: workflow.workflow_id.clone(),
        definition_hash: SCHEDULE_HASH.to_string(),
        trigger_id: "dailyReport".to_string(),
        trigger_handler: "trigger.schedule".to_string(),
        source_identity: format!(
          "{}:dailyReport:{}",
          workflow.workflow_id,
          planned.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        ),
        payload: payload.clone(),
        received_at: triggered,
      },
    )
    .unwrap();
  assert_eq!(cursor.next_scheduled_at, next);
  let events = store.events(&admitted.run_id).unwrap();
  let RunEventPayload::RunStarted(started) = &events[0].payload else {
    panic!("first event must be run_started");
  };
  assert_eq!(started.trigger, payload);

  let rollback_planned = next;
  let failed = store.claim_schedule_occurrence(
    rollback_planned,
    rollback_planned,
    TriggerAdmissionRequest {
      workflow_id: workflow.workflow_id.clone(),
      definition_hash: SCHEDULE_HASH.to_string(),
      trigger_id: "dailyReport".to_string(),
      trigger_handler: "trigger.schedule".to_string(),
      source_identity: "must-roll-back".to_string(),
      payload: Map::from_iter([
        (
          "scheduledAt".to_string(),
          json!(rollback_planned.to_rfc3339()),
        ),
        (
          "triggeredAt".to_string(),
          json!(rollback_planned.to_rfc3339()),
        ),
      ]),
      received_at: rollback_planned,
    },
  );
  assert!(matches!(failed, Err(DurableStoreError::Contract(_))));
  assert_eq!(
    store
      .schedule_cursor(&workflow.workflow_id, "dailyReport")
      .unwrap()
      .next_scheduled_at,
    next
  );
  assert_eq!(store.recover_undispatched_trigger_runs().unwrap().len(), 1);
}

#[actix_web::test]
async fn fake_clock_fires_due_schedules_and_restart_does_not_duplicate() {
  let database = TemporaryDatabase::new("fake-clock");
  let before = Utc.with_ymd_and_hms(2026, 8, 8, 6, 59, 0).unwrap();
  let due = Utc.with_ymd_and_hms(2026, 8, 8, 7, 0, 0).unwrap();
  let clock = Arc::new(FakeScheduleClock::new(before));
  let progress = Arc::new(Mutex::new(Vec::<TriggerProgress>::new()));
  let reporter = progress.clone();
  let server = WomlWebhookServer::start(WomlWebhookServerConfig {
    bind_address: "127.0.0.1:0".parse().unwrap(),
    database_path: database.path().to_path_buf(),
    registrations: vec![WebhookDefinitionRegistration::new(model(), SCHEDULE_HASH)],
    startup_manual_triggers: Default::default(),
    execution: RuntimeExecutionOptions::new(placeholder_host(), 100)
      .with_schedule_clock(clock.clone()),
    progress_reporter: Some(Arc::new(move |message| {
      reporter.lock().unwrap().push(message);
    })),
  })
  .await
  .unwrap();
  clock.advance_to(due);
  tokio::time::timeout(std::time::Duration::from_secs(2), async {
    loop {
      if progress.lock().unwrap().iter().any(|message| {
        matches!(
          message,
          TriggerProgress::OccurrenceAccepted {
            trigger_handler,
            duplicate: false,
            ..
          } if trigger_handler == "trigger.schedule"
        )
      }) {
        return;
      }
      tokio::task::yield_now().await;
    }
  })
  .await
  .unwrap();
  server.stop().await;

  assert_eq!(occurrence_count(database.path()), 1);
}

#[actix_web::test]
async fn restart_applies_skip_and_bounded_run_once_without_backlog_replay() {
  let before = Utc.with_ymd_and_hms(2026, 8, 8, 6, 59, 0).unwrap();
  let restarted_at = Utc.with_ymd_and_hms(2026, 8, 10, 8, 0, 0).unwrap();
  for (policy, expected_runs, hash) in [
    (
      "skip",
      0,
      "sha256:1111111111111111111111111111111111111111111111111111111111111111",
    ),
    (
      "run-once",
      1,
      "sha256:2222222222222222222222222222222222222222222222222222222222222222",
    ),
  ] {
    let database = TemporaryDatabase::new(policy);
    let clock = Arc::new(FakeScheduleClock::new(before));
    let workflow = model_with(&format!("restart-{policy}"), policy);
    let first = WomlWebhookServer::start(WomlWebhookServerConfig {
      bind_address: "127.0.0.1:0".parse().unwrap(),
      database_path: database.path().to_path_buf(),
      registrations: vec![WebhookDefinitionRegistration::new(workflow.clone(), hash)],
      startup_manual_triggers: Default::default(),
      execution: RuntimeExecutionOptions::new(placeholder_host(), 100)
        .with_schedule_clock(clock.clone()),
      progress_reporter: None,
    })
    .await
    .unwrap();
    first.stop().await;

    clock.advance_to(restarted_at);
    let second = WomlWebhookServer::start(WomlWebhookServerConfig {
      bind_address: "127.0.0.1:0".parse().unwrap(),
      database_path: database.path().to_path_buf(),
      registrations: vec![WebhookDefinitionRegistration::new(workflow, hash)],
      startup_manual_triggers: Default::default(),
      execution: RuntimeExecutionOptions::new(placeholder_host(), 100)
        .with_schedule_clock(clock.clone()),
      progress_reporter: None,
    })
    .await
    .unwrap();
    second.stop().await;
    assert_eq!(occurrence_count(database.path()), expected_runs, "{policy}");
  }
}

#[actix_web::test]
async fn independent_schedules_due_together_are_both_admitted() {
  let database = TemporaryDatabase::new("simultaneous");
  let before = Utc.with_ymd_and_hms(2026, 8, 8, 6, 59, 0).unwrap();
  let due = Utc.with_ymd_and_hms(2026, 8, 8, 7, 0, 0).unwrap();
  let clock = Arc::new(FakeScheduleClock::new(before));
  let progress = Arc::new(Mutex::new(Vec::<TriggerProgress>::new()));
  let reporter = progress.clone();
  let server = WomlWebhookServer::start(WomlWebhookServerConfig {
    bind_address: "127.0.0.1:0".parse().unwrap(),
    database_path: database.path().to_path_buf(),
    registrations: vec![
      WebhookDefinitionRegistration::new(
        model_with("simultaneous-a", "skip"),
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      ),
      WebhookDefinitionRegistration::new(
        model_with("simultaneous-b", "skip"),
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
      ),
    ],
    startup_manual_triggers: Default::default(),
    execution: RuntimeExecutionOptions::new(placeholder_host(), 100)
      .with_schedule_clock(clock.clone()),
    progress_reporter: Some(Arc::new(move |message| {
      reporter.lock().unwrap().push(message);
    })),
  })
  .await
  .unwrap();
  clock.advance_to(due);
  tokio::time::timeout(std::time::Duration::from_secs(2), async {
    loop {
      let accepted = progress
        .lock()
        .unwrap()
        .iter()
        .filter(|message| {
          matches!(
            message,
            TriggerProgress::OccurrenceAccepted { trigger_handler, .. }
              if trigger_handler == "trigger.schedule"
          )
        })
        .count();
      if accepted == 2 {
        return;
      }
      tokio::task::yield_now().await;
    }
  })
  .await
  .unwrap();
  server.stop().await;
  assert_eq!(occurrence_count(database.path()), 2);
}
