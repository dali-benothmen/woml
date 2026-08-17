use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use serde_json::{json, Map};
use woml_engine::{
  execute_workflow_durable, AttemptFailureKind, CapabilityRegistry, CompiledWorkflowDefinition,
  RunEventPayload, RuntimeExecutionError, RuntimeExecutionOptions, ScriptHostProcessOptions,
  TestCapabilityHandler,
};

const MODEL: &str = include_str!("../../../woml/tests/fixtures/services-bindings.compiled.v8.json");
const HASH: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn host_options() -> Option<ScriptHostProcessOptions> {
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
    .then(|| ScriptHostProcessOptions::new(bun, host))
}

#[tokio::test]
async fn uncaught_capability_failure_remains_service_failed_in_run_event_v8() {
  let Some(host) = host_options() else {
    return;
  };
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
      value: json!("return await services.test.control({ mode: 'fail' });"),
    },
  );
  let registry = Arc::new(CapabilityRegistry::default());
  registry.register(Arc::new(TestCapabilityHandler)).unwrap();
  let database_path = std::env::temp_dir().join(format!(
    "woml-sc3-failure-{}.sqlite",
    uuid::Uuid::new_v4().simple()
  ));
  let error = execute_workflow_durable(
    workflow,
    HASH.to_string(),
    Map::new(),
    RuntimeExecutionOptions::new(host, 2_000).with_capability_registry(registry),
    database_path.clone(),
  )
  .await
  .unwrap_err();
  let RuntimeExecutionError::RunFailed(details) = error else {
    panic!("expected a failed run");
  };
  assert_eq!(
    details.failure.kind,
    AttemptFailureKind::ServiceFailed,
    "unexpected failure: {:#?}",
    details.failure
  );
  assert_eq!(details.failure.code, "WOML_TEST_FAILURE");
  assert_eq!(details.failure.capability.as_deref(), Some("test"));
  assert_eq!(details.failure.operation.as_deref(), Some("control"));
  assert!(details.failure.cause.is_some());
  assert!(details
    .events
    .iter()
    .any(|event| { matches!(event.payload, RunEventPayload::OperationFailed(_)) }));

  let _ = std::fs::remove_file(&database_path);
  let _ = std::fs::remove_file(database_path.with_extension("sqlite-shm"));
  let _ = std::fs::remove_file(database_path.with_extension("sqlite-wal"));
}

#[tokio::test]
async fn model_v8_script_awaits_out_of_order_rust_calls_and_receives_only_declared_secrets() {
  let Some(host) = host_options() else {
    return;
  };
  let mut workflow: CompiledWorkflowDefinition = serde_json::from_str(MODEL).unwrap();
  workflow.graph.nodes.truncate(1);
  workflow.graph.edges.clear();
  let node = workflow.graph.nodes.first_mut().unwrap();
  let woml_engine::model::ValueExpression::Object { fields } = &mut node.inputs else {
    panic!("expected script inputs");
  };
  fields.insert(
    "source".to_string(),
    woml_engine::model::ValueExpression::Literal {
      value: json!(
        r#"
        const [slow, fast] = await Promise.all([
          services.test.control({ mode: "delay", delayMs: 80, value: "slow" }),
          services.test.control({ mode: "delay", delayMs: 5, value: "fast" })
        ]);
        return {
          slow,
          fast,
          secretLength: secrets.CUSTOMER_API_TOKEN.length,
          secretsFrozen: Object.isFrozen(secrets),
          servicesFrozen: Object.isFrozen(services)
        };
      "#
      ),
    },
  );

  let registry = Arc::new(CapabilityRegistry::default());
  registry.register(Arc::new(TestCapabilityHandler)).unwrap();
  let database_path =
    std::env::temp_dir().join(format!("woml-sc3-{}.sqlite", uuid::Uuid::new_v4().simple()));
  let options = RuntimeExecutionOptions::new(host, 2_000)
    .with_capability_registry(registry)
    .with_resolved_secrets(BTreeMap::from([(
      "CUSTOMER_API_TOKEN".to_string(),
      "sc3-secret-value".to_string(),
    )]));
  let result = execute_workflow_durable(
    workflow,
    HASH.to_string(),
    Map::new(),
    options,
    database_path.clone(),
  )
  .await
  .unwrap();

  assert_eq!(
    result.result,
    json!({
      "slow": "slow",
      "fast": "fast",
      "secretLength": 16,
      "secretsFrozen": true,
      "servicesFrozen": true
    })
  );
  assert_eq!(
    result
      .events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::OperationStarted(_)))
      .count(),
    2
  );
  assert_eq!(
    result
      .events
      .iter()
      .filter(|event| matches!(event.payload, RunEventPayload::OperationSucceeded(_)))
      .count(),
    2
  );
  let history = serde_json::to_string(&result.events).unwrap();
  assert!(!history.contains("sc3-secret-value"));

  let _ = std::fs::remove_file(&database_path);
  let _ = std::fs::remove_file(database_path.with_extension("sqlite-shm"));
  let _ = std::fs::remove_file(database_path.with_extension("sqlite-wal"));
}
