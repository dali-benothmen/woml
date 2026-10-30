//! Stable capability facade. Private modules retain existing authority, identities, limits, and lifecycle ownership.

mod authority;
mod cancellation;
mod concurrency;
mod contracts;
mod failures;
mod handler;
mod identity;
mod native_fetch;
mod operational_output;
mod policy;
mod registry;
mod test_handler;
mod validation;

pub(in crate::capability) use authority::generated_event_id;
pub use authority::{DurableCapabilityAuthority, DurableCapabilityAuthorityError};
pub use cancellation::CapabilityCancellationToken;
pub use contracts::{
  CapabilityCallFailed, CapabilityCallIdentity, CapabilityCallLimits, CapabilityCallRequest,
  CapabilityCallResult, CapabilityCallSucceeded, CapabilityDescriptor, CapabilityEffect,
  CapabilityFailure, CapabilityFailureKind, CapabilityIdentityMode, CAPABILITY_CALL_CONTRACT,
  CAPABILITY_CALL_CONTRACT_VERSION, DEFAULT_CAPABILITY_FRAME_BYTES, DEFAULT_CAPABILITY_INPUT_BYTES,
  DEFAULT_CAPABILITY_IN_FLIGHT_GLOBAL, DEFAULT_CAPABILITY_IN_FLIGHT_PER_INVOCATION,
  DEFAULT_CAPABILITY_RESULT_BYTES, DEFAULT_CAPABILITY_TIMEOUT_MS, MAX_CAPABILITY_TIMEOUT_MS,
};
pub use failures::capability_transport_failure;
pub(in crate::capability) use failures::{
  duration_ms, failed_fields, failed_result, failure, failure_with_size,
};
pub use handler::CapabilityHandler;
pub use identity::derive_operation_key;
pub(in crate::capability) use identity::{
  iteration_attempt_identity, operation_with_iteration_scope, operational_invocation_is_active,
};
pub use native_fetch::{
  NativeFetchFailure, NativeFetchFailureKind, NativeFetchInvocationIdentity,
  NativeFetchObservation, NATIVE_FETCH_OBSERVATION_CONTRACT,
  NATIVE_FETCH_OBSERVATION_CONTRACT_VERSION,
};
pub(in crate::capability) use policy::{
  infrastructure_policy_evidence, infrastructure_policy_execution_mode,
};
pub use registry::{CapabilityRegistry, CapabilityRegistryError};
pub use test_handler::TestCapabilityHandler;
pub use validation::validate_safe_metadata;
pub(in crate::capability) use validation::{
  serialized_bytes, valid_digest, valid_name, validate_result_base,
};
