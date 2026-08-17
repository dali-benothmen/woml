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
  CompiledWorkflowDefinition, DurableEventStore, DurableStoreError, IntervalCursorRegistration,
  RunEventPayload, RuntimeExecutionOptions, ScheduleClock, ScriptHostProcessOptions,
  TriggerAdmissionRequest, TriggerProgress, WebhookDefinitionRegistration, WomlInterval,
  WomlWebhookServer, WomlWebhookServerConfig,
};

const INTERVAL_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/triggers-interval.compiled.v7.json");

fn model_with(workflow_id: &str, on_missed: &str, every_ms: u64) -> CompiledWorkflowDefinition {
  let mut value: Value = serde_json::from_str(INTERVAL_MODEL).unwrap();
  value["workflowId"] = json!(workflow_id);
  value["triggers"][0]["config"]["fields"]["onMissed"]["value"] = json!(on_missed);
  value["triggers"][0]["config"]["fields"]["everyMs"]["value"] = json!(every_ms);
  serde_json::from_value(value).unwrap()
}

fn slow_model() -> CompiledWorkflowDefinition {
  let mut value: Value =
    serde_json::to_value(model_with("interval-overlap", "skip", 1_000)).unwrap();
  value["graph"]["nodes"][0]["inputs"]["fields"]["source"]["value"] =
    json!("await new Promise(resolve => setTimeout(resolve, 500)); return context.trigger;");
  serde_json::from_value(value).unwrap()
}

struct TemporaryDatabase(PathBuf);

impl TemporaryDatabase {
  fn new(label: &str) -> Self {
    Self(std::env::temp_dir().join(format!(
      "woml-t10-{label}-{}.sqlite",
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
    let _ = std::fs::remove_file(format!("{}-wal", self.0.display()));
    let _ = std::fs::remove_file(format!("{}-shm", self.0.display()));
  }
}

#[derive(Default)]
struct FakeClock {
  now: Mutex<DateTime<Utc>>,
  changed: Notify,
}

impl FakeClock {
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

impl ScheduleClock for FakeClock {
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
fn interval_grid_is_anchored_and_never_drifts() {
  let interval = WomlInterval::new(5_000).unwrap();
  let anchor = Utc.with_ymd_and_hms(2026, 8, 8, 9, 0, 0).unwrap();
  assert_eq!(
    interval.planned_at(anchor, 1).unwrap(),
    anchor + chrono::Duration::seconds(5)
  );
  assert_eq!(
    interval.planned_at(anchor, 100).unwrap(),
    anchor + chrono::Duration::seconds(500)
  );
  let late = anchor + chrono::Duration::milliseconds(17_900);
  assert_eq!(
    interval.latest_sequence_at_or_before(anchor, late).unwrap(),
    3
  );
  assert_eq!(interval.next_sequence_after(anchor, late).unwrap(), 4);
}

#[test]
fn cursor_and_occurrence_commit_atomically_with_exact_context() {
  let database = TemporaryDatabase::new("atomic");
  let workflow = model_with("interval-atomic", "skip", 1_000);
  let hash = "sha256:1010101010101010101010101010101010101010101010101010101010101010";
  let anchor = Utc.with_ymd_and_hms(2026, 8, 8, 9, 0, 0).unwrap();
  let planned = anchor + chrono::Duration::seconds(1);
  let next = anchor + chrono::Duration::seconds(2);
  let triggered = planned + chrono::Duration::milliseconds(20);
  let mut store = DurableEventStore::open(database.path()).unwrap();
  store.register_definition(&workflow, hash).unwrap();
  store
    .register_interval_cursor(
      &IntervalCursorRegistration {
        workflow_id: workflow.workflow_id.clone(),
        trigger_id: "refreshCache".to_string(),
        definition_hash: hash.to_string(),
        every_ms: 1_000,
        on_missed: "skip".to_string(),
      },
      anchor,
      anchor,
    )
    .unwrap();
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
    .claim_interval_occurrence(
      1,
      planned,
      2,
      next,
      TriggerAdmissionRequest {
        workflow_id: workflow.workflow_id.clone(),
        definition_hash: hash.to_string(),
        trigger_id: "refreshCache".to_string(),
        trigger_handler: "trigger.interval".to_string(),
        source_identity: format!("{}:refreshCache:{}:1", workflow.workflow_id, anchor),
        payload: payload.clone(),
        received_at: triggered,
      },
    )
    .unwrap();
  assert_eq!(cursor.next_sequence, 2);
  assert_eq!(cursor.next_scheduled_at, next);
  let RunEventPayload::RunStarted(started) = &store.events(&admitted.run_id).unwrap()[0].payload
  else {
    panic!("first event must be run_started");
  };
  assert_eq!(started.trigger, payload);

  let failed = store.claim_interval_occurrence(
    2,
    next,
    2,
    next,
    TriggerAdmissionRequest {
      workflow_id: workflow.workflow_id.clone(),
      definition_hash: hash.to_string(),
      trigger_id: "refreshCache".to_string(),
      trigger_handler: "trigger.interval".to_string(),
      source_identity: "must-roll-back".to_string(),
      payload: Map::new(),
      received_at: next,
    },
  );
  assert!(matches!(failed, Err(DurableStoreError::Contract(_))));
  assert_eq!(occurrence_count(database.path()), 1);
  assert_eq!(
    store
      .interval_cursor(&workflow.workflow_id, "refreshCache")
      .unwrap()
      .next_sequence,
    2
  );
}

#[actix_web::test]
async fn fake_clock_fires_each_grid_occurrence_and_restart_does_not_duplicate() {
  let database = TemporaryDatabase::new("fake-clock");
  let anchor = Utc.with_ymd_and_hms(2026, 8, 8, 9, 0, 0).unwrap();
  let clock = Arc::new(FakeClock::new(anchor));
  let progress = Arc::new(Mutex::new(Vec::<TriggerProgress>::new()));
  let reporter = progress.clone();
  let workflow = model_with("interval-fake-clock", "skip", 1_000);
  let hash = "sha256:2020202020202020202020202020202020202020202020202020202020202020";
  let server = WomlWebhookServer::start(WomlWebhookServerConfig {
    bind_address: "127.0.0.1:0".parse().unwrap(),
    database_path: database.path().to_path_buf(),
    registrations: vec![WebhookDefinitionRegistration::new(workflow.clone(), hash)],
    startup_manual_triggers: Default::default(),
    execution: RuntimeExecutionOptions::new(placeholder_host(), 100)
      .with_schedule_clock(clock.clone()),
    progress_reporter: Some(Arc::new(move |message| {
      reporter.lock().unwrap().push(message);
    })),
  })
  .await
  .unwrap();
  clock.advance_to(anchor + chrono::Duration::seconds(1));
  wait_for_accepted(&progress, 1).await;
  clock.advance_to(anchor + chrono::Duration::seconds(2));
  wait_for_accepted(&progress, 2).await;
  server.stop().await;
  assert_eq!(occurrence_count(database.path()), 2);

  let restarted = WomlWebhookServer::start(WomlWebhookServerConfig {
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
  restarted.stop().await;
  assert_eq!(occurrence_count(database.path()), 2);
}

#[actix_web::test]
async fn restart_misfire_is_bounded_for_skip_and_run_once() {
  let anchor = Utc.with_ymd_and_hms(2026, 8, 8, 9, 0, 0).unwrap();
  for (policy, expected, digit) in [("skip", 0, '3'), ("run-once", 1, '4')] {
    let database = TemporaryDatabase::new(policy);
    let clock = Arc::new(FakeClock::new(anchor));
    let workflow = model_with(&format!("interval-{policy}"), policy, 1_000);
    let hash = format!("sha256:{}", digit.to_string().repeat(64));
    let first = WomlWebhookServer::start(WomlWebhookServerConfig {
      bind_address: "127.0.0.1:0".parse().unwrap(),
      database_path: database.path().to_path_buf(),
      registrations: vec![WebhookDefinitionRegistration::new(workflow.clone(), &hash)],
      startup_manual_triggers: Default::default(),
      execution: RuntimeExecutionOptions::new(placeholder_host(), 100)
        .with_schedule_clock(clock.clone()),
      progress_reporter: None,
    })
    .await
    .unwrap();
    first.stop().await;
    clock.advance_to(anchor + chrono::Duration::seconds(100));
    let second = WomlWebhookServer::start(WomlWebhookServerConfig {
      bind_address: "127.0.0.1:0".parse().unwrap(),
      database_path: database.path().to_path_buf(),
      registrations: vec![WebhookDefinitionRegistration::new(workflow, &hash)],
      startup_manual_triggers: Default::default(),
      execution: RuntimeExecutionOptions::new(placeholder_host(), 100)
        .with_schedule_clock(clock.clone()),
      progress_reporter: None,
    })
    .await
    .unwrap();
    second.stop().await;
    assert_eq!(occurrence_count(database.path()), expected, "{policy}");
  }
}

#[actix_web::test]
async fn independent_intervals_due_together_are_both_admitted() {
  let database = TemporaryDatabase::new("simultaneous");
  let anchor = Utc.with_ymd_and_hms(2026, 8, 8, 9, 0, 0).unwrap();
  let clock = Arc::new(FakeClock::new(anchor));
  let progress = Arc::new(Mutex::new(Vec::<TriggerProgress>::new()));
  let reporter = progress.clone();
  let server = WomlWebhookServer::start(WomlWebhookServerConfig {
    bind_address: "127.0.0.1:0".parse().unwrap(),
    database_path: database.path().to_path_buf(),
    registrations: vec![
      WebhookDefinitionRegistration::new(
        model_with("interval-together-a", "skip", 1_000),
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      ),
      WebhookDefinitionRegistration::new(
        model_with("interval-together-b", "skip", 1_000),
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
  clock.advance_to(anchor + chrono::Duration::seconds(1));
  wait_for_accepted(&progress, 2).await;
  server.stop().await;
  assert_eq!(occurrence_count(database.path()), 2);
}

#[actix_web::test]
async fn a_slow_run_does_not_block_the_next_fixed_rate_occurrence() {
  let database = TemporaryDatabase::new("overlap");
  let anchor = Utc.with_ymd_and_hms(2026, 8, 8, 9, 0, 0).unwrap();
  let clock = Arc::new(FakeClock::new(anchor));
  let progress = Arc::new(Mutex::new(Vec::<TriggerProgress>::new()));
  let reporter = progress.clone();
  let server = WomlWebhookServer::start(WomlWebhookServerConfig {
    bind_address: "127.0.0.1:0".parse().unwrap(),
    database_path: database.path().to_path_buf(),
    registrations: vec![WebhookDefinitionRegistration::new(
      slow_model(),
      "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    )],
    startup_manual_triggers: Default::default(),
    execution: RuntimeExecutionOptions::new(placeholder_host(), 2_000)
      .with_schedule_clock(clock.clone()),
    progress_reporter: Some(Arc::new(move |message| {
      reporter.lock().unwrap().push(message);
    })),
  })
  .await
  .unwrap();
  clock.advance_to(anchor + chrono::Duration::seconds(1));
  wait_for_accepted(&progress, 1).await;
  clock.advance_to(anchor + chrono::Duration::seconds(2));
  wait_for_accepted(&progress, 2).await;
  assert_eq!(terminal_count(&progress), 0, "both runs must overlap");
  wait_for_terminal(&progress, 2).await;
  server.stop().await;
  assert_eq!(occurrence_count(database.path()), 2);
}

async fn wait_for_accepted(progress: &Mutex<Vec<TriggerProgress>>, expected: usize) {
  tokio::time::timeout(std::time::Duration::from_secs(2), async {
    loop {
      let count = progress
        .lock()
        .unwrap()
        .iter()
        .filter(|message| {
          matches!(
            message,
            TriggerProgress::OccurrenceAccepted { trigger_handler, duplicate: false, .. }
              if trigger_handler == "trigger.interval"
          )
        })
        .count();
      if count >= expected {
        return;
      }
      tokio::task::yield_now().await;
    }
  })
  .await
  .unwrap();
}

fn terminal_count(progress: &Mutex<Vec<TriggerProgress>>) -> usize {
  progress
    .lock()
    .unwrap()
    .iter()
    .filter(|message| matches!(message, TriggerProgress::RunTerminal { .. }))
    .count()
}

async fn wait_for_terminal(progress: &Mutex<Vec<TriggerProgress>>, expected: usize) {
  tokio::time::timeout(std::time::Duration::from_secs(3), async {
    loop {
      if terminal_count(progress) >= expected {
        return;
      }
      tokio::task::yield_now().await;
    }
  })
  .await
  .unwrap();
}
