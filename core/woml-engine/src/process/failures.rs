//! Body-free process failures with the existing author-facing corrective actions and ambiguity classification.

use crate::CapabilityFailure;
use crate::CapabilityFailureKind;
use serde_json::Map;
use serde_json::Value;

pub(in crate::process) fn invalid_input(message: &str) -> CapabilityFailure {
  failure(
    CapabilityFailureKind::InvalidInput,
    "WOML_PROCESS_INPUT_INVALID",
    message,
    false,
    false,
    None,
  )
}

pub(in crate::process) fn rejected(code: &str, message: &str) -> CapabilityFailure {
  failure(
    CapabilityFailureKind::ServiceRejected,
    code,
    message,
    false,
    false,
    None,
  )
}

pub(in crate::process) fn output_failure() -> CapabilityFailure {
  failure(
    CapabilityFailureKind::HandlerCrashed,
    "WOML_PROCESS_OUTPUT_FAILED",
    "The runtime could not safely capture process output.",
    false,
    true,
    None,
  )
}

pub(in crate::process) fn failure(
  kind: CapabilityFailureKind,
  code: &str,
  message: &str,
  retryable: bool,
  ambiguous: bool,
  details: Option<Map<String, Value>>,
) -> CapabilityFailure {
  CapabilityFailure {
    kind,
    code: code.to_string(),
    message: format!("{message} Corrective action: {}", corrective_action(code)),
    retryable,
    ambiguous,
    details,
  }
}

fn corrective_action(code: &str) -> &'static str {
  match code {
    "WOML_PROCESS_EXECUTABLE_DENIED" => {
      "add the intended executable alias to the operator-owned process policy"
    }
    "WOML_PROCESS_EXECUTABLE_NOT_FOUND" | "WOML_PROCESS_EXECUTABLE_CHANGED" => {
      "restore the reviewed executable and restart the WOML runtime so it can preflight it again"
    }
    "WOML_PROCESS_CWD_NOT_FOUND" | "WOML_PROCESS_CWD_DENIED" => {
      "use an existing project-relative directory allowed by the process policy"
    }
    "WOML_PROCESS_ENV_DENIED" => {
      "remove the variable or allow its name in the operator-owned process policy"
    }
    "WOML_PROCESS_OUTPUT_LIMIT_DENIED" => {
      "lower output.maxBytes or raise the policy ceiling after review"
    }
    "WOML_PROCESS_CONCURRENCY_LIMIT" => {
      "wait for another managed process to finish or raise the reviewed concurrency limit"
    }
    "WOML_PROCESS_SPAWN_FAILED" => {
      "verify executable permissions and host resources, then start a new run"
    }
    "WOML_PROCESS_TIMED_OUT" => {
      "optimize the command or increase its timeout within the policy ceiling"
    }
    "WOML_PROCESS_CANCELLED" => {
      "inspect possible partial external effects before starting a new run"
    }
    "WOML_PROCESS_EXIT_CODE" => {
      "inspect the tool result or explicitly accept that exit code when it represents success"
    }
    "WOML_PROCESS_SIGNALLED" => {
      "inspect host resource pressure and possible partial effects before retrying"
    }
    "WOML_PROCESS_INPUT_INVALID" => "correct the Process v1 request fields and bounds",
    _ => "inspect the runtime diagnostics and start a new run only when the cause is understood",
  }
}
