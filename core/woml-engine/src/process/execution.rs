//! Supervised process lifecycle, literal arguments, concurrent pipe tasks, deadline/cancellation, and bounded tree/drain cleanup.

use crate::process::close_process_stream;
use crate::process::drain_output;
use crate::process::exit_signal;
use crate::process::failure;
use crate::process::output_authority_failure;
use crate::process::output_failure;
use crate::process::prepare_process_containment;
use crate::process::prepare_process_environment;
use crate::process::redact_and_bound_output;
use crate::process::safe_duration;
use crate::process::sensitive_environment_values;
use crate::process::ActiveProcessStream;
use crate::process::AuthorizedProcess;
use crate::process::CapturedOutput;
use crate::process::PreparedProcessPolicy;
use crate::process::ProcessContainment;
use crate::process::ProcessRunResultV1;
use crate::process::ProcessRunResultV2;
use crate::process::ProcessStreamFailure;
use crate::process::ProcessStreamState;
use crate::process::ProcessStreamTarget;
use crate::process::PROCESS_CONTRACT;
use crate::process::PROCESS_CONTRACT_VERSION;
use crate::process::PROCESS_CONTRACT_VERSION_V2;
use crate::CapabilityCancellationToken;
use crate::CapabilityFailure;
use crate::CapabilityFailureKind;
use crate::OperationalCloseStatusV1;
use crate::OperationalSourceV1;
use serde_json::json;
use serde_json::Map;
use serde_json::Value;
use std::io;
use std::process::ExitStatus;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;
use tokio::process::Child;
use tokio::process::Command;
use tokio::sync::OwnedSemaphorePermit;
use tokio::task::JoinHandle;

pub(in crate::process) const PROCESS_TERMINATION_GRACE: Duration = Duration::from_millis(250);

const PROCESS_PIPE_DRAIN_GRACE: Duration = Duration::from_secs(1);

pub(in crate::process) async fn execute_process(
  authorized: AuthorizedProcess,
  policy: Arc<PreparedProcessPolicy>,
  cancellation: CapabilityCancellationToken,
  _permit: OwnedSemaphorePermit,
  stream_target: Option<ProcessStreamTarget>,
) -> Result<Value, CapabilityFailure> {
  let started = Instant::now();
  let request = authorized.request;
  // Process output is ephemeral, but it is still user-visible. Operator-
  // inherited values are sensitive at this boundary. The Bun worker separately
  // redacts resolved WOML secrets because secret provenance intentionally does
  // not cross the Process v1 contract.
  let sensitive_values = sensitive_environment_values(
    &policy,
    &request.env,
    stream_target
      .as_ref()
      .map(|target| target.resolved_secrets.as_ref()),
  );
  let redaction_lookahead = sensitive_values
    .iter()
    .map(String::len)
    .max()
    .unwrap_or(0)
    .saturating_sub(1);
  let stream = match stream_target {
    Some(target) => {
      let artifact_writer = target.artifact_handler.as_ref().and_then(|handler| {
        handler
          .begin_operational_output(
            &target.identity.workflow_id,
            &target.identity.run_id,
            target.identity.node_id.as_deref()?,
            target.identity.attempt?,
            target.identity.invocation_id.as_deref()?,
          )
          .ok()
          .flatten()
      });
      Some(ActiveProcessStream {
        store: target
          .store
          .activate()
          .map_err(|_| output_authority_failure(false))?,
        identity: target.identity,
        state: Arc::new(Mutex::new(ProcessStreamState {
          artifact_writer,
          ..ProcessStreamState::default()
        })),
        failure: Arc::new(ProcessStreamFailure::default()),
        max_retained_bytes: policy.limits.max_stream_bytes as u64,
      })
    }
    None => None,
  };
  let mut command = Command::new(&authorized.executable);
  prepare_process_environment(&mut command).map_err(|_| {
    failure(
      CapabilityFailureKind::ServiceRejected,
      "WOML_PROCESS_SPAWN_FAILED",
      "The approved executable could not be started.",
      true,
      false,
      None,
    )
  })?;
  command
    .args(&request.args)
    .current_dir(&authorized.cwd)
    .envs(&policy.inherited_environment)
    .envs(&request.env)
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(true);
  prepare_process_containment(&mut command);
  let mut child = command.spawn().map_err(|_| {
    failure(
      CapabilityFailureKind::ServiceRejected,
      "WOML_PROCESS_SPAWN_FAILED",
      "The approved executable could not be started.",
      true,
      false,
      None,
    )
  })?;
  let mut containment = match ProcessContainment::attach(&child) {
    Ok(containment) => containment,
    Err(_) => {
      let _ = child.kill().await;
      let _ = child.wait().await;
      return Err(failure(
        CapabilityFailureKind::HandlerCrashed,
        "WOML_PROCESS_CONTAINMENT_FAILED",
        "The process could not be attached to its operating-system containment boundary.",
        false,
        true,
        None,
      ));
    }
  };
  let stdout = child.stdout.take().ok_or_else(|| {
    failure(
      CapabilityFailureKind::HandlerCrashed,
      "WOML_PROCESS_PIPE_UNAVAILABLE",
      "The process stdout pipe could not be initialized.",
      false,
      true,
      None,
    )
  })?;
  let stderr = child.stderr.take().ok_or_else(|| {
    failure(
      CapabilityFailureKind::HandlerCrashed,
      "WOML_PROCESS_PIPE_UNAVAILABLE",
      "The process stderr pipe could not be initialized.",
      false,
      true,
      None,
    )
  })?;
  let stdout_task = tokio::spawn(drain_output(
    stdout,
    request.output.stdout,
    request.output.max_bytes,
    redaction_lookahead,
    sensitive_values.clone(),
    stream
      .clone()
      .map(|target| (target, OperationalSourceV1::Stdout)),
  ));
  let stderr_task = tokio::spawn(drain_output(
    stderr,
    request.output.stderr,
    request.output.max_bytes,
    redaction_lookahead,
    sensitive_values.clone(),
    stream
      .clone()
      .map(|target| (target, OperationalSourceV1::Stderr)),
  ));
  let timeout = tokio::time::sleep(Duration::from_millis(request.timeout_ms));
  tokio::pin!(timeout);
  let status = tokio::select! {
    biased;
    _ = cancellation.cancelled() => {
      let _ = terminate_process_tree(&mut child, &mut containment).await;
      let _ = collect_output_task(stdout_task).await;
      let _ = collect_output_task(stderr_task).await;
      close_process_stream(stream.as_ref(), OperationalCloseStatusV1::Cancelled, Some("WOML_PROCESS_CANCELLED"))?;
      return Err(failure(
        CapabilityFailureKind::Cancelled,
        "WOML_PROCESS_CANCELLED",
        "The process was cancelled.",
        false,
        true,
        Some(safe_duration(started)),
      ));
    }
    _ = &mut timeout => {
      let _ = terminate_process_tree(&mut child, &mut containment).await;
      let _ = collect_output_task(stdout_task).await;
      let _ = collect_output_task(stderr_task).await;
      close_process_stream(stream.as_ref(), OperationalCloseStatusV1::TimedOut, Some("WOML_PROCESS_TIMED_OUT"))?;
      return Err(failure(
        CapabilityFailureKind::TimedOut,
        "WOML_PROCESS_TIMED_OUT",
        "The process exceeded its approved timeout.",
        false,
        true,
        Some(safe_duration(started)),
      ));
    }
    failure = wait_for_process_stream_failure(stream.as_ref()), if stream.is_some() => {
      let _ = terminate_process_tree(&mut child, &mut containment).await;
      let _ = collect_output_task(stdout_task).await;
      let _ = collect_output_task(stderr_task).await;
      let _ = close_process_stream(
        stream.as_ref(),
        OperationalCloseStatusV1::Interrupted,
        Some("WOML_PROCESS_OUTPUT_AUTHORITY_FAILED"),
      );
      return Err(failure);
    }
    status = child.wait() => status.map_err(|_| failure(
      CapabilityFailureKind::HandlerCrashed,
      "WOML_PROCESS_WAIT_FAILED",
      "The runtime could not observe the process exit status.",
      false,
      true,
      Some(safe_duration(started)),
    ))?,
  };
  // Process v1 never permits detached work. If the leader exited after
  // starting descendants, close the containment boundary before waiting for
  // inherited stdout/stderr handles to reach EOF.
  containment.close_remaining_tree().await;
  let stdout = redact_and_bound_output(
    collect_output_task(stdout_task).await?,
    &sensitive_values,
    request.output.max_bytes,
  );
  let stderr = redact_and_bound_output(
    collect_output_task(stderr_task).await?,
    &sensitive_values,
    request.output.max_bytes,
  );
  let exit_code = status.code();
  let signal = exit_signal(&status);
  let duration_ms = started.elapsed().as_secs_f64() * 1_000.0;
  let stream_status = if exit_code.is_some_and(|code| request.accept_exit_codes.contains(&code)) {
    OperationalCloseStatusV1::Succeeded
  } else {
    OperationalCloseStatusV1::Failed
  };
  close_process_stream(
    stream.as_ref(),
    stream_status,
    if matches!(stream_status, OperationalCloseStatusV1::Succeeded) {
      None
    } else if exit_code.is_none() {
      Some("WOML_PROCESS_SIGNALLED")
    } else {
      Some("WOML_PROCESS_EXIT_CODE")
    },
  )?;
  let stream_state = stream.as_ref().and_then(|stream| {
    stream.state.lock().ok().map(|state| {
      (
        state.start_sequence,
        state.end_sequence,
        state.truncated,
        state.artifact.clone(),
      )
    })
  });
  let result = ProcessRunResultV1 {
    contract: PROCESS_CONTRACT,
    contract_version: PROCESS_CONTRACT_VERSION,
    kind: "result",
    exit_code,
    signal: signal.clone(),
    stdout: stdout.text,
    stderr: stderr.text,
    stdout_bytes: stdout.bytes,
    stderr_bytes: stderr.bytes,
    stdout_truncated: stdout.truncated,
    stderr_truncated: stderr.truncated,
    duration_ms,
  };
  if exit_code.is_none() || !request.accept_exit_codes.contains(&exit_code.unwrap_or(-1)) {
    let details = Map::from_iter([
      (
        "exitCode".to_string(),
        exit_code.map_or(Value::Null, Value::from),
      ),
      (
        "signal".to_string(),
        signal.map_or(Value::Null, Value::String),
      ),
      ("durationMs".to_string(), json!(duration_ms)),
      ("stdoutBytes".to_string(), json!(stdout.bytes)),
      ("stderrBytes".to_string(), json!(stderr.bytes)),
      ("stdoutTruncated".to_string(), json!(stdout.truncated)),
      ("stderrTruncated".to_string(), json!(stderr.truncated)),
    ]);
    let (code, message, ambiguous) = if exit_code.is_none() {
      (
        "WOML_PROCESS_SIGNALLED",
        "The process was terminated by a signal or platform-equivalent condition.",
        true,
      )
    } else {
      (
        "WOML_PROCESS_EXIT_CODE",
        "The process exited with a status not accepted by the workflow.",
        false,
      )
    };
    return Err(failure(
      CapabilityFailureKind::ServiceRejected,
      code,
      message,
      false,
      ambiguous,
      Some(details),
    ));
  }
  if request.contract_version == PROCESS_CONTRACT_VERSION_V2 {
    let (stream_start_sequence, stream_end_sequence, stream_truncated, stream_artifact) =
      stream_state.unwrap_or((None, None, false, None));
    serde_json::to_value(ProcessRunResultV2 {
      contract: PROCESS_CONTRACT,
      contract_version: PROCESS_CONTRACT_VERSION_V2,
      kind: "result",
      exit_code: result.exit_code,
      signal: result.signal,
      stdout: result.stdout,
      stderr: result.stderr,
      stdout_bytes: result.stdout_bytes,
      stderr_bytes: result.stderr_bytes,
      stdout_truncated: result.stdout_truncated,
      stderr_truncated: result.stderr_truncated,
      duration_ms: result.duration_ms,
      streamed: request.stream,
      stream_start_sequence,
      stream_end_sequence,
      stream_truncated,
      stream_artifact: stream_artifact
        .map(|artifact| serde_json::to_value(artifact).map_err(|_| output_failure()))
        .transpose()?,
    })
    .map_err(|_| output_failure())
  } else {
    serde_json::to_value(result).map_err(|_| output_failure())
  }
}

async fn collect_output_task(
  mut task: JoinHandle<Result<CapturedOutput, CapabilityFailure>>,
) -> Result<CapturedOutput, CapabilityFailure> {
  match tokio::time::timeout(PROCESS_PIPE_DRAIN_GRACE, &mut task).await {
    Ok(joined) => joined.map_err(|_| output_failure())?,
    Err(_) => {
      task.abort();
      let _ = task.await;
      Err(failure(
        CapabilityFailureKind::HandlerCrashed,
        "WOML_PROCESS_PIPE_DRAIN_TIMED_OUT",
        "The process output pipes did not close after the containment boundary terminated.",
        false,
        true,
        None,
      ))
    }
  }
}

async fn wait_for_process_stream_failure(
  stream: Option<&ActiveProcessStream>,
) -> CapabilityFailure {
  let Some(stream) = stream else {
    return std::future::pending().await;
  };
  loop {
    if let Some(failure) = stream.failure.take() {
      return failure;
    }
    stream.failure.notify.notified().await;
  }
}

async fn terminate_process_tree(
  child: &mut Child,
  containment: &mut ProcessContainment,
) -> io::Result<ExitStatus> {
  containment.graceful_terminate()?;
  tokio::time::sleep(PROCESS_TERMINATION_GRACE).await;
  containment.force_terminate()?;
  child.wait().await
}
