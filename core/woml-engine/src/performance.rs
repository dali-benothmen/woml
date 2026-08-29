//! Internal, opt-in performance tracing for the repository investigation.
//!
//! This is deliberately environment-gated and is not part of WOML's public
//! workflow, event, or runtime contracts.

use std::collections::BTreeMap;
use std::fs::{create_dir_all, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use serde::Serialize;

#[derive(Debug)]
struct PerformanceConfig {
  output_path: PathBuf,
  trace_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PerformanceSpanRecord<'a> {
  profile: &'static str,
  trace_id: &'a str,
  span_id: String,
  process: &'static str,
  layer: &'static str,
  name: &'static str,
  start_offset_ms: f64,
  duration_ms: f64,
  status: &'static str,
  #[serde(skip_serializing_if = "Option::is_none")]
  run_id: Option<&'a str>,
  #[serde(skip_serializing_if = "Option::is_none")]
  invocation_id: Option<&'a str>,
  #[serde(skip_serializing_if = "BTreeMap::is_empty")]
  counts: &'a BTreeMap<&'static str, u64>,
  #[serde(skip_serializing_if = "BTreeMap::is_empty")]
  bytes: &'a BTreeMap<&'static str, u64>,
}

static CONFIG: OnceLock<Option<PerformanceConfig>> = OnceLock::new();
static ORIGIN: OnceLock<Instant> = OnceLock::new();
static NEXT_SPAN: AtomicU64 = AtomicU64::new(1);
static WRITER: OnceLock<Mutex<Option<File>>> = OnceLock::new();

fn valid_trace_id(value: &str) -> bool {
  !value.is_empty()
    && value.len() <= 128
    && value
      .bytes()
      .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}

fn config() -> Option<&'static PerformanceConfig> {
  CONFIG
    .get_or_init(|| {
      if std::env::var("WOML_PROFILE").ok().as_deref() != Some("1") {
        return None;
      }
      let output_path = std::env::var_os("WOML_PROFILE_OUTPUT").map(PathBuf::from)?;
      let trace_id = std::env::var("WOML_PROFILE_TRACE_ID").ok()?;
      valid_trace_id(&trace_id).then_some(PerformanceConfig {
        output_path,
        trace_id,
      })
    })
    .as_ref()
}

/// Returns whether the internal repository profiler is active for this process.
pub fn enabled() -> bool {
  config().is_some()
}

/// One internal diagnostic span. A span is recorded as failed unless its
/// successful completion is marked explicitly.
pub struct PerformanceSpan {
  config: Option<&'static PerformanceConfig>,
  process: &'static str,
  layer: &'static str,
  name: &'static str,
  started_at: Instant,
  start_offset_ms: f64,
  succeeded: bool,
  run_id: Option<String>,
  invocation_id: Option<String>,
  counts: BTreeMap<&'static str, u64>,
  bytes: BTreeMap<&'static str, u64>,
}

impl PerformanceSpan {
  pub fn new(layer: &'static str, name: &'static str) -> Self {
    Self::new_for_process("rust", layer, name)
  }

  pub fn new_for_process(process: &'static str, layer: &'static str, name: &'static str) -> Self {
    let config = config();
    let origin = ORIGIN.get_or_init(Instant::now);
    let started_at = Instant::now();
    Self {
      config,
      process,
      layer,
      name,
      started_at,
      start_offset_ms: started_at.duration_since(*origin).as_secs_f64() * 1_000.0,
      succeeded: false,
      run_id: None,
      invocation_id: None,
      counts: BTreeMap::new(),
      bytes: BTreeMap::new(),
    }
  }

  pub fn run_id(&mut self, value: impl Into<String>) {
    if self.config.is_some() {
      self.run_id = Some(value.into());
    }
  }

  pub fn invocation_id(&mut self, value: impl Into<String>) {
    if self.config.is_some() {
      self.invocation_id = Some(value.into());
    }
  }

  pub fn count(&mut self, name: &'static str, value: usize) {
    if self.config.is_some() {
      self.counts.insert(name, value as u64);
    }
  }

  pub fn bytes(&mut self, name: &'static str, value: usize) {
    if self.config.is_some() {
      self.bytes.insert(name, value as u64);
    }
  }

  pub fn succeed(&mut self) {
    self.succeeded = true;
  }
}

impl Drop for PerformanceSpan {
  fn drop(&mut self) {
    let Some(config) = self.config else {
      return;
    };
    let duration_ms = self.started_at.elapsed().as_secs_f64() * 1_000.0;
    let record = PerformanceSpanRecord {
      profile: "woml.performance-span/v1",
      trace_id: &config.trace_id,
      span_id: format!(
        "span_rust_{}_{}",
        std::process::id(),
        NEXT_SPAN.fetch_add(1, Ordering::Relaxed)
      ),
      process: self.process,
      layer: self.layer,
      name: self.name,
      start_offset_ms: self.start_offset_ms,
      duration_ms,
      status: if self.succeeded {
        "succeeded"
      } else {
        "failed"
      },
      run_id: self.run_id.as_deref(),
      invocation_id: self.invocation_id.as_deref(),
      counts: &self.counts,
      bytes: &self.bytes,
    };
    let Ok(mut encoded) = serde_json::to_vec(&record) else {
      return;
    };
    encoded.push(b'\n');
    let writer = WRITER.get_or_init(|| Mutex::new(None));
    let Ok(mut output) = writer.lock() else {
      return;
    };
    if output.is_none() {
      if let Some(parent) = config.output_path.parent() {
        if create_dir_all(parent).is_err() {
          return;
        }
      }
      let Ok(file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&config.output_path)
      else {
        return;
      };
      *output = Some(file);
    }
    let Some(output) = output.as_mut() else {
      return;
    };
    let _ = output.write_all(&encoded);
  }
}
