//! Stable process facade. Private modules retain existing authority, identities, limits, and lifecycle ownership.

mod containment;
mod contracts;
mod environment;
mod executable;
mod execution;
mod failures;
mod handler;
mod output;
mod platform;
mod policy;
#[cfg(test)]
mod tests;
mod validation;

pub(in crate::process) use containment::ProcessContainment;
pub(in crate::process) use contracts::{
  OutputMode, ProcessRunRequest, ProcessRunResultV1, ProcessRunResultV2, MAX_PROCESS_STREAM_BYTES,
};
pub use contracts::{
  ProcessPolicyEnvironment, ProcessPolicyError, ProcessPolicyExecutable, ProcessPolicyLimits,
  ProcessPolicyModes, ProcessPolicyV1, ProcessRuntimeMode, PROCESS_CONTRACT,
  PROCESS_CONTRACT_VERSION, PROCESS_CONTRACT_VERSION_V2, PROCESS_POLICY_CONTRACT,
  PROCESS_POLICY_CONTRACT_VERSION,
};
pub(in crate::process) use environment::prepare_process_environment;
pub(in crate::process) use executable::{
  verify_executable, verify_executable_policy, ExecutableIdentity,
};
pub(in crate::process) use execution::execute_process;
#[cfg(unix)]
pub(in crate::process) use execution::PROCESS_TERMINATION_GRACE;
pub(in crate::process) use failures::{failure, invalid_input, output_failure, rejected};
pub use handler::ManagedProcessHandler;
pub(in crate::process) use handler::{safe_duration, AuthorizedProcess};
pub(in crate::process) use output::{
  append_process_text, close_process_stream, drain_output, output_authority_failure,
  output_mode_name, redact_and_bound_output, sensitive_environment_values, ActiveProcessStream,
  CapturedOutput, IncrementalTextRedactor, ProcessStreamFailure, ProcessStreamState,
  ProcessStreamTarget,
};
#[cfg(unix)]
pub(in crate::process) use platform::exit_signal;
#[cfg(not(unix))]
pub(in crate::process) use platform::exit_signal;
#[cfg(unix)]
pub(in crate::process) use platform::platform_executable;
#[cfg(not(unix))]
pub(in crate::process) use platform::platform_executable;
#[cfg(unix)]
pub(in crate::process) use platform::prepare_process_containment;
#[cfg(windows)]
pub(in crate::process) use platform::prepare_process_containment;
#[cfg(not(any(unix, windows)))]
pub(in crate::process) use platform::prepare_process_containment;
#[cfg(unix)]
pub(in crate::process) use platform::process_group_exists;
#[cfg(unix)]
pub(in crate::process) use platform::signal_process_group;
#[cfg(windows)]
pub(in crate::process) use platform::windows_file_identity;
#[cfg(windows)]
pub(in crate::process) use platform::windows_system_root;
#[cfg(windows)]
pub(in crate::process) use platform::WindowsJob;
pub(in crate::process) use policy::{ApprovedExecutable, PreparedProcessPolicy};
pub(in crate::process) use validation::{
  authorize_cwd, canonical_directory, parse_request, validate_alias, validate_environment_name,
  validate_limits,
};
