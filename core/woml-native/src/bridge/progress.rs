//! Observation delivery, not workflow authority.
//!
//! Ordinary progress retains its unbounded N-API queue and Blocking call mode.
//! Operational wakeups coalesce through one NonBlocking slot; enabled telemetry
//! uses its separately bounded NonBlocking queue. All callbacks retain unref.

use super::apply_runtime_capabilities;
use super::runtime_options_with_secrets;
use chrono::DateTime;
use chrono::Utc;
use napi::threadsafe_function::ErrorStrategy;
use napi::threadsafe_function::ThreadsafeFunctionCallMode;
use napi::Env;
use napi::JsFunction;
use napi::Status;
use serde::Serialize;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use woml_engine::IntervalProgress;
use woml_engine::IntervalProgressReporter;
use woml_engine::LifecycleProgress;
use woml_engine::OperationalOutputReporter;
use woml_engine::RuntimeExecutionOptions;
use woml_engine::RuntimePolicyProgress;
use woml_engine::RuntimePolicyProgressReporter;
use woml_engine::ScheduleProgress;
use woml_engine::ScheduleProgressReporter;
use woml_engine::TriggerProgress;
use woml_engine::TriggerProgressReporter;
use woml_engine::WorkflowCallProgress;
use woml_engine::WorkflowCallProgressReporter;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NativeRuntimeInstanceProgress {
  pub(super) profile: &'static str,
  pub(super) deployment_id: String,
  pub(super) activation_id: String,
  pub(super) runtime_instance_id: String,
  pub(super) runtime_version: &'static str,
  pub(super) native_version: &'static str,
  pub(super) lifecycle: &'static str,
  pub(super) started_at: DateTime<Utc>,
  pub(super) heartbeat_at: DateTime<Utc>,
  pub(super) lease_expires_at: DateTime<Utc>,
}

pub(super) type NativeRuntimeInstanceReporter =
  Arc<dyn Fn(NativeRuntimeInstanceProgress) + Send + Sync>;

fn native_operational_output_reporter(
  env: &Env,
  progress_callback: &JsFunction,
) -> napi::Result<OperationalOutputReporter> {
  let scheduled = Arc::new(AtomicBool::new(false));
  let newest_sequence = Arc::new(AtomicU64::new(0));
  let callback_scheduled = Arc::clone(&scheduled);
  let callback_sequence = Arc::clone(&newest_sequence);
  let mut wakeup = progress_callback
    .create_threadsafe_function::<(), String, _, ErrorStrategy::Fatal>(1, move |_| {
      callback_scheduled.store(false, Ordering::Release);
      Ok(vec![format!(
        "{{\"profile\":\"woml.operational-output-wakeup/v1\",\"newestSequence\":{}}}",
        callback_sequence.load(Ordering::Acquire)
      )])
    })?;
  wakeup.unref(env)?;
  let wakeup = Arc::new(wakeup);
  Ok(Arc::new(move |sequence| {
    newest_sequence.fetch_max(sequence, Ordering::AcqRel);
    if scheduled
      .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
      .is_ok()
    {
      if wakeup.call((), ThreadsafeFunctionCallMode::NonBlocking) != Status::Ok {
        scheduled.store(false, Ordering::Release);
      }
    }
  }))
}

pub(super) fn native_runtime_progress_reporters(
  env: &Env,
  progress_callback: JsFunction,
  telemetry_enabled: bool,
) -> napi::Result<(
  TriggerProgressReporter,
  ScheduleProgressReporter,
  IntervalProgressReporter,
  WorkflowCallProgressReporter,
  RuntimePolicyProgressReporter,
  NativeRuntimeInstanceReporter,
  OperationalOutputReporter,
  Option<woml_engine::TelemetrySignalReporter>,
  Option<woml_engine::TelemetryAdmissionLinkReporter>,
)> {
  let operational_output_reporter = native_operational_output_reporter(env, &progress_callback)?;
  let (telemetry_reporter, telemetry_admission_link_reporter) = if telemetry_enabled {
    let mut telemetry = progress_callback
      .create_threadsafe_function::<String, String, _, ErrorStrategy::Fatal>(1024, |context| {
        Ok(vec![context.value])
      })?;
    telemetry.unref(env)?;
    let telemetry = Arc::new(telemetry);
    let admission_links = Arc::clone(&telemetry);
    (
      Some(woml_engine::bounded_telemetry_reporter(
        1024,
        Arc::new(move |message| {
          if let Ok(json) = serde_json::to_string(&message) {
            let _ = telemetry.call(json, ThreadsafeFunctionCallMode::NonBlocking);
          }
        }),
      )),
      Some(
        Arc::new(move |message: woml_engine::TelemetryAdmissionLinkV1| {
          if let Ok(json) = serde_json::to_string(&message) {
            let _ = admission_links.call(json, ThreadsafeFunctionCallMode::NonBlocking);
          }
        }) as woml_engine::TelemetryAdmissionLinkReporter,
      ),
    )
  } else {
    (None, None)
  };
  let mut progress = progress_callback
    .create_threadsafe_function::<String, String, _, ErrorStrategy::Fatal>(0, |context| {
      Ok(vec![context.value])
    })?;
  progress.unref(env)?;
  let progress = Arc::new(progress);
  let trigger_progress = progress.clone();
  let schedule_progress = progress.clone();
  let interval_progress = progress;
  let workflow_call_progress = interval_progress.clone();
  let runtime_policy_progress = workflow_call_progress.clone();
  let runtime_instance_progress = runtime_policy_progress.clone();
  Ok((
    Arc::new(move |message: TriggerProgress| {
      if let Ok(json) = serde_json::to_string(&message) {
        let _ = trigger_progress.call(json, ThreadsafeFunctionCallMode::Blocking);
      }
    }),
    Arc::new(move |message: ScheduleProgress| {
      if let Ok(json) = serde_json::to_string(&message) {
        let _ = schedule_progress.call(json, ThreadsafeFunctionCallMode::Blocking);
      }
    }),
    Arc::new(move |message: IntervalProgress| {
      if let Ok(json) = serde_json::to_string(&message) {
        let _ = interval_progress.call(json, ThreadsafeFunctionCallMode::Blocking);
      }
    }),
    Arc::new(move |message: WorkflowCallProgress| {
      if let Ok(json) = serde_json::to_string(&message) {
        let _ = workflow_call_progress.call(json, ThreadsafeFunctionCallMode::Blocking);
      }
    }),
    Arc::new(move |message: RuntimePolicyProgress| {
      if let Ok(json) = serde_json::to_string(&message) {
        let _ = runtime_policy_progress.call(json, ThreadsafeFunctionCallMode::Blocking);
      }
    }),
    Arc::new(move |message: NativeRuntimeInstanceProgress| {
      if let Ok(json) = serde_json::to_string(&message) {
        let _ = runtime_instance_progress.call(json, ThreadsafeFunctionCallMode::Blocking);
      }
    }),
    operational_output_reporter,
    telemetry_reporter,
    telemetry_admission_link_reporter,
  ))
}

pub(super) fn runtime_options_with_progress(
  env: &Env,
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  progress_callback: JsFunction,
  resolved_secrets_json: String,
  runtime_capabilities_json: Option<String>,
) -> napi::Result<RuntimeExecutionOptions> {
  let operational_output_reporter = native_operational_output_reporter(env, &progress_callback)?;
  let mut progress = progress_callback
    .create_threadsafe_function::<String, String, _, ErrorStrategy::Fatal>(0, |context| {
      Ok(vec![context.value])
    })?;
  progress.unref(env)?;
  let lifecycle_progress = progress.clone();
  let runtime_policy_progress = lifecycle_progress.clone();
  apply_runtime_capabilities(
    runtime_options_with_secrets(
      bun_executable,
      script_host_path,
      script_timeout_ms,
      resolved_secrets_json,
    )?
    .with_progress_reporter(Arc::new(move |message| {
      if let Ok(json) = serde_json::to_string(&message) {
        let _ = progress.call(json, ThreadsafeFunctionCallMode::Blocking);
      }
    }))
    .with_lifecycle_progress_reporter(Arc::new(move |message: LifecycleProgress| {
      if let Ok(json) = serde_json::to_string(&message) {
        let _ = lifecycle_progress.call(json, ThreadsafeFunctionCallMode::Blocking);
      }
    }))
    .with_runtime_policy_progress_reporter(Arc::new(move |message: RuntimePolicyProgress| {
      if let Ok(json) = serde_json::to_string(&message) {
        let _ = runtime_policy_progress.call(json, ThreadsafeFunctionCallMode::Blocking);
      }
    }))
    .with_operational_output_reporter(operational_output_reporter),
    runtime_capabilities_json,
  )
}
