//! Version-aware interruption and primary-failure payload construction without replay permission.

use crate::event::ParallelFailurePolicy;
use crate::event::RunFailedData;
use crate::event::RunFailedDataV1;
use crate::event::RunFailedDataV2;
use crate::AttemptFailure;
use crate::AttemptFailureKind;
use crate::RUN_EVENT_SCHEMA_VERSION_V1;
use crate::RUN_EVENT_SCHEMA_VERSION_V2;
use crate::RUN_EVENT_SCHEMA_VERSION_V3;
use crate::RUN_EVENT_SCHEMA_VERSION_V4;
use crate::RUN_EVENT_SCHEMA_VERSION_V5;
use crate::RUN_EVENT_SCHEMA_VERSION_V6;
use crate::RUN_EVENT_SCHEMA_VERSION_V7;
use crate::RUN_EVENT_SCHEMA_VERSION_V8;
use crate::RUN_EVENT_SCHEMA_VERSION_V9;

pub(in crate::durable) fn interrupted_failure() -> AttemptFailure {
  AttemptFailure {
    kind: AttemptFailureKind::Interrupted,
    code: AttemptFailureKind::Interrupted.code().to_string(),
    message: "Recovery found a started attempt without a terminal event.".to_string(),
    details: None,
    ..AttemptFailure::legacy_defaults()
  }
}

pub(in crate::durable) fn interrupted_operation_failure(
  operation: &crate::projection::OperationProjection,
  observed: bool,
  loop_owned: bool,
) -> crate::CapabilityFailure {
  let process = operation.capability == "process" && operation.operation == "run";
  let (code, message) = if process {
    (
      "WOML_PROCESS_INTERRUPTED",
      "Recovery found an approved process start without a durable terminal outcome; the process result is ambiguous and WOML will not replay it.",
    )
  } else if observed {
    (
      "WOML_NATIVE_FETCH_INTERRUPTED",
      "Recovery found native Fetch without a durable terminal observation; its outcome is ambiguous and it will not be replayed.",
    )
  } else if loop_owned {
    (
      "WOML_CAPABILITY_INTERRUPTED",
      "Recovery found a loop-owned operation without a durable terminal event; its outcome is ambiguous and it will not be replayed.",
    )
  } else {
    (
      "WOML_CAPABILITY_INTERRUPTED",
      "Recovery found a managed operation without a durable terminal event; it will not be replayed.",
    )
  };
  crate::CapabilityFailure {
    kind: crate::CapabilityFailureKind::Interrupted,
    code: code.to_string(),
    message: message.to_string(),
    retryable: false,
    ambiguous: true,
    details: None,
  }
}

pub(in crate::durable) fn interrupted_operation_metadata(
  operation: &crate::projection::OperationProjection,
) -> serde_json::Map<String, serde_json::Value> {
  let mut metadata = operation.metadata.clone();
  if operation.capability == "process" && operation.operation == "run" {
    metadata.insert(
      "outcome".to_string(),
      serde_json::Value::String("interrupted".to_string()),
    );
  }
  metadata
}

pub(in crate::durable) fn parallel_failure_message(
  policy: ParallelFailurePolicy,
  failed_count: usize,
) -> String {
  match policy {
    ParallelFailurePolicy::FailFast => {
      "A parallel child failed; active siblings were cancelled.".to_string()
    }
    ParallelFailurePolicy::WaitAll if failed_count == 1 => "One parallel child failed.".to_string(),
    ParallelFailurePolicy::WaitAll => format!("{failed_count} parallel children failed."),
  }
}

pub(in crate::durable) fn attempt_run_failed_data(
  event_schema_version: u32,
  node_id: String,
  attempt: u32,
  invocation_id: String,
  failure: AttemptFailure,
) -> RunFailedData {
  match event_schema_version {
    RUN_EVENT_SCHEMA_VERSION_V1 => RunFailedData::V1(RunFailedDataV1 {
      node_id: Some(node_id),
      attempt: Some(attempt),
      invocation_id: Some(invocation_id),
      failure,
    }),
    RUN_EVENT_SCHEMA_VERSION_V2
    | RUN_EVENT_SCHEMA_VERSION_V3
    | RUN_EVENT_SCHEMA_VERSION_V4
    | RUN_EVENT_SCHEMA_VERSION_V5
    | RUN_EVENT_SCHEMA_VERSION_V6
    | RUN_EVENT_SCHEMA_VERSION_V7
    | RUN_EVENT_SCHEMA_VERSION_V8
    | RUN_EVENT_SCHEMA_VERSION_V9
    | crate::RUN_EVENT_SCHEMA_VERSION_V10
    | crate::RUN_EVENT_SCHEMA_VERSION_V11
    | crate::RUN_EVENT_SCHEMA_VERSION_V12
    | crate::RUN_EVENT_SCHEMA_VERSION_V13
    | crate::RUN_EVENT_SCHEMA_VERSION_V14
    | crate::RUN_EVENT_SCHEMA_VERSION_V15
    | crate::RUN_EVENT_SCHEMA_VERSION_V16
    | crate::RUN_EVENT_SCHEMA_VERSION_V17
    | crate::RUN_EVENT_SCHEMA_VERSION_V18
    | crate::RUN_EVENT_SCHEMA_VERSION_V19
    | crate::RUN_EVENT_SCHEMA_VERSION_V20 => RunFailedData::V2(RunFailedDataV2::Attempt {
      node_id,
      attempt,
      invocation_id,
      failure,
    }),
    _ => unreachable!("event versions are validated before recovery"),
  }
}
