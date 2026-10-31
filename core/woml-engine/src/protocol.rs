//! Stable protocol contract facade. Private modules separate shapes and validation.

mod bindings;
mod capabilities;
mod control;
mod execution;
mod modules;
mod operational;
mod outcomes;

pub use bindings::{
  CleanupBindingV1, CleanupReasonV1, CompensationBindingV1, CompensationStatusV1,
  CorrectionBindingRef, CorrectionReasonV1, LifecycleBindingV1, LifecycleFailureBindingV1,
  LifecycleStepBindingV1, LifecycleWorkflowBindingV1, ReusableDefinitionBindingV3,
  ReusableDefinitionKind, ReusableInvocationOutcome, ReusableLifecycleBindingV1,
  ReusableLifecycleErrorV1, ReusableLifecycleHook, ReusableScriptBindingV3, ScriptAttempt,
  ScriptBindings, ScriptExecutionContext, ScriptExecutionMode, ScriptIterationBinding,
};
pub use capabilities::{
  CapabilityCallMessage, CapabilityResultMessage, FetchObservationAckMessage,
  FetchObservationMessage,
};
pub use control::{CancelMessage, ReadyMessage};
pub use execution::ExecuteMessage;
pub use modules::{ModuleRegisteredMessage, RegisterModuleMessage, RuntimeModuleBinding};
pub use operational::{
  ProgressAckMessage, ProgressReportMessage, ScriptLogLevelV1, ScriptLogMessage,
};
pub use outcomes::{CompletedMessage, HostOutcome, HostReportedFailure, HostReportedFailureKind};

pub const SCRIPT_HOST_PROTOCOL: &str = "woml.script-host";

pub const SCRIPT_HOST_PROTOCOL_VERSION: u32 = 12;
