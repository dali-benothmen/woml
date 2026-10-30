//! Original process characterization and platform tests. Keep process::tests::fixture_child unchanged for child-process test invocation.

use super::*;
use crate::CapabilityCallRequest;
use crate::CapabilityCancellationToken;
use crate::CapabilityFailureKind;
use crate::CapabilityHandler;
use crate::LazyOperationalOutputStore;
use crate::OperationalBodyV1;
use crate::OperationalCloseStatusV1;
use crate::OperationalOutputStore;
use serde_json::json;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

#[test]
fn typescript_and_rust_share_the_frozen_process_v1_payloads() {
  let request: Value = serde_json::from_str(include_str!(
    "../../../../woml/tests/fixtures/services-contracts/process-request.v1.json"
  ))
  .expect("standalone Process v1 request fixture");
  let capability_request: Value = serde_json::from_str(include_str!(
    "../../../../woml/tests/fixtures/services-contracts/capability-process-request.v1.json"
  ))
  .expect("Capability Call v1 Process request fixture");
  assert_eq!(capability_request["input"], request);

  let parsed = parse_request(&request).expect("Rust accepts the TypeScript request fixture");
  assert_eq!(parsed.contract, PROCESS_CONTRACT);
  assert_eq!(parsed.contract_version, PROCESS_CONTRACT_VERSION);
  assert_eq!(parsed.kind, "request");
  assert_eq!(parsed.executable, "bun");
  assert_eq!(parsed.args, ["test", "--timeout", "5000"]);
  assert_eq!(parsed.cwd, ".");
  assert_eq!(parsed.env.get("CI").map(String::as_str), Some("true"));
  assert_eq!(parsed.timeout_ms, 300_000);
  assert_eq!(parsed.accept_exit_codes, [0]);

  let result: Value = serde_json::from_str(include_str!(
    "../../../../woml/tests/fixtures/services-contracts/process-result.v1.json"
  ))
  .expect("standalone Process v1 result fixture");
  let capability_result: Value = serde_json::from_str(include_str!(
    "../../../../woml/tests/fixtures/services-contracts/capability-process-succeeded.v1.json"
  ))
  .expect("Capability Call v1 Process result fixture");
  assert_eq!(capability_result["result"], result);
}

fn policy() -> ProcessPolicyV1 {
  ProcessPolicyV1 {
    contract: PROCESS_POLICY_CONTRACT.to_string(),
    contract_version: PROCESS_POLICY_CONTRACT_VERSION,
    enabled: true,
    modes: ProcessPolicyModes {
      foreground: true,
      background: false,
    },
    project_root: std::env::current_dir()
      .expect("current directory")
      .to_string_lossy()
      .into_owned(),
    executables: vec![ProcessPolicyExecutable {
      alias: "fixture".to_string(),
      path: std::env::current_exe()
        .expect("current test executable")
        .to_string_lossy()
        .into_owned(),
    }],
    working_directories: vec![".".to_string()],
    environment: ProcessPolicyEnvironment {
      inherit: Vec::new(),
      allow: vec![
        "WOML_PROCESS_FIXTURE".to_string(),
        "WOML_PROCESS_FIXTURE_TEXT".to_string(),
        "WOML_PROCESS_FIXTURE_EXIT".to_string(),
        "WOML_PROCESS_FIXTURE_TREE".to_string(),
        "WOML_PROCESS_FIXTURE_HEARTBEAT".to_string(),
        "WOML_PROCESS_FIXTURE_CWD".to_string(),
        "WOML_PROCESS_FIXTURE_ENV_NAME".to_string(),
        "WOML_PROCESS_FIXTURE_PRINT_ENV".to_string(),
        "PATH".to_string(),
      ],
    },
    limits: ProcessPolicyLimits {
      max_timeout_ms: 5_000,
      max_output_bytes: 1_024,
      max_stream_bytes: crate::DEFAULT_OPERATIONAL_INLINE_RUN_BYTES as usize,
      max_concurrent: 1,
    },
  }
}

fn request(environment: Value, max_bytes: usize, accepted: Vec<i32>) -> Value {
  json!({
    "contract": PROCESS_CONTRACT,
    "contractVersion": PROCESS_CONTRACT_VERSION,
    "kind": "request",
    "executable": "fixture",
    "args": ["--exact", "process::tests::fixture_child", "--nocapture"],
    "cwd": ".",
    "env": environment,
    "timeoutMs": 5_000,
    "acceptExitCodes": accepted,
    "output": { "stdout": "capture", "stderr": "capture", "maxBytes": max_bytes }
  })
}

fn with_cwd(mut request: Value, cwd: &str) -> Value {
  request["cwd"] = json!(cwd);
  request
}

fn temporary_directory(label: &str) -> PathBuf {
  let path = std::env::temp_dir().join(format!(
    "woml-process-{label}-{}",
    uuid::Uuid::new_v4().simple()
  ));
  std::fs::create_dir(&path).expect("create temporary directory");
  path
}

#[test]
fn fixture_child() {
  if std::env::var("WOML_PROCESS_FIXTURE").as_deref() != Ok("woml-process-fixture-active") {
    return;
  }
  if let Ok(role) = std::env::var("WOML_PROCESS_FIXTURE_TREE") {
    let heartbeat =
      std::env::var("WOML_PROCESS_FIXTURE_HEARTBEAT").expect("tree fixture heartbeat path");
    if role == "parent" {
      std::process::Command::new(std::env::current_exe().expect("fixture executable"))
        .args(["--exact", "process::tests::fixture_child", "--nocapture"])
        .env_clear()
        .env("WOML_PROCESS_FIXTURE", "woml-process-fixture-active")
        .env("WOML_PROCESS_FIXTURE_TREE", "grandchild")
        .env("WOML_PROCESS_FIXTURE_HEARTBEAT", &heartbeat)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn grandchild fixture");
    }
    let marker = if role == "parent" { b'P' } else { b'G' };
    loop {
      use std::io::Write;
      let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&heartbeat)
        .expect("open heartbeat");
      file.write_all(&[marker]).expect("write heartbeat");
      file.sync_all().expect("sync heartbeat");
      std::thread::sleep(Duration::from_millis(10));
    }
  }
  if std::env::var("WOML_PROCESS_FIXTURE_CWD").as_deref() == Ok("report-current-directory") {
    print!(
      "{}",
      std::env::current_dir()
        .expect("fixture cwd")
        .to_string_lossy()
    );
    return;
  }
  if let Ok(name) = std::env::var("WOML_PROCESS_FIXTURE_ENV_NAME") {
    print!(
      "{}",
      if std::env::var_os(name).is_some() {
        "present"
      } else {
        "absent"
      }
    );
    return;
  }
  if let Ok(name) = std::env::var("WOML_PROCESS_FIXTURE_PRINT_ENV") {
    print!("{}", std::env::var(name).unwrap_or_default());
    return;
  }
  let text = std::env::var("WOML_PROCESS_FIXTURE_TEXT").unwrap_or_default();
  print!("{text}");
  if let Ok(code) = std::env::var("WOML_PROCESS_FIXTURE_EXIT") {
    std::process::exit(code.parse().expect("fixture exit code"));
  }
}

#[tokio::test]
async fn approved_process_returns_bounded_output() {
  let handler =
    ManagedProcessHandler::new(policy(), ProcessRuntimeMode::Foreground).expect("valid policy");
  let value = handler
    .execute(
      request(
        json!({
          "WOML_PROCESS_FIXTURE": "woml-process-fixture-active",
          "WOML_PROCESS_FIXTURE_TEXT": "abcdefghijklmnopqrstuvwxyz"
        }),
        10,
        vec![0],
      ),
      CapabilityCancellationToken::default(),
    )
    .await
    .expect("approved process succeeds");
  assert_eq!(value["contract"], PROCESS_CONTRACT);
  assert_eq!(value["exitCode"], 0);
  assert_eq!(value["stdoutTruncated"], true);
  assert!(value["stdoutBytes"].as_u64().expect("byte count") > 10);
  assert!(value["stdout"].as_str().expect("stdout").len() <= 10);
}

#[tokio::test]
async fn process_v2_streams_redacted_output_and_preserves_bounded_capture() {
  let directory = temporary_directory("stream-v2");
  let output_path = directory.join("output.sqlite");
  let secrets = Arc::new(BTreeMap::from([(
    "token".to_string(),
    "stream-secret-42".to_string(),
  )]));
  let handler =
    ManagedProcessHandler::new_with_secrets(policy(), ProcessRuntimeMode::Foreground, secrets)
      .expect("valid Process policy");
  handler
    .configure_output_store(Arc::new(LazyOperationalOutputStore::new(
      &output_path,
      "deployment-process-v2",
    )))
    .expect("configure output store");
  let mut input = request(
    json!({
      "WOML_PROCESS_FIXTURE": "woml-process-fixture-active",
      "WOML_PROCESS_FIXTURE_TEXT": "before-stream-secret-42-after"
    }),
    128,
    vec![0],
  );
  input["contractVersion"] = json!(PROCESS_CONTRACT_VERSION_V2);
  input["stream"] = json!(true);
  let call = CapabilityCallRequest {
    contract: crate::CAPABILITY_CALL_CONTRACT.to_string(),
    contract_version: crate::CAPABILITY_CALL_CONTRACT_VERSION,
    message_type: "request".to_string(),
    invocation_id: "inv-process-v2".to_string(),
    call_id: "call-process-v2".to_string(),
    run_id: "run-process-v2".to_string(),
    node_id: "runCommand".to_string(),
    attempt_number: 1,
    capability: "process".to_string(),
    operation: "run".to_string(),
    input_contract_version: 1,
    result_contract_version: 1,
    identity: crate::CapabilityCallIdentity {
      mode: crate::capability::CapabilityIdentityMode::Named,
      step_idempotency_key: format!("sha256:{}", "1".repeat(64)),
      operation_name: "process.run.fixture".to_string(),
      operation_key: format!("sha256:{}", "2".repeat(64)),
      provider_idempotency_key: None,
    },
    limits: crate::CapabilityCallLimits::default(),
    input,
  };
  let result = handler
    .execute_request_scoped(
      &call,
      Some("process-stream-workflow".to_string()),
      CapabilityCancellationToken::default(),
    )
    .await
    .expect("Process v2 succeeds");
  assert_eq!(result["contractVersion"], PROCESS_CONTRACT_VERSION_V2);
  assert_eq!(result["streamed"], true);
  assert!(result["stdout"].as_str().unwrap().contains("[redacted]"));
  assert!(!result["stdout"]
    .as_str()
    .unwrap()
    .contains("stream-secret-42"));

  let store = OperationalOutputStore::open(&output_path, "deployment-process-v2")
    .expect("reopen output store");
  let records = store
    .read_run_page("run-process-v2", 0, 100)
    .expect("stream replay")
    .records;
  let encoded = serde_json::to_string(&records).expect("record JSON");
  assert!(encoded.contains("[redacted]"));
  assert!(!encoded.contains("stream-secret-42"));
  assert!(matches!(
    records.last().map(|record| &record.body),
    Some(OperationalBodyV1::Closed {
      status: OperationalCloseStatusV1::Succeeded,
      ..
    })
  ));
  let _ = std::fs::remove_dir_all(directory);
}

#[tokio::test]
async fn inherited_environment_values_are_redacted_from_captured_output() {
  let mut configured = policy();
  configured.environment.inherit = vec!["PATH".to_string()];
  let handler = ManagedProcessHandler::new(configured, ProcessRuntimeMode::Foreground)
    .expect("valid inherited environment policy");
  let value = handler
    .execute(
      request(
        json!({
          "WOML_PROCESS_FIXTURE": "woml-process-fixture-active",
          "WOML_PROCESS_FIXTURE_PRINT_ENV": "PATH"
        }),
        1_024,
        vec![0],
      ),
      CapabilityCancellationToken::default(),
    )
    .await
    .expect("approved process succeeds");
  assert!(value["stdout"]
    .as_str()
    .expect("stdout")
    .contains("[redacted]"));
  assert!(!value
    .to_string()
    .contains(&std::env::var("PATH").expect("host PATH")));
}

#[tokio::test]
async fn ambient_environment_is_cleared_unless_explicitly_inherited() {
  let handler =
    ManagedProcessHandler::new(policy(), ProcessRuntimeMode::Foreground).expect("valid policy");
  let value = handler
    .execute(
      request(
        json!({
          "WOML_PROCESS_FIXTURE": "woml-process-fixture-active",
          "WOML_PROCESS_FIXTURE_ENV_NAME": "HOME"
        }),
        128,
        vec![0],
      ),
      CapabilityCancellationToken::default(),
    )
    .await
    .expect("approved process succeeds");
  assert!(value["stdout"].as_str().expect("stdout").contains("absent"));
}

#[tokio::test]
async fn policy_inherits_only_names_the_operator_approved() {
  let mut configured = policy();
  configured.environment.inherit = vec!["PATH".to_string()];
  let handler = ManagedProcessHandler::new(configured, ProcessRuntimeMode::Foreground)
    .expect("valid inherited environment policy");
  let value = handler
    .execute(
      request(
        json!({
          "WOML_PROCESS_FIXTURE": "woml-process-fixture-active",
          "WOML_PROCESS_FIXTURE_ENV_NAME": "PATH"
        }),
        128,
        vec![0],
      ),
      CapabilityCancellationToken::default(),
    )
    .await
    .expect("approved inherited variable is available");
  assert!(value["stdout"]
    .as_str()
    .expect("stdout")
    .contains("present"));
}

#[tokio::test]
async fn path_shadowing_cannot_change_the_preflighted_executable() {
  let handler =
    ManagedProcessHandler::new(policy(), ProcessRuntimeMode::Foreground).expect("valid policy");
  let value = handler
    .execute(
      request(
        json!({
          "WOML_PROCESS_FIXTURE": "woml-process-fixture-active",
          "WOML_PROCESS_FIXTURE_TEXT": "approved-binary-ran",
          "PATH": "/definitely/not/the/operator/path"
        }),
        128,
        vec![0],
      ),
      CapabilityCancellationToken::default(),
    )
    .await
    .expect("canonical executable is used directly");
  assert_eq!(value["exitCode"], 0);
}

#[tokio::test]
async fn cwd_is_resolved_from_the_policy_project_root() {
  let root = temporary_directory("project-root");
  let mut configured = policy();
  configured.project_root = root.to_string_lossy().into_owned();
  configured.working_directories = vec![".".to_string()];
  let handler = ManagedProcessHandler::new(configured, ProcessRuntimeMode::Foreground)
    .expect("valid project policy");
  let value = handler
    .execute(
      request(
        json!({
          "WOML_PROCESS_FIXTURE": "woml-process-fixture-active",
          "WOML_PROCESS_FIXTURE_CWD": "report-current-directory"
        }),
        512,
        vec![0],
      ),
      CapabilityCancellationToken::default(),
    )
    .await
    .expect("approved process succeeds");
  let stdout = value["stdout"].as_str().expect("stdout");
  let canonical_root = root.canonicalize().expect("canonical policy project root");
  #[cfg(windows)]
  {
    let reported = stdout
      .replace(r"\\?\", "")
      .replace('\\', "/")
      .to_ascii_lowercase();
    let expected = canonical_root
      .to_string_lossy()
      .replace(r"\\?\", "")
      .replace('\\', "/")
      .to_ascii_lowercase();
    assert!(reported.contains(&expected));
  }
  #[cfg(not(windows))]
  assert!(stdout.contains(canonical_root.to_string_lossy().as_ref()));
  let _ = std::fs::remove_dir(root);
}

#[cfg(windows)]
#[tokio::test]
async fn windows_child_receives_system_root_without_inheriting_the_application_environment() {
  let handler =
    ManagedProcessHandler::new(policy(), ProcessRuntimeMode::Foreground).expect("valid policy");
  let value = handler
    .execute(
      request(
        json!({
          "WOML_PROCESS_FIXTURE": "woml-process-fixture-active",
          "WOML_PROCESS_FIXTURE_PRINT_ENV": "SystemRoot"
        }),
        1_024,
        vec![0],
      ),
      CapabilityCancellationToken::default(),
    )
    .await
    .expect("child starts with the Windows bootstrap environment");
  assert!(value["stdout"].as_str().expect("stdout").contains(
    windows_system_root()
      .expect("Windows system directory")
      .to_string_lossy()
      .as_ref()
  ));
  let value = handler
    .execute(
      request(
        json!({
          "WOML_PROCESS_FIXTURE": "woml-process-fixture-active",
          "WOML_PROCESS_FIXTURE_ENV_NAME": "PATH"
        }),
        128,
        vec![0],
      ),
      CapabilityCancellationToken::default(),
    )
    .await
    .expect("child starts without an ambient application PATH");
  assert!(value["stdout"].as_str().expect("stdout").contains("absent"));
}

#[cfg(windows)]
#[tokio::test]
#[ignore = "requires WOML_PROCESS_TEST_NODE; run explicitly by Windows CI"]
async fn windows_node_executes_javascript_with_only_the_os_bootstrap_environment() {
  let mut configured = policy();
  configured.limits.max_timeout_ms = 10_000;
  configured.executables = vec![ProcessPolicyExecutable {
    alias: "node".to_string(),
    path: std::env::var("WOML_PROCESS_TEST_NODE").expect("CI-provided Node executable"),
  }];
  let handler = ManagedProcessHandler::new(configured, ProcessRuntimeMode::Foreground)
    .expect("valid Node process policy");
  let arguments = ["space value", "café 雪", "\"quoted\"", "; echo not-a-shell"];
  let script = r#"
    const assert = require('node:assert/strict');
    assert.ok(process.env.SystemRoot);
    assert.equal(process.env.PATH, undefined);
    assert.equal(process.env.HOME, undefined);
    assert.equal(require('node:crypto').randomBytes(16).length, 16);
    process.stdout.write(JSON.stringify(process.argv.slice(1)));
  "#;
  let mut args = vec!["-e", script];
  args.extend(arguments);
  let mut input = request(json!({}), 1_024, vec![0]);
  input["executable"] = json!("node");
  input["args"] = json!(args);
  input["timeoutMs"] = json!(10_000);
  let value = handler
    .execute(input, CapabilityCancellationToken::default())
    .await
    .expect("Node executes JavaScript and crypto under Rust process containment");
  assert_eq!(value["exitCode"], 0);
  assert_eq!(value["stdout"], serde_json::to_string(&arguments).unwrap());
  assert_eq!(value["stderr"], "");
}

#[cfg(unix)]
#[test]
fn symlink_cannot_escape_an_allowed_working_directory() {
  use std::os::unix::fs::symlink;
  let root = temporary_directory("symlink-root");
  let outside = temporary_directory("symlink-outside");
  symlink(&outside, root.join("escape")).expect("create symlink");
  let mut configured = policy();
  configured.project_root = root.to_string_lossy().into_owned();
  configured.working_directories = vec![".".to_string()];
  let handler = ManagedProcessHandler::new(configured, ProcessRuntimeMode::Foreground)
    .expect("valid project policy");
  let error = handler
    .parse_and_authorize(&with_cwd(request(json!({}), 128, vec![0]), "escape"))
    .expect_err("symlink escape denied");
  assert_eq!(error.code, "WOML_PROCESS_CWD_DENIED");
  let _ = std::fs::remove_file(root.join("escape"));
  let _ = std::fs::remove_dir(root);
  let _ = std::fs::remove_dir(outside);
}

#[cfg(unix)]
#[test]
fn executable_replacement_is_detected_after_preflight() {
  let root = temporary_directory("executable-identity");
  let executable = root.join("fixture");
  std::fs::copy(
    std::env::current_exe().expect("current executable"),
    &executable,
  )
  .expect("copy executable");
  let mut configured = policy();
  configured.executables[0].path = executable.to_string_lossy().into_owned();
  let handler = ManagedProcessHandler::new(configured, ProcessRuntimeMode::Foreground)
    .expect("valid executable policy");
  let replacement = root.join("replacement");
  std::fs::copy(
    std::env::current_exe().expect("current executable"),
    &replacement,
  )
  .expect("copy replacement");
  std::fs::rename(&replacement, &executable).expect("replace executable");
  let error = handler
    .parse_and_authorize(&request(json!({}), 128, vec![0]))
    .expect_err("replacement denied");
  assert_eq!(error.code, "WOML_PROCESS_EXECUTABLE_CHANGED");
  let _ = std::fs::remove_file(executable);
  let _ = std::fs::remove_dir(root);
}

#[test]
fn validation_rejects_device_paths_and_resource_exhaustion() {
  for cwd in ["NUL", "con.txt", "COM1", "LPT9.log", "folder. "] {
    let error = parse_request(&with_cwd(request(json!({}), 128, vec![0]), cwd))
      .expect_err("portable device path denied");
    assert_eq!(error.code, "WOML_PROCESS_INPUT_INVALID");
  }
  let too_many_args = (0..257).map(|_| "x").collect::<Vec<_>>();
  let mut value = request(json!({}), 128, vec![0]);
  value["args"] = json!(too_many_args);
  assert_eq!(
    parse_request(&value)
      .expect_err("argument count bounded")
      .code,
    "WOML_PROCESS_INPUT_INVALID"
  );
  value = request(json!({}), 128, vec![0]);
  value["args"] = json!(["x".repeat(32_769)]);
  assert_eq!(
    parse_request(&value)
      .expect_err("argument size bounded")
      .code,
    "WOML_PROCESS_INPUT_INVALID"
  );
  let environment = (0..129)
    .map(|index| (format!("SAFE_{index}"), "x"))
    .collect::<BTreeMap<_, _>>();
  assert_eq!(
    parse_request(&request(json!(environment), 128, vec![0]))
      .expect_err("environment count bounded")
      .code,
    "WOML_PROCESS_INPUT_INVALID"
  );
  assert_eq!(
    parse_request(&request(
      json!({ "SAFE": "x".repeat(32_769) }),
      128,
      vec![0]
    ))
    .expect_err("environment value bounded")
    .code,
    "WOML_PROCESS_INPUT_INVALID"
  );
}

#[test]
fn generated_process_requests_are_validated_without_panics() {
  let alphabet = [b'a', b'Z', b'0', b'.', b'/', b'\\', b':', b' ', 0, 0xff];
  let mut state = 0x5eed_u64;
  for length in 0..256 {
    let mut bytes = Vec::with_capacity(length);
    for _ in 0..length {
      state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1);
      bytes.push(alphabet[(state as usize) % alphabet.len()]);
    }
    let candidate = String::from_utf8_lossy(&bytes).into_owned();
    let value = with_cwd(request(json!({}), 128, vec![0]), &candidate);
    let _ = parse_request(&value);
  }
}

#[test]
fn every_process_failure_has_an_author_facing_corrective_action() {
  for code in [
    "WOML_PROCESS_EXECUTABLE_DENIED",
    "WOML_PROCESS_EXECUTABLE_NOT_FOUND",
    "WOML_PROCESS_EXECUTABLE_CHANGED",
    "WOML_PROCESS_CWD_DENIED",
    "WOML_PROCESS_ENV_DENIED",
    "WOML_PROCESS_SPAWN_FAILED",
    "WOML_PROCESS_TIMED_OUT",
    "WOML_PROCESS_CANCELLED",
    "WOML_PROCESS_EXIT_CODE",
    "WOML_PROCESS_INPUT_INVALID",
  ] {
    let error = failure(
      CapabilityFailureKind::ServiceRejected,
      code,
      "Safe process failure.",
      false,
      false,
      None,
    );
    assert!(error.message.contains("Corrective action:"), "{code}");
    assert!(!error.message.contains('/') && !error.message.contains('\\'));
  }
}

#[tokio::test]
async fn nonaccepted_exit_code_has_only_safe_failure_metadata() {
  let handler =
    ManagedProcessHandler::new(policy(), ProcessRuntimeMode::Foreground).expect("valid policy");
  let error = handler
    .execute(
      request(
        json!({
          "WOML_PROCESS_FIXTURE": "woml-process-fixture-active",
          "WOML_PROCESS_FIXTURE_TEXT": "sensitive output",
          "WOML_PROCESS_FIXTURE_EXIT": "7"
        }),
        128,
        vec![0],
      ),
      CapabilityCancellationToken::default(),
    )
    .await
    .expect_err("exit code is rejected");
  assert_eq!(error.code, "WOML_PROCESS_EXIT_CODE");
  let details = error.details.expect("safe details");
  assert_eq!(details.get("exitCode"), Some(&json!(7)));
  assert!(!details.contains_key("stdout"));
  assert!(!details.contains_key("stderr"));
}

#[tokio::test]
async fn request_environment_is_enforced_again_in_rust() {
  let handler =
    ManagedProcessHandler::new(policy(), ProcessRuntimeMode::Foreground).expect("valid policy");
  let error = handler
    .execute(
      request(json!({ "UNAPPROVED_SECRET": "no" }), 128, vec![0]),
      CapabilityCancellationToken::default(),
    )
    .await
    .expect_err("environment is denied");
  assert_eq!(error.code, "WOML_PROCESS_ENV_DENIED");
}

#[test]
fn runtime_policy_caps_the_effective_process_timeout() {
  let handler =
    ManagedProcessHandler::new(policy(), ProcessRuntimeMode::Foreground).expect("valid policy");
  let mut input = request(
    json!({ "WOML_PROCESS_FIXTURE": "woml-process-fixture-active" }),
    128,
    vec![0],
  );
  input["timeoutMs"] = json!(10_000);
  let authorized = handler
    .parse_and_authorize(&input)
    .expect("timeout is bounded by policy");
  assert_eq!(authorized.request.timeout_ms, 5_000);
}

#[test]
fn process_versions_keep_v1_exact_and_accept_the_explicit_v2_shape() {
  let v1 = request(
    json!({ "WOML_PROCESS_FIXTURE": "woml-process-fixture-active" }),
    128,
    vec![0],
  );
  assert_eq!(
    parse_request(&v1)
      .expect("exact Process v1")
      .contract_version,
    PROCESS_CONTRACT_VERSION
  );

  let mut invalid_v1 = v1.clone();
  invalid_v1["stream"] = json!(false);
  assert!(parse_request(&invalid_v1).is_err());

  let mut v2_disabled = v1.clone();
  v2_disabled["contractVersion"] = json!(PROCESS_CONTRACT_VERSION_V2);
  v2_disabled["stream"] = json!(false);
  let parsed = parse_request(&v2_disabled).expect("explicit Process v2 without streaming");
  assert_eq!(parsed.contract_version, PROCESS_CONTRACT_VERSION_V2);
  assert!(!parsed.stream);

  let mut v2_streaming = v2_disabled;
  v2_streaming["stream"] = json!(true);
  assert!(
    parse_request(&v2_streaming)
      .expect("explicit Process v2 streaming")
      .stream
  );
}

#[test]
fn incremental_stream_redaction_covers_every_secret_split_and_invalid_utf8() {
  let secret = "token-秘密-42";
  let expected = "before-[redacted]-after";
  let source = format!("before-{secret}-after");
  for split in 0..=source.len() {
    let mut redactor = IncrementalTextRedactor::new(&[secret.to_string()]);
    let mut output = String::new();
    if let Some(value) = redactor.push(&source.as_bytes()[..split], false) {
      output.push_str(&value);
    }
    if let Some(value) = redactor.push(&source.as_bytes()[split..], true) {
      output.push_str(&value);
    }
    assert_eq!(output, expected, "secret split at byte {split}");
    assert!(!output.contains(secret));
  }

  let multibyte = "ready 🚀 完了";
  let mut redactor = IncrementalTextRedactor::new(&[]);
  let mut output = String::new();
  for byte in multibyte.as_bytes() {
    if let Some(value) = redactor.push(&[*byte], false) {
      output.push_str(&value);
    }
  }
  if let Some(value) = redactor.push(&[], true) {
    output.push_str(&value);
  }
  assert_eq!(output, multibyte);

  let mut invalid = IncrementalTextRedactor::new(&[]);
  let repaired = invalid
    .push(&[b'a', 0xff, b'b'], true)
    .expect("invalid bytes produce safe text");
  assert_eq!(repaired, "a\u{fffd}b");
}

#[test]
fn durable_metadata_excludes_arguments_environment_and_output() {
  let handler =
    ManagedProcessHandler::new(policy(), ProcessRuntimeMode::Foreground).expect("valid policy");
  let input = request(
    json!({
      "WOML_PROCESS_FIXTURE": "woml-process-fixture-active",
      "WOML_PROCESS_FIXTURE_TEXT": "never-persist-this-secret"
    }),
    128,
    vec![0],
  );
  let metadata = handler.safe_metadata(&input);
  let encoded = serde_json::to_string(&metadata).expect("metadata JSON");
  assert_eq!(metadata.get("executable"), Some(&json!("fixture")));
  assert_eq!(metadata.get("argumentCount"), Some(&json!(3)));
  assert!(!encoded.contains("never-persist-this-secret"));
  assert!(!encoded.contains("--exact"));
  assert!(!metadata.contains_key("args"));
  assert!(!metadata.contains_key("env"));
  assert!(!metadata.contains_key("stdout"));
  assert!(!metadata.contains_key("stderr"));
}

async fn assert_heartbeat_stops(path: &Path) {
  tokio::time::sleep(Duration::from_millis(150)).await;
  let first = std::fs::metadata(path).expect("heartbeat exists").len();
  tokio::time::sleep(Duration::from_millis(150)).await;
  let second = std::fs::metadata(path).expect("heartbeat remains").len();
  assert_eq!(first, second, "a descendant survived containment shutdown");
}

fn tree_request(path: &Path, timeout_ms: u64) -> Value {
  let mut value = request(
    json!({
      "WOML_PROCESS_FIXTURE": "woml-process-fixture-active",
      "WOML_PROCESS_FIXTURE_TREE": "parent",
      "WOML_PROCESS_FIXTURE_HEARTBEAT": path.to_string_lossy()
    }),
    128,
    vec![0],
  );
  value["timeoutMs"] = json!(timeout_ms);
  value
}

#[cfg(any(unix, windows))]
#[tokio::test]
async fn timeout_terminates_child_and_grandchild_tree() {
  let heartbeat = std::env::temp_dir().join(format!(
    "woml-process-timeout-heartbeat-{}",
    uuid::Uuid::new_v4()
  ));
  let handler =
    ManagedProcessHandler::new(policy(), ProcessRuntimeMode::Foreground).expect("valid policy");
  let error = handler
    .execute(
      tree_request(&heartbeat, 250),
      CapabilityCancellationToken::default(),
    )
    .await
    .expect_err("tree times out");
  assert_eq!(error.kind, CapabilityFailureKind::TimedOut);
  assert_eq!(error.code, "WOML_PROCESS_TIMED_OUT");
  assert!(error.ambiguous);
  assert_heartbeat_stops(&heartbeat).await;
  let _ = std::fs::remove_file(heartbeat);
}

#[cfg(any(unix, windows))]
#[tokio::test]
async fn cancellation_terminates_tree_and_cannot_report_success() {
  let heartbeat = std::env::temp_dir().join(format!(
    "woml-process-cancel-heartbeat-{}",
    uuid::Uuid::new_v4()
  ));
  let handler =
    ManagedProcessHandler::new(policy(), ProcessRuntimeMode::Foreground).expect("valid policy");
  let cancellation = CapabilityCancellationToken::default();
  let cancel = cancellation.clone();
  let request = tree_request(&heartbeat, 5_000);
  let execution = tokio::spawn(async move { handler.execute(request, cancellation).await });
  tokio::time::sleep(Duration::from_millis(150)).await;
  cancel.cancel();
  let error = execution
    .await
    .expect("execution task")
    .expect_err("tree is cancelled");
  assert_eq!(error.kind, CapabilityFailureKind::Cancelled);
  assert_eq!(error.code, "WOML_PROCESS_CANCELLED");
  assert!(error.ambiguous);
  assert_heartbeat_stops(&heartbeat).await;
  let _ = std::fs::remove_file(heartbeat);
}

#[cfg(any(unix, windows))]
#[tokio::test]
async fn concurrency_ceiling_rejects_excess_processes_without_queueing() {
  let heartbeat = std::env::temp_dir().join(format!(
    "woml-process-concurrency-heartbeat-{}",
    uuid::Uuid::new_v4()
  ));
  let handler =
    ManagedProcessHandler::new(policy(), ProcessRuntimeMode::Foreground).expect("valid policy");
  let cancellation = CapabilityCancellationToken::default();
  let running_handler = handler.clone();
  let running_cancellation = cancellation.clone();
  let running_request = tree_request(&heartbeat, 5_000);
  let running = tokio::spawn(async move {
    running_handler
      .execute(running_request, running_cancellation)
      .await
  });
  for _ in 0..100 {
    if heartbeat.exists() {
      break;
    }
    tokio::time::sleep(Duration::from_millis(5)).await;
  }
  assert!(heartbeat.exists(), "first process did not start");
  let error = handler
    .execute(
      request(
        json!({ "WOML_PROCESS_FIXTURE": "woml-process-fixture-active" }),
        128,
        vec![0],
      ),
      CapabilityCancellationToken::default(),
    )
    .await
    .expect_err("second process exceeds the policy ceiling");
  assert_eq!(error.code, "WOML_PROCESS_CONCURRENCY_LIMIT");
  cancellation.cancel();
  let _ = running.await;
  assert_heartbeat_stops(&heartbeat).await;
  let _ = std::fs::remove_file(heartbeat);
}

#[cfg(any(unix, windows))]
#[tokio::test]
async fn dropped_handler_future_closes_the_process_tree() {
  let heartbeat = std::env::temp_dir().join(format!(
    "woml-process-host-stop-heartbeat-{}",
    uuid::Uuid::new_v4()
  ));
  let handler =
    ManagedProcessHandler::new(policy(), ProcessRuntimeMode::Foreground).expect("valid policy");
  let request = tree_request(&heartbeat, 5_000);
  let execution = tokio::spawn(async move {
    handler
      .execute(request, CapabilityCancellationToken::default())
      .await
  });
  tokio::time::sleep(Duration::from_millis(150)).await;
  execution.abort();
  let _ = execution.await;
  assert_heartbeat_stops(&heartbeat).await;
  let _ = std::fs::remove_file(heartbeat);
}
