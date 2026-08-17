use std::{path::PathBuf, sync::Arc, time::Duration};

use futures_util::future::join_all;
use serde_json::{json, Map, Value};
use woml_engine::{
  execute_workflow_durable, CapabilityCancellationToken, CapabilityHandler,
  CompiledWorkflowDefinition, ManagedDatabaseHandler, ManagedDatabasePool, RunEventPayload,
  RuntimeExecutionError, RuntimeExecutionOptions, ScriptHostProcessOptions,
};

const MODEL: &str = include_str!("../../../woml/tests/fixtures/services-bindings.compiled.v8.json");
const HASH: &str = "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

fn postgres_url() -> Option<String> {
  std::env::var("WOML_TEST_POSTGRES_URL")
    .ok()
    .filter(|value| !value.is_empty())
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

fn state_path(name: &str) -> PathBuf {
  std::env::temp_dir().join(format!(
    "woml-sc8-{name}-{}.sqlite",
    uuid::Uuid::new_v4().simple()
  ))
}

fn remove_state(path: &PathBuf) {
  let _ = std::fs::remove_file(path);
  let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
  let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
}

fn js(value: &str) -> String {
  serde_json::to_string(value).unwrap()
}

fn request(connection: &str, operation: &str, input: Value) -> Value {
  json!({
    "contract": "woml.database",
    "contractVersion": 1,
    "kind": "request",
    "driver": "postgres",
    "connection": connection,
    "operation": operation,
    "input": input,
  })
}

fn sqlite_request(connection: &str, operation: &str, input: Value) -> Value {
  let mut value = request(connection, operation, input);
  value["driver"] = Value::String("sqlite".to_string());
  value
}

fn handler(pool: Arc<ManagedDatabasePool>, operation: &str) -> Arc<ManagedDatabaseHandler> {
  ManagedDatabaseHandler::handlers(pool)
    .into_iter()
    .find(|handler| handler.descriptor().operation == operation)
    .unwrap()
}

#[tokio::test]
async fn postgres_database_v1_runs_prepared_crud_transactions_and_parallel_reads() {
  let (Some(connection), Some(host)) = (postgres_url(), host_options()) else {
    return;
  };
  let table = format!("woml_sc8_{}", uuid::Uuid::new_v4().simple());
  let state = state_path("postgres-success");
  let source = format!(
    r#"
      const db = services.db({{ driver: "postgres", connection: {} }});
      await db.execute({{
        text: "CREATE TABLE {table} (id INTEGER GENERATED ALWAYS AS IDENTITY PRIMARY KEY, name TEXT UNIQUE NOT NULL, active BOOLEAN NOT NULL, score DOUBLE PRECISION, note TEXT, payload BYTEA)"
      }}, {{ name: "create-schema" }});
      const injected = await db.insert({{
        table: "{table}",
        values: {{ name: "Robert'); DROP TABLE {table};--", active: true, score: 3.5, note: null, payload: {{ bytesBase64: "AP8=" }} }}
      }}, {{ name: "insert-untrusted-name" }});
      const batch = await db.transaction({{
        operations: [
          {{ operation: "insert", table: "{table}", values: {{ name: "Ada", active: true, score: 4.5 }} }},
          {{ operation: "query", text: "SELECT COUNT(*)::INT4 AS total FROM {table}", values: [] }}
        ]
      }}, {{ name: "create-and-count" }});
      const read = await db.read({{
        table: "{table}", columns: ["id", "name", "active", "payload"],
        where: {{ active: true }}, orderBy: [{{ column: "id", direction: "asc" }}]
      }});
      const updated = await db.update({{
        table: "{table}", values: {{ active: false }}, where: {{ name: "Ada" }}
      }}, {{ name: "deactivate-ada" }});
      const deleted = await db.delete({{
        table: "{table}", where: {{ name: "Robert'); DROP TABLE {table};--" }}
      }}, {{ name: "delete-untrusted-name" }});
      const parallel = await Promise.all(Array.from({{ length: 24 }}, (_, index) =>
        db.query({{ text: "SELECT $1::INT4 AS value FROM pg_sleep(0.01)", values: [index] }})
      ));
      const finalRows = await db.query({{ text: "SELECT id, name, active FROM {table} ORDER BY id", values: [] }});
      await db.execute({{ text: "DROP TABLE {table}" }}, {{ name: "drop-schema" }});
      return {{ injected, batch, read, updated, deleted, parallel, finalRows }};
    "#,
    js(&connection)
  );
  let result = execute_workflow_durable(
    workflow_with_source(source),
    HASH.to_string(),
    Map::new(),
    RuntimeExecutionOptions::new(host, 20_000),
    state.clone(),
  )
  .await
  .unwrap();

  assert_eq!(result.result["injected"]["lastInsertId"], Value::Null);
  assert_eq!(result.result["batch"]["results"][1]["rows"][0]["total"], 2);
  assert_eq!(result.result["read"]["rowCount"], 2);
  assert_eq!(result.result["read"]["rows"][0]["active"], true);
  assert_eq!(
    result.result["read"]["rows"][0]["payload"]["bytesBase64"],
    "AP8="
  );
  assert_eq!(result.result["updated"]["rowsAffected"], 1);
  assert_eq!(result.result["deleted"]["rowsAffected"], 1);
  assert_eq!(result.result["parallel"].as_array().unwrap().len(), 24);
  assert_eq!(result.result["finalRows"]["rows"][0]["name"], "Ada");
  assert_eq!(result.result["finalRows"]["rows"][0]["active"], false);

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
  let history = serde_json::to_string(&operation_events).unwrap();
  assert!(!history.contains(&connection));
  assert!(!history.contains("Robert"));
  assert!(!history.contains("CREATE TABLE"));
  assert!(history.contains("\"driver\":\"postgres\""));
  assert!(result
    .events
    .iter()
    .any(|event| matches!(event.payload, RunEventPayload::OperationSucceeded(_))));
  remove_state(&state);
}

#[tokio::test]
async fn postgres_database_v1_rolls_back_constraint_failures() {
  let (Some(connection), Some(host)) = (postgres_url(), host_options()) else {
    return;
  };
  let table = format!("woml_sc8_rollback_{}", uuid::Uuid::new_v4().simple());
  let state = state_path("postgres-rollback");
  let source = format!(
    r#"
      const db = services.db({{ driver: "postgres", connection: {} }});
      await db.execute({{ text: "CREATE TABLE {table} (code TEXT UNIQUE NOT NULL)" }}, {{ name: "schema" }});
      return await db.transaction({{
        operations: [
          {{ operation: "insert", table: "{table}", values: {{ code: "same" }} }},
          {{ operation: "insert", table: "{table}", values: {{ code: "same" }} }}
        ]
      }}, {{ name: "duplicate-batch" }});
    "#,
    js(&connection)
  );
  let error = execute_workflow_durable(
    workflow_with_source(source),
    HASH.to_string(),
    Map::new(),
    RuntimeExecutionOptions::new(host.clone(), 10_000),
    state.clone(),
  )
  .await
  .unwrap_err();
  let RuntimeExecutionError::RunFailed(details) = error else {
    panic!("expected failed run");
  };
  assert_eq!(details.failure.code, "WOML_DATABASE_CONSTRAINT");
  assert!(!serde_json::to_string(&details.events)
    .unwrap()
    .contains("same"));

  let verify_source = format!(
    r#"
      const db = services.db({{ driver: "postgres", connection: {} }});
      const count = await db.query({{ text: "SELECT COUNT(*)::INT4 AS total FROM {table}" }});
      await db.execute({{ text: "DROP TABLE {table}" }}, {{ name: "cleanup" }});
      return count.rows[0];
    "#,
    js(&connection)
  );
  let verify_state = state_path("postgres-rollback-verify");
  let verified = execute_workflow_durable(
    workflow_with_source(verify_source),
    HASH.to_string(),
    Map::new(),
    RuntimeExecutionOptions::new(host, 10_000),
    verify_state.clone(),
  )
  .await
  .unwrap();
  assert_eq!(verified.result["total"], 0);
  remove_state(&state);
  remove_state(&verify_state);
}

#[tokio::test]
async fn sqlite_and_postgres_share_the_promised_portable_crud_results() {
  let Some(connection) = postgres_url() else {
    return;
  };
  let sqlite = state_path("portable-user-database");
  let sqlite_connection = sqlite.to_string_lossy().to_string();
  let table = format!("woml_sc8_portable_{}", uuid::Uuid::new_v4().simple());
  let pool = Arc::new(ManagedDatabasePool::default());

  for (driver, database_connection, schema) in [
    (
      "sqlite",
      sqlite_connection.as_str(),
      format!("CREATE TABLE {table} (name TEXT PRIMARY KEY, visits INTEGER NOT NULL)"),
    ),
    (
      "postgres",
      connection.as_str(),
      format!("CREATE TABLE {table} (name TEXT PRIMARY KEY, visits INTEGER NOT NULL)"),
    ),
  ] {
    let input = if driver == "sqlite" {
      sqlite_request(database_connection, "execute", json!({ "text": schema }))
    } else {
      request(database_connection, "execute", json!({ "text": schema }))
    };
    handler(pool.clone(), "execute")
      .execute(input, CapabilityCancellationToken::default())
      .await
      .unwrap();
  }

  let mut portable_results = Vec::new();
  for (driver, database_connection) in [
    ("sqlite", sqlite_connection.as_str()),
    ("postgres", connection.as_str()),
  ] {
    let envelope = |operation: &str, input: Value| {
      if driver == "sqlite" {
        sqlite_request(database_connection, operation, input)
      } else {
        request(database_connection, operation, input)
      }
    };
    let inserted = handler(pool.clone(), "insert")
      .execute(
        envelope(
          "insert",
          json!({ "table": table, "values": { "name": "Ada", "visits": 1 } }),
        ),
        CapabilityCancellationToken::default(),
      )
      .await
      .unwrap();
    let updated = handler(pool.clone(), "update")
      .execute(
        envelope(
          "update",
          json!({
            "table": table,
            "values": { "visits": 2 },
            "where": { "name": "Ada" }
          }),
        ),
        CapabilityCancellationToken::default(),
      )
      .await
      .unwrap();
    let selected = handler(pool.clone(), "read")
      .execute(
        envelope(
          "read",
          json!({
            "table": table,
            "columns": ["name", "visits"],
            "where": { "name": "Ada" }
          }),
        ),
        CapabilityCancellationToken::default(),
      )
      .await
      .unwrap();
    portable_results.push(json!({
      "inserted": inserted["data"]["rowsAffected"],
      "updated": updated["data"]["rowsAffected"],
      "selected": selected["data"]
    }));
  }
  assert_eq!(portable_results[0], portable_results[1]);

  for (driver, database_connection) in [
    ("sqlite", sqlite_connection.as_str()),
    ("postgres", connection.as_str()),
  ] {
    let input = if driver == "sqlite" {
      sqlite_request(
        database_connection,
        "execute",
        json!({ "text": format!("DROP TABLE {table}") }),
      )
    } else {
      request(
        database_connection,
        "execute",
        json!({ "text": format!("DROP TABLE {table}") }),
      )
    };
    handler(pool.clone(), "execute")
      .execute(input, CapabilityCancellationToken::default())
      .await
      .unwrap();
  }
  remove_state(&sqlite);
}

#[tokio::test]
async fn postgres_pool_reuses_connections_bounds_concurrency_and_recovers_after_cancellation() {
  let Some(connection) = postgres_url() else {
    return;
  };
  let pool = Arc::new(ManagedDatabasePool::default());
  let query = handler(pool, "query");

  let pid_request = || {
    request(
      &connection,
      "query",
      json!({ "text": "SELECT pg_backend_pid()::INT4 AS pid" }),
    )
  };
  let first = query
    .execute(pid_request(), CapabilityCancellationToken::default())
    .await
    .unwrap();
  let second = query
    .execute(pid_request(), CapabilityCancellationToken::default())
    .await
    .unwrap();
  assert_eq!(
    first["data"]["rows"][0]["pid"],
    second["data"]["rows"][0]["pid"]
  );

  let pid = first["data"]["rows"][0]["pid"].as_i64().unwrap();
  let separator = if connection.contains('?') { '&' } else { '?' };
  let admin_connection = format!("{connection}{separator}application_name=woml_sc8_recovery");
  let terminated = query
    .execute(
      request(
        &admin_connection,
        "query",
        json!({
          "text": "SELECT pg_terminate_backend($1::INT4) AS terminated",
          "values": [pid]
        }),
      ),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap();
  assert_eq!(terminated["data"]["rows"][0]["terminated"], true);
  tokio::time::sleep(Duration::from_millis(50)).await;
  let mut recovered_after_loss = false;
  for _ in 0..3 {
    match query
      .execute(pid_request(), CapabilityCancellationToken::default())
      .await
    {
      Ok(_) => {
        recovered_after_loss = true;
        break;
      }
      Err(failure) => {
        assert_eq!(failure.code, "WOML_DATABASE_CONNECTION_LOST");
        tokio::time::sleep(Duration::from_millis(25)).await;
      }
    }
  }
  assert!(recovered_after_loss);

  let calls = (0..24).map(|index| {
    query.execute(
      request(
        &connection,
        "query",
        json!({
          "text": "SELECT $1::INT4 AS value FROM pg_sleep(0.01)",
          "values": [index]
        }),
      ),
      CapabilityCancellationToken::default(),
    )
  });
  let results = join_all(calls).await;
  assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 24);

  let cancellation = CapabilityCancellationToken::default();
  let cancel_signal = cancellation.clone();
  tokio::spawn(async move {
    tokio::time::sleep(Duration::from_millis(50)).await;
    cancel_signal.cancel();
  });
  let cancelled = query
    .execute(
      request(
        &connection,
        "query",
        json!({ "text": "SELECT 1::INT4 AS value FROM pg_sleep(5)" }),
      ),
      cancellation,
    )
    .await
    .unwrap_err();
  assert_eq!(cancelled.code, "WOML_DATABASE_CANCELLED");
  assert!(!cancelled.ambiguous);

  let recovered = query
    .execute(
      request(
        &connection,
        "query",
        json!({ "text": "SELECT 1::INT4 AS ready" }),
      ),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap();
  assert_eq!(recovered["data"]["rows"][0]["ready"], 1);
}

#[tokio::test]
async fn postgres_connection_failures_are_safe_and_redacted() {
  let connection = "postgresql://woml_super_secret@127.0.0.1:9/unavailable?sslmode=disable";
  let pool = Arc::new(ManagedDatabasePool::default());
  let failure = handler(pool, "query")
    .execute(
      request(
        connection,
        "query",
        json!({ "text": "SELECT 1::INT4 AS value" }),
      ),
      CapabilityCancellationToken::default(),
    )
    .await
    .unwrap_err();
  assert_eq!(failure.code, "WOML_DATABASE_CONNECTION_LOST");
  assert!(failure.retryable);
  assert!(!failure.ambiguous);
  assert!(!serde_json::to_string(&failure)
    .unwrap()
    .contains("woml_super_secret"));
}
