use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::{json, Map};
use woml_engine::{
  execute_workflow_durable, AttemptFailureKind, CompiledWorkflowDefinition, RunEventPayload,
  RuntimeExecutionError, RuntimeExecutionOptions, ScriptHostProcessOptions,
};

const MODEL: &str = include_str!("../../../woml/tests/fixtures/services-bindings.compiled.v8.json");
const HASH: &str = "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

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

fn workflow_with_source(source: String) -> CompiledWorkflowDefinition {
  let mut workflow: CompiledWorkflowDefinition = serde_json::from_str(MODEL).unwrap();
  workflow.graph.nodes.truncate(1);
  workflow.graph.edges.clear();
  let node = workflow.graph.nodes.first_mut().unwrap();
  node
    .script_runtime
    .as_mut()
    .unwrap()
    .required_secrets
    .clear();
  let woml_engine::model::ValueExpression::Object { fields } = &mut node.inputs else {
    panic!("expected script inputs");
  };
  fields.insert(
    "source".to_string(),
    woml_engine::model::ValueExpression::Literal {
      value: json!(source),
    },
  );
  workflow
}

fn temporary_database(name: &str) -> PathBuf {
  std::env::temp_dir().join(format!(
    "woml-sc7-{name}-{}.sqlite",
    uuid::Uuid::new_v4().simple()
  ))
}

fn remove_database(path: &Path) {
  let _ = std::fs::remove_file(path);
  let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
  let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
}

fn javascript_string(value: &Path) -> String {
  serde_json::to_string(&value.to_string_lossy()).unwrap()
}

#[tokio::test]
async fn database_v1_runs_parameterized_crud_and_atomic_transactions_through_rust() {
  let Some(host) = host_options() else { return };
  let state = temporary_database("state-success");
  let user_database = temporary_database("user-success");
  let connection = javascript_string(&user_database);
  let source = format!(
    r#"
      const db = services.db({{ driver: "sqlite", connection: {connection} }});
      const schema = await db.execute({{
        text: "CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT UNIQUE NOT NULL, active INTEGER NOT NULL, score REAL, note TEXT, payload BLOB)"
      }}, {{ name: "create-schema" }});
      const injected = await db.insert({{
        table: "customers",
        values: {{ name: "Robert'); DROP TABLE customers;--", active: true, score: 3.5, note: null, payload: {{ bytesBase64: "AP8=" }} }}
      }}, {{ name: "insert-untrusted-name" }});
      const batch = await db.transaction({{
        operations: [
          {{ operation: "insert", table: "customers", values: {{ name: "Ada", active: true }} }},
          {{ operation: "query", text: "SELECT COUNT(*) AS total FROM customers", values: [] }}
        ]
      }}, {{ name: "create-and-count" }});
      const read = await db.read({{
        table: "customers", columns: ["id", "name", "active", "payload"],
        where: {{ active: true }}, orderBy: [{{ column: "id", direction: "asc" }}]
      }});
      const updated = await db.update({{
        table: "customers", values: {{ active: false }}, where: {{ name: "Ada" }}
      }}, {{ name: "deactivate-ada" }});
      const deleted = await db.delete({{
        table: "customers", where: {{ name: "Robert'); DROP TABLE customers;--" }}
      }}, {{ name: "delete-untrusted-name" }});
      const [finalRows, total] = await Promise.all([
        db.query({{ text: "SELECT id, name, active FROM customers ORDER BY id", values: [] }}),
        db.query({{ text: "SELECT COUNT(*) AS total FROM customers", values: [] }})
      ]);
      return {{ schema, injected, batch, read, updated, deleted, finalRows, total }};
    "#
  );
  let result = execute_workflow_durable(
    workflow_with_source(source),
    HASH.to_string(),
    Map::new(),
    RuntimeExecutionOptions::new(host, 10_000),
    state.clone(),
  )
  .await
  .unwrap();

  assert_eq!(result.result["schema"]["rowsAffected"], 0);
  assert_eq!(result.result["injected"]["rowsAffected"], 1);
  assert_eq!(result.result["batch"]["results"][1]["rows"][0]["total"], 2);
  assert_eq!(result.result["read"]["rowCount"], 2);
  assert_eq!(
    result.result["read"]["rows"][0]["payload"]["bytesBase64"],
    "AP8="
  );
  assert_eq!(result.result["updated"]["rowsAffected"], 1);
  assert_eq!(result.result["deleted"]["rowsAffected"], 1);
  assert_eq!(
    result.result["finalRows"]["rows"],
    json!([{ "id": 2, "name": "Ada", "active": 0 }])
  );
  assert_eq!(
    result
      .events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::OperationStarted(_)))
      .count(),
    8
  );
  let operation_events = result
    .events
    .iter()
    .filter(|event| {
      matches!(
        event.payload,
        RunEventPayload::OperationStarted(_)
          | RunEventPayload::OperationSucceeded(_)
          | RunEventPayload::OperationFailed(_)
      )
    })
    .collect::<Vec<_>>();
  let operation_history = serde_json::to_string(&operation_events).unwrap();
  assert!(!operation_history.contains(&user_database.to_string_lossy().to_string()));
  assert!(!operation_history.contains("Robert"));
  assert!(!operation_history.contains("CREATE TABLE"));
  assert!(operation_history.contains("\"driver\":\"sqlite\""));
  remove_database(&state);
  remove_database(&user_database);
}

#[tokio::test]
async fn database_v1_rolls_back_the_whole_transaction_on_a_constraint_failure() {
  let Some(host) = host_options() else { return };
  let state = temporary_database("state-rollback");
  let user_database = temporary_database("user-rollback");
  let connection = javascript_string(&user_database);
  let source = format!(
    r#"
      const db = services.db({{ driver: "sqlite", connection: {connection} }});
      await db.execute({{ text: "CREATE TABLE items (id INTEGER PRIMARY KEY, code TEXT UNIQUE)" }}, {{ name: "schema" }});
      return await db.transaction({{
        operations: [
          {{ operation: "insert", table: "items", values: {{ code: "same" }} }},
          {{ operation: "insert", table: "items", values: {{ code: "same" }} }}
        ]
      }}, {{ name: "duplicate-batch" }});
    "#
  );
  let error = execute_workflow_durable(
    workflow_with_source(source),
    HASH.to_string(),
    Map::new(),
    RuntimeExecutionOptions::new(host, 5_000),
    state.clone(),
  )
  .await
  .unwrap_err();
  let RuntimeExecutionError::RunFailed(details) = error else {
    panic!("expected failed run");
  };
  assert_eq!(details.failure.kind, AttemptFailureKind::ServiceFailed);
  assert_eq!(details.failure.code, "WOML_DATABASE_CONSTRAINT");
  let connection = Connection::open(&user_database).unwrap();
  let count: i64 = connection
    .query_row("SELECT COUNT(*) FROM items", [], |row| row.get(0))
    .unwrap();
  assert_eq!(count, 0);
  let history = serde_json::to_string(&details.events).unwrap();
  assert!(!history.contains("same"));
  drop(connection);
  remove_database(&state);
  remove_database(&user_database);
}

#[tokio::test]
async fn database_v1_blocks_runtime_state_and_rejects_oversized_results() {
  let Some(host) = host_options() else { return };
  let cases: [(&str, fn(String) -> String, &str); 3] = [
    (
      "protected",
      |connection: String| {
        format!(
          "const db = services.db({{ driver: 'sqlite', connection: {connection} }}); return await db.query({{ text: 'SELECT 1 AS value' }});"
        )
      },
      "WOML_DATABASE_PATH_FORBIDDEN",
    ),
    (
      "large",
      |connection: String| {
        format!(
          "const db = services.db({{ driver: 'sqlite', connection: {connection} }}); return await db.query({{ text: 'WITH RECURSIVE numbers(value) AS (SELECT 1 UNION ALL SELECT value + 1 FROM numbers WHERE value < 10001) SELECT value FROM numbers' }});"
        )
      },
      "WOML_DATABASE_ROWS_TOO_LARGE",
    ),
    (
      "large-value",
      |connection: String| {
        format!(
          "const db = services.db({{ driver: 'sqlite', connection: {connection} }}); return await db.query({{ text: 'SELECT hex(zeroblob(2097153)) AS payload' }});"
        )
      },
      "WOML_DATABASE_RESULT_TOO_LARGE",
    ),
  ];
  for (name, source_for, expected_code) in cases {
    let state = temporary_database(&format!("state-{name}"));
    let user_database = if name == "protected" {
      state.clone()
    } else {
      temporary_database("user-large")
    };
    if name.starts_with("large") {
      drop(Connection::open(&user_database).unwrap());
    }
    let source = source_for(javascript_string(&user_database));
    let error = execute_workflow_durable(
      workflow_with_source(source),
      HASH.to_string(),
      Map::new(),
      RuntimeExecutionOptions::new(host.clone(), 5_000),
      state.clone(),
    )
    .await
    .unwrap_err();
    let RuntimeExecutionError::RunFailed(details) = error else {
      panic!("expected failed run");
    };
    assert_eq!(details.failure.code, expected_code);
    remove_database(&state);
    if user_database != state {
      remove_database(&user_database);
    }
  }
}
