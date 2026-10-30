//! Frozen body-free artifact errors with unchanged retryability and ambiguity.

use crate::CapabilityFailure;
use crate::CapabilityFailureKind;

pub(in crate::artifacts) fn artifact_failure(
  kind: CapabilityFailureKind,
  code: &str,
  message: &str,
  retryable: bool,
  ambiguous: bool,
) -> CapabilityFailure {
  CapabilityFailure {
    kind,
    code: code.to_string(),
    message: message.to_string(),
    retryable,
    ambiguous,
    details: None,
  }
}

pub(in crate::artifacts) fn artifact_path_invalid() -> CapabilityFailure {
  artifact_failure(
    CapabilityFailureKind::InvalidInput,
    "WOML_ARTIFACT_PATH_INVALID",
    "The artifact source path is invalid.",
    false,
    false,
  )
}

pub(in crate::artifacts) fn source_denied() -> CapabilityFailure {
  artifact_failure(
    CapabilityFailureKind::ServiceRejected,
    "WOML_ARTIFACT_SOURCE_DENIED",
    "The artifact source is outside approved roots.",
    false,
    false,
  )
}

pub(in crate::artifacts) fn source_changed() -> CapabilityFailure {
  artifact_failure(
    CapabilityFailureKind::ServiceRejected,
    "WOML_ARTIFACT_SOURCE_CHANGED",
    "The artifact source changed while it was imported.",
    false,
    true,
  )
}

pub(in crate::artifacts) fn source_type_unsupported() -> CapabilityFailure {
  artifact_failure(
    CapabilityFailureKind::ServiceRejected,
    "WOML_ARTIFACT_SOURCE_TYPE_UNSUPPORTED",
    "Artifact v1 accepts regular files only.",
    false,
    false,
  )
}

pub(in crate::artifacts) fn artifact_too_large() -> CapabilityFailure {
  artifact_failure(
    CapabilityFailureKind::InputTooLarge,
    "WOML_ARTIFACT_TOO_LARGE",
    "The artifact exceeds its configured byte limit.",
    false,
    false,
  )
}

pub(in crate::artifacts) fn quota_exceeded() -> CapabilityFailure {
  artifact_failure(
    CapabilityFailureKind::ServiceRejected,
    "WOML_ARTIFACT_QUOTA_EXCEEDED",
    "The artifact quota would be exceeded.",
    false,
    false,
  )
}

pub(in crate::artifacts) fn retention_invalid() -> CapabilityFailure {
  artifact_failure(
    CapabilityFailureKind::InvalidInput,
    "WOML_ARTIFACT_RETENTION_INVALID",
    "The requested retention violates artifact policy.",
    false,
    false,
  )
}

pub(in crate::artifacts) fn artifact_not_found() -> CapabilityFailure {
  artifact_failure(
    CapabilityFailureKind::ServiceRejected,
    "WOML_ARTIFACT_NOT_FOUND",
    "The artifact does not exist in this durable store.",
    false,
    false,
  )
}

pub(in crate::artifacts) fn artifact_access_denied() -> CapabilityFailure {
  artifact_failure(
    CapabilityFailureKind::ServiceRejected,
    "WOML_ARTIFACT_ACCESS_DENIED",
    "The current run is not authorized to access this artifact.",
    false,
    false,
  )
}

pub(in crate::artifacts) fn artifact_expired() -> CapabilityFailure {
  artifact_failure(
    CapabilityFailureKind::ServiceRejected,
    "WOML_ARTIFACT_EXPIRED",
    "The artifact has expired and is no longer readable.",
    false,
    false,
  )
}

pub(in crate::artifacts) fn artifact_quarantined() -> CapabilityFailure {
  artifact_failure(
    CapabilityFailureKind::ServiceRejected,
    "WOML_ARTIFACT_QUARANTINED",
    "Artifact policy quarantined this body.",
    false,
    false,
  )
}

pub(in crate::artifacts) fn artifact_corrupt() -> CapabilityFailure {
  artifact_failure(
    CapabilityFailureKind::ServiceRejected,
    "WOML_ARTIFACT_CORRUPT",
    "Artifact integrity verification failed.",
    false,
    false,
  )
}

pub(in crate::artifacts) fn backend_unavailable() -> CapabilityFailure {
  artifact_failure(
    CapabilityFailureKind::TransportFailed,
    "WOML_ARTIFACT_BACKEND_UNAVAILABLE",
    "The artifact backend is unavailable.",
    true,
    false,
  )
}

pub(in crate::artifacts) fn materialization_unavailable() -> CapabilityFailure {
  artifact_failure(
    CapabilityFailureKind::TransportFailed,
    "WOML_ARTIFACT_MATERIALIZATION_UNAVAILABLE",
    "A private artifact materialization could not be created.",
    true,
    false,
  )
}

pub(in crate::artifacts) fn artifact_timed_out() -> CapabilityFailure {
  artifact_failure(
    CapabilityFailureKind::TimedOut,
    "WOML_ARTIFACT_TIMED_OUT",
    "The artifact transfer exceeded its deadline.",
    false,
    true,
  )
}

pub(in crate::artifacts) fn artifact_cancelled() -> CapabilityFailure {
  artifact_failure(
    CapabilityFailureKind::Cancelled,
    "WOML_ARTIFACT_CANCELLED",
    "The artifact transfer was cancelled.",
    false,
    true,
  )
}

pub(in crate::artifacts) fn operation_identity_conflict() -> CapabilityFailure {
  artifact_failure(
    CapabilityFailureKind::ServiceRejected,
    "WOML_ARTIFACT_OPERATION_IDENTITY_CONFLICT",
    "The artifact operation name was reused with different input.",
    false,
    false,
  )
}

pub(in crate::artifacts) fn artifact_interrupted() -> CapabilityFailure {
  artifact_failure(
    CapabilityFailureKind::Interrupted,
    "WOML_ARTIFACT_INTERRUPTED",
    "The artifact write outcome cannot be proven.",
    false,
    true,
  )
}
