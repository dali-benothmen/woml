//! Operational process stream state, authenticated-store publication, bounded records/closure, and explicit artifact rollover.

mod drain;
mod redaction;

pub(in crate::process) use drain::drain_output;
pub(in crate::process) use redaction::{
  redact_and_bound_output, sensitive_environment_values, IncrementalTextRedactor,
};

use crate::process::failure;
use crate::process::OutputMode;
use crate::CapabilityFailure;
use crate::CapabilityFailureKind;
use crate::LazyOperationalOutputStore;
use crate::OperationalBodyV1;
use crate::OperationalCloseStatusV1;
use crate::OperationalIdentityV1;
use crate::OperationalOutputStore;
use crate::OperationalRecordDraftV1;
use crate::OperationalSourceV1;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::sync::Notify;

#[derive(Debug)]
pub(in crate::process) struct CapturedOutput {
  pub(in crate::process) text: String,
  pub(in crate::process) bytes: u64,
  pub(in crate::process) truncated: bool,
}

#[derive(Debug, Default)]
pub(in crate::process) struct ProcessStreamState {
  pub(in crate::process) start_sequence: Option<u64>,
  pub(in crate::process) end_sequence: Option<u64>,
  pub(in crate::process) truncated: bool,
  pub(in crate::process) retained_bytes: u64,
  pub(in crate::process) artifact_writer: Option<crate::artifacts::OperationalArtifactWriter>,
  pub(in crate::process) artifact: Option<crate::ArtifactReferenceV1>,
}

#[derive(Debug, Default)]
pub(in crate::process) struct ProcessStreamFailure {
  pub(in crate::process) failure: Mutex<Option<CapabilityFailure>>,
  pub(in crate::process) notify: Notify,
}

impl ProcessStreamFailure {
  pub(in crate::process) fn report(&self, failure: CapabilityFailure) {
    if let Ok(mut stored) = self.failure.lock() {
      if stored.is_none() {
        *stored = Some(failure);
        self.notify.notify_one();
      }
    }
  }

  pub(in crate::process) fn take(&self) -> Option<CapabilityFailure> {
    self.failure.lock().ok()?.take()
  }
}

#[derive(Debug, Clone)]
pub(in crate::process) struct ProcessStreamTarget {
  pub(in crate::process) store: Arc<LazyOperationalOutputStore>,
  pub(in crate::process) identity: OperationalIdentityV1,
  pub(in crate::process) resolved_secrets: Arc<BTreeMap<String, String>>,
  pub(in crate::process) artifact_handler: Option<Arc<crate::ManagedArtifactPutHandler>>,
}

#[derive(Debug, Clone)]
pub(in crate::process) struct ActiveProcessStream {
  pub(in crate::process) store: Arc<OperationalOutputStore>,
  pub(in crate::process) identity: OperationalIdentityV1,
  pub(in crate::process) state: Arc<Mutex<ProcessStreamState>>,
  pub(in crate::process) failure: Arc<ProcessStreamFailure>,
  pub(in crate::process) max_retained_bytes: u64,
}

pub(in crate::process) fn append_process_text(
  target: &ActiveProcessStream,
  source: OperationalSourceV1,
  text: &str,
) -> Result<(), CapabilityFailure> {
  let mut remaining = text;
  while !remaining.is_empty() {
    let mut end = remaining.len().min(crate::MAX_OPERATIONAL_CHUNK_BYTES);
    while end > 0 && !remaining.is_char_boundary(end) {
      end -= 1;
    }
    if end == 0 {
      return Err(output_authority_failure(true));
    }
    append_process_output(target, source, remaining[..end].to_string())?;
    remaining = &remaining[end..];
  }
  Ok(())
}

fn append_process_output(
  target: &ActiveProcessStream,
  source: OperationalSourceV1,
  text: String,
) -> Result<(), CapabilityFailure> {
  let byte_count = text.len() as u64;
  let mut state = target
    .state
    .lock()
    .map_err(|_| output_authority_failure(true))?;
  if let Some(writer) = state.artifact_writer.as_mut() {
    let source_name = match source {
      OperationalSourceV1::Stdout => "stdout",
      OperationalSourceV1::Stderr => "stderr",
      OperationalSourceV1::Script => "script",
      OperationalSourceV1::Runtime => "runtime",
    };
    let _ = writer.append_redacted(source_name, &text);
  }
  if state.truncated {
    return Ok(());
  }
  if state.retained_bytes.saturating_add(byte_count) > target.max_retained_bytes {
    let record = target
      .store
      .append(OperationalRecordDraftV1 {
        identity: target.identity.clone(),
        body: OperationalBodyV1::Truncated {
          source,
          dropped_bytes: byte_count,
          reason: "inline_run_budget".to_string(),
        },
      })
      .map_err(|_| output_authority_failure(true))?;
    state.start_sequence.get_or_insert(record.sequence);
    state.end_sequence = Some(record.sequence);
    state.truncated = true;
    return Ok(());
  }
  let record = target
    .store
    .append(OperationalRecordDraftV1 {
      identity: target.identity.clone(),
      body: OperationalBodyV1::Output {
        source,
        level: None,
        text,
        byte_count,
      },
    })
    .map_err(|_| output_authority_failure(true))?;
  state.retained_bytes = state.retained_bytes.saturating_add(byte_count);
  state.start_sequence.get_or_insert(record.sequence);
  state.end_sequence = Some(record.sequence);
  Ok(())
}

pub(in crate::process) fn close_process_stream(
  stream: Option<&ActiveProcessStream>,
  status: OperationalCloseStatusV1,
  reason_code: Option<&str>,
) -> Result<(), CapabilityFailure> {
  let Some(stream) = stream else {
    return Ok(());
  };
  let (writer, publish) = {
    let mut state = stream
      .state
      .lock()
      .map_err(|_| output_authority_failure(true))?;
    (state.artifact_writer.take(), state.truncated)
  };
  if let Some(writer) = writer {
    if let Ok(Some(artifact)) = writer.finish(publish) {
      let record = stream
        .store
        .append(OperationalRecordDraftV1 {
          identity: stream.identity.clone(),
          body: OperationalBodyV1::Artifact {
            artifact_id: artifact.id.clone(),
            name: artifact.name.clone(),
            size: artifact.size,
            sha256: artifact.sha256.clone(),
            content_type: artifact.content_type.clone(),
          },
        })
        .map_err(|_| output_authority_failure(true))?;
      let mut state = stream
        .state
        .lock()
        .map_err(|_| output_authority_failure(true))?;
      state.start_sequence.get_or_insert(record.sequence);
      state.end_sequence = Some(record.sequence);
      state.artifact = Some(artifact);
    }
  }
  let record = stream
    .store
    .append(OperationalRecordDraftV1 {
      identity: stream.identity.clone(),
      body: OperationalBodyV1::Closed {
        status,
        reason_code: reason_code.map(str::to_string),
      },
    })
    .map_err(|_| output_authority_failure(true))?;
  let mut state = stream
    .state
    .lock()
    .map_err(|_| output_authority_failure(true))?;
  state.start_sequence.get_or_insert(record.sequence);
  state.end_sequence = Some(record.sequence);
  Ok(())
}

pub(in crate::process) fn output_authority_failure(ambiguous: bool) -> CapabilityFailure {
  failure(
    CapabilityFailureKind::TransportFailed,
    "WOML_PROCESS_OUTPUT_AUTHORITY_FAILED",
    "The operational-output authority could not accept streamed process output.",
    false,
    ambiguous,
    None,
  )
}

pub(in crate::process) fn output_mode_name(mode: OutputMode) -> &'static str {
  match mode {
    OutputMode::Capture => "capture",
    OutputMode::Discard => "discard",
  }
}
