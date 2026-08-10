use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::Utc;
use rusqlite::Connection;
use serde_json::{json, Map};
use uuid::Uuid;
use woml_engine::{
  CompiledWorkflowDefinition, DurableEventStore, RunEventPayload, RunStatus,
  RuntimeExecutionOptions, ScriptHostProcessOptions, TriggerAdmissionRequest,
  WebhookDefinitionRegistration, WomlWebhookServer, WomlWebhookServerConfig,
};

const PUBLISHER_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/services-bindings.compiled.v8.json");
const SUBSCRIBER_MODEL: &str =
  include_str!("../../../woml/tests/fixtures/triggers-event.compiled.v7.json");
const CONTROL_TOKEN: &str = "sc11-public-endpoint-token";

struct TemporaryDatabase(PathBuf);

impl TemporaryDatabase {
  fn new() -> Self {
    Self(std::env::temp_dir().join(format!("woml-sc11-{}.sqlite", Uuid::new_v4().simple())))
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

fn host_options() -> Option<ScriptHostProcessOptions> {
  std::process::Command::new("bun")
    .arg("--version")
    .output()
    .ok()?
    .status
    .success()
    .then_some(())?;
  let host = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../woml-cli/src/script-host.ts");
  host
    .exists()
    .then(|| ScriptHostProcessOptions::new(PathBuf::from("bun"), host))
}

fn hash(character: char) -> String {
  format!("sha256:{}", character.to_string().repeat(64))
}

fn publisher() -> CompiledWorkflowDefinition {
  let mut workflow: CompiledWorkflowDefinition = serde_json::from_str(PUBLISHER_MODEL).unwrap();
  workflow.workflow_id = "event-publisher".to_string();
  workflow.graph.nodes.truncate(1);
  workflow.graph.edges.clear();
  workflow.graph.nodes[0]
    .script_runtime
    .as_mut()
    .unwrap()
    .required_secrets
    .clear();
  let woml_engine::model::ValueExpression::Object { fields } = &mut workflow.graph.nodes[0].inputs
  else {
    panic!("script inputs must be an object");
  };
  fields.insert(
    "source".to_string(),
    woml_engine::model::ValueExpression::Literal {
      value: json!(
        r#"
        return await services.events.emit("order.created", {
          orderId: "order-internal-42"
        });
      "#
      ),
    },
  );
  workflow
}

fn subscriber() -> CompiledWorkflowDefinition {
  let mut workflow: CompiledWorkflowDefinition = serde_json::from_str(SUBSCRIBER_MODEL).unwrap();
  workflow.workflow_id = "event-subscriber".to_string();
  workflow
}

fn run_ids(database: &TemporaryDatabase) -> Vec<(String, String)> {
  let connection = Connection::open(database.path()).unwrap();
  let mut statement = connection
    .prepare("SELECT run_id, workflow_id FROM woml_runs ORDER BY created_at, run_id")
    .unwrap();
  statement
    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

async fn wait_for_two_terminal_runs(database: &TemporaryDatabase) -> Vec<(String, String)> {
  let deadline = Instant::now() + Duration::from_secs(15);
  loop {
    let runs = run_ids(database);
    if runs.len() == 2
      && runs.iter().all(|(run_id, _)| {
        DurableEventStore::open(database.path())
          .and_then(|store| store.projection(run_id))
          .is_ok_and(|projection| projection.status == RunStatus::Succeeded)
      })
    {
      return runs;
    }
    if Instant::now() >= deadline {
      let diagnostics = runs
        .iter()
        .map(|(run_id, workflow)| {
          let projection =
            DurableEventStore::open(database.path()).and_then(|store| store.projection(run_id));
          (run_id, workflow, projection)
        })
        .collect::<Vec<_>>();
      panic!("SC11 runs did not complete: {diagnostics:#?}");
    }
    actix_web::rt::time::sleep(Duration::from_millis(20)).await;
  }
}

#[actix_web::test]
async fn services_events_emit_reuses_durable_event_fanout_without_http_or_token() {
  let Some(host) = host_options() else { return };
  let database = TemporaryDatabase::new();
  let mut startup = BTreeMap::new();
  startup.insert("event-publisher".to_string(), "start".to_string());
  let server = WomlWebhookServer::start(WomlWebhookServerConfig {
    bind_address: "127.0.0.1:0".parse().unwrap(),
    database_path: database.path().to_path_buf(),
    registrations: vec![
      WebhookDefinitionRegistration::new(publisher(), hash('a')),
      WebhookDefinitionRegistration::new(subscriber(), hash('b'))
        .with_secret("EVENT_CONTROL_TOKEN", CONTROL_TOKEN),
    ],
    startup_manual_triggers: startup,
    execution: RuntimeExecutionOptions::new(host, 5_000),
    progress_reporter: None,
  })
  .await
  .unwrap();

  let runs = wait_for_two_terminal_runs(&database).await;
  let publisher_run = runs
    .iter()
    .find(|(_, workflow)| workflow == "event-publisher")
    .unwrap();
  let subscriber_run = runs
    .iter()
    .find(|(_, workflow)| workflow == "event-subscriber")
    .unwrap();
  let publisher_projection = DurableEventStore::open(database.path())
    .unwrap()
    .projection(&publisher_run.0)
    .unwrap();
  let result = publisher_projection.result.unwrap();
  assert_eq!(result["eventName"], "order.created");
  assert_eq!(result["status"], "accepted");
  assert_eq!(result["acceptedCount"], 1);
  assert_eq!(result["deliveries"][0]["runId"], subscriber_run.0);

  let subscriber_projection = DurableEventStore::open(database.path())
    .unwrap()
    .projection(&subscriber_run.0)
    .unwrap();
  assert_eq!(
    subscriber_projection.context.trigger,
    Map::from_iter([("orderId".to_string(), json!("order-internal-42"))])
  );

  let events = DurableEventStore::open(database.path())
    .unwrap()
    .events(&publisher_run.0)
    .unwrap();
  let operation_events = events
    .iter()
    .filter(|event| {
      matches!(
        event.payload,
        RunEventPayload::OperationStarted(_) | RunEventPayload::OperationSucceeded(_)
      )
    })
    .collect::<Vec<_>>();
  assert_eq!(operation_events.len(), 2);
  let encoded = serde_json::to_string(&operation_events).unwrap();
  assert!(!encoded.contains("order-internal-42"));
  assert!(!encoded.contains(CONTROL_TOKEN));

  let connection = Connection::open(database.path()).unwrap();
  let lineage: (i64, i64) = connection
    .query_row(
      "SELECT
         (SELECT COUNT(*) FROM woml_internal_event_publications),
         (SELECT COUNT(*) FROM woml_internal_event_deliveries)",
      [],
      |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .unwrap();
  assert_eq!(lineage, (1, 1));
  server.stop().await;
}

#[test]
fn internal_lineage_rejects_cycles_and_stops_at_depth_32() {
  let mut store = DurableEventStore::open_in_memory().unwrap();
  let publisher = publisher();
  store.register_definition(&publisher, &hash('c')).unwrap();
  let parent = store
    .admit_trigger_occurrence(TriggerAdmissionRequest {
      workflow_id: publisher.workflow_id.clone(),
      definition_hash: hash('c'),
      trigger_id: "start".to_string(),
      trigger_handler: "trigger.manual".to_string(),
      source_identity: "sc11-lineage-root".to_string(),
      payload: Map::new(),
      received_at: Utc::now(),
    })
    .unwrap();

  let mut parent_run_id = parent.run_id;
  let mut first_child = None;
  for index in 0..32_u32 {
    let mut workflow = subscriber();
    workflow.workflow_id = format!("lineage-subscriber-{index}");
    let definition_hash = format!("sha256:{index:064x}");
    store
      .register_definition(&workflow, &definition_hash)
      .unwrap();
    let publication_id = format!("internal:v1:sha256:{:064x}", index + 1);
    let request = woml_engine::InternalEventAdmissionRequest {
      publication_id,
      parent_run_id: parent_run_id.clone(),
      event_name: "order.created".to_string(),
      trigger: TriggerAdmissionRequest {
        workflow_id: workflow.workflow_id.clone(),
        definition_hash,
        trigger_id: "orderCreated".to_string(),
        trigger_handler: "trigger.event".to_string(),
        source_identity: format!("sc11-depth-{index}"),
        payload: Map::from_iter([("orderId".to_string(), json!("order-42"))]),
        received_at: Utc::now(),
      },
      emitted_at: Utc::now(),
    };
    let admitted = store
      .admit_internal_event_occurrence(request.clone())
      .unwrap();
    assert_eq!(admitted.depth, index + 1);
    if index == 0 {
      let duplicate = store.admit_internal_event_occurrence(request).unwrap();
      assert!(duplicate.occurrence.duplicate);
      assert_eq!(duplicate.occurrence.run_id, admitted.occurrence.run_id);
    }
    if first_child.is_none() {
      first_child = Some((workflow, admitted.occurrence.run_id.clone()));
    }
    parent_run_id = admitted.occurrence.run_id;
  }

  let mut overflow = subscriber();
  overflow.workflow_id = "lineage-overflow".to_string();
  store.register_definition(&overflow, &hash('d')).unwrap();
  let error = store
    .admit_internal_event_occurrence(woml_engine::InternalEventAdmissionRequest {
      publication_id: format!("internal:v1:sha256:{}", "f".repeat(64)),
      parent_run_id,
      event_name: "order.created".to_string(),
      trigger: TriggerAdmissionRequest {
        workflow_id: overflow.workflow_id,
        definition_hash: hash('d'),
        trigger_id: "orderCreated".to_string(),
        trigger_handler: "trigger.event".to_string(),
        source_identity: "sc11-depth-overflow".to_string(),
        payload: Map::from_iter([("orderId".to_string(), json!("order-42"))]),
        received_at: Utc::now(),
      },
      emitted_at: Utc::now(),
    })
    .unwrap_err();
  assert!(matches!(
    error,
    woml_engine::DurableStoreError::InternalEventDepthExceeded
  ));

  let (first_workflow, first_run_id) = first_child.unwrap();
  let cycle = store
    .admit_internal_event_occurrence(woml_engine::InternalEventAdmissionRequest {
      publication_id: format!("internal:v1:sha256:{}", "e".repeat(64)),
      parent_run_id: first_run_id,
      event_name: "order.created".to_string(),
      trigger: TriggerAdmissionRequest {
        workflow_id: first_workflow.workflow_id,
        definition_hash: format!("sha256:{:064x}", 0),
        trigger_id: "orderCreated".to_string(),
        trigger_handler: "trigger.event".to_string(),
        source_identity: "sc11-cycle".to_string(),
        payload: Map::from_iter([("orderId".to_string(), json!("order-42"))]),
        received_at: Utc::now(),
      },
      emitted_at: Utc::now(),
    })
    .unwrap_err();
  assert!(matches!(
    cycle,
    woml_engine::DurableStoreError::InternalEventCycle
  ));
}
