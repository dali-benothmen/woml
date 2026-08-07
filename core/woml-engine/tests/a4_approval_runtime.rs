use std::path::{Path, PathBuf};

use chrono::{Duration, Utc};
use rusqlite::Connection;
use serde_json::{json, Map};
use uuid::Uuid;
use woml_engine::{
  execute_workflow_durable, execute_workflow_durable_outcome, resume_workflow_durable_outcome,
  ApprovalRequestedData, ApprovalTimeoutPolicy, CompiledWorkflowDefinition, DurableDagEngine,
  DurableEventStore, RunEventPayload, RunStatus, RuntimeExecutionOptions, ScriptHostProcessOptions,
  WorkflowRuntimeOutcome,
};

const APPROVAL_MODEL: &str = include_str!("../../../woml/tests/fixtures/approval.compiled.v4.json");
const APPROVAL_HASH: &str =
  "sha256:c85377270773c4abb178ba2811109843be53df66c91fedea04bb37d586901aa9";
const TEST_MODEL_HASH: &str =
  "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn approval_model() -> CompiledWorkflowDefinition {
  serde_json::from_str(APPROVAL_MODEL).unwrap()
}

fn approval_first_model() -> CompiledWorkflowDefinition {
  let mut workflow = approval_model();
  workflow.graph.entry_node_ids = vec!["editorApproval".to_string()];
  workflow
    .graph
    .nodes
    .retain(|node| node.id != "prepareArticle");
  workflow
    .graph
    .edges
    .retain(|edge| edge.id != "prepareArticle-to-editorApproval");
  workflow
}

fn terminal_empty_approval_model() -> CompiledWorkflowDefinition {
  let mut workflow = approval_first_model();
  let join_id = "__woml_approval__editorApproval__join";
  workflow
    .graph
    .nodes
    .retain(|node| node.id == "editorApproval" || node.id == join_id);
  workflow
    .graph
    .edges
    .retain(|edge| edge.id == "editorApproval:approved" || edge.id == "editorApproval:rejected");
  for edge in &mut workflow.graph.edges {
    edge.to = join_id.to_string();
  }
  workflow
}

fn unavailable_host_options() -> RuntimeExecutionOptions {
  RuntimeExecutionOptions::new(
    ScriptHostProcessOptions::new(
      PathBuf::from("/woml-a4-no-such-bun"),
      PathBuf::from("/woml-a4-no-such-host.ts"),
    ),
    2_000,
  )
}

fn real_host_options() -> Option<RuntimeExecutionOptions> {
  let bun = std::process::Command::new("bun")
    .arg("--version")
    .output()
    .ok()?
    .status
    .success()
    .then_some(PathBuf::from("bun"))?;
  let host = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../woml-cli/src/script-host.ts");
  host
    .exists()
    .then(|| RuntimeExecutionOptions::new(ScriptHostProcessOptions::new(bun, host), 2_000))
}

struct TemporaryDatabase {
  path: PathBuf,
}

impl TemporaryDatabase {
  fn new(label: &str) -> Self {
    Self {
      path: std::env::temp_dir().join(format!(
        "woml-a4-{label}-{}.sqlite",
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

#[tokio::test]
async fn approval_first_pauses_without_starting_bun_or_any_route_attempt() {
  let database = TemporaryDatabase::new("approval-first");
  let outcome = execute_workflow_durable_outcome(
    approval_first_model(),
    TEST_MODEL_HASH.to_string(),
    Map::new(),
    unavailable_host_options(),
    database.path().to_path_buf(),
  )
  .await
  .unwrap();

  let WorkflowRuntimeOutcome::Waiting {
    contract,
    version,
    workflow_id,
    run_id,
    approval,
  } = outcome
  else {
    panic!("approval-first workflow must wait");
  };
  assert_eq!(contract, "woml.runtime-outcome");
  assert_eq!(version, 1);
  assert_eq!(workflow_id, "publish-article");
  assert_eq!(approval.approval_id, "editorApproval");
  assert_eq!(approval.name.as_deref(), Some("Editorial approval"));
  assert_eq!(
    approval.description.as_deref(),
    Some("Approve the prepared article for publication")
  );
  assert_eq!(approval.on_timeout, ApprovalTimeoutPolicy::Reject);
  assert!(approval.token.starts_with("apr_"));
  assert_eq!(approval.expires_at, Some(approval.credential_expires_at));

  let store = DurableEventStore::open(database.path()).unwrap();
  let events = store.events(&run_id).unwrap();
  assert_eq!(events.len(), 2);
  assert!(matches!(events[0].payload, RunEventPayload::RunStarted(_)));
  assert!(matches!(
    events[1].payload,
    RunEventPayload::ApprovalRequested(_)
  ));
  assert!(!events.iter().any(|event| matches!(
    event.payload,
    RunEventPayload::StepAttemptStarted(_)
      | RunEventPayload::StepAttemptSucceeded(_)
      | RunEventPayload::StepAttemptFailed(_)
  )));
  assert_eq!(
    store.projection(&run_id).unwrap().status,
    RunStatus::Waiting
  );
  assert_eq!(
    store
      .approval_token_count_for_request(&run_id, "editorApproval", &approval.request_id)
      .unwrap(),
    1
  );
  store
    .verify_approval_token(&approval.token, Utc::now())
    .unwrap();

  let encoded = serde_json::to_value(WorkflowRuntimeOutcome::Waiting {
    contract,
    version,
    workflow_id,
    run_id,
    approval,
  })
  .unwrap();
  assert_eq!(encoded["status"], "waiting");
  assert!(encoded.get("url").is_none());
  assert!(encoded.get("execution").is_none());
}

#[tokio::test]
async fn terminal_approval_with_empty_arms_is_a_valid_waiting_boundary() {
  let database = TemporaryDatabase::new("terminal-empty-arms");
  let workflow = terminal_empty_approval_model();
  workflow.validate_for_durable_execution().unwrap();
  assert_eq!(
    workflow.terminal_node_id(),
    Some("__woml_approval__editorApproval__join")
  );

  let outcome = execute_workflow_durable_outcome(
    workflow,
    TEST_MODEL_HASH.to_string(),
    Map::new(),
    unavailable_host_options(),
    database.path().to_path_buf(),
  )
  .await
  .unwrap();
  assert!(matches!(outcome, WorkflowRuntimeOutcome::Waiting { .. }));
}

#[tokio::test]
async fn legacy_success_only_api_rejects_approval_before_creating_a_run() {
  let database = TemporaryDatabase::new("legacy-api");
  let result = execute_workflow_durable(
    approval_first_model(),
    TEST_MODEL_HASH.to_string(),
    Map::new(),
    unavailable_host_options(),
    database.path().to_path_buf(),
  )
  .await;
  assert!(result.is_err());
  assert!(!database.path().exists());
}

#[tokio::test]
async fn restart_reissues_delivery_but_keeps_request_identity_and_never_replays_scripts() {
  let Some(options) = real_host_options() else {
    return;
  };
  let database = TemporaryDatabase::new("restart");
  let first = execute_workflow_durable_outcome(
    approval_model(),
    APPROVAL_HASH.to_string(),
    Map::new(),
    options,
    database.path().to_path_buf(),
  )
  .await
  .unwrap();
  let WorkflowRuntimeOutcome::Waiting {
    run_id,
    approval: first_approval,
    ..
  } = first
  else {
    panic!("reviewed approval workflow must wait");
  };

  let before = DurableEventStore::open(database.path())
    .unwrap()
    .events(&run_id)
    .unwrap();
  assert_eq!(
    before
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::StepAttemptStarted(_)))
      .count(),
    1
  );

  let resumed = resume_workflow_durable_outcome(
    database.path().to_path_buf(),
    &run_id,
    unavailable_host_options(),
  )
  .await
  .unwrap();
  let WorkflowRuntimeOutcome::Waiting {
    approval: resumed_approval,
    ..
  } = resumed
  else {
    panic!("reopened approval workflow must remain waiting");
  };
  assert_eq!(resumed_approval.request_id, first_approval.request_id);
  assert_eq!(resumed_approval.approval_id, first_approval.approval_id);
  assert_ne!(resumed_approval.token, first_approval.token);

  let store = DurableEventStore::open(database.path()).unwrap();
  assert_eq!(store.events(&run_id).unwrap(), before);
  assert_eq!(
    store
      .approval_token_count_for_request(&run_id, "editorApproval", &first_approval.request_id)
      .unwrap(),
    2
  );
}

#[test]
fn token_insert_failure_rolls_back_the_approval_event() {
  let database = TemporaryDatabase::new("atomic-rollback");
  let now = Utc::now();
  let run_id = "run_atomic_approval";
  {
    let store = DurableEventStore::open(database.path()).unwrap();
    let mut engine = DurableDagEngine::new(approval_first_model(), TEST_MODEL_HASH, store).unwrap();
    engine
      .start_run("evt_atomic_start", run_id, now, Map::new())
      .unwrap();
  }
  Connection::open(database.path())
    .unwrap()
    .execute_batch(
      "CREATE TRIGGER reject_approval_token_insert
       BEFORE INSERT ON woml_approval_tokens
       BEGIN
         SELECT RAISE(ABORT, 'test token insert failure');
       END;",
    )
    .unwrap();

  let store = DurableEventStore::open(database.path()).unwrap();
  let mut engine = DurableDagEngine::resume(store, run_id).unwrap();
  let result = engine.request_approval(
    run_id,
    now,
    ApprovalRequestedData {
      approval_id: "editorApproval".to_string(),
      request_id: "aprreq_atomic_request".to_string(),
      expires_at: Some(now + Duration::hours(24)),
      on_timeout: ApprovalTimeoutPolicy::Reject,
    },
  );
  assert!(result.is_err());

  let store = engine.into_store();
  assert_eq!(store.events(run_id).unwrap().len(), 1);
  let projection = store.projection(run_id).unwrap();
  assert_eq!(projection.status, RunStatus::Running);
  assert!(projection.approval_requests.is_empty());
  let token_count: i64 = Connection::open(database.path())
    .unwrap()
    .query_row("SELECT COUNT(*) FROM woml_approval_tokens", [], |row| {
      row.get(0)
    })
    .unwrap();
  assert_eq!(token_count, 0);
}

#[test]
fn durable_profile_accepts_approval_while_non_durable_profile_stays_closed() {
  let workflow = approval_model();
  workflow.validate_for_durable_execution().unwrap();
  assert!(workflow.validate_for_execution().is_err());
  assert_eq!(json!(workflow.schema_version), 4);
}
