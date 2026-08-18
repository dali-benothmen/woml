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
    vec!["woml-engine = { version = \"1.0.2\", path = \"../woml-engine\" }"],
    "The WOML native crate must not acquire another local dependency."
  );
  assert!(
    !manifest.lines().any(|line| {
      let line = line.trim();
      line.starts_with("core =")
        || line.starts_with("cronflow =")
        || line.contains("package = \"core\"")
        || line.contains("package = \"cronflow\"")
    }),
    "The WOML native crate must not depend on the legacy package under any alias."
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
fn workspace_contains_only_the_woml_engine_and_native_adapter() {
  let root = Path::new(env!("CARGO_MANIFEST_DIR"));
  let workspace = fs::read_to_string(root.join("../Cargo.toml")).unwrap();

  assert!(!workspace.contains("[package]"));
  assert!(workspace.contains("members = [\"woml-engine\", \"woml-native\"]"));
  for retired_path in [
    "../src/lib.rs",
    "../src/bridge.rs",
    "../src/schema.sql",
    "../build.rs",
    "../package.json",
  ] {
    assert!(
      !root.join(retired_path).exists(),
      "Legacy Rust package artifact returned at {retired_path}."
    );
  }
}
