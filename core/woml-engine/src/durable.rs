//! Stable durable-store contracts and one connection/cache owner.
//!
//! Private transaction, recovery, and adapter modules share this owner.
//! Module boundaries never create independent transaction or scheduling authority.

mod bootstrap;
#[cfg(test)]
mod boundary_tests;
mod definitions;
mod errors;
mod events;
mod indexes;
mod inspection;
mod migrations;
mod opening;
mod records;
mod schema;
mod validation;

pub(in crate::durable) use definitions::{
  definition_by_hash, definition_for_run, load_run_binding_optional, run_binding_in_transaction,
};
pub use definitions::{MAX_MODULE_ARTIFACT_BYTES, MAX_MODULE_ARTIFACT_SET_BYTES};
pub use errors::DurableStoreError;
pub(in crate::durable) use events::{
  append_to_history, append_to_history_scoped, append_to_history_scoped_incremental,
  cache_projection_state, ensure_run_exists, generated_event_id, lifecycle_failure_from_attempt,
  load_events, projection_state_for_events,
};
pub(crate) use events::{expand_model_v11_payload, lifecycle_warnings_from_projection};
pub(in crate::durable) use indexes::{
  rebuild_correction_indexes, rebuild_run_summary_index, rebuild_runtime_policy_indexes,
  runtime_policy_indexes_need_rebuild, write_correction_indexes_from_projection, write_run_summary,
  write_run_summary_from_projection, write_runtime_policy_indexes_for_run,
  write_runtime_policy_indexes_from_projection,
};
pub(in crate::durable) use migrations::{
  migrate_store_v10_to_v11, migrate_store_v11_to_v12, migrate_store_v12_to_v13,
  migrate_store_v13_to_v14, migrate_store_v14_to_v15, migrate_store_v15_to_v16,
  migrate_store_v16_to_v17, migrate_store_v17_to_v18, migrate_store_v18_to_v19,
  migrate_store_v19_to_v20, migrate_store_v1_to_v2, migrate_store_v2_to_v3, migrate_store_v3_to_v4,
  migrate_store_v4_to_v5, migrate_store_v5_to_v6, migrate_store_v6_to_v7, migrate_store_v7_to_v8,
  migrate_store_v8_to_v9, migrate_store_v9_to_v10,
};
pub use records::{
  ApprovalDecisionOutcome, ApprovalDecisionOutcomeStatus, ApprovalTimeoutSettlement,
  ApprovalTimeoutSettlementStatus, ApprovalTokenBinding, CorrectionInspectionV1,
  CorrectionObservationV1, CorrectionRecoveryActionV1, CorrectionRecoveryCandidateV1,
  CorrectionRecoveryRequestV1, CorrectionRecoveryResultV1, CorrectionRecoveryStatusV1,
  CorrectionSagaInspectionV1, DurableWaitRecordV1, ExternalSignalAdmissionOutcome,
  ExternalSignalAdmissionRequest, ExternalSignalAdmissionStatus, ExternalSignalResolution,
  ForkRecoveryBranchV1, ForkRecoveryWorkV1, InfrastructurePolicyRunBindingV1,
  InspectedBusinessOutcome, InspectedForEachStatusV6, InspectedForkJoinModeV4,
  InspectedForkStatusV4, InspectedLockStatusV1, InspectedReusableKindV5,
  InspectedReusableLifecycleStatusV5, InspectedReusableStatusV5, InspectedWaitStatusV1,
  InspectedWaitV1, InternalEventAdmissionOutcome, InternalEventAdmissionRequest, IntervalCursor,
  IntervalCursorRegistration, IntervalCursorRegistrationOutcome, IssuedApprovalToken,
  LockInspectionV1, LockLeaseMaintenance, LockObservationV1, LockRecoveryCandidateV1,
  LockRecoveryKindV1, NotificationDeliveryWork, NotificationDispatchReport,
  NotificationProviderAdapter, NotificationProviderDeliveryResult,
  NotificationProviderUpdateResult, NotificationUpdateWork, PolicyClaimWaitReason,
  PolicyExecutionClaimResult, PolicyWaitingFor, ProgrammaticLockAcquire,
  ProgrammaticLockAcquireState, ProgrammaticLockLease, PublicRunStatus, RecoveryReport,
  RichApprovalDecisionAdmission, RichApprovalDecisionInspection, RichApprovalDecisionOutcome,
  RichApprovalDecisionStatus, RichApprovalIdentityDecision, RichApprovalInspectionV1,
  RunCancellationCode, RunCancellationResult, RunCancellationStatus, RunDefinitionBinding,
  RunInspectionCancellationV2, RunInspectionForEachCountsV6, RunInspectionForEachItemV6,
  RunInspectionForEachV6, RunInspectionForkCountsV4, RunInspectionForkJoinV4, RunInspectionForkV4,
  RunInspectionForksV4, RunInspectionHookV2, RunInspectionPolicyV3, RunInspectionReusableCountsV5,
  RunInspectionReusableDefinitionsV5, RunInspectionReusableItemV5, RunInspectionRichApprovalV11,
  RunInspectionRichApprovalsV11, RunInspectionV11, RunInspectionV2, RunInspectionV3,
  RunInspectionV4, RunInspectionV5, RunInspectionV6, RunListV1, RunListV2, RunSummaryV1,
  RunSummaryV2, RunTimeoutSettlement, RuntimeObservationV1, RuntimeOwnerLease, ScheduleCursor,
  ScheduleCursorRegistration, ScheduleCursorRegistrationOutcome, SchedulerClaimV1,
  SignalWaitTimeoutSettlement, TimerWaitSettlement, TriggerAdmissionOutcome,
  TriggerAdmissionRequest, TriggerOccurrence, TriggerRecoveryWork, UnresolvedCorrectionActionV1,
  WaitInspectionV1, CORRECTION_INSPECTION_PROFILE, CORRECTION_OBSERVATION_PROFILE,
  CORRECTION_RECOVERY_BATCH_LIMIT, CORRECTION_RECOVERY_REQUEST_PROFILE,
  CORRECTION_RECOVERY_RESULT_PROFILE, CORRECTION_UNRESOLVED_STORE_LIMIT,
  EXTERNAL_SIGNAL_ADMISSION_PROFILE, EXTERNAL_SIGNAL_MAX_EARLY_PER_RUN,
  EXTERNAL_SIGNAL_MAX_PAYLOAD_BYTES, EXTERNAL_SIGNAL_MAX_PAYLOAD_DEPTH,
  EXTERNAL_SIGNAL_RESULT_PROFILE, LOCK_RECOVERY_BATCH_LIMIT, LOCK_WAITERS_PER_KEY_LIMIT,
  LOCK_WAITERS_PER_STORE_LIMIT, RICH_APPROVAL_INSPECTION_DECISION_LIMIT,
  RUN_INSPECTION_RICH_APPROVAL_LIMIT,
};
pub(crate) use records::{
  InfrastructurePolicyApprovalEvidenceV1, InfrastructurePolicyArtifactEvidenceV1,
  InfrastructurePolicyEvidenceSnapshotV1, InfrastructurePolicyLockEvidenceV1,
};
pub(in crate::durable) use records::{RichQuorumState, RichReviewRow};
pub use schema::DURABLE_STORE_SCHEMA_VERSION;
pub(in crate::durable) use schema::{
  CREATE_APPROVAL_SCHEMA_V2, CREATE_CORRECTION_SCHEMA_V17, CREATE_DURABLE_STATE_SCHEMA_V13,
  CREATE_DURABLE_WAIT_SCHEMA_V15, CREATE_INFRASTRUCTURE_POLICY_SCHEMA_V19,
  CREATE_INTERNAL_EVENT_SCHEMA_V7, CREATE_INTERVAL_SCHEMA_V6, CREATE_LOCK_SCHEMA_V16,
  CREATE_MODULE_ARTIFACT_SCHEMA_V8, CREATE_NOTIFICATION_SCHEMA_V3,
  CREATE_PRODUCTION_RUNTIME_SCHEMA_V14, CREATE_RICH_APPROVAL_SCHEMA_V18,
  CREATE_RUN_SUMMARY_SCHEMA_V11, CREATE_SCHEDULE_SCHEMA_V5, CREATE_SCHEMA,
  CREATE_TRIGGER_SCHEMA_V4, CREATE_WORKFLOW_CALL_SCHEMA_V9, CREATE_WORKFLOW_RUNTIME_SCHEMA_V10,
  MIGRATE_RUNTIME_POLICY_SCHEMA_V12, STORE_SCHEMA_VERSION_V1, STORE_SCHEMA_VERSION_V10,
  STORE_SCHEMA_VERSION_V11, STORE_SCHEMA_VERSION_V12, STORE_SCHEMA_VERSION_V13,
  STORE_SCHEMA_VERSION_V14, STORE_SCHEMA_VERSION_V15, STORE_SCHEMA_VERSION_V16,
  STORE_SCHEMA_VERSION_V17, STORE_SCHEMA_VERSION_V18, STORE_SCHEMA_VERSION_V19,
  STORE_SCHEMA_VERSION_V2, STORE_SCHEMA_VERSION_V20, STORE_SCHEMA_VERSION_V3,
  STORE_SCHEMA_VERSION_V4, STORE_SCHEMA_VERSION_V5, STORE_SCHEMA_VERSION_V6,
  STORE_SCHEMA_VERSION_V7, STORE_SCHEMA_VERSION_V8, STORE_SCHEMA_VERSION_V9,
};
pub(in crate::durable) use validation::{
  validate_store_v10_schema, validate_store_v11_schema, validate_store_v12_schema,
  validate_store_v13_schema, validate_store_v14_schema, validate_store_v15_schema,
  validate_store_v16_schema, validate_store_v17_schema, validate_store_v18_schema,
  validate_store_v19_schema, validate_store_v2_schema, validate_store_v3_schema,
  validate_store_v4_schema, validate_store_v5_schema, validate_store_v6_schema,
  validate_store_v7_schema, validate_store_v8_schema, validate_store_v9_schema,
};

mod transactions;
pub(in crate::durable) use transactions::{
  admit_trigger_occurrence_in_transaction, append_signal_resolution, ensure_lock_wait_capacity,
  expire_lock_owner, finite_duration_ms, grant_oldest_lock_waiter, load_trigger_occurrence_by_id,
  lock_descriptor_for_protected_node, notification_request_all_failed, programmatic_lock_lease_ms,
  programmatic_lock_session_ref, resolve_lock_execution, resolve_signal_correlation,
  rich_review_row, sha256_prefixed, valid_lock_key, validate_trigger_admission_request,
  validate_trigger_occurrence_history,
};
pub use transactions::{
  correction_invocation_id, derive_lifecycle_hook_invocation_id, RUNTIME_POLICY_QUEUE_CEILING,
};
mod credentials;
pub(in crate::durable) use credentials::{
  approval_credential_expiry, parse_approval_token, parse_notification_capability, random_hex,
};
mod record_values;
pub(in crate::durable) use record_values::{
  approval_decision_text, parse_approval_decision, parse_durable_timestamp, parse_stored_timestamp,
  parse_timestamp,
};
mod recovery;
pub(in crate::durable) use recovery::{
  attempt_run_failed_data, interrupted_failure, interrupted_operation_failure,
  interrupted_operation_metadata, recover_open_for_each_failure, RunRecovery,
};
mod dag_adapter;
mod integrity;
pub use dag_adapter::{
  DurableDagEngine, DurableEngineError, StepFailureCommit, StepFailureDisposition,
};

use crate::projection::ProjectionFoldState;
use rusqlite::Connection;
use std::cell::RefCell;
use std::collections::HashMap;

#[derive(Debug)]
pub struct DurableEventStore {
  pub(crate) connection: Connection,
  projection_cache: RefCell<HashMap<String, ProjectionFoldState>>,
}

#[cfg(test)]
mod notification_capability_tests {
  use super::parse_notification_capability;

  #[test]
  fn accepts_compact_and_legacy_notification_capabilities() {
    let compact_id = "1".repeat(16);
    let compact_secret = "2".repeat(32);
    let compact = format!("ncap_{compact_id}.{compact_secret}");
    assert_eq!(
      parse_notification_capability(&compact).unwrap(),
      (compact_id.as_str(), compact_secret.as_str())
    );

    let legacy_id = "3".repeat(32);
    let legacy_secret = "4".repeat(64);
    let legacy = format!("ncap_{legacy_id}.{legacy_secret}");
    assert_eq!(
      parse_notification_capability(&legacy).unwrap(),
      (legacy_id.as_str(), legacy_secret.as_str())
    );
  }
}
