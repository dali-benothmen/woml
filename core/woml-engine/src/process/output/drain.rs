//! Concurrent pipe draining and bounded capture. Reading continues after the retention budget is full to avoid blocking the child.

use crate::process::append_process_text;
use crate::process::output_failure;
use crate::process::ActiveProcessStream;
use crate::process::CapturedOutput;
use crate::process::IncrementalTextRedactor;
use crate::process::OutputMode;
use crate::CapabilityFailure;
use crate::OperationalSourceV1;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;

pub(in crate::process) async fn drain_output<R: AsyncRead + Unpin>(
  mut reader: R,
  mode: OutputMode,
  max_bytes: usize,
  redaction_lookahead: usize,
  sensitive_values: Vec<String>,
  stream: Option<(ActiveProcessStream, OperationalSourceV1)>,
) -> Result<CapturedOutput, CapabilityFailure> {
  let capture_limit = max_bytes.saturating_add(redaction_lookahead);
  let mut captured = Vec::with_capacity(match mode {
    OutputMode::Capture => capture_limit.min(64 * 1024),
    OutputMode::Discard => 0,
  });
  let mut total = 0_u64;
  let mut buffer = [0_u8; 8192];
  let mut stream_decoder = stream
    .as_ref()
    .map(|_| IncrementalTextRedactor::new(&sensitive_values));
  loop {
    let read = reader
      .read(&mut buffer)
      .await
      .map_err(|_| output_failure())?;
    if read == 0 {
      break;
    }
    total = total.saturating_add(read as u64);
    if matches!(mode, OutputMode::Capture) && captured.len() < capture_limit {
      let remaining = capture_limit - captured.len();
      captured.extend_from_slice(&buffer[..read.min(remaining)]);
    }
    if let (Some((target, source)), Some(decoder)) = (&stream, &mut stream_decoder) {
      if let Some(text) = decoder.push(&buffer[..read], false) {
        if let Err(error) = append_process_text(target, *source, &text) {
          target.failure.report(error.clone());
          return Err(error);
        }
      }
    }
  }
  if let (Some((target, source)), Some(decoder)) = (&stream, &mut stream_decoder) {
    if let Some(text) = decoder.push(&[], true) {
      if let Err(error) = append_process_text(target, *source, &text) {
        target.failure.report(error.clone());
        return Err(error);
      }
    }
  }
  Ok(CapturedOutput {
    text: String::from_utf8_lossy(&captured).into_owned(),
    bytes: total,
    truncated: matches!(mode, OutputMode::Capture) && total > max_bytes as u64,
  })
}
