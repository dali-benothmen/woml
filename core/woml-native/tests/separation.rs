use std::fs;
use std::path::Path;

const LEGACY_MODULES: &[&str] = &[
  "bridge",
  "condition_evaluator",
  "config",
  "context",
  "database",
  "dispatcher",
  "error",
  "job",
  "models",
  "state",
  "step_orchestrator",
  "trigger_executor",
  "triggers",
  "webhook_server",
  "workflow_state_machine",
];

#[test]
fn native_manifest_has_only_the_woml_engine_as_a_local_dependency() {
  let root = Path::new(env!("CARGO_MANIFEST_DIR"));
  let manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
  let local_dependencies = manifest
    .lines()
    .filter(|line| line.contains("path ="))
    .collect::<Vec<_>>();

  assert_eq!(
    local_dependencies,
    vec!["woml-engine = { path = \"../woml-engine\" }"],
    "The WOML native crate must not acquire another local dependency."
  );
}

#[test]
fn native_adapter_does_not_import_the_legacy_core_graph() {
  let root = Path::new(env!("CARGO_MANIFEST_DIR"));
  let adapter = fs::read_to_string(root.join("src/bridge.rs")).unwrap();

  assert!(
    !adapter.contains("crate::"),
    "The extracted adapter must depend on woml_engine, not crate-local legacy modules."
  );

  for module in LEGACY_MODULES {
    assert!(
      !adapter.contains(&format!("use {module}::")),
      "The extracted adapter imports legacy module {module}."
    );
  }

  assert!(
    adapter.contains("use woml_engine::"),
    "The extracted adapter must call the standalone WOML engine."
  );
}

#[test]
fn combined_core_uses_the_canonical_adapter_only_as_a_temporary_shim() {
  let root = Path::new(env!("CARGO_MANIFEST_DIR"));
  let shim = fs::read_to_string(root.join("../src/woml_bridge.rs")).unwrap();

  assert!(shim.contains("include!(\"../woml-native/src/bridge.rs\")"));
  assert!(!shim.contains("pub async fn execute_woml_workflow"));
}
