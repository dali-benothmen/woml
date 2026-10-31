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
    vec!["woml-engine = { version = \"1.1.0\", path = \"../woml-engine\" }"],
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
  let adapter = rust_sources(&root.join("src"));

  assert_adapter_sources(&adapter);
}

fn rust_sources(directory: &Path) -> String {
  let mut paths = fs::read_dir(directory)
    .unwrap()
    .map(|entry| entry.unwrap().path())
    .collect::<Vec<_>>();
  paths.sort();
  paths
    .into_iter()
    .map(|path| {
      assert!(
        !path.is_symlink(),
        "Rust source must not escape through a symlink"
      );
      if path.is_dir() {
        rust_sources(&path)
      } else if path.extension().is_some_and(|extension| extension == "rs") {
        fs::read_to_string(path).unwrap()
      } else {
        String::new()
      }
    })
    .collect::<Vec<_>>()
    .join("\n")
}

fn assert_adapter_sources(adapter: &str) {
  assert!(
    !adapter.contains("crate::"),
    "The extracted adapter must depend on woml_engine, not crate-local legacy modules."
  );

  for module in LEGACY_MODULES {
    assert!(
      !adapter.contains(&format!("use {module}::"))
        && !adapter.contains(&format!("super::{module}::")),
      "The extracted adapter imports legacy module {module}."
    );
  }

  assert!(
    adapter.contains("use woml_engine::"),
    "The extracted adapter must call the standalone WOML engine."
  );
}

#[test]
#[should_panic(expected = "crate-local legacy modules")]
fn native_adapter_guard_rejects_a_hidden_private_module_dependency() {
  let sources = [
    "use woml_engine::RunEvent; mod private;",
    "use crate::models::Workflow;",
  ]
  .join("\n");
  assert_adapter_sources(&sources);
}

#[test]
fn native_adapter_guard_allows_private_bridge_owners() {
  assert_adapter_sources(
    "use woml_engine::RunEvent; use super::configuration::runtime_options; use super::errors::native_execution_error;",
  );
}

#[test]
#[should_panic(expected = "legacy module models")]
fn native_adapter_guard_rejects_a_parent_relative_legacy_dependency() {
  assert_adapter_sources("use woml_engine::RunEvent; use super::super::models::Workflow;");
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
