//! Incremental split-secret and UTF-8 redaction, sensitive environment values, and bounded final captured text.

use crate::process::CapturedOutput;
use crate::process::PreparedProcessPolicy;
use std::collections::BTreeMap;

pub(in crate::process) fn sensitive_environment_values(
  policy: &PreparedProcessPolicy,
  request_environment: &BTreeMap<String, String>,
  resolved_secrets: Option<&BTreeMap<String, String>>,
) -> Vec<String> {
  let mut values = policy
    .inherited_environment
    .values()
    .filter(|value| !value.is_empty())
    .cloned()
    .collect::<Vec<_>>();
  values.extend(
    request_environment
      .values()
      .filter(|value| !value.is_empty())
      .cloned(),
  );
  if let Some(resolved_secrets) = resolved_secrets {
    values.extend(
      resolved_secrets
        .values()
        .filter(|value| !value.is_empty())
        .cloned(),
    );
  }
  values.sort_by_key(|value| std::cmp::Reverse(value.len()));
  values.dedup();
  values
}

#[derive(Debug)]
pub(in crate::process) struct IncrementalTextRedactor {
  pub(in crate::process) pending_bytes: Vec<u8>,
  pub(in crate::process) pending_text: String,
  pub(in crate::process) secrets: Vec<String>,
  pub(in crate::process) lookahead: usize,
}

impl IncrementalTextRedactor {
  pub(in crate::process) fn new(secrets: &[String]) -> Self {
    Self {
      pending_bytes: Vec::new(),
      pending_text: String::new(),
      secrets: secrets.to_vec(),
      lookahead: secrets
        .iter()
        .map(String::len)
        .max()
        .unwrap_or(1)
        .saturating_sub(1),
    }
  }

  pub(in crate::process) fn push(&mut self, bytes: &[u8], eof: bool) -> Option<String> {
    self.pending_bytes.extend_from_slice(bytes);
    loop {
      match std::str::from_utf8(&self.pending_bytes) {
        Ok(value) => {
          self.pending_text.push_str(value);
          self.pending_bytes.clear();
          break;
        }
        Err(error) => {
          let valid = error.valid_up_to();
          if valid > 0 {
            let prefix = String::from_utf8_lossy(&self.pending_bytes[..valid]);
            self.pending_text.push_str(&prefix);
            self.pending_bytes.drain(..valid);
          }
          match error.error_len() {
            Some(length) => {
              self.pending_text.push('\u{fffd}');
              self
                .pending_bytes
                .drain(..length.min(self.pending_bytes.len()));
            }
            None => break,
          }
        }
      }
    }
    if eof && !self.pending_bytes.is_empty() {
      self
        .pending_text
        .push_str(&String::from_utf8_lossy(&self.pending_bytes));
      self.pending_bytes.clear();
    }
    for secret in &self.secrets {
      self.pending_text = self.pending_text.replace(secret, "[redacted]");
    }
    let emit_bytes = if eof {
      self.pending_text.len()
    } else {
      self.pending_text.len().saturating_sub(self.lookahead)
    };
    let mut boundary = emit_bytes;
    while boundary > 0 && !self.pending_text.is_char_boundary(boundary) {
      boundary -= 1;
    }
    if boundary == 0 {
      return None;
    }
    let emitted = self.pending_text[..boundary].to_string();
    self.pending_text.drain(..boundary);
    (!emitted.is_empty()).then_some(emitted)
  }
}

pub(in crate::process) fn redact_and_bound_output(
  mut output: CapturedOutput,
  sensitive_values: &[String],
  max_bytes: usize,
) -> CapturedOutput {
  for value in sensitive_values {
    output.text = output.text.replace(value, "[redacted]");
  }
  if output.text.len() > max_bytes {
    let mut end = max_bytes;
    while !output.text.is_char_boundary(end) {
      end = end.saturating_sub(1);
    }
    output.text.truncate(end);
    output.truncated = true;
  }
  output
}
