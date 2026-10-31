//! Exactly one shared webhook registry and one shared remote-coordinator registry.
//!
//! Exported stop operations remove an owner before signalling and joining it.
//! Moving these declarations must never create a second OnceLock or detached owner.

use super::NativeTriggerRuntimeError;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::thread::JoinHandle;
use woml_engine::ExternalSignalAdmissionOutcome;
use woml_engine::ExternalSignalAdmissionRequest;
use woml_engine::ExternalTriggerAdmissionCommand;
use woml_engine::RemoteCoordinatorServerV1;
use woml_engine::RemoteDispatchTransportV1;
use woml_engine::RuntimeExecutionError;

pub(super) struct NativeWebhookRuntimeThread {
  pub(super) control: mpsc::Sender<NativeWebhookRuntimeCommand>,
  pub(super) ingress: tokio::sync::mpsc::UnboundedSender<ExternalTriggerAdmissionCommand>,
  pub(super) manual_targets: BTreeMap<(String, String), NativeManualTarget>,
  pub(super) join: JoinHandle<()>,
}

#[derive(Clone)]
pub(super) struct NativeManualTarget {
  pub(super) definition_hash: String,
  pub(super) admission_status: &'static str,
}

pub(super) enum NativeWebhookRuntimeCommand {
  Activate(mpsc::SyncSender<Result<(), NativeTriggerRuntimeError>>),
  Signal(
    ExternalSignalAdmissionRequest,
    mpsc::SyncSender<Result<ExternalSignalAdmissionOutcome, RuntimeExecutionError>>,
  ),
  OwnershipLost,
  Stop,
}

static WEBHOOK_RUNTIMES: OnceLock<Mutex<HashMap<String, NativeWebhookRuntimeThread>>> =
  OnceLock::new();

pub(super) fn webhook_runtimes() -> &'static Mutex<HashMap<String, NativeWebhookRuntimeThread>> {
  WEBHOOK_RUNTIMES.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) struct NativeRemoteCoordinatorRuntime {
  pub(super) server: RemoteCoordinatorServerV1,
  pub(super) transport: RemoteDispatchTransportV1,
}

static REMOTE_COORDINATORS: OnceLock<Mutex<HashMap<String, NativeRemoteCoordinatorRuntime>>> =
  OnceLock::new();

pub(super) fn remote_coordinators(
) -> &'static Mutex<HashMap<String, NativeRemoteCoordinatorRuntime>> {
  REMOTE_COORDINATORS.get_or_init(|| Mutex::new(HashMap::new()))
}
