use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Utc};
use getrandom::getrandom;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use serde_json_canonicalizer::to_vec as canonical_json;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use thiserror::Error;
use uuid::Uuid;

pub const RUNTIME_POLICY_QUEUE_CEILING: i64 = 10_000;
const MAX_ACTIVE_PROJECTION_CACHE_ENTRIES: usize = 256;

use crate::engine::{
  ready_node_ids_for_projection, ready_node_ids_for_projection_at, resolve_context_reference,
  step_effect_idempotency_key, validate_event_history_against_definition,
  validate_payload_against_definition, validate_scoped_payload_against_definition,
};
use crate::event::{
  is_definition_hash, ApprovalDecision, ApprovalDecisionSource, ApprovalFailure,
  ApprovalInvalidatedData, ApprovalQuorumProgressedData, ApprovalRequestedData, ApprovalResolution,
  ApprovalResolvedData, ApprovalReviewRequestedData, ApprovalReviewerDecisionRecordedData,
  ApprovalTimeoutPolicy, ForEachIterationFailedData, ForEachIterationFailureKind,
  ForEachIterationScope, ForEachIterationSkippedData, ForEachSettledData, ForEachSkipReason,
  LifecycleHookRequestedData, LifecycleSubject, LifecycleSubjectKind,
  NotificationDeliveryAttemptStartedData, NotificationDeliveryFailedData,
  NotificationDeliveryRequestedData, NotificationDeliverySucceededData,
  NotificationMessageUpdateAttemptStartedData, NotificationMessageUpdateFailedData,
  NotificationMessageUpdatedData, NotificationRunFailure, NotificationSafeFailure,
  OperationFailedData, OperationSucceededData, ParallelFailure, ParallelFailurePolicy,
  ParallelGroupCompletedData, ParallelGroupOutcome, ProviderMessageIdentity,
  RichApprovalResolutionReason, RunAdmissionQueue, RunAdmissionTrigger, RunAdmittedData,
  RunCancellationRequestedData, RunFailedData, RunFailedDataV1, RunFailedDataV2, RunFailedDataV3,
  RunFailedDataV4, RunFailedDataV5, RunIngress, RunStartedData, RunSucceededData,
  StepAttemptFailedData, StepRetryScheduledData,
};
use crate::model::ValueExpression;
use crate::projection::{
  ApprovalRequestStatus, AttemptStatus, ForEachIterationStatus, ForEachStatus,
  NotificationDeliveryStatus, NotificationMessageUpdateStatus, OperationProjection,
  ParallelGroupStatus, ProjectionFoldState,
};
use crate::remote_dispatch::{validate_remote_dispatch_schema, CREATE_REMOTE_DISPATCH_SCHEMA_V20};
use crate::runtime::RuntimeModuleArtifact;
use crate::workflow_calls::{
  WorkflowCallAdmission, WorkflowCallAdmissionOutcome, WorkflowCallAdmissionRequest,
  WorkflowCallIndexState, WorkflowCallRunRelations, WorkflowCallRunSummary, WorkflowRuntimeRoute,
  WorkflowTarget, MAX_WORKFLOW_CALL_DEPTH, MAX_WORKFLOW_CALL_INSPECTION_CHILDREN,
};
use crate::{
  fold_events, run_event_schema_version_for_model, AttemptFailure, AttemptFailureKind,
  CompiledWorkflowDefinition, FoldError, InfrastructurePolicyBundleV1,
  InfrastructurePolicySignatureStatusV1, InfrastructurePolicyTrustV1, ModelValidationError,
  PreparedInfrastructurePolicyV1, RunEvent, RunEventPayload, RunProjection, RunStatus,
  RUN_EVENT_SCHEMA_VERSION_V1, RUN_EVENT_SCHEMA_VERSION_V11, RUN_EVENT_SCHEMA_VERSION_V12,
  RUN_EVENT_SCHEMA_VERSION_V13, RUN_EVENT_SCHEMA_VERSION_V14, RUN_EVENT_SCHEMA_VERSION_V2,
  RUN_EVENT_SCHEMA_VERSION_V3, RUN_EVENT_SCHEMA_VERSION_V4, RUN_EVENT_SCHEMA_VERSION_V5,
  RUN_EVENT_SCHEMA_VERSION_V6, RUN_EVENT_SCHEMA_VERSION_V7, RUN_EVENT_SCHEMA_VERSION_V8,
  RUN_EVENT_SCHEMA_VERSION_V9,
};
use crate::{
  verify_bound_rich_approval_review, BoundRichApprovalReview, ReviewPayloadV1,
  ReviewerDirectoryAuthorityV1, ReviewerDirectoryError, ReviewerIdentityClaimV1,
};

pub const DURABLE_STORE_SCHEMA_VERSION: u32 = 20;
const STORE_SCHEMA_VERSION_V1: &str = "1";
const STORE_SCHEMA_VERSION_V2: &str = "2";
const STORE_SCHEMA_VERSION_V3: &str = "3";
const STORE_SCHEMA_VERSION_V4: &str = "4";
const STORE_SCHEMA_VERSION_V5: &str = "5";
const STORE_SCHEMA_VERSION_V6: &str = "6";
const STORE_SCHEMA_VERSION_V7: &str = "7";
const STORE_SCHEMA_VERSION_V8: &str = "8";
const STORE_SCHEMA_VERSION_V9: &str = "9";
const STORE_SCHEMA_VERSION_V10: &str = "10";
const STORE_SCHEMA_VERSION_V11: &str = "11";
const STORE_SCHEMA_VERSION_V12: &str = "12";
const STORE_SCHEMA_VERSION_V13: &str = "13";
const STORE_SCHEMA_VERSION_V14: &str = "14";
const STORE_SCHEMA_VERSION_V15: &str = "15";
const STORE_SCHEMA_VERSION_V16: &str = "16";
const STORE_SCHEMA_VERSION_V17: &str = "17";
const STORE_SCHEMA_VERSION_V18: &str = "18";
const STORE_SCHEMA_VERSION_V19: &str = "19";
const STORE_SCHEMA_VERSION_V20: &str = "20";
const DEFAULT_APPROVAL_CREDENTIAL_LIFETIME_HOURS: i64 = 24;

const CREATE_INFRASTRUCTURE_POLICY_SCHEMA_V19: &str = r#"
CREATE TABLE IF NOT EXISTS woml_infrastructure_policy_bundles (
  policy_digest TEXT PRIMARY KEY,
  policy_id TEXT NOT NULL,
  policy_version TEXT NOT NULL,
  environment TEXT NOT NULL,
  canonical_payload BLOB NOT NULL,
  signature_status TEXT NOT NULL CHECK (signature_status IN ('unsigned', 'verified')),
  signing_key_id TEXT,
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS woml_run_infrastructure_policies (
  run_id TEXT PRIMARY KEY,
  policy_digest TEXT NOT NULL,
  admitted_at TEXT NOT NULL,
  FOREIGN KEY (run_id) REFERENCES woml_runs(run_id),
  FOREIGN KEY (policy_digest) REFERENCES woml_infrastructure_policy_bundles(policy_digest)
);

CREATE INDEX IF NOT EXISTS woml_run_infrastructure_policies_digest
  ON woml_run_infrastructure_policies(policy_digest, run_id);

CREATE TRIGGER IF NOT EXISTS woml_infrastructure_policy_bundles_no_update
BEFORE UPDATE ON woml_infrastructure_policy_bundles
BEGIN
  SELECT RAISE(ABORT, 'WOML infrastructure policy bundles are immutable');
END;

CREATE TRIGGER IF NOT EXISTS woml_infrastructure_policy_bundles_no_delete
BEFORE DELETE ON woml_infrastructure_policy_bundles
BEGIN
  SELECT RAISE(ABORT, 'WOML infrastructure policy bundles are retention-managed');
END;

CREATE TRIGGER IF NOT EXISTS woml_run_infrastructure_policies_no_update
BEFORE UPDATE ON woml_run_infrastructure_policies
BEGIN
  SELECT RAISE(ABORT, 'WOML run infrastructure policy bindings are immutable');
END;

CREATE TRIGGER IF NOT EXISTS woml_run_infrastructure_policies_no_delete
BEFORE DELETE ON woml_run_infrastructure_policies
BEGIN
  SELECT RAISE(ABORT, 'WOML run infrastructure policy bindings are retention-managed');
END;
"#;

const CREATE_RUN_SUMMARY_SCHEMA_V11: &str = r#"
CREATE TABLE IF NOT EXISTS woml_run_summaries (
  run_id TEXT PRIMARY KEY,
  workflow_id TEXT NOT NULL,
  status TEXT NOT NULL,
  started_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY (run_id) REFERENCES woml_runs(run_id)
);

CREATE INDEX IF NOT EXISTS woml_run_summaries_updated
  ON woml_run_summaries(updated_at DESC, run_id DESC);
"#;

const MIGRATE_RUNTIME_POLICY_SCHEMA_V12: &str = r#"
ALTER TABLE woml_run_summaries RENAME TO woml_run_summaries_v11;
DROP INDEX woml_run_summaries_updated;

CREATE TABLE woml_run_summaries (
  run_id TEXT PRIMARY KEY,
  workflow_id TEXT NOT NULL,
  status TEXT NOT NULL,
  admitted_at TEXT NOT NULL,
  started_at TEXT,
  updated_at TEXT NOT NULL,
  queue_name TEXT,
  waiting_for TEXT CHECK (waiting_for IS NULL OR waiting_for IN ('concurrency', 'rate_limit')),
  eligible_at TEXT,
  FOREIGN KEY (run_id) REFERENCES woml_runs(run_id)
);

CREATE INDEX woml_run_summaries_updated
  ON woml_run_summaries(updated_at DESC, run_id DESC);

DROP TABLE IF EXISTS woml_scheduler_claims;
DROP TABLE IF EXISTS woml_runtime_policy_starts;
DROP TABLE IF EXISTS woml_runtime_policy_queue;
DROP TABLE IF EXISTS woml_runtime_policy_bindings;

CREATE TABLE woml_runtime_policy_bindings (
  run_id TEXT PRIMARY KEY,
  workflow_id TEXT NOT NULL,
  definition_hash TEXT NOT NULL,
  policy_hash TEXT NOT NULL,
  queue_name TEXT NOT NULL,
  admitted_at TEXT NOT NULL,
  occurrence_sequence INTEGER NOT NULL CHECK (occurrence_sequence >= 0),
  FOREIGN KEY (run_id) REFERENCES woml_runs(run_id),
  FOREIGN KEY (definition_hash) REFERENCES woml_definitions(definition_hash)
);

CREATE INDEX woml_runtime_policy_bindings_workflow
  ON woml_runtime_policy_bindings(workflow_id, admitted_at, occurrence_sequence, run_id);

CREATE TABLE woml_runtime_policy_queue (
  run_id TEXT PRIMARY KEY,
  workflow_id TEXT NOT NULL,
  queue_name TEXT NOT NULL,
  admitted_at TEXT NOT NULL,
  occurrence_sequence INTEGER NOT NULL CHECK (occurrence_sequence >= 0),
  FOREIGN KEY (run_id) REFERENCES woml_runs(run_id)
);

CREATE INDEX woml_runtime_policy_queue_order
  ON woml_runtime_policy_queue(queue_name, admitted_at, occurrence_sequence, run_id);

CREATE TABLE woml_runtime_policy_starts (
  run_id TEXT PRIMARY KEY,
  workflow_id TEXT NOT NULL,
  started_at TEXT NOT NULL,
  FOREIGN KEY (run_id) REFERENCES woml_runs(run_id)
);

CREATE INDEX woml_runtime_policy_starts_window
  ON woml_runtime_policy_starts(workflow_id, started_at);

CREATE TABLE woml_scheduler_claims (
  run_id TEXT PRIMARY KEY,
  workflow_id TEXT NOT NULL,
  owner_id TEXT NOT NULL,
  claim_id TEXT NOT NULL UNIQUE,
  claimed_at TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  FOREIGN KEY (run_id) REFERENCES woml_runs(run_id)
);

CREATE INDEX woml_scheduler_claims_expiry
  ON woml_scheduler_claims(expires_at);

DROP TABLE woml_run_summaries_v11;
"#;

const CREATE_DURABLE_STATE_SCHEMA_V13: &str = r#"
CREATE TABLE IF NOT EXISTS woml_state_entries (
  scope_digest TEXT NOT NULL,
  key_digest TEXT NOT NULL,
  key_text TEXT NOT NULL,
  value_json TEXT NOT NULL,
  value_bytes INTEGER NOT NULL CHECK (value_bytes BETWEEN 1 AND 262144),
  version INTEGER NOT NULL CHECK (version BETWEEN 1 AND 9007199254740991),
  updated_at TEXT NOT NULL,
  PRIMARY KEY (scope_digest, key_text),
  UNIQUE (scope_digest, key_digest)
);

CREATE INDEX IF NOT EXISTS woml_state_entries_scope
  ON woml_state_entries(scope_digest, key_digest);

CREATE TABLE IF NOT EXISTS woml_state_mutations (
  operation_key TEXT PRIMARY KEY,
  scope_digest TEXT NOT NULL,
  operation_name TEXT NOT NULL,
  operation TEXT NOT NULL CHECK (operation IN ('set', 'delete', 'increment', 'set_if_absent')),
  key_digest TEXT NOT NULL,
  input_digest TEXT NOT NULL,
  result_json TEXT NOT NULL,
  result_digest TEXT NOT NULL,
  committed_version INTEGER CHECK (
    committed_version IS NULL OR committed_version BETWEEN 1 AND 9007199254740991
  ),
  committed_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS woml_state_mutations_scope_key
  ON woml_state_mutations(scope_digest, key_digest, committed_at, operation_key);

CREATE TRIGGER IF NOT EXISTS woml_state_mutations_no_update
BEFORE UPDATE ON woml_state_mutations
BEGIN
  SELECT RAISE(ABORT, 'WOML durable state mutation results are immutable');
END;

CREATE TRIGGER IF NOT EXISTS woml_state_mutations_no_delete
BEFORE DELETE ON woml_state_mutations
BEGIN
  SELECT RAISE(ABORT, 'WOML durable state mutation results are immutable');
END;

CREATE TABLE IF NOT EXISTS woml_state_quotas (
  scope_digest TEXT PRIMARY KEY,
  live_keys INTEGER NOT NULL CHECK (live_keys BETWEEN 0 AND 10000),
  value_bytes INTEGER NOT NULL CHECK (value_bytes BETWEEN 0 AND 67108864)
);
"#;

const CREATE_PRODUCTION_RUNTIME_SCHEMA_V14: &str = r#"
CREATE TABLE IF NOT EXISTS woml_runtime_owner (
  singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
  deployment_id TEXT NOT NULL,
  activation_id TEXT NOT NULL,
  runtime_instance_id TEXT NOT NULL,
  heartbeat_at TEXT NOT NULL,
  lease_expires_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS woml_maintenance_lease (
  singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
  lease_id TEXT NOT NULL UNIQUE,
  operation TEXT NOT NULL CHECK (operation IN ('backup', 'restore', 'retention', 'checkpoint', 'vacuum')),
  owner_id TEXT NOT NULL,
  expires_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS woml_last_verified_backup (
  singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
  backup_id TEXT NOT NULL,
  completed_at TEXT NOT NULL,
  verified INTEGER NOT NULL CHECK (verified = 1)
);
"#;

const CREATE_DURABLE_WAIT_SCHEMA_V15: &str = r#"
CREATE TABLE IF NOT EXISTS woml_durable_waits (
  run_id TEXT NOT NULL,
  wait_id TEXT NOT NULL,
  iteration_identity TEXT NOT NULL DEFAULT '',
  node_id TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('duration', 'until', 'signal')),
  registered_at TEXT NOT NULL,
  deadline TEXT,
  signals_json TEXT,
  correlation_digest TEXT,
  state TEXT NOT NULL CHECK (state IN ('registered', 'satisfied', 'timed_out', 'cancelled')),
  resolution_event_id TEXT UNIQUE,
  resolved_at TEXT,
  PRIMARY KEY (run_id, wait_id, iteration_identity),
  FOREIGN KEY (run_id) REFERENCES woml_runs(run_id),
  CHECK (
    (state = 'registered' AND resolution_event_id IS NULL AND resolved_at IS NULL)
    OR
    (state != 'registered' AND resolution_event_id IS NOT NULL AND resolved_at IS NOT NULL)
  )
);

CREATE INDEX IF NOT EXISTS woml_durable_waits_due
  ON woml_durable_waits(state, deadline, run_id, wait_id, iteration_identity);

CREATE INDEX IF NOT EXISTS woml_durable_waits_active_target
  ON woml_durable_waits(run_id, wait_id, state, iteration_identity);

CREATE TABLE IF NOT EXISTS woml_external_signals (
  signal_id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL,
  wait_id TEXT NOT NULL,
  iteration_identity TEXT NOT NULL DEFAULT '',
  signal TEXT NOT NULL,
  received_at TEXT NOT NULL,
  idempotency_digest TEXT NOT NULL,
  request_digest TEXT NOT NULL,
  correlation_digest TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  authentication_class TEXT NOT NULL,
  consumed_at TEXT,
  UNIQUE (run_id, wait_id, iteration_identity, signal_id),
  FOREIGN KEY (run_id) REFERENCES woml_runs(run_id)
);

CREATE INDEX IF NOT EXISTS woml_external_signals_target
  ON woml_external_signals(run_id, wait_id, iteration_identity, received_at, signal_id);

CREATE TABLE IF NOT EXISTS woml_signal_idempotency (
  idempotency_digest TEXT PRIMARY KEY,
  request_digest TEXT NOT NULL,
  signal_id TEXT NOT NULL UNIQUE,
  created_at TEXT NOT NULL,
  FOREIGN KEY (signal_id) REFERENCES woml_external_signals(signal_id)
);
"#;

const CREATE_LOCK_SCHEMA_V16: &str = r#"
CREATE TABLE IF NOT EXISTS woml_lock_resources (
  key TEXT PRIMARY KEY,
  fencing_high_water INTEGER NOT NULL CHECK (fencing_high_water >= 0),
  owner_lease_id TEXT UNIQUE,
  owner_run_id TEXT,
  owner_node_execution_id TEXT,
  owner_fencing_token INTEGER,
  acquired_at TEXT,
  expires_at TEXT,
  updated_at TEXT NOT NULL,
  CHECK (
    (owner_lease_id IS NULL AND owner_run_id IS NULL AND owner_node_execution_id IS NULL
      AND owner_fencing_token IS NULL AND acquired_at IS NULL AND expires_at IS NULL)
    OR
    (owner_lease_id IS NOT NULL AND owner_run_id IS NOT NULL AND owner_node_execution_id IS NOT NULL
      AND owner_fencing_token IS NOT NULL AND acquired_at IS NOT NULL AND expires_at IS NOT NULL)
  ),
  FOREIGN KEY (owner_run_id) REFERENCES woml_runs(run_id)
);

CREATE TABLE IF NOT EXISTS woml_lock_waiters (
  waiter_id TEXT PRIMARY KEY,
  key TEXT NOT NULL,
  run_id TEXT NOT NULL,
  node_execution_id TEXT NOT NULL,
  sequence INTEGER NOT NULL CHECK (sequence >= 1),
  registered_at TEXT NOT NULL,
  deadline TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('waiting', 'held', 'released', 'timed_out', 'cancelled', 'lost')),
  input_digest TEXT NOT NULL,
  UNIQUE (run_id, node_execution_id),
  FOREIGN KEY (run_id) REFERENCES woml_runs(run_id),
  FOREIGN KEY (key) REFERENCES woml_lock_resources(key)
);

CREATE UNIQUE INDEX IF NOT EXISTS woml_lock_resources_one_owner
  ON woml_lock_resources(key) WHERE owner_lease_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS woml_lock_waiters_fair_queue
  ON woml_lock_waiters(key, status, sequence, waiter_id);
CREATE INDEX IF NOT EXISTS woml_lock_waiters_recovery
  ON woml_lock_waiters(status, deadline, run_id, node_execution_id);
"#;

const CREATE_CORRECTION_SCHEMA_V17: &str = r#"
CREATE TABLE IF NOT EXISTS woml_saga_scopes (
  run_id TEXT NOT NULL,
  saga_id TEXT NOT NULL,
  phase TEXT NOT NULL CHECK (phase IN ('forward', 'forward_succeeded', 'correcting', 'correction_completed', 'manual_recovery')),
  original_outcome TEXT CHECK (original_outcome IS NULL OR original_outcome IN ('success', 'failure', 'cancel')),
  started_sequence INTEGER NOT NULL CHECK (started_sequence >= 1),
  committed_sequence INTEGER,
  updated_at TEXT NOT NULL,
  model_digest TEXT NOT NULL,
  PRIMARY KEY (run_id, saga_id),
  FOREIGN KEY (run_id) REFERENCES woml_runs(run_id)
);

CREATE TABLE IF NOT EXISTS woml_correction_actions (
  run_id TEXT NOT NULL,
  saga_id TEXT NOT NULL,
  action_id TEXT NOT NULL,
  action_kind TEXT NOT NULL CHECK (action_kind IN ('compensation', 'cleanup')),
  for_step_execution_id TEXT,
  completion_sequence INTEGER,
  status TEXT NOT NULL CHECK (status IN ('registered', 'started', 'retry_scheduled', 'succeeded', 'failed', 'interrupted', 'manual_recovery')),
  attempts INTEGER NOT NULL CHECK (attempts BETWEEN 0 AND 10),
  next_attempt_at TEXT,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (run_id, saga_id, action_id),
  FOREIGN KEY (run_id, saga_id) REFERENCES woml_saga_scopes(run_id, saga_id)
);

CREATE TABLE IF NOT EXISTS woml_correction_attempts (
  run_id TEXT NOT NULL,
  saga_id TEXT NOT NULL,
  action_id TEXT NOT NULL,
  attempt INTEGER NOT NULL CHECK (attempt BETWEEN 1 AND 10),
  invocation_id TEXT NOT NULL UNIQUE,
  status TEXT NOT NULL CHECK (status IN ('started', 'succeeded', 'failed', 'interrupted')),
  started_at TEXT NOT NULL,
  finished_at TEXT,
  failure_code TEXT,
  PRIMARY KEY (run_id, saga_id, action_id, attempt),
  FOREIGN KEY (run_id, saga_id, action_id) REFERENCES woml_correction_actions(run_id, saga_id, action_id)
);

CREATE TABLE IF NOT EXISTS woml_correction_manual_recovery (
  run_id TEXT NOT NULL,
  saga_id TEXT NOT NULL,
  action_id TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('required', 'externally_resolved')),
  reason_code TEXT NOT NULL,
  created_at TEXT NOT NULL,
  resolved_at TEXT,
  PRIMARY KEY (run_id, saga_id, action_id),
  FOREIGN KEY (run_id, saga_id, action_id) REFERENCES woml_correction_actions(run_id, saga_id, action_id)
);

CREATE INDEX IF NOT EXISTS woml_correction_actions_pending
  ON woml_correction_actions(status, next_attempt_at, run_id, saga_id);
CREATE INDEX IF NOT EXISTS woml_correction_attempts_active
  ON woml_correction_attempts(status, run_id, saga_id, action_id);
CREATE INDEX IF NOT EXISTS woml_correction_manual_recovery_pending
  ON woml_correction_manual_recovery(status, created_at, run_id, saga_id);
"#;

const CREATE_RICH_APPROVAL_SCHEMA_V18: &str = r#"
CREATE TABLE IF NOT EXISTS woml_approval_reviews (
  run_id TEXT NOT NULL,
  approval_id TEXT NOT NULL,
  request_id TEXT NOT NULL,
  definition_hash TEXT NOT NULL,
  workflow_version TEXT NOT NULL,
  review_json TEXT NOT NULL,
  review_digest TEXT NOT NULL,
  required_approvals INTEGER NOT NULL CHECK(required_approvals BETWEEN 1 AND 20),
  required_roles_json TEXT NOT NULL,
  directory_version TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('pending','approved','rejected','expired','cancelled','invalidated')),
  created_at TEXT NOT NULL,
  resolved_at TEXT,
  PRIMARY KEY(run_id, approval_id),
  UNIQUE(run_id, request_id),
  UNIQUE(request_id),
  FOREIGN KEY(run_id) REFERENCES woml_runs(run_id)
);

CREATE TABLE IF NOT EXISTS woml_approval_reviewer_decisions (
  run_id TEXT NOT NULL,
  request_id TEXT NOT NULL,
  principal_id TEXT NOT NULL,
  decision TEXT NOT NULL CHECK(decision IN ('approved','rejected')),
  provider TEXT NOT NULL,
  provider_account_id TEXT NOT NULL,
  provider_actor_id TEXT NOT NULL,
  roles_json TEXT NOT NULL,
  decided_at TEXT NOT NULL,
  PRIMARY KEY(run_id, request_id, principal_id),
  FOREIGN KEY(run_id, request_id) REFERENCES woml_approval_reviews(run_id, request_id)
);

CREATE TABLE IF NOT EXISTS woml_approval_artifact_pins (
  run_id TEXT NOT NULL,
  request_id TEXT NOT NULL,
  artifact_id TEXT NOT NULL,
  artifact_sha256 TEXT NOT NULL,
  released_at TEXT,
  PRIMARY KEY(run_id, request_id, artifact_id),
  FOREIGN KEY(run_id, request_id) REFERENCES woml_approval_reviews(run_id, request_id)
);

CREATE INDEX IF NOT EXISTS woml_approval_reviews_pending
  ON woml_approval_reviews(status, created_at, run_id, request_id);
CREATE INDEX IF NOT EXISTS woml_approval_decisions_request
  ON woml_approval_reviewer_decisions(run_id, request_id, decided_at, principal_id);
CREATE INDEX IF NOT EXISTS woml_approval_pins_live
  ON woml_approval_artifact_pins(artifact_id, released_at);

CREATE TRIGGER IF NOT EXISTS woml_approval_reviews_no_delete
BEFORE DELETE ON woml_approval_reviews BEGIN
  SELECT RAISE(ABORT, 'WOML rich approval reviews are retention-managed');
END;
CREATE TRIGGER IF NOT EXISTS woml_approval_reviews_immutable
BEFORE UPDATE OF run_id, approval_id, request_id, definition_hash, workflow_version,
  review_json, review_digest, required_approvals, required_roles_json,
  directory_version, created_at ON woml_approval_reviews BEGIN
  SELECT RAISE(ABORT, 'WOML rich approval review bindings are immutable');
END;
CREATE TRIGGER IF NOT EXISTS woml_approval_decisions_no_update
BEFORE UPDATE ON woml_approval_reviewer_decisions BEGIN
  SELECT RAISE(ABORT, 'WOML rich approval decisions are immutable');
END;
CREATE TRIGGER IF NOT EXISTS woml_approval_decisions_no_delete
BEFORE DELETE ON woml_approval_reviewer_decisions BEGIN
  SELECT RAISE(ABORT, 'WOML rich approval decisions are retention-managed');
END;
CREATE TRIGGER IF NOT EXISTS woml_approval_pins_immutable
BEFORE UPDATE OF run_id, request_id, artifact_id, artifact_sha256
ON woml_approval_artifact_pins BEGIN
  SELECT RAISE(ABORT, 'WOML rich approval artifact identities are immutable');
END;
"#;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeOwnerLease {
  pub deployment_id: String,
  pub activation_id: String,
  pub runtime_instance_id: String,
  pub heartbeat_at: DateTime<Utc>,
  pub lease_expires_at: DateTime<Utc>,
}

const CREATE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS woml_store_metadata (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS woml_definitions (
  definition_hash TEXT PRIMARY KEY,
  workflow_id TEXT NOT NULL,
  schema_version INTEGER NOT NULL,
  model_json TEXT NOT NULL,
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS woml_runs (
  run_id TEXT PRIMARY KEY,
  workflow_id TEXT NOT NULL,
  definition_hash TEXT NOT NULL,
  created_at TEXT NOT NULL,
  FOREIGN KEY (definition_hash) REFERENCES woml_definitions(definition_hash)
);

CREATE TABLE IF NOT EXISTS woml_run_events (
  run_id TEXT NOT NULL,
  sequence INTEGER NOT NULL CHECK (sequence >= 1),
  event_id TEXT NOT NULL UNIQUE,
  event_schema_version INTEGER NOT NULL,
  event_json TEXT NOT NULL,
  PRIMARY KEY (run_id, sequence),
  FOREIGN KEY (run_id) REFERENCES woml_runs(run_id)
);

CREATE INDEX IF NOT EXISTS woml_run_events_event_id
  ON woml_run_events(event_id);

CREATE TRIGGER IF NOT EXISTS woml_definitions_no_update
BEFORE UPDATE ON woml_definitions
BEGIN
  SELECT RAISE(ABORT, 'WOML compiled definitions are immutable');
END;

CREATE TRIGGER IF NOT EXISTS woml_definitions_no_delete
BEFORE DELETE ON woml_definitions
BEGIN
  SELECT RAISE(ABORT, 'WOML compiled definitions are immutable');
END;

CREATE TRIGGER IF NOT EXISTS woml_runs_no_update
BEFORE UPDATE ON woml_runs
BEGIN
  SELECT RAISE(ABORT, 'WOML run bindings are immutable');
END;

CREATE TRIGGER IF NOT EXISTS woml_runs_no_delete
BEFORE DELETE ON woml_runs
BEGIN
  SELECT RAISE(ABORT, 'WOML run bindings are immutable');
END;

CREATE TRIGGER IF NOT EXISTS woml_run_events_no_update
BEFORE UPDATE ON woml_run_events
BEGIN
  SELECT RAISE(ABORT, 'WOML run events are append-only');
END;

CREATE TRIGGER IF NOT EXISTS woml_run_events_no_delete
BEFORE DELETE ON woml_run_events
BEGIN
  SELECT RAISE(ABORT, 'WOML run events are append-only');
END;
"#;

const CREATE_APPROVAL_SCHEMA_V2: &str = r#"
CREATE TABLE woml_approval_tokens (
  token_id TEXT PRIMARY KEY,
  secret_hash BLOB NOT NULL CHECK (length(secret_hash) = 32),
  request_id TEXT NOT NULL,
  run_id TEXT NOT NULL,
  approval_id TEXT NOT NULL,
  issued_at TEXT NOT NULL,
  credential_expires_at TEXT NOT NULL,
  FOREIGN KEY (run_id) REFERENCES woml_runs(run_id)
);

CREATE INDEX woml_approval_tokens_request
  ON woml_approval_tokens(run_id, approval_id, request_id);

CREATE TRIGGER woml_approval_tokens_no_update
BEFORE UPDATE ON woml_approval_tokens
BEGIN
  SELECT RAISE(ABORT, 'WOML approval credentials are append-only');
END;

CREATE TRIGGER woml_approval_tokens_no_delete
BEFORE DELETE ON woml_approval_tokens
BEGIN
  SELECT RAISE(ABORT, 'WOML approval credentials are append-only');
END;
"#;

const CREATE_NOTIFICATION_SCHEMA_V3: &str = r#"
CREATE TABLE woml_notification_capabilities (
  capability_id TEXT PRIMARY KEY,
  secret_hash BLOB NOT NULL CHECK (length(secret_hash) = 32),
  attempt_id TEXT NOT NULL UNIQUE,
  request_id TEXT NOT NULL,
  run_id TEXT NOT NULL,
  approval_id TEXT NOT NULL,
  delivery_id TEXT NOT NULL,
  issued_at TEXT NOT NULL,
  credential_expires_at TEXT NOT NULL,
  FOREIGN KEY (run_id) REFERENCES woml_runs(run_id)
);

CREATE INDEX woml_notification_capabilities_delivery
  ON woml_notification_capabilities(run_id, approval_id, request_id, delivery_id);

CREATE TRIGGER woml_notification_capabilities_no_update
BEFORE UPDATE ON woml_notification_capabilities
BEGIN
  SELECT RAISE(ABORT, 'WOML notification capabilities are append-only');
END;

CREATE TRIGGER woml_notification_capabilities_no_delete
BEFORE DELETE ON woml_notification_capabilities
BEGIN
  SELECT RAISE(ABORT, 'WOML notification capabilities are append-only');
END;
"#;

const CREATE_TRIGGER_SCHEMA_V4: &str = r#"
CREATE TABLE woml_trigger_occurrences (
  occurrence_id TEXT PRIMARY KEY,
  workflow_id TEXT NOT NULL,
  trigger_id TEXT NOT NULL,
  trigger_handler TEXT NOT NULL,
  definition_hash TEXT NOT NULL,
  source_identity_hash TEXT NOT NULL,
  payload_hash TEXT NOT NULL,
  received_at TEXT NOT NULL,
  run_id TEXT NOT NULL UNIQUE,
  UNIQUE (workflow_id, trigger_id, source_identity_hash),
  FOREIGN KEY (definition_hash) REFERENCES woml_definitions(definition_hash),
  FOREIGN KEY (run_id) REFERENCES woml_runs(run_id)
);

CREATE INDEX woml_trigger_occurrences_run
  ON woml_trigger_occurrences(run_id);

CREATE TRIGGER woml_trigger_occurrences_no_update
BEFORE UPDATE ON woml_trigger_occurrences
BEGIN
  SELECT RAISE(ABORT, 'WOML trigger occurrences are immutable');
END;

CREATE TRIGGER woml_trigger_occurrences_no_delete
BEFORE DELETE ON woml_trigger_occurrences
BEGIN
  SELECT RAISE(ABORT, 'WOML trigger occurrences are immutable');
END;
"#;

const CREATE_SCHEDULE_SCHEMA_V5: &str = r#"
CREATE TABLE woml_schedule_cursors (
  workflow_id TEXT NOT NULL,
  trigger_id TEXT NOT NULL,
  definition_hash TEXT NOT NULL,
  cron TEXT NOT NULL,
  timezone TEXT NOT NULL,
  on_missed TEXT NOT NULL CHECK (on_missed IN ('skip', 'run-once')),
  next_scheduled_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (workflow_id, trigger_id),
  FOREIGN KEY (definition_hash) REFERENCES woml_definitions(definition_hash)
);

CREATE INDEX woml_schedule_cursors_definition
  ON woml_schedule_cursors(definition_hash);
"#;

const CREATE_INTERVAL_SCHEMA_V6: &str = r#"
CREATE TABLE woml_interval_cursors (
  workflow_id TEXT NOT NULL,
  trigger_id TEXT NOT NULL,
  definition_hash TEXT NOT NULL,
  every_ms INTEGER NOT NULL CHECK (every_ms BETWEEN 1000 AND 2592000000),
  on_missed TEXT NOT NULL CHECK (on_missed IN ('skip', 'run-once')),
  anchor_at TEXT NOT NULL,
  next_sequence INTEGER NOT NULL CHECK (next_sequence >= 1),
  next_scheduled_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (workflow_id, trigger_id),
  FOREIGN KEY (definition_hash) REFERENCES woml_definitions(definition_hash)
);

CREATE INDEX woml_interval_cursors_definition
  ON woml_interval_cursors(definition_hash);
"#;

const CREATE_INTERNAL_EVENT_SCHEMA_V7: &str = r#"
CREATE TABLE woml_internal_event_publications (
  publication_id TEXT PRIMARY KEY,
  parent_run_id TEXT NOT NULL,
  event_name TEXT NOT NULL,
  payload_hash TEXT NOT NULL,
  depth INTEGER NOT NULL CHECK (depth BETWEEN 1 AND 32),
  emitted_at TEXT NOT NULL,
  FOREIGN KEY (parent_run_id) REFERENCES woml_runs(run_id)
);

CREATE TABLE woml_internal_event_deliveries (
  publication_id TEXT NOT NULL,
  workflow_id TEXT NOT NULL,
  trigger_id TEXT NOT NULL,
  run_id TEXT NOT NULL UNIQUE,
  PRIMARY KEY (publication_id, workflow_id, trigger_id),
  FOREIGN KEY (publication_id) REFERENCES woml_internal_event_publications(publication_id),
  FOREIGN KEY (run_id) REFERENCES woml_runs(run_id)
);

CREATE INDEX woml_internal_event_deliveries_run
  ON woml_internal_event_deliveries(run_id);

CREATE TRIGGER woml_internal_event_publications_no_update
BEFORE UPDATE ON woml_internal_event_publications
BEGIN
  SELECT RAISE(ABORT, 'WOML internal event publications are immutable');
END;

CREATE TRIGGER woml_internal_event_publications_no_delete
BEFORE DELETE ON woml_internal_event_publications
BEGIN
  SELECT RAISE(ABORT, 'WOML internal event publications are immutable');
END;

CREATE TRIGGER woml_internal_event_deliveries_no_update
BEFORE UPDATE ON woml_internal_event_deliveries
BEGIN
  SELECT RAISE(ABORT, 'WOML internal event deliveries are immutable');
END;

CREATE TRIGGER woml_internal_event_deliveries_no_delete
BEFORE DELETE ON woml_internal_event_deliveries
BEGIN
  SELECT RAISE(ABORT, 'WOML internal event deliveries are immutable');
END;
"#;

const CREATE_MODULE_ARTIFACT_SCHEMA_V8: &str = r#"
CREATE TABLE woml_definition_module_artifacts (
  definition_hash TEXT NOT NULL,
  module_name TEXT NOT NULL,
  bundle_digest TEXT NOT NULL,
  source_map_digest TEXT NOT NULL,
  exports_json TEXT NOT NULL,
  bundle TEXT NOT NULL,
  source_map TEXT NOT NULL,
  created_at TEXT NOT NULL,
  PRIMARY KEY (definition_hash, module_name),
  FOREIGN KEY (definition_hash) REFERENCES woml_definitions(definition_hash)
);

CREATE INDEX woml_definition_module_artifacts_bundle
  ON woml_definition_module_artifacts(bundle_digest);

CREATE TRIGGER woml_definition_module_artifacts_no_update
BEFORE UPDATE ON woml_definition_module_artifacts
BEGIN
  SELECT RAISE(ABORT, 'WOML definition module artifacts are immutable');
END;

CREATE TRIGGER woml_definition_module_artifacts_no_delete
BEFORE DELETE ON woml_definition_module_artifacts
BEGIN
  SELECT RAISE(ABORT, 'WOML definition module artifacts are immutable');
END;
"#;

const CREATE_WORKFLOW_CALL_SCHEMA_V9: &str = r#"
CREATE TABLE woml_workflow_calls (
  call_key TEXT PRIMARY KEY,
  parent_run_id TEXT NOT NULL,
  parent_node_id TEXT NOT NULL,
  parent_attempt INTEGER NOT NULL CHECK (parent_attempt BETWEEN 1 AND 10),
  target_workflow_id TEXT NOT NULL,
  target_definition_hash TEXT NOT NULL,
  child_run_id TEXT NOT NULL UNIQUE,
  payload_digest TEXT NOT NULL,
  depth INTEGER NOT NULL CHECK (depth BETWEEN 1 AND 32),
  state TEXT NOT NULL CHECK (state IN ('admitted', 'running', 'succeeded', 'failed')),
  admitted_at TEXT NOT NULL,
  FOREIGN KEY (parent_run_id) REFERENCES woml_runs(run_id),
  FOREIGN KEY (child_run_id) REFERENCES woml_runs(run_id),
  FOREIGN KEY (target_definition_hash) REFERENCES woml_definitions(definition_hash)
);

CREATE INDEX woml_workflow_calls_parent
  ON woml_workflow_calls(parent_run_id, parent_node_id, parent_attempt);

CREATE INDEX woml_workflow_calls_target
  ON woml_workflow_calls(target_workflow_id, target_definition_hash);

CREATE TRIGGER woml_workflow_calls_identity_no_update
BEFORE UPDATE ON woml_workflow_calls
WHEN OLD.call_key != NEW.call_key
  OR OLD.parent_run_id != NEW.parent_run_id
  OR OLD.parent_node_id != NEW.parent_node_id
  OR OLD.parent_attempt != NEW.parent_attempt
  OR OLD.target_workflow_id != NEW.target_workflow_id
  OR OLD.target_definition_hash != NEW.target_definition_hash
  OR OLD.child_run_id != NEW.child_run_id
  OR OLD.payload_digest != NEW.payload_digest
  OR OLD.depth != NEW.depth
  OR OLD.admitted_at != NEW.admitted_at
BEGIN
  SELECT RAISE(ABORT, 'WOML workflow call identity is immutable');
END;

CREATE TRIGGER woml_workflow_calls_no_delete
BEFORE DELETE ON woml_workflow_calls
BEGIN
  SELECT RAISE(ABORT, 'WOML workflow calls are durable');
END;
"#;

const CREATE_WORKFLOW_RUNTIME_SCHEMA_V10: &str = r#"
CREATE TABLE woml_workflow_runtime_routes (
  workflow_id TEXT PRIMARY KEY,
  definition_hash TEXT NOT NULL,
  runtime_id TEXT NOT NULL,
  endpoint TEXT NOT NULL,
  session_credential_hash TEXT NOT NULL,
  lease_expires_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY (definition_hash) REFERENCES woml_definitions(definition_hash)
);

CREATE INDEX woml_workflow_runtime_routes_runtime
  ON woml_workflow_runtime_routes(runtime_id);
"#;

const SUSPENDED_POLICY_WORKFLOW_ID: &str = "__woml_policy_wait__";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunDefinitionBinding {
  pub run_id: String,
  pub workflow_id: String,
  pub definition_hash: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TriggerAdmissionRequest {
  pub workflow_id: String,
  pub definition_hash: String,
  pub trigger_id: String,
  pub trigger_handler: String,
  pub source_identity: String,
  pub payload: Map<String, Value>,
  pub received_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerOccurrence {
  pub occurrence_schema_version: u32,
  pub occurrence_id: String,
  pub workflow_id: String,
  pub trigger_id: String,
  pub trigger_handler: String,
  pub definition_hash: String,
  pub source_identity_hash: String,
  pub payload_hash: String,
  pub received_at: DateTime<Utc>,
  pub run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerAdmissionOutcome {
  pub occurrence_id: String,
  pub run_id: String,
  pub duplicate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleCursorRegistration {
  pub workflow_id: String,
  pub trigger_id: String,
  pub definition_hash: String,
  pub cron: String,
  pub timezone: String,
  pub on_missed: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleCursor {
  pub workflow_id: String,
  pub trigger_id: String,
  pub definition_hash: String,
  pub cron: String,
  pub timezone: String,
  pub on_missed: String,
  pub next_scheduled_at: DateTime<Utc>,
  pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleCursorRegistrationOutcome {
  pub cursor: ScheduleCursor,
  pub initialized: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntervalCursorRegistration {
  pub workflow_id: String,
  pub trigger_id: String,
  pub definition_hash: String,
  pub every_ms: u64,
  pub on_missed: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntervalCursor {
  pub workflow_id: String,
  pub trigger_id: String,
  pub definition_hash: String,
  pub every_ms: u64,
  pub on_missed: String,
  pub anchor_at: DateTime<Utc>,
  pub next_sequence: u64,
  pub next_scheduled_at: DateTime<Utc>,
  pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntervalCursorRegistrationOutcome {
  pub cursor: IntervalCursor,
  pub initialized: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerRecoveryWork {
  pub occurrence: TriggerOccurrence,
  pub trigger: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InternalEventAdmissionRequest {
  pub publication_id: String,
  pub parent_run_id: String,
  pub event_name: String,
  pub trigger: TriggerAdmissionRequest,
  pub emitted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InternalEventAdmissionOutcome {
  pub depth: u32,
  pub occurrence: TriggerAdmissionOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedApprovalToken {
  pub token: String,
  pub token_id: String,
  pub request_id: String,
  pub run_id: String,
  pub approval_id: String,
  pub issued_at: DateTime<Utc>,
  pub credential_expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalTokenBinding {
  pub token_id: String,
  pub request_id: String,
  pub run_id: String,
  pub approval_id: String,
  pub issued_at: DateTime<Utc>,
  pub credential_expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NotificationDeliveryWork {
  pub run_id: String,
  pub approval_id: String,
  pub request_id: String,
  pub delivery_id: String,
  pub provider: String,
  pub destination: String,
  pub credentials: BTreeMap<String, String>,
  pub provider_id: Option<String>,
  pub message: Option<crate::model::ValueExpression>,
  pub template_name: Option<String>,
  pub language: Option<String>,
  pub attempt: u32,
  pub attempt_id: String,
  pub idempotency_key: String,
  pub decision_capability: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationProviderDeliveryResult {
  Succeeded(ProviderMessageIdentity),
  Failed(NotificationSafeFailure),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationUpdateWork {
  pub run_id: String,
  pub approval_id: String,
  pub request_id: String,
  pub delivery_id: String,
  pub provider: String,
  pub credentials: BTreeMap<String, String>,
  pub provider_message: ProviderMessageIdentity,
  pub resolution: crate::event::NotificationResolution,
  pub update_id: String,
  pub idempotency_key: String,
  pub attempt: u32,
  pub attempt_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationProviderUpdateResult {
  Succeeded,
  Failed(NotificationSafeFailure),
}

pub trait NotificationProviderAdapter {
  fn deliver(&mut self, work: &NotificationDeliveryWork) -> NotificationProviderDeliveryResult;
  fn update(&mut self, work: &NotificationUpdateWork) -> NotificationProviderUpdateResult;
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationDispatchReport {
  pub attempted: usize,
  pub succeeded: usize,
  pub failed: usize,
  pub run_failed: bool,
  pub updates_attempted: usize,
  pub updates_succeeded: usize,
  pub updates_failed: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecisionOutcomeStatus {
  Accepted,
  AlreadyResolved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalDecisionOutcome {
  pub status: ApprovalDecisionOutcomeStatus,
  pub run_id: String,
  pub approval_id: String,
  pub request_id: String,
  pub decision: ApprovalDecision,
  pub source: ApprovalDecisionSource,
  pub decided_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalTimeoutSettlementStatus {
  Settled,
  AlreadyResolved,
  NotDue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalTimeoutSettlement {
  pub status: ApprovalTimeoutSettlementStatus,
  pub run_id: String,
  pub approval_id: String,
  pub request_id: String,
  pub resolution: Option<ApprovalResolution>,
  pub settled_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryReport {
  pub inspected_runs: usize,
  pub recovered_runs: usize,
  pub interrupted_attempts: usize,
  pub resumable_runs: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InfrastructurePolicyRunBindingV1 {
  pub run_id: String,
  pub policy_id: String,
  pub policy_version: String,
  pub policy_digest: String,
  pub canonical_bundle_reference: String,
  pub signature_status: InfrastructurePolicySignatureStatusV1,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub signing_key_id: Option<String>,
  pub admitted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InfrastructurePolicyApprovalEvidenceV1 {
  pub request_id: String,
  pub review_digest: String,
  pub principal_id: String,
  pub roles: BTreeSet<String>,
  pub decided_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InfrastructurePolicyArtifactEvidenceV1 {
  pub artifact_id: String,
  pub sha256: String,
  pub review_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InfrastructurePolicyLockEvidenceV1 {
  pub key: String,
  pub lease_id: String,
  pub fencing_token: String,
  pub acquired_at: DateTime<Utc>,
  pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct InfrastructurePolicyEvidenceSnapshotV1 {
  pub approvals: Vec<InfrastructurePolicyApprovalEvidenceV1>,
  pub artifacts: Vec<InfrastructurePolicyArtifactEvidenceV1>,
  pub locks: Vec<InfrastructurePolicyLockEvidenceV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicRunStatus {
  NotStarted,
  Queued,
  Running,
  Waiting,
  Cancelling,
  Finalizing,
  Succeeded,
  Failed,
  Cancelled,
}

impl PublicRunStatus {
  const fn as_str(self) -> &'static str {
    match self {
      Self::NotStarted => "not_started",
      Self::Queued => "queued",
      Self::Running => "running",
      Self::Waiting => "waiting",
      Self::Cancelling => "cancelling",
      Self::Finalizing => "finalizing",
      Self::Succeeded => "succeeded",
      Self::Failed => "failed",
      Self::Cancelled => "cancelled",
    }
  }

  fn parse(value: &str) -> Option<Self> {
    Some(match value {
      "not_started" => Self::NotStarted,
      "queued" => Self::Queued,
      "running" => Self::Running,
      "waiting" => Self::Waiting,
      "cancelling" => Self::Cancelling,
      "finalizing" => Self::Finalizing,
      "succeeded" => Self::Succeeded,
      "failed" => Self::Failed,
      "cancelled" => Self::Cancelled,
      _ => return None,
    })
  }
}

impl From<RunStatus> for PublicRunStatus {
  fn from(value: RunStatus) -> Self {
    match value {
      RunStatus::NotStarted => Self::NotStarted,
      RunStatus::Queued => Self::Queued,
      RunStatus::Running => Self::Running,
      RunStatus::Waiting => Self::Waiting,
      RunStatus::Cancelling => Self::Cancelling,
      RunStatus::Finalizing => Self::Finalizing,
      RunStatus::Succeeded => Self::Succeeded,
      RunStatus::Failed => Self::Failed,
      RunStatus::Cancelled => Self::Cancelled,
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummaryV1 {
  pub run_id: String,
  pub workflow_id: String,
  pub status: PublicRunStatus,
  pub started_at: DateTime<Utc>,
  pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunListV1 {
  pub profile: &'static str,
  pub runs: Vec<RunSummaryV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyWaitingFor {
  Concurrency,
  RateLimit,
}

impl PolicyWaitingFor {
  fn parse(value: &str) -> Option<Self> {
    match value {
      "concurrency" => Some(Self::Concurrency),
      "rate_limit" => Some(Self::RateLimit),
      _ => None,
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummaryV2 {
  pub run_id: String,
  pub workflow_id: String,
  pub status: PublicRunStatus,
  pub admitted_at: DateTime<Utc>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub started_at: Option<DateTime<Utc>>,
  pub updated_at: DateTime<Utc>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub queue: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub waiting_for: Option<PolicyWaitingFor>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub eligible_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunListV2 {
  pub profile: &'static str,
  pub runs: Vec<RunSummaryV2>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeObservationV1 {
  pub profile: &'static str,
  pub status_totals: BTreeMap<String, u64>,
  pub retrying_run_ids: Vec<String>,
  pub approval_waiting_run_ids: Vec<String>,
  pub retries_total: u64,
  pub triggers_total: u64,
  pub workflow_calls_active: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionHookV2 {
  pub hook_id: String,
  pub subject_kind: crate::event::LifecycleSubjectKind,
  pub subject_id: String,
  pub status: crate::projection::LifecycleHookStatus,
  pub failed_actions: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionCancellationV2 {
  pub requested: bool,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionV2 {
  pub profile: &'static str,
  pub run_id: String,
  pub workflow_id: String,
  pub status: PublicRunStatus,
  pub business_outcome: InspectedBusinessOutcome,
  pub lifecycle_status: crate::projection::LifecycleStatus,
  pub hooks: Vec<RunInspectionHookV2>,
  pub warnings: Vec<crate::event::LifecycleWarning>,
  pub cancellation: RunInspectionCancellationV2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionPolicyV3 {
  pub queue: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub waiting_for: Option<PolicyWaitingFor>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub eligible_at: Option<DateTime<Utc>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub timeout_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionV3 {
  pub profile: &'static str,
  pub run_id: String,
  pub workflow_id: String,
  pub status: PublicRunStatus,
  pub business_outcome: InspectedBusinessOutcome,
  pub lifecycle_status: crate::projection::LifecycleStatus,
  pub hooks: Vec<RunInspectionHookV2>,
  pub warnings: Vec<crate::event::LifecycleWarning>,
  pub cancellation: RunInspectionCancellationV2,
  pub policy: RunInspectionPolicyV3,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectedForkStatusV4 {
  Active,
  Waiting,
  Succeeded,
  Failed,
  Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectedForkJoinModeV4 {
  All,
  Selected,
  None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionForkCountsV4 {
  pub opened: u32,
  pub active: u32,
  pub waiting: u32,
  pub succeeded: u32,
  pub failed: u32,
  pub cancelled: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionForkJoinV4 {
  pub mode: InspectedForkJoinModeV4,
  pub branch_ids: Vec<String>,
  pub status: crate::projection::ForkJoinStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionForkV4 {
  pub fork_id: String,
  pub status: InspectedForkStatusV4,
  pub join: RunInspectionForkJoinV4,
  pub branches: RunInspectionForkCountsV4,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionForksV4 {
  pub counts: RunInspectionForkCountsV4,
  pub items: Vec<RunInspectionForkV4>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionV4 {
  pub profile: &'static str,
  pub run_id: String,
  pub workflow_id: String,
  pub status: PublicRunStatus,
  pub business_outcome: InspectedBusinessOutcome,
  pub lifecycle_status: crate::projection::LifecycleStatus,
  pub hooks: Vec<RunInspectionHookV2>,
  pub warnings: Vec<crate::event::LifecycleWarning>,
  pub cancellation: RunInspectionCancellationV2,
  pub policy: RunInspectionPolicyV3,
  pub forks: RunInspectionForksV4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum InspectedReusableKindV5 {
  Step,
  NotificationProvider,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectedReusableStatusV5 {
  Pending,
  Running,
  Succeeded,
  Failed,
  Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectedReusableLifecycleStatusV5 {
  Idle,
  Running,
  Completed,
  CompletedWithWarnings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionReusableItemV5 {
  pub invocation_id: String,
  pub alias: String,
  pub definition_digest_prefix: String,
  pub kind: InspectedReusableKindV5,
  pub status: InspectedReusableStatusV5,
  pub lifecycle_status: InspectedReusableLifecycleStatusV5,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionReusableCountsV5 {
  pub pending: u32,
  pub running: u32,
  pub succeeded: u32,
  pub failed: u32,
  pub cancelled: u32,
  pub completed_with_warnings: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionReusableDefinitionsV5 {
  pub counts: RunInspectionReusableCountsV5,
  pub items: Vec<RunInspectionReusableItemV5>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionV5 {
  pub profile: &'static str,
  pub run_id: String,
  pub workflow_id: String,
  pub status: PublicRunStatus,
  pub business_outcome: InspectedBusinessOutcome,
  pub lifecycle_status: crate::projection::LifecycleStatus,
  pub hooks: Vec<RunInspectionHookV2>,
  pub warnings: Vec<crate::event::LifecycleWarning>,
  pub cancellation: RunInspectionCancellationV2,
  pub policy: RunInspectionPolicyV3,
  pub forks: RunInspectionForksV4,
  pub reusable_definitions: RunInspectionReusableDefinitionsV5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectedForEachStatusV6 {
  Active,
  Succeeded,
  Failed,
  Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionForEachCountsV6 {
  pub opened: u32,
  pub active: u32,
  pub succeeded: u32,
  pub failed: u32,
  pub cancelled: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionForEachItemV6 {
  pub for_each_id: String,
  pub status: InspectedForEachStatusV6,
  pub total: u32,
  pub succeeded: u32,
  pub failed: u32,
  pub skipped: u32,
  pub active: u32,
  pub pending: u32,
  pub concurrency: u32,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub failed_index: Option<u32>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub failed_node_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionForEachV6 {
  pub counts: RunInspectionForEachCountsV6,
  pub items: Vec<RunInspectionForEachItemV6>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionV6 {
  pub profile: &'static str,
  pub run_id: String,
  pub workflow_id: String,
  pub status: PublicRunStatus,
  pub business_outcome: InspectedBusinessOutcome,
  pub lifecycle_status: crate::projection::LifecycleStatus,
  pub hooks: Vec<RunInspectionHookV2>,
  pub warnings: Vec<crate::event::LifecycleWarning>,
  pub cancellation: RunInspectionCancellationV2,
  pub policy: RunInspectionPolicyV3,
  pub forks: RunInspectionForksV4,
  pub reusable_definitions: RunInspectionReusableDefinitionsV5,
  pub for_each: RunInspectionForEachV6,
}

pub const RUN_INSPECTION_RICH_APPROVAL_LIMIT: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionRichApprovalV11 {
  pub approval_id: String,
  pub request_id: String,
  pub status: String,
  pub review_digest: String,
  pub review: ReviewPayloadV1,
  pub required: u32,
  pub received: u32,
  pub required_roles: Vec<String>,
  pub represented_roles: Vec<String>,
  pub directory_version: String,
  pub requested_at: DateTime<Utc>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub deadline: Option<DateTime<Utc>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub resolved_at: Option<DateTime<Utc>>,
  pub decisions: Vec<RichApprovalDecisionInspection>,
  pub total_decisions: u32,
  pub decisions_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionRichApprovalsV11 {
  pub pending: u32,
  pub items: Vec<RunInspectionRichApprovalV11>,
  pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInspectionV11 {
  pub profile: &'static str,
  pub run_id: String,
  pub workflow_id: String,
  pub status: PublicRunStatus,
  pub business_outcome: InspectedBusinessOutcome,
  pub lifecycle_status: crate::projection::LifecycleStatus,
  pub hooks: Vec<RunInspectionHookV2>,
  pub warnings: Vec<crate::event::LifecycleWarning>,
  pub cancellation: RunInspectionCancellationV2,
  pub policy: RunInspectionPolicyV3,
  pub forks: RunInspectionForksV4,
  pub reusable_definitions: RunInspectionReusableDefinitionsV5,
  pub for_each: RunInspectionForEachV6,
  pub approvals: RunInspectionRichApprovalsV11,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkRecoveryBranchV1 {
  pub fork_id: String,
  pub branch_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkRecoveryWorkV1 {
  pub profile: &'static str,
  pub openable_fork_ids: Vec<String>,
  pub pending_branches: Vec<ForkRecoveryBranchV1>,
  pub joinable_fork_ids: Vec<String>,
  pub pending_choice_ids: Vec<String>,
  pub ambiguous_active_node_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerClaimV1 {
  pub profile: &'static str,
  pub run_id: String,
  pub workflow_id: String,
  pub owner_id: String,
  pub claim_id: String,
  pub claimed_at: DateTime<Utc>,
  pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DurableWaitRecordV1 {
  pub run_id: String,
  pub wait_id: String,
  pub node_id: String,
  pub kind: crate::DurableWaitKind,
  pub registered_at: DateTime<Utc>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub deadline: Option<DateTime<Utc>>,
  pub state: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub resolved_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectedWaitStatusV1 {
  Pending,
  Signaled,
  Elapsed,
  TimedOut,
  Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectedWaitV1 {
  pub run_id: String,
  pub wait_id: String,
  pub name: String,
  pub kind: crate::DurableWaitKind,
  pub status: InspectedWaitStatusV1,
  pub registered_at: DateTime<Utc>,
  #[serde(skip_serializing_if = "Vec::is_empty")]
  pub signals: Vec<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub correlation_digest: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub deadline: Option<DateTime<Utc>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub resolved_at: Option<DateTime<Utc>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub winning_signal: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitInspectionV1 {
  pub profile: &'static str,
  pub run_id: String,
  pub waits: Vec<InspectedWaitV1>,
  pub pending_waits: u32,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub oldest_pending_wait_age_ms: Option<u64>,
}

pub const CORRECTION_INSPECTION_PROFILE: &str = "woml.correction-inspection/v1";
pub const CORRECTION_RECOVERY_REQUEST_PROFILE: &str = "woml.correction-recovery-request/v1";
pub const CORRECTION_RECOVERY_RESULT_PROFILE: &str = "woml.correction-recovery-result/v1";
pub const CORRECTION_OBSERVATION_PROFILE: &str = "woml.correction-observation/v1";
pub const CORRECTION_RECOVERY_BATCH_LIMIT: usize = 100;
pub const CORRECTION_UNRESOLVED_STORE_LIMIT: usize = 10_000;
const MAX_CORRECTION_INSPECTION_ROWS: usize = 100;
const MAX_CORRECTION_INSPECTION_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionSagaInspectionV1 {
  pub saga_id: String,
  pub status: String,
  pub registered_compensations: u32,
  pub completed_compensations: u32,
  pub completed_cleanup: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnresolvedCorrectionActionV1 {
  pub action_id: String,
  pub kind: crate::CorrectiveActionKind,
  pub code: String,
  pub ambiguous: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionInspectionV1 {
  pub profile: &'static str,
  pub run_id: String,
  pub phase: String,
  pub original_outcome: String,
  pub sagas: Vec<CorrectionSagaInspectionV1>,
  pub unresolved_actions: Vec<UnresolvedCorrectionActionV1>,
  pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CorrectionRecoveryActionV1 {
  Retry,
  ExternallyResolve,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CorrectionRecoveryRequestV1 {
  pub profile: String,
  pub run_id: String,
  pub action: CorrectionRecoveryActionV1,
  pub action_id: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CorrectionRecoveryStatusV1 {
  RetryScheduled,
  ExternallyResolved,
  Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionRecoveryResultV1 {
  pub profile: &'static str,
  pub run_id: String,
  pub action_id: String,
  pub status: CorrectionRecoveryStatusV1,
  pub audit_recorded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionRecoveryCandidateV1 {
  pub run_id: String,
  pub saga_id: String,
  pub action_id: String,
  pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionObservationV1 {
  pub profile: &'static str,
  pub open_sagas: u64,
  pub pending_actions: u64,
  pub active_attempts: u64,
  pub manual_recovery_actions: u64,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub oldest_manual_recovery_age_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectedLockStatusV1 {
  Waiting,
  Held,
  Released,
  TimedOut,
  Cancelled,
  Lost,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LockInspectionV1 {
  pub profile: &'static str,
  pub run_id: String,
  pub lock_id: String,
  pub key: String,
  pub status: InspectedLockStatusV1,
  pub registered_at: DateTime<Utc>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub acquired_at: Option<DateTime<Utc>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub released_at: Option<DateTime<Utc>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub deadline: Option<DateTime<Utc>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub expires_at: Option<DateTime<Utc>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub fencing_token: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub waited_ms: Option<u64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub held_ms: Option<u64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub failure_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LockObservationV1 {
  pub profile: &'static str,
  pub held_locks: u64,
  pub waiting_acquisitions: u64,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub oldest_waiter_age_ms: Option<u64>,
  pub renewal_failures: u64,
  pub expired_takeovers: u64,
  pub acquisition_timeouts: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LockRecoveryKindV1 {
  AcquisitionTimeout,
  LeaseExpiry,
}

/// Internal scheduler work exposed as a bounded, deterministic batch. It
/// contains coordination identities only; lease credentials, input digests,
/// script data, and payloads never cross this boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LockRecoveryCandidateV1 {
  pub profile: &'static str,
  pub kind: LockRecoveryKindV1,
  pub key: String,
  pub run_id: String,
  pub execution_identity: String,
  pub due_at: DateTime<Utc>,
}

pub const EXTERNAL_SIGNAL_ADMISSION_PROFILE: &str = "woml.external-signal-admission/v1";
pub const EXTERNAL_SIGNAL_RESULT_PROFILE: &str = "woml.signal-result/v1";
pub const EXTERNAL_SIGNAL_MAX_PAYLOAD_BYTES: usize = 1_048_576;
pub const EXTERNAL_SIGNAL_MAX_PAYLOAD_DEPTH: usize = 32;
pub const EXTERNAL_SIGNAL_MAX_EARLY_PER_RUN: usize = 64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExternalSignalAdmissionRequest {
  pub profile: String,
  pub signal: String,
  pub run_id: String,
  pub wait_id: String,
  pub correlation: String,
  pub idempotency_key: String,
  pub payload: Value,
  pub received_at: DateTime<Utc>,
  pub authentication_class: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalSignalAdmissionStatus {
  Accepted,
  Duplicate,
  IdentityConflict,
  AlreadyResolved,
  TargetNotFound,
  Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalSignalResolution {
  Early,
  Resolved,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExternalSignalAdmissionOutcome {
  pub profile: &'static str,
  pub status: ExternalSignalAdmissionStatus,
  pub signal_id: String,
  pub run_id: String,
  pub wait_id: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub resolution: Option<ExternalSignalResolution>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub code: Option<String>,
}

impl ExternalSignalAdmissionOutcome {
  pub fn resumes_run(&self) -> bool {
    self.status == ExternalSignalAdmissionStatus::Accepted
      && self.resolution == Some(ExternalSignalResolution::Resolved)
  }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SignalWaitTimeoutSettlement {
  NotDue { deadline: DateTime<Utc> },
  Continued { projection: RunProjection },
  Failed { projection: RunProjection },
  AlreadyResolved { projection: RunProjection },
}

#[derive(Debug, Clone, PartialEq)]
pub enum TimerWaitSettlement {
  NotDue { deadline: DateTime<Utc> },
  Resolved { projection: RunProjection },
  AlreadyResolved { projection: RunProjection },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyClaimWaitReason {
  Concurrency,
  RateLimit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PolicyExecutionClaimResult {
  Claimed {
    claim: SchedulerClaimV1,
    #[serde(rename = "firstStart")]
    first_start: bool,
  },
  Waiting {
    #[serde(rename = "runId")]
    run_id: String,
    #[serde(rename = "workflowId")]
    workflow_id: String,
    queue: String,
    reason: PolicyClaimWaitReason,
    #[serde(rename = "eligibleAt", skip_serializing_if = "Option::is_none")]
    eligible_at: Option<DateTime<Utc>>,
  },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectedBusinessOutcome {
  Undecided,
  Succeeded,
  Failed,
  Cancelled,
}

impl From<Option<crate::event::BusinessOutcome>> for InspectedBusinessOutcome {
  fn from(value: Option<crate::event::BusinessOutcome>) -> Self {
    match value {
      None => Self::Undecided,
      Some(crate::event::BusinessOutcome::Succeeded) => Self::Succeeded,
      Some(crate::event::BusinessOutcome::Failed) => Self::Failed,
      Some(crate::event::BusinessOutcome::Cancelled) => Self::Cancelled,
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunCancellationStatus {
  Accepted,
  AlreadyRequested,
  AlreadyCancelled,
  Rejected,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RunTimeoutSettlement {
  NotConfigured,
  NotDue { deadline_at: DateTime<Utc> },
  LostRace { projection: RunProjection },
  TimedOut { projection: RunProjection },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RunCancellationCode {
  WomlRunNotFound,
  WomlRunOutcomeAlreadyDecided,
  WomlRunAlreadyTerminal,
  WomlRunControlVersionUnsupported,
  WomlRunCancellationFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunCancellationResult {
  pub profile: &'static str,
  pub command_id: String,
  pub run_id: String,
  pub status: RunCancellationStatus,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub code: Option<RunCancellationCode>,
}

#[derive(Debug, Error)]
pub enum DurableStoreError {
  #[error(transparent)]
  Sqlite(#[from] rusqlite::Error),
  #[error(transparent)]
  Json(#[from] serde_json::Error),
  #[error(transparent)]
  Fold(#[from] FoldError),
  #[error(transparent)]
  InvalidModel(#[from] ModelValidationError),
  #[error("unsupported WOML event-store schema version {0:?}")]
  UnsupportedStoreVersion(String),
  #[error("compiled definition {0:?} is not registered")]
  DefinitionNotFound(String),
  #[error("definition hash {0:?} is already bound to a different compiled model")]
  DefinitionConflict(String),
  #[error("run {0:?} does not exist")]
  RunNotFound(String),
  #[error("run {0:?} already exists and cannot be rebound")]
  RunAlreadyExists(String),
  #[error("trigger {trigger_id:?} is not registered for workflow {workflow_id:?}")]
  TriggerNotFound {
    workflow_id: String,
    trigger_id: String,
  },
  #[error("trigger admission does not match its registered workflow definition")]
  TriggerDefinitionMismatch,
  #[error("trigger admission handler does not match the compiled trigger")]
  TriggerHandlerMismatch,
  #[error("the trigger source identity is already bound to a different payload")]
  TriggerIdempotencyConflict,
  #[error("workflow {0:?} has an active run bound to a different definition or runtime policy")]
  RuntimePolicyDefinitionConflict(String),
  #[error("the durable runtime-policy queue reached its local safety ceiling")]
  RuntimePolicyQueueFull,
  #[error("run {0:?} already has a live scheduler claim")]
  SchedulerClaimConflict(String),
  #[error("run {0:?} requires fail-closed recovery before it can be scheduled")]
  SchedulerRecoveryRequired(String),
  #[error("the internal event publication identity is already bound to different data")]
  InternalEventIdempotencyConflict,
  #[error("the internal event would repeat a subscriber already present in its lineage")]
  InternalEventCycle,
  #[error("the internal event exceeds the maximum lineage depth")]
  InternalEventDepthExceeded,
  #[error("the workflow call identity is already bound to different input")]
  WorkflowCallIdempotencyConflict,
  #[error("the workflow call exceeds the maximum lineage depth")]
  WorkflowCallDepthExceeded,
  #[error("the workflow call would create a recursive call cycle")]
  WorkflowCallCycle,
  #[error("the workflow call target does not match its registered definition")]
  WorkflowCallDefinitionMismatch,
  #[error("stored workflow call history is contradictory: {0}")]
  WorkflowCallHistoryInvalid(String),
  #[error("workflow ID {0:?} already has a live local owner")]
  WorkflowRuntimeDuplicateOwner(String),
  #[error("deployment {deployment_id:?} is already owned by runtime {runtime_instance_id:?}")]
  DeploymentRuntimeOwned {
    deployment_id: String,
    runtime_instance_id: String,
  },
  #[error("the durable schedule cursor changed before this operation could commit")]
  ScheduleCursorConflict,
  #[error("the durable interval cursor changed before this operation could commit")]
  IntervalCursorConflict,
  #[error("stored trigger occurrence history is contradictory: {0}")]
  TriggerHistoryInvalid(String),
  #[error("stored event is invalid: {0}")]
  InvalidStoredEvent(String),
  #[error("approval token is invalid")]
  InvalidApprovalToken,
  #[error("approval token has expired")]
  ExpiredApprovalToken,
  #[error("approval deadline has already been reached")]
  ApprovalExpired,
  #[error("a different human approval decision is already durable")]
  ApprovalDecisionConflict,
  #[error("the rich approval request was not found")]
  RichApprovalNotFound,
  #[error("the rich approval binding digest does not match")]
  RichApprovalBindingMismatch,
  #[error("the canonical reviewer already recorded a conflicting decision")]
  RichApprovalReviewerConflict,
  #[error(transparent)]
  ReviewerDirectory(#[from] ReviewerDirectoryError),
  #[error("the lock acquisition identity is already bound to different input")]
  LockIdempotencyConflict,
  #[error("the durable lock wait queue reached its safety ceiling")]
  LockWaitLimit,
  #[error("the durable lock fencing-token space is exhausted")]
  LockFencingExhausted,
  #[error("stored lock history is contradictory: {0}")]
  LockHistoryInvalid(String),
  #[error("approval timeout settlement requires a request with a deadline")]
  ApprovalHasNoDeadline,
  #[error("{0}")]
  Contract(String),
}

#[derive(Debug)]
pub struct DurableEventStore {
  pub(crate) connection: Connection,
  projection_cache: RefCell<HashMap<String, ProjectionFoldState>>,
}

pub const RICH_APPROVAL_INSPECTION_DECISION_LIMIT: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RichApprovalDecisionAdmission {
  pub run_id: String,
  pub request_id: String,
  pub review_digest: String,
  pub principal_id: String,
  pub decision: ApprovalDecision,
  pub provider: String,
  pub provider_account_id: String,
  pub provider_actor_id: String,
  pub roles: Vec<String>,
  pub decided_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RichApprovalIdentityDecision {
  pub run_id: String,
  pub request_id: String,
  pub review_digest: String,
  pub decision: ApprovalDecision,
  pub identity: ReviewerIdentityClaimV1,
  pub decided_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RichApprovalDecisionStatus {
  Pending,
  Approved,
  Rejected,
  Idempotent,
  AlreadyResolved,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RichApprovalDecisionOutcome {
  pub status: RichApprovalDecisionStatus,
  pub required: u32,
  pub received: u32,
  pub represented_roles: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RichApprovalDecisionInspection {
  pub principal_id: String,
  pub decision: ApprovalDecision,
  pub provider: String,
  pub provider_account_id: String,
  pub roles: Vec<String>,
  pub decided_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RichApprovalInspectionV1 {
  pub run_id: String,
  pub approval_id: String,
  pub request_id: String,
  pub review_digest: String,
  pub review: ReviewPayloadV1,
  pub required: u32,
  pub required_roles: Vec<String>,
  pub directory_version: String,
  pub status: String,
  pub created_at: DateTime<Utc>,
  pub resolved_at: Option<DateTime<Utc>>,
  pub decisions: Vec<RichApprovalDecisionInspection>,
  pub total_decisions: u32,
  pub decisions_truncated: bool,
}

#[derive(Debug, Clone)]
struct RichReviewRow {
  approval_id: String,
  request_id: String,
  review_digest: String,
  required: u32,
  required_roles: Vec<String>,
  directory_version: String,
  status: String,
}

#[derive(Debug, Clone)]
struct RichQuorumState {
  received: u32,
  represented_roles: Vec<String>,
}

fn rich_review_row(
  connection: &Connection,
  run_id: &str,
  request_id: &str,
) -> Result<Option<RichReviewRow>, DurableStoreError> {
  let row: Option<(String, String, String, i64, String, String, String)> = connection
    .query_row(
      "SELECT approval_id, request_id, review_digest, required_approvals, required_roles_json,
              directory_version, status
       FROM woml_approval_reviews WHERE run_id = ?1 AND request_id = ?2",
      params![run_id, request_id],
      |row| {
        Ok((
          row.get(0)?,
          row.get(1)?,
          row.get(2)?,
          row.get(3)?,
          row.get(4)?,
          row.get(5)?,
          row.get(6)?,
        ))
      },
    )
    .optional()?;
  row
    .map(
      |(
        approval_id,
        request_id,
        review_digest,
        required,
        required_roles_json,
        directory_version,
        status,
      )| {
        Ok(RichReviewRow {
          approval_id,
          request_id,
          review_digest,
          required: u32::try_from(required).map_err(|_| {
            DurableStoreError::Contract(
              "Stored rich approval quorum is outside the frozen range.".to_string(),
            )
          })?,
          required_roles: serde_json::from_str(&required_roles_json)?,
          directory_version,
          status,
        })
      },
    )
    .transpose()
}

fn rich_quorum_state(
  connection: &Connection,
  review: &RichReviewRow,
) -> Result<RichQuorumState, DurableStoreError> {
  let mut statement = connection.prepare(
    "SELECT decision, roles_json FROM woml_approval_reviewer_decisions
     WHERE request_id = ?1 ORDER BY principal_id",
  )?;
  let rows = statement
    .query_map([&review.request_id], |row| {
      Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?
    .collect::<Result<Vec<_>, _>>()?;
  let mut received = 0u32;
  let mut represented = BTreeMap::<String, ()>::new();
  for (decision, roles_json) in rows {
    if decision != "approved" {
      continue;
    }
    received = received.saturating_add(1);
    let roles: Vec<String> = serde_json::from_str(&roles_json)?;
    for role in roles {
      if review.required_roles.contains(&role) {
        represented.insert(role, ());
      }
    }
  }
  Ok(RichQuorumState {
    received,
    represented_roles: represented.into_keys().collect(),
  })
}

fn settle_rich_review_row(
  transaction: &Transaction<'_>,
  run_id: &str,
  request_id: &str,
  status: &str,
  settled_at: DateTime<Utc>,
) -> Result<(), DurableStoreError> {
  let changed = transaction.execute(
    "UPDATE woml_approval_reviews SET status = ?3, resolved_at = ?4
     WHERE run_id = ?1 AND request_id = ?2 AND status = 'pending'",
    params![run_id, request_id, status, settled_at.to_rfc3339()],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::ApprovalDecisionConflict);
  }
  transaction.execute(
    "UPDATE woml_approval_artifact_pins SET released_at = ?3
     WHERE run_id = ?1 AND request_id = ?2 AND released_at IS NULL",
    params![run_id, request_id, settled_at.to_rfc3339()],
  )?;
  Ok(())
}

fn validate_rich_decision_admission(
  admission: &RichApprovalDecisionAdmission,
) -> Result<(), DurableStoreError> {
  let valid_text = |value: &str, max: usize| {
    !value.is_empty()
      && value.len() <= max
      && !value.chars().any(|character| character.is_control())
  };
  if !valid_text(&admission.run_id, 320)
    || !admission.request_id.starts_with("aprreq_")
    || !is_definition_hash(&admission.review_digest)
    || !valid_text(&admission.principal_id, 128)
    || !valid_text(&admission.provider, 64)
    || !valid_text(&admission.provider_account_id, 256)
    || !valid_text(&admission.provider_actor_id, 256)
    || admission.roles.len() > crate::RICH_APPROVAL_MAX_ROLES
    || !admission.roles.windows(2).all(|pair| pair[0] < pair[1])
  {
    return Err(DurableStoreError::Contract(
      "The rich approval decision admission is invalid.".to_string(),
    ));
  }
  Ok(())
}

fn approval_decision_text(decision: ApprovalDecision) -> &'static str {
  match decision {
    ApprovalDecision::Approved => "approved",
    ApprovalDecision::Rejected => "rejected",
  }
}

fn parse_approval_decision(value: &str) -> Result<ApprovalDecision, DurableStoreError> {
  match value {
    "approved" => Ok(ApprovalDecision::Approved),
    "rejected" => Ok(ApprovalDecision::Rejected),
    _ => Err(DurableStoreError::Contract(
      "Stored rich approval decision is invalid.".to_string(),
    )),
  }
}

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>, DurableStoreError> {
  DateTime::parse_from_rfc3339(value)
    .map(|value| value.with_timezone(&Utc))
    .map_err(|_| DurableStoreError::Contract("Stored timestamp is invalid.".to_string()))
}

fn append_rich_approval_payloads(
  transaction: &Transaction<'_>,
  projection_cache: &RefCell<HashMap<String, ProjectionFoldState>>,
  run_id: &str,
  occurred_at: DateTime<Utc>,
  payloads: Vec<RunEventPayload>,
) -> Result<RunProjection, DurableStoreError> {
  let mut events = load_events(transaction, run_id)?;
  let mut projection_state = projection_state_for_events(projection_cache, run_id, &events)?;
  let version = events
    .first()
    .map(|event| event.event_schema_version)
    .ok_or_else(|| DurableStoreError::Contract("A rich approval requires a started run.".into()))?;
  if !matches!(
    version,
    crate::RUN_EVENT_SCHEMA_VERSION_V19 | crate::RUN_EVENT_SCHEMA_VERSION_V20
  ) {
    return Err(DurableStoreError::Contract(
      "Rich approval persistence requires Event v19 or later.".to_string(),
    ));
  }
  for payload in payloads {
    append_to_history_scoped_incremental(
      transaction,
      &mut events,
      &mut projection_state,
      run_id,
      generated_event_id(),
      occurred_at,
      version,
      None,
      payload,
      true,
    )?;
  }
  Ok(projection_state.projection().clone())
}

enum RunRecovery {
  Unchanged,
  Resumable,
  Recovered { interrupted_attempts: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockLeaseMaintenance {
  Renewed,
  Lost,
  NotHeld,
}

/// Compact store-owned state for one callback-scoped programmatic lease.
/// The session identity never enters public inspection; callers receive only
/// the key, fencing token, and acquisition time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgrammaticLockLease {
  pub session_id: String,
  pub key: String,
  pub fencing_token: String,
  pub acquired_at: DateTime<Utc>,
  pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgrammaticLockAcquireState {
  Waiting,
  Held,
  TimedOut,
  Cancelled,
  Lost,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgrammaticLockAcquire {
  pub state: ProgrammaticLockAcquireState,
  pub lease: Option<ProgrammaticLockLease>,
}

/// Frozen Store v16 safety ceilings. These limits bound one hot resource and
/// the total local durable backlog without preventing an uncontended lock
/// from being acquired immediately.
pub const LOCK_WAITERS_PER_KEY_LIMIT: usize = 1_024;
pub const LOCK_WAITERS_PER_STORE_LIMIT: usize = 10_000;
pub const LOCK_RECOVERY_BATCH_LIMIT: usize = 250;

pub const MAX_MODULE_ARTIFACT_BYTES: usize = 3 * 1024 * 1024;
pub const MAX_MODULE_ARTIFACT_SET_BYTES: usize = 32 * 1024 * 1024;

pub fn derive_lifecycle_hook_invocation_id(
  run_id: &str,
  hook_id: &str,
  subject_kind: crate::event::LifecycleSubjectKind,
  subject_id: &str,
) -> String {
  let subject_kind = match subject_kind {
    crate::event::LifecycleSubjectKind::Workflow => "workflow",
    crate::event::LifecycleSubjectKind::Step => "step",
  };
  let canonical = [run_id, hook_id, subject_kind, subject_id]
    .into_iter()
    .map(|part| format!("{}:{part}", part.len()))
    .collect::<Vec<_>>()
    .join("|");
  format!("sha256:{:x}", Sha256::digest(canonical.as_bytes()))
}

fn module_artifact_sha256(content: &str) -> String {
  format!("sha256:{:x}", Sha256::digest(content.as_bytes()))
}

fn reusable_lifecycle_artifact_name(action_id: &str) -> String {
  format!(
    "__woml_lifecycle__{}",
    module_artifact_sha256(action_id).trim_start_matches("sha256:")
  )
}

fn validate_definition_module_artifacts(
  workflow: &CompiledWorkflowDefinition,
  artifacts: &[RuntimeModuleArtifact],
) -> Result<(), DurableStoreError> {
  let module_artifacts = artifacts
    .iter()
    .filter(|artifact| !artifact.name.starts_with("__woml_"))
    .collect::<Vec<_>>();
  let provider_artifacts = artifacts
    .iter()
    .filter(|artifact| artifact.name.starts_with("__woml_provider__"))
    .collect::<Vec<_>>();
  let reusable_step_artifacts = artifacts
    .iter()
    .filter(|artifact| artifact.name.starts_with("__woml_reusable__"))
    .collect::<Vec<_>>();
  let reusable_lifecycle_artifacts = artifacts
    .iter()
    .filter(|artifact| artifact.name.starts_with("__woml_lifecycle__"))
    .collect::<Vec<_>>();
  let runtime_bindings = workflow
    .module_runtime
    .as_ref()
    .map_or(&[][..], |runtime| runtime.modules.as_slice());
  if runtime_bindings.len() != module_artifacts.len() {
    return Err(DurableStoreError::Contract(
      "Stored module artifacts do not match the compiled definition.".to_string(),
    ));
  }
  let mut total_bytes = 0usize;
  for (binding, artifact) in runtime_bindings.iter().zip(module_artifacts) {
    let bundle_bytes = artifact.bundle.len();
    let source_map_bytes = artifact.source_map.len();
    total_bytes = total_bytes
      .checked_add(bundle_bytes)
      .and_then(|value| value.checked_add(source_map_bytes))
      .ok_or_else(|| {
        DurableStoreError::Contract("Module artifact byte count overflowed.".to_string())
      })?;
    if binding.name != artifact.name
      || binding.bundle_digest != artifact.bundle_digest
      || binding.source_map_digest != artifact.source_map_digest
      || binding.exports != artifact.exports
      || module_artifact_sha256(&artifact.bundle) != artifact.bundle_digest
      || module_artifact_sha256(&artifact.source_map) != artifact.source_map_digest
      || bundle_bytes > MAX_MODULE_ARTIFACT_BYTES
      || source_map_bytes > MAX_MODULE_ARTIFACT_BYTES
    {
      return Err(DurableStoreError::Contract(format!(
        "Module artifact {:?} failed its immutable identity or size contract.",
        binding.name
      )));
    }
  }
  let provider_descriptors = workflow
    .reusable_definitions
    .iter()
    .flatten()
    .filter_map(|definition| match definition {
      crate::model::CompiledReusableInvocation::NotificationProvider {
        script_artifact_id, ..
      } => Some(script_artifact_id.as_str()),
      _ => None,
    })
    .collect::<HashSet<_>>();
  let mut provider_ids = HashSet::new();
  for artifact in provider_artifacts {
    let artifact_id = artifact.name.trim_start_matches("__woml_provider__");
    total_bytes = total_bytes
      .checked_add(artifact.bundle.len())
      .and_then(|value| value.checked_add(artifact.source_map.len()))
      .ok_or_else(|| {
        DurableStoreError::Contract("Provider artifact byte count overflowed.".to_string())
      })?;
    if !provider_descriptors.contains(artifact_id)
      || !provider_ids.insert(artifact_id)
      || !artifact.exports.is_empty()
      || !artifact.source_map.is_empty()
      || module_artifact_sha256(&artifact.bundle) != artifact.bundle_digest
      || module_artifact_sha256("") != artifact.source_map_digest
      || artifact.bundle.len() > MAX_MODULE_ARTIFACT_BYTES
    {
      return Err(DurableStoreError::Contract(format!(
        "Provider artifact {artifact_id:?} failed its immutable identity or size contract."
      )));
    }
  }
  if provider_ids.len() != provider_descriptors.len() {
    return Err(DurableStoreError::Contract(
      "Stored provider artifacts do not match the compiled definition.".to_string(),
    ));
  }
  let reusable_step_descriptors = workflow
    .reusable_definitions
    .iter()
    .flatten()
    .filter_map(|definition| match definition {
      crate::model::CompiledReusableInvocation::Step {
        script_artifact_id, ..
      } => Some(script_artifact_id.as_str()),
      _ => None,
    })
    .collect::<HashSet<_>>();
  let mut reusable_step_ids = HashSet::new();
  for artifact in reusable_step_artifacts {
    let artifact_id = artifact.name.trim_start_matches("__woml_reusable__");
    total_bytes = total_bytes
      .checked_add(artifact.bundle.len())
      .and_then(|value| value.checked_add(artifact.source_map.len()))
      .ok_or_else(|| {
        DurableStoreError::Contract("Custom-step artifact byte count overflowed.".to_string())
      })?;
    if !reusable_step_descriptors.contains(artifact_id)
      || !reusable_step_ids.insert(artifact_id)
      || !artifact.exports.is_empty()
      || !artifact.source_map.is_empty()
      || module_artifact_sha256(&artifact.bundle) != artifact.bundle_digest
      || module_artifact_sha256("") != artifact.source_map_digest
      || artifact.bundle.len() > MAX_MODULE_ARTIFACT_BYTES
    {
      return Err(DurableStoreError::Contract(format!(
        "Custom-step artifact {artifact_id:?} failed its immutable identity or size contract."
      )));
    }
  }
  if reusable_step_ids.len() != reusable_step_descriptors.len() {
    return Err(DurableStoreError::Contract(
      "Stored custom-step artifacts do not match the compiled definition.".to_string(),
    ));
  }
  let reusable_lifecycle_names = workflow
    .reusable_definitions
    .iter()
    .flatten()
    .flat_map(|definition| match definition {
      crate::model::CompiledReusableInvocation::Step { lifecycle, .. }
      | crate::model::CompiledReusableInvocation::NotificationProvider { lifecycle, .. } => {
        lifecycle
          .iter()
          .flat_map(|lifecycle| {
            lifecycle
              .on_success
              .iter()
              .chain(&lifecycle.on_error)
              .chain(&lifecycle.on_complete)
              .flat_map(|actions| actions.iter())
              .map(|action_id| reusable_lifecycle_artifact_name(action_id))
              .collect::<Vec<_>>()
          })
          .collect::<Vec<_>>()
      }
    })
    .collect::<HashSet<_>>();
  let mut found_lifecycle_names = HashSet::new();
  for artifact in reusable_lifecycle_artifacts {
    total_bytes = total_bytes
      .checked_add(artifact.bundle.len())
      .and_then(|value| value.checked_add(artifact.source_map.len()))
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "Reusable lifecycle artifact byte count overflowed.".to_string(),
        )
      })?;
    if !reusable_lifecycle_names.contains(&artifact.name)
      || !found_lifecycle_names.insert(artifact.name.as_str())
      || !artifact.exports.is_empty()
      || !artifact.source_map.is_empty()
      || module_artifact_sha256(&artifact.bundle) != artifact.bundle_digest
      || module_artifact_sha256("") != artifact.source_map_digest
      || artifact.bundle.len() > MAX_MODULE_ARTIFACT_BYTES
    {
      return Err(DurableStoreError::Contract(format!(
        "Reusable lifecycle artifact {:?} failed its immutable identity or size contract.",
        artifact.name
      )));
    }
  }
  if found_lifecycle_names.len() != reusable_lifecycle_names.len() {
    return Err(DurableStoreError::Contract(
      "Stored reusable lifecycle artifacts do not match the compiled definition.".to_string(),
    ));
  }
  if total_bytes > MAX_MODULE_ARTIFACT_SET_BYTES {
    return Err(DurableStoreError::Contract(format!(
      "Module artifacts exceed the {} byte definition limit.",
      MAX_MODULE_ARTIFACT_SET_BYTES
    )));
  }
  Ok(())
}

fn migrate_store_v1_to_v2(connection: &mut Connection) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_APPROVAL_SCHEMA_V2)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V2, STORE_SCHEMA_VERSION_V1],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v1-to-v2 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

fn migrate_store_v2_to_v3(connection: &mut Connection) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_NOTIFICATION_SCHEMA_V3)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V3, STORE_SCHEMA_VERSION_V2],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v2-to-v3 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

fn migrate_store_v3_to_v4(connection: &mut Connection) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_TRIGGER_SCHEMA_V4)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V4, STORE_SCHEMA_VERSION_V3],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v3-to-v4 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

fn migrate_store_v4_to_v5(connection: &mut Connection) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_SCHEDULE_SCHEMA_V5)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V5, STORE_SCHEMA_VERSION_V4],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v4-to-v5 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

fn migrate_store_v5_to_v6(connection: &mut Connection) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_INTERVAL_SCHEMA_V6)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V6, STORE_SCHEMA_VERSION_V5],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v5-to-v6 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

fn migrate_store_v6_to_v7(connection: &mut Connection) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_INTERNAL_EVENT_SCHEMA_V7)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V7, STORE_SCHEMA_VERSION_V6],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v6-to-v7 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

fn migrate_store_v7_to_v8(connection: &mut Connection) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_MODULE_ARTIFACT_SCHEMA_V8)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V8, STORE_SCHEMA_VERSION_V7],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v7-to-v8 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

fn migrate_store_v8_to_v9(connection: &mut Connection) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_WORKFLOW_CALL_SCHEMA_V9)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V9, STORE_SCHEMA_VERSION_V8],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v8-to-v9 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

fn migrate_store_v9_to_v10(connection: &mut Connection) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_WORKFLOW_RUNTIME_SCHEMA_V10)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V10, STORE_SCHEMA_VERSION_V9],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v9-to-v10 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

fn migrate_store_v10_to_v11(connection: &mut Connection) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_RUN_SUMMARY_SCHEMA_V11)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V11, STORE_SCHEMA_VERSION_V10],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v10-to-v11 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

fn migrate_store_v11_to_v12(connection: &mut Connection) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(MIGRATE_RUNTIME_POLICY_SCHEMA_V12)?;
  rebuild_run_summary_index(&transaction)?;
  rebuild_runtime_policy_indexes(&transaction)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V12, STORE_SCHEMA_VERSION_V11],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v11-to-v12 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

fn migrate_store_v12_to_v13(connection: &mut Connection) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_DURABLE_STATE_SCHEMA_V13)?;
  validate_store_v13_schema(&transaction)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V13, STORE_SCHEMA_VERSION_V12],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v12-to-v13 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

fn migrate_store_v13_to_v14(connection: &mut Connection) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_PRODUCTION_RUNTIME_SCHEMA_V14)?;
  validate_store_v14_schema(&transaction)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V14, STORE_SCHEMA_VERSION_V13],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v13-to-v14 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

fn migrate_store_v14_to_v15(connection: &mut Connection) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_DURABLE_WAIT_SCHEMA_V15)?;
  validate_store_v15_schema(&transaction)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V15, STORE_SCHEMA_VERSION_V14],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v14-to-v15 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

fn migrate_store_v15_to_v16(connection: &mut Connection) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_LOCK_SCHEMA_V16)?;
  validate_store_v16_schema(&transaction)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V16, STORE_SCHEMA_VERSION_V15],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v15-to-v16 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

fn migrate_store_v16_to_v17(connection: &mut Connection) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_CORRECTION_SCHEMA_V17)?;
  validate_store_v17_schema(&transaction)?;
  rebuild_correction_indexes(&transaction)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V17, STORE_SCHEMA_VERSION_V16],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v16-to-v17 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

fn migrate_store_v17_to_v18(connection: &mut Connection) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_RICH_APPROVAL_SCHEMA_V18)?;
  validate_store_v18_schema(&transaction)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V18, STORE_SCHEMA_VERSION_V17],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v17-to-v18 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

fn migrate_store_v18_to_v19(connection: &mut Connection) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_INFRASTRUCTURE_POLICY_SCHEMA_V19)?;
  validate_store_v19_schema(&transaction)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V19, STORE_SCHEMA_VERSION_V18],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v18-to-v19 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

fn migrate_store_v19_to_v20(connection: &mut Connection) -> Result<(), DurableStoreError> {
  let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  transaction.execute_batch(CREATE_REMOTE_DISPATCH_SCHEMA_V20)?;
  validate_remote_dispatch_schema(&transaction)?;
  let changed = transaction.execute(
    "UPDATE woml_store_metadata SET value = ?1 WHERE key = 'schema_version' AND value = ?2",
    [STORE_SCHEMA_VERSION_V20, STORE_SCHEMA_VERSION_V19],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Store v19-to-v20 migration could not update the schema version atomically.".to_string(),
    ));
  }
  transaction.commit()?;
  Ok(())
}

fn validate_store_v2_schema(connection: &Connection) -> Result<(), DurableStoreError> {
  for (object_type, name) in [
    ("table", "woml_approval_tokens"),
    ("index", "woml_approval_tokens_request"),
    ("trigger", "woml_approval_tokens_no_update"),
    ("trigger", "woml_approval_tokens_no_delete"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(
         SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2
       )",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v2 is missing required {object_type} {name:?}."
      )));
    }
  }
  Ok(())
}

fn validate_store_v3_schema(connection: &Connection) -> Result<(), DurableStoreError> {
  validate_store_v2_schema(connection)?;
  for (object_type, name) in [
    ("table", "woml_notification_capabilities"),
    ("index", "woml_notification_capabilities_delivery"),
    ("trigger", "woml_notification_capabilities_no_update"),
    ("trigger", "woml_notification_capabilities_no_delete"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v3 is missing required {object_type} {name:?}."
      )));
    }
  }
  Ok(())
}

fn validate_store_v4_schema(connection: &Connection) -> Result<(), DurableStoreError> {
  validate_store_v3_schema(connection)?;
  for (object_type, name) in [
    ("table", "woml_trigger_occurrences"),
    ("index", "woml_trigger_occurrences_run"),
    ("trigger", "woml_trigger_occurrences_no_update"),
    ("trigger", "woml_trigger_occurrences_no_delete"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v4 is missing required {object_type} {name:?}."
      )));
    }
  }
  Ok(())
}

fn validate_store_v5_schema(connection: &Connection) -> Result<(), DurableStoreError> {
  validate_store_v4_schema(connection)?;
  for (object_type, name) in [
    ("table", "woml_schedule_cursors"),
    ("index", "woml_schedule_cursors_definition"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v5 is missing required {object_type} {name:?}."
      )));
    }
  }
  Ok(())
}

fn validate_store_v6_schema(connection: &Connection) -> Result<(), DurableStoreError> {
  validate_store_v5_schema(connection)?;
  for (object_type, name) in [
    ("table", "woml_interval_cursors"),
    ("index", "woml_interval_cursors_definition"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v6 is missing required {object_type} {name:?}."
      )));
    }
  }
  Ok(())
}

fn validate_store_v7_schema(connection: &Connection) -> Result<(), DurableStoreError> {
  validate_store_v6_schema(connection)?;
  for (object_type, name) in [
    ("table", "woml_internal_event_publications"),
    ("table", "woml_internal_event_deliveries"),
    ("index", "woml_internal_event_deliveries_run"),
    ("trigger", "woml_internal_event_publications_no_update"),
    ("trigger", "woml_internal_event_publications_no_delete"),
    ("trigger", "woml_internal_event_deliveries_no_update"),
    ("trigger", "woml_internal_event_deliveries_no_delete"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v7 is missing required {object_type} {name:?}."
      )));
    }
  }
  Ok(())
}

fn validate_store_v8_schema(connection: &Connection) -> Result<(), DurableStoreError> {
  validate_store_v7_schema(connection)?;
  for (object_type, name) in [
    ("table", "woml_definition_module_artifacts"),
    ("index", "woml_definition_module_artifacts_bundle"),
    ("trigger", "woml_definition_module_artifacts_no_update"),
    ("trigger", "woml_definition_module_artifacts_no_delete"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v8 is missing required {object_type} {name:?}."
      )));
    }
  }
  Ok(())
}

fn validate_store_v9_schema(connection: &Connection) -> Result<(), DurableStoreError> {
  validate_store_v8_schema(connection)?;
  for (object_type, name) in [
    ("table", "woml_workflow_calls"),
    ("index", "woml_workflow_calls_parent"),
    ("index", "woml_workflow_calls_target"),
    ("trigger", "woml_workflow_calls_identity_no_update"),
    ("trigger", "woml_workflow_calls_no_delete"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v9 is missing required {object_type} {name:?}."
      )));
    }
  }
  Ok(())
}

fn validate_store_v10_schema(connection: &Connection) -> Result<(), DurableStoreError> {
  validate_store_v9_schema(connection)?;
  for (object_type, name) in [
    ("table", "woml_workflow_runtime_routes"),
    ("index", "woml_workflow_runtime_routes_runtime"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v10 is missing required {object_type} {name:?}."
      )));
    }
  }
  Ok(())
}

fn validate_store_v11_schema(connection: &Connection) -> Result<(), DurableStoreError> {
  validate_store_v10_schema(connection)?;
  for (object_type, name) in [
    ("table", "woml_run_summaries"),
    ("index", "woml_run_summaries_updated"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v11 is missing required {object_type} {name:?}."
      )));
    }
  }

  let mut statement = connection.prepare("PRAGMA table_info(woml_run_summaries)")?;
  let actual_columns = statement
    .query_map([], |row| row.get::<_, String>(1))?
    .collect::<Result<Vec<_>, _>>()?;
  let expected_columns = [
    "run_id",
    "workflow_id",
    "status",
    "started_at",
    "updated_at",
  ];
  if actual_columns != expected_columns {
    return Err(DurableStoreError::Contract(format!(
      "Store v11 woml_run_summaries columns do not match the frozen schema: expected {expected_columns:?}, found {actual_columns:?}."
    )));
  }
  Ok(())
}

fn validate_store_v12_schema(connection: &Connection) -> Result<(), DurableStoreError> {
  validate_store_v10_schema(connection)?;
  for (object_type, name) in [
    ("table", "woml_run_summaries"),
    ("index", "woml_run_summaries_updated"),
    ("table", "woml_runtime_policy_bindings"),
    ("index", "woml_runtime_policy_bindings_workflow"),
    ("table", "woml_runtime_policy_queue"),
    ("index", "woml_runtime_policy_queue_order"),
    ("table", "woml_runtime_policy_starts"),
    ("index", "woml_runtime_policy_starts_window"),
    ("table", "woml_scheduler_claims"),
    ("index", "woml_scheduler_claims_expiry"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v12 is missing required {object_type} {name:?}."
      )));
    }
  }
  let mut statement = connection.prepare("PRAGMA table_info(woml_run_summaries)")?;
  let actual_columns = statement
    .query_map([], |row| row.get::<_, String>(1))?
    .collect::<Result<Vec<_>, _>>()?;
  let expected_columns = [
    "run_id",
    "workflow_id",
    "status",
    "admitted_at",
    "started_at",
    "updated_at",
    "queue_name",
    "waiting_for",
    "eligible_at",
  ];
  if actual_columns != expected_columns {
    return Err(DurableStoreError::Contract(format!(
      "Store v12 woml_run_summaries columns do not match the frozen schema: expected {expected_columns:?}, found {actual_columns:?}."
    )));
  }
  Ok(())
}

fn validate_store_v13_schema(connection: &Connection) -> Result<(), DurableStoreError> {
  validate_store_v12_schema(connection)?;
  for (object_type, name) in [
    ("table", "woml_state_entries"),
    ("index", "woml_state_entries_scope"),
    ("table", "woml_state_mutations"),
    ("index", "woml_state_mutations_scope_key"),
    ("trigger", "woml_state_mutations_no_update"),
    ("trigger", "woml_state_mutations_no_delete"),
    ("table", "woml_state_quotas"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v13 is missing required {object_type} {name:?}."
      )));
    }
  }
  for (table, expected_columns) in [
    (
      "woml_state_entries",
      vec![
        "scope_digest",
        "key_digest",
        "key_text",
        "value_json",
        "value_bytes",
        "version",
        "updated_at",
      ],
    ),
    (
      "woml_state_mutations",
      vec![
        "operation_key",
        "scope_digest",
        "operation_name",
        "operation",
        "key_digest",
        "input_digest",
        "result_json",
        "result_digest",
        "committed_version",
        "committed_at",
      ],
    ),
    (
      "woml_state_quotas",
      vec!["scope_digest", "live_keys", "value_bytes"],
    ),
  ] {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let actual_columns = statement
      .query_map([], |row| row.get::<_, String>(1))?
      .collect::<Result<Vec<_>, _>>()?;
    if actual_columns != expected_columns {
      return Err(DurableStoreError::Contract(format!(
        "Store v13 {table} columns do not match the frozen schema: expected {expected_columns:?}, found {actual_columns:?}."
      )));
    }
  }
  Ok(())
}

fn validate_store_v14_schema(connection: &Connection) -> Result<(), DurableStoreError> {
  validate_store_v13_schema(connection)?;
  for (name, expected_columns) in [
    (
      "woml_runtime_owner",
      &[
        "singleton",
        "deployment_id",
        "activation_id",
        "runtime_instance_id",
        "heartbeat_at",
        "lease_expires_at",
      ][..],
    ),
    (
      "woml_maintenance_lease",
      &[
        "singleton",
        "lease_id",
        "operation",
        "owner_id",
        "expires_at",
      ][..],
    ),
    (
      "woml_last_verified_backup",
      &["singleton", "backup_id", "completed_at", "verified"][..],
    ),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
      [name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v14 is missing required table {name:?}."
      )));
    }
    let mut statement = connection.prepare(&format!("PRAGMA table_info({name})"))?;
    let columns = statement
      .query_map([], |row| row.get::<_, String>(1))?
      .collect::<Result<Vec<_>, _>>()?;
    if columns != expected_columns {
      return Err(DurableStoreError::Contract(format!(
        "Store v14 {name} columns do not match the frozen schema: expected {expected_columns:?}, found {columns:?}."
      )));
    }
  }
  Ok(())
}

fn validate_store_v15_schema(connection: &Connection) -> Result<(), DurableStoreError> {
  validate_store_v14_schema(connection)?;
  for (name, expected_columns) in [
    (
      "woml_durable_waits",
      &[
        "run_id",
        "wait_id",
        "iteration_identity",
        "node_id",
        "kind",
        "registered_at",
        "deadline",
        "signals_json",
        "correlation_digest",
        "state",
        "resolution_event_id",
        "resolved_at",
      ][..],
    ),
    (
      "woml_external_signals",
      &[
        "signal_id",
        "run_id",
        "wait_id",
        "iteration_identity",
        "signal",
        "received_at",
        "idempotency_digest",
        "request_digest",
        "correlation_digest",
        "payload_json",
        "authentication_class",
        "consumed_at",
      ][..],
    ),
    (
      "woml_signal_idempotency",
      &[
        "idempotency_digest",
        "request_digest",
        "signal_id",
        "created_at",
      ][..],
    ),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
      [name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v15 is missing required table {name:?}."
      )));
    }
    let mut statement = connection.prepare(&format!("PRAGMA table_info({name})"))?;
    let columns = statement
      .query_map([], |row| row.get::<_, String>(1))?
      .collect::<Result<Vec<_>, _>>()?;
    if columns != expected_columns {
      return Err(DurableStoreError::Contract(format!(
        "Store v15 {name} columns do not match the frozen schema: expected {expected_columns:?}, found {columns:?}."
      )));
    }
  }
  for name in [
    "woml_durable_waits_due",
    "woml_durable_waits_active_target",
    "woml_external_signals_target",
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1)",
      [name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v15 is missing required index {name:?}."
      )));
    }
  }
  Ok(())
}

fn validate_store_v16_schema(connection: &Connection) -> Result<(), DurableStoreError> {
  validate_store_v15_schema(connection)?;
  for (name, expected_columns) in [
    (
      "woml_lock_resources",
      &[
        "key",
        "fencing_high_water",
        "owner_lease_id",
        "owner_run_id",
        "owner_node_execution_id",
        "owner_fencing_token",
        "acquired_at",
        "expires_at",
        "updated_at",
      ][..],
    ),
    (
      "woml_lock_waiters",
      &[
        "waiter_id",
        "key",
        "run_id",
        "node_execution_id",
        "sequence",
        "registered_at",
        "deadline",
        "status",
        "input_digest",
      ][..],
    ),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
      [name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v16 is missing required table {name:?}."
      )));
    }
    let mut statement = connection.prepare(&format!("PRAGMA table_info({name})"))?;
    let columns = statement
      .query_map([], |row| row.get::<_, String>(1))?
      .collect::<Result<Vec<_>, _>>()?;
    if columns != expected_columns {
      return Err(DurableStoreError::Contract(format!("Store v16 {name} columns do not match the frozen schema: expected {expected_columns:?}, found {columns:?}.")));
    }
  }
  for name in [
    "woml_lock_resources_one_owner",
    "woml_lock_waiters_fair_queue",
    "woml_lock_waiters_recovery",
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1)",
      [name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v16 is missing required index {name:?}."
      )));
    }
  }
  Ok(())
}

fn validate_store_v17_schema(connection: &Connection) -> Result<(), DurableStoreError> {
  validate_store_v16_schema(connection)?;
  for (name, expected_columns) in [
    (
      "woml_saga_scopes",
      &[
        "run_id",
        "saga_id",
        "phase",
        "original_outcome",
        "started_sequence",
        "committed_sequence",
        "updated_at",
        "model_digest",
      ][..],
    ),
    (
      "woml_correction_actions",
      &[
        "run_id",
        "saga_id",
        "action_id",
        "action_kind",
        "for_step_execution_id",
        "completion_sequence",
        "status",
        "attempts",
        "next_attempt_at",
        "updated_at",
      ][..],
    ),
    (
      "woml_correction_attempts",
      &[
        "run_id",
        "saga_id",
        "action_id",
        "attempt",
        "invocation_id",
        "status",
        "started_at",
        "finished_at",
        "failure_code",
      ][..],
    ),
    (
      "woml_correction_manual_recovery",
      &[
        "run_id",
        "saga_id",
        "action_id",
        "status",
        "reason_code",
        "created_at",
        "resolved_at",
      ][..],
    ),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
      [name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v17 is missing required table {name:?}."
      )));
    }
    let mut statement = connection.prepare(&format!("PRAGMA table_info({name})"))?;
    let columns = statement
      .query_map([], |row| row.get::<_, String>(1))?
      .collect::<Result<Vec<_>, _>>()?;
    if columns != expected_columns {
      return Err(DurableStoreError::Contract(format!(
        "Store v17 {name} columns do not match the frozen schema: expected {expected_columns:?}, found {columns:?}."
      )));
    }
  }
  for name in [
    "woml_correction_actions_pending",
    "woml_correction_attempts_active",
    "woml_correction_manual_recovery_pending",
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1)",
      [name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v17 is missing required index {name:?}."
      )));
    }
  }
  Ok(())
}

fn validate_store_v18_schema(connection: &Connection) -> Result<(), DurableStoreError> {
  validate_store_v17_schema(connection)?;
  for (name, expected_columns) in [
    (
      "woml_approval_reviews",
      &[
        "run_id",
        "approval_id",
        "request_id",
        "definition_hash",
        "workflow_version",
        "review_json",
        "review_digest",
        "required_approvals",
        "required_roles_json",
        "directory_version",
        "status",
        "created_at",
        "resolved_at",
      ][..],
    ),
    (
      "woml_approval_reviewer_decisions",
      &[
        "run_id",
        "request_id",
        "principal_id",
        "decision",
        "provider",
        "provider_account_id",
        "provider_actor_id",
        "roles_json",
        "decided_at",
      ][..],
    ),
    (
      "woml_approval_artifact_pins",
      &[
        "run_id",
        "request_id",
        "artifact_id",
        "artifact_sha256",
        "released_at",
      ][..],
    ),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
      [name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v18 is missing required table {name:?}."
      )));
    }
    let mut statement = connection.prepare(&format!("PRAGMA table_info({name})"))?;
    let columns = statement
      .query_map([], |row| row.get::<_, String>(1))?
      .collect::<Result<Vec<_>, _>>()?;
    if columns != expected_columns {
      return Err(DurableStoreError::Contract(format!(
        "Store v18 {name} columns do not match the frozen schema: expected {expected_columns:?}, found {columns:?}."
      )));
    }
  }
  for name in [
    "woml_approval_reviews_pending",
    "woml_approval_decisions_request",
    "woml_approval_pins_live",
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1)",
      [name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v18 is missing required index {name:?}."
      )));
    }
  }
  for name in [
    "woml_approval_reviews_no_delete",
    "woml_approval_reviews_immutable",
    "woml_approval_decisions_no_update",
    "woml_approval_decisions_no_delete",
    "woml_approval_pins_immutable",
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'trigger' AND name = ?1)",
      [name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v18 is missing required trigger {name:?}."
      )));
    }
  }
  Ok(())
}

fn validate_store_v19_schema(connection: &Connection) -> Result<(), DurableStoreError> {
  validate_store_v18_schema(connection)?;
  for (name, expected_columns) in [
    (
      "woml_infrastructure_policy_bundles",
      &[
        "policy_digest",
        "policy_id",
        "policy_version",
        "environment",
        "canonical_payload",
        "signature_status",
        "signing_key_id",
        "created_at",
      ][..],
    ),
    (
      "woml_run_infrastructure_policies",
      &["run_id", "policy_digest", "admitted_at"][..],
    ),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
      [name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v19 is missing required table {name:?}."
      )));
    }
    let mut statement = connection.prepare(&format!("PRAGMA table_info({name})"))?;
    let columns = statement
      .query_map([], |row| row.get::<_, String>(1))?
      .collect::<Result<Vec<_>, _>>()?;
    if columns != expected_columns {
      return Err(DurableStoreError::Contract(format!(
        "Store v19 {name} columns do not match the frozen schema: expected {expected_columns:?}, found {columns:?}."
      )));
    }
  }
  for (object_type, name) in [
    ("index", "woml_run_infrastructure_policies_digest"),
    ("trigger", "woml_infrastructure_policy_bundles_no_update"),
    ("trigger", "woml_infrastructure_policy_bundles_no_delete"),
    ("trigger", "woml_run_infrastructure_policies_no_update"),
    ("trigger", "woml_run_infrastructure_policies_no_delete"),
  ] {
    let exists: bool = connection.query_row(
      "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2)",
      [object_type, name],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(DurableStoreError::Contract(format!(
        "Store v19 is missing required {object_type} {name:?}."
      )));
    }
  }
  Ok(())
}

fn runtime_policy_indexes_need_rebuild(connection: &Connection) -> Result<bool, DurableStoreError> {
  let count = |sql: &str| -> Result<i64, DurableStoreError> {
    Ok(connection.query_row(sql, [], |row| row.get(0))?)
  };
  Ok(
    count("SELECT COUNT(*) FROM woml_runs")? != count("SELECT COUNT(*) FROM woml_run_summaries")?
      || count(
        "SELECT COUNT(*) FROM woml_run_events
         WHERE sequence = 1 AND event_schema_version = 11",
      )? != count("SELECT COUNT(*) FROM woml_runtime_policy_bindings")?
      || count("SELECT COUNT(*) FROM woml_run_summaries WHERE status = 'queued'")?
        != count("SELECT COUNT(*) FROM woml_runtime_policy_queue")?
      || count(
        "SELECT COUNT(*) FROM woml_run_summaries
         WHERE queue_name IS NOT NULL AND started_at IS NOT NULL",
      )? != count("SELECT COUNT(*) FROM woml_runtime_policy_starts")?,
  )
}

impl DurableEventStore {
  pub fn open(path: impl AsRef<Path>) -> Result<Self, DurableStoreError> {
    let mut span = crate::performance::PerformanceSpan::new("sqlite", "sqlite.open_initialize");
    let path = path.as_ref();
    let result = (|| {
      let connection = Connection::open(path)?;
      Self::initialize(connection)
    })();
    if result.is_ok() {
      if let Ok(metadata) = std::fs::metadata(path) {
        span.bytes(
          "database",
          usize::try_from(metadata.len()).unwrap_or(usize::MAX),
        );
      }
      span.succeed();
    }
    result
  }

  /// Opens a store that the runtime has already initialized and validated.
  ///
  /// Request admission must not repeat schema DDL or migration checks: doing
  /// so can turn a short, recoverable SQLite writer conflict into a lock-upgrade
  /// deadlock. Runtime startup remains the sole schema/migration authority.
  pub(crate) fn open_ready(path: impl AsRef<Path>) -> Result<Self, DurableStoreError> {
    let mut span = crate::performance::PerformanceSpan::new("sqlite", "sqlite.open_ready");
    let path = path.as_ref();
    let connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    let version: String = connection.query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )?;
    if version != STORE_SCHEMA_VERSION_V20 {
      return Err(DurableStoreError::UnsupportedStoreVersion(version));
    }
    if let Ok(metadata) = std::fs::metadata(path) {
      span.bytes(
        "database",
        usize::try_from(metadata.len()).unwrap_or(usize::MAX),
      );
    }
    span.succeed();
    Ok(Self {
      connection,
      projection_cache: RefCell::new(HashMap::new()),
    })
  }

  pub fn open_in_memory() -> Result<Self, DurableStoreError> {
    let connection = Connection::open_in_memory()?;
    Self::initialize(connection)
  }

  pub fn bind_infrastructure_policy(
    &mut self,
    run_id: &str,
    policy: &PreparedInfrastructurePolicyV1,
    admitted_at: DateTime<Utc>,
  ) -> Result<InfrastructurePolicyRunBindingV1, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let signature_status = match policy.signature_status {
      InfrastructurePolicySignatureStatusV1::Unsigned => "unsigned",
      InfrastructurePolicySignatureStatusV1::Verified => "verified",
    };
    transaction.execute(
      "INSERT OR IGNORE INTO woml_infrastructure_policy_bundles(
         policy_digest, policy_id, policy_version, environment, canonical_payload,
         signature_status, signing_key_id, created_at
       ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
      params![
        policy.digest,
        policy.bundle.id,
        policy.bundle.version,
        policy.bundle.environment,
        policy.canonical_payload,
        signature_status,
        policy.signing_key_id,
        admitted_at.to_rfc3339(),
      ],
    )?;
    let stored: (String, String, String, Vec<u8>, String, Option<String>) = transaction.query_row(
      "SELECT policy_id, policy_version, environment, canonical_payload,
              signature_status, signing_key_id
       FROM woml_infrastructure_policy_bundles WHERE policy_digest = ?1",
      [&policy.digest],
      |row| {
        Ok((
          row.get(0)?,
          row.get(1)?,
          row.get(2)?,
          row.get(3)?,
          row.get(4)?,
          row.get(5)?,
        ))
      },
    )?;
    if stored.0 != policy.bundle.id
      || stored.1 != policy.bundle.version
      || stored.2 != policy.bundle.environment
      || stored.3 != policy.canonical_payload
      || stored.4 != signature_status
      || stored.5 != policy.signing_key_id
    {
      return Err(DurableStoreError::Contract(
        "WOML_POLICY_INVALID: the stored policy digest is bound to different bytes or identity."
          .to_string(),
      ));
    }
    transaction.execute(
      "INSERT OR IGNORE INTO woml_run_infrastructure_policies(run_id, policy_digest, admitted_at)
       VALUES (?1, ?2, ?3)",
      params![run_id, policy.digest, admitted_at.to_rfc3339()],
    )?;
    let (bound_digest, bound_at): (String, String) = transaction.query_row(
      "SELECT policy_digest, admitted_at FROM woml_run_infrastructure_policies WHERE run_id = ?1",
      [run_id],
      |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if bound_digest != policy.digest {
      return Err(DurableStoreError::Contract(
        "WOML_POLICY_INVALID: the run is already pinned to a different infrastructure policy."
          .to_string(),
      ));
    }
    transaction.commit()?;
    Ok(InfrastructurePolicyRunBindingV1 {
      run_id: run_id.to_string(),
      policy_id: policy.bundle.id.clone(),
      policy_version: policy.bundle.version.clone(),
      policy_digest: policy.digest.clone(),
      canonical_bundle_reference: format!("policy:{}", policy.digest),
      signature_status: policy.signature_status,
      signing_key_id: policy.signing_key_id.clone(),
      admitted_at: parse_timestamp(&bound_at)?,
    })
  }

  pub fn infrastructure_policy_binding(
    &self,
    run_id: &str,
  ) -> Result<Option<InfrastructurePolicyRunBindingV1>, DurableStoreError> {
    let row = self
      .connection
      .query_row(
        "SELECT bundles.policy_id, bundles.policy_version, bindings.policy_digest,
                bundles.signature_status, bundles.signing_key_id, bindings.admitted_at
         FROM woml_run_infrastructure_policies AS bindings
         JOIN woml_infrastructure_policy_bundles AS bundles
           ON bundles.policy_digest = bindings.policy_digest
         WHERE bindings.run_id = ?1",
        [run_id],
        |row| {
          Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, String>(5)?,
          ))
        },
      )
      .optional()?;
    let Some((policy_id, policy_version, digest, status, signing_key_id, admitted_at)) = row else {
      return Ok(None);
    };
    let signature_status = match status.as_str() {
      "unsigned" => InfrastructurePolicySignatureStatusV1::Unsigned,
      "verified" => InfrastructurePolicySignatureStatusV1::Verified,
      _ => {
        return Err(DurableStoreError::Contract(
          "WOML_POLICY_INVALID: admitted policy signature status is invalid.".to_string(),
        ))
      }
    };
    Ok(Some(InfrastructurePolicyRunBindingV1 {
      run_id: run_id.to_string(),
      policy_id,
      policy_version,
      canonical_bundle_reference: format!("policy:{digest}"),
      policy_digest: digest,
      signature_status,
      signing_key_id,
      admitted_at: parse_timestamp(&admitted_at)?,
    }))
  }

  pub fn infrastructure_policy_for_run(
    &self,
    run_id: &str,
  ) -> Result<Option<PreparedInfrastructurePolicyV1>, DurableStoreError> {
    let bound_digest = self
      .connection
      .query_row(
        "SELECT policy_digest FROM woml_run_infrastructure_policies WHERE run_id = ?1",
        [run_id],
        |row| row.get::<_, String>(0),
      )
      .optional()?;
    let Some(bound_digest) = bound_digest else {
      return Ok(None);
    };
    let stored = self
      .connection
      .query_row(
        "SELECT policy_id, policy_version, environment, policy_digest,
                canonical_payload, signature_status, signing_key_id
         FROM woml_infrastructure_policy_bundles WHERE policy_digest = ?1",
        [&bound_digest],
        |row| {
          Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Vec<u8>>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, Option<String>>(6)?,
          ))
        },
      )
      .optional()?;
    let Some((policy_id, policy_version, environment, digest, canonical_payload, status, key_id)) =
      stored
    else {
      return Err(DurableStoreError::Contract(
        "WOML_POLICY_UNAVAILABLE: admitted canonical policy bytes are missing.".to_string(),
      ));
    };
    if digest != bound_digest {
      return Err(DurableStoreError::Contract(
        "WOML_POLICY_INVALID: admitted policy binding does not match stored bytes.".to_string(),
      ));
    }
    let bundle: InfrastructurePolicyBundleV1 =
      serde_json::from_slice(&canonical_payload).map_err(|_| {
        DurableStoreError::Contract(
          "WOML_POLICY_INVALID: admitted policy bytes are malformed.".to_string(),
        )
      })?;
    let mut prepared =
      crate::prepare_infrastructure_policy(bundle, &InfrastructurePolicyTrustV1::default())
        .map_err(|_| {
          DurableStoreError::Contract(
            "WOML_POLICY_INVALID: admitted policy bytes failed canonical validation.".to_string(),
          )
        })?;
    if prepared.digest != digest
      || prepared.canonical_payload != canonical_payload
      || prepared.bundle.id != policy_id
      || prepared.bundle.version != policy_version
      || prepared.bundle.environment != environment
    {
      return Err(DurableStoreError::Contract(
        "WOML_POLICY_INVALID: admitted policy identity does not match its canonical bytes."
          .to_string(),
      ));
    }
    prepared.signature_status = match status.as_str() {
      "unsigned" if key_id.is_none() => InfrastructurePolicySignatureStatusV1::Unsigned,
      "verified" if key_id.is_some() => InfrastructurePolicySignatureStatusV1::Verified,
      _ => {
        return Err(DurableStoreError::Contract(
          "WOML_POLICY_INVALID: admitted policy signature status is contradictory.".to_string(),
        ))
      }
    };
    prepared.signing_key_id = key_id;
    Ok(Some(prepared))
  }

  pub(crate) fn infrastructure_policy_evidence_snapshot(
    &self,
    run_id: &str,
    now: DateTime<Utc>,
  ) -> Result<InfrastructurePolicyEvidenceSnapshotV1, DurableStoreError> {
    ensure_run_exists(&self.connection, run_id)?;
    let mut approvals_statement = self.connection.prepare(
      "SELECT reviews.request_id, reviews.review_digest, decisions.principal_id,
              decisions.roles_json, decisions.decided_at
       FROM woml_approval_reviews AS reviews
       JOIN woml_approval_reviewer_decisions AS decisions
         ON decisions.run_id = reviews.run_id AND decisions.request_id = reviews.request_id
       WHERE reviews.run_id = ?1 AND reviews.status = 'approved'
         AND decisions.decision = 'approved'
       ORDER BY decisions.decided_at, decisions.principal_id",
    )?;
    let approvals = approvals_statement
      .query_map([run_id], |row| {
        Ok((
          row.get::<_, String>(0)?,
          row.get::<_, String>(1)?,
          row.get::<_, String>(2)?,
          row.get::<_, String>(3)?,
          row.get::<_, String>(4)?,
        ))
      })?
      .map(|row| {
        let (request_id, review_digest, principal_id, roles_json, decided_at) = row?;
        Ok(InfrastructurePolicyApprovalEvidenceV1 {
          request_id,
          review_digest,
          principal_id,
          roles: serde_json::from_str::<Vec<String>>(&roles_json)?
            .into_iter()
            .collect(),
          decided_at: parse_timestamp(&decided_at)?,
        })
      })
      .collect::<Result<Vec<_>, DurableStoreError>>()?;

    let mut artifacts_statement = self.connection.prepare(
      "SELECT pins.artifact_id, pins.artifact_sha256, reviews.review_digest
       FROM woml_approval_artifact_pins AS pins
       JOIN woml_approval_reviews AS reviews
         ON reviews.run_id = pins.run_id AND reviews.request_id = pins.request_id
       WHERE reviews.run_id = ?1 AND reviews.status = 'approved'
       ORDER BY pins.artifact_id, reviews.review_digest",
    )?;
    let artifacts = artifacts_statement
      .query_map([run_id], |row| {
        Ok(InfrastructurePolicyArtifactEvidenceV1 {
          artifact_id: row.get(0)?,
          sha256: row.get(1)?,
          review_digest: row.get(2)?,
        })
      })?
      .collect::<Result<Vec<_>, _>>()?;

    let mut locks_statement = self.connection.prepare(
      "SELECT key, owner_lease_id, CAST(owner_fencing_token AS TEXT), acquired_at, expires_at
       FROM woml_lock_resources
       WHERE owner_run_id = ?1 AND owner_lease_id IS NOT NULL
         AND owner_fencing_token IS NOT NULL AND acquired_at IS NOT NULL
         AND expires_at IS NOT NULL AND expires_at > ?2
       ORDER BY key",
    )?;
    let locks = locks_statement
      .query_map(params![run_id, now.to_rfc3339()], |row| {
        Ok((
          row.get::<_, String>(0)?,
          row.get::<_, String>(1)?,
          row.get::<_, String>(2)?,
          row.get::<_, String>(3)?,
          row.get::<_, String>(4)?,
        ))
      })?
      .map(|row| {
        let (key, lease_id, fencing_token, acquired_at, expires_at) = row?;
        Ok(InfrastructurePolicyLockEvidenceV1 {
          key,
          lease_id,
          fencing_token,
          acquired_at: parse_timestamp(&acquired_at)?,
          expires_at: parse_timestamp(&expires_at)?,
        })
      })
      .collect::<Result<Vec<_>, DurableStoreError>>()?;

    Ok(InfrastructurePolicyEvidenceSnapshotV1 {
      approvals,
      artifacts,
      locks,
    })
  }

  fn initialize(mut connection: Connection) -> Result<Self, DurableStoreError> {
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    connection.execute_batch(CREATE_SCHEMA)?;
    let version: Option<String> = connection
      .query_row(
        "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
        [],
        |row| row.get(0),
      )
      .optional()?;
    match version.as_deref() {
      Some(STORE_SCHEMA_VERSION_V20) => {
        validate_store_v19_schema(&connection)?;
        validate_remote_dispatch_schema(&connection)?;
      }
      Some(STORE_SCHEMA_VERSION_V19) => validate_store_v19_schema(&connection)?,
      Some(STORE_SCHEMA_VERSION_V18) => validate_store_v18_schema(&connection)?,
      Some(STORE_SCHEMA_VERSION_V17) => validate_store_v17_schema(&connection)?,
      Some(STORE_SCHEMA_VERSION_V16) => validate_store_v16_schema(&connection)?,
      Some(STORE_SCHEMA_VERSION_V15) => validate_store_v15_schema(&connection)?,
      Some(STORE_SCHEMA_VERSION_V14) => validate_store_v14_schema(&connection)?,
      Some(STORE_SCHEMA_VERSION_V13) => validate_store_v13_schema(&connection)?,
      Some(STORE_SCHEMA_VERSION_V12) => validate_store_v12_schema(&connection)?,
      Some(STORE_SCHEMA_VERSION_V11) => validate_store_v11_schema(&connection)?,
      Some(STORE_SCHEMA_VERSION_V10) => validate_store_v10_schema(&connection)?,
      Some(STORE_SCHEMA_VERSION_V9) => validate_store_v9_schema(&connection)?,
      Some(STORE_SCHEMA_VERSION_V8) => {
        validate_store_v8_schema(&connection)?;
        migrate_store_v8_to_v9(&mut connection)?;
      }
      Some(STORE_SCHEMA_VERSION_V7) => {
        validate_store_v7_schema(&connection)?;
        migrate_store_v7_to_v8(&mut connection)?;
        migrate_store_v8_to_v9(&mut connection)?;
      }
      Some(STORE_SCHEMA_VERSION_V6) => {
        validate_store_v6_schema(&connection)?;
        migrate_store_v6_to_v7(&mut connection)?;
        migrate_store_v7_to_v8(&mut connection)?;
        migrate_store_v8_to_v9(&mut connection)?;
      }
      Some(STORE_SCHEMA_VERSION_V5) => {
        validate_store_v5_schema(&connection)?;
        migrate_store_v5_to_v6(&mut connection)?;
        migrate_store_v6_to_v7(&mut connection)?;
        migrate_store_v7_to_v8(&mut connection)?;
        migrate_store_v8_to_v9(&mut connection)?;
      }
      Some(STORE_SCHEMA_VERSION_V4) => {
        validate_store_v4_schema(&connection)?;
        migrate_store_v4_to_v5(&mut connection)?;
        migrate_store_v5_to_v6(&mut connection)?;
        migrate_store_v6_to_v7(&mut connection)?;
        migrate_store_v7_to_v8(&mut connection)?;
        migrate_store_v8_to_v9(&mut connection)?;
      }
      Some(STORE_SCHEMA_VERSION_V3) => {
        validate_store_v3_schema(&connection)?;
        migrate_store_v3_to_v4(&mut connection)?;
        migrate_store_v4_to_v5(&mut connection)?;
        migrate_store_v5_to_v6(&mut connection)?;
        migrate_store_v6_to_v7(&mut connection)?;
        migrate_store_v7_to_v8(&mut connection)?;
        migrate_store_v8_to_v9(&mut connection)?;
      }
      Some(STORE_SCHEMA_VERSION_V2) => {
        validate_store_v2_schema(&connection)?;
        migrate_store_v2_to_v3(&mut connection)?;
        migrate_store_v3_to_v4(&mut connection)?;
        migrate_store_v4_to_v5(&mut connection)?;
        migrate_store_v5_to_v6(&mut connection)?;
        migrate_store_v6_to_v7(&mut connection)?;
        migrate_store_v7_to_v8(&mut connection)?;
        migrate_store_v8_to_v9(&mut connection)?;
      }
      Some(STORE_SCHEMA_VERSION_V1) => {
        migrate_store_v1_to_v2(&mut connection)?;
        validate_store_v2_schema(&connection)?;
        migrate_store_v2_to_v3(&mut connection)?;
        migrate_store_v3_to_v4(&mut connection)?;
        migrate_store_v4_to_v5(&mut connection)?;
        migrate_store_v5_to_v6(&mut connection)?;
        migrate_store_v6_to_v7(&mut connection)?;
        migrate_store_v7_to_v8(&mut connection)?;
        migrate_store_v8_to_v9(&mut connection)?;
      }
      Some(version) => {
        return Err(DurableStoreError::UnsupportedStoreVersion(
          version.to_string(),
        ));
      }
      None => {
        connection.execute(
          "INSERT INTO woml_store_metadata(key, value) VALUES ('schema_version', ?1)",
          [STORE_SCHEMA_VERSION_V1],
        )?;
        migrate_store_v1_to_v2(&mut connection)?;
        validate_store_v2_schema(&connection)?;
        migrate_store_v2_to_v3(&mut connection)?;
        migrate_store_v3_to_v4(&mut connection)?;
        migrate_store_v4_to_v5(&mut connection)?;
        migrate_store_v5_to_v6(&mut connection)?;
        migrate_store_v6_to_v7(&mut connection)?;
        migrate_store_v7_to_v8(&mut connection)?;
        migrate_store_v8_to_v9(&mut connection)?;
      }
    }
    let current_version: String = connection.query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )?;
    if current_version == STORE_SCHEMA_VERSION_V9 {
      migrate_store_v9_to_v10(&mut connection)?;
    }
    let current_version: String = connection.query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )?;
    if current_version == STORE_SCHEMA_VERSION_V10 {
      migrate_store_v10_to_v11(&mut connection)?;
    }
    let current_version: String = connection.query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )?;
    if current_version == STORE_SCHEMA_VERSION_V11 {
      migrate_store_v11_to_v12(&mut connection)?;
    }
    let current_version: String = connection.query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )?;
    if current_version == STORE_SCHEMA_VERSION_V12 {
      migrate_store_v12_to_v13(&mut connection)?;
    }
    let current_version: String = connection.query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )?;
    if current_version == STORE_SCHEMA_VERSION_V13 {
      migrate_store_v13_to_v14(&mut connection)?;
    }
    let current_version: String = connection.query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )?;
    if current_version == STORE_SCHEMA_VERSION_V14 {
      migrate_store_v14_to_v15(&mut connection)?;
    }
    let current_version: String = connection.query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )?;
    if current_version == STORE_SCHEMA_VERSION_V15 {
      migrate_store_v15_to_v16(&mut connection)?;
    }
    let current_version: String = connection.query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )?;
    if current_version == STORE_SCHEMA_VERSION_V16 {
      migrate_store_v16_to_v17(&mut connection)?;
    }
    let current_version: String = connection.query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )?;
    if current_version == STORE_SCHEMA_VERSION_V17 {
      migrate_store_v17_to_v18(&mut connection)?;
    }
    let current_version: String = connection.query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )?;
    if current_version == STORE_SCHEMA_VERSION_V18 {
      migrate_store_v18_to_v19(&mut connection)?;
    }
    let current_version: String = connection.query_row(
      "SELECT value FROM woml_store_metadata WHERE key = 'schema_version'",
      [],
      |row| row.get(0),
    )?;
    if current_version == STORE_SCHEMA_VERSION_V19 {
      migrate_store_v19_to_v20(&mut connection)?;
    }
    validate_store_v19_schema(&connection)?;
    validate_remote_dispatch_schema(&connection)?;
    {
      let recovery = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
      rebuild_correction_indexes(&recovery)?;
      recovery.commit()?;
    }
    if runtime_policy_indexes_need_rebuild(&connection)? {
      let recovery = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
      rebuild_runtime_policy_indexes(&recovery)?;
      rebuild_run_summary_index(&recovery)?;
      recovery.commit()?;
    }
    connection.execute(
      "DELETE FROM woml_scheduler_claims WHERE expires_at <= ?1",
      [Utc::now().to_rfc3339()],
    )?;
    Ok(Self {
      connection,
      projection_cache: RefCell::new(HashMap::new()),
    })
  }

  pub fn register_definition(
    &mut self,
    workflow: &CompiledWorkflowDefinition,
    definition_hash: &str,
  ) -> Result<(), DurableStoreError> {
    workflow.validate_structure()?;
    if !is_definition_hash(definition_hash) {
      return Err(DurableStoreError::Contract(
        "A durable definition requires a valid RFC 8785 SHA-256 hash.".to_string(),
      ));
    }
    let model_json = serde_json::to_string(workflow)?;
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let stored: Option<(String, String)> = transaction
      .query_row(
        "SELECT workflow_id, model_json FROM woml_definitions WHERE definition_hash = ?1",
        [definition_hash],
        |row| Ok((row.get(0)?, row.get(1)?)),
      )
      .optional()?;
    if let Some((workflow_id, stored_json)) = stored {
      let stored_value: Value = serde_json::from_str(&stored_json)?;
      let incoming_value: Value = serde_json::from_str(&model_json)?;
      if workflow_id != workflow.workflow_id || stored_value != incoming_value {
        return Err(DurableStoreError::DefinitionConflict(
          definition_hash.to_string(),
        ));
      }
    } else {
      transaction.execute(
        "INSERT INTO woml_definitions(
           definition_hash, workflow_id, schema_version, model_json, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
          definition_hash,
          workflow.workflow_id,
          i64::from(workflow.schema_version),
          model_json,
          Utc::now().to_rfc3339(),
        ],
      )?;
    }
    transaction.commit()?;
    Ok(())
  }

  pub fn definition(
    &self,
    definition_hash: &str,
  ) -> Result<CompiledWorkflowDefinition, DurableStoreError> {
    let mut statement = self
      .connection
      .prepare_cached("SELECT model_json FROM woml_definitions WHERE definition_hash = ?1")?;
    let model_json: String = statement
      .query_row([definition_hash], |row| row.get(0))
      .optional()?
      .ok_or_else(|| DurableStoreError::DefinitionNotFound(definition_hash.to_string()))?;
    let workflow: CompiledWorkflowDefinition = serde_json::from_str(&model_json)?;
    workflow.validate_structure()?;
    Ok(workflow)
  }

  pub fn has_definition_for_workflow(&self, workflow_id: &str) -> Result<bool, DurableStoreError> {
    if workflow_id.is_empty() || workflow_id.len() > 256 {
      return Err(DurableStoreError::Contract(
        "A workflow definition lookup requires a valid workflow ID.".to_string(),
      ));
    }
    self
      .connection
      .query_row(
        "SELECT EXISTS(SELECT 1 FROM woml_definitions WHERE workflow_id = ?1)",
        [workflow_id],
        |row| row.get::<_, bool>(0),
      )
      .map_err(DurableStoreError::from)
  }

  /// Returns the durable lineage depth of a run. Top-level trigger runs have
  /// depth zero; an admitted workflow-call child stores its assigned depth.
  pub fn workflow_call_depth_for_parent(&self, run_id: &str) -> Result<u32, DurableStoreError> {
    ensure_run_exists(&self.connection, run_id)?;
    let depth: Option<i64> = self
      .connection
      .query_row(
        "SELECT depth FROM woml_workflow_calls WHERE child_run_id = ?1",
        [run_id],
        |row| row.get(0),
      )
      .optional()?;
    match depth {
      None => Ok(0),
      Some(depth) if (1..=i64::from(MAX_WORKFLOW_CALL_DEPTH)).contains(&depth) => Ok(depth as u32),
      Some(_) => Err(DurableStoreError::WorkflowCallHistoryInvalid(
        "stored lineage depth is invalid".to_string(),
      )),
    }
  }

  pub fn workflow_call(
    &self,
    call_key: &str,
  ) -> Result<Option<WorkflowCallAdmission>, DurableStoreError> {
    load_workflow_call(&self.connection, call_key)
  }

  pub fn workflow_call_for_child(
    &self,
    child_run_id: &str,
  ) -> Result<Option<WorkflowCallAdmission>, DurableStoreError> {
    let call_key: Option<String> = self
      .connection
      .query_row(
        "SELECT call_key FROM woml_workflow_calls WHERE child_run_id = ?1",
        [child_run_id],
        |row| row.get(0),
      )
      .optional()?;
    call_key
      .map(|call_key| load_workflow_call(&self.connection, &call_key))
      .transpose()
      .map(Option::flatten)
  }

  /// Returns a bounded, non-sensitive view of the parent/child relationships
  /// for one run. Payloads, digests, definition hashes, and call keys are
  /// deliberately excluded from this operator-facing surface.
  pub fn workflow_call_relations_for_run(
    &self,
    run_id: &str,
  ) -> Result<WorkflowCallRunRelations, DurableStoreError> {
    ensure_run_exists(&self.connection, run_id)?;
    let parent_call = self
      .workflow_call_for_child(run_id)?
      .map(WorkflowCallRunSummary::from);
    let mut statement = self.connection.prepare(
      "SELECT call_key FROM woml_workflow_calls
       WHERE parent_run_id = ?1
       ORDER BY admitted_at, call_key
       LIMIT ?2",
    )?;
    let limit = i64::try_from(MAX_WORKFLOW_CALL_INSPECTION_CHILDREN + 1)
      .expect("workflow call inspection limit fits in SQLite integer");
    let call_keys = statement
      .query_map(params![run_id, limit], |row| row.get::<_, String>(0))?
      .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    let child_calls_truncated = call_keys.len() > MAX_WORKFLOW_CALL_INSPECTION_CHILDREN;
    let child_calls = call_keys
      .into_iter()
      .take(MAX_WORKFLOW_CALL_INSPECTION_CHILDREN)
      .map(|call_key| {
        load_workflow_call(&self.connection, &call_key)?
          .map(WorkflowCallRunSummary::from)
          .ok_or_else(|| {
            DurableStoreError::WorkflowCallHistoryInvalid(
              "workflow call disappeared during run inspection".to_string(),
            )
          })
      })
      .collect::<Result<Vec<_>, _>>()?;
    Ok(WorkflowCallRunRelations {
      parent_call,
      child_calls,
      child_calls_truncated,
    })
  }

  pub fn admitted_workflow_calls_for_target(
    &self,
    target: &WorkflowTarget,
  ) -> Result<Vec<WorkflowCallAdmission>, DurableStoreError> {
    let mut statement = self.connection.prepare(
      "SELECT call_key FROM woml_workflow_calls
       WHERE target_workflow_id = ?1 AND target_definition_hash = ?2 AND state = 'admitted'
       ORDER BY admitted_at, call_key",
    )?;
    let call_keys = statement
      .query_map([&target.workflow_id, &target.definition_hash], |row| {
        row.get::<_, String>(0)
      })?
      .collect::<Result<Vec<_>, _>>()?;
    call_keys
      .into_iter()
      .map(|call_key| {
        load_workflow_call(&self.connection, &call_key)?.ok_or_else(|| {
          DurableStoreError::WorkflowCallHistoryInvalid(
            "pending workflow call disappeared during routing scan".to_string(),
          )
        })
      })
      .collect()
  }

  pub fn register_workflow_runtime_routes(
    &mut self,
    runtime_id: &str,
    targets: &[WorkflowTarget],
    endpoint: &str,
    session_credential_hash: &str,
    now: DateTime<Utc>,
    lease_expires_at: DateTime<Utc>,
  ) -> Result<(), DurableStoreError> {
    validate_runtime_route_fields(
      runtime_id,
      targets,
      endpoint,
      session_credential_hash,
      now,
      lease_expires_at,
    )?;
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    for target in targets {
      let existing: Option<(String, String)> = transaction
        .query_row(
          "SELECT runtime_id, lease_expires_at FROM woml_workflow_runtime_routes
           WHERE workflow_id = ?1",
          [&target.workflow_id],
          |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
      if let Some((owner, expires_at)) = existing {
        let expires_at = DateTime::parse_from_rfc3339(&expires_at)
          .map_err(|_| {
            DurableStoreError::Contract(
              "A stored workflow runtime lease has an invalid timestamp.".to_string(),
            )
          })?
          .with_timezone(&Utc);
        if owner != runtime_id && expires_at > now {
          return Err(DurableStoreError::WorkflowRuntimeDuplicateOwner(
            target.workflow_id.clone(),
          ));
        }
      }
    }
    for target in targets {
      transaction.execute(
        "INSERT INTO woml_workflow_runtime_routes(
           workflow_id, definition_hash, runtime_id, endpoint,
           session_credential_hash, lease_expires_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(workflow_id) DO UPDATE SET
           definition_hash = excluded.definition_hash,
           runtime_id = excluded.runtime_id,
           endpoint = excluded.endpoint,
           session_credential_hash = excluded.session_credential_hash,
           lease_expires_at = excluded.lease_expires_at,
           updated_at = excluded.updated_at",
        params![
          target.workflow_id,
          target.definition_hash,
          runtime_id,
          endpoint,
          session_credential_hash,
          lease_expires_at.to_rfc3339(),
          now.to_rfc3339(),
        ],
      )?;
    }
    transaction.commit()?;
    Ok(())
  }

  pub fn audit_integrity(&self) -> Result<(), DurableStoreError> {
    let result: String = self
      .connection
      .query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if result != "ok" {
      return Err(DurableStoreError::Contract(format!(
        "The durable WOML store failed its integrity audit: {result}."
      )));
    }
    validate_store_v19_schema(&self.connection)?;
    validate_remote_dispatch_schema(&self.connection)?;
    let mut statement = self
      .connection
      .prepare("SELECT run_id FROM woml_run_infrastructure_policies ORDER BY run_id")?;
    let run_ids = statement
      .query_map([], |row| row.get::<_, String>(0))?
      .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for run_id in run_ids {
      self
        .infrastructure_policy_for_run(&run_id)?
        .ok_or_else(|| {
          DurableStoreError::Contract(
            "WOML_POLICY_UNAVAILABLE: an admitted run lost its canonical policy bundle."
              .to_string(),
          )
        })?;
    }
    Ok(())
  }

  pub fn runtime_owner(&self) -> Result<Option<RuntimeOwnerLease>, DurableStoreError> {
    let stored: Option<(String, String, String, String, String)> = self
      .connection
      .query_row(
        "SELECT deployment_id, activation_id, runtime_instance_id, heartbeat_at, lease_expires_at
         FROM woml_runtime_owner WHERE singleton = 1",
        [],
        |row| {
          Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
          ))
        },
      )
      .optional()?;
    stored
      .map(
        |(deployment_id, activation_id, runtime_instance_id, heartbeat, expires)| {
          let heartbeat_at = DateTime::parse_from_rfc3339(&heartbeat)
            .map_err(|_| {
              DurableStoreError::Contract("Stored runtime heartbeat is invalid.".to_string())
            })?
            .with_timezone(&Utc);
          let lease_expires_at = DateTime::parse_from_rfc3339(&expires)
            .map_err(|_| {
              DurableStoreError::Contract("Stored runtime lease expiry is invalid.".to_string())
            })?
            .with_timezone(&Utc);
          Ok(RuntimeOwnerLease {
            deployment_id,
            activation_id,
            runtime_instance_id,
            heartbeat_at,
            lease_expires_at,
          })
        },
      )
      .transpose()
  }

  pub fn acquire_runtime_owner(
    &mut self,
    deployment_id: &str,
    activation_id: &str,
    runtime_instance_id: &str,
    now: DateTime<Utc>,
    lease_expires_at: DateTime<Utc>,
  ) -> Result<(), DurableStoreError> {
    if deployment_id.is_empty()
      || deployment_id.len() > 320
      || runtime_instance_id.is_empty()
      || runtime_instance_id.len() > 320
      || !is_definition_hash(activation_id)
      || lease_expires_at <= now
    {
      return Err(DurableStoreError::Contract(
        "Runtime owner lease fields are invalid.".to_string(),
      ));
    }
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing: Option<(String, String, String)> = transaction
      .query_row(
        "SELECT deployment_id, runtime_instance_id, lease_expires_at
         FROM woml_runtime_owner WHERE singleton = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
      )
      .optional()?;
    if let Some((owner_deployment, owner_runtime, expires)) = existing {
      let expires = DateTime::parse_from_rfc3339(&expires)
        .map_err(|_| {
          DurableStoreError::Contract("Stored runtime lease expiry is invalid.".to_string())
        })?
        .with_timezone(&Utc);
      if owner_runtime != runtime_instance_id && expires > now {
        return Err(DurableStoreError::DeploymentRuntimeOwned {
          deployment_id: owner_deployment,
          runtime_instance_id: owner_runtime,
        });
      }
    }
    transaction.execute(
      "INSERT INTO woml_runtime_owner(
         singleton, deployment_id, activation_id, runtime_instance_id, heartbeat_at, lease_expires_at
       ) VALUES (1, ?1, ?2, ?3, ?4, ?5)
       ON CONFLICT(singleton) DO UPDATE SET
         deployment_id = excluded.deployment_id,
         activation_id = excluded.activation_id,
         runtime_instance_id = excluded.runtime_instance_id,
         heartbeat_at = excluded.heartbeat_at,
         lease_expires_at = excluded.lease_expires_at",
      params![
        deployment_id,
        activation_id,
        runtime_instance_id,
        now.to_rfc3339(),
        lease_expires_at.to_rfc3339(),
      ],
    )?;
    transaction.commit()?;
    Ok(())
  }

  pub fn renew_runtime_owner(
    &mut self,
    runtime_instance_id: &str,
    now: DateTime<Utc>,
    lease_expires_at: DateTime<Utc>,
  ) -> Result<bool, DurableStoreError> {
    if runtime_instance_id.is_empty() || lease_expires_at <= now {
      return Err(DurableStoreError::Contract(
        "Runtime owner renewal is invalid.".to_string(),
      ));
    }
    Ok(
      self.connection.execute(
        "UPDATE woml_runtime_owner SET heartbeat_at = ?1, lease_expires_at = ?2
       WHERE singleton = 1 AND runtime_instance_id = ?3",
        params![
          now.to_rfc3339(),
          lease_expires_at.to_rfc3339(),
          runtime_instance_id
        ],
      )? == 1,
    )
  }

  pub fn release_runtime_owner(
    &mut self,
    runtime_instance_id: &str,
  ) -> Result<bool, DurableStoreError> {
    Ok(
      self.connection.execute(
        "DELETE FROM woml_runtime_owner WHERE singleton = 1 AND runtime_instance_id = ?1",
        [runtime_instance_id],
      )? == 1,
    )
  }

  pub fn renew_workflow_runtime_routes(
    &mut self,
    runtime_id: &str,
    now: DateTime<Utc>,
    lease_expires_at: DateTime<Utc>,
  ) -> Result<usize, DurableStoreError> {
    if runtime_id.is_empty() || lease_expires_at <= now {
      return Err(DurableStoreError::Contract(
        "Workflow runtime lease renewal is invalid.".to_string(),
      ));
    }
    self
      .connection
      .execute(
        "UPDATE woml_workflow_runtime_routes
         SET lease_expires_at = ?1, updated_at = ?2 WHERE runtime_id = ?3",
        params![lease_expires_at.to_rfc3339(), now.to_rfc3339(), runtime_id],
      )
      .map_err(DurableStoreError::from)
  }

  pub fn unregister_workflow_runtime_routes(
    &mut self,
    runtime_id: &str,
  ) -> Result<usize, DurableStoreError> {
    self
      .connection
      .execute(
        "DELETE FROM woml_workflow_runtime_routes WHERE runtime_id = ?1",
        [runtime_id],
      )
      .map_err(DurableStoreError::from)
  }

  pub fn workflow_runtime_route(
    &self,
    workflow_id: &str,
    now: DateTime<Utc>,
  ) -> Result<Option<WorkflowRuntimeRoute>, DurableStoreError> {
    let stored: Option<(String, String, String, String, String)> = self
      .connection
      .query_row(
        "SELECT runtime_id, definition_hash, endpoint,
                session_credential_hash, lease_expires_at
         FROM woml_workflow_runtime_routes
         WHERE workflow_id = ?1 AND lease_expires_at > ?2",
        params![workflow_id, now.to_rfc3339()],
        |row| {
          Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
          ))
        },
      )
      .optional()?;
    stored
      .map(
        |(runtime_id, definition_hash, endpoint, session_credential_hash, expires)| {
          let lease_expires_at = DateTime::parse_from_rfc3339(&expires)
            .map_err(|_| {
              DurableStoreError::Contract(
                "A stored workflow runtime lease has an invalid timestamp.".to_string(),
              )
            })?
            .with_timezone(&Utc);
          Ok(WorkflowRuntimeRoute {
            runtime_id,
            workflow_id: workflow_id.to_string(),
            definition_hash,
            endpoint,
            session_credential_hash,
            lease_expires_at,
          })
        },
      )
      .transpose()
  }

  pub fn has_live_workflow_runtime_routes(
    &self,
    now: DateTime<Utc>,
  ) -> Result<bool, DurableStoreError> {
    self
      .connection
      .query_row(
        "SELECT EXISTS(
           SELECT 1 FROM woml_workflow_runtime_routes WHERE lease_expires_at > ?1
         )",
        [now.to_rfc3339()],
        |row| row.get(0),
      )
      .map_err(DurableStoreError::from)
  }

  /// Advances only the mutable execution state of a durable workflow call.
  /// All identity-bearing columns remain protected by the schema trigger.
  pub fn transition_workflow_call_state(
    &mut self,
    call_key: &str,
    expected: WorkflowCallIndexState,
    next: WorkflowCallIndexState,
  ) -> Result<(), DurableStoreError> {
    let valid = matches!(
      (expected, next),
      (
        WorkflowCallIndexState::Admitted,
        WorkflowCallIndexState::Running
      ) | (
        WorkflowCallIndexState::Running,
        WorkflowCallIndexState::Succeeded
      ) | (
        WorkflowCallIndexState::Running,
        WorkflowCallIndexState::Failed
      )
    );
    if !valid {
      return Err(DurableStoreError::WorkflowCallHistoryInvalid(
        "workflow call state transition is invalid".to_string(),
      ));
    }
    let changed = self.connection.execute(
      "UPDATE woml_workflow_calls SET state = ?1 WHERE call_key = ?2 AND state = ?3",
      params![
        workflow_call_state_name(next),
        call_key,
        workflow_call_state_name(expected),
      ],
    )?;
    if changed != 1 {
      return Err(DurableStoreError::WorkflowCallHistoryInvalid(
        "workflow call state changed unexpectedly".to_string(),
      ));
    }
    Ok(())
  }

  /// Claims an admitted child for execution. Concurrent callers race through
  /// one conditional update, so exactly one caller becomes the executor and
  /// every other caller observes the same child run.
  pub fn claim_workflow_call_execution(
    &mut self,
    call_key: &str,
  ) -> Result<bool, DurableStoreError> {
    let changed = self.connection.execute(
      "UPDATE woml_workflow_calls SET state = 'running'
       WHERE call_key = ?1 AND state = 'admitted'",
      [call_key],
    )?;
    if changed > 1 {
      return Err(DurableStoreError::WorkflowCallHistoryInvalid(
        "more than one workflow call row was claimed".to_string(),
      ));
    }
    Ok(changed == 1)
  }

  /// Atomically binds one stable parent call identity to exactly one child run.
  /// Repeating the same request returns the existing child; changing any
  /// identity-bearing input fails closed.
  pub fn admit_workflow_call(
    &mut self,
    request: WorkflowCallAdmissionRequest,
  ) -> Result<WorkflowCallAdmissionOutcome, DurableStoreError> {
    validate_workflow_call_admission_request(&request)?;
    let payload_digest = durable_workflow_call_payload_digest(&request.payload)?;
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, &request.parent_run_id)?;
    let parent_workflow = definition_for_run(&transaction, &request.parent_run_id)?;
    if parent_workflow.node(&request.parent_node_id).is_none() {
      return Err(DurableStoreError::WorkflowCallHistoryInvalid(
        "parent node is not present in its bound definition".to_string(),
      ));
    }
    let parent_events = load_events(&transaction, &request.parent_run_id)?;
    let parent_projection = fold_events(&parent_events)?;
    let active_parent_attempt = parent_projection
      .latest_attempt(&request.parent_node_id)
      .is_some_and(|attempt| {
        attempt.identity.attempt == request.parent_attempt
          && attempt.status == AttemptStatus::Started
      });
    if !active_parent_attempt {
      return Err(DurableStoreError::WorkflowCallHistoryInvalid(
        "parent step attempt is not active".to_string(),
      ));
    }

    if workflow_call_would_cycle(
      &transaction,
      &request.parent_run_id,
      &request.target_workflow_id,
    )? {
      return Err(DurableStoreError::WorkflowCallCycle);
    }

    let workflow = definition_by_hash(&transaction, &request.target_definition_hash)?;
    if workflow.workflow_id != request.target_workflow_id {
      return Err(DurableStoreError::WorkflowCallDefinitionMismatch);
    }

    let parent_depth: Option<i64> = transaction
      .query_row(
        "SELECT depth FROM woml_workflow_calls WHERE child_run_id = ?1",
        [&request.parent_run_id],
        |row| row.get(0),
      )
      .optional()?;
    let parent_depth = parent_depth.unwrap_or(0);
    let depth = parent_depth
      .checked_add(1)
      .ok_or(DurableStoreError::WorkflowCallDepthExceeded)?;
    if !(1..=i64::from(MAX_WORKFLOW_CALL_DEPTH)).contains(&depth) {
      return Err(DurableStoreError::WorkflowCallDepthExceeded);
    }

    if let Some(existing) = load_workflow_call(&transaction, &request.call_key)? {
      let identical = existing.parent_run_id == request.parent_run_id
        && existing.parent_node_id == request.parent_node_id
        && existing.target_workflow_id == request.target_workflow_id
        && existing.target_definition_hash == request.target_definition_hash
        && existing.child_run_id == request.child_run_id
        && existing.payload_digest == payload_digest
        && existing.depth == depth as u32;
      if !identical {
        return Err(DurableStoreError::WorkflowCallIdempotencyConflict);
      }
      validate_workflow_call_child(&transaction, &existing, &request.payload)?;
      transaction.commit()?;
      return Ok(WorkflowCallAdmissionOutcome {
        admission: existing,
        duplicate: true,
      });
    }

    if load_run_binding_optional(&transaction, &request.child_run_id)?.is_some() {
      return Err(DurableStoreError::WorkflowCallIdempotencyConflict);
    }

    if workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V12 {
      let queued: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM woml_runtime_policy_queue",
        [],
        |row| row.get(0),
      )?;
      if queued >= RUNTIME_POLICY_QUEUE_CEILING {
        return Err(DurableStoreError::RuntimePolicyQueueFull);
      }
    }

    transaction.execute(
      "INSERT INTO woml_runs(run_id, workflow_id, definition_hash, created_at)
       VALUES (?1, ?2, ?3, ?4)",
      params![
        request.child_run_id,
        request.target_workflow_id,
        request.target_definition_hash,
        request.admitted_at.to_rfc3339(),
      ],
    )?;
    let payload = if workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V12 {
      let policy_hash = workflow.runtime_policy_hash().ok_or_else(|| {
        DurableStoreError::Contract("Model v12 is missing its runtime policy identity.".to_string())
      })?;
      let queue_name = workflow.runtime_policy_queue_name().ok_or_else(|| {
        DurableStoreError::Contract("Model v12 is missing its runtime policy queue.".to_string())
      })?;
      let conflict: bool = transaction.query_row(
        "SELECT EXISTS(
         SELECT 1
         FROM woml_runtime_policy_bindings AS bindings
         JOIN woml_run_summaries AS summaries ON summaries.run_id = bindings.run_id
         WHERE bindings.workflow_id = ?1
           AND summaries.status IN ('queued', 'running', 'waiting', 'cancelling', 'finalizing')
           AND (bindings.definition_hash != ?2 OR bindings.policy_hash != ?3)
       )",
        params![
          request.target_workflow_id,
          request.target_definition_hash,
          policy_hash
        ],
        |row| row.get(0),
      )?;
      if conflict {
        return Err(DurableStoreError::RuntimePolicyDefinitionConflict(
          request.target_workflow_id.clone(),
        ));
      }
      RunEventPayload::RunAdmitted(RunAdmittedData {
        definition_hash: request.target_definition_hash.clone(),
        policy_hash,
        trigger: RunAdmissionTrigger {
          id: "workflow-call".to_string(),
          handler: "trigger.event".to_string(),
        },
        payload: request.payload.clone(),
        queue: RunAdmissionQueue {
          name: queue_name,
          discipline: crate::model::QueueDiscipline::WorkConservingFifo,
        },
        admitted_at: request.admitted_at,
        occurrence_sequence: 0,
      })
    } else {
      RunEventPayload::RunStarted(RunStartedData {
        workflow_id: request.target_workflow_id.clone(),
        definition_hash: request.target_definition_hash.clone(),
        trigger_id: None,
        trigger_handler: None,
        trigger_occurrence_id: None,
        ingress: Some(RunIngress::WorkflowCall {
          call_key: request.call_key.clone(),
        }),
        trigger: request.payload.clone(),
      })
    };
    validate_payload_against_definition(&workflow, &request.target_definition_hash, &payload)
      .map_err(DurableStoreError::Contract)?;
    let mut events = Vec::new();
    append_to_history(
      &transaction,
      &mut events,
      &request.child_run_id,
      generated_event_id(),
      request.admitted_at,
      run_event_schema_version_for_model(workflow.schema_version),
      payload,
    )?;
    validate_event_history_against_definition(&workflow, &request.target_definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;

    transaction.execute(
      "INSERT INTO woml_workflow_calls(
         call_key, parent_run_id, parent_node_id, parent_attempt,
         target_workflow_id, target_definition_hash, child_run_id,
         payload_digest, depth, state, admitted_at
       ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'admitted', ?10)",
      params![
        request.call_key,
        request.parent_run_id,
        request.parent_node_id,
        i64::from(request.parent_attempt),
        request.target_workflow_id,
        request.target_definition_hash,
        request.child_run_id,
        payload_digest,
        depth,
        request.admitted_at.to_rfc3339(),
      ],
    )?;
    let admission = load_workflow_call(&transaction, &request.call_key)?.ok_or_else(|| {
      DurableStoreError::WorkflowCallHistoryInvalid(
        "new admission disappeared before commit".to_string(),
      )
    })?;
    transaction.commit()?;
    Ok(WorkflowCallAdmissionOutcome {
      admission,
      duplicate: false,
    })
  }

  pub fn register_definition_module_artifacts(
    &mut self,
    workflow: &CompiledWorkflowDefinition,
    definition_hash: &str,
    artifacts: &[RuntimeModuleArtifact],
  ) -> Result<(), DurableStoreError> {
    let mut span = crate::performance::PerformanceSpan::new("sqlite", "sqlite.register_definition");
    span.count("nodes", workflow.graph.nodes.len());
    span.count("modules", artifacts.len());
    span.count("transactions", 2);
    self.register_definition(workflow, definition_hash)?;
    validate_definition_module_artifacts(workflow, artifacts)?;

    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut statement = transaction.prepare(
      "SELECT module_name, bundle_digest, source_map_digest, exports_json, bundle, source_map
       FROM woml_definition_module_artifacts
       WHERE definition_hash = ?1
       ORDER BY module_name",
    )?;
    let stored = statement
      .query_map([definition_hash], |row| {
        Ok((
          row.get::<_, String>(0)?,
          row.get::<_, String>(1)?,
          row.get::<_, String>(2)?,
          row.get::<_, String>(3)?,
          row.get::<_, String>(4)?,
          row.get::<_, String>(5)?,
        ))
      })?
      .collect::<Result<Vec<_>, _>>()?;
    drop(statement);

    if !stored.is_empty() {
      if stored.len() != artifacts.len() {
        return Err(DurableStoreError::DefinitionConflict(
          definition_hash.to_string(),
        ));
      }
      for (stored, incoming) in stored.iter().zip(artifacts) {
        let exports: Vec<String> = serde_json::from_str(&stored.3)?;
        if stored.0 != incoming.name
          || stored.1 != incoming.bundle_digest
          || stored.2 != incoming.source_map_digest
          || exports != incoming.exports
          || stored.4 != incoming.bundle
          || stored.5 != incoming.source_map
        {
          return Err(DurableStoreError::DefinitionConflict(
            definition_hash.to_string(),
          ));
        }
      }
      transaction.commit()?;
      span.succeed();
      return Ok(());
    }

    for artifact in artifacts {
      transaction.execute(
        "INSERT INTO woml_definition_module_artifacts(
           definition_hash, module_name, bundle_digest, source_map_digest,
           exports_json, bundle, source_map, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
          definition_hash,
          artifact.name,
          artifact.bundle_digest,
          artifact.source_map_digest,
          serde_json::to_string(&artifact.exports)?,
          artifact.bundle,
          artifact.source_map,
          Utc::now().to_rfc3339(),
        ],
      )?;
    }
    transaction.commit()?;
    span.succeed();
    Ok(())
  }

  pub fn validate_runtime_policy_activation(
    &self,
    workflow: &CompiledWorkflowDefinition,
    definition_hash: &str,
  ) -> Result<(), DurableStoreError> {
    if workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V12 {
      return Ok(());
    }
    let policy_hash = workflow.runtime_policy_hash().ok_or_else(|| {
      DurableStoreError::Contract("Model v12 is missing its runtime policy identity.".to_string())
    })?;
    let conflict: bool = self.connection.query_row(
      "SELECT EXISTS(
       SELECT 1
       FROM woml_runtime_policy_bindings AS bindings
       JOIN woml_run_summaries AS summaries ON summaries.run_id = bindings.run_id
       WHERE bindings.workflow_id = ?1
         AND summaries.status IN ('queued', 'running', 'waiting', 'cancelling', 'finalizing')
         AND (bindings.definition_hash != ?2 OR bindings.policy_hash != ?3)
     )",
      params![workflow.workflow_id, definition_hash, policy_hash],
      |row| row.get(0),
    )?;
    if conflict {
      return Err(DurableStoreError::RuntimePolicyDefinitionConflict(
        workflow.workflow_id.clone(),
      ));
    }
    Ok(())
  }

  pub fn definition_module_artifacts(
    &self,
    definition_hash: &str,
  ) -> Result<Vec<RuntimeModuleArtifact>, DurableStoreError> {
    let workflow = self.definition(definition_hash)?;
    let mut statement = self.connection.prepare(
      "SELECT module_name, bundle_digest, source_map_digest, exports_json, bundle, source_map
       FROM woml_definition_module_artifacts
       WHERE definition_hash = ?1
       ORDER BY module_name",
    )?;
    let artifacts = statement
      .query_map([definition_hash], |row| {
        let exports_json: String = row.get(3)?;
        let exports = serde_json::from_str(&exports_json).map_err(|error| {
          rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(error))
        })?;
        Ok(RuntimeModuleArtifact {
          name: row.get(0)?,
          bundle_digest: row.get(1)?,
          source_map_digest: row.get(2)?,
          exports,
          bundle: row.get(4)?,
          source_map: row.get(5)?,
        })
      })?
      .collect::<Result<Vec<_>, _>>()?;
    validate_definition_module_artifacts(&workflow, &artifacts)?;
    Ok(artifacts)
  }

  pub fn run_binding(&self, run_id: &str) -> Result<RunDefinitionBinding, DurableStoreError> {
    let mut statement = self
      .connection
      .prepare_cached("SELECT workflow_id, definition_hash FROM woml_runs WHERE run_id = ?1")?;
    statement
      .query_row([run_id], |row| {
        Ok(RunDefinitionBinding {
          run_id: run_id.to_string(),
          workflow_id: row.get(0)?,
          definition_hash: row.get(1)?,
        })
      })
      .optional()?
      .ok_or_else(|| DurableStoreError::RunNotFound(run_id.to_string()))
  }

  pub fn trigger_occurrence(
    &self,
    occurrence_id: &str,
  ) -> Result<TriggerOccurrence, DurableStoreError> {
    load_trigger_occurrence_by_id(&self.connection, occurrence_id)?.ok_or_else(|| {
      DurableStoreError::TriggerHistoryInvalid(format!(
        "trigger occurrence {occurrence_id:?} does not exist"
      ))
    })
  }

  pub fn admit_trigger_occurrence(
    &mut self,
    request: TriggerAdmissionRequest,
  ) -> Result<TriggerAdmissionOutcome, DurableStoreError> {
    let mut span = crate::performance::PerformanceSpan::new("sqlite", "sqlite.admit_trigger");
    span.count("transactions", 1);
    validate_trigger_admission_request(&request)?;
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let outcome = admit_trigger_occurrence_in_transaction(&transaction, &request)?;
    transaction.commit()?;
    span.run_id(outcome.run_id.clone());
    span.count("duplicate", usize::from(outcome.duplicate));
    span.succeed();
    Ok(outcome)
  }

  /// Admits one subscriber delivery for a workflow-originated named event and
  /// stores its hidden lineage in the same transaction. Repeating the stable
  /// publication identity safely returns the original child run.
  pub fn admit_internal_event_occurrence(
    &mut self,
    request: InternalEventAdmissionRequest,
  ) -> Result<InternalEventAdmissionOutcome, DurableStoreError> {
    validate_trigger_admission_request(&request.trigger)?;
    if request.trigger.trigger_handler != "trigger.event"
      || !request.publication_id.starts_with("internal:v1:sha256:")
      || request.publication_id.len() != 83
      || request.event_name.is_empty()
      || request.event_name.len() > 256
    {
      return Err(DurableStoreError::Contract(
        "Internal event admission identity is invalid.".to_string(),
      ));
    }
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, &request.parent_run_id)?;
    let payload_hash = canonical_payload_hash(&request.trigger.payload)?;
    let parent_depth: Option<u32> = transaction
      .query_row(
        "SELECT publications.depth
         FROM woml_internal_event_deliveries AS deliveries
         JOIN woml_internal_event_publications AS publications
           ON publications.publication_id = deliveries.publication_id
         WHERE deliveries.run_id = ?1",
        [&request.parent_run_id],
        |row| row.get(0),
      )
      .optional()?;
    let depth = parent_depth.unwrap_or(0).saturating_add(1);
    if depth > 32 {
      return Err(DurableStoreError::InternalEventDepthExceeded);
    }

    let mut ancestor_run = Some(request.parent_run_id.clone());
    let mut inspected = 0;
    while let Some(run_id) = ancestor_run {
      inspected += 1;
      if inspected > 32 {
        return Err(DurableStoreError::InternalEventDepthExceeded);
      }
      let source: Option<(String, String)> = transaction
        .query_row(
          "SELECT workflow_id, trigger_id FROM woml_trigger_occurrences WHERE run_id = ?1",
          [&run_id],
          |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
      if source.as_ref()
        == Some(&(
          request.trigger.workflow_id.clone(),
          request.trigger.trigger_id.clone(),
        ))
      {
        return Err(DurableStoreError::InternalEventCycle);
      }
      ancestor_run = transaction
        .query_row(
          "SELECT publications.parent_run_id
           FROM woml_internal_event_deliveries AS deliveries
           JOIN woml_internal_event_publications AS publications
             ON publications.publication_id = deliveries.publication_id
           WHERE deliveries.run_id = ?1",
          [&run_id],
          |row| row.get(0),
        )
        .optional()?;
    }

    let existing: Option<(String, String, String, u32)> = transaction
      .query_row(
        "SELECT parent_run_id, event_name, payload_hash, depth
         FROM woml_internal_event_publications WHERE publication_id = ?1",
        [&request.publication_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
      )
      .optional()?;
    if let Some(existing) = existing {
      if existing
        != (
          request.parent_run_id.clone(),
          request.event_name.clone(),
          payload_hash.clone(),
          depth,
        )
      {
        return Err(DurableStoreError::InternalEventIdempotencyConflict);
      }
    } else {
      transaction.execute(
        "INSERT INTO woml_internal_event_publications(
           publication_id, parent_run_id, event_name, payload_hash, depth, emitted_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
          request.publication_id,
          request.parent_run_id,
          request.event_name,
          payload_hash,
          depth,
          request.emitted_at.to_rfc3339(),
        ],
      )?;
    }

    let outcome = admit_trigger_occurrence_in_transaction(&transaction, &request.trigger)?;
    let existing_delivery: Option<String> = transaction
      .query_row(
        "SELECT run_id FROM woml_internal_event_deliveries
         WHERE publication_id = ?1 AND workflow_id = ?2 AND trigger_id = ?3",
        params![
          request.publication_id,
          request.trigger.workflow_id,
          request.trigger.trigger_id,
        ],
        |row| row.get(0),
      )
      .optional()?;
    if let Some(run_id) = existing_delivery {
      if run_id != outcome.run_id {
        return Err(DurableStoreError::InternalEventIdempotencyConflict);
      }
    } else {
      transaction.execute(
        "INSERT INTO woml_internal_event_deliveries(
           publication_id, workflow_id, trigger_id, run_id
         ) VALUES (?1, ?2, ?3, ?4)",
        params![
          request.publication_id,
          request.trigger.workflow_id,
          request.trigger.trigger_id,
          outcome.run_id,
        ],
      )?;
    }
    transaction.commit()?;
    Ok(InternalEventAdmissionOutcome {
      depth,
      occurrence: outcome,
    })
  }

  pub fn register_schedule_cursor(
    &mut self,
    registration: &ScheduleCursorRegistration,
    initial_next_scheduled_at: DateTime<Utc>,
    registered_at: DateTime<Utc>,
  ) -> Result<ScheduleCursorRegistrationOutcome, DurableStoreError> {
    validate_schedule_cursor_registration(registration)?;
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    validate_schedule_registration_definition(&transaction, registration)?;
    let existing = load_schedule_cursor(
      &transaction,
      &registration.workflow_id,
      &registration.trigger_id,
    )?;
    let initialized = existing.as_ref().is_none_or(|cursor| {
      cursor.definition_hash != registration.definition_hash
        || cursor.cron != registration.cron
        || cursor.timezone != registration.timezone
        || cursor.on_missed != registration.on_missed
    });
    if initialized {
      transaction.execute(
        "INSERT INTO woml_schedule_cursors(
           workflow_id, trigger_id, definition_hash, cron, timezone, on_missed,
           next_scheduled_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(workflow_id, trigger_id) DO UPDATE SET
           definition_hash = excluded.definition_hash,
           cron = excluded.cron,
           timezone = excluded.timezone,
           on_missed = excluded.on_missed,
           next_scheduled_at = excluded.next_scheduled_at,
           updated_at = excluded.updated_at",
        params![
          registration.workflow_id,
          registration.trigger_id,
          registration.definition_hash,
          registration.cron,
          registration.timezone,
          registration.on_missed,
          initial_next_scheduled_at.to_rfc3339(),
          registered_at.to_rfc3339(),
        ],
      )?;
    }
    let cursor = load_schedule_cursor(
      &transaction,
      &registration.workflow_id,
      &registration.trigger_id,
    )?
    .ok_or_else(|| DurableStoreError::Contract("Schedule cursor was not stored.".to_string()))?;
    transaction.commit()?;
    Ok(ScheduleCursorRegistrationOutcome {
      cursor,
      initialized,
    })
  }

  pub fn schedule_cursor(
    &self,
    workflow_id: &str,
    trigger_id: &str,
  ) -> Result<ScheduleCursor, DurableStoreError> {
    load_schedule_cursor(&self.connection, workflow_id, trigger_id)?.ok_or_else(|| {
      DurableStoreError::Contract(format!(
        "Schedule cursor for workflow {workflow_id:?}, trigger {trigger_id:?} does not exist."
      ))
    })
  }

  pub fn advance_schedule_cursor(
    &mut self,
    workflow_id: &str,
    trigger_id: &str,
    expected_next_scheduled_at: DateTime<Utc>,
    next_scheduled_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
  ) -> Result<ScheduleCursor, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    update_schedule_cursor(
      &transaction,
      workflow_id,
      trigger_id,
      expected_next_scheduled_at,
      next_scheduled_at,
      updated_at,
    )?;
    let cursor = load_schedule_cursor(&transaction, workflow_id, trigger_id)?
      .ok_or(DurableStoreError::ScheduleCursorConflict)?;
    transaction.commit()?;
    Ok(cursor)
  }

  pub fn claim_schedule_occurrence(
    &mut self,
    expected_next_scheduled_at: DateTime<Utc>,
    next_scheduled_at: DateTime<Utc>,
    request: TriggerAdmissionRequest,
  ) -> Result<(TriggerAdmissionOutcome, ScheduleCursor), DurableStoreError> {
    validate_trigger_admission_request(&request)?;
    if request.trigger_handler != "trigger.schedule" {
      return Err(DurableStoreError::TriggerHandlerMismatch);
    }
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let outcome = admit_trigger_occurrence_in_transaction(&transaction, &request)?;
    update_schedule_cursor(
      &transaction,
      &request.workflow_id,
      &request.trigger_id,
      expected_next_scheduled_at,
      next_scheduled_at,
      request.received_at,
    )?;
    let cursor = load_schedule_cursor(&transaction, &request.workflow_id, &request.trigger_id)?
      .ok_or(DurableStoreError::ScheduleCursorConflict)?;
    transaction.commit()?;
    Ok((outcome, cursor))
  }

  pub fn register_interval_cursor(
    &mut self,
    registration: &IntervalCursorRegistration,
    anchor_at: DateTime<Utc>,
    registered_at: DateTime<Utc>,
  ) -> Result<IntervalCursorRegistrationOutcome, DurableStoreError> {
    validate_interval_cursor_registration(registration)?;
    let interval = crate::interval::WomlInterval::new(registration.every_ms)
      .map_err(|error| DurableStoreError::Contract(error.to_string()))?;
    let anchor_at = crate::interval::WomlInterval::normalize_anchor(anchor_at)
      .map_err(|error| DurableStoreError::Contract(error.to_string()))?;
    let initial_next_scheduled_at = interval
      .planned_at(anchor_at, 1)
      .map_err(|error| DurableStoreError::Contract(error.to_string()))?;
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    validate_interval_registration_definition(&transaction, registration)?;
    let existing = load_interval_cursor(
      &transaction,
      &registration.workflow_id,
      &registration.trigger_id,
    )?;
    let initialized = existing.as_ref().is_none_or(|cursor| {
      cursor.definition_hash != registration.definition_hash
        || cursor.every_ms != registration.every_ms
        || cursor.on_missed != registration.on_missed
    });
    if initialized {
      transaction.execute(
        "INSERT INTO woml_interval_cursors(
           workflow_id, trigger_id, definition_hash, every_ms, on_missed,
           anchor_at, next_sequence, next_scheduled_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?8)
         ON CONFLICT(workflow_id, trigger_id) DO UPDATE SET
           definition_hash = excluded.definition_hash,
           every_ms = excluded.every_ms,
           on_missed = excluded.on_missed,
           anchor_at = excluded.anchor_at,
           next_sequence = excluded.next_sequence,
           next_scheduled_at = excluded.next_scheduled_at,
           updated_at = excluded.updated_at",
        params![
          registration.workflow_id,
          registration.trigger_id,
          registration.definition_hash,
          i64::try_from(registration.every_ms).map_err(|_| DurableStoreError::Contract(
            "Interval duration exceeds the durable integer range.".to_string()
          ))?,
          registration.on_missed,
          anchor_at.to_rfc3339(),
          initial_next_scheduled_at.to_rfc3339(),
          registered_at.to_rfc3339(),
        ],
      )?;
    }
    let cursor = load_interval_cursor(
      &transaction,
      &registration.workflow_id,
      &registration.trigger_id,
    )?
    .ok_or_else(|| DurableStoreError::Contract("Interval cursor was not stored.".to_string()))?;
    transaction.commit()?;
    Ok(IntervalCursorRegistrationOutcome {
      cursor,
      initialized,
    })
  }

  pub fn interval_cursor(
    &self,
    workflow_id: &str,
    trigger_id: &str,
  ) -> Result<IntervalCursor, DurableStoreError> {
    load_interval_cursor(&self.connection, workflow_id, trigger_id)?.ok_or_else(|| {
      DurableStoreError::Contract(format!(
        "Interval cursor for workflow {workflow_id:?}, trigger {trigger_id:?} does not exist."
      ))
    })
  }

  #[allow(clippy::too_many_arguments)]
  pub fn advance_interval_cursor(
    &mut self,
    workflow_id: &str,
    trigger_id: &str,
    expected_sequence: u64,
    expected_scheduled_at: DateTime<Utc>,
    next_sequence: u64,
    next_scheduled_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
  ) -> Result<IntervalCursor, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    update_interval_cursor(
      &transaction,
      workflow_id,
      trigger_id,
      expected_sequence,
      expected_scheduled_at,
      next_sequence,
      next_scheduled_at,
      updated_at,
    )?;
    let cursor = load_interval_cursor(&transaction, workflow_id, trigger_id)?
      .ok_or(DurableStoreError::IntervalCursorConflict)?;
    transaction.commit()?;
    Ok(cursor)
  }

  #[allow(clippy::too_many_arguments)]
  pub fn claim_interval_occurrence(
    &mut self,
    expected_sequence: u64,
    expected_scheduled_at: DateTime<Utc>,
    next_sequence: u64,
    next_scheduled_at: DateTime<Utc>,
    request: TriggerAdmissionRequest,
  ) -> Result<(TriggerAdmissionOutcome, IntervalCursor), DurableStoreError> {
    validate_trigger_admission_request(&request)?;
    if request.trigger_handler != "trigger.interval" {
      return Err(DurableStoreError::TriggerHandlerMismatch);
    }
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let outcome = admit_trigger_occurrence_in_transaction(&transaction, &request)?;
    update_interval_cursor(
      &transaction,
      &request.workflow_id,
      &request.trigger_id,
      expected_sequence,
      expected_scheduled_at,
      next_sequence,
      next_scheduled_at,
      request.received_at,
    )?;
    let cursor = load_interval_cursor(&transaction, &request.workflow_id, &request.trigger_id)?
      .ok_or(DurableStoreError::IntervalCursorConflict)?;
    transaction.commit()?;
    Ok((outcome, cursor))
  }

  pub fn recover_undispatched_trigger_runs(
    &self,
  ) -> Result<Vec<TriggerRecoveryWork>, DurableStoreError> {
    let now = Utc::now().to_rfc3339();
    let mut statement = self.connection.prepare(
      "SELECT occurrences.occurrence_id
       FROM woml_trigger_occurrences AS occurrences
       JOIN woml_runs AS runs ON runs.run_id = occurrences.run_id
       JOIN woml_run_summaries AS summaries ON summaries.run_id = occurrences.run_id
       WHERE
         (
           SELECT COUNT(*) FROM woml_run_events AS events
           WHERE events.run_id = occurrences.run_id
         ) = 1
         OR (
           EXISTS (
             SELECT 1 FROM woml_run_events AS events
             WHERE events.run_id = occurrences.run_id
               AND events.event_schema_version = 11
           )
           AND summaries.status IN ('queued', 'running', 'cancelling', 'finalizing')
           AND NOT EXISTS (
             SELECT 1 FROM woml_scheduler_claims AS claims
             WHERE claims.run_id = occurrences.run_id
               AND claims.expires_at > ?1
           )
         )
       ORDER BY occurrences.received_at, occurrences.occurrence_id",
    )?;
    let occurrence_ids = statement
      .query_map([now], |row| row.get::<_, String>(0))?
      .collect::<Result<Vec<_>, _>>()?;
    drop(statement);

    occurrence_ids
      .into_iter()
      .map(|occurrence_id| {
        let occurrence = load_trigger_occurrence_by_id(&self.connection, &occurrence_id)?
          .ok_or_else(|| {
            DurableStoreError::TriggerHistoryInvalid(format!(
              "trigger occurrence {occurrence_id:?} disappeared during recovery"
            ))
          })?;
        let trigger = validate_trigger_occurrence_history(&self.connection, &occurrence)?;
        Ok(TriggerRecoveryWork {
          occurrence,
          trigger,
        })
      })
      .collect()
  }

  pub fn start_run(
    &mut self,
    event_id: impl Into<String>,
    run_id: impl Into<String>,
    occurred_at: DateTime<Utc>,
    workflow_id: impl Into<String>,
    definition_hash: impl Into<String>,
    trigger: Map<String, Value>,
  ) -> Result<(RunEvent, RunProjection), DurableStoreError> {
    let workflow_id = workflow_id.into();
    let definition_hash = definition_hash.into();
    self.append_payload(
      run_id.into(),
      event_id.into(),
      occurred_at,
      RunEventPayload::RunStarted(RunStartedData {
        workflow_id,
        definition_hash,
        trigger_id: None,
        trigger_handler: None,
        trigger_occurrence_id: None,
        ingress: None,
        trigger,
      }),
    )
  }

  pub fn append_payload(
    &mut self,
    run_id: impl Into<String>,
    event_id: impl Into<String>,
    occurred_at: DateTime<Utc>,
    payload: RunEventPayload,
  ) -> Result<(RunEvent, RunProjection), DurableStoreError> {
    self.append_payload_scoped(run_id, event_id, occurred_at, None, payload)
  }

  pub fn append_payload_scoped(
    &mut self,
    run_id: impl Into<String>,
    event_id: impl Into<String>,
    occurred_at: DateTime<Utc>,
    iteration: Option<crate::event::ForEachIterationScope>,
    payload: RunEventPayload,
  ) -> Result<(RunEvent, RunProjection), DurableStoreError> {
    let run_id = run_id.into();
    let event_id = event_id.into();
    let mut span = crate::performance::PerformanceSpan::new("sqlite", "sqlite.append_events");
    span.run_id(run_id.clone());
    span.count("transactions", 1);
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;

    let event_schema_version = match &payload {
      RunEventPayload::RunAdmitted(data) => {
        let existing: Option<String> = transaction
          .query_row(
            "SELECT run_id FROM woml_runs WHERE run_id = ?1",
            [&run_id],
            |row| row.get(0),
          )
          .optional()?;
        if existing.is_some() {
          return Err(DurableStoreError::RunAlreadyExists(run_id));
        }
        let registered: Option<(String, i64)> = transaction
          .query_row(
            "SELECT workflow_id, schema_version FROM woml_definitions WHERE definition_hash = ?1",
            [&data.definition_hash],
            |row| Ok((row.get(0)?, row.get(1)?)),
          )
          .optional()?;
        let (workflow_id, model_schema_version) = registered
          .ok_or_else(|| DurableStoreError::DefinitionNotFound(data.definition_hash.clone()))?;
        let model_schema_version = u32::try_from(model_schema_version).map_err(|_| {
          DurableStoreError::Contract(
            "Stored compiled-model schema version is invalid.".to_string(),
          )
        })?;
        let event_schema_version = run_event_schema_version_for_model(model_schema_version);
        if event_schema_version < RUN_EVENT_SCHEMA_VERSION_V11 {
          return Err(DurableStoreError::Contract(
            "run_admitted requires compiled Model v12 or later.".to_string(),
          ));
        }
        transaction.execute(
          "INSERT INTO woml_runs(run_id, workflow_id, definition_hash, created_at)
           VALUES (?1, ?2, ?3, ?4)",
          params![
            run_id,
            workflow_id,
            data.definition_hash,
            occurred_at.to_rfc3339(),
          ],
        )?;
        event_schema_version
      }
      RunEventPayload::RunStarted(data) => {
        let existing: Option<String> = transaction
          .query_row(
            "SELECT run_id FROM woml_runs WHERE run_id = ?1",
            [&run_id],
            |row| row.get(0),
          )
          .optional()?;
        if existing.is_some() {
          return Err(DurableStoreError::RunAlreadyExists(run_id));
        }
        let registered_workflow: Option<(String, i64)> = transaction
          .query_row(
            "SELECT workflow_id, schema_version FROM woml_definitions WHERE definition_hash = ?1",
            [&data.definition_hash],
            |row| Ok((row.get(0)?, row.get(1)?)),
          )
          .optional()?;
        let (registered_workflow, model_schema_version) = registered_workflow
          .ok_or_else(|| DurableStoreError::DefinitionNotFound(data.definition_hash.clone()))?;
        if registered_workflow != data.workflow_id {
          return Err(DurableStoreError::Contract(
            "run_started workflowId does not match its registered definition.".to_string(),
          ));
        }
        transaction.execute(
          "INSERT INTO woml_runs(run_id, workflow_id, definition_hash, created_at)
           VALUES (?1, ?2, ?3, ?4)",
          params![
            run_id,
            data.workflow_id,
            data.definition_hash,
            occurred_at.to_rfc3339(),
          ],
        )?;
        let model_schema_version = u32::try_from(model_schema_version).map_err(|_| {
          DurableStoreError::Contract(
            "Stored compiled-model schema version is invalid.".to_string(),
          )
        })?;
        run_event_schema_version_for_model(model_schema_version)
      }
      _ => {
        ensure_run_exists(&transaction, &run_id)?;
        let existing = load_events(&transaction, &run_id)?;
        existing
          .first()
          .map(|event| event.event_schema_version)
          .ok_or_else(|| {
            DurableStoreError::Contract(
              "A non-start event cannot be appended before run_started.".to_string(),
            )
          })?
      }
    };

    let mut events = load_events(&transaction, &run_id)?;
    let mut projection_state =
      projection_state_for_events(&self.projection_cache, &run_id, &events)?;
    let existing_event_count = events.len();
    let workflow = definition_for_run(&transaction, &run_id)?;
    let binding = run_binding_in_transaction(&transaction, &run_id)?;
    let payloads = if iteration.is_some() {
      vec![payload]
    } else if matches!(
      event_schema_version,
      crate::RUN_EVENT_SCHEMA_VERSION_V10
        | crate::RUN_EVENT_SCHEMA_VERSION_V11
        | crate::RUN_EVENT_SCHEMA_VERSION_V12
        | crate::RUN_EVENT_SCHEMA_VERSION_V13
        | crate::RUN_EVENT_SCHEMA_VERSION_V14
        | crate::RUN_EVENT_SCHEMA_VERSION_V15
        | crate::RUN_EVENT_SCHEMA_VERSION_V16
        | crate::RUN_EVENT_SCHEMA_VERSION_V17
        | crate::RUN_EVENT_SCHEMA_VERSION_V18
        | crate::RUN_EVENT_SCHEMA_VERSION_V19
        | crate::RUN_EVENT_SCHEMA_VERSION_V20
    ) {
      expand_model_v11_payload(&workflow, &run_id, payload)?
    } else {
      vec![payload]
    };
    let mut first_event = None;
    for (index, payload) in payloads.into_iter().enumerate() {
      if let Some(scope) = iteration.as_ref() {
        validate_scoped_payload_against_definition(
          &workflow,
          &binding.definition_hash,
          &run_id,
          scope,
          &payload,
        )
        .map_err(DurableStoreError::Contract)?;
      } else {
        validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
          .map_err(DurableStoreError::Contract)?;
      }
      if iteration.is_none() {
        if let RunEventPayload::BranchSelected(data) = &payload {
          let projection = projection_state.projection();
          if projection.branch_selections.contains_key(&data.branch_id) {
            return Err(DurableStoreError::Contract(format!(
              "Branch {:?} already has an immutable selection.",
              data.branch_id
            )));
          }
          let selector_id = format!("__woml_branch__{}__select", data.branch_id);
          let ready =
            ready_node_ids_for_projection(&workflow, &binding.definition_hash, projection)
              .map_err(DurableStoreError::Contract)?;
          if !ready.iter().any(|node_id| node_id == &selector_id) {
            return Err(DurableStoreError::Contract(format!(
              "Branch selector {selector_id:?} is not ready for selection."
            )));
          }
        }
      }
      let event = append_to_history_scoped_incremental(
        &transaction,
        &mut events,
        &mut projection_state,
        &run_id,
        if index == 0 {
          event_id.clone()
        } else {
          generated_event_id()
        },
        occurred_at,
        event_schema_version,
        iteration.clone(),
        payload,
        true,
      )?;
      first_event.get_or_insert(event);
    }
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = projection_state.projection().clone();
    if matches!(
      event_schema_version,
      crate::RUN_EVENT_SCHEMA_VERSION_V18
        | crate::RUN_EVENT_SCHEMA_VERSION_V19
        | crate::RUN_EVENT_SCHEMA_VERSION_V20
    ) {
      write_correction_indexes_from_projection(
        &transaction,
        &run_id,
        &binding.definition_hash,
        &events,
        &projection,
      )?;
    }
    transaction.commit()?;
    cache_projection_state(&self.projection_cache, &run_id, projection_state);
    span.count("events", events.len().saturating_sub(existing_event_count));
    span.succeed();
    Ok((first_event.unwrap(), projection))
  }

  /// Persists one immutable review snapshot and its artifact pins before any
  /// decision capability may be exposed. Model v20 execution is enabled only
  /// in RPA5; this method is the RPA2/RPA3 storage authority used by that
  /// future execution path.
  pub fn create_rich_approval_request(
    &mut self,
    binding: &BoundRichApprovalReview,
    created_at: DateTime<Utc>,
  ) -> Result<RunProjection, DurableStoreError> {
    verify_bound_rich_approval_review(binding)
      .map_err(|_| DurableStoreError::RichApprovalBindingMismatch)?;
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let run_binding = run_binding_in_transaction(&transaction, &binding.run_id)?;
    let workflow = definition_for_run(&transaction, &binding.run_id)?;
    let workflow_version = workflow
      .metadata
      .as_ref()
      .and_then(|metadata| metadata.version.as_deref())
      .unwrap_or("0.0.0");
    let approval = workflow.approval(&binding.approval_id).ok_or_else(|| {
      DurableStoreError::Contract("The rich approval is not in Model v20.".into())
    })?;
    if workflow.schema_version != crate::COMPILED_MODEL_SCHEMA_VERSION_V20
      || approval.review.is_none()
      || approval.required_approvals != Some(binding.required_approvals)
      || approval.required_roles != binding.required_roles
      || run_binding.definition_hash != binding.definition_hash
      || workflow.workflow_id != binding.workflow_id
      || workflow_version != binding.workflow_version
    {
      return Err(DurableStoreError::RichApprovalBindingMismatch);
    }
    let review_json = String::from_utf8(canonical_json(&binding.review)?).map_err(|_| {
      DurableStoreError::Contract("The immutable review is not valid UTF-8 JSON.".to_string())
    })?;
    let required_roles_json = serde_json::to_string(&binding.required_roles)?;
    let inserted = transaction.execute(
      "INSERT OR IGNORE INTO woml_approval_reviews(
         run_id, approval_id, request_id, definition_hash, workflow_version,
         review_json, review_digest, required_approvals, required_roles_json,
         directory_version, status, created_at, resolved_at
       ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'pending', ?11, NULL)",
      params![
        binding.run_id,
        binding.approval_id,
        binding.request_id,
        binding.definition_hash,
        binding.workflow_version,
        review_json,
        binding.review_digest,
        i64::from(binding.required_approvals),
        required_roles_json,
        binding.directory_version,
        created_at.to_rfc3339(),
      ],
    )?;
    if inserted == 0 {
      let existing: Option<(String, String)> = transaction
        .query_row(
          "SELECT request_id, review_digest FROM woml_approval_reviews
           WHERE run_id = ?1 AND approval_id = ?2",
          params![binding.run_id, binding.approval_id],
          |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
      if existing.as_ref() == Some(&(binding.request_id.clone(), binding.review_digest.clone())) {
        return Ok(fold_events(&load_events(&transaction, &binding.run_id)?)?);
      }
      return Err(DurableStoreError::RichApprovalBindingMismatch);
    }
    for artifact in &binding.artifacts {
      transaction.execute(
        "INSERT INTO woml_approval_artifact_pins(
           run_id, request_id, artifact_id, artifact_sha256, released_at
         ) VALUES (?1, ?2, ?3, ?4, NULL)",
        params![
          binding.run_id,
          binding.request_id,
          artifact.id,
          artifact.sha256
        ],
      )?;
    }

    let mut events = load_events(&transaction, &binding.run_id)?;
    let mut projection_state =
      projection_state_for_events(&self.projection_cache, &binding.run_id, &events)?;
    let event_schema_version = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| {
        DurableStoreError::Contract("A rich approval requires a started run.".into())
      })?;
    if !matches!(
      event_schema_version,
      crate::RUN_EVENT_SCHEMA_VERSION_V19 | crate::RUN_EVENT_SCHEMA_VERSION_V20
    ) {
      return Err(DurableStoreError::Contract(
        "A rich approval request requires Event v19 or later.".to_string(),
      ));
    }
    append_to_history_scoped_incremental(
      &transaction,
      &mut events,
      &mut projection_state,
      &binding.run_id,
      generated_event_id(),
      created_at,
      event_schema_version,
      None,
      RunEventPayload::ApprovalReviewRequested(ApprovalReviewRequestedData {
        approval_id: binding.approval_id.clone(),
        request_id: binding.request_id.clone(),
        review_digest: binding.review_digest.clone(),
        directory_version: binding.directory_version.clone(),
        required: binding.required_approvals,
        required_roles: binding.required_roles.clone(),
      }),
      true,
    )?;
    let projection = projection_state.projection().clone();
    transaction.commit()?;
    cache_projection_state(&self.projection_cache, &binding.run_id, projection_state);
    Ok(projection)
  }

  pub fn record_rich_approval_decision(
    &mut self,
    admission: RichApprovalDecisionAdmission,
  ) -> Result<RichApprovalDecisionOutcome, DurableStoreError> {
    validate_rich_decision_admission(&admission)?;
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let review = rich_review_row(&transaction, &admission.run_id, &admission.request_id)?
      .ok_or(DurableStoreError::RichApprovalNotFound)?;
    if review.review_digest != admission.review_digest {
      return Err(DurableStoreError::RichApprovalBindingMismatch);
    }
    let current = rich_quorum_state(&transaction, &review)?;
    if review.status != "pending" {
      return Ok(RichApprovalDecisionOutcome {
        status: RichApprovalDecisionStatus::AlreadyResolved,
        required: review.required,
        received: current.received,
        represented_roles: current.represented_roles,
      });
    }
    let existing: Option<String> = transaction
      .query_row(
        "SELECT decision FROM woml_approval_reviewer_decisions
         WHERE run_id = ?1 AND request_id = ?2 AND principal_id = ?3",
        params![
          admission.run_id,
          admission.request_id,
          admission.principal_id
        ],
        |row| row.get(0),
      )
      .optional()?;
    if let Some(existing) = existing {
      if existing != approval_decision_text(admission.decision) {
        return Err(DurableStoreError::RichApprovalReviewerConflict);
      }
      return Ok(RichApprovalDecisionOutcome {
        status: RichApprovalDecisionStatus::Idempotent,
        required: review.required,
        received: current.received,
        represented_roles: current.represented_roles,
      });
    }
    transaction.execute(
      "INSERT INTO woml_approval_reviewer_decisions(
         run_id, request_id, principal_id, decision, provider, provider_account_id,
         provider_actor_id, roles_json, decided_at
       ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
      params![
        admission.run_id,
        admission.request_id,
        admission.principal_id,
        approval_decision_text(admission.decision),
        admission.provider,
        admission.provider_account_id,
        admission.provider_actor_id,
        serde_json::to_string(&admission.roles)?,
        admission.decided_at.to_rfc3339(),
      ],
    )?;
    let quorum = rich_quorum_state(&transaction, &review)?;
    let terminal = if admission.decision == ApprovalDecision::Rejected {
      Some((
        "rejected",
        RichApprovalResolutionReason::Rejection,
        ApprovalDecision::Rejected,
      ))
    } else if quorum.received >= review.required
      && review
        .required_roles
        .iter()
        .all(|role| quorum.represented_roles.contains(role))
    {
      Some((
        "approved",
        RichApprovalResolutionReason::Quorum,
        ApprovalDecision::Approved,
      ))
    } else {
      None
    };
    if let Some((status, _, _)) = terminal {
      settle_rich_review_row(
        &transaction,
        &admission.run_id,
        &admission.request_id,
        status,
        admission.decided_at,
      )?;
    }

    let mut payloads = vec![RunEventPayload::ApprovalReviewerDecisionRecorded(
      ApprovalReviewerDecisionRecordedData {
        approval_id: review.approval_id.clone(),
        request_id: admission.request_id.clone(),
        review_digest: admission.review_digest.clone(),
        principal_id: admission.principal_id.clone(),
        decision: admission.decision,
      },
    )];
    if admission.decision == ApprovalDecision::Approved {
      payloads.push(RunEventPayload::ApprovalQuorumProgressed(
        ApprovalQuorumProgressedData {
          approval_id: review.approval_id.clone(),
          request_id: admission.request_id.clone(),
          review_digest: admission.review_digest.clone(),
          required: review.required,
          received: quorum.received,
          required_roles: review.required_roles.clone(),
          represented_roles: quorum.represented_roles.clone(),
        },
      ));
    }
    if let Some((_, reason, decision)) = terminal {
      payloads.push(RunEventPayload::ApprovalResolved(ApprovalResolvedData {
        approval_id: review.approval_id.clone(),
        request_id: admission.request_id.clone(),
        resolution: None,
        review_digest: Some(admission.review_digest.clone()),
        decision: Some(decision),
        reason: Some(reason),
        required: Some(review.required),
        received: Some(quorum.received),
        required_roles: Some(review.required_roles.clone()),
        represented_roles: Some(quorum.represented_roles.clone()),
      }));
    }
    append_rich_approval_payloads(
      &transaction,
      &self.projection_cache,
      &admission.run_id,
      admission.decided_at,
      payloads,
    )?;
    transaction.commit()?;
    self.projection_cache.borrow_mut().remove(&admission.run_id);
    Ok(RichApprovalDecisionOutcome {
      status: match terminal {
        Some(("approved", _, _)) => RichApprovalDecisionStatus::Approved,
        Some(_) => RichApprovalDecisionStatus::Rejected,
        None => RichApprovalDecisionStatus::Pending,
      },
      required: review.required,
      received: quorum.received,
      represented_roles: quorum.represented_roles,
    })
  }

  /// Resolves an authenticated provider identity through the operator-owned
  /// directory version pinned by the immutable review, then converges on the
  /// same atomic quorum transaction used by every rich-approval ingress path.
  /// Principal IDs and roles are never accepted from callback input.
  pub fn record_rich_approval_identity_decision(
    &mut self,
    authority: &ReviewerDirectoryAuthorityV1,
    decision: RichApprovalIdentityDecision,
  ) -> Result<RichApprovalDecisionOutcome, DurableStoreError> {
    let review = rich_review_row(&self.connection, &decision.run_id, &decision.request_id)?
      .ok_or(DurableStoreError::RichApprovalNotFound)?;
    if review.review_digest != decision.review_digest {
      return Err(DurableStoreError::RichApprovalBindingMismatch);
    }
    authority.validate_requirements(
      &review.directory_version,
      review.required,
      &review.required_roles,
    )?;
    let reviewer = authority.resolve(
      &review.directory_version,
      &decision.identity,
      &review.required_roles,
    )?;
    self.record_rich_approval_decision(RichApprovalDecisionAdmission {
      run_id: decision.run_id,
      request_id: decision.request_id,
      review_digest: decision.review_digest,
      principal_id: reviewer.principal_id,
      decision: decision.decision,
      provider: reviewer.provider.as_str().to_string(),
      provider_account_id: reviewer.account_id,
      provider_actor_id: reviewer.actor_id,
      roles: reviewer.roles,
      decided_at: decision.decided_at,
    })
  }

  pub fn settle_rich_approval_without_vote(
    &mut self,
    run_id: &str,
    request_id: &str,
    review_digest: &str,
    reason: RichApprovalResolutionReason,
    settled_at: DateTime<Utc>,
  ) -> Result<RichApprovalDecisionOutcome, DurableStoreError> {
    if matches!(
      reason,
      RichApprovalResolutionReason::Quorum | RichApprovalResolutionReason::Rejection
    ) {
      return Err(DurableStoreError::Contract(
        "Human rich approval outcomes require a reviewer decision.".to_string(),
      ));
    }
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let review = rich_review_row(&transaction, run_id, request_id)?
      .ok_or(DurableStoreError::RichApprovalNotFound)?;
    if review.review_digest != review_digest {
      return Err(DurableStoreError::RichApprovalBindingMismatch);
    }
    let quorum = rich_quorum_state(&transaction, &review)?;
    if review.status != "pending" {
      return Ok(RichApprovalDecisionOutcome {
        status: RichApprovalDecisionStatus::AlreadyResolved,
        required: review.required,
        received: quorum.received,
        represented_roles: quorum.represented_roles,
      });
    }
    let status = match reason {
      RichApprovalResolutionReason::Timeout => "expired",
      RichApprovalResolutionReason::Cancellation => "cancelled",
      RichApprovalResolutionReason::ArtifactInvalidated => "invalidated",
      _ => unreachable!(),
    };
    settle_rich_review_row(&transaction, run_id, request_id, status, settled_at)?;
    let mut payloads = Vec::new();
    if reason == RichApprovalResolutionReason::ArtifactInvalidated {
      payloads.push(RunEventPayload::ApprovalInvalidated(
        ApprovalInvalidatedData {
          approval_id: review.approval_id.clone(),
          request_id: request_id.to_string(),
          review_digest: review_digest.to_string(),
          reason,
        },
      ));
    }
    payloads.push(RunEventPayload::ApprovalResolved(ApprovalResolvedData {
      approval_id: review.approval_id.clone(),
      request_id: request_id.to_string(),
      resolution: None,
      review_digest: Some(review_digest.to_string()),
      decision: Some(ApprovalDecision::Rejected),
      reason: Some(reason),
      required: Some(review.required),
      received: Some(quorum.received),
      required_roles: Some(review.required_roles),
      represented_roles: Some(quorum.represented_roles.clone()),
    }));
    if reason == RichApprovalResolutionReason::Timeout {
      let workflow = definition_for_run(&transaction, run_id)?;
      let approval = workflow.approval(&review.approval_id).ok_or_else(|| {
        DurableStoreError::Contract("A rich approval timeout has no compiled descriptor.".into())
      })?;
      if approval.on_timeout == "fail" {
        payloads.push(RunEventPayload::RunFailed(RunFailedData::V4(
          crate::RunFailedDataV4::Approval {
            approval_id: review.approval_id.clone(),
            request_id: request_id.to_string(),
            failure: ApprovalFailure {
              kind: "approval_timeout".to_string(),
              code: "WOML_APPROVAL_TIMEOUT".to_string(),
              message: format!("Approval {:?} reached its deadline.", review.approval_id),
            },
          },
        )));
      }
    }
    append_rich_approval_payloads(
      &transaction,
      &self.projection_cache,
      run_id,
      settled_at,
      payloads,
    )?;
    transaction.commit()?;
    self.projection_cache.borrow_mut().remove(run_id);
    Ok(RichApprovalDecisionOutcome {
      status: RichApprovalDecisionStatus::Rejected,
      required: review.required,
      received: quorum.received,
      represented_roles: quorum.represented_roles,
    })
  }

  pub fn inspect_rich_approval(
    &self,
    run_id: &str,
    request_id: &str,
    decision_limit: usize,
  ) -> Result<RichApprovalInspectionV1, DurableStoreError> {
    let limit = decision_limit.clamp(1, RICH_APPROVAL_INSPECTION_DECISION_LIMIT);
    let row = rich_review_row(&self.connection, run_id, request_id)?
      .ok_or(DurableStoreError::RichApprovalNotFound)?;
    let (review_json, directory_version, created_at, resolved_at): (
      String,
      String,
      String,
      Option<String>,
    ) = self.connection.query_row(
      "SELECT review_json, directory_version, created_at, resolved_at
       FROM woml_approval_reviews WHERE run_id = ?1 AND request_id = ?2",
      params![run_id, request_id],
      |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let total_decisions: u32 = self.connection.query_row(
      "SELECT COUNT(*) FROM woml_approval_reviewer_decisions
       WHERE run_id = ?1 AND request_id = ?2",
      params![run_id, request_id],
      |row| row.get(0),
    )?;
    let mut statement = self.connection.prepare(
      "SELECT principal_id, decision, provider, provider_account_id, roles_json, decided_at
       FROM woml_approval_reviewer_decisions
       WHERE run_id = ?1 AND request_id = ?2
       ORDER BY decided_at, principal_id LIMIT ?3",
    )?;
    let decisions = statement
      .query_map(params![run_id, request_id, limit as u64], |row| {
        let decision: String = row.get(1)?;
        let roles_json: String = row.get(4)?;
        let decided_at: String = row.get(5)?;
        Ok((
          row.get::<_, String>(0)?,
          decision,
          row.get::<_, String>(2)?,
          row.get::<_, String>(3)?,
          roles_json,
          decided_at,
        ))
      })?
      .map(|row| {
        let (principal_id, decision, provider, provider_account_id, roles_json, decided_at) = row?;
        Ok(RichApprovalDecisionInspection {
          principal_id,
          decision: parse_approval_decision(&decision)?,
          provider,
          provider_account_id,
          roles: serde_json::from_str(&roles_json)?,
          decided_at: parse_timestamp(&decided_at)?,
        })
      })
      .collect::<Result<Vec<_>, DurableStoreError>>()?;
    Ok(RichApprovalInspectionV1 {
      run_id: run_id.to_string(),
      approval_id: row.approval_id,
      request_id: request_id.to_string(),
      review_digest: row.review_digest,
      review: serde_json::from_str(&review_json)?,
      required: row.required,
      required_roles: row.required_roles,
      directory_version,
      status: row.status,
      created_at: parse_timestamp(&created_at)?,
      resolved_at: resolved_at.as_deref().map(parse_timestamp).transpose()?,
      decisions_truncated: total_decisions as usize > decisions.len(),
      total_decisions,
      decisions,
    })
  }

  pub(crate) fn append_payloads_atomically(
    &mut self,
    run_id: &str,
    payloads: Vec<(String, DateTime<Utc>, RunEventPayload)>,
  ) -> Result<RunProjection, DurableStoreError> {
    self.append_payloads_scoped_atomically(run_id, None, payloads)
  }

  pub(crate) fn append_payloads_scoped_atomically(
    &mut self,
    run_id: &str,
    iteration: Option<crate::event::ForEachIterationScope>,
    payloads: Vec<(String, DateTime<Utc>, RunEventPayload)>,
  ) -> Result<RunProjection, DurableStoreError> {
    let mut span = crate::performance::PerformanceSpan::new("sqlite", "sqlite.append_event_batch");
    span.run_id(run_id.to_string());
    span.count("events", payloads.len());
    span.count("transactions", 1);
    if payloads.is_empty() {
      return Err(DurableStoreError::Contract(
        "An atomic event batch must not be empty.".to_string(),
      ));
    }
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let mut events = load_events(&transaction, run_id)?;
    let mut projection_state =
      projection_state_for_events(&self.projection_cache, run_id, &events)?;
    let event_schema_version = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "An atomic event batch cannot be appended before run_started.".to_string(),
        )
      })?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    for (event_id, occurred_at, payload) in payloads {
      let translated = if matches!(
        event_schema_version,
        crate::RUN_EVENT_SCHEMA_VERSION_V10
          | crate::RUN_EVENT_SCHEMA_VERSION_V11
          | crate::RUN_EVENT_SCHEMA_VERSION_V12
          | crate::RUN_EVENT_SCHEMA_VERSION_V13
          | crate::RUN_EVENT_SCHEMA_VERSION_V14
          | crate::RUN_EVENT_SCHEMA_VERSION_V15
          | crate::RUN_EVENT_SCHEMA_VERSION_V16
          | crate::RUN_EVENT_SCHEMA_VERSION_V17
          | crate::RUN_EVENT_SCHEMA_VERSION_V18
          | crate::RUN_EVENT_SCHEMA_VERSION_V19
          | crate::RUN_EVENT_SCHEMA_VERSION_V20
      ) {
        expand_model_v11_payload(&workflow, run_id, payload)?
      } else {
        vec![payload]
      };
      for (index, mut payload) in translated.into_iter().enumerate() {
        if let RunEventPayload::CompensationRegistered(registration) = &mut payload {
          if let Some(previous) = events.last() {
            if matches!(
              &previous.payload,
              RunEventPayload::StepAttemptSucceeded(succeeded)
                if succeeded.node_id == registration.for_step_id
            ) {
              // The store owns global sequencing. Concurrent loop iterations
              // cannot safely predict their success sequence before this
              // immediate transaction acquires the writer lock.
              registration.completion_sequence = previous.sequence;
            }
          }
        }
        if let Some(scope) = iteration.as_ref() {
          validate_scoped_payload_against_definition(
            &workflow,
            &binding.definition_hash,
            run_id,
            scope,
            &payload,
          )
          .map_err(DurableStoreError::Contract)?;
        } else {
          validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
            .map_err(DurableStoreError::Contract)?;
        }
        append_to_history_scoped_incremental(
          &transaction,
          &mut events,
          &mut projection_state,
          run_id,
          if index == 0 {
            event_id.clone()
          } else {
            generated_event_id()
          },
          occurred_at,
          event_schema_version,
          iteration.clone(),
          payload,
          false,
        )?;
      }
    }
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = projection_state.projection().clone();
    let last_occurred_at = events
      .last()
      .expect("an atomic event batch preserves a non-empty history")
      .occurred_at;
    write_run_summary_from_projection(
      &transaction,
      events
        .first()
        .expect("an atomic event batch preserves a non-empty history"),
      last_occurred_at,
      &projection,
    )?;
    if let RunEventPayload::RunAdmitted(admission) = &events
      .first()
      .expect("an atomic event batch preserves a non-empty history")
      .payload
    {
      write_runtime_policy_indexes_from_projection(&transaction, run_id, admission, &projection)?;
    }
    if matches!(
      event_schema_version,
      crate::RUN_EVENT_SCHEMA_VERSION_V18
        | crate::RUN_EVENT_SCHEMA_VERSION_V19
        | crate::RUN_EVENT_SCHEMA_VERSION_V20
    ) {
      write_correction_indexes_from_projection(
        &transaction,
        run_id,
        &binding.definition_hash,
        &events,
        &projection,
      )?;
    }
    transaction.commit()?;
    cache_projection_state(&self.projection_cache, run_id, projection_state);
    span.succeed();
    Ok(projection)
  }

  pub fn events(&self, run_id: &str) -> Result<Vec<RunEvent>, DurableStoreError> {
    self.run_binding(run_id)?;
    load_events(&self.connection, run_id)
  }

  /// Count a run's durable events without allocating or decoding its history.
  /// Read models use this to reject pathological histories before projection.
  pub fn event_count(&self, run_id: &str) -> Result<usize, DurableStoreError> {
    self.run_binding(run_id)?;
    let mut statement = self
      .connection
      .prepare_cached("SELECT COUNT(*) FROM woml_run_events WHERE run_id = ?1")?;
    let count: i64 = statement.query_row([run_id], |row| row.get(0))?;
    usize::try_from(count)
      .map_err(|_| DurableStoreError::Contract("run event count is invalid".to_string()))
  }

  pub fn projection(&self, run_id: &str) -> Result<RunProjection, DurableStoreError> {
    let mut span = crate::performance::PerformanceSpan::new("sqlite", "sqlite.project_run");
    span.run_id(run_id.to_string());
    let binding = self.run_binding(run_id)?;
    let mut statement = self.connection.prepare_cached(
      "SELECT COUNT(*), (
         SELECT event_id FROM woml_run_events
         WHERE run_id = ?1 ORDER BY sequence DESC LIMIT 1
       )
       FROM woml_run_events WHERE run_id = ?1",
    )?;
    let (event_count, last_event_id): (i64, Option<String>) =
      statement.query_row([run_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
    let event_count = u64::try_from(event_count)
      .map_err(|_| DurableStoreError::Contract("run event count is invalid".to_string()))?;
    span.count("events", event_count as usize);
    if let Some(cached) = self.projection_cache.borrow().get(run_id) {
      if cached.projection().last_sequence == event_count
        && cached.last_event_id() == last_event_id.as_deref()
      {
        span.count("projection_cache_hits", 1);
        let mut projection = cached.projection().clone();
        if projection
          .event_schema_version
          .is_some_and(|version| version >= RUN_EVENT_SCHEMA_VERSION_V11)
          && projection.workflow_id.is_none()
        {
          projection.workflow_id = Some(binding.workflow_id);
        }
        hydrate_rich_approval_public_outputs(&self.connection, run_id, &mut projection)?;
        span.succeed();
        return Ok(projection);
      }
    }
    span.count("projection_cache_misses", 1);
    let events = load_events(&self.connection, run_id)?;
    let workflow = self.definition(&binding.definition_hash)?;
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let state = ProjectionFoldState::from_events(&events)?;
    let mut projection = state.projection().clone();
    if projection
      .event_schema_version
      .is_some_and(|version| version >= RUN_EVENT_SCHEMA_VERSION_V11)
      && projection.workflow_id.is_none()
    {
      projection.workflow_id = Some(binding.workflow_id);
    }
    hydrate_rich_approval_public_outputs(&self.connection, run_id, &mut projection)?;
    cache_projection_state(&self.projection_cache, run_id, state);
    span.succeed();
    Ok(projection)
  }

  pub fn rebuild_run_summaries(&mut self) -> Result<(), DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    rebuild_run_summary_index(&transaction)?;
    transaction.commit()?;
    Ok(())
  }

  pub fn rebuild_runtime_policy_indexes(&mut self) -> Result<(), DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    rebuild_runtime_policy_indexes(&transaction)?;
    rebuild_run_summary_index(&transaction)?;
    transaction.commit()?;
    Ok(())
  }

  pub fn list_runs(&self, limit: usize) -> Result<RunListV1, DurableStoreError> {
    self.list_runs_filtered(limit, None, None)
  }

  pub fn list_runs_filtered(
    &self,
    limit: usize,
    workflow_id: Option<&str>,
    status: Option<&str>,
  ) -> Result<RunListV1, DurableStoreError> {
    if !(1..=200).contains(&limit) {
      return Err(DurableStoreError::Contract(
        "Run list limit must be between 1 and 200.".to_string(),
      ));
    }
    if workflow_id.is_some_and(str::is_empty) {
      return Err(DurableStoreError::Contract(
        "Run list workflow filter must not be empty.".to_string(),
      ));
    }
    let status = status
      .map(|value| {
        PublicRunStatus::parse(value).ok_or_else(|| {
          DurableStoreError::Contract(format!("Run list status filter {value:?} is invalid."))
        })
      })
      .transpose()?;
    let mut statement = self.connection.prepare(
      "SELECT run_id, workflow_id, status, started_at, updated_at
       FROM woml_run_summaries
       WHERE (?2 IS NULL OR workflow_id = ?2)
         AND (?3 IS NULL OR status = ?3)
         AND started_at IS NOT NULL
         AND status != 'queued'
       ORDER BY updated_at DESC, run_id DESC
       LIMIT ?1",
    )?;
    let rows = statement
      .query_map(
        rusqlite::params![
          i64::try_from(limit).unwrap_or(200),
          workflow_id,
          status.map(PublicRunStatus::as_str),
        ],
        |row| {
          Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
          ))
        },
      )?
      .collect::<Result<Vec<_>, _>>()?;
    let mut runs = Vec::with_capacity(rows.len());
    for (run_id, workflow_id, status, started_at, updated_at) in rows {
      runs.push(RunSummaryV1 {
        run_id,
        workflow_id,
        status: PublicRunStatus::parse(&status).ok_or_else(|| {
          DurableStoreError::Contract("Stored run summary status is invalid.".to_string())
        })?,
        started_at: parse_stored_timestamp(&started_at)?,
        updated_at: parse_stored_timestamp(&updated_at)?,
      });
    }
    Ok(RunListV1 {
      profile: "woml.run-list/v1",
      runs,
    })
  }

  pub fn list_runs_v2(&self, limit: usize) -> Result<RunListV2, DurableStoreError> {
    self.list_runs_v2_filtered(limit, None, None)
  }

  pub fn list_runs_v2_filtered(
    &self,
    limit: usize,
    workflow_id: Option<&str>,
    status: Option<&str>,
  ) -> Result<RunListV2, DurableStoreError> {
    if !(1..=200).contains(&limit) {
      return Err(DurableStoreError::Contract(
        "Run list limit must be between 1 and 200.".to_string(),
      ));
    }
    if workflow_id.is_some_and(str::is_empty) {
      return Err(DurableStoreError::Contract(
        "Run list workflow filter must not be empty.".to_string(),
      ));
    }
    let status = status
      .map(|value| {
        PublicRunStatus::parse(value).ok_or_else(|| {
          DurableStoreError::Contract(format!("Run list status filter {value:?} is invalid."))
        })
      })
      .transpose()?;
    let mut statement = self.connection.prepare(
      "SELECT run_id, workflow_id, status, admitted_at, started_at, updated_at,
              queue_name, waiting_for, eligible_at
       FROM woml_run_summaries
       WHERE (?2 IS NULL OR workflow_id = ?2)
         AND (?3 IS NULL OR status = ?3)
       ORDER BY updated_at DESC, run_id DESC
       LIMIT ?1",
    )?;
    let rows = statement
      .query_map(
        rusqlite::params![
          i64::try_from(limit).unwrap_or(200),
          workflow_id,
          status.map(PublicRunStatus::as_str),
        ],
        |row| {
          Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, Option<String>>(7)?,
            row.get::<_, Option<String>>(8)?,
          ))
        },
      )?
      .collect::<Result<Vec<_>, _>>()?;
    let mut runs = Vec::with_capacity(rows.len());
    for row in rows {
      runs.push(RunSummaryV2 {
        run_id: row.0,
        workflow_id: row.1,
        status: PublicRunStatus::parse(&row.2).ok_or_else(|| {
          DurableStoreError::Contract("Stored run summary status is invalid.".to_string())
        })?,
        admitted_at: parse_stored_timestamp(&row.3)?,
        started_at: row.4.as_deref().map(parse_stored_timestamp).transpose()?,
        updated_at: parse_stored_timestamp(&row.5)?,
        queue: row.6,
        waiting_for: row
          .7
          .as_deref()
          .map(|value| {
            PolicyWaitingFor::parse(value).ok_or_else(|| {
              DurableStoreError::Contract("Stored policy wait reason is invalid.".to_string())
            })
          })
          .transpose()?,
        eligible_at: row.8.as_deref().map(parse_stored_timestamp).transpose()?,
      });
    }
    Ok(RunListV2 {
      profile: "woml.run-list/v2",
      runs,
    })
  }

  /// Returns a compact, payload-free projection for Runtime Operations v1.
  /// Counts come from durable coordination/event facts and therefore survive
  /// process replacement without making telemetry authoritative.
  pub fn runtime_observation_v1(&self) -> Result<RuntimeObservationV1, DurableStoreError> {
    let mut status_totals = BTreeMap::new();
    let mut statement = self
      .connection
      .prepare("SELECT status, COUNT(*) FROM woml_run_summaries GROUP BY status ORDER BY status")?;
    let rows = statement
      .query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
      })?
      .collect::<Result<Vec<_>, _>>()?;
    for (status, count) in rows {
      if PublicRunStatus::parse(&status).is_none() {
        return Err(DurableStoreError::Contract(
          "Stored runtime observation status is invalid.".to_string(),
        ));
      }
      status_totals.insert(status, count);
    }

    let latest_event_run_ids = |event_type: &str| -> Result<Vec<String>, DurableStoreError> {
      let pattern = format!("%\"type\":\"{event_type}\"%");
      let mut statement = self.connection.prepare(
        "SELECT summaries.run_id
         FROM woml_run_summaries AS summaries
         JOIN woml_run_events AS events
           ON events.run_id = summaries.run_id
          AND events.sequence = (
            SELECT MAX(latest.sequence)
            FROM woml_run_events AS latest
            WHERE latest.run_id = summaries.run_id
          )
         WHERE events.event_json LIKE ?1
         ORDER BY summaries.updated_at DESC, summaries.run_id DESC
         LIMIT 1000",
      )?;
      let rows = statement
        .query_map([pattern], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
      Ok(rows)
    };
    let retrying_run_ids = latest_event_run_ids("step_retry_scheduled")?;
    let mut approval_statement = self.connection.prepare(
      "SELECT summaries.run_id
       FROM woml_run_summaries AS summaries
       WHERE summaries.status = 'waiting'
         AND EXISTS (
           SELECT 1 FROM woml_run_events AS requested
           WHERE requested.run_id = summaries.run_id
             AND requested.event_json LIKE '%\"type\":\"approval_requested\"%'
         )
         AND NOT EXISTS (
           SELECT 1 FROM woml_run_events AS resolved
           WHERE resolved.run_id = summaries.run_id
             AND resolved.event_json LIKE '%\"type\":\"approval_resolved\"%'
         )
       ORDER BY summaries.updated_at DESC, summaries.run_id DESC
       LIMIT 1000",
    )?;
    let approval_waiting_run_ids = approval_statement
      .query_map([], |row| row.get::<_, String>(0))?
      .collect::<Result<Vec<_>, _>>()?;
    let count = |sql: &str| -> Result<u64, DurableStoreError> {
      Ok(self.connection.query_row(sql, [], |row| row.get(0))?)
    };

    Ok(RuntimeObservationV1 {
      profile: "woml.runtime-observation/v1",
      status_totals,
      retrying_run_ids,
      approval_waiting_run_ids,
      retries_total: count(
        "SELECT COUNT(*) FROM woml_run_events
         WHERE event_json LIKE '%\"type\":\"step_retry_scheduled\"%'",
      )?,
      triggers_total: count("SELECT COUNT(*) FROM woml_trigger_occurrences")?,
      workflow_calls_active: count(
        "SELECT COUNT(*) FROM woml_workflow_calls WHERE state IN ('admitted', 'running')",
      )?,
    })
  }

  pub fn inspect_run_v2(&self, run_id: &str) -> Result<RunInspectionV2, DurableStoreError> {
    let projection = self.projection(run_id)?;
    let run_id = projection.run_id.clone().ok_or_else(|| {
      DurableStoreError::Contract("Run inspection requires run_started.".to_string())
    })?;
    let workflow_id = projection.workflow_id.clone().ok_or_else(|| {
      DurableStoreError::Contract("Run inspection requires a workflow identity.".to_string())
    })?;
    let hooks = projection
      .lifecycle_hooks
      .values()
      .map(|hook| RunInspectionHookV2 {
        hook_id: hook.hook_id.clone(),
        subject_kind: hook.subject.kind,
        subject_id: hook.subject.id.clone(),
        status: hook.status,
        failed_actions: hook.failed_actions,
      })
      .collect();
    Ok(RunInspectionV2 {
      profile: "woml.run-inspection/v2",
      run_id,
      workflow_id,
      status: projection.status.into(),
      business_outcome: projection.business_outcome.into(),
      lifecycle_status: projection.lifecycle_status,
      hooks,
      warnings: projection.lifecycle_warnings,
      cancellation: RunInspectionCancellationV2 {
        requested: projection.cancellation_request_id.is_some(),
        request_id: projection.cancellation_request_id,
      },
    })
  }

  pub fn inspect_run_v3(&self, run_id: &str) -> Result<RunInspectionV3, DurableStoreError> {
    let binding = self.run_binding(run_id)?;
    let projection = self.projection(run_id)?;
    let run_id = projection.run_id.clone().ok_or_else(|| {
      DurableStoreError::Contract("Run inspection requires an admission event.".to_string())
    })?;
    let queue = projection.queue.clone().ok_or_else(|| {
      DurableStoreError::Contract(
        "Run Inspection v3 is available only for Event v11 policy runs.".to_string(),
      )
    })?;
    let (waiting_for, eligible_at): (Option<String>, Option<String>) = self.connection.query_row(
      "SELECT waiting_for, eligible_at FROM woml_run_summaries WHERE run_id = ?1",
      [&run_id],
      |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let hooks = projection
      .lifecycle_hooks
      .values()
      .map(|hook| RunInspectionHookV2 {
        hook_id: hook.hook_id.clone(),
        subject_kind: hook.subject.kind,
        subject_id: hook.subject.id.clone(),
        status: hook.status,
        failed_actions: hook.failed_actions,
      })
      .collect();
    Ok(RunInspectionV3 {
      profile: "woml.run-inspection/v3",
      run_id,
      workflow_id: binding.workflow_id,
      status: projection.status.into(),
      business_outcome: projection.business_outcome.into(),
      lifecycle_status: projection.lifecycle_status,
      hooks,
      warnings: projection.lifecycle_warnings,
      cancellation: RunInspectionCancellationV2 {
        requested: projection.cancellation_request_id.is_some(),
        request_id: projection.cancellation_request_id,
      },
      policy: RunInspectionPolicyV3 {
        queue,
        waiting_for: waiting_for
          .as_deref()
          .map(|value| {
            PolicyWaitingFor::parse(value).ok_or_else(|| {
              DurableStoreError::Contract("Stored policy wait reason is invalid.".to_string())
            })
          })
          .transpose()?,
        eligible_at: eligible_at
          .as_deref()
          .map(parse_stored_timestamp)
          .transpose()?,
        timeout_at: projection.timeout_at,
      },
    })
  }

  pub fn inspect_run_v4(&self, run_id: &str) -> Result<RunInspectionV4, DurableStoreError> {
    let base = self.inspect_run_v3(run_id)?;
    let binding = self.run_binding(run_id)?;
    let workflow = self.definition(&binding.definition_hash)?;
    if workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V13 {
      return Err(DurableStoreError::Contract(
        "Run Inspection v4 is available only for compiled Model v13 runs.".to_string(),
      ));
    }
    let projection = self.projection(run_id)?;
    let mut items = Vec::new();
    for fork in workflow.graph.forks.as_deref().unwrap_or_default() {
      let Some(projected) = projection.forks.get(&fork.fork_id) else {
        continue;
      };
      let mut branches = RunInspectionForkCountsV4 {
        opened: u32::try_from(fork.branches.len()).unwrap_or(u32::MAX),
        active: 0,
        waiting: 0,
        succeeded: 0,
        failed: 0,
        cancelled: 0,
      };
      for branch in &fork.branches {
        match projected
          .branches
          .get(&branch.branch_id)
          .and_then(|branch| branch.outcome)
        {
          None => branches.active += 1,
          Some(crate::event::ForkBranchOutcome::Succeeded) => branches.succeeded += 1,
          Some(crate::event::ForkBranchOutcome::Failed) => branches.failed += 1,
          Some(crate::event::ForkBranchOutcome::Cancelled) => branches.cancelled += 1,
        }
      }
      let status = if projected.join_status == crate::projection::ForkJoinStatus::Failed
        || branches.failed > 0
      {
        InspectedForkStatusV4::Failed
      } else if projected.join_status == crate::projection::ForkJoinStatus::Cancelled
        || branches.cancelled > 0
      {
        InspectedForkStatusV4::Cancelled
      } else if branches.active > 0 {
        InspectedForkStatusV4::Active
      } else if projected.join_status == crate::projection::ForkJoinStatus::Pending {
        InspectedForkStatusV4::Waiting
      } else {
        InspectedForkStatusV4::Succeeded
      };
      let mode = if fork.joined_branch_ids.is_empty() {
        InspectedForkJoinModeV4::None
      } else if fork.joined_branch_ids.len() == fork.branches.len() {
        InspectedForkJoinModeV4::All
      } else {
        InspectedForkJoinModeV4::Selected
      };
      items.push(RunInspectionForkV4 {
        fork_id: fork.fork_id.clone(),
        status,
        join: RunInspectionForkJoinV4 {
          mode,
          branch_ids: fork.joined_branch_ids.clone(),
          status: projected.join_status,
        },
        branches,
      });
    }
    let mut counts = RunInspectionForkCountsV4 {
      opened: u32::try_from(items.len()).unwrap_or(u32::MAX),
      active: 0,
      waiting: 0,
      succeeded: 0,
      failed: 0,
      cancelled: 0,
    };
    for fork in &items {
      match fork.status {
        InspectedForkStatusV4::Active => counts.active += 1,
        InspectedForkStatusV4::Waiting => counts.waiting += 1,
        InspectedForkStatusV4::Succeeded => counts.succeeded += 1,
        InspectedForkStatusV4::Failed => counts.failed += 1,
        InspectedForkStatusV4::Cancelled => counts.cancelled += 1,
      }
    }
    Ok(RunInspectionV4 {
      profile: "woml.run-inspection/v4",
      run_id: base.run_id,
      workflow_id: base.workflow_id,
      status: base.status,
      business_outcome: base.business_outcome,
      lifecycle_status: base.lifecycle_status,
      hooks: base.hooks,
      warnings: base.warnings,
      cancellation: base.cancellation,
      policy: base.policy,
      forks: RunInspectionForksV4 { counts, items },
    })
  }

  pub fn inspect_run_v5(&self, run_id: &str) -> Result<RunInspectionV5, DurableStoreError> {
    let base = self.inspect_run_v4(run_id)?;
    let binding = self.run_binding(run_id)?;
    let workflow = self.definition(&binding.definition_hash)?;
    if workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V14 {
      return Err(DurableStoreError::Contract(
        "Run Inspection v5 is available only for compiled Model v14 or later runs.".to_string(),
      ));
    }
    let projection = self.projection(run_id)?;
    let mut items = Vec::new();
    for definition in workflow.reusable_definitions.iter().flatten() {
      let (invocation_id, alias, definition_digest, kind, lifecycle, status) = match definition {
        crate::model::CompiledReusableInvocation::Step {
          invocation_id,
          node_id,
          alias,
          definition_digest,
          lifecycle,
          ..
        } => {
          let status = match projection.latest_attempt(node_id) {
            None => InspectedReusableStatusV5::Pending,
            Some(attempt) => match &attempt.status {
              AttemptStatus::Started => InspectedReusableStatusV5::Running,
              AttemptStatus::Succeeded { .. } => InspectedReusableStatusV5::Succeeded,
              AttemptStatus::Failed { failure }
                if failure.kind == AttemptFailureKind::InvocationCancelled =>
              {
                InspectedReusableStatusV5::Cancelled
              }
              AttemptStatus::Failed { .. } => InspectedReusableStatusV5::Failed,
            },
          };
          (
            invocation_id,
            alias,
            definition_digest,
            InspectedReusableKindV5::Step,
            lifecycle,
            status,
          )
        }
        crate::model::CompiledReusableInvocation::NotificationProvider {
          provider_id,
          alias,
          definition_digest,
          lifecycle,
          ..
        } => {
          let status = match projection.notification_deliveries.get(provider_id) {
            None => InspectedReusableStatusV5::Pending,
            Some(delivery) => match &delivery.status {
              crate::projection::NotificationDeliveryStatus::Requested
              | crate::projection::NotificationDeliveryStatus::AttemptStarted { .. } => {
                InspectedReusableStatusV5::Running
              }
              crate::projection::NotificationDeliveryStatus::Succeeded { .. } => {
                InspectedReusableStatusV5::Succeeded
              }
              crate::projection::NotificationDeliveryStatus::Failed { failure, .. }
                if failure.kind == "cancelled" =>
              {
                InspectedReusableStatusV5::Cancelled
              }
              crate::projection::NotificationDeliveryStatus::Failed { .. } => {
                InspectedReusableStatusV5::Failed
              }
            },
          };
          (
            provider_id,
            alias,
            definition_digest,
            InspectedReusableKindV5::NotificationProvider,
            lifecycle,
            status,
          )
        }
      };
      let lifecycle_hooks = projection
        .reusable_lifecycle_hooks
        .values()
        .filter(|hook| hook.invocation_id == *invocation_id)
        .collect::<Vec<_>>();
      let lifecycle_status = if lifecycle.is_none() || lifecycle_hooks.is_empty() {
        InspectedReusableLifecycleStatusV5::Idle
      } else if lifecycle_hooks.iter().any(|hook| {
        hook.status == crate::projection::ReusableLifecycleStatus::CompletedWithWarnings
      }) {
        InspectedReusableLifecycleStatusV5::CompletedWithWarnings
      } else if lifecycle_hooks
        .iter()
        .all(|hook| hook.status == crate::projection::ReusableLifecycleStatus::Completed)
      {
        InspectedReusableLifecycleStatusV5::Completed
      } else {
        InspectedReusableLifecycleStatusV5::Running
      };
      items.push(RunInspectionReusableItemV5 {
        invocation_id: invocation_id.clone(),
        alias: alias.clone(),
        definition_digest_prefix: definition_digest
          .trim_start_matches("sha256:")
          .chars()
          .take(12)
          .collect(),
        kind,
        status,
        lifecycle_status,
      });
    }
    items.sort_by(|left, right| left.invocation_id.cmp(&right.invocation_id));
    let mut counts = RunInspectionReusableCountsV5::default();
    for item in &items {
      match item.status {
        InspectedReusableStatusV5::Pending => counts.pending += 1,
        InspectedReusableStatusV5::Running => counts.running += 1,
        InspectedReusableStatusV5::Succeeded => counts.succeeded += 1,
        InspectedReusableStatusV5::Failed => counts.failed += 1,
        InspectedReusableStatusV5::Cancelled => counts.cancelled += 1,
      }
      if item.lifecycle_status == InspectedReusableLifecycleStatusV5::CompletedWithWarnings {
        counts.completed_with_warnings += 1;
      }
    }
    Ok(RunInspectionV5 {
      profile: "woml.run-inspection/v5",
      run_id: base.run_id,
      workflow_id: base.workflow_id,
      status: base.status,
      business_outcome: base.business_outcome,
      lifecycle_status: base.lifecycle_status,
      hooks: base.hooks,
      warnings: base.warnings,
      cancellation: base.cancellation,
      policy: base.policy,
      forks: base.forks,
      reusable_definitions: RunInspectionReusableDefinitionsV5 { counts, items },
    })
  }

  pub fn inspect_run_v6(&self, run_id: &str) -> Result<RunInspectionV6, DurableStoreError> {
    let base = self.inspect_run_v5(run_id)?;
    let binding = self.run_binding(run_id)?;
    let workflow = self.definition(&binding.definition_hash)?;
    if workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V16 {
      return Err(DurableStoreError::Contract(
        "Run Inspection v6 is available only for compiled Model v16 or later runs.".to_string(),
      ));
    }
    let projection = self.projection(run_id)?;
    let mut items = Vec::new();
    for descriptor in workflow.graph.for_each.as_deref().unwrap_or_default() {
      let Some(state) = projection.for_each.get(&descriptor.for_each_id) else {
        continue;
      };
      let succeeded = state
        .iterations
        .values()
        .filter(|status| matches!(status, ForEachIterationStatus::Succeeded { .. }))
        .count() as u32;
      let failed = state
        .iterations
        .values()
        .filter(|status| matches!(status, ForEachIterationStatus::Failed { .. }))
        .count() as u32;
      let skipped = state
        .iterations
        .values()
        .filter(|status| matches!(status, ForEachIterationStatus::Skipped))
        .count() as u32;
      let active = state
        .iterations
        .values()
        .filter(|status| matches!(status, ForEachIterationStatus::Started))
        .count() as u32;
      let pending = state
        .total
        .saturating_sub(succeeded + failed + skipped + active);
      let (failed_index, failed_node_id) = state
        .iterations
        .iter()
        .find_map(|(index, status)| match status {
          ForEachIterationStatus::Failed { failed_node_id } => {
            Some((Some(*index), Some(failed_node_id.clone())))
          }
          _ => None,
        })
        .unwrap_or((None, None));
      items.push(RunInspectionForEachItemV6 {
        for_each_id: descriptor.for_each_id.clone(),
        status: match state.status {
          ForEachStatus::Open => InspectedForEachStatusV6::Active,
          ForEachStatus::Succeeded => InspectedForEachStatusV6::Succeeded,
          ForEachStatus::Failed => InspectedForEachStatusV6::Failed,
          ForEachStatus::Cancelled => InspectedForEachStatusV6::Cancelled,
        },
        total: state.total,
        succeeded,
        failed,
        skipped,
        active,
        pending,
        concurrency: state.concurrency,
        failed_index,
        failed_node_id,
      });
    }
    let mut counts = RunInspectionForEachCountsV6 {
      opened: u32::try_from(items.len()).unwrap_or(u32::MAX),
      ..RunInspectionForEachCountsV6::default()
    };
    for item in &items {
      match item.status {
        InspectedForEachStatusV6::Active => counts.active += 1,
        InspectedForEachStatusV6::Succeeded => counts.succeeded += 1,
        InspectedForEachStatusV6::Failed => counts.failed += 1,
        InspectedForEachStatusV6::Cancelled => counts.cancelled += 1,
      }
    }
    Ok(RunInspectionV6 {
      profile: "woml.run-inspection/v6",
      run_id: base.run_id,
      workflow_id: base.workflow_id,
      status: base.status,
      business_outcome: base.business_outcome,
      lifecycle_status: base.lifecycle_status,
      hooks: base.hooks,
      warnings: base.warnings,
      cancellation: base.cancellation,
      policy: base.policy,
      forks: base.forks,
      reusable_definitions: base.reusable_definitions,
      for_each: RunInspectionForEachV6 { counts, items },
    })
  }

  pub fn inspect_run_v11(&self, run_id: &str) -> Result<RunInspectionV11, DurableStoreError> {
    let base = self.inspect_run_v6(run_id)?;
    let binding = self.run_binding(run_id)?;
    let workflow = self.definition(&binding.definition_hash)?;
    if workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V20 {
      return Err(DurableStoreError::Contract(
        "Run Inspection v11 is available only for compiled Model v20 or later runs.".to_string(),
      ));
    }
    let projection = self.projection(run_id)?;
    let total = projection.rich_approval_requests.len();
    let mut items = Vec::new();
    for request in projection
      .rich_approval_requests
      .values()
      .take(RUN_INSPECTION_RICH_APPROVAL_LIMIT)
    {
      let inspection = self.inspect_rich_approval(
        run_id,
        &request.request_id,
        RICH_APPROVAL_INSPECTION_DECISION_LIMIT,
      )?;
      let approval_deadline = workflow
        .approval(&request.approval_id)
        .and_then(|approval| approval.timeout_ms)
        .and_then(|milliseconds| i64::try_from(milliseconds).ok())
        .and_then(|milliseconds| {
          request
            .requested_at
            .checked_add_signed(chrono::Duration::milliseconds(milliseconds))
        });
      let deadline = match (approval_deadline, projection.timeout_at) {
        (Some(approval), Some(workflow)) => Some(approval.min(workflow)),
        (Some(approval), None) => Some(approval),
        (None, Some(workflow)) => Some(workflow),
        (None, None) => None,
      };
      let received = request
        .decisions
        .values()
        .filter(|decision| decision.decision == ApprovalDecision::Approved)
        .count() as u32;
      items.push(RunInspectionRichApprovalV11 {
        approval_id: inspection.approval_id,
        request_id: inspection.request_id,
        status: inspection.status,
        review_digest: inspection.review_digest,
        review: inspection.review,
        required: inspection.required,
        received,
        required_roles: inspection.required_roles,
        represented_roles: request.represented_roles.clone(),
        directory_version: inspection.directory_version,
        requested_at: request.requested_at,
        deadline,
        resolved_at: inspection.resolved_at,
        decisions: inspection.decisions,
        total_decisions: inspection.total_decisions,
        decisions_truncated: inspection.decisions_truncated,
      });
    }
    let pending = items
      .iter()
      .filter(|approval| approval.status == "pending")
      .count() as u32;
    Ok(RunInspectionV11 {
      profile: "woml.run-inspection/v11",
      run_id: base.run_id,
      workflow_id: base.workflow_id,
      status: base.status,
      business_outcome: base.business_outcome,
      lifecycle_status: base.lifecycle_status,
      hooks: base.hooks,
      warnings: base.warnings,
      cancellation: base.cancellation,
      policy: base.policy,
      forks: base.forks,
      reusable_definitions: base.reusable_definitions,
      for_each: base.for_each,
      approvals: RunInspectionRichApprovalsV11 {
        pending,
        items,
        truncated: total > RUN_INSPECTION_RICH_APPROVAL_LIMIT,
      },
    })
  }

  pub fn inspect_waits_v1(&self, run_id: &str) -> Result<WaitInspectionV1, DurableStoreError> {
    let binding = self.run_binding(run_id)?;
    let workflow = self.definition(&binding.definition_hash)?;
    let projection = self.projection(run_id)?;
    let now = Utc::now();
    let mut waits = projection
      .durable_waits
      .values()
      .map(|wait| {
        let winning_signal = match wait.status {
          crate::projection::DurableWaitStatus::Satisfied {
            winner: crate::DurableWaitWinner::Signal,
          } => projection
            .external_signals
            .iter()
            .rev()
            .find(|signal| signal.wait_id == wait.wait_id)
            .map(|signal| signal.signal.clone()),
          _ => None,
        };
        let status = match wait.status {
          crate::projection::DurableWaitStatus::Registered => InspectedWaitStatusV1::Pending,
          crate::projection::DurableWaitStatus::Satisfied {
            winner: crate::DurableWaitWinner::Signal,
          } => InspectedWaitStatusV1::Signaled,
          crate::projection::DurableWaitStatus::Satisfied {
            winner: crate::DurableWaitWinner::Timer,
          } => InspectedWaitStatusV1::Elapsed,
          crate::projection::DurableWaitStatus::TimedOut { .. } => InspectedWaitStatusV1::TimedOut,
          crate::projection::DurableWaitStatus::Cancelled => InspectedWaitStatusV1::Cancelled,
        };
        let name = workflow
          .node(&wait.node_id)
          .and_then(|node| node.metadata.as_ref())
          .and_then(|metadata| metadata.get("name"))
          .and_then(Value::as_str)
          .unwrap_or(&wait.wait_id)
          .to_string();
        InspectedWaitV1 {
          run_id: run_id.to_string(),
          wait_id: wait.wait_id.clone(),
          name,
          kind: wait.kind,
          status,
          registered_at: wait.registered_at,
          signals: wait.signals.clone(),
          correlation_digest: wait.correlation_digest.clone(),
          deadline: wait.deadline,
          resolved_at: wait.resolved_at,
          winning_signal,
        }
      })
      .collect::<Vec<_>>();
    waits.sort_by(|left, right| {
      left
        .registered_at
        .cmp(&right.registered_at)
        .then_with(|| left.wait_id.cmp(&right.wait_id))
    });
    let pending = waits
      .iter()
      .filter(|wait| wait.status == InspectedWaitStatusV1::Pending)
      .collect::<Vec<_>>();
    let oldest_pending_wait_age_ms = pending
      .iter()
      .map(|wait| now.signed_duration_since(wait.registered_at))
      .filter_map(|age| u64::try_from(age.num_milliseconds().max(0)).ok())
      .max();
    Ok(WaitInspectionV1 {
      profile: "woml.wait-inspection/v1",
      run_id: run_id.to_string(),
      pending_waits: u32::try_from(pending.len()).unwrap_or(u32::MAX),
      oldest_pending_wait_age_ms,
      waits,
    })
  }

  /// Builds the bounded public correction view exclusively from immutable run
  /// events. Store v17 tables remain disposable acceleration indexes.
  pub fn inspect_correction_v1(
    &self,
    run_id: &str,
  ) -> Result<CorrectionInspectionV1, DurableStoreError> {
    let binding = self.run_binding(run_id)?;
    let workflow = self.definition(&binding.definition_hash)?;
    if workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V19 {
      return Err(DurableStoreError::Contract(
        "Correction Inspection v1 is available only for compiled Model v19 or later runs."
          .to_string(),
      ));
    }
    let projection = self.projection(run_id)?;
    let phase = if projection
      .sagas
      .values()
      .any(|saga| saga.phase == crate::SagaPhase::ManualRecovery)
    {
      "manual_recovery"
    } else if projection
      .sagas
      .values()
      .any(|saga| saga.phase == crate::SagaPhase::Correcting)
    {
      let cleanup_started = projection.sagas.values().any(|saga| {
        saga.actions.values().any(|action| {
          action.action_kind == crate::CorrectiveActionKind::Cleanup
            && action.status != crate::CorrectionActionStatus::Registered
        })
      });
      if cleanup_started {
        "cleanup"
      } else {
        "compensating"
      }
    } else if projection
      .sagas
      .values()
      .any(|saga| saga.phase == crate::SagaPhase::Forward)
    {
      "forward"
    } else if projection
      .sagas
      .values()
      .all(|saga| saga.phase == crate::SagaPhase::CorrectionCompleted)
    {
      "completed"
    } else {
      "forward"
    };
    let original_outcome = projection
      .sagas
      .values()
      .find_map(|saga| saga.reason)
      .map_or("running", |reason| match reason {
        crate::SagaCorrectionReason::Failure => "failed",
        crate::SagaCorrectionReason::Cancel => "cancelled",
        crate::SagaCorrectionReason::Success => "succeeded",
      });
    let mut sagas = Vec::new();
    let mut unresolved = Vec::new();
    for saga in projection.sagas.values() {
      let status = match saga.phase {
        crate::SagaPhase::Forward => "forward",
        crate::SagaPhase::ForwardSucceeded => "forward_succeeded",
        crate::SagaPhase::Correcting => "correcting",
        crate::SagaPhase::CorrectionCompleted => {
          if saga.actions.values().any(|action| {
            matches!(
              action.status,
              crate::CorrectionActionStatus::Failed | crate::CorrectionActionStatus::Interrupted
            )
          }) {
            "correction_incomplete"
          } else {
            "corrected"
          }
        }
        crate::SagaPhase::ManualRecovery => "correction_incomplete",
      };
      sagas.push(CorrectionSagaInspectionV1 {
        saga_id: saga.saga_id.clone(),
        status: status.to_string(),
        registered_compensations: u32::try_from(
          saga
            .actions
            .values()
            .filter(|action| action.action_kind == crate::CorrectiveActionKind::Compensation)
            .count(),
        )
        .unwrap_or(u32::MAX),
        completed_compensations: u32::try_from(
          saga
            .actions
            .values()
            .filter(|action| {
              action.action_kind == crate::CorrectiveActionKind::Compensation
                && action.status == crate::CorrectionActionStatus::Succeeded
            })
            .count(),
        )
        .unwrap_or(u32::MAX),
        completed_cleanup: u32::try_from(
          saga
            .actions
            .values()
            .filter(|action| {
              action.action_kind == crate::CorrectiveActionKind::Cleanup
                && action.status == crate::CorrectionActionStatus::Succeeded
            })
            .count(),
        )
        .unwrap_or(u32::MAX),
      });
      unresolved.extend(saga.actions.values().filter_map(|action| {
        let externally_settled = saga.phase == crate::SagaPhase::CorrectionCompleted
          && action.status == crate::CorrectionActionStatus::ManualRecovery;
        let unresolved = matches!(
          action.status,
          crate::CorrectionActionStatus::Failed
            | crate::CorrectionActionStatus::Interrupted
            | crate::CorrectionActionStatus::ManualRecovery
        );
        (unresolved && !externally_settled).then(|| UnresolvedCorrectionActionV1 {
          action_id: action.action_id.clone(),
          kind: action.action_kind,
          code: action
            .failure_code
            .clone()
            .unwrap_or_else(|| "WOML_CORRECTION_INCOMPLETE".to_string()),
          ambiguous: action.ambiguous,
        })
      }));
    }
    sagas.sort_by(|left, right| left.saga_id.cmp(&right.saga_id));
    unresolved.sort_by(|left, right| left.action_id.cmp(&right.action_id));
    let truncated = unresolved.len() > MAX_CORRECTION_INSPECTION_ROWS;
    unresolved.truncate(MAX_CORRECTION_INSPECTION_ROWS);
    let inspection = CorrectionInspectionV1 {
      profile: CORRECTION_INSPECTION_PROFILE,
      run_id: run_id.to_string(),
      phase: phase.to_string(),
      original_outcome: original_outcome.to_string(),
      sagas,
      unresolved_actions: unresolved,
      truncated,
    };
    if serde_json::to_vec(&inspection)?.len() > MAX_CORRECTION_INSPECTION_BYTES {
      return Err(DurableStoreError::Contract(
        "Correction Inspection v1 exceeded its frozen 1 MiB response limit.".to_string(),
      ));
    }
    Ok(inspection)
  }

  pub fn correction_action_externally_resolved(
    &self,
    run_id: &str,
    saga_id: &str,
    action_id: &str,
  ) -> Result<bool, DurableStoreError> {
    let resolved: bool = self.connection.query_row(
      "SELECT EXISTS(
         SELECT 1 FROM woml_correction_manual_recovery
         WHERE run_id = ?1 AND saga_id = ?2 AND action_id = ?3
           AND status = 'externally_resolved'
       )",
      params![run_id, saga_id, action_id],
      |row| row.get(0),
    )?;
    Ok(resolved)
  }

  pub fn recover_correction_v1(
    &mut self,
    request: CorrectionRecoveryRequestV1,
    now: DateTime<Utc>,
  ) -> Result<CorrectionRecoveryResultV1, DurableStoreError> {
    if request.profile != CORRECTION_RECOVERY_REQUEST_PROFILE
      || request.run_id.is_empty()
      || request.run_id.len() > 320
      || request.action_id.is_empty()
      || request.action_id.len() > 768
      || request.note.as_ref().is_some_and(|note| {
        note.is_empty() || note.len() > 1000 || note.chars().any(|character| character.is_control())
      })
      || request.action == CorrectionRecoveryActionV1::ExternallyResolve && request.note.is_none()
    {
      return Err(DurableStoreError::Contract(
        "Correction Recovery v1 request is invalid.".to_string(),
      ));
    }
    let projection = self.projection(&request.run_id)?;
    let mut matches = projection.sagas.values().filter_map(|saga| {
      saga
        .actions
        .get(&request.action_id)
        .map(|action| (saga.saga_id.clone(), action.clone()))
    });
    let Some((saga_id, action)) = matches.next() else {
      return Err(DurableStoreError::Contract(
        "WOML_CORRECTION_NOT_RECOVERABLE: the requested action is unknown.".to_string(),
      ));
    };
    if matches.next().is_some()
      || action.status != crate::CorrectionActionStatus::ManualRecovery
      || !action.ambiguous
    {
      return Err(DurableStoreError::Contract(
        "WOML_CORRECTION_NOT_RECOVERABLE: the requested action is not awaiting manual recovery."
          .to_string(),
      ));
    }
    let status = match request.action {
      CorrectionRecoveryActionV1::Retry => {
        if action.attempts >= 10 {
          return Err(DurableStoreError::Contract(
            "WOML_CORRECTION_LIMIT_EXCEEDED: the action reached the frozen ten-attempt limit."
              .to_string(),
          ));
        }
        self.append_payloads_atomically(
          &request.run_id,
          vec![(
            generated_event_id(),
            now,
            RunEventPayload::CorrectionRetryScheduled(crate::CorrectionRetryScheduledData {
              saga_id,
              action_id: request.action_id.clone(),
              action_kind: action.action_kind,
              attempt: action.attempts,
              retry_at: now,
            }),
          )],
        )?;
        CorrectionRecoveryStatusV1::RetryScheduled
      }
      CorrectionRecoveryActionV1::ExternallyResolve => {
        let note = request
          .note
          .as_deref()
          .expect("external resolution requires a note");
        let note_digest = sha256_prefixed(note.as_bytes());
        let transaction = self
          .connection
          .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
          "UPDATE woml_correction_manual_recovery
           SET status = 'externally_resolved',
               reason_code = ?1,
               resolved_at = ?2
           WHERE run_id = ?3 AND saga_id = ?4 AND action_id = ?5
             AND status = 'required'",
          params![
            format!("WOML_CORRECTION_EXTERNALLY_RESOLVED:{note_digest}"),
            now.to_rfc3339(),
            request.run_id,
            saga_id,
            request.action_id
          ],
        )?;
        if changed != 1 {
          return Err(DurableStoreError::Contract(
            "WOML_CORRECTION_NOT_RECOVERABLE: the recovery decision was already recorded."
              .to_string(),
          ));
        }
        transaction.commit()?;
        CorrectionRecoveryStatusV1::ExternallyResolved
      }
    };
    Ok(CorrectionRecoveryResultV1 {
      profile: CORRECTION_RECOVERY_RESULT_PROFILE,
      run_id: request.run_id,
      action_id: request.action_id,
      status,
      audit_recorded: true,
    })
  }

  pub fn correction_recovery_batch_v1(
    &self,
    limit: usize,
  ) -> Result<Vec<CorrectionRecoveryCandidateV1>, DurableStoreError> {
    self.query_correction_recovery_batch_v1(limit, None)
  }

  pub(crate) fn correction_recovery_batch_for_workflows_v1(
    &self,
    limit: usize,
    workflow_ids: &HashSet<String>,
  ) -> Result<Vec<CorrectionRecoveryCandidateV1>, DurableStoreError> {
    if workflow_ids.is_empty() {
      return Ok(Vec::new());
    }
    self.query_correction_recovery_batch_v1(limit, Some(workflow_ids))
  }

  fn query_correction_recovery_batch_v1(
    &self,
    limit: usize,
    workflow_ids: Option<&HashSet<String>>,
  ) -> Result<Vec<CorrectionRecoveryCandidateV1>, DurableStoreError> {
    if limit == 0 || limit > CORRECTION_RECOVERY_BATCH_LIMIT {
      return Err(DurableStoreError::Contract(format!(
        "Correction recovery limit must be between 1 and {CORRECTION_RECOVERY_BATCH_LIMIT}."
      )));
    }
    let mut parameters = Vec::new();
    let workflow_filter = workflow_ids.map(|workflow_ids| {
      let mut workflow_ids = workflow_ids.iter().cloned().collect::<Vec<_>>();
      workflow_ids.sort();
      let placeholders = workflow_ids
        .iter()
        .map(|workflow_id| {
          parameters.push(rusqlite::types::Value::Text(workflow_id.clone()));
          format!("?{}", parameters.len())
        })
        .collect::<Vec<_>>()
        .join(", ");
      format!(" AND runs.workflow_id IN ({placeholders})")
    });
    parameters.push(rusqlite::types::Value::Integer(
      i64::try_from(limit).unwrap_or(i64::MAX),
    ));
    let limit_parameter = parameters.len();
    let query = format!(
      "SELECT actions.run_id, actions.saga_id, actions.action_id, actions.status
       FROM woml_correction_actions AS actions
       JOIN woml_saga_scopes AS sagas
         ON sagas.run_id = actions.run_id AND sagas.saga_id = actions.saga_id
       JOIN woml_runs AS runs ON runs.run_id = actions.run_id
       LEFT JOIN woml_correction_manual_recovery AS manual
         ON manual.run_id = actions.run_id
        AND manual.saga_id = actions.saga_id
        AND manual.action_id = actions.action_id
       WHERE (
         (sagas.phase = 'correcting' AND actions.status IN ('registered', 'retry_scheduled'))
         OR
         (sagas.phase = 'manual_recovery' AND actions.status = 'manual_recovery'
           AND manual.status = 'externally_resolved')
       ){}
       ORDER BY actions.updated_at, actions.run_id, actions.saga_id, actions.action_id
       LIMIT ?{limit_parameter}",
      workflow_filter.as_deref().unwrap_or_default()
    );
    let mut statement = self.connection.prepare(&query)?;
    let candidates = statement
      .query_map(rusqlite::params_from_iter(parameters.iter()), |row| {
        Ok(CorrectionRecoveryCandidateV1 {
          run_id: row.get(0)?,
          saga_id: row.get(1)?,
          action_id: row.get(2)?,
          status: row.get(3)?,
        })
      })?
      .collect::<Result<Vec<_>, _>>()
      .map_err(DurableStoreError::from)?;
    Ok(candidates)
  }

  pub fn correction_observation_v1(&self) -> Result<CorrectionObservationV1, DurableStoreError> {
    let count = |sql: &str| -> Result<u64, DurableStoreError> {
      let value: i64 = self.connection.query_row(sql, [], |row| row.get(0))?;
      u64::try_from(value)
        .map_err(|_| DurableStoreError::Contract("Correction metric is invalid.".to_string()))
    };
    let oldest: Option<String> = self.connection.query_row(
      "SELECT MIN(created_at) FROM woml_correction_manual_recovery WHERE status = 'required'",
      [],
      |row| row.get(0),
    )?;
    let oldest_manual_recovery_age_ms = oldest
      .map(|value| {
        DateTime::parse_from_rfc3339(&value)
          .map(|time| Utc::now().signed_duration_since(time.with_timezone(&Utc)))
          .map_err(|_| DurableStoreError::Contract("Correction timestamp is invalid.".to_string()))
      })
      .transpose()?
      .and_then(|age| u64::try_from(age.num_milliseconds().max(0)).ok());
    Ok(CorrectionObservationV1 {
      profile: CORRECTION_OBSERVATION_PROFILE,
      open_sagas: count(
        "SELECT COUNT(*) FROM woml_saga_scopes WHERE phase IN ('correcting', 'manual_recovery')",
      )?,
      pending_actions: count(
        "SELECT COUNT(*) FROM woml_correction_actions WHERE status IN ('registered', 'started', 'retry_scheduled', 'failed', 'interrupted', 'manual_recovery')",
      )?,
      active_attempts: count(
        "SELECT COUNT(*) FROM woml_correction_attempts WHERE status = 'started'",
      )?,
      manual_recovery_actions: count(
        "SELECT COUNT(*) FROM woml_correction_manual_recovery WHERE status = 'required'",
      )?,
      oldest_manual_recovery_age_ms,
    })
  }

  /// Returns at most 100 redacted lock records for one run. Each item exactly
  /// matches the frozen Lock Inspection v1 schema; internal lease IDs,
  /// execution identities, digests, payloads, and runtime-owner data never
  /// cross this boundary.
  pub fn inspect_locks_v1(&self, run_id: &str) -> Result<Vec<LockInspectionV1>, DurableStoreError> {
    ensure_run_exists(&self.connection, run_id)?;
    let projection = self.projection(run_id)?;
    #[derive(Default)]
    struct ProgrammaticLockHistory {
      acquired_at: Option<DateTime<Utc>>,
      released_at: Option<DateTime<Utc>>,
      resolved_at: Option<DateTime<Utc>>,
      fencing_token: Option<String>,
      waited_ms: Option<u64>,
    }
    let mut programmatic_history = HashMap::<String, ProgrammaticLockHistory>::new();
    for event in load_events(&self.connection, run_id)? {
      match &event.payload {
        RunEventPayload::OperationSucceeded(operation) if operation.capability == "locks" => {
          let Some(session_ref) = operation.metadata.get("sessionRef").and_then(Value::as_str)
          else {
            continue;
          };
          let history = programmatic_history
            .entry(session_ref.to_string())
            .or_default();
          match operation.operation.as_str() {
            "acquire" => {
              history.acquired_at = operation
                .metadata
                .get("acquiredAt")
                .and_then(Value::as_str)
                .map(parse_stored_timestamp)
                .transpose()?
                .or(Some(event.occurred_at));
              history.fencing_token = operation
                .metadata
                .get("fence")
                .and_then(Value::as_str)
                .map(str::to_string);
              history.waited_ms = finite_duration_ms(operation.duration_ms);
            }
            "release" => {
              history.released_at = operation
                .metadata
                .get("releasedAt")
                .and_then(Value::as_str)
                .map(parse_stored_timestamp)
                .transpose()?
                .or(Some(event.occurred_at));
              history.resolved_at = history.released_at;
            }
            _ => {}
          }
        }
        RunEventPayload::OperationFailed(operation) if operation.capability == "locks" => {
          if let Some(session_ref) = operation.metadata.get("sessionRef").and_then(Value::as_str) {
            programmatic_history
              .entry(session_ref.to_string())
              .or_default()
              .resolved_at = Some(event.occurred_at);
          }
        }
        _ => {}
      }
    }
    let now = Utc::now();
    let mut projected = HashMap::new();
    for lock in projection.locks.values() {
      projected.insert(lock.execution_identity.clone(), lock.clone());
    }
    for loop_state in projection.for_each.values() {
      for execution in loop_state.executions.values() {
        for lock in execution.locks.values() {
          projected.insert(lock.execution_identity.clone(), lock.clone());
        }
      }
    }
    let mut statement = self.connection.prepare(
      "SELECT waiters.key, waiters.node_execution_id, waiters.registered_at,
              waiters.deadline, waiters.status, resources.acquired_at,
              resources.expires_at, CAST(resources.owner_fencing_token AS TEXT)
       FROM woml_lock_waiters AS waiters
       LEFT JOIN woml_lock_resources AS resources
         ON resources.key = waiters.key
        AND resources.owner_run_id = waiters.run_id
        AND resources.owner_node_execution_id = waiters.node_execution_id
       WHERE waiters.run_id = ?1
       ORDER BY waiters.registered_at, waiters.sequence, waiters.waiter_id
       LIMIT 100",
    )?;
    let rows = statement
      .query_map([run_id], |row| {
        Ok((
          row.get::<_, String>(0)?,
          row.get::<_, String>(1)?,
          row.get::<_, String>(2)?,
          row.get::<_, String>(3)?,
          row.get::<_, String>(4)?,
          row.get::<_, Option<String>>(5)?,
          row.get::<_, Option<String>>(6)?,
          row.get::<_, Option<String>>(7)?,
        ))
      })?
      .collect::<Result<Vec<_>, _>>()?;
    let mut locks = Vec::with_capacity(rows.len());
    for (
      key,
      execution_identity,
      registered_at,
      deadline,
      stored_status,
      stored_acquired_at,
      stored_expires_at,
      stored_fencing_token,
    ) in rows
    {
      let registered_at = parse_stored_timestamp(&registered_at)?;
      let deadline = parse_stored_timestamp(&deadline)?;
      let projected_lock = projected.get(&execution_identity);
      let capability_history = programmatic_lock_session_ref(&execution_identity)
        .and_then(|session_ref| programmatic_history.get(&session_ref));
      let acquired_at = stored_acquired_at
        .as_deref()
        .map(parse_stored_timestamp)
        .transpose()?
        .or_else(|| projected_lock.and_then(|lock| lock.acquired_at))
        .or_else(|| capability_history.and_then(|history| history.acquired_at));
      let expires_at = stored_expires_at
        .as_deref()
        .map(parse_stored_timestamp)
        .transpose()?
        .or_else(|| projected_lock.and_then(|lock| lock.expires_at));
      let released_at = projected_lock
        .and_then(|lock| lock.resolved_at)
        .or_else(|| capability_history.and_then(|history| history.released_at));
      let mut status = match stored_status.as_str() {
        "waiting" => InspectedLockStatusV1::Waiting,
        "held" => InspectedLockStatusV1::Held,
        "released" => InspectedLockStatusV1::Released,
        "timed_out" => InspectedLockStatusV1::TimedOut,
        "cancelled" => InspectedLockStatusV1::Cancelled,
        "lost" => InspectedLockStatusV1::Lost,
        _ => {
          return Err(DurableStoreError::LockHistoryInvalid(
            "lock inspection found an unsupported stored status".to_string(),
          ))
        }
      };
      if status == InspectedLockStatusV1::Held && expires_at.is_some_and(|expiry| expiry <= now) {
        status = InspectedLockStatusV1::Lost;
      }
      let waited_ms = capability_history
        .and_then(|history| history.waited_ms)
        .or_else(|| {
          let waited_until = match status {
            InspectedLockStatusV1::Waiting => Some(now),
            InspectedLockStatusV1::Held | InspectedLockStatusV1::Released => acquired_at,
            InspectedLockStatusV1::TimedOut
            | InspectedLockStatusV1::Cancelled
            | InspectedLockStatusV1::Lost => capability_history
              .and_then(|history| history.resolved_at)
              .or(released_at),
          }?;
          u64::try_from(
            waited_until
              .signed_duration_since(registered_at)
              .num_milliseconds()
              .max(0),
          )
          .ok()
        });
      let held_ms = acquired_at.and_then(|acquired| {
        let held_until = released_at
          .or_else(|| capability_history.and_then(|history| history.resolved_at))
          .or_else(|| (status == InspectedLockStatusV1::Held).then_some(now))?;
        u64::try_from(
          held_until
            .signed_duration_since(acquired)
            .num_milliseconds()
            .max(0),
        )
        .ok()
      });
      let failure_code = match status {
        InspectedLockStatusV1::TimedOut => Some("WOML_LOCK_ACQUIRE_TIMED_OUT".to_string()),
        InspectedLockStatusV1::Cancelled => Some("WOML_LOCK_ACQUISITION_CANCELLED".to_string()),
        InspectedLockStatusV1::Lost => Some("WOML_LOCK_LEASE_LOST".to_string()),
        _ => None,
      };
      locks.push(LockInspectionV1 {
        profile: "woml.lock-inspection/v1",
        run_id: run_id.to_string(),
        lock_id: projected_lock
          .map(|lock| lock.lock_id.clone())
          .unwrap_or_else(|| "services.locks.withLease".to_string()),
        key,
        status,
        registered_at,
        acquired_at,
        released_at,
        deadline: Some(deadline),
        expires_at,
        fencing_token: stored_fencing_token
          .or_else(|| projected_lock.and_then(|lock| lock.fencing_token.clone()))
          .or_else(|| capability_history.and_then(|history| history.fencing_token.clone())),
        waited_ms,
        held_ms,
        failure_code,
      });
    }
    Ok(locks)
  }

  pub fn lock_observation_v1(&self) -> Result<LockObservationV1, DurableStoreError> {
    let now = Utc::now();
    let held_locks = self.connection.query_row(
      "SELECT COUNT(*) FROM woml_lock_resources WHERE owner_lease_id IS NOT NULL AND expires_at > ?1",
      [now.to_rfc3339()],
      |row| row.get(0),
    )?;
    let waiting_acquisitions = self.connection.query_row(
      "SELECT COUNT(*) FROM woml_lock_waiters WHERE status = 'waiting'",
      [],
      |row| row.get(0),
    )?;
    let oldest_waiter: Option<String> = self.connection.query_row(
      "SELECT MIN(registered_at) FROM woml_lock_waiters WHERE status = 'waiting'",
      [],
      |row| row.get(0),
    )?;
    let oldest_waiter_age_ms = oldest_waiter
      .as_deref()
      .map(parse_stored_timestamp)
      .transpose()?
      .and_then(|registered_at| {
        u64::try_from(
          now
            .signed_duration_since(registered_at)
            .num_milliseconds()
            .max(0),
        )
        .ok()
      });
    let count = |status: &str| -> Result<u64, DurableStoreError> {
      Ok(self.connection.query_row(
        "SELECT COUNT(*) FROM woml_lock_waiters WHERE status = ?1",
        [status],
        |row| row.get(0),
      )?)
    };
    let renewal_failures = count("lost")?;
    let expired_takeovers = self.connection.query_row(
      "SELECT COUNT(*) FROM woml_lock_waiters AS lost
       WHERE lost.status = 'lost'
         AND EXISTS (
           SELECT 1 FROM woml_lock_waiters AS successor
           WHERE successor.key = lost.key
             AND successor.sequence > lost.sequence
             AND successor.status IN ('held', 'released', 'lost')
         )",
      [],
      |row| row.get(0),
    )?;
    Ok(LockObservationV1 {
      profile: "woml.lock-observation/v1",
      held_locks,
      waiting_acquisitions,
      oldest_waiter_age_ms,
      renewal_failures,
      expired_takeovers,
      acquisition_timeouts: count("timed_out")?,
    })
  }

  /// Returns at most one frozen recovery batch. Repeated calls after settling
  /// the returned work drain large backlogs without loading the whole queue
  /// into memory or monopolizing the durable store.
  pub fn lock_recovery_batch_v1(
    &self,
    now: DateTime<Utc>,
  ) -> Result<Vec<LockRecoveryCandidateV1>, DurableStoreError> {
    let limit = i64::try_from(LOCK_RECOVERY_BATCH_LIMIT)
      .map_err(|_| DurableStoreError::Contract("Lock recovery limit is invalid.".to_string()))?;
    let mut statement = self.connection.prepare(
      "SELECT kind, key, run_id, node_execution_id, due_at FROM (
         SELECT 'acquisition_timeout' AS kind, key, run_id, node_execution_id,
                deadline AS due_at
         FROM woml_lock_waiters
         WHERE status = 'waiting' AND deadline <= ?1
         UNION ALL
         SELECT 'lease_expiry' AS kind, key, owner_run_id AS run_id,
                owner_node_execution_id AS node_execution_id, expires_at AS due_at
         FROM woml_lock_resources
         WHERE owner_lease_id IS NOT NULL AND expires_at <= ?1
       )
       ORDER BY due_at, kind, key, run_id, node_execution_id
       LIMIT ?2",
    )?;
    let rows = statement
      .query_map(params![now.to_rfc3339(), limit], |row| {
        Ok((
          row.get::<_, String>(0)?,
          row.get::<_, String>(1)?,
          row.get::<_, String>(2)?,
          row.get::<_, String>(3)?,
          row.get::<_, String>(4)?,
        ))
      })?
      .collect::<Result<Vec<_>, _>>()?;
    rows
      .into_iter()
      .map(|(kind, key, run_id, execution_identity, due_at)| {
        let kind = match kind.as_str() {
          "acquisition_timeout" => LockRecoveryKindV1::AcquisitionTimeout,
          "lease_expiry" => LockRecoveryKindV1::LeaseExpiry,
          _ => {
            return Err(DurableStoreError::LockHistoryInvalid(
              "lock recovery found an unsupported work kind".to_string(),
            ))
          }
        };
        Ok(LockRecoveryCandidateV1 {
          profile: "woml.lock-recovery-candidate/v1",
          kind,
          key,
          run_id,
          execution_identity,
          due_at: parse_stored_timestamp(&due_at)?,
        })
      })
      .collect()
  }

  pub fn fork_recovery_work_v1(
    &self,
    run_id: &str,
  ) -> Result<ForkRecoveryWorkV1, DurableStoreError> {
    let binding = self.run_binding(run_id)?;
    let workflow = self.definition(&binding.definition_hash)?;
    if workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V13 {
      return Err(DurableStoreError::Contract(
        "Fork recovery work is available only for compiled Model v13 runs.".to_string(),
      ));
    }
    let projection = self.projection(run_id)?;
    let ready = ready_node_ids_for_projection(&workflow, &binding.definition_hash, &projection)
      .map_err(DurableStoreError::Contract)?;
    let mut openable_fork_ids = Vec::new();
    let mut pending_branches = Vec::new();
    let mut joinable_fork_ids = Vec::new();
    let mut pending_choice_ids = Vec::new();
    let mut ambiguous_active_node_ids = projection
      .active_attempt_node_ids()
      .into_iter()
      .map(str::to_string)
      .collect::<Vec<_>>();
    ambiguous_active_node_ids.sort();

    for choice in workflow.graph.choices.as_deref().unwrap_or_default() {
      if !projection.choice_selections.contains_key(&choice.choice_id)
        && ready.contains(&choice.selector_node_id)
      {
        pending_choice_ids.push(choice.choice_id.clone());
      }
    }
    for fork in workflow.graph.forks.as_deref().unwrap_or_default() {
      let Some(projected) = projection.forks.get(&fork.fork_id) else {
        if ready.contains(&fork.open_node_id) {
          openable_fork_ids.push(fork.fork_id.clone());
        }
        continue;
      };
      for branch in &fork.branches {
        if projected.branches.contains_key(&branch.branch_id) {
          continue;
        }
        let mut route = std::collections::HashSet::new();
        let mut queue = std::collections::VecDeque::from([branch.entry_node_id.as_str()]);
        while let Some(node_id) = queue.pop_front() {
          if !route.insert(node_id) || node_id == branch.terminal_node_id {
            continue;
          }
          for edge in workflow
            .graph
            .edges
            .iter()
            .filter(|edge| edge.from == node_id)
          {
            queue.push_back(edge.to.as_str());
          }
        }
        if !ambiguous_active_node_ids
          .iter()
          .any(|node_id| route.contains(node_id.as_str()))
        {
          pending_branches.push(ForkRecoveryBranchV1 {
            fork_id: fork.fork_id.clone(),
            branch_id: branch.branch_id.clone(),
          });
        }
      }
      if projected.join_status == crate::projection::ForkJoinStatus::Pending
        && fork.joined_branch_ids.iter().all(|branch_id| {
          projected
            .branches
            .get(branch_id)
            .and_then(|branch| branch.outcome)
            == Some(crate::event::ForkBranchOutcome::Succeeded)
        })
      {
        joinable_fork_ids.push(fork.fork_id.clone());
      }
    }
    Ok(ForkRecoveryWorkV1 {
      profile: "woml.fork-recovery-work/v1",
      openable_fork_ids,
      pending_branches,
      joinable_fork_ids,
      pending_choice_ids,
      ambiguous_active_node_ids,
    })
  }

  pub fn claim_policy_run(
    &mut self,
    run_id: &str,
    owner_id: &str,
    now: DateTime<Utc>,
    lease_duration: Duration,
  ) -> Result<SchedulerClaimV1, DurableStoreError> {
    match self.try_claim_policy_run(run_id, owner_id, now, lease_duration)? {
      PolicyExecutionClaimResult::Claimed { claim, .. } => Ok(claim),
      PolicyExecutionClaimResult::Waiting { .. } => Err(DurableStoreError::SchedulerClaimConflict(
        run_id.to_string(),
      )),
    }
  }

  pub fn try_claim_policy_run(
    &mut self,
    run_id: &str,
    owner_id: &str,
    now: DateTime<Utc>,
    lease_duration: Duration,
  ) -> Result<PolicyExecutionClaimResult, DurableStoreError> {
    self.try_claim_policy_run_at(run_id, owner_id, now, now, lease_duration)
  }

  /// Claims executable capacity using separate clocks for the expiring owner
  /// lease and the durable policy timeline. Production uses wall time for the
  /// lease and the runtime clock for deterministic policy eligibility.
  pub fn try_claim_policy_run_at(
    &mut self,
    run_id: &str,
    owner_id: &str,
    lease_now: DateTime<Utc>,
    policy_now: DateTime<Utc>,
    lease_duration: Duration,
  ) -> Result<PolicyExecutionClaimResult, DurableStoreError> {
    if owner_id.is_empty() || owner_id.len() > 320 || lease_duration.is_zero() {
      return Err(DurableStoreError::Contract(
        "A scheduler claim requires a valid owner and positive lease.".to_string(),
      ));
    }
    let expires_at = lease_now
      + chrono::Duration::from_std(lease_duration).map_err(|_| {
        DurableStoreError::Contract("Scheduler lease duration is out of range.".to_string())
      })?;
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute(
      "DELETE FROM woml_scheduler_claims WHERE expires_at <= ?1",
      [lease_now.to_rfc3339()],
    )?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let policy = workflow.runtime_policy.as_ref().ok_or_else(|| {
      DurableStoreError::Contract("Scheduler claims require compiled Model v12.".to_string())
    })?;
    let queue = workflow.runtime_policy_queue_name().ok_or_else(|| {
      DurableStoreError::Contract("Model v12 has no runtime policy queue.".to_string())
    })?;
    let events = load_events(&transaction, run_id)?;
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = fold_events(&events)?;
    if matches!(
      projection.status,
      RunStatus::NotStarted
        | RunStatus::Waiting
        | RunStatus::Succeeded
        | RunStatus::Failed
        | RunStatus::Cancelled
    ) {
      return Err(DurableStoreError::Contract(
        "Only an executable Event v11 policy run may acquire a scheduler claim.".to_string(),
      ));
    }
    let existing: Option<(String, String, String, String)> = transaction
      .query_row(
        "SELECT owner_id, claim_id, claimed_at, expires_at
         FROM woml_scheduler_claims WHERE run_id = ?1",
        [run_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
      )
      .optional()?;
    if let Some((existing_owner, claim_id, claimed_at, expires_at)) = existing {
      if existing_owner != owner_id {
        return Ok(PolicyExecutionClaimResult::Waiting {
          run_id: run_id.to_string(),
          workflow_id: binding.workflow_id,
          queue,
          reason: PolicyClaimWaitReason::Concurrency,
          eligible_at: None,
        });
      }
      return Ok(PolicyExecutionClaimResult::Claimed {
        claim: SchedulerClaimV1 {
          profile: "woml.scheduler-claim/v1",
          run_id: run_id.to_string(),
          workflow_id: binding.workflow_id,
          owner_id: existing_owner,
          claim_id,
          claimed_at: parse_durable_timestamp(&claimed_at, "scheduler claim instant")?,
          expires_at: parse_durable_timestamp(&expires_at, "scheduler claim expiry")?,
        },
        first_start: false,
      });
    }
    let has_ambiguous_work = projection
      .attempts
      .iter()
      .any(|attempt| attempt.status == AttemptStatus::Started)
      || projection
        .operations
        .values()
        .any(|operation| operation.status == crate::projection::OperationStatus::Started)
      || projection.lifecycle_hooks.values().any(|hook| {
        hook
          .actions
          .values()
          .any(|action| action.status == crate::projection::LifecycleActionStatus::Started)
      })
      || projection.for_each.values().any(|loop_state| {
        loop_state.status == ForEachStatus::Open
          && loop_state.executions.values().any(|execution| {
            execution
              .attempts
              .iter()
              .any(|attempt| attempt.status == AttemptStatus::Started)
              || execution
                .operations
                .values()
                .any(|operation| operation.status == crate::projection::OperationStatus::Started)
              || execution.lifecycle_hooks.values().any(|hook| {
                matches!(
                  hook.status,
                  crate::projection::LifecycleHookStatus::Requested
                    | crate::projection::LifecycleHookStatus::Running
                )
              })
              || execution.reusable_lifecycle_hooks.values().any(|hook| {
                matches!(
                  hook.status,
                  crate::projection::ReusableLifecycleStatus::Requested
                    | crate::projection::ReusableLifecycleStatus::Running
                )
              })
          })
      });
    if has_ambiguous_work {
      return Err(DurableStoreError::SchedulerRecoveryRequired(
        run_id.to_string(),
      ));
    }
    let first_start = projection.started_at.is_none() && projection.status == RunStatus::Queued;
    let rate_eligibility = if first_start {
      workflow_rate_eligibility(
        &transaction,
        &binding.workflow_id,
        policy.rate_limit.as_ref(),
        policy_now,
      )?
    } else {
      PolicyRateEligibility::Eligible
    };
    if let PolicyRateEligibility::Waiting { eligible_at } = rate_eligibility {
      transaction.execute(
        "UPDATE woml_run_summaries
         SET waiting_for = 'rate_limit', eligible_at = ?1, updated_at = ?2
         WHERE run_id = ?3",
        params![eligible_at.to_rfc3339(), policy_now.to_rfc3339(), run_id],
      )?;
      transaction.commit()?;
      return Ok(PolicyExecutionClaimResult::Waiting {
        run_id: run_id.to_string(),
        workflow_id: binding.workflow_id,
        queue,
        reason: PolicyClaimWaitReason::RateLimit,
        eligible_at: Some(eligible_at),
      });
    }
    if !workflow_has_policy_capacity(&transaction, &binding.workflow_id, policy.concurrency)?
      || (projection.status == RunStatus::Queued
        && earlier_eligible_policy_run_exists(
          &transaction,
          run_id,
          &queue,
          projection.admitted_at.ok_or_else(|| {
            DurableStoreError::Contract("Queued policy run has no admittedAt.".to_string())
          })?,
          projection.occurrence_sequence.unwrap_or(0),
          policy_now,
        )?)
    {
      transaction.execute(
        "UPDATE woml_run_summaries
         SET waiting_for = 'concurrency', eligible_at = NULL, updated_at = ?1
         WHERE run_id = ?2",
        params![policy_now.to_rfc3339(), run_id],
      )?;
      transaction.commit()?;
      return Ok(PolicyExecutionClaimResult::Waiting {
        run_id: run_id.to_string(),
        workflow_id: binding.workflow_id,
        queue,
        reason: PolicyClaimWaitReason::Concurrency,
        eligible_at: None,
      });
    }
    let claim_id = format!("claim_{}", Uuid::new_v4().simple());
    transaction.execute(
      "INSERT INTO woml_scheduler_claims(
         run_id, workflow_id, owner_id, claim_id, claimed_at, expires_at
       ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
      params![
        run_id,
        binding.workflow_id,
        owner_id,
        claim_id,
        lease_now.to_rfc3339(),
        expires_at.to_rfc3339(),
      ],
    )?;
    if first_start {
      let timeout_at = policy.timeout_ms.map(|timeout_ms| {
        policy_now + chrono::Duration::milliseconds(i64::try_from(timeout_ms).unwrap_or(i64::MAX))
      });
      let mut events = events;
      let event_schema_version = events
        .first()
        .map(|event| event.event_schema_version)
        .ok_or_else(|| {
          DurableStoreError::Contract("Policy run has no admission event.".to_string())
        })?;
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        policy_now,
        event_schema_version,
        RunEventPayload::RunExecutionStarted(crate::event::RunExecutionStartedData {
          started_at: policy_now,
          timeout_at,
        }),
      )?;
      validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
        .map_err(DurableStoreError::Contract)?;
    }
    transaction.execute(
      "UPDATE woml_run_summaries
       SET waiting_for = NULL, eligible_at = NULL, updated_at = ?1
       WHERE run_id = ?2",
      params![policy_now.to_rfc3339(), run_id],
    )?;
    transaction.commit()?;
    Ok(PolicyExecutionClaimResult::Claimed {
      claim: SchedulerClaimV1 {
        profile: "woml.scheduler-claim/v1",
        run_id: run_id.to_string(),
        workflow_id: binding.workflow_id,
        owner_id: owner_id.to_string(),
        claim_id,
        claimed_at: lease_now,
        expires_at,
      },
      first_start,
    })
  }

  pub fn renew_policy_claim(
    &mut self,
    run_id: &str,
    owner_id: &str,
    claim_id: &str,
    now: DateTime<Utc>,
    lease_duration: Duration,
  ) -> Result<SchedulerClaimV1, DurableStoreError> {
    let expires_at = now
      + chrono::Duration::from_std(lease_duration).map_err(|_| {
        DurableStoreError::Contract("Scheduler lease duration is out of range.".to_string())
      })?;
    let changed = self.connection.execute(
      "UPDATE woml_scheduler_claims SET claimed_at = ?1, expires_at = ?2
       WHERE run_id = ?3 AND owner_id = ?4 AND claim_id = ?5 AND expires_at > ?1",
      params![
        now.to_rfc3339(),
        expires_at.to_rfc3339(),
        run_id,
        owner_id,
        claim_id,
      ],
    )?;
    if changed != 1 {
      return Err(DurableStoreError::SchedulerClaimConflict(
        run_id.to_string(),
      ));
    }
    let workflow_id = self.run_binding(run_id)?.workflow_id;
    Ok(SchedulerClaimV1 {
      profile: "woml.scheduler-claim/v1",
      run_id: run_id.to_string(),
      workflow_id,
      owner_id: owner_id.to_string(),
      claim_id: claim_id.to_string(),
      claimed_at: now,
      expires_at,
    })
  }

  pub fn suspend_policy_claim(
    &mut self,
    run_id: &str,
    owner_id: &str,
    claim_id: &str,
  ) -> Result<bool, DurableStoreError> {
    Ok(
      self.connection.execute(
        "UPDATE woml_scheduler_claims SET workflow_id = ?1
         WHERE run_id = ?2 AND owner_id = ?3 AND claim_id = ?4",
        params![SUSPENDED_POLICY_WORKFLOW_ID, run_id, owner_id, claim_id],
      )? == 1,
    )
  }

  pub fn resume_policy_claim(
    &mut self,
    run_id: &str,
    owner_id: &str,
    claim_id: &str,
    now: DateTime<Utc>,
  ) -> Result<bool, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute(
      "DELETE FROM woml_scheduler_claims WHERE expires_at <= ?1",
      [now.to_rfc3339()],
    )?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let concurrency = workflow
      .runtime_policy
      .as_ref()
      .ok_or_else(|| {
        DurableStoreError::Contract("Policy resume requires compiled Model v12.".to_string())
      })?
      .concurrency;
    let owned: bool = transaction.query_row(
      "SELECT EXISTS(
         SELECT 1 FROM woml_scheduler_claims
         WHERE run_id = ?1 AND owner_id = ?2 AND claim_id = ?3
           AND workflow_id = ?4 AND expires_at > ?5
       )",
      params![
        run_id,
        owner_id,
        claim_id,
        SUSPENDED_POLICY_WORKFLOW_ID,
        now.to_rfc3339(),
      ],
      |row| row.get(0),
    )?;
    if !owned {
      return Err(DurableStoreError::SchedulerClaimConflict(
        run_id.to_string(),
      ));
    }
    if !workflow_has_policy_capacity(&transaction, &binding.workflow_id, concurrency)? {
      transaction.commit()?;
      return Ok(false);
    }
    let changed = transaction.execute(
      "UPDATE woml_scheduler_claims SET workflow_id = ?1
       WHERE run_id = ?2 AND owner_id = ?3 AND claim_id = ?4
         AND workflow_id = ?5",
      params![
        binding.workflow_id,
        run_id,
        owner_id,
        claim_id,
        SUSPENDED_POLICY_WORKFLOW_ID,
      ],
    )?;
    transaction.commit()?;
    Ok(changed == 1)
  }

  pub fn release_policy_claim(
    &mut self,
    run_id: &str,
    owner_id: &str,
    claim_id: &str,
  ) -> Result<bool, DurableStoreError> {
    Ok(
      self.connection.execute(
        "DELETE FROM woml_scheduler_claims
         WHERE run_id = ?1 AND owner_id = ?2 AND claim_id = ?3",
        params![run_id, owner_id, claim_id],
      )? == 1,
    )
  }

  pub fn active_policy_claim_count(
    &self,
    workflow_id: &str,
    now: DateTime<Utc>,
  ) -> Result<u32, DurableStoreError> {
    let count: i64 = self.connection.query_row(
      "SELECT COUNT(*) FROM woml_scheduler_claims
       WHERE workflow_id = ?1 AND expires_at > ?2",
      params![workflow_id, now.to_rfc3339()],
      |row| row.get(0),
    )?;
    u32::try_from(count)
      .map_err(|_| DurableStoreError::Contract("Scheduler claim count exceeded u32.".to_string()))
  }

  pub(crate) fn policy_run_has_live_claim(
    &mut self,
    run_id: &str,
    now: DateTime<Utc>,
  ) -> Result<bool, DurableStoreError> {
    self.connection.execute(
      "DELETE FROM woml_scheduler_claims WHERE expires_at <= ?1",
      [now.to_rfc3339()],
    )?;
    self
      .connection
      .query_row(
        "SELECT EXISTS(SELECT 1 FROM woml_scheduler_claims WHERE run_id = ?1)",
        [run_id],
        |row| row.get(0),
      )
      .map_err(Into::into)
  }

  /// Applies the normal fail-closed recovery policy after a scheduler owner
  /// disappeared. A live owner can never be recovered by another process.
  pub fn recover_policy_run_after_lease_loss(
    &mut self,
    run_id: &str,
    now: DateTime<Utc>,
  ) -> Result<bool, DurableStoreError> {
    self.connection.execute(
      "DELETE FROM woml_scheduler_claims WHERE expires_at <= ?1",
      [now.to_rfc3339()],
    )?;
    let live_claim: bool = self.connection.query_row(
      "SELECT EXISTS(
         SELECT 1 FROM woml_scheduler_claims
         WHERE run_id = ?1 AND expires_at > ?2
       )",
      params![run_id, now.to_rfc3339()],
      |row| row.get(0),
    )?;
    if live_claim {
      return Err(DurableStoreError::SchedulerClaimConflict(
        run_id.to_string(),
      ));
    }
    Ok(matches!(
      self.recover_run(run_id)?,
      RunRecovery::Recovered { .. }
    ))
  }

  pub fn request_run_cancellation(
    &mut self,
    run_id: &str,
    command_id: &str,
    now: DateTime<Utc>,
  ) -> Result<RunCancellationResult, DurableStoreError> {
    if run_id.is_empty() || run_id.len() > 320 || command_id.is_empty() || command_id.len() > 320 {
      return Err(DurableStoreError::Contract(
        "Run cancellation requires bounded run and command identities.".to_string(),
      ));
    }
    let result = |status, code| RunCancellationResult {
      profile: "woml.run-control.result/v1",
      command_id: command_id.to_string(),
      run_id: run_id.to_string(),
      status,
      code,
    };
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(binding) = load_run_binding_optional(&transaction, run_id)? else {
      return Ok(result(
        RunCancellationStatus::Rejected,
        Some(RunCancellationCode::WomlRunNotFound),
      ));
    };
    let workflow = definition_by_hash(&transaction, &binding.definition_hash)?;
    if !matches!(
      workflow.schema_version,
      crate::COMPILED_MODEL_SCHEMA_VERSION_V11
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V12
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V13
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V14
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V15
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V16
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V17
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V18
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V19
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V20
    ) {
      return Ok(result(
        RunCancellationStatus::Rejected,
        Some(RunCancellationCode::WomlRunControlVersionUnsupported),
      ));
    }
    let mut events = load_events(&transaction, run_id)?;
    let projection = fold_events(&events)?;
    if projection.business_outcome == Some(crate::event::BusinessOutcome::Cancelled) {
      return Ok(result(RunCancellationStatus::AlreadyCancelled, None));
    }
    if projection.cancellation_request_id.is_some() {
      return Ok(result(RunCancellationStatus::AlreadyRequested, None));
    }
    if projection.business_outcome.is_some() {
      return Ok(result(
        RunCancellationStatus::Rejected,
        Some(RunCancellationCode::WomlRunOutcomeAlreadyDecided),
      ));
    }
    if matches!(
      projection.status,
      RunStatus::Succeeded | RunStatus::Failed | RunStatus::Cancelled
    ) {
      return Ok(result(
        RunCancellationStatus::Rejected,
        Some(RunCancellationCode::WomlRunAlreadyTerminal),
      ));
    }
    let event_schema_version = projection.event_schema_version.ok_or_else(|| {
      DurableStoreError::Contract("A run cancellation requires run_started.".to_string())
    })?;
    append_to_history(
      &transaction,
      &mut events,
      run_id,
      generated_event_id(),
      now,
      event_schema_version,
      RunEventPayload::RunCancellationRequested(RunCancellationRequestedData {
        request_id: command_id.to_string(),
      }),
    )?;
    if event_schema_version == crate::RUN_EVENT_SCHEMA_VERSION_V16
      && projection.status == RunStatus::Waiting
    {
      let active_wait_ids = projection
        .durable_waits
        .values()
        .filter(|wait| wait.status == crate::DurableWaitStatus::Registered)
        .map(|wait| wait.wait_id.clone())
        .collect::<Vec<_>>();
      for wait_id in active_wait_ids {
        let resolution_event_id = generated_event_id();
        append_to_history(
          &transaction,
          &mut events,
          run_id,
          resolution_event_id.clone(),
          now,
          event_schema_version,
          RunEventPayload::WaitCancelled(crate::WaitCancelledData {
            wait_id: wait_id.clone(),
            cancelled_at: now,
            cancellation_id: command_id.to_string(),
          }),
        )?;
        let changed = transaction.execute(
          "UPDATE woml_durable_waits
           SET state = 'cancelled', resolution_event_id = ?1, resolved_at = ?2
           WHERE run_id = ?3 AND wait_id = ?4 AND iteration_identity = ''
             AND state = 'registered'",
          params![resolution_event_id, now.to_rfc3339(), run_id, wait_id],
        )?;
        if changed != 1 {
          return Err(DurableStoreError::Contract(
            "Run cancellation lost the durable wait winner transaction.".to_string(),
          ));
        }
      }
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        now,
        event_schema_version,
        RunEventPayload::RunOutcomeDecided(crate::RunOutcomeDecidedData::Cancelled {
          cancellation_request_id: command_id.to_string(),
        }),
      )?;
    }
    if let Some(hook) =
      workflow.lifecycle_hook_for_event(crate::model::LifecycleEventName::RunCancel)
    {
      let subject = LifecycleSubject {
        kind: LifecycleSubjectKind::Workflow,
        id: run_id.to_string(),
      };
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        now,
        event_schema_version,
        RunEventPayload::LifecycleHookRequested(LifecycleHookRequestedData {
          hook_invocation_id: derive_lifecycle_hook_invocation_id(
            run_id,
            &hook.hook_id,
            subject.kind,
            &subject.id,
          ),
          hook_id: hook.hook_id.clone(),
          event: hook.event,
          subject,
        }),
      )?;
    }
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    transaction.commit()?;
    Ok(result(RunCancellationStatus::Accepted, None))
  }

  pub fn decide_run_outcome(
    &mut self,
    run_id: &str,
    outcome: crate::event::RunOutcomeDecidedData,
    now: DateTime<Utc>,
  ) -> Result<RunProjection, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    if !matches!(
      workflow.schema_version,
      crate::COMPILED_MODEL_SCHEMA_VERSION_V11
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V12
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V13
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V14
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V15
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V16
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V17
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V18
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V19
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V20
    ) {
      return Err(DurableStoreError::Contract(
        "Business-outcome authority requires compiled Model v11+.".to_string(),
      ));
    }
    let mut events = load_events(&transaction, run_id)?;
    let projection = fold_events(&events)?;
    if projection.business_outcome.is_some() {
      return Err(DurableStoreError::Contract(
        "A run business outcome is already durable.".to_string(),
      ));
    }
    let event_schema_version = projection.event_schema_version.ok_or_else(|| {
      DurableStoreError::Contract("Business outcome requires run_started.".to_string())
    })?;
    let lifecycle_event = match outcome.outcome() {
      crate::event::BusinessOutcome::Succeeded => crate::model::LifecycleEventName::RunSuccess,
      crate::event::BusinessOutcome::Failed => crate::model::LifecycleEventName::RunFailure,
      crate::event::BusinessOutcome::Cancelled => crate::model::LifecycleEventName::RunCancel,
    };
    append_to_history(
      &transaction,
      &mut events,
      run_id,
      generated_event_id(),
      now,
      event_schema_version,
      RunEventPayload::RunOutcomeDecided(outcome),
    )?;
    if let Some(hook) = workflow
      .lifecycle_hook_for_event(lifecycle_event)
      .or_else(|| workflow.lifecycle_hook_for_event(crate::model::LifecycleEventName::RunComplete))
    {
      let already_requested = fold_events(&events)?
        .lifecycle_hooks
        .values()
        .any(|candidate| {
          candidate.hook_id == hook.hook_id
            && candidate.subject.kind == LifecycleSubjectKind::Workflow
            && candidate.subject.id == run_id
        });
      if !already_requested {
        let subject = LifecycleSubject {
          kind: LifecycleSubjectKind::Workflow,
          id: run_id.to_string(),
        };
        append_to_history(
          &transaction,
          &mut events,
          run_id,
          generated_event_id(),
          now,
          event_schema_version,
          RunEventPayload::LifecycleHookRequested(LifecycleHookRequestedData {
            hook_invocation_id: derive_lifecycle_hook_invocation_id(
              run_id,
              &hook.hook_id,
              subject.kind,
              &subject.id,
            ),
            hook_id: hook.hook_id.clone(),
            event: hook.event,
            subject,
          }),
        )?;
      }
    }
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = fold_events(&events)?;
    transaction.commit()?;
    Ok(projection)
  }

  /// Atomically lets the immutable workflow deadline race the normal outcome
  /// and operator cancellation authorities. A committed outcome or
  /// cancellation request always wins and can never be rewritten by timeout.
  pub fn settle_run_timeout(
    &mut self,
    run_id: &str,
    now: DateTime<Utc>,
  ) -> Result<RunTimeoutSettlement, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    if workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V12 {
      return Err(DurableStoreError::Contract(
        "Workflow timeout authority requires compiled Model v12.".to_string(),
      ));
    }
    let mut events = load_events(&transaction, run_id)?;
    let projection = fold_events(&events)?;
    let event_schema_version = projection.event_schema_version.ok_or_else(|| {
      DurableStoreError::Contract("Workflow timeout run has no event version.".to_string())
    })?;
    let Some(deadline_at) = projection.timeout_at else {
      return Ok(RunTimeoutSettlement::NotConfigured);
    };
    if projection.business_outcome.is_some()
      || projection.cancellation_request_id.is_some()
      || projection.timeout_reached_at.is_some()
    {
      return Ok(RunTimeoutSettlement::LostRace { projection });
    }
    if now < deadline_at {
      return Ok(RunTimeoutSettlement::NotDue { deadline_at });
    }
    if !matches!(projection.status, RunStatus::Running | RunStatus::Waiting) {
      return Ok(RunTimeoutSettlement::LostRace { projection });
    }
    append_to_history(
      &transaction,
      &mut events,
      run_id,
      generated_event_id(),
      deadline_at,
      event_schema_version,
      RunEventPayload::RunTimeoutReached(crate::event::RunTimeoutReachedData {
        deadline_at,
        code: "WOML_WORKFLOW_TIMED_OUT".to_string(),
      }),
    )?;
    for operation in projection
      .operations
      .values()
      .filter(|operation| operation.status == crate::projection::OperationStatus::Started)
    {
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        now,
        event_schema_version,
        RunEventPayload::OperationFailed(OperationFailedData {
          node_id: operation.node_id.clone(),
          attempt_number: operation.attempt_number,
          invocation_id: operation.identity.invocation_id.clone(),
          call_id: operation.identity.call_id.clone(),
          operation_key: operation.operation_key.clone(),
          capability: operation.capability.clone(),
          operation: operation.operation.clone(),
          execution_mode: operation.execution_mode,
          metadata: operation.metadata.clone(),
          duration_ms: 0.0,
          failure: crate::CapabilityFailure {
            kind: crate::CapabilityFailureKind::Ambiguous,
            code: "WOML_CAPABILITY_TIMEOUT_AMBIGUOUS".to_string(),
            message: "The workflow deadline interrupted a managed operation before its terminal outcome became durable; it will not be replayed.".to_string(),
            retryable: false,
            ambiguous: true,
            details: None,
          },
        }),
      )?;
    }
    for attempt in projection
      .attempts
      .iter()
      .filter(|attempt| attempt.status == AttemptStatus::Started)
    {
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        now,
        event_schema_version,
        RunEventPayload::StepAttemptFailed(StepAttemptFailedData {
          node_id: attempt.identity.node_id.clone(),
          attempt: attempt.identity.attempt,
          invocation_id: attempt.identity.invocation_id.clone(),
          failure: AttemptFailure {
            kind: AttemptFailureKind::Interrupted,
            code: AttemptFailureKind::Interrupted.code().to_string(),
            message: "The workflow deadline interrupted this attempt before its terminal outcome became durable.".to_string(),
            details: None,
            ..AttemptFailure::legacy_defaults()
          },
        }),
      )?;
    }
    // Model v13 owns every opened fork branch until it reaches a durable
    // terminal boundary. A workflow timeout closes those boundaries before
    // deciding the run outcome so recovery cannot leave orphaned branch work.
    if workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V13 {
      for fork in workflow.graph.forks.as_deref().unwrap_or_default() {
        let Some(projected) = projection.forks.get(&fork.fork_id) else {
          continue;
        };
        for branch in &fork.branches {
          if projected.branches.contains_key(&branch.branch_id) {
            continue;
          }
          append_to_history(
            &transaction,
            &mut events,
            run_id,
            generated_event_id(),
            now,
            event_schema_version,
            RunEventPayload::ForkBranchSettled(crate::event::ForkBranchSettledData {
              fork_id: fork.fork_id.clone(),
              branch_id: branch.branch_id.clone(),
              terminal_node_id: branch.terminal_node_id.clone(),
              outcome: crate::event::ForkBranchOutcome::Cancelled,
            }),
          )?;
        }
        if projected.join_status == crate::projection::ForkJoinStatus::Pending {
          let (outcome, blocking_branch_id) =
            if let Some(branch_id) = fork.joined_branch_ids.first() {
              (
                crate::event::ForkJoinOutcome::Cancelled,
                Some(branch_id.clone()),
              )
            } else {
              (crate::event::ForkJoinOutcome::Succeeded, None)
            };
          append_to_history(
            &transaction,
            &mut events,
            run_id,
            generated_event_id(),
            now,
            event_schema_version,
            RunEventPayload::ForkJoinSettled(crate::event::ForkJoinSettledData {
              fork_id: fork.fork_id.clone(),
              outcome,
              blocking_branch_id,
            }),
          )?;
        }
      }
    }
    recover_open_for_each_failure(
      &transaction,
      &workflow,
      &mut events,
      run_id,
      event_schema_version,
      now,
      Some(AttemptFailure {
        kind: AttemptFailureKind::ScriptTimedOut,
        code: AttemptFailureKind::ScriptTimedOut.code().to_string(),
        message: "The workflow deadline interrupted this loop attempt before its terminal outcome became durable.".to_string(),
        details: None,
        ..AttemptFailure::legacy_defaults()
      }),
    )?;
    append_to_history(
      &transaction,
      &mut events,
      run_id,
      generated_event_id(),
      now,
      event_schema_version,
      RunEventPayload::RunOutcomeDecided(crate::event::RunOutcomeDecidedData::Failed {
        failure: crate::event::LifecycleFailure {
          kind: crate::event::LifecycleFailureKind::TimedOut,
          code: "WOML_WORKFLOW_TIMED_OUT".to_string(),
          message: "The workflow exceeded its configured total run timeout.".to_string(),
        },
      }),
    )?;
    if let Some(hook) = workflow
      .lifecycle_hook_for_event(crate::model::LifecycleEventName::RunFailure)
      .or_else(|| workflow.lifecycle_hook_for_event(crate::model::LifecycleEventName::RunComplete))
    {
      let subject = LifecycleSubject {
        kind: LifecycleSubjectKind::Workflow,
        id: run_id.to_string(),
      };
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        now,
        event_schema_version,
        RunEventPayload::LifecycleHookRequested(LifecycleHookRequestedData {
          hook_invocation_id: derive_lifecycle_hook_invocation_id(
            run_id,
            &hook.hook_id,
            subject.kind,
            &subject.id,
          ),
          hook_id: hook.hook_id.clone(),
          event: hook.event,
          subject,
        }),
      )?;
    }
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = fold_events(&events)?;
    transaction.commit()?;
    Ok(RunTimeoutSettlement::TimedOut { projection })
  }

  pub fn finalize_run_v11(
    &mut self,
    run_id: &str,
    now: DateTime<Utc>,
  ) -> Result<RunProjection, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    if !matches!(
      workflow.schema_version,
      crate::COMPILED_MODEL_SCHEMA_VERSION_V11
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V12
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V13
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V14
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V15
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V16
        | crate::COMPILED_MODEL_SCHEMA_VERSION_V17
    ) {
      return Err(DurableStoreError::Contract(
        "Run finalization authority requires compiled Model v11+.".to_string(),
      ));
    }
    let mut events = load_events(&transaction, run_id)?;
    let projection = fold_events(&events)?;
    let outcome = projection.business_outcome.ok_or_else(|| {
      DurableStoreError::Contract("Run finalization requires a decided outcome.".to_string())
    })?;
    if projection.lifecycle_hooks.values().any(|hook| {
      matches!(
        hook.status,
        crate::projection::LifecycleHookStatus::Requested
          | crate::projection::LifecycleHookStatus::Running
      )
    }) {
      return Err(DurableStoreError::Contract(
        "Run finalization requires every admitted lifecycle hook to complete.".to_string(),
      ));
    }
    let warnings = lifecycle_warnings_from_projection(&projection);
    let lifecycle_status = if warnings.is_empty() {
      crate::event::FinalLifecycleStatus::Completed
    } else {
      crate::event::FinalLifecycleStatus::CompletedWithWarnings
    };
    let event_schema_version = projection.event_schema_version.unwrap();
    append_to_history(
      &transaction,
      &mut events,
      run_id,
      generated_event_id(),
      now,
      event_schema_version,
      RunEventPayload::RunFinalized(crate::event::RunFinalizedData {
        outcome,
        lifecycle_status,
        warnings,
      }),
    )?;
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = fold_events(&events)?;
    transaction.commit()?;
    Ok(projection)
  }

  pub fn begin_notification_delivery(
    &mut self,
    run_id: &str,
    delivery_id: &str,
    now: DateTime<Utc>,
  ) -> Result<NotificationDeliveryWork, DurableStoreError> {
    // Telegram callback_data is limited to 64 bytes. A 64-bit random lookup
    // identity plus a 128-bit secret keeps the full capability (including the
    // approve/reject prefix) within that limit; authority still rests on the
    // secret hash stored by Rust.
    let capability_id = random_hex(8)?;
    let capability_secret = random_hex(16)?;
    let capability_hash = Sha256::digest(capability_secret.as_bytes());
    let attempt_id = format!("nattempt_{}", Uuid::new_v4().simple());
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let mut events = load_events(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    let projection = fold_events(&events)?;
    let delivery = projection
      .notification_deliveries
      .get(delivery_id)
      .ok_or_else(|| {
        DurableStoreError::Contract(format!("Unknown notification delivery {delivery_id:?}."))
      })?;
    let request = projection
      .approval_requests
      .get(&delivery.approval_id)
      .ok_or_else(|| {
        DurableStoreError::Contract("Notification delivery has no approval request.".to_string())
      })?;
    if projection.status != RunStatus::Waiting
      || !matches!(request.status, ApprovalRequestStatus::Waiting)
    {
      return Err(DurableStoreError::Contract(
        "Notification delivery may start only while its approval is waiting.".to_string(),
      ));
    }
    let attempt = next_notification_attempt(&delivery.status, now)?.ok_or_else(|| {
      DurableStoreError::Contract(
        "Notification delivery is not ready for another attempt.".to_string(),
      )
    })?;
    let approval = workflow.approval(&delivery.approval_id).ok_or_else(|| {
      DurableStoreError::Contract(
        "Notification delivery references an unknown compiled approval.".to_string(),
      )
    })?;
    let definition = approval
      .notifications
      .iter()
      .find(|item| item.delivery_id == delivery_id)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "Notification delivery is absent from its compiled approval.".to_string(),
        )
      })?;
    let idempotency_key = notification_idempotency_key(run_id, &delivery.request_id, delivery_id);
    let payload =
      RunEventPayload::NotificationDeliveryAttemptStarted(NotificationDeliveryAttemptStartedData {
        approval_id: delivery.approval_id.clone(),
        request_id: delivery.request_id.clone(),
        delivery_id: delivery_id.to_string(),
        attempt,
        attempt_id: attempt_id.clone(),
        idempotency_key: idempotency_key.clone(),
      });
    validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
      .map_err(DurableStoreError::Contract)?;
    let event_schema_version = projection.event_schema_version.ok_or_else(|| {
      DurableStoreError::Contract("Notification delivery requires run_started.".to_string())
    })?;
    append_to_history(
      &transaction,
      &mut events,
      run_id,
      generated_event_id(),
      now,
      event_schema_version,
      payload,
    )?;
    let credential_expires_at = approval_credential_expiry(now, request.expires_at)?;
    transaction.execute(
      "INSERT INTO woml_notification_capabilities(
         capability_id, secret_hash, attempt_id, request_id, run_id, approval_id,
         delivery_id, issued_at, credential_expires_at
       ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
      params![
        capability_id,
        capability_hash.as_slice(),
        attempt_id,
        delivery.request_id,
        run_id,
        delivery.approval_id,
        delivery_id,
        now.to_rfc3339(),
        credential_expires_at.to_rfc3339(),
      ],
    )?;
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    transaction.commit()?;
    Ok(NotificationDeliveryWork {
      run_id: run_id.to_string(),
      approval_id: delivery.approval_id.clone(),
      request_id: delivery.request_id.clone(),
      delivery_id: delivery_id.to_string(),
      provider: definition.provider.clone(),
      destination: definition.destination.clone(),
      credentials: definition.credentials.clone(),
      provider_id: definition.provider_id.clone(),
      message: definition.message.clone(),
      template_name: definition.template_name.clone(),
      language: definition.language.clone(),
      attempt,
      attempt_id,
      idempotency_key,
      decision_capability: format!("ncap_{capability_id}.{capability_secret}"),
    })
  }

  pub fn complete_notification_delivery(
    &mut self,
    work: &NotificationDeliveryWork,
    result: NotificationProviderDeliveryResult,
    now: DateTime<Utc>,
  ) -> Result<RunProjection, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut events = load_events(&transaction, &work.run_id)?;
    let workflow = definition_for_run(&transaction, &work.run_id)?;
    let binding = run_binding_in_transaction(&transaction, &work.run_id)?;
    let projection = fold_events(&events)?;
    let delivery = projection
      .notification_deliveries
      .get(&work.delivery_id)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "Notification result references an unknown delivery.".to_string(),
        )
      })?;
    if !matches!(&delivery.status, NotificationDeliveryStatus::AttemptStarted { attempt, attempt_id, idempotency_key } if *attempt == work.attempt && attempt_id == &work.attempt_id && idempotency_key == &work.idempotency_key)
    {
      return Err(DurableStoreError::Contract(
        "Notification result does not match the active durable attempt.".to_string(),
      ));
    }
    let payload = match result {
      NotificationProviderDeliveryResult::Succeeded(provider_message) => {
        RunEventPayload::NotificationDeliverySucceeded(NotificationDeliverySucceededData {
          approval_id: work.approval_id.clone(),
          request_id: work.request_id.clone(),
          delivery_id: work.delivery_id.clone(),
          attempt: work.attempt,
          attempt_id: work.attempt_id.clone(),
          provider_message,
        })
      }
      NotificationProviderDeliveryResult::Failed(failure) => {
        let final_ = !failure.retryable
          || work.attempt >= 3
          || matches!(failure.kind.as_str(), "delivery_ambiguous" | "host_crashed");
        RunEventPayload::NotificationDeliveryFailed(NotificationDeliveryFailedData {
          approval_id: work.approval_id.clone(),
          request_id: work.request_id.clone(),
          delivery_id: work.delivery_id.clone(),
          attempt: work.attempt,
          attempt_id: work.attempt_id.clone(),
          final_,
          failure,
        })
      }
    };
    validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
      .map_err(DurableStoreError::Contract)?;
    let version = projection
      .event_schema_version
      .unwrap_or(RUN_EVENT_SCHEMA_VERSION_V5);
    append_to_history(
      &transaction,
      &mut events,
      &work.run_id,
      generated_event_id(),
      now,
      version,
      payload,
    )?;
    let mut projection = fold_events(&events)?;
    if notification_request_all_failed(&workflow, &projection, &work.approval_id, &work.request_id)
    {
      let failed_delivery_ids = workflow
        .approval(&work.approval_id)
        .unwrap()
        .notifications
        .into_iter()
        .map(|delivery| delivery.delivery_id)
        .collect::<Vec<_>>();
      let payload = RunEventPayload::RunFailed(RunFailedData::V5(RunFailedDataV5::Notification {
        approval_id: work.approval_id.clone(),
        request_id: work.request_id.clone(),
        failed_delivery_ids,
        failure: NotificationRunFailure {
          kind: "all_deliveries_failed".to_string(),
          code: "WOML_NOTIFICATION_DELIVERY_FAILED".to_string(),
          message: "Every configured approval notification delivery failed.".to_string(),
        },
      }));
      append_to_history(
        &transaction,
        &mut events,
        &work.run_id,
        generated_event_id(),
        now,
        version,
        payload,
      )?;
      projection = fold_events(&events)?;
    }
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    transaction.commit()?;
    Ok(projection)
  }

  pub fn dispatch_ready_notifications(
    &mut self,
    run_id: &str,
    now: DateTime<Utc>,
    adapter: &mut dyn NotificationProviderAdapter,
  ) -> Result<NotificationDispatchReport, DurableStoreError> {
    let projection = self.projection(run_id)?;
    let ready = projection
      .notification_deliveries
      .values()
      .filter(|delivery| {
        next_notification_attempt(&delivery.status, now)
          .ok()
          .flatten()
          .is_some()
      })
      .map(|delivery| delivery.delivery_id.clone())
      .collect::<Vec<_>>();
    let mut report = NotificationDispatchReport::default();
    for delivery_id in ready {
      let work = self.begin_notification_delivery(run_id, &delivery_id, now)?;
      let result = adapter.deliver(&work);
      let succeeded = matches!(result, NotificationProviderDeliveryResult::Succeeded(_));
      let projection = self.complete_notification_delivery(&work, result, now)?;
      report.attempted += 1;
      if succeeded {
        report.succeeded += 1;
      } else {
        report.failed += 1;
      }
      report.run_failed = projection.status == RunStatus::Failed;
      if report.run_failed {
        break;
      }
    }
    Ok(report)
  }

  pub fn resolve_notification_approval(
    &mut self,
    capability: &str,
    provider_actor_id: &str,
    decision: ApprovalDecision,
    now: DateTime<Utc>,
  ) -> Result<ApprovalDecisionOutcome, DurableStoreError> {
    self.resolve_notification_approval_internal(capability, None, provider_actor_id, decision, now)
  }

  pub fn resolve_notification_approval_from_provider(
    &mut self,
    capability: &str,
    delivery_id: &str,
    provider: &str,
    provider_actor_id: &str,
    decision: ApprovalDecision,
    now: DateTime<Utc>,
  ) -> Result<ApprovalDecisionOutcome, DurableStoreError> {
    self.resolve_notification_approval_internal(
      capability,
      Some((delivery_id, provider)),
      provider_actor_id,
      decision,
      now,
    )
  }

  fn resolve_notification_approval_internal(
    &mut self,
    capability: &str,
    provider_identity: Option<(&str, &str)>,
    provider_actor_id: &str,
    decision: ApprovalDecision,
    now: DateTime<Utc>,
  ) -> Result<ApprovalDecisionOutcome, DurableStoreError> {
    let (capability_id, secret) = parse_notification_capability(capability)?;
    let candidate_hash = Sha256::digest(secret.as_bytes());
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let row: Option<(Vec<u8>, String, String, String, String, String)> = transaction
      .query_row(
        "SELECT secret_hash, request_id, run_id, approval_id, delivery_id, credential_expires_at
       FROM woml_notification_capabilities WHERE capability_id = ?1",
        [capability_id],
        |row| {
          Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
          ))
        },
      )
      .optional()?;
    let Some((stored_hash, request_id, run_id, approval_id, delivery_id, expires_at)) = row else {
      return Err(DurableStoreError::InvalidApprovalToken);
    };
    if stored_hash.len() != 32 || candidate_hash.as_slice().ct_eq(&stored_hash).unwrap_u8() != 1 {
      return Err(DurableStoreError::InvalidApprovalToken);
    }
    if now >= parse_stored_timestamp(&expires_at)? {
      return Err(DurableStoreError::ExpiredApprovalToken);
    }
    let mut events = load_events(&transaction, &run_id)?;
    let workflow = definition_for_run(&transaction, &run_id)?;
    let binding = run_binding_in_transaction(&transaction, &run_id)?;
    let projection = fold_events(&events)?;
    if matches!(
      projection.status,
      RunStatus::Cancelling | RunStatus::Finalizing
    ) || projection.business_outcome == Some(crate::event::BusinessOutcome::Cancelled)
    {
      return Err(DurableStoreError::ApprovalExpired);
    }
    let request = projection
      .approval_requests
      .get(&approval_id)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "Notification capability references an unknown approval.".to_string(),
        )
      })?;
    if request.request_id != request_id {
      return Err(DurableStoreError::Contract(
        "Notification capability does not match its request.".to_string(),
      ));
    }
    if let ApprovalRequestStatus::Resolved {
      resolution:
        ApprovalResolution::Decision {
          decision: existing,
          source: ApprovalDecisionSource::Human,
        },
      resolved_at,
    } = &request.status
    {
      if *existing == decision {
        return Ok(ApprovalDecisionOutcome {
          status: ApprovalDecisionOutcomeStatus::AlreadyResolved,
          run_id,
          approval_id,
          request_id,
          decision,
          source: ApprovalDecisionSource::Human,
          decided_at: *resolved_at,
        });
      }
      return Err(DurableStoreError::ApprovalDecisionConflict);
    }
    if !matches!(request.status, ApprovalRequestStatus::Waiting) {
      return Err(DurableStoreError::ApprovalExpired);
    }
    let delivery = projection
      .notification_deliveries
      .get(&delivery_id)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "Notification capability references an unknown delivery.".to_string(),
        )
      })?;
    if provider_identity.is_some_and(|(expected_delivery_id, expected_provider)| {
      expected_delivery_id != delivery_id || expected_provider != delivery.provider
    }) {
      return Err(DurableStoreError::InvalidApprovalToken);
    }
    if !matches!(
      delivery.status,
      NotificationDeliveryStatus::Succeeded { .. }
    ) {
      return Err(DurableStoreError::Contract(
        "Only a successfully delivered notification can resolve approval.".to_string(),
      ));
    }
    let mut payloads = vec![
      RunEventPayload::NotificationDecisionAccepted(
        crate::event::NotificationDecisionAcceptedData {
          approval_id: approval_id.clone(),
          request_id: request_id.clone(),
          delivery_id: delivery_id.clone(),
          provider: delivery.provider.clone(),
          provider_actor_id: provider_actor_id.to_string(),
          decision,
        },
      ),
      RunEventPayload::ApprovalResolved(ApprovalResolvedData::basic(
        approval_id.clone(),
        request_id.clone(),
        ApprovalResolution::Decision {
          decision,
          source: ApprovalDecisionSource::Human,
        },
      )),
    ];
    let resolution = match decision {
      ApprovalDecision::Approved => crate::event::NotificationResolution::Approved,
      ApprovalDecision::Rejected => crate::event::NotificationResolution::Rejected,
    };
    for delivered in projection.notification_deliveries.values().filter(|item| {
      item.approval_id == approval_id
        && item.request_id == request_id
        && matches!(
          item.provider.as_str(),
          "slack" | "telegram" | "discord" | "whatsapp"
        )
        && matches!(item.status, NotificationDeliveryStatus::Succeeded { .. })
    }) {
      payloads.push(RunEventPayload::NotificationMessageUpdateRequested(
        crate::event::NotificationMessageUpdateRequestedData {
          approval_id: approval_id.clone(),
          request_id: request_id.clone(),
          delivery_id: delivered.delivery_id.clone(),
          update_id: format!("nupdate_{}", Uuid::new_v4().simple()),
          resolution,
        },
      ));
    }
    let version = projection
      .event_schema_version
      .unwrap_or(RUN_EVENT_SCHEMA_VERSION_V5);
    for payload in payloads {
      validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
        .map_err(DurableStoreError::Contract)?;
      append_to_history(
        &transaction,
        &mut events,
        &run_id,
        generated_event_id(),
        now,
        version,
        payload,
      )?;
    }
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    transaction.commit()?;
    Ok(ApprovalDecisionOutcome {
      status: ApprovalDecisionOutcomeStatus::Accepted,
      run_id,
      approval_id,
      request_id,
      decision,
      source: ApprovalDecisionSource::Human,
      decided_at: now,
    })
  }

  pub fn begin_notification_update(
    &mut self,
    run_id: &str,
    delivery_id: &str,
    now: DateTime<Utc>,
  ) -> Result<NotificationUpdateWork, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut events = load_events(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    let projection = fold_events(&events)?;
    let update = projection
      .notification_updates
      .get(delivery_id)
      .ok_or_else(|| {
        DurableStoreError::Contract("Unknown notification message update.".to_string())
      })?;
    let attempt = next_notification_update_attempt(&update.status, now)?.ok_or_else(|| {
      DurableStoreError::Contract("Notification message update is not ready.".to_string())
    })?;
    let delivery = projection
      .notification_deliveries
      .get(delivery_id)
      .ok_or_else(|| {
        DurableStoreError::Contract("Notification update has no delivery.".to_string())
      })?;
    let NotificationDeliveryStatus::Succeeded {
      provider_message, ..
    } = &delivery.status
    else {
      return Err(DurableStoreError::Contract(
        "Notification update requires a successful message.".to_string(),
      ));
    };
    let approval = workflow.approval(&update.approval_id).ok_or_else(|| {
      DurableStoreError::Contract("Notification update has no compiled approval.".to_string())
    })?;
    let definition = approval
      .notifications
      .iter()
      .find(|item| item.delivery_id == delivery_id)
      .ok_or_else(|| {
        DurableStoreError::Contract("Notification update has no compiled delivery.".to_string())
      })?;
    let attempt_id = format!("nattempt_{}", Uuid::new_v4().simple());
    let idempotency_key = notification_update_idempotency_key(
      run_id,
      &update.request_id,
      delivery_id,
      &update.update_id,
    );
    let payload = RunEventPayload::NotificationMessageUpdateAttemptStarted(
      NotificationMessageUpdateAttemptStartedData {
        approval_id: update.approval_id.clone(),
        request_id: update.request_id.clone(),
        delivery_id: delivery_id.to_string(),
        update_id: update.update_id.clone(),
        attempt,
        attempt_id: attempt_id.clone(),
      },
    );
    validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
      .map_err(DurableStoreError::Contract)?;
    let version = projection
      .event_schema_version
      .unwrap_or(RUN_EVENT_SCHEMA_VERSION_V5);
    append_to_history(
      &transaction,
      &mut events,
      run_id,
      generated_event_id(),
      now,
      version,
      payload,
    )?;
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let work = NotificationUpdateWork {
      run_id: run_id.to_string(),
      approval_id: update.approval_id.clone(),
      request_id: update.request_id.clone(),
      delivery_id: delivery_id.to_string(),
      provider: definition.provider.clone(),
      credentials: definition.credentials.clone(),
      provider_message: provider_message.clone(),
      resolution: update.resolution,
      update_id: update.update_id.clone(),
      idempotency_key,
      attempt,
      attempt_id,
    };
    transaction.commit()?;
    Ok(work)
  }

  pub fn complete_notification_update(
    &mut self,
    work: &NotificationUpdateWork,
    result: NotificationProviderUpdateResult,
    now: DateTime<Utc>,
  ) -> Result<RunProjection, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut events = load_events(&transaction, &work.run_id)?;
    let workflow = definition_for_run(&transaction, &work.run_id)?;
    let binding = run_binding_in_transaction(&transaction, &work.run_id)?;
    let projection = fold_events(&events)?;
    let update = projection
      .notification_updates
      .get(&work.delivery_id)
      .ok_or_else(|| {
        DurableStoreError::Contract("Unknown active notification update.".to_string())
      })?;
    if !matches!(&update.status, NotificationMessageUpdateStatus::AttemptStarted { attempt, attempt_id } if *attempt == work.attempt && attempt_id == &work.attempt_id)
    {
      return Err(DurableStoreError::Contract(
        "Notification update result does not match its active attempt.".to_string(),
      ));
    }
    let payload = match result {
      NotificationProviderUpdateResult::Succeeded => {
        RunEventPayload::NotificationMessageUpdated(NotificationMessageUpdatedData {
          approval_id: work.approval_id.clone(),
          request_id: work.request_id.clone(),
          delivery_id: work.delivery_id.clone(),
          update_id: work.update_id.clone(),
          attempt: work.attempt,
          attempt_id: work.attempt_id.clone(),
        })
      }
      NotificationProviderUpdateResult::Failed(failure) => {
        let final_ = !failure.retryable || work.attempt >= 3;
        RunEventPayload::NotificationMessageUpdateFailed(NotificationMessageUpdateFailedData {
          approval_id: work.approval_id.clone(),
          request_id: work.request_id.clone(),
          delivery_id: work.delivery_id.clone(),
          update_id: work.update_id.clone(),
          attempt: work.attempt,
          attempt_id: work.attempt_id.clone(),
          final_,
          failure,
        })
      }
    };
    validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
      .map_err(DurableStoreError::Contract)?;
    let version = projection
      .event_schema_version
      .unwrap_or(RUN_EVENT_SCHEMA_VERSION_V5);
    append_to_history(
      &transaction,
      &mut events,
      &work.run_id,
      generated_event_id(),
      now,
      version,
      payload,
    )?;
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = fold_events(&events)?;
    transaction.commit()?;
    Ok(projection)
  }

  pub fn dispatch_ready_notification_updates(
    &mut self,
    run_id: &str,
    now: DateTime<Utc>,
    adapter: &mut dyn NotificationProviderAdapter,
  ) -> Result<NotificationDispatchReport, DurableStoreError> {
    let projection = self.projection(run_id)?;
    let ready = projection
      .notification_updates
      .values()
      .filter(|update| {
        next_notification_update_attempt(&update.status, now)
          .ok()
          .flatten()
          .is_some()
      })
      .map(|update| update.delivery_id.clone())
      .collect::<Vec<_>>();
    let mut report = NotificationDispatchReport::default();
    for delivery_id in ready {
      let work = self.begin_notification_update(run_id, &delivery_id, now)?;
      let result = adapter.update(&work);
      let succeeded = matches!(result, NotificationProviderUpdateResult::Succeeded);
      self.complete_notification_update(&work, result, now)?;
      report.updates_attempted += 1;
      if succeeded {
        report.updates_succeeded += 1;
      } else {
        report.updates_failed += 1;
      }
    }
    Ok(report)
  }

  pub fn issue_approval_token(
    &mut self,
    run_id: &str,
    approval_id: &str,
    request_id: &str,
    issued_at: DateTime<Utc>,
  ) -> Result<IssuedApprovalToken, DurableStoreError> {
    let projection = self.projection(run_id)?;
    let request = projection
      .approval_requests
      .get(approval_id)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "Cannot issue a credential for an unknown approval request.".to_string(),
        )
      })?;
    if request.request_id != request_id
      || !matches!(request.status, ApprovalRequestStatus::Waiting)
      || projection.status != RunStatus::Waiting
    {
      return Err(DurableStoreError::Contract(
        "Approval credentials may be issued only for the matching unresolved request.".to_string(),
      ));
    }
    let credential_expires_at = approval_credential_expiry(issued_at, request.expires_at)?;

    let token_id = random_hex(16)?;
    let secret = random_hex(32)?;
    let secret_hash = Sha256::digest(secret.as_bytes());
    self.connection.execute(
      "INSERT INTO woml_approval_tokens(
         token_id, secret_hash, request_id, run_id, approval_id,
         issued_at, credential_expires_at
       ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
      params![
        token_id,
        secret_hash.as_slice(),
        request_id,
        run_id,
        approval_id,
        issued_at.to_rfc3339(),
        credential_expires_at.to_rfc3339(),
      ],
    )?;
    Ok(IssuedApprovalToken {
      token: format!("apr_{token_id}.{secret}"),
      token_id,
      request_id: request_id.to_string(),
      run_id: run_id.to_string(),
      approval_id: approval_id.to_string(),
      issued_at,
      credential_expires_at,
    })
  }

  pub fn request_approval_atomically(
    &mut self,
    run_id: &str,
    event_id: impl Into<String>,
    occurred_at: DateTime<Utc>,
    request: ApprovalRequestedData,
  ) -> Result<(RunProjection, IssuedApprovalToken), DurableStoreError> {
    let token_id = random_hex(16)?;
    let secret = random_hex(32)?;
    let secret_hash = Sha256::digest(secret.as_bytes());
    let credential_expires_at = approval_credential_expiry(occurred_at, request.expires_at)?;

    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let mut events = load_events(&transaction, run_id)?;
    let event_schema_version = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "An approval cannot be requested before run_started.".to_string(),
        )
      })?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    let notification_definitions = workflow
      .approval(&request.approval_id)
      .map(|approval| approval.notifications)
      .unwrap_or_default();
    let payload = RunEventPayload::ApprovalRequested(request.clone());
    validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
      .map_err(DurableStoreError::Contract)?;
    append_to_history(
      &transaction,
      &mut events,
      run_id,
      event_id.into(),
      occurred_at,
      event_schema_version,
      payload,
    )?;
    for notification in notification_definitions {
      let payload =
        RunEventPayload::NotificationDeliveryRequested(NotificationDeliveryRequestedData {
          approval_id: request.approval_id.clone(),
          request_id: request.request_id.clone(),
          delivery_id: notification.delivery_id,
          provider: notification.provider,
          destination: notification.destination,
        });
      validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
        .map_err(DurableStoreError::Contract)?;
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        occurred_at,
        event_schema_version,
        payload,
      )?;
    }
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = fold_events(&events)?;
    let folded_request = projection
      .approval_requests
      .get(&request.approval_id)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "The approval request did not enter the durable waiting projection.".to_string(),
        )
      })?;
    if folded_request.request_id != request.request_id
      || !matches!(folded_request.status, ApprovalRequestStatus::Waiting)
      || projection.status != RunStatus::Waiting
    {
      return Err(DurableStoreError::Contract(
        "The approval request did not produce the matching waiting state.".to_string(),
      ));
    }

    transaction.execute(
      "INSERT INTO woml_approval_tokens(
         token_id, secret_hash, request_id, run_id, approval_id,
         issued_at, credential_expires_at
       ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
      params![
        token_id,
        secret_hash.as_slice(),
        request.request_id,
        run_id,
        request.approval_id,
        occurred_at.to_rfc3339(),
        credential_expires_at.to_rfc3339(),
      ],
    )?;
    transaction.commit()?;

    Ok((
      projection,
      IssuedApprovalToken {
        token: format!("apr_{token_id}.{secret}"),
        token_id,
        request_id: request.request_id,
        run_id: run_id.to_string(),
        approval_id: request.approval_id,
        issued_at: occurred_at,
        credential_expires_at,
      },
    ))
  }

  pub fn register_durable_wait_atomically(
    &mut self,
    run_id: &str,
    event_id: impl Into<String>,
    occurred_at: DateTime<Utc>,
    registration: crate::WaitRegisteredData,
  ) -> Result<RunProjection, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let mut events = load_events(&transaction, run_id)?;
    let event_schema_version = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "A durable wait cannot be registered before run_started.".to_string(),
        )
      })?;
    if event_schema_version != crate::RUN_EVENT_SCHEMA_VERSION_V16 {
      return Err(DurableStoreError::Contract(
        "Durable wait registration requires Event v16.".to_string(),
      ));
    }
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    let payload = RunEventPayload::WaitRegistered(registration.clone());
    validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
      .map_err(DurableStoreError::Contract)?;
    let event_id = event_id.into();
    append_to_history(
      &transaction,
      &mut events,
      run_id,
      event_id,
      occurred_at,
      event_schema_version,
      payload,
    )?;
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = fold_events(&events)?;
    if projection.status != RunStatus::Waiting
      || !matches!(
        projection.durable_waits.get(&registration.wait_id),
        Some(wait) if wait.status == crate::DurableWaitStatus::Registered
      )
    {
      return Err(DurableStoreError::Contract(
        "The wait registration did not produce the matching durable waiting state.".to_string(),
      ));
    }
    let kind = match registration.kind {
      crate::DurableWaitKind::Duration => "duration",
      crate::DurableWaitKind::Until => "until",
      crate::DurableWaitKind::Signal => "signal",
    };
    let signals_json = registration
      .signals
      .as_ref()
      .map(serde_json::to_string)
      .transpose()?;
    transaction.execute(
      "INSERT INTO woml_durable_waits(
         run_id, wait_id, iteration_identity, node_id, kind, registered_at,
         deadline, signals_json, correlation_digest, state
       ) VALUES (?1, ?2, '', ?3, ?4, ?5, ?6, ?7, ?8, 'registered')",
      params![
        run_id,
        registration.wait_id,
        registration.node_id,
        kind,
        registration.registered_at.to_rfc3339(),
        registration.deadline.map(|value| value.to_rfc3339()),
        signals_json,
        registration.correlation_digest,
      ],
    )?;
    if registration.kind == crate::DurableWaitKind::Signal {
      let signals = registration.signals.as_deref().unwrap_or_default();
      let correlation_digest = registration
        .correlation_digest
        .as_deref()
        .unwrap_or_default();
      let correlation = match workflow.durable_wait(&registration.wait_id) {
        Some(crate::CompiledWait::Signal { correlation, .. }) => {
          resolve_signal_correlation(correlation, &projection.context).ok_or_else(|| {
            DurableStoreError::Contract(
              "The registered signal wait has no resolvable correlation value.".to_string(),
            )
          })?
        }
        _ => {
          return Err(DurableStoreError::Contract(
            "The registered signal wait has no compiled signal descriptor.".to_string(),
          ));
        }
      };
      let early: Option<(String, String, String, String, String, String)> = transaction
        .prepare(
          "SELECT signal_id, signal, received_at, idempotency_digest,
                  payload_json, authentication_class
           FROM woml_external_signals
           WHERE run_id = ?1 AND wait_id = ?2 AND iteration_identity = ''
             AND correlation_digest = ?3 AND consumed_at IS NULL
           ORDER BY received_at, signal_id",
        )?
        .query_map(
          params![run_id, registration.wait_id, correlation_digest],
          |row| {
            Ok((
              row.get::<_, String>(0)?,
              row.get::<_, String>(1)?,
              row.get::<_, String>(2)?,
              row.get::<_, String>(3)?,
              row.get::<_, String>(4)?,
              row.get::<_, String>(5)?,
            ))
          },
        )?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .find(|(_, signal, _, _, _, _)| signals.iter().any(|accepted| accepted == signal));
      if let Some((signal_id, signal, received_at, idempotency, payload, authentication_class)) =
        early
      {
        let received_at = parse_stored_timestamp(&received_at)?;
        let payload: Value = serde_json::from_str(&payload)?;
        let projection = append_signal_resolution(
          &transaction,
          &workflow,
          &binding.definition_hash,
          &mut events,
          run_id,
          &registration.wait_id,
          &signal_id,
          &signal,
          &correlation,
          &idempotency,
          &authentication_class,
          received_at,
          registration.registered_at,
          payload,
        )?;
        transaction.commit()?;
        return Ok(projection);
      }
    }
    transaction.commit()?;
    Ok(projection)
  }

  pub fn register_lock_waiter_atomically(
    &mut self,
    run_id: &str,
    lock_id: &str,
    key: &str,
    execution_identity: &str,
    registered_at: DateTime<Utc>,
    deadline: DateTime<Utc>,
  ) -> Result<RunProjection, DurableStoreError> {
    self.register_lock_waiter_atomically_scoped(
      run_id,
      lock_id,
      key,
      execution_identity,
      None,
      registered_at,
      deadline,
    )
  }

  #[allow(clippy::too_many_arguments)]
  pub fn register_lock_waiter_atomically_scoped(
    &mut self,
    run_id: &str,
    lock_id: &str,
    key: &str,
    execution_identity: &str,
    iteration: Option<ForEachIterationScope>,
    registered_at: DateTime<Utc>,
    deadline: DateTime<Utc>,
  ) -> Result<RunProjection, DurableStoreError> {
    if !valid_lock_key(key) || execution_identity.is_empty() || deadline < registered_at {
      return Err(DurableStoreError::Contract(
        "Lock acquisition input is invalid.".to_string(),
      ));
    }
    let input_digest = format!(
      "sha256:{:x}",
      Sha256::digest(format!("{key}\0{}", deadline.to_rfc3339()).as_bytes())
    );
    let waiter_id = format!(
      "waiter_{:x}",
      Sha256::digest(format!("{run_id}\0{execution_identity}").as_bytes())
    );
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let descriptor = workflow.durable_lock(lock_id).ok_or_else(|| {
      DurableStoreError::Contract(
        "Lock acquisition references an unknown Model v18 lock.".to_string(),
      )
    })?;
    let expected_execution_identity = iteration.as_ref().map_or_else(
      || descriptor.acquire_node_id.clone(),
      |scope| {
        format!(
          "{}:{}:{}",
          scope.for_each_id, scope.index, descriptor.acquire_node_id
        )
      },
    );
    if expected_execution_identity != execution_identity {
      return Err(DurableStoreError::Contract(
        "Lock execution identity does not match the compiled acquire node.".to_string(),
      ));
    }
    let mut events = load_events(&transaction, run_id)?;
    let event_schema_version = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| {
        DurableStoreError::Contract("A lock cannot be registered before run_started.".to_string())
      })?;
    if !matches!(
      event_schema_version,
      crate::RUN_EVENT_SCHEMA_VERSION_V17
        | crate::RUN_EVENT_SCHEMA_VERSION_V18
        | crate::RUN_EVENT_SCHEMA_VERSION_V19
        | crate::RUN_EVENT_SCHEMA_VERSION_V20
    ) {
      return Err(DurableStoreError::Contract(
        "Durable locks require Event v17.".to_string(),
      ));
    }
    let existing: Option<(String, String, String)> = transaction.query_row(
      "SELECT key, status, input_digest FROM woml_lock_waiters WHERE run_id = ?1 AND node_execution_id = ?2",
      params![run_id, execution_identity], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).optional()?;
    if let Some((stored_key, _, stored_digest)) = existing {
      if stored_key != key || stored_digest != input_digest {
        return Err(DurableStoreError::LockIdempotencyConflict);
      }
    } else {
      transaction.execute(
        "INSERT INTO woml_lock_resources(key, fencing_high_water, updated_at) VALUES (?1, 0, ?2)
         ON CONFLICT(key) DO NOTHING",
        params![key, registered_at.to_rfc3339()],
      )?;
      ensure_lock_wait_capacity(&transaction, key)?;
      let sequence: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(sequence), 0) + 1 FROM woml_lock_waiters WHERE key = ?1",
        [key],
        |row| row.get(0),
      )?;
      transaction.execute(
        "INSERT INTO woml_lock_waiters(waiter_id, key, run_id, node_execution_id, sequence, registered_at, deadline, status, input_digest)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'waiting', ?8)",
        params![waiter_id, key, run_id, execution_identity, sequence, registered_at.to_rfc3339(), deadline.to_rfc3339(), input_digest],
      )?;
      append_to_history_scoped(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        registered_at,
        event_schema_version,
        iteration,
        RunEventPayload::LockWaitRegistered(crate::LockWaitRegisteredData {
          lock_id: lock_id.to_string(),
          key: key.to_string(),
          waiter_sequence: u64::try_from(sequence).map_err(|_| {
            DurableStoreError::LockHistoryInvalid(
              "waiter sequence exceeded the public contract".to_string(),
            )
          })?,
          registered_at,
          deadline,
          execution_identity: execution_identity.to_string(),
        }),
      )?;
    }
    grant_oldest_lock_waiter(&transaction, key, registered_at)?;
    let projection = fold_events(&load_events(&transaction, run_id)?)?;
    transaction.commit()?;
    Ok(projection)
  }

  /// Registers or reattaches one callback-scoped lease request. Programmatic
  /// sessions share the exact same resource row, fair queue, and fencing
  /// high-water as markup locks, but they deliberately do not append workflow
  /// control-flow events: the surrounding capability operation is their
  /// durable attempt record.
  #[allow(clippy::too_many_arguments)]
  pub fn acquire_programmatic_lock_atomically(
    &mut self,
    run_id: &str,
    node_id: &str,
    invocation_id: &str,
    session_id: &str,
    key: &str,
    acquire_timeout_ms: u64,
    lease_ms: u64,
    registered_at: DateTime<Utc>,
  ) -> Result<ProgrammaticLockAcquire, DurableStoreError> {
    validate_programmatic_lock_input(
      node_id,
      invocation_id,
      session_id,
      key,
      acquire_timeout_ms,
      lease_ms,
    )?;
    let deadline = registered_at
      + chrono::Duration::milliseconds(i64::try_from(acquire_timeout_ms).map_err(|_| {
        DurableStoreError::Contract(
          "Lock acquisition timeout exceeds the runtime clock.".to_string(),
        )
      })?);
    let execution_identity = programmatic_lock_execution_identity(session_id, lease_ms);
    let input_digest = format!(
      "sha256:{:x}",
      Sha256::digest(
        format!("{key}\0{node_id}\0{invocation_id}\0{acquire_timeout_ms}\0{lease_ms}").as_bytes()
      )
    );
    let waiter_id = format!(
      "waiter_{:x}",
      Sha256::digest(format!("{run_id}\0{execution_identity}").as_bytes())
    );
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    if lock_descriptor_for_protected_node(&workflow, node_id).is_some() {
      return Err(DurableStoreError::Contract(
        "services.locks.withLease() cannot run inside a markup <lock> scope.".to_string(),
      ));
    }
    let projection = fold_events(&load_events(&transaction, run_id)?)?;
    let active = projection.attempts.iter().any(|attempt| {
      attempt.identity.node_id == node_id
        && attempt.identity.invocation_id == invocation_id
        && attempt.status == AttemptStatus::Started
    }) || projection.for_each.values().any(|loop_state| {
      loop_state.executions.values().any(|execution| {
        execution.attempts.iter().any(|attempt| {
          attempt.identity.node_id == node_id
            && attempt.identity.invocation_id == invocation_id
            && attempt.status == AttemptStatus::Started
        })
      })
    });
    if !active {
      return Err(DurableStoreError::Contract(
        "Programmatic lock acquisition requires its exact active script attempt.".to_string(),
      ));
    }
    let existing: Option<(String, String)> = transaction
      .query_row(
        "SELECT key, input_digest FROM woml_lock_waiters WHERE run_id = ?1 AND node_execution_id = ?2",
        params![run_id, execution_identity],
        |row| Ok((row.get(0)?, row.get(1)?)),
      )
      .optional()?;
    if let Some((stored_key, stored_digest)) = existing {
      if stored_key != key || stored_digest != input_digest {
        return Err(DurableStoreError::LockIdempotencyConflict);
      }
    } else {
      transaction.execute(
        "INSERT INTO woml_lock_resources(key, fencing_high_water, updated_at) VALUES (?1, 0, ?2)
         ON CONFLICT(key) DO NOTHING",
        params![key, registered_at.to_rfc3339()],
      )?;
      ensure_lock_wait_capacity(&transaction, key)?;
      let sequence: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(sequence), 0) + 1 FROM woml_lock_waiters WHERE key = ?1",
        [key],
        |row| row.get(0),
      )?;
      transaction.execute(
        "INSERT INTO woml_lock_waiters(waiter_id, key, run_id, node_execution_id, sequence, registered_at, deadline, status, input_digest)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'waiting', ?8)",
        params![waiter_id, key, run_id, execution_identity, sequence, registered_at.to_rfc3339(), deadline.to_rfc3339(), input_digest],
      )?;
    }
    grant_oldest_lock_waiter(&transaction, key, registered_at)?;
    let state = programmatic_lock_state_in_transaction(
      &transaction,
      run_id,
      session_id,
      lease_ms,
      registered_at,
    )?;
    transaction.commit()?;
    Ok(state)
  }

  pub fn programmatic_lock_state(
    &mut self,
    run_id: &str,
    session_id: &str,
    lease_ms: u64,
    now: DateTime<Utc>,
  ) -> Result<ProgrammaticLockAcquire, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let identity = programmatic_lock_execution_identity(session_id, lease_ms);
    let waiting: Option<(String, String)> = transaction
      .query_row(
        "SELECT key, deadline FROM woml_lock_waiters WHERE run_id = ?1 AND node_execution_id = ?2",
        params![run_id, identity],
        |row| Ok((row.get(0)?, row.get(1)?)),
      )
      .optional()?;
    if let Some((key, deadline)) = &waiting {
      grant_oldest_lock_waiter(&transaction, key, now)?;
      let status: Option<String> = transaction
        .query_row(
          "SELECT status FROM woml_lock_waiters WHERE run_id = ?1 AND node_execution_id = ?2",
          params![run_id, identity],
          |row| row.get(0),
        )
        .optional()?;
      if status.as_deref() == Some("waiting") && parse_stored_timestamp(deadline)? <= now {
        transaction.execute(
          "UPDATE woml_lock_waiters SET status = 'timed_out'
           WHERE run_id = ?1 AND node_execution_id = ?2 AND status = 'waiting'",
          params![run_id, identity],
        )?;
        grant_oldest_lock_waiter(&transaction, key, now)?;
      }
    }
    let state =
      programmatic_lock_state_in_transaction(&transaction, run_id, session_id, lease_ms, now)?;
    transaction.commit()?;
    Ok(state)
  }

  pub fn renew_programmatic_lock_atomically(
    &mut self,
    run_id: &str,
    session_id: &str,
    lease_ms: u64,
    now: DateTime<Utc>,
  ) -> Result<LockLeaseMaintenance, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let identity = programmatic_lock_execution_identity(session_id, lease_ms);
    let owner: Option<(String, String, String)> = transaction
      .query_row(
        "SELECT resources.key, resources.owner_lease_id, resources.expires_at
         FROM woml_lock_resources AS resources
         WHERE resources.owner_run_id = ?1 AND resources.owner_node_execution_id = ?2",
        params![run_id, identity],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
      )
      .optional()?;
    let Some((key, lease_id, expires_at)) = owner else {
      transaction.commit()?;
      return Ok(LockLeaseMaintenance::NotHeld);
    };
    let stored_expiry = parse_stored_timestamp(&expires_at)?;
    if stored_expiry <= now {
      expire_lock_owner(&transaction, &key, now)?;
      grant_oldest_lock_waiter(&transaction, &key, now)?;
      transaction.commit()?;
      return Ok(LockLeaseMaintenance::Lost);
    }
    let requested_expiry = now
      + chrono::Duration::milliseconds(i64::try_from(lease_ms).map_err(|_| {
        DurableStoreError::Contract("Lock lease exceeds the runtime clock.".to_string())
      })?);
    let next_expiry = requested_expiry.max(stored_expiry);
    let changed = transaction.execute(
      "UPDATE woml_lock_resources SET expires_at = ?1, updated_at = ?2
       WHERE key = ?3 AND owner_run_id = ?4 AND owner_node_execution_id = ?5
         AND owner_lease_id = ?6 AND expires_at = ?7",
      params![
        next_expiry.to_rfc3339(),
        now.to_rfc3339(),
        key,
        run_id,
        identity,
        lease_id,
        expires_at
      ],
    )?;
    transaction.commit()?;
    Ok(if changed == 1 {
      LockLeaseMaintenance::Renewed
    } else {
      LockLeaseMaintenance::Lost
    })
  }

  pub fn release_programmatic_lock_atomically(
    &mut self,
    run_id: &str,
    session_id: &str,
    lease_ms: u64,
    released_at: DateTime<Utc>,
  ) -> Result<bool, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let identity = programmatic_lock_execution_identity(session_id, lease_ms);
    let key: Option<String> = transaction
      .query_row(
        "SELECT key FROM woml_lock_waiters WHERE run_id = ?1 AND node_execution_id = ?2",
        params![run_id, identity],
        |row| row.get(0),
      )
      .optional()?;
    let Some(key) = key else {
      transaction.commit()?;
      return Ok(false);
    };
    grant_oldest_lock_waiter(&transaction, &key, released_at)?;
    let changed = transaction.execute(
      "UPDATE woml_lock_resources SET owner_lease_id = NULL, owner_run_id = NULL,
         owner_node_execution_id = NULL, owner_fencing_token = NULL, acquired_at = NULL,
         expires_at = NULL, updated_at = ?1
       WHERE key = ?2 AND owner_run_id = ?3 AND owner_node_execution_id = ?4",
      params![released_at.to_rfc3339(), key, run_id, identity],
    )?;
    if changed == 1 {
      transaction.execute(
        "UPDATE woml_lock_waiters SET status = 'released'
         WHERE run_id = ?1 AND node_execution_id = ?2 AND status = 'held'",
        params![run_id, identity],
      )?;
      grant_oldest_lock_waiter(&transaction, &key, released_at)?;
    }
    transaction.commit()?;
    Ok(changed == 1)
  }

  pub fn cancel_programmatic_lock_waiter_atomically(
    &mut self,
    run_id: &str,
    session_id: &str,
    lease_ms: u64,
    cancelled_at: DateTime<Utc>,
  ) -> Result<bool, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let identity = programmatic_lock_execution_identity(session_id, lease_ms);
    let key: Option<String> = transaction
      .query_row(
        "SELECT key FROM woml_lock_waiters WHERE run_id = ?1 AND node_execution_id = ?2",
        params![run_id, identity],
        |row| row.get(0),
      )
      .optional()?;
    let changed = transaction.execute(
      "UPDATE woml_lock_waiters SET status = 'cancelled'
       WHERE run_id = ?1 AND node_execution_id = ?2 AND status = 'waiting'",
      params![run_id, identity],
    )?;
    if changed == 1 {
      if let Some(key) = key {
        grant_oldest_lock_waiter(&transaction, &key, cancelled_at)?;
      }
    }
    transaction.commit()?;
    Ok(changed == 1)
  }

  pub fn release_lock_atomically(
    &mut self,
    run_id: &str,
    lock_id: &str,
    result: Value,
    released_at: DateTime<Utc>,
  ) -> Result<RunProjection, DurableStoreError> {
    self.release_lock_atomically_scoped(run_id, lock_id, None, result, released_at)
  }

  pub fn release_lock_atomically_scoped(
    &mut self,
    run_id: &str,
    lock_id: &str,
    iteration: Option<ForEachIterationScope>,
    result: Value,
    released_at: DateTime<Utc>,
  ) -> Result<RunProjection, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let mut events = load_events(&transaction, run_id)?;
    let initial = fold_events(&events)?;
    let initial_lock = iteration
      .as_ref()
      .and_then(|scope| initial.for_each.get(&scope.for_each_id))
      .and_then(|loop_| loop_.executions.get(&iteration.as_ref()?.index))
      .and_then(|execution| execution.locks.get(lock_id))
      .or_else(|| {
        iteration
          .is_none()
          .then(|| initial.locks.get(lock_id))
          .flatten()
      })
      .ok_or_else(|| {
        DurableStoreError::LockHistoryInvalid("release has no projected lock".to_string())
      })?;
    if initial_lock.status != crate::LockStatus::Held {
      return Err(DurableStoreError::LockHistoryInvalid(
        "release requires one held lease".to_string(),
      ));
    }
    let key = initial_lock.key.clone();
    // Expiry wins atomically over a late release. This both records lease loss
    // before clearing the old owner and grants the oldest eligible successor.
    grant_oldest_lock_waiter(&transaction, &key, released_at)?;
    events = load_events(&transaction, run_id)?;
    let projection = fold_events(&events)?;
    let held = iteration
      .as_ref()
      .and_then(|scope| projection.for_each.get(&scope.for_each_id))
      .and_then(|loop_| loop_.executions.get(&iteration.as_ref()?.index))
      .and_then(|execution| execution.locks.get(lock_id))
      .or_else(|| {
        iteration
          .is_none()
          .then(|| projection.locks.get(lock_id))
          .flatten()
      })
      .ok_or_else(|| {
        DurableStoreError::LockHistoryInvalid("release has no projected lock".to_string())
      })?;
    if held.status != crate::LockStatus::Held {
      transaction.commit()?;
      return Err(DurableStoreError::LockHistoryInvalid(
        "release rejected because the lease expired before settlement".to_string(),
      ));
    }
    let lease_id = held.lease_id.clone().ok_or_else(|| {
      DurableStoreError::LockHistoryInvalid("held lock has no lease ID".to_string())
    })?;
    let fencing_token = held.fencing_token.clone().ok_or_else(|| {
      DurableStoreError::LockHistoryInvalid("held lock has no fencing token".to_string())
    })?;
    let changed = transaction.execute(
      "UPDATE woml_lock_resources SET owner_lease_id = NULL, owner_run_id = NULL,
       owner_node_execution_id = NULL, owner_fencing_token = NULL, acquired_at = NULL,
       expires_at = NULL, updated_at = ?1 WHERE key = ?2 AND owner_run_id = ?3
       AND owner_lease_id = ?4 AND CAST(owner_fencing_token AS TEXT) = ?5",
      params![
        released_at.to_rfc3339(),
        held.key,
        run_id,
        lease_id,
        fencing_token
      ],
    )?;
    if changed != 1 {
      return Err(DurableStoreError::LockHistoryInvalid(
        "durable owner differs from projected held lease".to_string(),
      ));
    }
    transaction.execute(
      "UPDATE woml_lock_waiters SET status = 'released' WHERE run_id = ?1 AND node_execution_id = ?2 AND status = 'held'",
      params![run_id, held.execution_identity],
    )?;
    let schema = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| {
        DurableStoreError::LockHistoryInvalid("release history is empty".to_string())
      })?;
    append_to_history_scoped(
      &transaction,
      &mut events,
      run_id,
      generated_event_id(),
      released_at,
      schema,
      iteration,
      RunEventPayload::LockReleased(crate::LockReleasedData {
        lock_id: lock_id.to_string(),
        key: held.key.clone(),
        lease_id,
        fencing_token,
        released_at,
        result,
      }),
    )?;
    grant_oldest_lock_waiter(&transaction, &held.key, released_at)?;
    let projection = fold_events(&events)?;
    transaction.commit()?;
    Ok(projection)
  }

  /// Renews one exact lease or durably fences it when its expiry has already
  /// passed. Renewal is a compact coordination update: it deliberately does
  /// not append an event on every heartbeat.
  pub fn renew_lock_lease_atomically(
    &mut self,
    run_id: &str,
    lock_id: &str,
    now: DateTime<Utc>,
  ) -> Result<LockLeaseMaintenance, DurableStoreError> {
    self.renew_lock_lease_atomically_scoped(run_id, lock_id, None, now)
  }

  pub fn renew_lock_lease_atomically_scoped(
    &mut self,
    run_id: &str,
    lock_id: &str,
    iteration: Option<ForEachIterationScope>,
    now: DateTime<Utc>,
  ) -> Result<LockLeaseMaintenance, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let descriptor = workflow.durable_lock(lock_id).ok_or_else(|| {
      DurableStoreError::Contract("Lock renewal references an unknown Model v18 lock.".to_string())
    })?;
    let events = load_events(&transaction, run_id)?;
    let projection = fold_events(&events)?;
    let held = iteration
      .as_ref()
      .and_then(|scope| projection.for_each.get(&scope.for_each_id))
      .and_then(|loop_| loop_.executions.get(&iteration.as_ref()?.index))
      .and_then(|execution| execution.locks.get(lock_id))
      .or_else(|| {
        iteration
          .is_none()
          .then(|| projection.locks.get(lock_id))
          .flatten()
      });
    let Some(held) = held.filter(|lock| lock.status == crate::LockStatus::Held) else {
      transaction.commit()?;
      return Ok(LockLeaseMaintenance::NotHeld);
    };
    let lease_id = held.lease_id.as_deref().ok_or_else(|| {
      DurableStoreError::LockHistoryInvalid("held lock has no lease ID".to_string())
    })?;
    let fencing_token = held.fencing_token.as_deref().ok_or_else(|| {
      DurableStoreError::LockHistoryInvalid("held lock has no fencing token".to_string())
    })?;
    let stored_expiry: Option<String> = transaction
      .query_row(
        "SELECT expires_at FROM woml_lock_resources
         WHERE key = ?1 AND owner_run_id = ?2 AND owner_node_execution_id = ?3
           AND owner_lease_id = ?4 AND CAST(owner_fencing_token AS TEXT) = ?5",
        params![
          held.key,
          run_id,
          held.execution_identity,
          lease_id,
          fencing_token
        ],
        |row| row.get(0),
      )
      .optional()?;
    let Some(stored_expiry) = stored_expiry else {
      transaction.commit()?;
      return Ok(LockLeaseMaintenance::Lost);
    };
    let parsed_stored_expiry = parse_stored_timestamp(&stored_expiry)?;
    if parsed_stored_expiry <= now {
      expire_lock_owner(&transaction, &held.key, now)?;
      grant_oldest_lock_waiter(&transaction, &held.key, now)?;
      transaction.commit()?;
      return Ok(LockLeaseMaintenance::Lost);
    }
    if projection.business_outcome.is_some() {
      transaction.commit()?;
      return Ok(LockLeaseMaintenance::NotHeld);
    }
    let requested_expiry = now
      + chrono::Duration::milliseconds(i64::try_from(descriptor.lease_ms).map_err(|_| {
        DurableStoreError::Contract("Lock lease exceeds the runtime clock.".to_string())
      })?);
    let next_expiry = requested_expiry.max(parsed_stored_expiry);
    let changed = transaction.execute(
      "UPDATE woml_lock_resources SET expires_at = ?1, updated_at = ?2
       WHERE key = ?3 AND owner_run_id = ?4 AND owner_node_execution_id = ?5
         AND owner_lease_id = ?6 AND CAST(owner_fencing_token AS TEXT) = ?7
         AND expires_at = ?8",
      params![
        next_expiry.to_rfc3339(),
        now.to_rfc3339(),
        held.key,
        run_id,
        held.execution_identity,
        lease_id,
        fencing_token,
        stored_expiry
      ],
    )?;
    transaction.commit()?;
    Ok(if changed == 1 {
      LockLeaseMaintenance::Renewed
    } else {
      LockLeaseMaintenance::Lost
    })
  }

  /// Verifies that a managed effect is still protected by the exact live
  /// lease projected for its node. Expiry is settled before returning false,
  /// so a stale owner cannot race a newer grant through WOML.
  pub fn managed_effect_has_current_lock(
    &mut self,
    run_id: &str,
    node_id: &str,
    invocation_id: &str,
    now: DateTime<Utc>,
  ) -> Result<bool, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let Some(descriptor) = lock_descriptor_for_protected_node(&workflow, node_id) else {
      transaction.commit()?;
      return Ok(true);
    };
    let projection = fold_events(&load_events(&transaction, run_id)?)?;
    let iteration_lock = projection.for_each.values().find_map(|loop_| {
      loop_.executions.values().find_map(|execution| {
        execution
          .attempts
          .iter()
          .any(|attempt| {
            attempt.identity.node_id == node_id
              && attempt.identity.invocation_id == invocation_id
              && attempt.status == crate::projection::AttemptStatus::Started
          })
          .then(|| execution.locks.get(&descriptor.lock_id))
          .flatten()
      })
    });
    let Some(held) = projection
      .locks
      .get(&descriptor.lock_id)
      .or(iteration_lock)
      .filter(|lock| lock.status == crate::LockStatus::Held)
    else {
      transaction.commit()?;
      return Ok(false);
    };
    let lease_id = held.lease_id.as_deref().unwrap_or_default();
    let fencing_token = held.fencing_token.as_deref().unwrap_or_default();
    let expiry: Option<String> = transaction
      .query_row(
        "SELECT expires_at FROM woml_lock_resources
         WHERE key = ?1 AND owner_run_id = ?2 AND owner_node_execution_id = ?3
           AND owner_lease_id = ?4 AND CAST(owner_fencing_token AS TEXT) = ?5",
        params![
          held.key,
          run_id,
          held.execution_identity,
          lease_id,
          fencing_token
        ],
        |row| row.get(0),
      )
      .optional()?;
    let current = expiry
      .as_deref()
      .map(parse_stored_timestamp)
      .transpose()?
      .is_some_and(|expires_at| expires_at > now);
    if !current {
      expire_lock_owner(&transaction, &held.key, now)?;
      grant_oldest_lock_waiter(&transaction, &held.key, now)?;
    }
    transaction.commit()?;
    Ok(current)
  }

  pub fn timeout_lock_waiter_atomically(
    &mut self,
    run_id: &str,
    lock_id: &str,
    resolved_at: DateTime<Utc>,
  ) -> Result<RunProjection, DurableStoreError> {
    self.timeout_lock_waiter_atomically_scoped(run_id, lock_id, None, resolved_at)
  }

  pub fn timeout_lock_waiter_atomically_scoped(
    &mut self,
    run_id: &str,
    lock_id: &str,
    iteration: Option<ForEachIterationScope>,
    resolved_at: DateTime<Utc>,
  ) -> Result<RunProjection, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let mut events = load_events(&transaction, run_id)?;
    let projection = fold_events(&events)?;
    let waiting = iteration
      .as_ref()
      .and_then(|scope| projection.for_each.get(&scope.for_each_id))
      .and_then(|loop_| loop_.executions.get(&iteration.as_ref()?.index))
      .and_then(|execution| execution.locks.get(lock_id))
      .or_else(|| {
        iteration
          .is_none()
          .then(|| projection.locks.get(lock_id))
          .flatten()
      })
      .ok_or_else(|| {
        DurableStoreError::LockHistoryInvalid("timeout has no projected lock waiter".to_string())
      })?;
    if waiting.status == crate::LockStatus::AcquireTimedOut {
      return Ok(projection);
    }
    if waiting.status != crate::LockStatus::Waiting || resolved_at < waiting.deadline {
      return Err(DurableStoreError::LockHistoryInvalid(
        "lock timeout requires an expired waiting acquisition".to_string(),
      ));
    }
    if transaction.execute(
      "UPDATE woml_lock_waiters SET status = 'timed_out'
       WHERE run_id = ?1 AND node_execution_id = ?2 AND status = 'waiting'",
      params![run_id, waiting.execution_identity],
    )? != 1
    {
      return Err(DurableStoreError::LockHistoryInvalid(
        "durable waiter changed before timeout settlement".to_string(),
      ));
    }
    let schema = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| {
        DurableStoreError::LockHistoryInvalid("timeout history is empty".to_string())
      })?;
    append_to_history_scoped(
      &transaction,
      &mut events,
      run_id,
      generated_event_id(),
      resolved_at,
      schema,
      iteration,
      RunEventPayload::LockAcquireTimedOut(crate::LockAcquireTimedOutData {
        lock_id: lock_id.to_string(),
        key: waiting.key.clone(),
        deadline: waiting.deadline,
        resolved_at,
      }),
    )?;
    grant_oldest_lock_waiter(&transaction, &waiting.key, resolved_at)?;
    let projection = fold_events(&events)?;
    transaction.commit()?;
    Ok(projection)
  }

  /// Removes durable waiters during cancellation. Held leases are
  /// intentionally not released here: cancellation history alone does not
  /// prove that supervised protected work has stopped, so an owner that
  /// cannot complete normal release must age out through lease expiry.
  pub fn cancel_lock_waiters_atomically(
    &mut self,
    run_id: &str,
    cancellation_id: &str,
    cancelled_at: DateTime<Utc>,
  ) -> Result<RunProjection, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let mut events = load_events(&transaction, run_id)?;
    let initial = fold_events(&events)?;
    if initial.status != RunStatus::Cancelling
      || initial.cancellation_request_id.as_deref() != Some(cancellation_id)
    {
      return Err(DurableStoreError::LockHistoryInvalid(
        "lock waiter cancellation requires the matching durable run cancellation".to_string(),
      ));
    }
    let schema = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| DurableStoreError::LockHistoryInvalid("lock history is empty".to_string()))?;
    let mut waiting = initial
      .locks
      .values()
      .filter(|lock| lock.status == crate::LockStatus::Waiting)
      .cloned()
      .map(|lock| (lock, None))
      .collect::<Vec<_>>();
    waiting.extend(
      initial
        .for_each
        .iter()
        .flat_map(|(for_each_id, loop_state)| {
          loop_state
            .executions
            .iter()
            .flat_map(move |(index, execution)| {
              execution
                .locks
                .values()
                .filter(|lock| lock.status == crate::LockStatus::Waiting)
                .cloned()
                .map(move |lock| {
                  (
                    lock,
                    Some(ForEachIterationScope {
                      for_each_id: for_each_id.clone(),
                      index: *index,
                    }),
                  )
                })
            })
        }),
    );
    for (lock, iteration) in waiting {
      if transaction.execute(
        "UPDATE woml_lock_waiters SET status = 'cancelled'
         WHERE run_id = ?1 AND node_execution_id = ?2 AND status = 'waiting'",
        params![run_id, lock.execution_identity],
      )? != 1
      {
        return Err(DurableStoreError::LockHistoryInvalid(
          "lock waiter changed before cancellation settlement".to_string(),
        ));
      }
      append_to_history_scoped(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        cancelled_at,
        schema,
        iteration,
        RunEventPayload::LockAcquisitionCancelled(crate::LockAcquisitionCancelledData {
          lock_id: lock.lock_id,
          key: lock.key.clone(),
          cancelled_at,
          cancellation_id: cancellation_id.to_string(),
        }),
      )?;
      grant_oldest_lock_waiter(&transaction, &lock.key, cancelled_at)?;
    }
    let projection = fold_events(&events)?;
    transaction.commit()?;
    Ok(projection)
  }

  pub fn admit_external_signal_atomically(
    &mut self,
    request: ExternalSignalAdmissionRequest,
    known_secrets: &[String],
  ) -> Result<ExternalSignalAdmissionOutcome, DurableStoreError> {
    let idempotency_digest = sha256_prefixed(request.idempotency_key.as_bytes());
    let signal_id = format!(
      "signal_{}",
      &idempotency_digest
        .strip_prefix("sha256:")
        .unwrap_or(&idempotency_digest)[..32]
    );
    let outcome = |status: ExternalSignalAdmissionStatus,
                   resolution: Option<ExternalSignalResolution>,
                   code: Option<&str>| ExternalSignalAdmissionOutcome {
      profile: EXTERNAL_SIGNAL_RESULT_PROFILE,
      status,
      signal_id: signal_id.clone(),
      run_id: request.run_id.clone(),
      wait_id: request.wait_id.clone(),
      resolution,
      code: code.map(str::to_string),
    };
    if request.profile != EXTERNAL_SIGNAL_ADMISSION_PROFILE
      || !valid_external_signal_name(&request.signal)
      || !valid_bounded_identity(&request.run_id, 320)
      || !valid_public_wait_identity(&request.wait_id)
      || !valid_bounded_signal_value(&request.correlation, 256)
      || !valid_bounded_signal_value(&request.idempotency_key, 256)
      || !valid_bounded_signal_value(&request.authentication_class, 64)
      || json_depth(&request.payload) > EXTERNAL_SIGNAL_MAX_PAYLOAD_DEPTH
    {
      return Ok(outcome(
        ExternalSignalAdmissionStatus::Rejected,
        None,
        Some("WOML_SIGNAL_PAYLOAD_INVALID"),
      ));
    }
    let payload_json = canonical_json(&request.payload)?;
    if payload_json.len() > EXTERNAL_SIGNAL_MAX_PAYLOAD_BYTES {
      return Ok(outcome(
        ExternalSignalAdmissionStatus::Rejected,
        None,
        Some("WOML_SIGNAL_PAYLOAD_TOO_LARGE"),
      ));
    }
    if known_secrets.iter().any(|secret| {
      secret.len() >= 8
        && payload_json
          .windows(secret.len())
          .any(|window| window == secret.as_bytes())
    }) {
      return Ok(outcome(
        ExternalSignalAdmissionStatus::Rejected,
        None,
        Some("WOML_SIGNAL_PAYLOAD_SECRET"),
      ));
    }
    let request_digest = sha256_prefixed(&canonical_json(&serde_json::json!({
      "signal": request.signal,
      "runId": request.run_id,
      "waitId": request.wait_id,
      "correlation": request.correlation,
      "payload": request.payload,
    }))?);
    let correlation_digest = sha256_prefixed(request.correlation.as_bytes());
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let prior: Option<(String, String)> = transaction
      .query_row(
        "SELECT request_digest, signal_id FROM woml_signal_idempotency
         WHERE idempotency_digest = ?1",
        [&idempotency_digest],
        |row| Ok((row.get(0)?, row.get(1)?)),
      )
      .optional()?;
    if let Some((prior_digest, prior_signal_id)) = prior {
      let mut duplicate = outcome(
        if prior_digest == request_digest {
          ExternalSignalAdmissionStatus::Duplicate
        } else {
          ExternalSignalAdmissionStatus::IdentityConflict
        },
        None,
        (prior_digest != request_digest).then_some("WOML_SIGNAL_IDENTITY_CONFLICT"),
      );
      duplicate.signal_id = prior_signal_id;
      return Ok(duplicate);
    }
    let Some(binding) = load_run_binding_optional(&transaction, &request.run_id)? else {
      return Ok(outcome(
        ExternalSignalAdmissionStatus::TargetNotFound,
        None,
        Some("WOML_SIGNAL_TARGET_NOT_FOUND"),
      ));
    };
    let workflow = definition_by_hash(&transaction, &binding.definition_hash)?;
    let Some(crate::CompiledWait::Signal {
      signals,
      correlation,
      ..
    }) = workflow.durable_wait(&request.wait_id)
    else {
      return Ok(outcome(
        ExternalSignalAdmissionStatus::TargetNotFound,
        None,
        Some("WOML_SIGNAL_TARGET_NOT_FOUND"),
      ));
    };
    let Some(signal) = signals
      .iter()
      .find(|signal| signal.signal == request.signal)
    else {
      return Ok(outcome(
        ExternalSignalAdmissionStatus::TargetNotFound,
        None,
        Some("WOML_SIGNAL_TARGET_NOT_FOUND"),
      ));
    };
    let mut events = load_events(&transaction, &request.run_id)?;
    let projection = fold_events(&events)?;
    if matches!(
      projection.status,
      RunStatus::Succeeded | RunStatus::Failed | RunStatus::Cancelled | RunStatus::Finalizing
    ) || projection.business_outcome.is_some()
    {
      return Ok(outcome(
        ExternalSignalAdmissionStatus::AlreadyResolved,
        None,
        Some("WOML_SIGNAL_ALREADY_RESOLVED"),
      ));
    }
    let expected_correlation = resolve_signal_correlation(correlation, &projection.context)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "Signal correlation is unavailable before its dominating predecessor completes."
            .to_string(),
        )
      })?;
    if expected_correlation != request.correlation {
      return Ok(outcome(
        ExternalSignalAdmissionStatus::Rejected,
        None,
        Some("WOML_SIGNAL_CORRELATION_MISMATCH"),
      ));
    }
    let validator = jsonschema::draft202012::options()
      .should_validate_formats(true)
      .build(&signal.schema)
      .map_err(|error| {
        DurableStoreError::Contract(format!("Invalid stored signal schema: {error}"))
      })?;
    if !validator.is_valid(&request.payload) {
      return Ok(outcome(
        ExternalSignalAdmissionStatus::Rejected,
        None,
        Some("WOML_SIGNAL_PAYLOAD_INVALID"),
      ));
    }
    let active_wait: Option<(String, Option<String>)> = transaction
      .query_row(
        "SELECT state, correlation_digest FROM woml_durable_waits
         WHERE run_id = ?1 AND wait_id = ?2 AND iteration_identity = ''",
        params![request.run_id, request.wait_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
      )
      .optional()?;
    if active_wait
      .as_ref()
      .is_some_and(|(state, _)| state != "registered")
    {
      return Ok(outcome(
        ExternalSignalAdmissionStatus::AlreadyResolved,
        None,
        Some("WOML_SIGNAL_ALREADY_RESOLVED"),
      ));
    }
    if active_wait
      .as_ref()
      .is_some_and(|(_, digest)| digest.as_deref() != Some(correlation_digest.as_str()))
    {
      return Ok(outcome(
        ExternalSignalAdmissionStatus::Rejected,
        None,
        Some("WOML_SIGNAL_CORRELATION_MISMATCH"),
      ));
    }
    if active_wait.is_none() {
      let pending: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM woml_external_signals
         WHERE run_id = ?1 AND consumed_at IS NULL",
        [&request.run_id],
        |row| row.get(0),
      )?;
      if pending >= i64::try_from(EXTERNAL_SIGNAL_MAX_EARLY_PER_RUN).unwrap_or(64) {
        return Ok(outcome(
          ExternalSignalAdmissionStatus::Rejected,
          None,
          Some("WOML_SIGNAL_EARLY_LIMIT_REACHED"),
        ));
      }
    }
    transaction.execute(
      "INSERT INTO woml_external_signals(
         signal_id, run_id, wait_id, iteration_identity, signal, received_at,
         idempotency_digest, request_digest, correlation_digest,
         payload_json, authentication_class, consumed_at
       ) VALUES (?1, ?2, ?3, '', ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL)",
      params![
        signal_id,
        request.run_id,
        request.wait_id,
        request.signal,
        request.received_at.to_rfc3339(),
        idempotency_digest,
        request_digest,
        correlation_digest,
        String::from_utf8(payload_json).map_err(|_| DurableStoreError::Contract(
          "Canonical signal payload is not UTF-8.".to_string()
        ))?,
        request.authentication_class,
      ],
    )?;
    transaction.execute(
      "INSERT INTO woml_signal_idempotency(
         idempotency_digest, request_digest, signal_id, created_at
       ) VALUES (?1, ?2, ?3, ?4)",
      params![
        idempotency_digest,
        request_digest,
        signal_id,
        request.received_at.to_rfc3339(),
      ],
    )?;
    if active_wait.is_none() {
      transaction.commit()?;
      return Ok(outcome(
        ExternalSignalAdmissionStatus::Accepted,
        Some(ExternalSignalResolution::Early),
        None,
      ));
    }
    append_signal_resolution(
      &transaction,
      &workflow,
      &binding.definition_hash,
      &mut events,
      &request.run_id,
      &request.wait_id,
      &signal_id,
      &request.signal,
      &request.correlation,
      &idempotency_digest,
      &request.authentication_class,
      request.received_at,
      request.received_at,
      request.payload,
    )?;
    transaction.commit()?;
    Ok(outcome(
      ExternalSignalAdmissionStatus::Accepted,
      Some(ExternalSignalResolution::Resolved),
      None,
    ))
  }

  pub fn due_timer_waits(
    &self,
    now: DateTime<Utc>,
    limit: usize,
  ) -> Result<Vec<DurableWaitRecordV1>, DurableStoreError> {
    self.query_due_waits(now, limit, None, false)
  }

  pub(crate) fn due_timer_waits_for_workflows(
    &self,
    now: DateTime<Utc>,
    limit: usize,
    workflow_ids: &HashSet<String>,
  ) -> Result<Vec<DurableWaitRecordV1>, DurableStoreError> {
    if workflow_ids.is_empty() {
      return Ok(Vec::new());
    }
    self.query_due_waits(now, limit, Some(workflow_ids), false)
  }

  pub(crate) fn due_signal_waits_for_workflows(
    &self,
    now: DateTime<Utc>,
    limit: usize,
    workflow_ids: &HashSet<String>,
  ) -> Result<Vec<DurableWaitRecordV1>, DurableStoreError> {
    if workflow_ids.is_empty() {
      return Ok(Vec::new());
    }
    self.query_due_waits(now, limit, Some(workflow_ids), true)
  }

  fn query_due_waits(
    &self,
    now: DateTime<Utc>,
    limit: usize,
    workflow_ids: Option<&HashSet<String>>,
    signal_waits: bool,
  ) -> Result<Vec<DurableWaitRecordV1>, DurableStoreError> {
    if limit == 0 || limit > 1_000 {
      return Err(DurableStoreError::Contract(
        "Due-wait scans require a limit between 1 and 1000.".to_string(),
      ));
    }
    let mut parameters = vec![rusqlite::types::Value::Text(now.to_rfc3339())];
    let workflow_filter = workflow_ids.map(|workflow_ids| {
      let mut workflow_ids = workflow_ids.iter().cloned().collect::<Vec<_>>();
      workflow_ids.sort();
      let placeholders = workflow_ids
        .iter()
        .map(|workflow_id| {
          parameters.push(rusqlite::types::Value::Text(workflow_id.clone()));
          format!("?{}", parameters.len())
        })
        .collect::<Vec<_>>()
        .join(", ");
      format!(
        " AND EXISTS (
           SELECT 1 FROM woml_runs r
           WHERE r.run_id = w.run_id AND r.workflow_id IN ({placeholders})
         )"
      )
    });
    parameters.push(rusqlite::types::Value::Integer(
      i64::try_from(limit).unwrap_or(1_000),
    ));
    let limit_parameter = parameters.len();
    let kind_filter = if signal_waits {
      "w.kind = 'signal'"
    } else {
      "w.kind IN ('duration', 'until')"
    };
    let query = format!(
      "SELECT w.run_id, w.wait_id, w.node_id, w.kind, w.registered_at,
              w.deadline, w.state, w.resolved_at
       FROM woml_durable_waits w
       WHERE w.state = 'registered'
         AND {kind_filter}
         AND w.deadline <= ?1{}
       ORDER BY w.deadline, w.run_id, w.wait_id
       LIMIT ?{limit_parameter}",
      workflow_filter.as_deref().unwrap_or_default()
    );
    let mut statement = self.connection.prepare(&query)?;
    let records = statement
      .query_map(rusqlite::params_from_iter(parameters.iter()), |row| {
        let kind: String = row.get(3)?;
        let registered_at: String = row.get(4)?;
        let deadline: Option<String> = row.get(5)?;
        let resolved_at: Option<String> = row.get(7)?;
        Ok((
          row.get::<_, String>(0)?,
          row.get::<_, String>(1)?,
          row.get::<_, String>(2)?,
          kind,
          registered_at,
          deadline,
          row.get::<_, String>(6)?,
          resolved_at,
        ))
      })?
      .map(|row| {
        let (run_id, wait_id, node_id, kind, registered_at, deadline, state, resolved_at) = row?;
        Ok(DurableWaitRecordV1 {
          run_id,
          wait_id,
          node_id,
          kind: match kind.as_str() {
            "duration" => crate::DurableWaitKind::Duration,
            "until" => crate::DurableWaitKind::Until,
            "signal" => crate::DurableWaitKind::Signal,
            _ => {
              return Err(DurableStoreError::Contract(
                "Stored durable wait has an invalid kind.".to_string(),
              ))
            }
          },
          registered_at: parse_stored_timestamp(&registered_at)?,
          deadline: deadline
            .as_deref()
            .map(parse_stored_timestamp)
            .transpose()?,
          state,
          resolved_at: resolved_at
            .as_deref()
            .map(parse_stored_timestamp)
            .transpose()?,
        })
      })
      .collect();
    records
  }

  pub fn settle_timer_wait(
    &mut self,
    run_id: &str,
    wait_id: &str,
    now: DateTime<Utc>,
  ) -> Result<TimerWaitSettlement, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let row: Option<(String, String, Option<String>, String)> = transaction
      .query_row(
        "SELECT node_id, kind, deadline, state FROM woml_durable_waits
         WHERE run_id = ?1 AND wait_id = ?2 AND iteration_identity = ''",
        params![run_id, wait_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
      )
      .optional()?;
    let Some((node_id, kind, deadline, state)) = row else {
      return Err(DurableStoreError::Contract(
        "Timer settlement references an unknown durable wait.".to_string(),
      ));
    };
    let events = load_events(&transaction, run_id)?;
    if state != "registered" {
      return Ok(TimerWaitSettlement::AlreadyResolved {
        projection: fold_events(&events)?,
      });
    }
    if !matches!(kind.as_str(), "duration" | "until") {
      return Err(DurableStoreError::Contract(
        "Timer settlement requires a duration or until wait.".to_string(),
      ));
    }
    let deadline = deadline
      .as_deref()
      .map(parse_stored_timestamp)
      .transpose()?
      .ok_or_else(|| DurableStoreError::Contract("Timer wait has no deadline.".to_string()))?;
    if now < deadline {
      return Ok(TimerWaitSettlement::NotDue { deadline });
    }
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    let mut events = events;
    let event_schema_version = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| DurableStoreError::Contract("Timer wait has no run history.".to_string()))?;
    let event_id = generated_event_id();
    let result = serde_json::json!({
      "status": "elapsed",
      "scheduledAt": deadline.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
      "resumedAt": now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    });
    let payload = RunEventPayload::WaitSatisfied(crate::WaitSatisfiedData {
      wait_id: wait_id.to_string(),
      winner: crate::DurableWaitWinner::Timer,
      result,
      resolved_at: now,
    });
    validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
      .map_err(DurableStoreError::Contract)?;
    append_to_history(
      &transaction,
      &mut events,
      run_id,
      event_id.clone(),
      now,
      event_schema_version,
      payload,
    )?;
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let changed = transaction.execute(
      "UPDATE woml_durable_waits
       SET state = 'satisfied', resolution_event_id = ?1, resolved_at = ?2
       WHERE run_id = ?3 AND wait_id = ?4 AND iteration_identity = '' AND state = 'registered'",
      params![event_id, now.to_rfc3339(), run_id, wait_id],
    )?;
    if changed != 1 {
      return Ok(TimerWaitSettlement::AlreadyResolved {
        projection: fold_events(&load_events(&transaction, run_id)?)?,
      });
    }
    let projection = fold_events(&events)?;
    if projection.status != RunStatus::Running || !projection.context.steps.contains_key(&node_id) {
      return Err(DurableStoreError::Contract(
        "Timer settlement did not publish the wait result into context.".to_string(),
      ));
    }
    transaction.commit()?;
    Ok(TimerWaitSettlement::Resolved { projection })
  }

  pub fn settle_signal_wait_timeout(
    &mut self,
    run_id: &str,
    wait_id: &str,
    now: DateTime<Utc>,
  ) -> Result<SignalWaitTimeoutSettlement, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    let Some(crate::CompiledWait::Signal { on_timeout, .. }) = workflow.durable_wait(wait_id)
    else {
      return Err(DurableStoreError::Contract(
        "Signal timeout references an unknown signal wait.".to_string(),
      ));
    };
    let row: Option<(Option<String>, String)> = transaction
      .query_row(
        "SELECT deadline, state FROM woml_durable_waits
         WHERE run_id = ?1 AND wait_id = ?2 AND iteration_identity = ''",
        params![run_id, wait_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
      )
      .optional()?;
    let Some((deadline, state)) = row else {
      return Err(DurableStoreError::Contract(
        "Signal timeout references an unregistered wait.".to_string(),
      ));
    };
    let events = load_events(&transaction, run_id)?;
    if state != "registered" {
      return Ok(SignalWaitTimeoutSettlement::AlreadyResolved {
        projection: fold_events(&events)?,
      });
    }
    let deadline = deadline
      .as_deref()
      .map(parse_stored_timestamp)
      .transpose()?
      .ok_or_else(|| DurableStoreError::Contract("Signal wait has no timeout.".to_string()))?;
    if now < deadline {
      return Ok(SignalWaitTimeoutSettlement::NotDue { deadline });
    }
    let mut events = events;
    let event_schema_version = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| DurableStoreError::Contract("Signal wait has no run history.".to_string()))?;
    let behavior = if on_timeout == "continue" {
      crate::DurableWaitTimeoutBehavior::Continue
    } else {
      crate::DurableWaitTimeoutBehavior::Fail
    };
    let resolution_event_id = generated_event_id();
    append_to_history(
      &transaction,
      &mut events,
      run_id,
      resolution_event_id.clone(),
      now,
      event_schema_version,
      RunEventPayload::WaitTimedOut(crate::WaitTimedOutData {
        wait_id: wait_id.to_string(),
        deadline,
        resolved_at: now,
        behavior,
      }),
    )?;
    if behavior == crate::DurableWaitTimeoutBehavior::Fail {
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        now,
        event_schema_version,
        RunEventPayload::RunOutcomeDecided(crate::RunOutcomeDecidedData::Failed {
          failure: crate::LifecycleFailure {
            kind: crate::LifecycleFailureKind::TimedOut,
            code: "WOML_WAIT_TIMED_OUT".to_string(),
            message: format!("Durable wait {wait_id:?} reached its signal timeout."),
          },
        }),
      )?;
      if let Some(hook) = workflow
        .lifecycle_hook_for_event(crate::model::LifecycleEventName::RunFailure)
        .or_else(|| {
          workflow.lifecycle_hook_for_event(crate::model::LifecycleEventName::RunComplete)
        })
      {
        let subject = LifecycleSubject {
          kind: LifecycleSubjectKind::Workflow,
          id: run_id.to_string(),
        };
        append_to_history(
          &transaction,
          &mut events,
          run_id,
          generated_event_id(),
          now,
          event_schema_version,
          RunEventPayload::LifecycleHookRequested(LifecycleHookRequestedData {
            hook_invocation_id: derive_lifecycle_hook_invocation_id(
              run_id,
              &hook.hook_id,
              subject.kind,
              &subject.id,
            ),
            hook_id: hook.hook_id.clone(),
            event: hook.event,
            subject,
          }),
        )?;
      }
    }
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let changed = transaction.execute(
      "UPDATE woml_durable_waits
       SET state = 'timed_out', resolution_event_id = ?1, resolved_at = ?2
       WHERE run_id = ?3 AND wait_id = ?4 AND iteration_identity = '' AND state = 'registered'",
      params![resolution_event_id, now.to_rfc3339(), run_id, wait_id],
    )?;
    if changed != 1 {
      return Ok(SignalWaitTimeoutSettlement::AlreadyResolved {
        projection: fold_events(&load_events(&transaction, run_id)?)?,
      });
    }
    let projection = fold_events(&events)?;
    transaction.commit()?;
    Ok(if behavior == crate::DurableWaitTimeoutBehavior::Continue {
      SignalWaitTimeoutSettlement::Continued { projection }
    } else {
      SignalWaitTimeoutSettlement::Failed { projection }
    })
  }

  pub fn resolve_human_approval(
    &mut self,
    token: &str,
    decision: ApprovalDecision,
    now: DateTime<Utc>,
  ) -> Result<ApprovalDecisionOutcome, DurableStoreError> {
    let (token_id, secret) = parse_approval_token(token)?;
    let candidate_hash = Sha256::digest(secret.as_bytes());
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let row: Option<(Vec<u8>, String, String, String, String)> = transaction
      .query_row(
        "SELECT secret_hash, request_id, run_id, approval_id, credential_expires_at
         FROM woml_approval_tokens WHERE token_id = ?1",
        [token_id],
        |row| {
          Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
          ))
        },
      )
      .optional()?;
    let Some((stored_hash, request_id, run_id, approval_id, credential_expires_at)) = row else {
      return Err(DurableStoreError::InvalidApprovalToken);
    };
    if stored_hash.len() != 32
      || candidate_hash
        .as_slice()
        .ct_eq(stored_hash.as_slice())
        .unwrap_u8()
        != 1
    {
      return Err(DurableStoreError::InvalidApprovalToken);
    }
    let credential_expires_at = parse_stored_timestamp(&credential_expires_at)?;

    let mut events = load_events(&transaction, &run_id)?;
    let workflow = definition_for_run(&transaction, &run_id)?;
    let binding = run_binding_in_transaction(&transaction, &run_id)?;
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = fold_events(&events)?;
    if matches!(
      projection.status,
      RunStatus::Cancelling | RunStatus::Finalizing
    ) || projection.business_outcome == Some(crate::event::BusinessOutcome::Cancelled)
    {
      return Err(DurableStoreError::ApprovalExpired);
    }
    let request = projection
      .approval_requests
      .get(&approval_id)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "Approval credential references an unknown durable request.".to_string(),
        )
      })?;
    if request.request_id != request_id {
      return Err(DurableStoreError::Contract(
        "Approval credential does not match its durable request.".to_string(),
      ));
    }
    match &request.status {
      ApprovalRequestStatus::Resolved {
        resolution:
          ApprovalResolution::Decision {
            decision: existing,
            source: ApprovalDecisionSource::Human,
          },
        resolved_at,
      } if *existing == decision => {
        return Ok(ApprovalDecisionOutcome {
          status: ApprovalDecisionOutcomeStatus::AlreadyResolved,
          run_id,
          approval_id,
          request_id,
          decision,
          source: ApprovalDecisionSource::Human,
          decided_at: *resolved_at,
        });
      }
      ApprovalRequestStatus::Resolved {
        resolution:
          ApprovalResolution::Decision {
            source: ApprovalDecisionSource::Human,
            ..
          },
        ..
      } => return Err(DurableStoreError::ApprovalDecisionConflict),
      ApprovalRequestStatus::Resolved { .. } => {
        return Err(DurableStoreError::ApprovalExpired);
      }
      ApprovalRequestStatus::Waiting => {}
    }
    if request.expires_at.is_some_and(|deadline| now >= deadline) {
      return Err(DurableStoreError::ApprovalExpired);
    }
    if now >= credential_expires_at {
      return Err(DurableStoreError::ExpiredApprovalToken);
    }

    let mut payloads = vec![RunEventPayload::ApprovalResolved(
      ApprovalResolvedData::basic(
        approval_id.clone(),
        request_id.clone(),
        ApprovalResolution::Decision {
          decision,
          source: ApprovalDecisionSource::Human,
        },
      ),
    )];
    let notification_resolution = match decision {
      ApprovalDecision::Approved => crate::event::NotificationResolution::Approved,
      ApprovalDecision::Rejected => crate::event::NotificationResolution::Rejected,
    };
    for delivery in projection
      .notification_deliveries
      .values()
      .filter(|delivery| {
        delivery.approval_id == approval_id
          && delivery.request_id == request_id
          && matches!(
            delivery.provider.as_str(),
            "slack" | "telegram" | "discord" | "whatsapp"
          )
          && matches!(
            delivery.status,
            NotificationDeliveryStatus::Succeeded { .. }
          )
      })
    {
      payloads.push(RunEventPayload::NotificationMessageUpdateRequested(
        crate::event::NotificationMessageUpdateRequestedData {
          approval_id: approval_id.clone(),
          request_id: request_id.clone(),
          delivery_id: delivery.delivery_id.clone(),
          update_id: format!("nupdate_{}", Uuid::new_v4().simple()),
          resolution: notification_resolution,
        },
      ));
    }
    let event_schema_version = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| {
        DurableStoreError::Contract("Approval resolution requires run_started.".to_string())
      })?;
    for payload in payloads {
      validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
        .map_err(DurableStoreError::Contract)?;
      append_to_history(
        &transaction,
        &mut events,
        &run_id,
        generated_event_id(),
        now,
        event_schema_version,
        payload,
      )?;
    }
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    transaction.commit()?;
    Ok(ApprovalDecisionOutcome {
      status: ApprovalDecisionOutcomeStatus::Accepted,
      run_id,
      approval_id,
      request_id,
      decision,
      source: ApprovalDecisionSource::Human,
      decided_at: now,
    })
  }

  pub fn settle_approval_timeout(
    &mut self,
    run_id: &str,
    approval_id: &str,
    now: DateTime<Utc>,
  ) -> Result<ApprovalTimeoutSettlement, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let mut events = load_events(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = fold_events(&events)?;
    let request = projection
      .approval_requests
      .get(approval_id)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "Timeout settlement references an unknown approval request.".to_string(),
        )
      })?;
    if let ApprovalRequestStatus::Resolved {
      resolution,
      resolved_at,
    } = &request.status
    {
      return Ok(ApprovalTimeoutSettlement {
        status: ApprovalTimeoutSettlementStatus::AlreadyResolved,
        run_id: run_id.to_string(),
        approval_id: approval_id.to_string(),
        request_id: request.request_id.clone(),
        resolution: Some(resolution.clone()),
        settled_at: Some(*resolved_at),
      });
    }
    let deadline = request
      .expires_at
      .ok_or(DurableStoreError::ApprovalHasNoDeadline)?;
    if now < deadline {
      return Ok(ApprovalTimeoutSettlement {
        status: ApprovalTimeoutSettlementStatus::NotDue,
        run_id: run_id.to_string(),
        approval_id: approval_id.to_string(),
        request_id: request.request_id.clone(),
        resolution: None,
        settled_at: None,
      });
    }
    let resolution = match request.on_timeout {
      ApprovalTimeoutPolicy::Reject => ApprovalResolution::Decision {
        decision: ApprovalDecision::Rejected,
        source: ApprovalDecisionSource::Timeout,
      },
      ApprovalTimeoutPolicy::Fail => ApprovalResolution::TimeoutFailure,
    };
    let mut payloads = vec![RunEventPayload::ApprovalResolved(
      ApprovalResolvedData::basic(
        approval_id.to_string(),
        request.request_id.clone(),
        resolution.clone(),
      ),
    )];
    let notification_resolution = match &resolution {
      ApprovalResolution::Decision {
        decision: ApprovalDecision::Rejected,
        ..
      } => crate::event::NotificationResolution::Rejected,
      ApprovalResolution::TimeoutFailure => crate::event::NotificationResolution::TimeoutFailed,
      ApprovalResolution::Decision {
        decision: ApprovalDecision::Approved,
        ..
      } => crate::event::NotificationResolution::Approved,
    };
    for delivery in projection
      .notification_deliveries
      .values()
      .filter(|delivery| {
        delivery.approval_id == approval_id
          && delivery.request_id == request.request_id
          && matches!(
            delivery.provider.as_str(),
            "slack" | "telegram" | "discord" | "whatsapp"
          )
          && matches!(
            delivery.status,
            NotificationDeliveryStatus::Succeeded { .. }
          )
      })
    {
      payloads.push(RunEventPayload::NotificationMessageUpdateRequested(
        crate::event::NotificationMessageUpdateRequestedData {
          approval_id: approval_id.to_string(),
          request_id: request.request_id.clone(),
          delivery_id: delivery.delivery_id.clone(),
          update_id: format!("nupdate_{}", Uuid::new_v4().simple()),
          resolution: notification_resolution,
        },
      ));
    }
    if request.on_timeout == ApprovalTimeoutPolicy::Fail {
      payloads.push(RunEventPayload::RunFailed(RunFailedData::V4(
        RunFailedDataV4::Approval {
          approval_id: approval_id.to_string(),
          request_id: request.request_id.clone(),
          failure: ApprovalFailure {
            kind: "approval_timeout".to_string(),
            code: "WOML_APPROVAL_TIMEOUT".to_string(),
            message: format!("Approval {approval_id:?} reached its deadline."),
          },
        },
      )));
    }
    let event_schema_version = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| {
        DurableStoreError::Contract("Approval timeout requires run_started.".to_string())
      })?;
    for payload in payloads {
      validate_payload_against_definition(&workflow, &binding.definition_hash, &payload)
        .map_err(DurableStoreError::Contract)?;
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        now,
        event_schema_version,
        payload,
      )?;
    }
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    fold_events(&events)?;
    transaction.commit()?;
    Ok(ApprovalTimeoutSettlement {
      status: ApprovalTimeoutSettlementStatus::Settled,
      run_id: run_id.to_string(),
      approval_id: approval_id.to_string(),
      request_id: request.request_id.clone(),
      resolution: Some(resolution),
      settled_at: Some(now),
    })
  }

  pub fn verify_approval_token(
    &self,
    token: &str,
    now: DateTime<Utc>,
  ) -> Result<ApprovalTokenBinding, DurableStoreError> {
    let (token_id, secret) = parse_approval_token(token)?;
    let row: Option<(Vec<u8>, String, String, String, String, String)> = self
      .connection
      .query_row(
        "SELECT secret_hash, request_id, run_id, approval_id,
                issued_at, credential_expires_at
         FROM woml_approval_tokens WHERE token_id = ?1",
        [token_id],
        |row| {
          Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
          ))
        },
      )
      .optional()?;
    let Some((stored_hash, request_id, run_id, approval_id, issued_at, expires_at)) = row else {
      return Err(DurableStoreError::InvalidApprovalToken);
    };
    let candidate_hash = Sha256::digest(secret.as_bytes());
    if stored_hash.len() != 32
      || candidate_hash
        .as_slice()
        .ct_eq(stored_hash.as_slice())
        .unwrap_u8()
        != 1
    {
      return Err(DurableStoreError::InvalidApprovalToken);
    }
    let issued_at = parse_stored_timestamp(&issued_at)?;
    let credential_expires_at = parse_stored_timestamp(&expires_at)?;
    if now >= credential_expires_at {
      return Err(DurableStoreError::ExpiredApprovalToken);
    }
    let projection = self.projection(&run_id)?;
    let request_is_waiting =
      projection
        .approval_requests
        .get(&approval_id)
        .is_some_and(|request| {
          request.request_id == request_id
            && matches!(request.status, ApprovalRequestStatus::Waiting)
        });
    if projection.status != RunStatus::Waiting || !request_is_waiting {
      return Err(DurableStoreError::ApprovalExpired);
    }
    Ok(ApprovalTokenBinding {
      token_id: token_id.to_string(),
      request_id,
      run_id,
      approval_id,
      issued_at,
      credential_expires_at,
    })
  }

  pub fn reissue_approval_token(
    &mut self,
    run_id: &str,
    approval_id: &str,
    request_id: &str,
    issued_at: DateTime<Utc>,
  ) -> Result<IssuedApprovalToken, DurableStoreError> {
    self.issue_approval_token(run_id, approval_id, request_id, issued_at)
  }

  pub fn approval_token_count_for_request(
    &self,
    run_id: &str,
    approval_id: &str,
    request_id: &str,
  ) -> Result<usize, DurableStoreError> {
    let count: i64 = self.connection.query_row(
      "SELECT COUNT(*) FROM woml_approval_tokens
       WHERE run_id = ?1 AND approval_id = ?2 AND request_id = ?3",
      params![run_id, approval_id, request_id],
      |row| row.get(0),
    )?;
    usize::try_from(count)
      .map_err(|_| DurableStoreError::Contract("Approval credential count is invalid.".to_string()))
  }

  pub fn recover_interrupted_runs(&mut self) -> Result<RecoveryReport, DurableStoreError> {
    let run_ids = self.run_ids()?;
    let mut report = RecoveryReport {
      inspected_runs: run_ids.len(),
      ..RecoveryReport::default()
    };
    for run_id in run_ids {
      match self.recover_run(&run_id)? {
        RunRecovery::Recovered {
          interrupted_attempts,
        } => {
          report.recovered_runs += 1;
          report.interrupted_attempts += interrupted_attempts;
        }
        RunRecovery::Resumable => {
          report.resumable_runs += 1;
        }
        RunRecovery::Unchanged => {}
      }
    }
    self.reconcile_workflow_call_states()?;
    Ok(report)
  }

  /// Recovers only workflow-call children owned by the supplied local target
  /// definitions. This avoids treating runs executing in other live WOML
  /// processes as interrupted during cross-process startup.
  pub fn recover_workflow_call_children_for_targets(
    &mut self,
    targets: &[WorkflowTarget],
  ) -> Result<RecoveryReport, DurableStoreError> {
    let mut run_ids = Vec::new();
    for target in targets {
      let mut statement = self.connection.prepare(
        "SELECT child_run_id FROM woml_workflow_calls
         WHERE target_workflow_id = ?1 AND target_definition_hash = ?2
           AND state IN ('admitted', 'running')
         ORDER BY admitted_at, call_key",
      )?;
      run_ids.extend(
        statement
          .query_map([&target.workflow_id, &target.definition_hash], |row| {
            row.get::<_, String>(0)
          })?
          .collect::<Result<Vec<_>, _>>()?,
      );
    }
    run_ids.sort();
    run_ids.dedup();
    let mut report = RecoveryReport {
      inspected_runs: run_ids.len(),
      ..RecoveryReport::default()
    };
    for run_id in run_ids {
      match self.recover_run(&run_id)? {
        RunRecovery::Recovered {
          interrupted_attempts,
        } => {
          report.recovered_runs += 1;
          report.interrupted_attempts += interrupted_attempts;
        }
        RunRecovery::Resumable => report.resumable_runs += 1,
        RunRecovery::Unchanged => {}
      }
    }
    self.reconcile_workflow_call_states()?;
    Ok(report)
  }

  /// Repairs the reconstructible call index from authoritative child history
  /// after run recovery. No result value is copied into the index.
  pub fn reconcile_workflow_call_states(&mut self) -> Result<(), DurableStoreError> {
    let pending = {
      let mut statement = self.connection.prepare(
        "SELECT call_key, child_run_id, state FROM woml_workflow_calls
         WHERE state IN ('admitted', 'running') ORDER BY admitted_at, call_key",
      )?;
      let rows = statement
        .query_map([], |row| {
          Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
          ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
      rows
    };
    for (call_key, child_run_id, stored_state) in pending {
      let projection = self.projection(&child_run_id)?;
      let next = match projection.status {
        RunStatus::Succeeded => Some("succeeded"),
        RunStatus::Failed | RunStatus::Cancelled | RunStatus::Waiting => Some("failed"),
        RunStatus::Running if stored_state == "running" => Some("admitted"),
        RunStatus::NotStarted
        | RunStatus::Queued
        | RunStatus::Running
        | RunStatus::Cancelling
        | RunStatus::Finalizing => None,
      };
      if let Some(next) = next {
        self.connection.execute(
          "UPDATE woml_workflow_calls SET state = ?1 WHERE call_key = ?2",
          params![next, call_key],
        )?;
      }
    }
    Ok(())
  }

  fn run_ids(&self) -> Result<Vec<String>, DurableStoreError> {
    let mut statement = self
      .connection
      .prepare("SELECT run_id FROM woml_runs ORDER BY created_at, run_id")?;
    let rows = statement.query_map([], |row| row.get(0))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
  }

  fn recover_run(&mut self, run_id: &str) -> Result<RunRecovery, DurableStoreError> {
    let transaction = self
      .connection
      .transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_run_exists(&transaction, run_id)?;
    let mut events = load_events(&transaction, run_id)?;
    let workflow = definition_for_run(&transaction, run_id)?;
    let binding = run_binding_in_transaction(&transaction, run_id)?;
    validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
      .map_err(DurableStoreError::Contract)?;
    let projection = fold_events(&events)?;
    let event_schema_version = projection.event_schema_version.ok_or_else(|| {
      DurableStoreError::Contract("A stored run has no event schema version.".to_string())
    })?;
    if matches!(
      event_schema_version,
      crate::RUN_EVENT_SCHEMA_VERSION_V18
        | crate::RUN_EVENT_SCHEMA_VERSION_V19
        | crate::RUN_EVENT_SCHEMA_VERSION_V20
    ) {
      let active_corrections = projection
        .sagas
        .values()
        .flat_map(|saga| {
          saga.actions.values().filter_map(|action| {
            (action.status == crate::CorrectionActionStatus::Started).then_some((
              saga.saga_id.clone(),
              action.action_id.clone(),
              action.action_kind,
              action.active_attempt.unwrap_or(action.attempts),
            ))
          })
        })
        .collect::<Vec<_>>();
      if !active_corrections.is_empty() {
        if active_corrections.len() > CORRECTION_RECOVERY_BATCH_LIMIT {
          return Err(DurableStoreError::Contract(
            "WOML_CORRECTION_LIMIT_EXCEEDED: interrupted correction batch exceeds 100 actions."
              .to_string(),
          ));
        }
        let now = Utc::now();
        for (saga_id, action_id, action_kind, attempt) in &active_corrections {
          append_to_history(
            &transaction,
            &mut events,
            run_id,
            generated_event_id(),
            now,
            event_schema_version,
            RunEventPayload::CorrectionInterrupted(crate::CorrectionInterruptedData {
              saga_id: saga_id.clone(),
              action_id: action_id.clone(),
              action_kind: *action_kind,
              attempt: *attempt,
              ambiguous: true,
            }),
          )?;
          append_to_history(
            &transaction,
            &mut events,
            run_id,
            generated_event_id(),
            now,
            event_schema_version,
            RunEventPayload::ManualRecoveryRequired(crate::ManualRecoveryRequiredData {
              saga_id: saga_id.clone(),
              action_id: action_id.clone(),
              action_kind: *action_kind,
              code: "WOML_CORRECTION_INTERRUPTED".to_string(),
            }),
          )?;
        }
        validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
          .map_err(DurableStoreError::Contract)?;
        let recovered = fold_events(&events)?;
        write_correction_indexes_from_projection(
          &transaction,
          run_id,
          &binding.definition_hash,
          &events,
          &recovered,
        )?;
        transaction.commit()?;
        return Ok(RunRecovery::Recovered {
          interrupted_attempts: active_corrections.len(),
        });
      }
      let correction_can_resume = projection.sagas.values().any(|saga| {
        matches!(
          saga.phase,
          crate::SagaPhase::Correcting | crate::SagaPhase::CorrectionCompleted
        )
      });
      if correction_can_resume
        && matches!(
          projection.status,
          RunStatus::Running | RunStatus::Cancelling | RunStatus::Finalizing
        )
      {
        return Ok(RunRecovery::Resumable);
      }
    }
    if let Some((interrupted_attempts, failure)) = recover_open_for_each_failure(
      &transaction,
      &workflow,
      &mut events,
      run_id,
      event_schema_version,
      Utc::now(),
      None,
    )? {
      let now = Utc::now();
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        now,
        event_schema_version,
        RunEventPayload::RunOutcomeDecided(crate::event::RunOutcomeDecidedData::Failed {
          failure: lifecycle_failure_from_attempt(&failure),
        }),
      )?;
      if let Some(hook) = workflow
        .lifecycle_hook_for_event(crate::model::LifecycleEventName::RunFailure)
        .or_else(|| {
          workflow.lifecycle_hook_for_event(crate::model::LifecycleEventName::RunComplete)
        })
      {
        let subject = LifecycleSubject {
          kind: LifecycleSubjectKind::Workflow,
          id: run_id.to_string(),
        };
        append_to_history(
          &transaction,
          &mut events,
          run_id,
          generated_event_id(),
          now,
          event_schema_version,
          RunEventPayload::LifecycleHookRequested(LifecycleHookRequestedData {
            hook_invocation_id: derive_lifecycle_hook_invocation_id(
              run_id,
              &hook.hook_id,
              subject.kind,
              &subject.id,
            ),
            hook_id: hook.hook_id.clone(),
            event: hook.event,
            subject,
          }),
        )?;
      } else {
        append_to_history(
          &transaction,
          &mut events,
          run_id,
          generated_event_id(),
          now,
          event_schema_version,
          RunEventPayload::RunFinalized(crate::event::RunFinalizedData {
            outcome: crate::event::BusinessOutcome::Failed,
            lifecycle_status: crate::event::FinalLifecycleStatus::Completed,
            warnings: Vec::new(),
          }),
        )?;
      }
      validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
        .map_err(DurableStoreError::Contract)?;
      transaction.commit()?;
      return Ok(RunRecovery::Recovered {
        interrupted_attempts,
      });
    }
    if matches!(
      event_schema_version,
      crate::RUN_EVENT_SCHEMA_VERSION_V10
        | crate::RUN_EVENT_SCHEMA_VERSION_V11
        | crate::RUN_EVENT_SCHEMA_VERSION_V12
        | crate::RUN_EVENT_SCHEMA_VERSION_V13
        | crate::RUN_EVENT_SCHEMA_VERSION_V14
        | crate::RUN_EVENT_SCHEMA_VERSION_V15
        | crate::RUN_EVENT_SCHEMA_VERSION_V16
        | crate::RUN_EVENT_SCHEMA_VERSION_V17
        | crate::RUN_EVENT_SCHEMA_VERSION_V18
        | crate::RUN_EVENT_SCHEMA_VERSION_V19
        | crate::RUN_EVENT_SCHEMA_VERSION_V20
    ) {
      let ambiguous_actions = projection
        .lifecycle_hooks
        .values()
        .flat_map(|hook| {
          hook.actions.values().filter_map(|action| {
            (action.status == crate::projection::LifecycleActionStatus::Started).then_some((
              hook.hook_invocation_id.clone(),
              action.action_id.clone(),
              action.attempt,
            ))
          })
        })
        .collect::<Vec<_>>();
      if !ambiguous_actions.is_empty() {
        let now = Utc::now();
        for (hook_invocation_id, action_id, attempt) in &ambiguous_actions {
          append_to_history(
            &transaction,
            &mut events,
            run_id,
            generated_event_id(),
            now,
            event_schema_version,
            RunEventPayload::LifecycleActionFailed(crate::event::LifecycleActionFailedData {
              hook_invocation_id: hook_invocation_id.clone(),
              action_id: action_id.clone(),
              attempt: *attempt,
              failure: crate::event::LifecycleFailure {
                kind: crate::event::LifecycleFailureKind::Interrupted,
                code: "WOML_LIFECYCLE_ACTION_INTERRUPTED".to_string(),
                message: "Recovery found a started lifecycle action without a terminal event; its outcome is ambiguous and it will not be replayed.".to_string(),
              },
            }),
          )?;
        }
        let recovered = fold_events(&events)?;
        for hook_invocation_id in ambiguous_actions
          .iter()
          .map(|(hook_id, _, _)| hook_id)
          .collect::<std::collections::HashSet<_>>()
        {
          let hook_projection = recovered.lifecycle_hooks.get(hook_invocation_id).unwrap();
          let compiled = workflow
            .lifecycle_hook(&hook_projection.hook_id)
            .ok_or_else(|| {
              DurableStoreError::Contract("Recovered hook is absent from Model v11.".to_string())
            })?;
          if hook_projection.actions.len() == compiled.actions.len() {
            let completed_event = compiled.event;
            let completed_subject = hook_projection.subject.clone();
            append_to_history(
              &transaction,
              &mut events,
              run_id,
              generated_event_id(),
              now,
              event_schema_version,
              RunEventPayload::LifecycleHookCompleted(crate::event::LifecycleHookCompletedData {
                hook_invocation_id: hook_invocation_id.clone(),
                status: crate::event::LifecycleHookCompletionStatus::CompletedWithWarnings,
                failed_actions: hook_projection.failed_actions,
              }),
            )?;
            let next_hook = match completed_event {
              crate::model::LifecycleEventName::StepSuccess
              | crate::model::LifecycleEventName::StepFailure => workflow
                .lifecycle_hook_for_step_event(
                  crate::model::LifecycleEventName::StepComplete,
                  &completed_subject.id,
                ),
              crate::model::LifecycleEventName::RunSuccess
              | crate::model::LifecycleEventName::RunFailure
              | crate::model::LifecycleEventName::RunCancel => {
                workflow.lifecycle_hook_for_event(crate::model::LifecycleEventName::RunComplete)
              }
              _ => None,
            };
            if let Some(next_hook) = next_hook {
              let subject = if next_hook.event.is_step() {
                completed_subject
              } else {
                LifecycleSubject {
                  kind: LifecycleSubjectKind::Workflow,
                  id: run_id.to_string(),
                }
              };
              append_to_history(
                &transaction,
                &mut events,
                run_id,
                generated_event_id(),
                now,
                event_schema_version,
                RunEventPayload::LifecycleHookRequested(LifecycleHookRequestedData {
                  hook_invocation_id: derive_lifecycle_hook_invocation_id(
                    run_id,
                    &next_hook.hook_id,
                    subject.kind,
                    &subject.id,
                  ),
                  hook_id: next_hook.hook_id.clone(),
                  event: next_hook.event,
                  subject,
                }),
              )?;
            }
          }
        }
        validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
          .map_err(DurableStoreError::Contract)?;
        transaction.commit()?;
        return Ok(RunRecovery::Recovered {
          interrupted_attempts: 0,
        });
      }
      let has_pending_hook = projection.lifecycle_hooks.values().any(|hook| {
        matches!(
          hook.status,
          crate::projection::LifecycleHookStatus::Requested
            | crate::projection::LifecycleHookStatus::Running
        )
      });
      let has_active_business_attempt = projection
        .attempts
        .iter()
        .any(|attempt| attempt.status == AttemptStatus::Started);
      let has_active_operation = projection
        .operations
        .values()
        .any(|operation| operation.status == crate::projection::OperationStatus::Started);
      if (has_pending_hook && !has_active_business_attempt && !has_active_operation)
        || matches!(
          projection.status,
          RunStatus::Cancelling | RunStatus::Finalizing
        )
      {
        return Ok(RunRecovery::Resumable);
      }
    }
    if projection.status == RunStatus::Waiting {
      let started = projection
        .notification_deliveries
        .values()
        .filter_map(|delivery| {
          if let NotificationDeliveryStatus::AttemptStarted {
            attempt,
            attempt_id,
            ..
          } = &delivery.status
          {
            Some((delivery.clone(), *attempt, attempt_id.clone()))
          } else {
            None
          }
        })
        .collect::<Vec<_>>();
      if started.is_empty() {
        return Ok(RunRecovery::Unchanged);
      }
      let now = Utc::now();
      for (delivery, attempt, attempt_id) in &started {
        append_to_history(
          &transaction,
          &mut events,
          run_id,
          generated_event_id(),
          now,
          event_schema_version,
          RunEventPayload::NotificationDeliveryFailed(NotificationDeliveryFailedData {
            approval_id: delivery.approval_id.clone(),
            request_id: delivery.request_id.clone(),
            delivery_id: delivery.delivery_id.clone(),
            attempt: *attempt,
            attempt_id: attempt_id.clone(),
            final_: true,
            failure: NotificationSafeFailure {
              kind: "delivery_ambiguous".to_string(),
              code: "WOML_NOTIFICATION_DELIVERY_AMBIGUOUS".to_string(),
              message: "Recovery found an uncertain notification send and will not replay it."
                .to_string(),
              retryable: false,
              retry_after_ms: None,
            },
          }),
        )?;
      }
      let mut recovered = fold_events(&events)?;
      for (approval_id, request_id) in started
        .iter()
        .map(|(delivery, _, _)| (&delivery.approval_id, &delivery.request_id))
      {
        if recovered.status == RunStatus::Waiting
          && notification_request_all_failed(&workflow, &recovered, approval_id, request_id)
        {
          let ids = workflow
            .approval(approval_id)
            .unwrap()
            .notifications
            .into_iter()
            .map(|item| item.delivery_id)
            .collect();
          append_to_history(
            &transaction,
            &mut events,
            run_id,
            generated_event_id(),
            now,
            event_schema_version,
            RunEventPayload::RunFailed(RunFailedData::V5(RunFailedDataV5::Notification {
              approval_id: approval_id.clone(),
              request_id: request_id.clone(),
              failed_delivery_ids: ids,
              failure: NotificationRunFailure {
                kind: "all_deliveries_failed".to_string(),
                code: "WOML_NOTIFICATION_DELIVERY_FAILED".to_string(),
                message: "Every configured approval notification delivery failed.".to_string(),
              },
            })),
          )?;
          recovered = fold_events(&events)?;
          break;
        }
      }
      validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
        .map_err(DurableStoreError::Contract)?;
      transaction.commit()?;
      let _ = recovered;
      return Ok(RunRecovery::Recovered {
        interrupted_attempts: 0,
      });
    }
    let started_updates = projection
      .notification_updates
      .values()
      .filter_map(|update| {
        if let NotificationMessageUpdateStatus::AttemptStarted {
          attempt,
          attempt_id,
        } = &update.status
        {
          Some((update.clone(), *attempt, attempt_id.clone()))
        } else {
          None
        }
      })
      .collect::<Vec<_>>();
    if !started_updates.is_empty() {
      let now = Utc::now();
      for (update, attempt, attempt_id) in &started_updates {
        append_to_history(
          &transaction,
          &mut events,
          run_id,
          generated_event_id(),
          now,
          event_schema_version,
          RunEventPayload::NotificationMessageUpdateFailed(
            NotificationMessageUpdateFailedData {
              approval_id: update.approval_id.clone(),
              request_id: update.request_id.clone(),
              delivery_id: update.delivery_id.clone(),
              update_id: update.update_id.clone(),
              attempt: *attempt,
              attempt_id: attempt_id.clone(),
              final_: *attempt >= 3,
              failure: NotificationSafeFailure {
                kind: "update_failed".to_string(),
                code: "WOML_NOTIFICATION_UPDATE_INTERRUPTED".to_string(),
                message: "Recovery found an interrupted message update; the durable update remains retryable."
                  .to_string(),
                retryable: *attempt < 3,
                retry_after_ms: None,
              },
            },
          ),
        )?;
      }
      validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
        .map_err(DurableStoreError::Contract)?;
      transaction.commit()?;
      return Ok(RunRecovery::Recovered {
        interrupted_attempts: 0,
      });
    }
    if projection.status != RunStatus::Running {
      return Ok(RunRecovery::Unchanged);
    }
    let active_operations = projection
      .operations
      .values()
      .filter(|operation| operation.status == crate::projection::OperationStatus::Started)
      .cloned()
      .collect::<Vec<_>>();
    if !active_operations.is_empty() {
      let now = Utc::now();
      for operation in &active_operations {
        if let Some(settlement) = recovered_state_mutation_settlement(&transaction, operation)? {
          append_to_history(
            &transaction,
            &mut events,
            run_id,
            generated_event_id(),
            now,
            event_schema_version,
            RunEventPayload::OperationSucceeded(OperationSucceededData {
              node_id: operation.node_id.clone(),
              attempt_number: operation.attempt_number,
              invocation_id: operation.identity.invocation_id.clone(),
              call_id: operation.identity.call_id.clone(),
              operation_key: operation.operation_key.clone(),
              capability: operation.capability.clone(),
              operation: operation.operation.clone(),
              execution_mode: operation.execution_mode,
              metadata: settlement.metadata,
              duration_ms: 0.0,
              result_bytes: settlement.result_bytes,
              result_digest: settlement.result_digest,
            }),
          )?;
          continue;
        }
        let observed = operation.execution_mode == crate::event::OperationExecutionMode::Observed;
        append_to_history(
          &transaction,
          &mut events,
          run_id,
          generated_event_id(),
          now,
          event_schema_version,
          RunEventPayload::OperationFailed(OperationFailedData {
            node_id: operation.node_id.clone(),
            attempt_number: operation.attempt_number,
            invocation_id: operation.identity.invocation_id.clone(),
            call_id: operation.identity.call_id.clone(),
            operation_key: operation.operation_key.clone(),
            capability: operation.capability.clone(),
            operation: operation.operation.clone(),
            execution_mode: operation.execution_mode,
            metadata: interrupted_operation_metadata(operation),
            duration_ms: 0.0,
            failure: interrupted_operation_failure(operation, observed, false),
          }),
        )?;
      }
    }
    let started = projection
      .attempts
      .iter()
      .filter(|attempt| attempt.status == AttemptStatus::Started)
      .map(|attempt| attempt.identity.clone())
      .collect::<Vec<_>>();
    if let Some(first) = started.first().cloned() {
      let failure = interrupted_failure();
      for attempt in &started {
        let payload = RunEventPayload::StepAttemptFailed(StepAttemptFailedData {
          node_id: attempt.node_id.clone(),
          attempt: attempt.attempt,
          invocation_id: attempt.invocation_id.clone(),
          failure: failure.clone(),
        });
        let payloads = if matches!(
          event_schema_version,
          crate::RUN_EVENT_SCHEMA_VERSION_V10
            | crate::RUN_EVENT_SCHEMA_VERSION_V11
            | crate::RUN_EVENT_SCHEMA_VERSION_V12
            | crate::RUN_EVENT_SCHEMA_VERSION_V13
            | crate::RUN_EVENT_SCHEMA_VERSION_V14
            | crate::RUN_EVENT_SCHEMA_VERSION_V15
            | crate::RUN_EVENT_SCHEMA_VERSION_V16
            | crate::RUN_EVENT_SCHEMA_VERSION_V17
            | crate::RUN_EVENT_SCHEMA_VERSION_V18
            | crate::RUN_EVENT_SCHEMA_VERSION_V19
            | crate::RUN_EVENT_SCHEMA_VERSION_V20
        ) {
          expand_model_v11_payload(&workflow, run_id, payload)?
        } else {
          vec![payload]
        };
        for payload in payloads {
          append_to_history(
            &transaction,
            &mut events,
            run_id,
            generated_event_id(),
            Utc::now(),
            event_schema_version,
            payload,
          )?;
        }
      }
      let has_unsettled_fork_work = workflow.schema_version
        >= crate::COMPILED_MODEL_SCHEMA_VERSION_V13
        && projection.forks.values().any(|fork| {
          fork.join_status == crate::projection::ForkJoinStatus::Pending
            || fork
              .branches
              .values()
              .any(|branch| branch.outcome.is_none())
        });
      if has_unsettled_fork_work {
        // The interrupted attempts are now durably failed, but Model v13 owns
        // every opened branch until it settles. Let the resumed fork scheduler
        // derive branch and join outcomes before it decides the run outcome;
        // appending RunFailed here would create an impossible half-open fork.
        validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
          .map_err(DurableStoreError::Contract)?;
        transaction.commit()?;
        return Ok(RunRecovery::Recovered {
          interrupted_attempts: started.len(),
        });
      }
      let run_failure = RunEventPayload::RunFailed(attempt_run_failed_data(
        event_schema_version,
        first.node_id,
        first.attempt,
        first.invocation_id,
        failure,
      ));
      let run_failure_payloads = if matches!(
        event_schema_version,
        crate::RUN_EVENT_SCHEMA_VERSION_V10
          | crate::RUN_EVENT_SCHEMA_VERSION_V11
          | crate::RUN_EVENT_SCHEMA_VERSION_V12
          | crate::RUN_EVENT_SCHEMA_VERSION_V13
          | crate::RUN_EVENT_SCHEMA_VERSION_V14
          | crate::RUN_EVENT_SCHEMA_VERSION_V15
          | crate::RUN_EVENT_SCHEMA_VERSION_V16
          | crate::RUN_EVENT_SCHEMA_VERSION_V17
          | crate::RUN_EVENT_SCHEMA_VERSION_V18
          | crate::RUN_EVENT_SCHEMA_VERSION_V19
          | crate::RUN_EVENT_SCHEMA_VERSION_V20
      ) {
        expand_model_v11_payload(&workflow, run_id, run_failure)?
      } else {
        vec![run_failure]
      };
      for payload in run_failure_payloads {
        append_to_history(
          &transaction,
          &mut events,
          run_id,
          generated_event_id(),
          Utc::now(),
          event_schema_version,
          payload,
        )?;
      }
      validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
        .map_err(DurableStoreError::Contract)?;
      transaction.commit()?;
      return Ok(RunRecovery::Recovered {
        interrupted_attempts: started.len(),
      });
    }

    for group_projection in projection.parallel_groups.values() {
      if group_projection.status != ParallelGroupStatus::Started {
        continue;
      }
      let group = workflow
        .parallel_group(&group_projection.parallel_id)
        .ok_or_else(|| {
          DurableStoreError::Contract(format!(
            "Stored run references unknown parallel group {:?}.",
            group_projection.parallel_id
          ))
        })?;
      let child_statuses = group
        .child_node_ids
        .iter()
        .map(|node_id| {
          (
            node_id,
            projection
              .attempts
              .iter()
              .rev()
              .find(|attempt| attempt.identity.node_id == *node_id)
              .map(|attempt| &attempt.status),
          )
        })
        .collect::<Vec<_>>();
      let every_succeeded = child_statuses
        .iter()
        .all(|(_, status)| matches!(status, Some(AttemptStatus::Succeeded { .. })));
      if every_succeeded {
        append_to_history(
          &transaction,
          &mut events,
          run_id,
          generated_event_id(),
          Utc::now(),
          event_schema_version,
          RunEventPayload::ParallelGroupCompleted(ParallelGroupCompletedData {
            parallel_id: group.parallel_id.clone(),
            outcome: ParallelGroupOutcome::Succeeded,
            failed_node_ids: Vec::new(),
            cancelled_node_ids: Vec::new(),
          }),
        )?;
        validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
          .map_err(DurableStoreError::Contract)?;
        transaction.commit()?;
        return Ok(RunRecovery::Recovered {
          interrupted_attempts: 0,
        });
      }

      let every_terminal = child_statuses.iter().all(|(node_id, status)| {
        matches!(
          status,
          Some(AttemptStatus::Succeeded { .. } | AttemptStatus::Failed { .. })
        ) && !projection.pending_retries.contains_key(*node_id)
      });
      let failed_node_ids = child_statuses
        .iter()
        .filter_map(|(node_id, status)| match status {
          Some(AttemptStatus::Failed { failure })
            if failure.kind != AttemptFailureKind::InvocationCancelled
              && !projection.pending_retries.contains_key(*node_id) =>
          {
            Some((*node_id).clone())
          }
          _ => None,
        })
        .collect::<Vec<_>>();
      if failed_node_ids.is_empty() || (group.on_error == "wait-all" && !every_terminal) {
        continue;
      }
      let cancelled_node_ids = child_statuses
        .iter()
        .filter_map(|(node_id, status)| match status {
          Some(AttemptStatus::Failed { failure })
            if failure.kind == AttemptFailureKind::InvocationCancelled =>
          {
            Some((*node_id).clone())
          }
          _ => None,
        })
        .collect::<Vec<_>>();
      let primary_node_id = events
        .iter()
        .find_map(|event| match &event.payload {
          RunEventPayload::StepAttemptFailed(data) if failed_node_ids.contains(&data.node_id) => {
            Some(data.node_id.clone())
          }
          _ => None,
        })
        .ok_or_else(|| {
          DurableStoreError::Contract(format!(
            "Parallel group {:?} has failed children without a failure event.",
            group.parallel_id
          ))
        })?;
      let policy = if group.on_error == "wait-all" {
        ParallelFailurePolicy::WaitAll
      } else {
        ParallelFailurePolicy::FailFast
      };
      let message = parallel_failure_message(policy, failed_node_ids.len());
      let failure = ParallelFailure {
        kind: "parallel_child_failed".to_string(),
        code: "WOML_PARALLEL_CHILD_FAILED".to_string(),
        message,
      };
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        Utc::now(),
        event_schema_version,
        RunEventPayload::ParallelGroupCompleted(ParallelGroupCompletedData {
          parallel_id: group.parallel_id.clone(),
          outcome: ParallelGroupOutcome::Failed,
          failed_node_ids: failed_node_ids.clone(),
          cancelled_node_ids: cancelled_node_ids.clone(),
        }),
      )?;
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        Utc::now(),
        event_schema_version,
        RunEventPayload::RunFailed(RunFailedData::V3(RunFailedDataV3::Parallel {
          parallel_id: group.parallel_id.clone(),
          policy,
          primary_node_id,
          failed_node_ids,
          cancelled_node_ids,
          failure,
        })),
      )?;
      validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
        .map_err(DurableStoreError::Contract)?;
      transaction.commit()?;
      return Ok(RunRecovery::Recovered {
        interrupted_attempts: 0,
      });
    }

    // A failed attempt followed by a durable retry schedule is unfinished work,
    // not a terminal failure. The schedule is the persistence boundary that
    // makes starting the next attempt safe after a process restart.
    if !projection.pending_retries.is_empty() {
      return Ok(RunRecovery::Resumable);
    }

    // Only the latest attempt for a node can determine its current state. An
    // older failure must never override a later successful retry.
    let failed_attempt = projection.attempts.iter().rev().find_map(|attempt| {
      let is_latest_for_node = projection
        .attempts
        .iter()
        .rev()
        .find(|candidate| candidate.identity.node_id == attempt.identity.node_id)
        .is_some_and(|latest| latest.identity == attempt.identity);
      if is_latest_for_node {
        if let AttemptStatus::Failed { failure } = &attempt.status {
          return Some((attempt.identity.clone(), failure.clone()));
        }
      }
      None
    });
    if let Some((identity, failure)) = failed_attempt {
      if workflow
        .parallel_group_for_child(&identity.node_id)
        .is_some_and(|group| projection.parallel_groups.contains_key(&group.parallel_id))
      {
        return Ok(RunRecovery::Resumable);
      }
      let run_failure = RunEventPayload::RunFailed(attempt_run_failed_data(
        event_schema_version,
        identity.node_id,
        identity.attempt,
        identity.invocation_id,
        failure,
      ));
      let run_failure_payloads = if matches!(
        event_schema_version,
        crate::RUN_EVENT_SCHEMA_VERSION_V10
          | crate::RUN_EVENT_SCHEMA_VERSION_V11
          | crate::RUN_EVENT_SCHEMA_VERSION_V12
          | crate::RUN_EVENT_SCHEMA_VERSION_V13
          | crate::RUN_EVENT_SCHEMA_VERSION_V14
          | crate::RUN_EVENT_SCHEMA_VERSION_V15
          | crate::RUN_EVENT_SCHEMA_VERSION_V16
          | crate::RUN_EVENT_SCHEMA_VERSION_V17
          | crate::RUN_EVENT_SCHEMA_VERSION_V18
          | crate::RUN_EVENT_SCHEMA_VERSION_V19
          | crate::RUN_EVENT_SCHEMA_VERSION_V20
      ) {
        expand_model_v11_payload(&workflow, run_id, run_failure)?
      } else {
        vec![run_failure]
      };
      for payload in run_failure_payloads {
        append_to_history(
          &transaction,
          &mut events,
          run_id,
          generated_event_id(),
          Utc::now(),
          event_schema_version,
          payload,
        )?;
      }
      transaction.commit()?;
      return Ok(RunRecovery::Recovered {
        interrupted_attempts: 0,
      });
    }

    let terminal_node_id = workflow.terminal_node_id().ok_or_else(|| {
      DurableStoreError::Contract("Stored workflow has no terminal node.".to_string())
    })?;
    if let Some(result) = projection.context.steps.get(terminal_node_id).cloned() {
      append_to_history(
        &transaction,
        &mut events,
        run_id,
        generated_event_id(),
        Utc::now(),
        event_schema_version,
        RunEventPayload::RunSucceeded(RunSucceededData {
          terminal_node_id: terminal_node_id.to_string(),
          result,
        }),
      )?;
      transaction.commit()?;
      return Ok(RunRecovery::Recovered {
        interrupted_attempts: 0,
      });
    }

    Ok(RunRecovery::Resumable)
  }
}

fn hydrate_rich_approval_public_outputs(
  connection: &Connection,
  run_id: &str,
  projection: &mut RunProjection,
) -> Result<(), DurableStoreError> {
  for request in projection.rich_approval_requests.values() {
    if request.status == crate::RichApprovalRequestStatus::Waiting {
      continue;
    }
    let review_json: Option<String> = connection
      .query_row(
        "SELECT review_json FROM woml_approval_reviews
         WHERE run_id = ?1 AND request_id = ?2",
        params![run_id, request.request_id],
        |row| row.get(0),
      )
      .optional()?;
    let Some(review_json) = review_json else {
      return Err(DurableStoreError::Contract(
        "A settled rich approval has no immutable review snapshot.".to_string(),
      ));
    };
    let review: ReviewPayloadV1 = serde_json::from_str(&review_json)?;
    let output = projection
      .context
      .steps
      .get_mut(&request.approval_id)
      .and_then(Value::as_object_mut)
      .ok_or_else(|| {
        DurableStoreError::Contract(
          "A settled rich approval has no public context output.".to_string(),
        )
      })?;
    output.insert(
      "review".to_string(),
      serde_json::json!({
        "digest": request.review_digest,
        "title": review.title,
        "risk": review.risk,
        "artifacts": review.artifacts,
      }),
    );
  }
  Ok(())
}

fn random_hex(byte_count: usize) -> Result<String, DurableStoreError> {
  let mut bytes = vec![0_u8; byte_count];
  getrandom(&mut bytes).map_err(|_| {
    DurableStoreError::Contract(
      "The operating system could not generate a secure approval credential.".to_string(),
    )
  })?;
  Ok(hex::encode(bytes))
}

fn notification_idempotency_key(run_id: &str, request_id: &str, delivery_id: &str) -> String {
  let mut digest = Sha256::new();
  digest.update(b"woml.notification.delivery.v1\0");
  digest.update(run_id.as_bytes());
  digest.update(b"\0");
  digest.update(request_id.as_bytes());
  digest.update(b"\0");
  digest.update(delivery_id.as_bytes());
  format!("sha256:{}", hex::encode(digest.finalize()))
}

fn notification_update_idempotency_key(
  run_id: &str,
  request_id: &str,
  delivery_id: &str,
  update_id: &str,
) -> String {
  let mut digest = Sha256::new();
  digest.update(b"woml.notification.update.v1\0");
  digest.update(run_id.as_bytes());
  digest.update(b"\0");
  digest.update(request_id.as_bytes());
  digest.update(b"\0");
  digest.update(delivery_id.as_bytes());
  digest.update(b"\0");
  digest.update(update_id.as_bytes());
  format!("sha256:{}", hex::encode(digest.finalize()))
}

fn next_notification_attempt(
  status: &NotificationDeliveryStatus,
  now: DateTime<Utc>,
) -> Result<Option<u32>, DurableStoreError> {
  match status {
    NotificationDeliveryStatus::Requested => Ok(Some(1)),
    NotificationDeliveryStatus::Failed {
      attempt,
      final_: false,
      failure,
      failed_at,
      ..
    } if failure.retryable && *attempt < 3 => {
      let scheduled_delay = if *attempt == 1 { 1_000 } else { 5_000 };
      let delay_ms = scheduled_delay.max(failure.retry_after_ms.unwrap_or(0));
      let delay = i64::try_from(delay_ms)
        .map(chrono::Duration::milliseconds)
        .map_err(|_| {
          DurableStoreError::Contract("Notification retry delay is invalid.".to_string())
        })?;
      Ok((now >= *failed_at + delay).then_some(*attempt + 1))
    }
    _ => Ok(None),
  }
}

struct RecoveredStateMutationSettlement {
  metadata: Map<String, Value>,
  result_bytes: u64,
  result_digest: String,
}

/// A state mutation and its immutable result are committed together. If the
/// process dies before appending operation_succeeded, recovery may use that
/// settlement proof instead of incorrectly classifying the mutation itself as
/// ambiguous. The script attempt still fails closed and is never replayed.
fn recovered_state_mutation_settlement(
  transaction: &Transaction<'_>,
  operation: &OperationProjection,
) -> Result<Option<RecoveredStateMutationSettlement>, DurableStoreError> {
  if operation.capability != "state"
    || operation.execution_mode != crate::event::OperationExecutionMode::Managed
    || !matches!(
      operation.operation.as_str(),
      "set" | "delete" | "increment" | "set_if_absent"
    )
  {
    return Ok(None);
  }
  let stored = transaction
    .query_row(
      "SELECT operation, key_digest, input_digest, result_json, result_digest,
              committed_version
       FROM woml_state_mutations WHERE operation_key = ?1",
      [&operation.operation_key],
      |row| {
        Ok((
          row.get::<_, String>(0)?,
          row.get::<_, String>(1)?,
          row.get::<_, String>(2)?,
          row.get::<_, String>(3)?,
          row.get::<_, String>(4)?,
          row.get::<_, Option<u64>>(5)?,
        ))
      },
    )
    .optional()?;
  let Some((stored_operation, key_digest, input_digest, result_json, result_digest, version)) =
    stored
  else {
    return Ok(None);
  };
  let expected_key_digest = operation.metadata.get("keyDigest").and_then(Value::as_str);
  let expected_input_digest = operation
    .metadata
    .get("inputDigest")
    .and_then(Value::as_str);
  let result: Value = serde_json::from_str(&result_json).map_err(|_| {
    DurableStoreError::Contract("A settled state mutation contains invalid JSON.".to_string())
  })?;
  let canonical = canonical_json(&result)?;
  let stored_result_digest = format!(
    "sha256:{}",
    hex::encode({
      let mut hasher = Sha256::new();
      hasher.update(b"woml.state-result\0v1\0");
      hasher.update(&canonical);
      hasher.finalize()
    })
  );
  if stored_operation != operation.operation
    || expected_key_digest != Some(key_digest.as_str())
    || expected_input_digest != Some(input_digest.as_str())
    || result_digest != stored_result_digest
    || result.get("contract") != Some(&Value::String("woml.state".to_string()))
    || result.get("contractVersion") != Some(&Value::from(1))
    || result.get("kind") != Some(&Value::String("result".to_string()))
    || result.get("operation") != Some(&Value::String(operation.operation.clone()))
    || !crate::durable_state::valid_mutation_result(&operation.operation, &result, version)
  {
    return Err(DurableStoreError::Contract(
      "A settled state mutation does not match its interrupted managed operation.".to_string(),
    ));
  }
  let encoded = serde_json::to_vec(&result)?;
  let mut metadata = operation.metadata.clone();
  metadata.insert(
    "outcome".to_string(),
    Value::String("succeeded".to_string()),
  );
  metadata.insert("resultDigest".to_string(), Value::String(result_digest));
  metadata.insert("durationMs".to_string(), Value::from(0.0));
  if let Some(version) = version {
    metadata.insert("version".to_string(), Value::from(version));
  }
  Ok(Some(RecoveredStateMutationSettlement {
    metadata,
    result_bytes: encoded.len() as u64,
    result_digest: format!("sha256:{}", hex::encode(Sha256::digest(&encoded))),
  }))
}

fn notification_request_all_failed(
  workflow: &CompiledWorkflowDefinition,
  projection: &RunProjection,
  approval_id: &str,
  request_id: &str,
) -> bool {
  let Some(approval) = workflow.approval(approval_id) else {
    return false;
  };
  !approval.notifications.is_empty()
    && approval.notifications.iter().all(|definition| {
      projection
        .notification_deliveries
        .get(&definition.delivery_id)
        .is_some_and(|delivery| {
          delivery.request_id == request_id
            && matches!(
              delivery.status,
              NotificationDeliveryStatus::Failed { final_: true, .. }
            )
        })
    })
}

fn next_notification_update_attempt(
  status: &NotificationMessageUpdateStatus,
  now: DateTime<Utc>,
) -> Result<Option<u32>, DurableStoreError> {
  match status {
    NotificationMessageUpdateStatus::Requested => Ok(Some(1)),
    NotificationMessageUpdateStatus::Failed {
      attempt,
      final_: false,
      failure,
      failed_at,
      ..
    } if failure.retryable && *attempt < 3 => {
      let scheduled_delay = if *attempt == 1 { 1_000 } else { 5_000 };
      let delay_ms = scheduled_delay.max(failure.retry_after_ms.unwrap_or(0));
      let delay = i64::try_from(delay_ms)
        .map(chrono::Duration::milliseconds)
        .map_err(|_| {
          DurableStoreError::Contract("Notification update retry delay is invalid.".to_string())
        })?;
      Ok((now >= *failed_at + delay).then_some(*attempt + 1))
    }
    _ => Ok(None),
  }
}

fn approval_credential_expiry(
  issued_at: DateTime<Utc>,
  approval_deadline: Option<DateTime<Utc>>,
) -> Result<DateTime<Utc>, DurableStoreError> {
  let default_expiry = issued_at
    .checked_add_signed(chrono::Duration::hours(
      DEFAULT_APPROVAL_CREDENTIAL_LIFETIME_HOURS,
    ))
    .ok_or_else(|| {
      DurableStoreError::Contract("Approval credential expiry exceeds the clock range.".to_string())
    })?;
  let credential_expires_at =
    approval_deadline.map_or(default_expiry, |deadline| deadline.min(default_expiry));
  if credential_expires_at <= issued_at {
    return Err(DurableStoreError::Contract(
      "Cannot issue an approval credential at or after the request deadline.".to_string(),
    ));
  }
  Ok(credential_expires_at)
}

fn parse_approval_token(token: &str) -> Result<(&str, &str), DurableStoreError> {
  let Some(body) = token.strip_prefix("apr_") else {
    return Err(DurableStoreError::InvalidApprovalToken);
  };
  let Some((token_id, secret)) = body.split_once('.') else {
    return Err(DurableStoreError::InvalidApprovalToken);
  };
  if token_id.len() != 32
    || secret.len() != 64
    || token_id.contains('.')
    || secret.contains('.')
    || !token_id.bytes().all(|byte| byte.is_ascii_hexdigit())
    || !secret.bytes().all(|byte| byte.is_ascii_hexdigit())
  {
    return Err(DurableStoreError::InvalidApprovalToken);
  }
  Ok((token_id, secret))
}

fn parse_notification_capability(token: &str) -> Result<(&str, &str), DurableStoreError> {
  let Some(token) = token.strip_prefix("ncap_") else {
    return Err(DurableStoreError::InvalidApprovalToken);
  };
  let Some((id, secret)) = token.split_once('.') else {
    return Err(DurableStoreError::InvalidApprovalToken);
  };
  let compact = id.len() == 16 && secret.len() == 32;
  let legacy = id.len() == 32 && secret.len() == 64;
  if (!compact && !legacy)
    || !id
      .bytes()
      .chain(secret.bytes())
      .all(|byte| byte.is_ascii_hexdigit())
  {
    return Err(DurableStoreError::InvalidApprovalToken);
  }
  Ok((id, secret))
}

fn parse_stored_timestamp(value: &str) -> Result<DateTime<Utc>, DurableStoreError> {
  DateTime::parse_from_rfc3339(value)
    .map(|timestamp| timestamp.with_timezone(&Utc))
    .map_err(|_| {
      DurableStoreError::Contract("Stored approval credential timestamp is invalid.".to_string())
    })
}

fn validate_workflow_call_admission_request(
  request: &WorkflowCallAdmissionRequest,
) -> Result<(), DurableStoreError> {
  let expected_child = format!(
    "run_call_{}",
    request
      .call_key
      .strip_prefix("sha256:")
      .unwrap_or(&request.call_key)
  );
  if !is_definition_hash(&request.call_key)
    || !is_definition_hash(&request.target_definition_hash)
    || request.child_run_id != expected_child
    || request.parent_run_id.is_empty()
    || request.parent_run_id.len() > 256
    || request.parent_node_id.is_empty()
    || request.parent_node_id.len() > 256
    || !(1..=10).contains(&request.parent_attempt)
    || request.target_workflow_id.is_empty()
    || request.target_workflow_id.len() > 256
  {
    return Err(DurableStoreError::Contract(
      "Workflow call admission identity is invalid.".to_string(),
    ));
  }
  durable_workflow_call_payload_digest(&request.payload)?;
  Ok(())
}

fn validate_runtime_route_fields(
  runtime_id: &str,
  targets: &[WorkflowTarget],
  endpoint: &str,
  session_credential_hash: &str,
  now: DateTime<Utc>,
  lease_expires_at: DateTime<Utc>,
) -> Result<(), DurableStoreError> {
  let port = endpoint
    .strip_prefix("http://127.0.0.1:")
    .and_then(|value| value.parse::<u16>().ok())
    .filter(|port| *port > 0);
  if runtime_id.is_empty()
    || runtime_id.len() > 256
    || targets.is_empty()
    || port.is_none()
    || !is_definition_hash(session_credential_hash)
    || lease_expires_at <= now
    || targets.iter().any(|target| {
      target.runtime_id != runtime_id
        || target.workflow_id.is_empty()
        || !is_definition_hash(&target.definition_hash)
    })
  {
    return Err(DurableStoreError::Contract(
      "Workflow runtime route registration is invalid.".to_string(),
    ));
  }
  Ok(())
}

fn durable_workflow_call_payload_digest(
  payload: &Map<String, Value>,
) -> Result<String, DurableStoreError> {
  let encoded = canonical_json(payload).map_err(|_| {
    DurableStoreError::Contract("Workflow call payload is not canonical JSON.".to_string())
  })?;
  if encoded.len() > 1_048_576 {
    return Err(DurableStoreError::Contract(
      "Workflow call payload exceeds the 1 MiB durable admission limit.".to_string(),
    ));
  }
  Ok(format!("sha256:{:x}", Sha256::digest(encoded)))
}

fn definition_by_hash(
  connection: &Connection,
  definition_hash: &str,
) -> Result<CompiledWorkflowDefinition, DurableStoreError> {
  let model_json: String = connection
    .query_row(
      "SELECT model_json FROM woml_definitions WHERE definition_hash = ?1",
      [definition_hash],
      |row| row.get(0),
    )
    .optional()?
    .ok_or_else(|| DurableStoreError::DefinitionNotFound(definition_hash.to_string()))?;
  let workflow: CompiledWorkflowDefinition = serde_json::from_str(&model_json)?;
  workflow.validate_structure()?;
  Ok(workflow)
}

fn load_run_binding_optional(
  connection: &Connection,
  run_id: &str,
) -> Result<Option<RunDefinitionBinding>, DurableStoreError> {
  connection
    .query_row(
      "SELECT run_id, workflow_id, definition_hash FROM woml_runs WHERE run_id = ?1",
      [run_id],
      |row| {
        Ok(RunDefinitionBinding {
          run_id: row.get(0)?,
          workflow_id: row.get(1)?,
          definition_hash: row.get(2)?,
        })
      },
    )
    .optional()
    .map_err(DurableStoreError::from)
}

fn load_workflow_call(
  connection: &Connection,
  call_key: &str,
) -> Result<Option<WorkflowCallAdmission>, DurableStoreError> {
  let stored: Option<(
    String,
    String,
    i64,
    String,
    String,
    String,
    String,
    i64,
    String,
    String,
  )> = connection
    .query_row(
      "SELECT parent_run_id, parent_node_id, parent_attempt,
                target_workflow_id, target_definition_hash, child_run_id,
                payload_digest, depth, state, admitted_at
         FROM woml_workflow_calls WHERE call_key = ?1",
      [call_key],
      |row| {
        Ok((
          row.get(0)?,
          row.get(1)?,
          row.get(2)?,
          row.get(3)?,
          row.get(4)?,
          row.get(5)?,
          row.get(6)?,
          row.get(7)?,
          row.get(8)?,
          row.get(9)?,
        ))
      },
    )
    .optional()?;
  let Some(stored) = stored else {
    return Ok(None);
  };
  if !is_definition_hash(call_key) {
    return Err(DurableStoreError::WorkflowCallHistoryInvalid(
      "stored call key is invalid".to_string(),
    ));
  }
  let expected_child_run_id = format!(
    "run_call_{}",
    call_key.strip_prefix("sha256:").unwrap_or(call_key)
  );
  if stored.5 != expected_child_run_id {
    return Err(DurableStoreError::WorkflowCallHistoryInvalid(
      "stored child run identity does not match its call key".to_string(),
    ));
  }
  let parent_attempt = u32::try_from(stored.2).map_err(|_| {
    DurableStoreError::WorkflowCallHistoryInvalid("stored parent attempt is invalid".to_string())
  })?;
  if !(1..=10).contains(&parent_attempt) {
    return Err(DurableStoreError::WorkflowCallHistoryInvalid(
      "stored parent attempt is invalid".to_string(),
    ));
  }
  let depth = u32::try_from(stored.7).map_err(|_| {
    DurableStoreError::WorkflowCallHistoryInvalid("stored lineage depth is invalid".to_string())
  })?;
  if !(1..=MAX_WORKFLOW_CALL_DEPTH).contains(&depth) {
    return Err(DurableStoreError::WorkflowCallHistoryInvalid(
      "stored lineage depth is invalid".to_string(),
    ));
  }
  if stored.1.is_empty()
    || stored.1.len() > 128
    || stored.3.is_empty()
    || stored.3.len() > 128
    || !is_definition_hash(&stored.4)
    || !is_definition_hash(&stored.6)
  {
    return Err(DurableStoreError::WorkflowCallHistoryInvalid(
      "stored workflow call identity fields are invalid".to_string(),
    ));
  }
  let parent_binding = load_run_binding_optional(connection, &stored.0)?.ok_or_else(|| {
    DurableStoreError::WorkflowCallHistoryInvalid(
      "stored workflow call parent run is missing".to_string(),
    )
  })?;
  let child_binding = load_run_binding_optional(connection, &stored.5)?.ok_or_else(|| {
    DurableStoreError::WorkflowCallHistoryInvalid(
      "stored workflow call child run is missing".to_string(),
    )
  })?;
  if parent_binding.run_id != stored.0
    || child_binding.run_id != stored.5
    || child_binding.workflow_id != stored.3
    || child_binding.definition_hash != stored.4
  {
    return Err(DurableStoreError::WorkflowCallHistoryInvalid(
      "stored child run binding does not match its workflow call".to_string(),
    ));
  }
  let target_definition_workflow_id: Option<String> = connection
    .query_row(
      "SELECT workflow_id FROM woml_definitions WHERE definition_hash = ?1",
      [&stored.4],
      |row| row.get(0),
    )
    .optional()?;
  if target_definition_workflow_id.as_deref() != Some(stored.3.as_str()) {
    return Err(DurableStoreError::WorkflowCallHistoryInvalid(
      "stored target definition does not match its workflow call".to_string(),
    ));
  }
  let state = match stored.8.as_str() {
    "admitted" => WorkflowCallIndexState::Admitted,
    "running" => WorkflowCallIndexState::Running,
    "succeeded" => WorkflowCallIndexState::Succeeded,
    "failed" => WorkflowCallIndexState::Failed,
    _ => {
      return Err(DurableStoreError::WorkflowCallHistoryInvalid(
        "stored call state is invalid".to_string(),
      ))
    }
  };
  let admitted_at = DateTime::parse_from_rfc3339(&stored.9)
    .map_err(|_| {
      DurableStoreError::WorkflowCallHistoryInvalid(
        "stored admission timestamp is invalid".to_string(),
      )
    })?
    .with_timezone(&Utc);
  Ok(Some(WorkflowCallAdmission {
    call_key: call_key.to_string(),
    parent_run_id: stored.0,
    parent_node_id: stored.1,
    parent_attempt,
    target_workflow_id: stored.3,
    target_definition_hash: stored.4,
    child_run_id: stored.5,
    payload_digest: stored.6,
    depth,
    state,
    admitted_at,
  }))
}

fn workflow_call_would_cycle(
  connection: &Connection,
  parent_run_id: &str,
  target_workflow_id: &str,
) -> Result<bool, DurableStoreError> {
  connection
    .query_row(
      "WITH RECURSIVE lineage(run_id, workflow_id) AS (
         SELECT run_id, workflow_id FROM woml_runs WHERE run_id = ?1
         UNION
         SELECT parent.parent_run_id, runs.workflow_id
         FROM lineage current
         JOIN woml_workflow_calls parent ON parent.child_run_id = current.run_id
         JOIN woml_runs runs ON runs.run_id = parent.parent_run_id
       )
       SELECT EXISTS(SELECT 1 FROM lineage WHERE workflow_id = ?2)",
      params![parent_run_id, target_workflow_id],
      |row| row.get(0),
    )
    .map_err(DurableStoreError::from)
}

fn workflow_call_state_name(state: WorkflowCallIndexState) -> &'static str {
  match state {
    WorkflowCallIndexState::Admitted => "admitted",
    WorkflowCallIndexState::Running => "running",
    WorkflowCallIndexState::Succeeded => "succeeded",
    WorkflowCallIndexState::Failed => "failed",
  }
}

fn validate_workflow_call_child(
  connection: &Connection,
  admission: &WorkflowCallAdmission,
  expected_trigger: &Map<String, Value>,
) -> Result<(), DurableStoreError> {
  let binding =
    load_run_binding_optional(connection, &admission.child_run_id)?.ok_or_else(|| {
      DurableStoreError::WorkflowCallHistoryInvalid("child run is missing".to_string())
    })?;
  if binding.workflow_id != admission.target_workflow_id
    || binding.definition_hash != admission.target_definition_hash
  {
    return Err(DurableStoreError::WorkflowCallHistoryInvalid(
      "child run binding differs from its call index".to_string(),
    ));
  }
  let workflow = definition_by_hash(connection, &binding.definition_hash)?;
  let events = load_events(connection, &binding.run_id)?;
  let Some(first) = events.first() else {
    return Err(DurableStoreError::WorkflowCallHistoryInvalid(
      "child run has no run_started event".to_string(),
    ));
  };
  let valid = match &first.payload {
    RunEventPayload::RunStarted(started) => {
      let valid_ingress = matches!(
        started.ingress.as_ref(),
        Some(RunIngress::WorkflowCall { call_key }) if call_key == &admission.call_key
      );
      matches!(
        first.event_schema_version,
        RUN_EVENT_SCHEMA_VERSION_V9 | crate::RUN_EVENT_SCHEMA_VERSION_V10
      ) && started.trigger == *expected_trigger
        && started.trigger_id.is_none()
        && started.trigger_handler.is_none()
        && started.trigger_occurrence_id.is_none()
        && valid_ingress
    }
    RunEventPayload::RunAdmitted(admitted) => {
      first.event_schema_version == RUN_EVENT_SCHEMA_VERSION_V11
        && admitted.payload == *expected_trigger
        && admitted.trigger.id == "workflow-call"
        && admitted.trigger.handler == "trigger.event"
    }
    _ => false,
  };
  if !valid {
    return Err(DurableStoreError::WorkflowCallHistoryInvalid(
      "child admission identity differs from its call index".to_string(),
    ));
  }
  validate_event_history_against_definition(&workflow, &binding.definition_hash, &events)
    .map_err(DurableStoreError::WorkflowCallHistoryInvalid)
}

fn ensure_run_exists(connection: &Connection, run_id: &str) -> Result<(), DurableStoreError> {
  let exists: bool = connection.query_row(
    "SELECT EXISTS(SELECT 1 FROM woml_runs WHERE run_id = ?1)",
    [run_id],
    |row| row.get(0),
  )?;
  if !exists {
    return Err(DurableStoreError::RunNotFound(run_id.to_string()));
  }
  Ok(())
}

fn validate_trigger_admission_request(
  request: &TriggerAdmissionRequest,
) -> Result<(), DurableStoreError> {
  if request.source_identity.is_empty() || request.source_identity.len() > 512 {
    return Err(DurableStoreError::Contract(
      "Trigger sourceIdentity must contain between 1 and 512 UTF-8 bytes.".to_string(),
    ));
  }
  if !is_definition_hash(&request.definition_hash) {
    return Err(DurableStoreError::TriggerDefinitionMismatch);
  }
  Ok(())
}

fn workflow_has_policy_capacity(
  connection: &Connection,
  workflow_id: &str,
  concurrency: Option<u32>,
) -> Result<bool, DurableStoreError> {
  let Some(limit) = concurrency else {
    return Ok(true);
  };
  let active: i64 = connection.query_row(
    "SELECT COUNT(*) FROM woml_scheduler_claims WHERE workflow_id = ?1",
    [workflow_id],
    |row| row.get(0),
  )?;
  Ok(active < i64::from(limit))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PolicyRateEligibility {
  Eligible,
  Waiting { eligible_at: DateTime<Utc> },
}

fn workflow_rate_eligibility(
  connection: &Connection,
  workflow_id: &str,
  rate_limit: Option<&crate::model::CompiledRateLimitPolicy>,
  now: DateTime<Utc>,
) -> Result<PolicyRateEligibility, DurableStoreError> {
  let Some(rate_limit) = rate_limit else {
    return Ok(PolicyRateEligibility::Eligible);
  };
  let window =
    chrono::Duration::milliseconds(i64::try_from(rate_limit.window_ms).map_err(|_| {
      DurableStoreError::Contract("Rate-limit window exceeds the runtime clock range.".to_string())
    })?);
  let cutoff = now - window;
  let starts = {
    let mut statement = connection.prepare(
      "SELECT started_at FROM woml_runtime_policy_starts
       WHERE workflow_id = ?1 AND started_at > ?2 ORDER BY started_at",
    )?;
    let rows = statement
      .query_map(params![workflow_id, cutoff.to_rfc3339()], |row| {
        row.get::<_, String>(0)
      })?
      .collect::<Result<Vec<_>, _>>()?;
    rows
  };
  let mut active = starts
    .iter()
    .map(|value| parse_durable_timestamp(value, "runtime-policy start"))
    .collect::<Result<Vec<_>, _>>()?
    .into_iter()
    // Future starts are deliberately retained so a wall-clock rollback fails
    // closed instead of granting extra capacity.
    .collect::<Vec<_>>();
  active.sort_unstable();
  if active.len() < rate_limit.count as usize {
    return Ok(PolicyRateEligibility::Eligible);
  }
  Ok(PolicyRateEligibility::Waiting {
    eligible_at: active[active.len() - rate_limit.count as usize] + window,
  })
}

fn earlier_eligible_policy_run_exists(
  connection: &Connection,
  run_id: &str,
  queue: &str,
  admitted_at: DateTime<Utc>,
  occurrence_sequence: u64,
  now: DateTime<Utc>,
) -> Result<bool, DurableStoreError> {
  let occurrence_sequence = i64::try_from(occurrence_sequence).map_err(|_| {
    DurableStoreError::Contract("occurrenceSequence exceeds SQLite integer range".to_string())
  })?;
  let earlier = {
    let mut statement = connection.prepare(
      "SELECT run_id, workflow_id
       FROM woml_runtime_policy_queue
       WHERE queue_name = ?1 AND run_id != ?2
         AND (
           admitted_at < ?3
           OR (admitted_at = ?3 AND occurrence_sequence < ?4)
           OR (admitted_at = ?3 AND occurrence_sequence = ?4 AND run_id < ?2)
         )
       ORDER BY admitted_at, occurrence_sequence, run_id",
    )?;
    let rows = statement
      .query_map(
        params![queue, run_id, admitted_at.to_rfc3339(), occurrence_sequence],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
      )?
      .collect::<Result<Vec<_>, _>>()?;
    rows
  };
  for (earlier_run_id, workflow_id) in earlier {
    let definition_hash: String = connection.query_row(
      "SELECT definition_hash FROM woml_runtime_policy_bindings WHERE run_id = ?1",
      [&earlier_run_id],
      |row| row.get(0),
    )?;
    let model_json: String = connection.query_row(
      "SELECT model_json FROM woml_definitions WHERE definition_hash = ?1",
      [&definition_hash],
      |row| row.get(0),
    )?;
    let workflow: CompiledWorkflowDefinition = serde_json::from_str(&model_json)?;
    let policy = workflow.runtime_policy.as_ref().ok_or_else(|| {
      DurableStoreError::Contract("Queued policy run has no Model v12 runtime policy.".to_string())
    })?;
    if workflow_has_policy_capacity(connection, &workflow_id, policy.concurrency)?
      && matches!(
        workflow_rate_eligibility(connection, &workflow_id, policy.rate_limit.as_ref(), now)?,
        PolicyRateEligibility::Eligible
      )
    {
      return Ok(true);
    }
  }
  Ok(false)
}

fn admit_trigger_occurrence_in_transaction(
  transaction: &Transaction<'_>,
  request: &TriggerAdmissionRequest,
) -> Result<TriggerAdmissionOutcome, DurableStoreError> {
  let source_identity_hash = sha256_prefixed(request.source_identity.as_bytes());
  let payload_hash = canonical_payload_hash(&request.payload)?;
  let model_json: String = transaction
    .query_row(
      "SELECT model_json FROM woml_definitions WHERE definition_hash = ?1",
      [&request.definition_hash],
      |row| row.get(0),
    )
    .optional()?
    .ok_or_else(|| DurableStoreError::DefinitionNotFound(request.definition_hash.clone()))?;
  let workflow: CompiledWorkflowDefinition = serde_json::from_str(&model_json)?;
  workflow.validate_structure()?;
  if workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V7
    || workflow.workflow_id != request.workflow_id
  {
    return Err(DurableStoreError::TriggerDefinitionMismatch);
  }
  let trigger =
    workflow
      .trigger(&request.trigger_id)
      .ok_or_else(|| DurableStoreError::TriggerNotFound {
        workflow_id: request.workflow_id.clone(),
        trigger_id: request.trigger_id.clone(),
      })?;
  if trigger.handler != request.trigger_handler {
    return Err(DurableStoreError::TriggerHandlerMismatch);
  }

  let runtime_policy_identity =
    if workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V12 {
      let policy_hash = workflow.runtime_policy_hash().ok_or_else(|| {
        DurableStoreError::Contract("Model v12 is missing its runtime policy identity.".to_string())
      })?;
      let queue_name = workflow.runtime_policy_queue_name().ok_or_else(|| {
        DurableStoreError::Contract("Model v12 is missing its runtime policy queue.".to_string())
      })?;
      let conflict: bool = transaction.query_row(
        "SELECT EXISTS(
         SELECT 1
         FROM woml_runtime_policy_bindings AS bindings
         JOIN woml_run_summaries AS summaries ON summaries.run_id = bindings.run_id
         WHERE bindings.workflow_id = ?1
           AND summaries.status IN ('queued', 'running', 'waiting', 'cancelling', 'finalizing')
           AND (bindings.definition_hash != ?2 OR bindings.policy_hash != ?3)
       )",
        params![request.workflow_id, request.definition_hash, policy_hash],
        |row| row.get(0),
      )?;
      if conflict {
        return Err(DurableStoreError::RuntimePolicyDefinitionConflict(
          request.workflow_id.clone(),
        ));
      }
      Some((policy_hash, queue_name))
    } else {
      None
    };

  if let Some(existing) = load_trigger_occurrence_by_identity(
    transaction,
    &request.workflow_id,
    &request.trigger_id,
    &source_identity_hash,
  )? {
    if existing.payload_hash != payload_hash {
      return Err(DurableStoreError::TriggerIdempotencyConflict);
    }
    validate_trigger_occurrence_history(transaction, &existing)?;
    return Ok(TriggerAdmissionOutcome {
      occurrence_id: existing.occurrence_id,
      run_id: existing.run_id,
      duplicate: true,
    });
  }

  if runtime_policy_identity.is_some() {
    let queued: i64 = transaction.query_row(
      "SELECT COUNT(*) FROM woml_runtime_policy_queue",
      [],
      |row| row.get(0),
    )?;
    if queued >= RUNTIME_POLICY_QUEUE_CEILING {
      return Err(DurableStoreError::RuntimePolicyQueueFull);
    }
  }

  let occurrence_id = format!("occ_{}", Uuid::new_v4().simple());
  let run_id = format!("run_{}", Uuid::new_v4().simple());
  transaction.execute(
    "INSERT INTO woml_runs(run_id, workflow_id, definition_hash, created_at)
     VALUES (?1, ?2, ?3, ?4)",
    params![
      run_id,
      request.workflow_id,
      request.definition_hash,
      request.received_at.to_rfc3339(),
    ],
  )?;

  let payload = if let Some((policy_hash, queue_name)) = runtime_policy_identity {
    RunEventPayload::RunAdmitted(RunAdmittedData {
      definition_hash: request.definition_hash.clone(),
      policy_hash,
      trigger: RunAdmissionTrigger {
        id: request.trigger_id.clone(),
        handler: request.trigger_handler.clone(),
      },
      payload: request.payload.clone(),
      queue: RunAdmissionQueue {
        name: queue_name,
        discipline: crate::model::QueueDiscipline::WorkConservingFifo,
      },
      admitted_at: request.received_at,
      occurrence_sequence: 0,
    })
  } else {
    RunEventPayload::RunStarted(RunStartedData {
      workflow_id: request.workflow_id.clone(),
      definition_hash: request.definition_hash.clone(),
      trigger_id: Some(request.trigger_id.clone()),
      trigger_handler: Some(request.trigger_handler.clone()),
      trigger_occurrence_id: Some(occurrence_id.clone()),
      ingress: None,
      trigger: request.payload.clone(),
    })
  };
  validate_payload_against_definition(&workflow, &request.definition_hash, &payload)
    .map_err(DurableStoreError::Contract)?;
  let mut events = Vec::new();
  append_to_history(
    transaction,
    &mut events,
    &run_id,
    generated_event_id(),
    request.received_at,
    run_event_schema_version_for_model(workflow.schema_version),
    payload,
  )?;
  validate_event_history_against_definition(&workflow, &request.definition_hash, &events)
    .map_err(DurableStoreError::Contract)?;

  transaction.execute(
    "INSERT INTO woml_trigger_occurrences(
       occurrence_id, workflow_id, trigger_id, trigger_handler,
       definition_hash, source_identity_hash, payload_hash, received_at, run_id
     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    params![
      occurrence_id,
      request.workflow_id,
      request.trigger_id,
      request.trigger_handler,
      request.definition_hash,
      source_identity_hash,
      payload_hash,
      request.received_at.to_rfc3339(),
      run_id,
    ],
  )?;
  Ok(TriggerAdmissionOutcome {
    occurrence_id,
    run_id,
    duplicate: false,
  })
}

fn validate_schedule_cursor_registration(
  registration: &ScheduleCursorRegistration,
) -> Result<(), DurableStoreError> {
  if registration.workflow_id.is_empty()
    || registration.trigger_id.is_empty()
    || registration.cron.is_empty()
    || registration.timezone.is_empty()
    || !matches!(registration.on_missed.as_str(), "skip" | "run-once")
    || !is_definition_hash(&registration.definition_hash)
  {
    return Err(DurableStoreError::Contract(
      "Schedule cursor registration is invalid.".to_string(),
    ));
  }
  Ok(())
}

fn validate_schedule_registration_definition(
  transaction: &Transaction<'_>,
  registration: &ScheduleCursorRegistration,
) -> Result<(), DurableStoreError> {
  let model_json: String = transaction
    .query_row(
      "SELECT model_json FROM woml_definitions WHERE definition_hash = ?1",
      [&registration.definition_hash],
      |row| row.get(0),
    )
    .optional()?
    .ok_or_else(|| DurableStoreError::DefinitionNotFound(registration.definition_hash.clone()))?;
  let workflow: CompiledWorkflowDefinition = serde_json::from_str(&model_json)?;
  if workflow.workflow_id != registration.workflow_id {
    return Err(DurableStoreError::TriggerDefinitionMismatch);
  }
  let trigger = workflow.trigger(&registration.trigger_id).ok_or_else(|| {
    DurableStoreError::TriggerNotFound {
      workflow_id: registration.workflow_id.clone(),
      trigger_id: registration.trigger_id.clone(),
    }
  })?;
  if trigger.handler != "trigger.schedule" {
    return Err(DurableStoreError::TriggerHandlerMismatch);
  }
  let crate::model::ValueExpression::Object { fields } = &trigger.config else {
    return Err(DurableStoreError::TriggerDefinitionMismatch);
  };
  let literal = |name: &str| match fields.get(name) {
    Some(crate::model::ValueExpression::Literal { value }) => value.as_str(),
    _ => None,
  };
  if literal("cron") != Some(registration.cron.as_str())
    || literal("timezone") != Some(registration.timezone.as_str())
    || literal("onMissed") != Some(registration.on_missed.as_str())
  {
    return Err(DurableStoreError::TriggerDefinitionMismatch);
  }
  Ok(())
}

fn load_schedule_cursor(
  connection: &Connection,
  workflow_id: &str,
  trigger_id: &str,
) -> Result<Option<ScheduleCursor>, DurableStoreError> {
  let stored = connection
    .query_row(
      "SELECT workflow_id, trigger_id, definition_hash, cron, timezone, on_missed,
              next_scheduled_at, updated_at
       FROM woml_schedule_cursors
       WHERE workflow_id = ?1 AND trigger_id = ?2",
      params![workflow_id, trigger_id],
      |row| {
        Ok((
          row.get::<_, String>(0)?,
          row.get::<_, String>(1)?,
          row.get::<_, String>(2)?,
          row.get::<_, String>(3)?,
          row.get::<_, String>(4)?,
          row.get::<_, String>(5)?,
          row.get::<_, String>(6)?,
          row.get::<_, String>(7)?,
        ))
      },
    )
    .optional()?;
  stored
    .map(|stored| {
      Ok(ScheduleCursor {
        workflow_id: stored.0,
        trigger_id: stored.1,
        definition_hash: stored.2,
        cron: stored.3,
        timezone: stored.4,
        on_missed: stored.5,
        next_scheduled_at: parse_durable_timestamp(&stored.6, "schedule next instant")?,
        updated_at: parse_durable_timestamp(&stored.7, "schedule update instant")?,
      })
    })
    .transpose()
}

fn update_schedule_cursor(
  transaction: &Transaction<'_>,
  workflow_id: &str,
  trigger_id: &str,
  expected_next_scheduled_at: DateTime<Utc>,
  next_scheduled_at: DateTime<Utc>,
  updated_at: DateTime<Utc>,
) -> Result<(), DurableStoreError> {
  if next_scheduled_at <= expected_next_scheduled_at {
    return Err(DurableStoreError::Contract(
      "A schedule cursor must advance to a later planned instant.".to_string(),
    ));
  }
  let changed = transaction.execute(
    "UPDATE woml_schedule_cursors
     SET next_scheduled_at = ?1, updated_at = ?2
     WHERE workflow_id = ?3 AND trigger_id = ?4 AND next_scheduled_at = ?5",
    params![
      next_scheduled_at.to_rfc3339(),
      updated_at.to_rfc3339(),
      workflow_id,
      trigger_id,
      expected_next_scheduled_at.to_rfc3339(),
    ],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::ScheduleCursorConflict);
  }
  Ok(())
}

fn validate_interval_cursor_registration(
  registration: &IntervalCursorRegistration,
) -> Result<(), DurableStoreError> {
  if registration.workflow_id.is_empty()
    || registration.trigger_id.is_empty()
    || !matches!(registration.on_missed.as_str(), "skip" | "run-once")
    || !is_definition_hash(&registration.definition_hash)
    || crate::interval::WomlInterval::new(registration.every_ms).is_err()
  {
    return Err(DurableStoreError::Contract(
      "Interval cursor registration is invalid.".to_string(),
    ));
  }
  Ok(())
}

fn validate_interval_registration_definition(
  transaction: &Transaction<'_>,
  registration: &IntervalCursorRegistration,
) -> Result<(), DurableStoreError> {
  let model_json: String = transaction
    .query_row(
      "SELECT model_json FROM woml_definitions WHERE definition_hash = ?1",
      [&registration.definition_hash],
      |row| row.get(0),
    )
    .optional()?
    .ok_or_else(|| DurableStoreError::DefinitionNotFound(registration.definition_hash.clone()))?;
  let workflow: CompiledWorkflowDefinition = serde_json::from_str(&model_json)?;
  if workflow.workflow_id != registration.workflow_id {
    return Err(DurableStoreError::TriggerDefinitionMismatch);
  }
  let trigger = workflow.trigger(&registration.trigger_id).ok_or_else(|| {
    DurableStoreError::TriggerNotFound {
      workflow_id: registration.workflow_id.clone(),
      trigger_id: registration.trigger_id.clone(),
    }
  })?;
  if trigger.handler != "trigger.interval" {
    return Err(DurableStoreError::TriggerHandlerMismatch);
  }
  let crate::model::ValueExpression::Object { fields } = &trigger.config else {
    return Err(DurableStoreError::TriggerDefinitionMismatch);
  };
  let every_ms = match fields.get("everyMs") {
    Some(crate::model::ValueExpression::Literal { value }) => value.as_u64(),
    _ => None,
  };
  let on_missed = match fields.get("onMissed") {
    Some(crate::model::ValueExpression::Literal { value }) => value.as_str(),
    _ => None,
  };
  if every_ms != Some(registration.every_ms) || on_missed != Some(registration.on_missed.as_str()) {
    return Err(DurableStoreError::TriggerDefinitionMismatch);
  }
  Ok(())
}

fn load_interval_cursor(
  connection: &Connection,
  workflow_id: &str,
  trigger_id: &str,
) -> Result<Option<IntervalCursor>, DurableStoreError> {
  let stored = connection
    .query_row(
      "SELECT workflow_id, trigger_id, definition_hash, every_ms, on_missed,
              anchor_at, next_sequence, next_scheduled_at, updated_at
       FROM woml_interval_cursors
       WHERE workflow_id = ?1 AND trigger_id = ?2",
      params![workflow_id, trigger_id],
      |row| {
        Ok((
          row.get::<_, String>(0)?,
          row.get::<_, String>(1)?,
          row.get::<_, String>(2)?,
          row.get::<_, i64>(3)?,
          row.get::<_, String>(4)?,
          row.get::<_, String>(5)?,
          row.get::<_, i64>(6)?,
          row.get::<_, String>(7)?,
          row.get::<_, String>(8)?,
        ))
      },
    )
    .optional()?;
  stored
    .map(|stored| {
      Ok(IntervalCursor {
        workflow_id: stored.0,
        trigger_id: stored.1,
        definition_hash: stored.2,
        every_ms: u64::try_from(stored.3).map_err(|_| {
          DurableStoreError::Contract("Stored interval duration is invalid.".to_string())
        })?,
        on_missed: stored.4,
        anchor_at: parse_durable_timestamp(&stored.5, "interval anchor")?,
        next_sequence: u64::try_from(stored.6).map_err(|_| {
          DurableStoreError::Contract("Stored interval sequence is invalid.".to_string())
        })?,
        next_scheduled_at: parse_durable_timestamp(&stored.7, "interval next instant")?,
        updated_at: parse_durable_timestamp(&stored.8, "interval update instant")?,
      })
    })
    .transpose()
}

#[allow(clippy::too_many_arguments)]
fn update_interval_cursor(
  transaction: &Transaction<'_>,
  workflow_id: &str,
  trigger_id: &str,
  expected_sequence: u64,
  expected_scheduled_at: DateTime<Utc>,
  next_sequence: u64,
  next_scheduled_at: DateTime<Utc>,
  updated_at: DateTime<Utc>,
) -> Result<(), DurableStoreError> {
  if next_sequence <= expected_sequence || next_scheduled_at <= expected_scheduled_at {
    return Err(DurableStoreError::Contract(
      "An interval cursor must advance to a later sequence and planned instant.".to_string(),
    ));
  }
  let expected_sequence = i64::try_from(expected_sequence).map_err(|_| {
    DurableStoreError::Contract("Interval sequence exceeds the durable integer range.".to_string())
  })?;
  let next_sequence = i64::try_from(next_sequence).map_err(|_| {
    DurableStoreError::Contract("Interval sequence exceeds the durable integer range.".to_string())
  })?;
  let changed = transaction.execute(
    "UPDATE woml_interval_cursors
     SET next_sequence = ?1, next_scheduled_at = ?2, updated_at = ?3
     WHERE workflow_id = ?4 AND trigger_id = ?5
       AND next_sequence = ?6 AND next_scheduled_at = ?7",
    params![
      next_sequence,
      next_scheduled_at.to_rfc3339(),
      updated_at.to_rfc3339(),
      workflow_id,
      trigger_id,
      expected_sequence,
      expected_scheduled_at.to_rfc3339(),
    ],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::IntervalCursorConflict);
  }
  Ok(())
}

fn parse_durable_timestamp(value: &str, label: &str) -> Result<DateTime<Utc>, DurableStoreError> {
  DateTime::parse_from_rfc3339(value)
    .map(|value| value.with_timezone(&Utc))
    .map_err(|_| DurableStoreError::Contract(format!("Stored {label} is not RFC 3339.")))
}

type StoredTriggerOccurrence = (
  String,
  String,
  String,
  String,
  String,
  String,
  String,
  String,
  String,
);

fn trigger_occurrence_from_stored(
  stored: StoredTriggerOccurrence,
) -> Result<TriggerOccurrence, DurableStoreError> {
  let valid_id = |value: &str| !value.is_empty() && value.chars().count() <= 256;
  let valid_handler = matches!(
    stored.3.as_str(),
    "trigger.manual"
      | "trigger.webhook"
      | "trigger.slack"
      | "trigger.telegram"
      | "trigger.discord"
      | "trigger.whatsapp"
      | "trigger.schedule"
      | "trigger.interval"
      | "trigger.event"
  );
  if !valid_id(&stored.0)
    || !valid_id(&stored.1)
    || !valid_id(&stored.2)
    || !valid_handler
    || !is_definition_hash(&stored.4)
    || !is_definition_hash(&stored.5)
    || !is_definition_hash(&stored.6)
    || !valid_id(&stored.8)
  {
    return Err(DurableStoreError::TriggerHistoryInvalid(
      "trigger occurrence contains an invalid identity or hash".to_string(),
    ));
  }
  let received_at = DateTime::parse_from_rfc3339(&stored.7)
    .map_err(|_| {
      DurableStoreError::TriggerHistoryInvalid(
        "trigger occurrence receivedAt is not RFC 3339".to_string(),
      )
    })?
    .with_timezone(&Utc);
  Ok(TriggerOccurrence {
    occurrence_schema_version: 1,
    occurrence_id: stored.0,
    workflow_id: stored.1,
    trigger_id: stored.2,
    trigger_handler: stored.3,
    definition_hash: stored.4,
    source_identity_hash: stored.5,
    payload_hash: stored.6,
    received_at,
    run_id: stored.8,
  })
}

fn load_trigger_occurrence_by_id(
  connection: &Connection,
  occurrence_id: &str,
) -> Result<Option<TriggerOccurrence>, DurableStoreError> {
  let stored: Option<StoredTriggerOccurrence> = connection
    .query_row(
      "SELECT occurrence_id, workflow_id, trigger_id, trigger_handler,
              definition_hash, source_identity_hash, payload_hash, received_at, run_id
       FROM woml_trigger_occurrences WHERE occurrence_id = ?1",
      [occurrence_id],
      |row| {
        Ok((
          row.get(0)?,
          row.get(1)?,
          row.get(2)?,
          row.get(3)?,
          row.get(4)?,
          row.get(5)?,
          row.get(6)?,
          row.get(7)?,
          row.get(8)?,
        ))
      },
    )
    .optional()?;
  stored.map(trigger_occurrence_from_stored).transpose()
}

fn load_trigger_occurrence_by_identity(
  connection: &Connection,
  workflow_id: &str,
  trigger_id: &str,
  source_identity_hash: &str,
) -> Result<Option<TriggerOccurrence>, DurableStoreError> {
  let occurrence_id: Option<String> = connection
    .query_row(
      "SELECT occurrence_id FROM woml_trigger_occurrences
       WHERE workflow_id = ?1 AND trigger_id = ?2 AND source_identity_hash = ?3",
      params![workflow_id, trigger_id, source_identity_hash],
      |row| row.get(0),
    )
    .optional()?;
  occurrence_id
    .map(|occurrence_id| load_trigger_occurrence_by_id(connection, &occurrence_id))
    .transpose()
    .map(Option::flatten)
}

fn sha256_prefixed(bytes: &[u8]) -> String {
  format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

fn valid_external_signal_name(value: &str) -> bool {
  if value.is_empty() || value.len() > 128 {
    return false;
  }
  let mut characters = value.chars();
  if !matches!(characters.next(), Some(first) if first.is_ascii_lowercase()) {
    return false;
  }
  let mut saw_separator = false;
  let mut previous_was_separator = false;
  for character in characters {
    if matches!(character, '.' | '_' | '-') {
      if previous_was_separator {
        return false;
      }
      saw_separator = true;
      previous_was_separator = true;
    } else if character.is_ascii_lowercase() || character.is_ascii_digit() {
      previous_was_separator = false;
    } else {
      return false;
    }
  }
  saw_separator && !previous_was_separator
}

fn valid_bounded_identity(value: &str, maximum: usize) -> bool {
  !value.is_empty()
    && value.len() <= maximum
    && value.trim() == value
    && !value.chars().any(char::is_control)
}

fn valid_public_wait_identity(value: &str) -> bool {
  value.len() <= 128 && {
    let mut characters = value.chars();
    matches!(characters.next(), Some(first) if first.is_ascii_lowercase())
      && characters.all(|character| character.is_ascii_alphanumeric())
  }
}

fn valid_bounded_signal_value(value: &str, maximum: usize) -> bool {
  valid_bounded_identity(value, maximum)
}

fn json_depth(value: &Value) -> usize {
  match value {
    Value::Array(values) => 1 + values.iter().map(json_depth).max().unwrap_or(0),
    Value::Object(values) => 1 + values.values().map(json_depth).max().unwrap_or(0),
    _ => 1,
  }
}

fn resolve_signal_correlation(
  expression: &ValueExpression,
  context: &crate::WorkflowContext,
) -> Option<String> {
  let value = match expression {
    ValueExpression::Literal { value } => value.clone(),
    ValueExpression::ContextReference { .. } => {
      resolve_context_reference(expression, context).ok()?
    }
    _ => return None,
  };
  value
    .as_str()
    .filter(|value| valid_bounded_signal_value(value, 256))
    .map(str::to_string)
}

#[allow(clippy::too_many_arguments)]
fn append_signal_resolution(
  transaction: &Transaction<'_>,
  workflow: &CompiledWorkflowDefinition,
  definition_hash: &str,
  events: &mut Vec<RunEvent>,
  run_id: &str,
  wait_id: &str,
  signal_id: &str,
  signal: &str,
  correlation: &str,
  idempotency_digest: &str,
  authentication_class: &str,
  received_at: DateTime<Utc>,
  resolved_at: DateTime<Utc>,
  payload: Value,
) -> Result<RunProjection, DurableStoreError> {
  let event_schema_version = events
    .first()
    .map(|event| event.event_schema_version)
    .ok_or_else(|| DurableStoreError::Contract("Signal target has no run history.".to_string()))?;
  append_to_history(
    transaction,
    events,
    run_id,
    generated_event_id(),
    received_at,
    event_schema_version,
    RunEventPayload::ExternalSignalReceived(crate::ExternalSignalReceivedData {
      signal_id: signal_id.to_string(),
      wait_id: wait_id.to_string(),
      signal: signal.to_string(),
      received_at,
      idempotency_digest: idempotency_digest.to_string(),
      payload: payload.clone(),
      correlation: Some(correlation.to_string()),
      authentication_class: Some(authentication_class.to_string()),
    }),
  )?;
  let resolution_event_id = generated_event_id();
  append_to_history(
    transaction,
    events,
    run_id,
    resolution_event_id.clone(),
    resolved_at,
    event_schema_version,
    RunEventPayload::WaitSatisfied(crate::WaitSatisfiedData {
      wait_id: wait_id.to_string(),
      winner: crate::DurableWaitWinner::Signal,
      result: serde_json::json!({
        "status": "signaled",
        "signal": signal,
        "signalId": signal_id,
        "correlation": correlation,
        "receivedAt": received_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "payload": payload,
      }),
      resolved_at,
    }),
  )?;
  validate_event_history_against_definition(workflow, definition_hash, events)
    .map_err(DurableStoreError::Contract)?;
  let changed = transaction.execute(
    "UPDATE woml_durable_waits
     SET state = 'satisfied', resolution_event_id = ?1, resolved_at = ?2
     WHERE run_id = ?3 AND wait_id = ?4 AND iteration_identity = '' AND state = 'registered'",
    params![
      resolution_event_id,
      resolved_at.to_rfc3339(),
      run_id,
      wait_id
    ],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::Contract(
      "Signal resolution lost the durable wait winner transaction.".to_string(),
    ));
  }
  transaction.execute(
    "UPDATE woml_external_signals SET consumed_at = ?1
     WHERE signal_id = ?2 AND consumed_at IS NULL",
    params![resolved_at.to_rfc3339(), signal_id],
  )?;
  let projection = fold_events(events)?;
  if projection.status != RunStatus::Running || !projection.context.steps.contains_key(wait_id) {
    return Err(DurableStoreError::Contract(
      "Signal settlement did not publish the wait result into context.".to_string(),
    ));
  }
  Ok(projection)
}

fn canonical_payload_hash(payload: &Map<String, Value>) -> Result<String, DurableStoreError> {
  let canonical = canonical_json(payload)?;
  Ok(sha256_prefixed(&canonical))
}

fn validate_trigger_occurrence_history(
  connection: &Connection,
  occurrence: &TriggerOccurrence,
) -> Result<Map<String, Value>, DurableStoreError> {
  let binding = run_binding_in_transaction(connection, &occurrence.run_id).map_err(|error| {
    DurableStoreError::TriggerHistoryInvalid(format!(
      "occurrence run binding is missing or invalid: {error}"
    ))
  })?;
  if binding.workflow_id != occurrence.workflow_id
    || binding.definition_hash != occurrence.definition_hash
  {
    return Err(DurableStoreError::TriggerHistoryInvalid(
      "occurrence does not match its immutable run binding".to_string(),
    ));
  }
  let workflow = definition_for_run(connection, &occurrence.run_id).map_err(|error| {
    DurableStoreError::TriggerHistoryInvalid(format!(
      "occurrence definition is missing or invalid: {error}"
    ))
  })?;
  let events = load_events(connection, &occurrence.run_id).map_err(|error| {
    DurableStoreError::TriggerHistoryInvalid(format!(
      "occurrence event history cannot be loaded: {error}"
    ))
  })?;
  validate_event_history_against_definition(&workflow, &occurrence.definition_hash, &events)
    .map_err(DurableStoreError::TriggerHistoryInvalid)?;
  let Some(first) = events.first() else {
    return Err(DurableStoreError::TriggerHistoryInvalid(
      "occurrence run has no admission event".to_string(),
    ));
  };
  let payload = match &first.payload {
    RunEventPayload::RunAdmitted(admission)
      if matches!(
        first.event_schema_version,
        RUN_EVENT_SCHEMA_VERSION_V11
          | RUN_EVENT_SCHEMA_VERSION_V12
          | RUN_EVENT_SCHEMA_VERSION_V13
          | RUN_EVENT_SCHEMA_VERSION_V14
          | crate::RUN_EVENT_SCHEMA_VERSION_V15
          | crate::RUN_EVENT_SCHEMA_VERSION_V16
          | crate::RUN_EVENT_SCHEMA_VERSION_V17
          | crate::RUN_EVENT_SCHEMA_VERSION_V18
          | crate::RUN_EVENT_SCHEMA_VERSION_V19
          | crate::RUN_EVENT_SCHEMA_VERSION_V20
      ) && admission.definition_hash == occurrence.definition_hash
        && admission.trigger.id == occurrence.trigger_id
        && admission.trigger.handler == occurrence.trigger_handler
        && first.occurred_at == occurrence.received_at
        && admission.admitted_at == occurrence.received_at =>
    {
      &admission.payload
    }
    RunEventPayload::RunStarted(start)
      if matches!(
        first.event_schema_version,
        RUN_EVENT_SCHEMA_VERSION_V7
          | RUN_EVENT_SCHEMA_VERSION_V8
          | RUN_EVENT_SCHEMA_VERSION_V9
          | crate::RUN_EVENT_SCHEMA_VERSION_V10
      ) && start.workflow_id == occurrence.workflow_id
        && start.definition_hash == occurrence.definition_hash
        && start.trigger_id.as_deref() == Some(occurrence.trigger_id.as_str())
        && start.trigger_handler.as_deref() == Some(occurrence.trigger_handler.as_str())
        && start.trigger_occurrence_id.as_deref() == Some(occurrence.occurrence_id.as_str())
        && first.occurred_at == occurrence.received_at =>
    {
      &start.trigger
    }
    _ => {
      return Err(DurableStoreError::TriggerHistoryInvalid(
        "occurrence fields contradict its durable admission event".to_string(),
      ));
    }
  };
  if canonical_payload_hash(payload)? != occurrence.payload_hash {
    return Err(DurableStoreError::TriggerHistoryInvalid(
      "occurrence payload contradicts its durable admission event".to_string(),
    ));
  }
  Ok(payload.clone())
}

fn run_binding_in_transaction(
  connection: &Connection,
  run_id: &str,
) -> Result<RunDefinitionBinding, DurableStoreError> {
  let mut statement = connection
    .prepare_cached("SELECT workflow_id, definition_hash FROM woml_runs WHERE run_id = ?1")?;
  statement
    .query_row([run_id], |row| {
      Ok(RunDefinitionBinding {
        run_id: run_id.to_string(),
        workflow_id: row.get(0)?,
        definition_hash: row.get(1)?,
      })
    })
    .optional()?
    .ok_or_else(|| DurableStoreError::RunNotFound(run_id.to_string()))
}

fn definition_for_run(
  connection: &Connection,
  run_id: &str,
) -> Result<CompiledWorkflowDefinition, DurableStoreError> {
  let mut statement = connection.prepare_cached(
    "SELECT definitions.model_json
     FROM woml_runs AS runs
     JOIN woml_definitions AS definitions
       ON definitions.definition_hash = runs.definition_hash
     WHERE runs.run_id = ?1",
  )?;
  let model_json: String = statement
    .query_row([run_id], |row| row.get(0))
    .optional()?
    .ok_or_else(|| DurableStoreError::RunNotFound(run_id.to_string()))?;
  let workflow: CompiledWorkflowDefinition = serde_json::from_str(&model_json)?;
  workflow.validate_structure()?;
  Ok(workflow)
}

fn load_events(connection: &Connection, run_id: &str) -> Result<Vec<RunEvent>, DurableStoreError> {
  let mut statement = connection.prepare_cached(
    "SELECT sequence, event_schema_version, event_json
     FROM woml_run_events WHERE run_id = ?1 ORDER BY sequence",
  )?;
  let mut rows = statement.query([run_id])?;
  let mut events = Vec::new();
  while let Some(row) = rows.next()? {
    let stored_sequence: i64 = row.get(0)?;
    let stored_schema_version: i64 = row.get(1)?;
    let event_json: String = row.get(2)?;
    let event: RunEvent = serde_json::from_str(&event_json)?;
    if i64::try_from(event.sequence).ok() != Some(stored_sequence)
      || i64::from(event.event_schema_version) != stored_schema_version
      || event.run_id != run_id
    {
      return Err(DurableStoreError::InvalidStoredEvent(
        "indexed event columns do not match event_json".to_string(),
      ));
    }
    event
      .validate()
      .map_err(|error| DurableStoreError::InvalidStoredEvent(error.to_string()))?;
    events.push(event);
  }
  Ok(events)
}

pub(crate) fn lifecycle_warnings_from_projection(
  projection: &RunProjection,
) -> Vec<crate::event::LifecycleWarning> {
  projection
    .lifecycle_hooks
    .values()
    .flat_map(|hook| {
      hook.actions.values().filter_map(|action| {
        let failure = action.failure.as_ref()?;
        Some(crate::event::LifecycleWarning {
          hook_id: hook.hook_id.clone(),
          action_id: action.action_id.clone(),
          step_id: matches!(hook.subject.kind, LifecycleSubjectKind::Step)
            .then(|| hook.subject.id.clone()),
          provider: None,
          destination: None,
          code: failure.code.clone(),
        })
      })
    })
    .collect()
}

fn lifecycle_failure_from_attempt(failure: &AttemptFailure) -> crate::event::LifecycleFailure {
  let kind = match failure.kind {
    AttemptFailureKind::ScriptThrew => crate::event::LifecycleFailureKind::ScriptThrew,
    AttemptFailureKind::ScriptTimedOut => crate::event::LifecycleFailureKind::TimedOut,
    AttemptFailureKind::InvalidScriptResult => crate::event::LifecycleFailureKind::NonJson,
    AttemptFailureKind::ContextTooLarge | AttemptFailureKind::ResultTooLarge => {
      crate::event::LifecycleFailureKind::SizeLimitExceeded
    }
    AttemptFailureKind::WorkerCrashed => crate::event::LifecycleFailureKind::WorkerCrashed,
    AttemptFailureKind::HostCrashed => crate::event::LifecycleFailureKind::HostCrashed,
    AttemptFailureKind::InvocationCancelled => crate::event::LifecycleFailureKind::Cancelled,
    AttemptFailureKind::Interrupted => crate::event::LifecycleFailureKind::Interrupted,
    AttemptFailureKind::ServiceFailed => crate::event::LifecycleFailureKind::ProviderFailed,
  };
  crate::event::LifecycleFailure {
    kind,
    code: failure.code.clone(),
    message: failure.message.clone(),
  }
}

fn lifecycle_failure_from_legacy(data: &RunFailedData) -> crate::event::LifecycleFailure {
  match data {
    RunFailedData::V1(data) => lifecycle_failure_from_attempt(&data.failure),
    RunFailedData::V2(RunFailedDataV2::Attempt { failure, .. }) => {
      lifecycle_failure_from_attempt(failure)
    }
    RunFailedData::V2(RunFailedDataV2::Branch { failure, .. }) => {
      let message = match failure {
        crate::event::BranchFailure::BranchTestNotBoolean { message, .. }
        | crate::event::BranchFailure::ReferenceNotAvailable { message, .. }
        | crate::event::BranchFailure::BranchSelectionInvalid { message, .. } => message.clone(),
      };
      crate::event::LifecycleFailure {
        kind: crate::event::LifecycleFailureKind::ProviderFailed,
        code: failure.code().to_string(),
        message,
      }
    }
    RunFailedData::V3(RunFailedDataV3::Parallel { failure, .. }) => {
      crate::event::LifecycleFailure {
        kind: crate::event::LifecycleFailureKind::ProviderFailed,
        code: failure.code.clone(),
        message: failure.message.clone(),
      }
    }
    RunFailedData::V4(RunFailedDataV4::Approval { failure, .. }) => {
      crate::event::LifecycleFailure {
        kind: crate::event::LifecycleFailureKind::TimedOut,
        code: failure.code.clone(),
        message: failure.message.clone(),
      }
    }
    RunFailedData::V5(RunFailedDataV5::Notification { failure, .. }) => {
      crate::event::LifecycleFailure {
        kind: crate::event::LifecycleFailureKind::ProviderFailed,
        code: failure.code.clone(),
        message: failure.message.clone(),
      }
    }
  }
}

pub(crate) fn expand_model_v11_payload(
  workflow: &CompiledWorkflowDefinition,
  run_id: &str,
  payload: RunEventPayload,
) -> Result<Vec<RunEventPayload>, DurableStoreError> {
  fn step_hook_request(
    workflow: &CompiledWorkflowDefinition,
    run_id: &str,
    event: crate::model::LifecycleEventName,
    step_id: &str,
  ) -> Option<RunEventPayload> {
    let hook = workflow.lifecycle_hook_for_step_event(event, step_id)?;
    let subject = LifecycleSubject {
      kind: LifecycleSubjectKind::Step,
      id: step_id.to_string(),
    };
    Some(RunEventPayload::LifecycleHookRequested(
      LifecycleHookRequestedData {
        hook_invocation_id: derive_lifecycle_hook_invocation_id(
          run_id,
          &hook.hook_id,
          subject.kind,
          &subject.id,
        ),
        hook_id: hook.hook_id.clone(),
        event: hook.event,
        subject,
      },
    ))
  }

  let payload = if let RunEventPayload::RunStarted(data) = payload {
    let mut payloads = vec![RunEventPayload::RunStarted(data)];
    if let Some(hook) =
      workflow.lifecycle_hook_for_event(crate::model::LifecycleEventName::RunStart)
    {
      let subject = LifecycleSubject {
        kind: LifecycleSubjectKind::Workflow,
        id: run_id.to_string(),
      };
      payloads.push(RunEventPayload::LifecycleHookRequested(
        LifecycleHookRequestedData {
          hook_invocation_id: derive_lifecycle_hook_invocation_id(
            run_id,
            &hook.hook_id,
            subject.kind,
            &subject.id,
          ),
          hook_id: hook.hook_id.clone(),
          event: hook.event,
          subject,
        },
      ));
    }
    return Ok(payloads);
  } else {
    payload
  };
  let payload = match payload {
    RunEventPayload::StepAttemptStarted(data) if data.attempt == 1 => {
      let request = step_hook_request(
        workflow,
        run_id,
        crate::model::LifecycleEventName::StepStart,
        &data.node_id,
      );
      let mut payloads = vec![RunEventPayload::StepAttemptStarted(data)];
      payloads.extend(request);
      return Ok(payloads);
    }
    RunEventPayload::StepAttemptSucceeded(data) => {
      let request = step_hook_request(
        workflow,
        run_id,
        crate::model::LifecycleEventName::StepSuccess,
        &data.node_id,
      )
      .or_else(|| {
        step_hook_request(
          workflow,
          run_id,
          crate::model::LifecycleEventName::StepComplete,
          &data.node_id,
        )
      });
      let mut payloads = vec![RunEventPayload::StepAttemptSucceeded(data)];
      payloads.extend(request);
      return Ok(payloads);
    }
    RunEventPayload::StepAttemptFailed(data) => {
      let retryable = workflow
        .node(&data.node_id)
        .and_then(|node| node.retry_policy.as_ref())
        .is_some_and(|policy| {
          data.failure.kind == AttemptFailureKind::ScriptThrew && data.attempt < policy.max_attempts
        });
      if retryable {
        return Ok(vec![RunEventPayload::StepAttemptFailed(data)]);
      }
      let event = if data.failure.kind == AttemptFailureKind::InvocationCancelled {
        crate::model::LifecycleEventName::StepComplete
      } else {
        crate::model::LifecycleEventName::StepFailure
      };
      let request = step_hook_request(workflow, run_id, event, &data.node_id).or_else(|| {
        (event != crate::model::LifecycleEventName::StepComplete)
          .then(|| {
            step_hook_request(
              workflow,
              run_id,
              crate::model::LifecycleEventName::StepComplete,
              &data.node_id,
            )
          })
          .flatten()
      });
      let mut payloads = vec![RunEventPayload::StepAttemptFailed(data)];
      payloads.extend(request);
      return Ok(payloads);
    }
    payload => payload,
  };
  let outcome = match payload {
    RunEventPayload::RunSucceeded(data) => crate::event::RunOutcomeDecidedData::Succeeded {
      result: data.result,
    },
    RunEventPayload::RunFailed(data) => crate::event::RunOutcomeDecidedData::Failed {
      failure: lifecycle_failure_from_legacy(&data),
    },
    payload => return Ok(vec![payload]),
  };
  let business_outcome = outcome.outcome();
  let lifecycle_event = match business_outcome {
    crate::event::BusinessOutcome::Succeeded => crate::model::LifecycleEventName::RunSuccess,
    crate::event::BusinessOutcome::Failed => crate::model::LifecycleEventName::RunFailure,
    crate::event::BusinessOutcome::Cancelled => crate::model::LifecycleEventName::RunCancel,
  };
  let mut payloads = vec![RunEventPayload::RunOutcomeDecided(outcome)];
  let next_hook = workflow
    .lifecycle_hook_for_event(lifecycle_event)
    .or_else(|| workflow.lifecycle_hook_for_event(crate::model::LifecycleEventName::RunComplete));
  if let Some(hook) = next_hook {
    let subject = LifecycleSubject {
      kind: LifecycleSubjectKind::Workflow,
      id: run_id.to_string(),
    };
    payloads.push(RunEventPayload::LifecycleHookRequested(
      LifecycleHookRequestedData {
        hook_invocation_id: derive_lifecycle_hook_invocation_id(
          run_id,
          &hook.hook_id,
          subject.kind,
          &subject.id,
        ),
        hook_id: hook.hook_id.clone(),
        event: hook.event,
        subject,
      },
    ));
  } else if workflow.lifecycle.is_none() {
    payloads.push(RunEventPayload::RunFinalized(
      crate::event::RunFinalizedData {
        outcome: business_outcome,
        lifecycle_status: crate::event::FinalLifecycleStatus::Completed,
        warnings: Vec::new(),
      },
    ));
  }
  Ok(payloads)
}

fn write_run_summary(
  connection: &Connection,
  events: &[RunEvent],
) -> Result<(), DurableStoreError> {
  let Some(first) = events.first() else {
    return Ok(());
  };
  let projection = fold_events(events)?;
  write_run_summary_from_projection(
    connection,
    first,
    events.last().unwrap().occurred_at,
    &projection,
  )
}

fn write_run_summary_from_projection(
  connection: &Connection,
  first: &RunEvent,
  updated_at: DateTime<Utc>,
  projection: &RunProjection,
) -> Result<(), DurableStoreError> {
  let workflow_id: String = connection.query_row(
    "SELECT workflow_id FROM woml_runs WHERE run_id = ?1",
    [&first.run_id],
    |row| row.get(0),
  )?;
  connection.execute(
    "INSERT INTO woml_run_summaries(
       run_id, workflow_id, status, admitted_at, started_at, updated_at,
       queue_name, waiting_for, eligible_at
     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL)
     ON CONFLICT(run_id) DO UPDATE SET
       workflow_id = excluded.workflow_id,
       status = excluded.status,
       admitted_at = excluded.admitted_at,
       started_at = excluded.started_at,
       updated_at = excluded.updated_at,
       queue_name = excluded.queue_name",
    params![
      first.run_id,
      workflow_id,
      PublicRunStatus::from(projection.status).as_str(),
      first.occurred_at.to_rfc3339(),
      projection.started_at.map(|value| value.to_rfc3339()),
      updated_at.to_rfc3339(),
      projection.queue,
    ],
  )?;
  Ok(())
}

fn write_runtime_policy_indexes_for_run(
  connection: &Connection,
  events: &[RunEvent],
) -> Result<(), DurableStoreError> {
  let Some(RunEvent {
    run_id,
    payload: RunEventPayload::RunAdmitted(admission),
    ..
  }) = events.first()
  else {
    return Ok(());
  };
  let projection = fold_events(events)?;
  write_runtime_policy_indexes_from_projection(connection, run_id, admission, &projection)
}

fn write_runtime_policy_indexes_from_projection(
  connection: &Connection,
  run_id: &str,
  admission: &RunAdmittedData,
  projection: &RunProjection,
) -> Result<(), DurableStoreError> {
  let workflow_id: String = connection.query_row(
    "SELECT workflow_id FROM woml_runs WHERE run_id = ?1",
    [run_id],
    |row| row.get(0),
  )?;
  let occurrence_sequence = i64::try_from(admission.occurrence_sequence).map_err(|_| {
    DurableStoreError::Contract("occurrenceSequence exceeds SQLite integer range".to_string())
  })?;
  connection.execute(
    "INSERT INTO woml_runtime_policy_bindings(
       run_id, workflow_id, definition_hash, policy_hash, queue_name, admitted_at,
       occurrence_sequence
     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
     ON CONFLICT(run_id) DO UPDATE SET
       workflow_id = excluded.workflow_id,
       definition_hash = excluded.definition_hash,
       policy_hash = excluded.policy_hash,
       queue_name = excluded.queue_name,
       admitted_at = excluded.admitted_at,
       occurrence_sequence = excluded.occurrence_sequence",
    params![
      run_id,
      workflow_id,
      admission.definition_hash,
      admission.policy_hash,
      admission.queue.name,
      admission.admitted_at.to_rfc3339(),
      occurrence_sequence,
    ],
  )?;
  connection.execute(
    "DELETE FROM woml_runtime_policy_queue WHERE run_id = ?1",
    [run_id],
  )?;
  connection.execute(
    "DELETE FROM woml_runtime_policy_starts WHERE run_id = ?1",
    [run_id],
  )?;
  if projection.status == RunStatus::Queued {
    connection.execute(
      "INSERT INTO woml_runtime_policy_queue(
         run_id, workflow_id, queue_name, admitted_at, occurrence_sequence
       ) VALUES (?1, ?2, ?3, ?4, ?5)",
      params![
        run_id,
        workflow_id,
        admission.queue.name,
        admission.admitted_at.to_rfc3339(),
        occurrence_sequence,
      ],
    )?;
  }
  if let Some(started_at) = projection.started_at {
    connection.execute(
      "INSERT INTO woml_runtime_policy_starts(run_id, workflow_id, started_at)
       VALUES (?1, ?2, ?3)",
      params![run_id, workflow_id, started_at.to_rfc3339()],
    )?;
  }
  Ok(())
}

fn rebuild_runtime_policy_indexes(connection: &Connection) -> Result<(), DurableStoreError> {
  connection.execute("DELETE FROM woml_runtime_policy_queue", [])?;
  connection.execute("DELETE FROM woml_runtime_policy_starts", [])?;
  connection.execute("DELETE FROM woml_runtime_policy_bindings", [])?;
  let run_ids = {
    let mut statement = connection.prepare("SELECT run_id FROM woml_runs ORDER BY run_id")?;
    let rows = statement
      .query_map([], |row| row.get::<_, String>(0))?
      .collect::<Result<Vec<_>, _>>()?;
    rows
  };
  for run_id in run_ids {
    let events = load_events(connection, &run_id)?;
    write_runtime_policy_indexes_for_run(connection, &events)?;
  }
  Ok(())
}

fn rebuild_run_summary_index(connection: &Connection) -> Result<(), DurableStoreError> {
  connection.execute("DELETE FROM woml_run_summaries", [])?;
  let run_ids = {
    let mut statement = connection.prepare("SELECT run_id FROM woml_runs ORDER BY run_id")?;
    let rows = statement
      .query_map([], |row| row.get::<_, String>(0))?
      .collect::<Result<Vec<_>, _>>()?;
    rows
  };
  for run_id in run_ids {
    let events = load_events(connection, &run_id)?;
    write_run_summary(connection, &events)?;
  }
  Ok(())
}

pub fn correction_invocation_id(
  run_id: &str,
  saga_id: &str,
  action_id: &str,
  attempt: u32,
) -> String {
  let mut digest = Sha256::new();
  digest.update(b"woml:correction-invocation:v1\0");
  for value in [run_id, saga_id, action_id] {
    digest.update(value.as_bytes());
    digest.update([0]);
  }
  digest.update(attempt.to_be_bytes());
  format!("corr_{}", hex::encode(digest.finalize()))
}

fn correction_action_kind_name(kind: crate::CorrectiveActionKind) -> &'static str {
  match kind {
    crate::CorrectiveActionKind::Compensation => "compensation",
    crate::CorrectiveActionKind::Cleanup => "cleanup",
  }
}

fn write_correction_indexes_from_projection(
  connection: &Connection,
  run_id: &str,
  definition_hash: &str,
  events: &[RunEvent],
  projection: &RunProjection,
) -> Result<(), DurableStoreError> {
  let externally_resolved = {
    let mut statement = connection.prepare(
      "SELECT saga_id, action_id, reason_code, created_at, resolved_at
       FROM woml_correction_manual_recovery
       WHERE run_id = ?1 AND status = 'externally_resolved'",
    )?;
    let rows = statement
      .query_map([run_id], |row| {
        Ok((
          row.get::<_, String>(0)?,
          row.get::<_, String>(1)?,
          row.get::<_, String>(2)?,
          row.get::<_, String>(3)?,
          row.get::<_, Option<String>>(4)?,
        ))
      })?
      .collect::<Result<Vec<_>, _>>()?;
    rows
  };
  connection.execute(
    "DELETE FROM woml_correction_manual_recovery WHERE run_id = ?1",
    [run_id],
  )?;
  connection.execute(
    "DELETE FROM woml_correction_attempts WHERE run_id = ?1",
    [run_id],
  )?;
  connection.execute(
    "DELETE FROM woml_correction_actions WHERE run_id = ?1",
    [run_id],
  )?;
  connection.execute("DELETE FROM woml_saga_scopes WHERE run_id = ?1", [run_id])?;
  let updated_at = events
    .last()
    .map(|event| event.occurred_at)
    .unwrap_or_else(Utc::now)
    .to_rfc3339();
  for saga in projection.sagas.values() {
    let phase = match saga.phase {
      crate::SagaPhase::Forward => "forward",
      crate::SagaPhase::ForwardSucceeded => "forward_succeeded",
      crate::SagaPhase::Correcting => "correcting",
      crate::SagaPhase::CorrectionCompleted => "correction_completed",
      crate::SagaPhase::ManualRecovery => "manual_recovery",
    };
    let original_outcome = saga.reason.map(|reason| match reason {
      crate::SagaCorrectionReason::Success => "success",
      crate::SagaCorrectionReason::Failure => "failure",
      crate::SagaCorrectionReason::Cancel => "cancel",
    });
    connection.execute(
      "INSERT INTO woml_saga_scopes(
         run_id, saga_id, phase, original_outcome, started_sequence,
         committed_sequence, updated_at, model_digest
       ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
      params![
        run_id,
        saga.saga_id,
        phase,
        original_outcome,
        i64::try_from(saga.started_sequence).map_err(|_| DurableStoreError::Contract(
          "Saga sequence exceeds SQLite integer range.".to_string()
        ))?,
        saga
          .correction_completed_sequence
          .map(i64::try_from)
          .transpose()
          .map_err(|_| {
            DurableStoreError::Contract("Saga sequence exceeds SQLite integer range.".to_string())
          })?,
        updated_at,
        definition_hash,
      ],
    )?;
    for action in saga.actions.values() {
      let status = match action.status {
        crate::CorrectionActionStatus::Registered => "registered",
        crate::CorrectionActionStatus::Started => "started",
        crate::CorrectionActionStatus::RetryScheduled => "retry_scheduled",
        crate::CorrectionActionStatus::Succeeded => "succeeded",
        crate::CorrectionActionStatus::Failed => "failed",
        crate::CorrectionActionStatus::Interrupted => "interrupted",
        crate::CorrectionActionStatus::ManualRecovery => "manual_recovery",
      };
      connection.execute(
        "INSERT INTO woml_correction_actions(
           run_id, saga_id, action_id, action_kind, for_step_execution_id,
           completion_sequence, status, attempts, next_attempt_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
          run_id,
          saga.saga_id,
          action.action_id,
          correction_action_kind_name(action.action_kind),
          action.for_step_id,
          action
            .completion_sequence
            .map(i64::try_from)
            .transpose()
            .map_err(|_| {
              DurableStoreError::Contract(
                "Correction sequence exceeds SQLite integer range.".to_string(),
              )
            })?,
          status,
          i64::from(action.attempts),
          action.retry_at.map(|value| value.to_rfc3339()),
          updated_at,
        ],
      )?;
    }
  }
  for event in events {
    match &event.payload {
      RunEventPayload::CorrectionAttemptStarted(data) => {
        connection.execute(
          "INSERT INTO woml_correction_attempts(
             run_id, saga_id, action_id, attempt, invocation_id, status,
             started_at, finished_at, failure_code
           ) VALUES (?1, ?2, ?3, ?4, ?5, 'started', ?6, NULL, NULL)",
          params![
            run_id,
            data.saga_id,
            data.action_id,
            i64::from(data.attempt),
            correction_invocation_id(run_id, &data.saga_id, &data.action_id, data.attempt),
            event.occurred_at.to_rfc3339(),
          ],
        )?;
      }
      RunEventPayload::CorrectionAttemptSucceeded(data) => {
        connection.execute(
          "UPDATE woml_correction_attempts
           SET status = 'succeeded', finished_at = ?1
           WHERE run_id = ?2 AND saga_id = ?3 AND action_id = ?4 AND attempt = ?5",
          params![
            event.occurred_at.to_rfc3339(),
            run_id,
            data.saga_id,
            data.action_id,
            i64::from(data.attempt)
          ],
        )?;
      }
      RunEventPayload::CorrectionAttemptFailed(data) => {
        connection.execute(
          "UPDATE woml_correction_attempts
           SET status = 'failed', finished_at = ?1, failure_code = ?2
           WHERE run_id = ?3 AND saga_id = ?4 AND action_id = ?5 AND attempt = ?6",
          params![
            event.occurred_at.to_rfc3339(),
            data.code,
            run_id,
            data.saga_id,
            data.action_id,
            i64::from(data.attempt)
          ],
        )?;
      }
      RunEventPayload::CorrectionInterrupted(data) => {
        connection.execute(
          "UPDATE woml_correction_attempts
           SET status = 'interrupted', finished_at = ?1, failure_code = 'WOML_CORRECTION_INTERRUPTED'
           WHERE run_id = ?2 AND saga_id = ?3 AND action_id = ?4 AND attempt = ?5",
          params![event.occurred_at.to_rfc3339(), run_id, data.saga_id, data.action_id, i64::from(data.attempt)],
        )?;
      }
      RunEventPayload::ManualRecoveryRequired(data) => {
        let unresolved: i64 = connection.query_row(
          "SELECT COUNT(*) FROM woml_correction_manual_recovery WHERE status = 'required'",
          [],
          |row| row.get(0),
        )?;
        if unresolved >= i64::try_from(CORRECTION_UNRESOLVED_STORE_LIMIT).unwrap_or(i64::MAX) {
          return Err(DurableStoreError::Contract(
            "WOML_CORRECTION_LIMIT_EXCEEDED: the store reached 10000 unresolved recovery records."
              .to_string(),
          ));
        }
        connection.execute(
          "INSERT INTO woml_correction_manual_recovery(
             run_id, saga_id, action_id, status, reason_code, created_at, resolved_at
           ) VALUES (?1, ?2, ?3, 'required', ?4, ?5, NULL)",
          params![
            run_id,
            data.saga_id,
            data.action_id,
            data.code,
            event.occurred_at.to_rfc3339()
          ],
        )?;
      }
      RunEventPayload::CorrectionRetryScheduled(data) => {
        connection.execute(
          "DELETE FROM woml_correction_manual_recovery
           WHERE run_id = ?1 AND saga_id = ?2 AND action_id = ?3",
          params![run_id, data.saga_id, data.action_id],
        )?;
      }
      _ => {}
    }
  }
  for (saga_id, action_id, reason_code, created_at, resolved_at) in externally_resolved {
    connection.execute(
      "UPDATE woml_correction_manual_recovery
       SET status = 'externally_resolved', reason_code = ?1, created_at = ?2, resolved_at = ?3
       WHERE run_id = ?4 AND saga_id = ?5 AND action_id = ?6",
      params![
        reason_code,
        created_at,
        resolved_at,
        run_id,
        saga_id,
        action_id
      ],
    )?;
  }
  Ok(())
}

fn rebuild_correction_indexes(connection: &Connection) -> Result<(), DurableStoreError> {
  let externally_resolved = {
    let mut statement = connection.prepare(
      "SELECT run_id, saga_id, action_id, reason_code, created_at, resolved_at
       FROM woml_correction_manual_recovery
       WHERE status = 'externally_resolved'",
    )?;
    let rows = statement
      .query_map([], |row| {
        Ok((
          row.get::<_, String>(0)?,
          row.get::<_, String>(1)?,
          row.get::<_, String>(2)?,
          row.get::<_, String>(3)?,
          row.get::<_, String>(4)?,
          row.get::<_, Option<String>>(5)?,
        ))
      })?
      .collect::<Result<Vec<_>, _>>()?;
    rows
  };
  connection.execute("DELETE FROM woml_correction_manual_recovery", [])?;
  connection.execute("DELETE FROM woml_correction_attempts", [])?;
  connection.execute("DELETE FROM woml_correction_actions", [])?;
  connection.execute("DELETE FROM woml_saga_scopes", [])?;
  let mut statement = connection
    .prepare("SELECT run_id, definition_hash FROM woml_runs ORDER BY created_at, run_id")?;
  let runs = statement
    .query_map([], |row| {
      Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?
    .collect::<Result<Vec<_>, _>>()?;
  drop(statement);
  for (run_id, definition_hash) in runs {
    let events = load_events(connection, &run_id)?;
    if events.first().is_some_and(|event| {
      matches!(
        event.event_schema_version,
        crate::RUN_EVENT_SCHEMA_VERSION_V18
          | crate::RUN_EVENT_SCHEMA_VERSION_V19
          | crate::RUN_EVENT_SCHEMA_VERSION_V20
      )
    }) {
      let projection = fold_events(&events)?;
      write_correction_indexes_from_projection(
        connection,
        &run_id,
        &definition_hash,
        &events,
        &projection,
      )?;
    }
  }
  for (run_id, saga_id, action_id, reason_code, created_at, resolved_at) in externally_resolved {
    connection.execute(
      "UPDATE woml_correction_manual_recovery
       SET status = 'externally_resolved', reason_code = ?1, created_at = ?2, resolved_at = ?3
       WHERE run_id = ?4 AND saga_id = ?5 AND action_id = ?6",
      params![
        reason_code,
        created_at,
        resolved_at,
        run_id,
        saga_id,
        action_id
      ],
    )?;
  }
  Ok(())
}

fn projection_state_for_events(
  cache: &RefCell<HashMap<String, ProjectionFoldState>>,
  run_id: &str,
  events: &[RunEvent],
) -> Result<ProjectionFoldState, DurableStoreError> {
  let cached = cache.borrow_mut().remove(run_id);
  if let Some(cached) = cached {
    let sequence_matches = cached.projection().last_sequence == events.len() as u64;
    let event_matches =
      cached.last_event_id() == events.last().map(|event| event.event_id.as_str());
    if sequence_matches && event_matches {
      return Ok(cached);
    }
  }
  Ok(ProjectionFoldState::from_events(events)?)
}

fn cache_projection_state(
  cache: &RefCell<HashMap<String, ProjectionFoldState>>,
  run_id: &str,
  state: ProjectionFoldState,
) {
  let mut cache = cache.borrow_mut();
  if matches!(
    state.projection().status,
    RunStatus::Succeeded | RunStatus::Failed | RunStatus::Cancelled
  ) {
    cache.remove(run_id);
    return;
  }
  if !cache.contains_key(run_id) && cache.len() >= MAX_ACTIVE_PROJECTION_CACHE_ENTRIES {
    if let Some(evicted) = cache.keys().next().cloned() {
      cache.remove(&evicted);
    }
  }
  cache.insert(run_id.to_string(), state);
}

#[allow(clippy::too_many_arguments)]
fn append_to_history_scoped_incremental(
  transaction: &Transaction<'_>,
  events: &mut Vec<RunEvent>,
  state: &mut ProjectionFoldState,
  run_id: &str,
  event_id: String,
  occurred_at: DateTime<Utc>,
  event_schema_version: u32,
  iteration: Option<crate::event::ForEachIterationScope>,
  payload: RunEventPayload,
  update_read_models: bool,
) -> Result<RunEvent, DurableStoreError> {
  let payloads = if matches!(
    event_schema_version,
    crate::RUN_EVENT_SCHEMA_VERSION_V10
      | crate::RUN_EVENT_SCHEMA_VERSION_V11
      | crate::RUN_EVENT_SCHEMA_VERSION_V12
      | crate::RUN_EVENT_SCHEMA_VERSION_V13
      | crate::RUN_EVENT_SCHEMA_VERSION_V14
      | crate::RUN_EVENT_SCHEMA_VERSION_V15
      | crate::RUN_EVENT_SCHEMA_VERSION_V16
      | crate::RUN_EVENT_SCHEMA_VERSION_V17
      | crate::RUN_EVENT_SCHEMA_VERSION_V18
      | crate::RUN_EVENT_SCHEMA_VERSION_V19
      | crate::RUN_EVENT_SCHEMA_VERSION_V20
  ) && matches!(
    payload,
    RunEventPayload::RunSucceeded(_) | RunEventPayload::RunFailed(_)
  ) {
    expand_model_v11_payload(&definition_for_run(transaction, run_id)?, run_id, payload)?
  } else {
    vec![payload]
  };
  let mut first = None;
  for (index, payload) in payloads.into_iter().enumerate() {
    let event = append_single_to_history_incremental(
      transaction,
      events,
      state,
      run_id,
      if index == 0 {
        event_id.clone()
      } else {
        generated_event_id()
      },
      occurred_at,
      event_schema_version,
      iteration.clone(),
      payload,
      update_read_models,
    )?;
    first.get_or_insert(event);
  }
  Ok(first.unwrap())
}

#[allow(clippy::too_many_arguments)]
fn append_single_to_history_incremental(
  transaction: &Transaction<'_>,
  events: &mut Vec<RunEvent>,
  state: &mut ProjectionFoldState,
  run_id: &str,
  event_id: String,
  occurred_at: DateTime<Utc>,
  event_schema_version: u32,
  iteration: Option<crate::event::ForEachIterationScope>,
  payload: RunEventPayload,
  update_read_models: bool,
) -> Result<RunEvent, DurableStoreError> {
  let sequence = events.len() as u64 + 1;
  let event = RunEvent {
    event_schema_version,
    event_id,
    run_id: run_id.to_string(),
    sequence,
    occurred_at,
    iteration,
    payload,
  };
  state.apply(&event)?;
  let event_json = serde_json::to_string(&event)?;
  let stored_sequence = i64::try_from(sequence).map_err(|_| {
    DurableStoreError::Contract("event sequence exceeds SQLite integer range".to_string())
  })?;
  transaction.execute(
    "INSERT INTO woml_run_events(
       run_id, sequence, event_id, event_schema_version, event_json
     ) VALUES (?1, ?2, ?3, ?4, ?5)",
    params![
      run_id,
      stored_sequence,
      event.event_id,
      i64::from(event.event_schema_version),
      event_json,
    ],
  )?;
  events.push(event.clone());
  if update_read_models {
    write_run_summary_from_projection(
      transaction,
      events.first().unwrap(),
      event.occurred_at,
      state.projection(),
    )?;
    if let RunEventPayload::RunAdmitted(admission) = &events.first().unwrap().payload {
      write_runtime_policy_indexes_from_projection(
        transaction,
        run_id,
        admission,
        state.projection(),
      )?;
    }
  }
  Ok(event)
}

fn append_to_history(
  transaction: &Transaction<'_>,
  events: &mut Vec<RunEvent>,
  run_id: &str,
  event_id: String,
  occurred_at: DateTime<Utc>,
  event_schema_version: u32,
  payload: RunEventPayload,
) -> Result<RunEvent, DurableStoreError> {
  append_to_history_scoped(
    transaction,
    events,
    run_id,
    event_id,
    occurred_at,
    event_schema_version,
    None,
    payload,
  )
}

fn resolve_lock_execution<'a>(
  workflow: &'a CompiledWorkflowDefinition,
  projection: &RunProjection,
  execution_identity: &str,
) -> Option<(&'a crate::CompiledLock, Option<ForEachIterationScope>)> {
  workflow
    .graph
    .locks
    .as_deref()
    .unwrap_or_default()
    .iter()
    .find_map(|lock| {
      if lock.acquire_node_id == execution_identity {
        return Some((lock, None));
      }
      workflow
        .graph
        .for_each
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter(|loop_| {
          loop_
            .body
            .nodes
            .iter()
            .any(|node| node.id == lock.acquire_node_id)
        })
        .find_map(|loop_| {
          let prefix = format!("{}:", loop_.for_each_id);
          let suffix = format!(":{}", lock.acquire_node_id);
          let index = execution_identity
            .strip_prefix(&prefix)?
            .strip_suffix(&suffix)?
            .parse::<u32>()
            .ok()?;
          projection
            .for_each
            .get(&loop_.for_each_id)
            .and_then(|state| state.iterations.get(&index))
            .filter(|status| matches!(status, ForEachIterationStatus::Started))?;
          Some((
            lock,
            Some(ForEachIterationScope {
              for_each_id: loop_.for_each_id.clone(),
              index,
            }),
          ))
        })
    })
}

const PROGRAMMATIC_LOCK_IDENTITY_PREFIX: &str = "capability:";

fn valid_lock_key(key: &str) -> bool {
  !key.is_empty()
    && key.len() <= 256
    && key.trim() == key
    && !key.chars().any(char::is_control)
    && !key.split('/').any(str::is_empty)
}

fn ensure_lock_wait_capacity(
  transaction: &Transaction<'_>,
  key: &str,
) -> Result<(), DurableStoreError> {
  let occupied: bool = transaction.query_row(
    "SELECT owner_lease_id IS NOT NULL OR EXISTS(
       SELECT 1 FROM woml_lock_waiters WHERE key = ?1 AND status = 'waiting'
     ) FROM woml_lock_resources WHERE key = ?1",
    [key],
    |row| row.get(0),
  )?;
  if !occupied {
    return Ok(());
  }
  let key_waiters: i64 = transaction.query_row(
    "SELECT COUNT(*) FROM woml_lock_waiters WHERE key = ?1 AND status = 'waiting'",
    [key],
    |row| row.get(0),
  )?;
  let store_waiters: i64 = transaction.query_row(
    "SELECT COUNT(*) FROM woml_lock_waiters WHERE status = 'waiting'",
    [],
    |row| row.get(0),
  )?;
  if key_waiters >= LOCK_WAITERS_PER_KEY_LIMIT as i64
    || store_waiters >= LOCK_WAITERS_PER_STORE_LIMIT as i64
  {
    return Err(DurableStoreError::LockWaitLimit);
  }
  Ok(())
}

fn validate_programmatic_lock_input(
  node_id: &str,
  invocation_id: &str,
  session_id: &str,
  key: &str,
  acquire_timeout_ms: u64,
  lease_ms: u64,
) -> Result<(), DurableStoreError> {
  if node_id.is_empty()
    || invocation_id.is_empty()
    || session_id.is_empty()
    || session_id.len() > 128
    || !session_id
      .bytes()
      .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    || !valid_lock_key(key)
    || !(1_000..=604_800_000).contains(&acquire_timeout_ms)
    || !(5_000..=600_000).contains(&lease_ms)
  {
    return Err(DurableStoreError::Contract(
      "Programmatic lock acquisition input is invalid.".to_string(),
    ));
  }
  Ok(())
}

fn programmatic_lock_execution_identity(session_id: &str, lease_ms: u64) -> String {
  format!("{PROGRAMMATIC_LOCK_IDENTITY_PREFIX}{lease_ms}:{session_id}")
}

fn programmatic_lock_lease_ms(execution_identity: &str) -> Option<u64> {
  let remainder = execution_identity.strip_prefix(PROGRAMMATIC_LOCK_IDENTITY_PREFIX)?;
  let (lease_ms, session_id) = remainder.split_once(':')?;
  (!session_id.is_empty())
    .then(|| lease_ms.parse::<u64>().ok())
    .flatten()
    .filter(|lease_ms| (5_000..=600_000).contains(lease_ms))
}

fn programmatic_lock_session_ref(execution_identity: &str) -> Option<String> {
  let remainder = execution_identity.strip_prefix(PROGRAMMATIC_LOCK_IDENTITY_PREFIX)?;
  let (_, session_id) = remainder.split_once(':')?;
  (!session_id.is_empty()).then(|| {
    format!(
      "sha256:{}",
      hex::encode(Sha256::digest(session_id.as_bytes()))
    )
  })
}

fn finite_duration_ms(duration_ms: f64) -> Option<u64> {
  (duration_ms.is_finite() && duration_ms >= 0.0 && duration_ms <= u64::MAX as f64)
    .then(|| duration_ms.round() as u64)
}

fn programmatic_lock_state_in_transaction(
  transaction: &Transaction<'_>,
  run_id: &str,
  session_id: &str,
  lease_ms: u64,
  now: DateTime<Utc>,
) -> Result<ProgrammaticLockAcquire, DurableStoreError> {
  let identity = programmatic_lock_execution_identity(session_id, lease_ms);
  let waiter: Option<(String, String)> = transaction
    .query_row(
      "SELECT key, status FROM woml_lock_waiters WHERE run_id = ?1 AND node_execution_id = ?2",
      params![run_id, identity],
      |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()?;
  let Some((key, status)) = waiter else {
    return Err(DurableStoreError::LockHistoryInvalid(
      "programmatic lease session does not exist".to_string(),
    ));
  };
  let state = match status.as_str() {
    "waiting" => ProgrammaticLockAcquireState::Waiting,
    "timed_out" => ProgrammaticLockAcquireState::TimedOut,
    "cancelled" => ProgrammaticLockAcquireState::Cancelled,
    "lost" => ProgrammaticLockAcquireState::Lost,
    "released" => ProgrammaticLockAcquireState::Lost,
    "held" => {
      let owner: Option<(String, String, String, String)> = transaction
        .query_row(
          "SELECT owner_lease_id, CAST(owner_fencing_token AS TEXT), acquired_at, expires_at
           FROM woml_lock_resources
           WHERE key = ?1 AND owner_run_id = ?2 AND owner_node_execution_id = ?3",
          params![key, run_id, identity],
          |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
      let Some((_lease_id, fencing_token, acquired_at, expires_at)) = owner else {
        return Ok(ProgrammaticLockAcquire {
          state: ProgrammaticLockAcquireState::Lost,
          lease: None,
        });
      };
      let expires_at = parse_stored_timestamp(&expires_at)?;
      if expires_at <= now {
        ProgrammaticLockAcquireState::Lost
      } else {
        return Ok(ProgrammaticLockAcquire {
          state: ProgrammaticLockAcquireState::Held,
          lease: Some(ProgrammaticLockLease {
            session_id: session_id.to_string(),
            key,
            fencing_token,
            acquired_at: parse_stored_timestamp(&acquired_at)?,
            expires_at,
          }),
        });
      }
    }
    _ => {
      return Err(DurableStoreError::LockHistoryInvalid(
        "programmatic lease has an unsupported stored state".to_string(),
      ))
    }
  };
  Ok(ProgrammaticLockAcquire { state, lease: None })
}

fn grant_oldest_lock_waiter(
  transaction: &Transaction<'_>,
  key: &str,
  now: DateTime<Utc>,
) -> Result<Option<String>, DurableStoreError> {
  expire_lock_owner(transaction, key, now)?;
  let owner: Option<String> = transaction
    .query_row(
      "SELECT owner_lease_id FROM woml_lock_resources WHERE key = ?1",
      [key],
      |row| row.get(0),
    )
    .optional()?
    .flatten();
  if owner.is_some() {
    return Ok(None);
  }
  let selected: Option<(String, String, String)> = transaction
    .query_row(
      "SELECT waiter_id, run_id, node_execution_id FROM woml_lock_waiters
     WHERE key = ?1 AND status = 'waiting' AND deadline >= ?2
     ORDER BY sequence, waiter_id LIMIT 1",
      params![key, now.to_rfc3339()],
      |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .optional()?;
  let Some((waiter_id, run_id, execution_identity)) = selected else {
    return Ok(None);
  };
  let programmatic_lease_ms = programmatic_lock_lease_ms(&execution_identity);
  let (descriptor, iteration, lease_ms) = if let Some(lease_ms) = programmatic_lease_ms {
    (None, None, lease_ms)
  } else {
    let workflow = definition_for_run(transaction, &run_id)?;
    let projection = fold_events(&load_events(transaction, &run_id)?)?;
    let (descriptor, iteration) =
      resolve_lock_execution(&workflow, &projection, &execution_identity).ok_or_else(|| {
        DurableStoreError::LockHistoryInvalid(
          "waiting acquisition has no exact active Model v18 execution identity".to_string(),
        )
      })?;
    (
      Some(descriptor.lock_id.clone()),
      iteration,
      descriptor.lease_ms,
    )
  };
  let current_high_water: i64 = transaction.query_row(
    "SELECT fencing_high_water FROM woml_lock_resources WHERE key = ?1",
    [key],
    |row| row.get(0),
  )?;
  let next_token = current_high_water
    .checked_add(1)
    .ok_or(DurableStoreError::LockFencingExhausted)?;
  let lease_id = format!("lease_{}", Uuid::new_v4().simple());
  let expires_at = now
    + chrono::Duration::milliseconds(i64::try_from(lease_ms).map_err(|_| {
      DurableStoreError::Contract("Lock lease exceeds the runtime clock.".to_string())
    })?);
  let changed = transaction.execute(
    "UPDATE woml_lock_resources SET fencing_high_water = ?1, owner_lease_id = ?2,
     owner_run_id = ?3, owner_node_execution_id = ?4, owner_fencing_token = ?1,
     acquired_at = ?5, expires_at = ?6, updated_at = ?5
     WHERE key = ?7 AND owner_lease_id IS NULL AND fencing_high_water = ?8",
    params![
      next_token,
      lease_id,
      run_id,
      execution_identity,
      now.to_rfc3339(),
      expires_at.to_rfc3339(),
      key,
      current_high_water
    ],
  )?;
  if changed != 1 {
    return Err(DurableStoreError::LockHistoryInvalid(
      "atomic lock grant lost ownership race".to_string(),
    ));
  }
  if transaction.execute(
    "UPDATE woml_lock_waiters SET status = 'held' WHERE waiter_id = ?1 AND status = 'waiting'",
    [&waiter_id],
  )? != 1
  {
    return Err(DurableStoreError::LockHistoryInvalid(
      "selected waiter changed before grant".to_string(),
    ));
  }
  if let Some(lock_id) = descriptor {
    let mut events = load_events(transaction, &run_id)?;
    let schema = events
      .first()
      .map(|event| event.event_schema_version)
      .ok_or_else(|| {
        DurableStoreError::LockHistoryInvalid(
          "selected waiter run has no event history".to_string(),
        )
      })?;
    append_to_history_scoped(
      transaction,
      &mut events,
      &run_id,
      generated_event_id(),
      now,
      schema,
      iteration,
      RunEventPayload::LockAcquired(crate::LockAcquiredData {
        lock_id,
        key: key.to_string(),
        lease_id,
        fencing_token: next_token.to_string(),
        acquired_at: now,
        expires_at,
        execution_identity,
      }),
    )?;
  }
  Ok(Some(run_id))
}

fn expire_lock_owner(
  transaction: &Transaction<'_>,
  key: &str,
  now: DateTime<Utc>,
) -> Result<bool, DurableStoreError> {
  let owner: Option<(String, String, String, String, String)> = transaction
    .query_row(
      "SELECT owner_lease_id, owner_run_id, owner_node_execution_id,
              CAST(owner_fencing_token AS TEXT), expires_at
       FROM woml_lock_resources WHERE key = ?1 AND owner_lease_id IS NOT NULL",
      [key],
      |row| {
        Ok((
          row.get(0)?,
          row.get(1)?,
          row.get(2)?,
          row.get(3)?,
          row.get(4)?,
        ))
      },
    )
    .optional()?;
  let Some((lease_id, run_id, execution_identity, fencing_token, expires_at)) = owner else {
    return Ok(false);
  };
  let expired_at = parse_stored_timestamp(&expires_at)?;
  if expired_at > now {
    return Ok(false);
  }
  if programmatic_lock_lease_ms(&execution_identity).is_some() {
    if transaction.execute(
      "UPDATE woml_lock_resources SET owner_lease_id = NULL, owner_run_id = NULL,
         owner_node_execution_id = NULL, owner_fencing_token = NULL, acquired_at = NULL,
         expires_at = NULL, updated_at = ?1
       WHERE key = ?2 AND owner_run_id = ?3 AND owner_lease_id = ?4",
      params![now.to_rfc3339(), key, run_id, lease_id],
    )? != 1
    {
      return Err(DurableStoreError::LockHistoryInvalid(
        "expired programmatic owner changed before lease-loss settlement".to_string(),
      ));
    }
    transaction.execute(
      "UPDATE woml_lock_waiters SET status = 'lost'
       WHERE run_id = ?1 AND node_execution_id = ?2 AND status = 'held'",
      params![run_id, execution_identity],
    )?;
    return Ok(true);
  }
  let workflow = definition_for_run(transaction, &run_id)?;
  let mut events = load_events(transaction, &run_id)?;
  let projection = fold_events(&events)?;
  let (descriptor, iteration) = resolve_lock_execution(&workflow, &projection, &execution_identity)
    .ok_or_else(|| {
      DurableStoreError::LockHistoryInvalid(
        "expired owner has no exact active Model v18 execution identity".to_string(),
      )
    })?;
  let held = iteration
    .as_ref()
    .and_then(|scope| projection.for_each.get(&scope.for_each_id))
    .and_then(|loop_| loop_.executions.get(&iteration.as_ref()?.index))
    .and_then(|execution| execution.locks.get(&descriptor.lock_id))
    .or_else(|| {
      iteration
        .is_none()
        .then(|| projection.locks.get(&descriptor.lock_id))
        .flatten()
    })
    .ok_or_else(|| {
      DurableStoreError::LockHistoryInvalid("expired owner has no projected lock".to_string())
    })?;
  if held.status != crate::LockStatus::Held
    || held.key != key
    || held.lease_id.as_deref() != Some(lease_id.as_str())
    || held.fencing_token.as_deref() != Some(fencing_token.as_str())
  {
    return Err(DurableStoreError::LockHistoryInvalid(
      "expired durable owner differs from projected held lease".to_string(),
    ));
  }
  let active_effect = iteration.as_ref().map_or_else(
    || {
      projection
        .operations
        .values()
        .any(|operation| operation.status == crate::projection::OperationStatus::Started)
    },
    |scope| {
      projection
        .for_each
        .get(&scope.for_each_id)
        .and_then(|loop_| loop_.executions.get(&scope.index))
        .is_some_and(|execution| {
          execution
            .operations
            .values()
            .any(|operation| operation.status == crate::projection::OperationStatus::Started)
        })
    },
  );
  let schema = events
    .first()
    .map(|event| event.event_schema_version)
    .ok_or_else(|| DurableStoreError::LockHistoryInvalid("owner history is empty".to_string()))?;
  append_to_history_scoped(
    transaction,
    &mut events,
    &run_id,
    generated_event_id(),
    now,
    schema,
    iteration,
    RunEventPayload::LockLeaseLost(crate::LockLeaseLostData {
      lock_id: descriptor.lock_id.clone(),
      key: key.to_string(),
      lease_id: lease_id.clone(),
      fencing_token,
      expired_at,
      detected_at: now,
      ambiguous: active_effect,
    }),
  )?;
  if transaction.execute(
    "UPDATE woml_lock_resources SET owner_lease_id = NULL, owner_run_id = NULL,
       owner_node_execution_id = NULL, owner_fencing_token = NULL, acquired_at = NULL,
       expires_at = NULL, updated_at = ?1
     WHERE key = ?2 AND owner_run_id = ?3 AND owner_lease_id = ?4",
    params![now.to_rfc3339(), key, run_id, lease_id],
  )? != 1
  {
    return Err(DurableStoreError::LockHistoryInvalid(
      "expired owner changed before lease-loss settlement".to_string(),
    ));
  }
  transaction.execute(
    "UPDATE woml_lock_waiters SET status = 'lost'
     WHERE run_id = ?1 AND node_execution_id = ?2 AND status = 'held'",
    params![run_id, execution_identity],
  )?;
  Ok(true)
}

fn lock_descriptor_for_protected_node<'a>(
  workflow: &'a CompiledWorkflowDefinition,
  node_id: &str,
) -> Option<&'a crate::CompiledLock> {
  fn contains_node(
    lock: &crate::CompiledLock,
    node_id: &str,
    edges: &[crate::model::CompiledWorkflowEdge],
  ) -> bool {
    let mut pending = vec![lock.body_entry_node_id.as_str()];
    let mut visited = HashSet::new();
    while let Some(candidate) = pending.pop() {
      if !visited.insert(candidate) {
        continue;
      }
      if candidate == node_id {
        return true;
      }
      if candidate == lock.body_terminal_node_id {
        continue;
      }
      pending.extend(
        edges
          .iter()
          .filter(|edge| edge.from == candidate)
          .map(|edge| edge.to.as_str()),
      );
    }
    false
  }
  workflow
    .graph
    .locks
    .as_deref()
    .unwrap_or_default()
    .iter()
    .find(|lock| {
      contains_node(lock, node_id, &workflow.graph.edges)
        || workflow
          .graph
          .for_each
          .as_deref()
          .unwrap_or_default()
          .iter()
          .any(|loop_| contains_node(lock, node_id, &loop_.body.edges))
    })
}

#[allow(clippy::too_many_arguments)]
fn append_to_history_scoped(
  transaction: &Transaction<'_>,
  events: &mut Vec<RunEvent>,
  run_id: &str,
  event_id: String,
  occurred_at: DateTime<Utc>,
  event_schema_version: u32,
  iteration: Option<crate::event::ForEachIterationScope>,
  payload: RunEventPayload,
) -> Result<RunEvent, DurableStoreError> {
  let payloads = if matches!(
    event_schema_version,
    crate::RUN_EVENT_SCHEMA_VERSION_V10
      | crate::RUN_EVENT_SCHEMA_VERSION_V11
      | crate::RUN_EVENT_SCHEMA_VERSION_V12
      | crate::RUN_EVENT_SCHEMA_VERSION_V13
      | crate::RUN_EVENT_SCHEMA_VERSION_V14
      | crate::RUN_EVENT_SCHEMA_VERSION_V15
      | crate::RUN_EVENT_SCHEMA_VERSION_V16
      | crate::RUN_EVENT_SCHEMA_VERSION_V17
      | crate::RUN_EVENT_SCHEMA_VERSION_V18
      | crate::RUN_EVENT_SCHEMA_VERSION_V19
      | crate::RUN_EVENT_SCHEMA_VERSION_V20
  ) && matches!(
    payload,
    RunEventPayload::RunSucceeded(_) | RunEventPayload::RunFailed(_)
  ) {
    expand_model_v11_payload(&definition_for_run(transaction, run_id)?, run_id, payload)?
  } else {
    vec![payload]
  };
  let mut first = None;
  for (index, payload) in payloads.into_iter().enumerate() {
    let event = append_single_to_history(
      transaction,
      events,
      run_id,
      if index == 0 {
        event_id.clone()
      } else {
        generated_event_id()
      },
      occurred_at,
      event_schema_version,
      iteration.clone(),
      payload,
    )?;
    first.get_or_insert(event);
  }
  Ok(first.unwrap())
}

fn append_single_to_history(
  transaction: &Transaction<'_>,
  events: &mut Vec<RunEvent>,
  run_id: &str,
  event_id: String,
  occurred_at: DateTime<Utc>,
  event_schema_version: u32,
  iteration: Option<crate::event::ForEachIterationScope>,
  payload: RunEventPayload,
) -> Result<RunEvent, DurableStoreError> {
  let sequence = events.len() as u64 + 1;
  let event = RunEvent {
    event_schema_version,
    event_id,
    run_id: run_id.to_string(),
    sequence,
    occurred_at,
    iteration,
    payload,
  };
  let mut candidate = events.clone();
  candidate.push(event.clone());
  fold_events(&candidate)?;
  let event_json = serde_json::to_string(&event)?;
  let stored_sequence = i64::try_from(sequence).map_err(|_| {
    DurableStoreError::Contract("event sequence exceeds SQLite integer range".to_string())
  })?;
  transaction.execute(
    "INSERT INTO woml_run_events(
       run_id, sequence, event_id, event_schema_version, event_json
     ) VALUES (?1, ?2, ?3, ?4, ?5)",
    params![
      run_id,
      stored_sequence,
      event.event_id,
      i64::from(event.event_schema_version),
      event_json,
    ],
  )?;
  events.push(event.clone());
  write_run_summary(transaction, events)?;
  write_runtime_policy_indexes_for_run(transaction, events)?;
  Ok(event)
}

fn interrupted_failure() -> AttemptFailure {
  AttemptFailure {
    kind: AttemptFailureKind::Interrupted,
    code: AttemptFailureKind::Interrupted.code().to_string(),
    message: "Recovery found a started attempt without a terminal event.".to_string(),
    details: None,
    ..AttemptFailure::legacy_defaults()
  }
}

fn interrupted_operation_failure(
  operation: &crate::projection::OperationProjection,
  observed: bool,
  loop_owned: bool,
) -> crate::CapabilityFailure {
  let process = operation.capability == "process" && operation.operation == "run";
  let (code, message) = if process {
    (
      "WOML_PROCESS_INTERRUPTED",
      "Recovery found an approved process start without a durable terminal outcome; the process result is ambiguous and WOML will not replay it.",
    )
  } else if observed {
    (
      "WOML_NATIVE_FETCH_INTERRUPTED",
      "Recovery found native Fetch without a durable terminal observation; its outcome is ambiguous and it will not be replayed.",
    )
  } else if loop_owned {
    (
      "WOML_CAPABILITY_INTERRUPTED",
      "Recovery found a loop-owned operation without a durable terminal event; its outcome is ambiguous and it will not be replayed.",
    )
  } else {
    (
      "WOML_CAPABILITY_INTERRUPTED",
      "Recovery found a managed operation without a durable terminal event; it will not be replayed.",
    )
  };
  crate::CapabilityFailure {
    kind: crate::CapabilityFailureKind::Interrupted,
    code: code.to_string(),
    message: message.to_string(),
    retryable: false,
    ambiguous: true,
    details: None,
  }
}

fn interrupted_operation_metadata(
  operation: &crate::projection::OperationProjection,
) -> serde_json::Map<String, serde_json::Value> {
  let mut metadata = operation.metadata.clone();
  if operation.capability == "process" && operation.operation == "run" {
    metadata.insert(
      "outcome".to_string(),
      serde_json::Value::String("interrupted".to_string()),
    );
  }
  metadata
}

fn for_each_recovery_failure_kind(kind: AttemptFailureKind) -> ForEachIterationFailureKind {
  match kind {
    AttemptFailureKind::ScriptThrew => ForEachIterationFailureKind::ScriptThrew,
    AttemptFailureKind::ScriptTimedOut => ForEachIterationFailureKind::TimedOut,
    AttemptFailureKind::InvalidScriptResult => ForEachIterationFailureKind::NonJson,
    AttemptFailureKind::WorkerCrashed => ForEachIterationFailureKind::WorkerCrashed,
    AttemptFailureKind::HostCrashed => ForEachIterationFailureKind::HostCrashed,
    AttemptFailureKind::ResultTooLarge => ForEachIterationFailureKind::ResultTooLarge,
    AttemptFailureKind::ContextTooLarge => ForEachIterationFailureKind::ContextTooLarge,
    AttemptFailureKind::Interrupted => ForEachIterationFailureKind::Interrupted,
    AttemptFailureKind::ServiceFailed => ForEachIterationFailureKind::CapabilityFailed,
    AttemptFailureKind::InvocationCancelled => ForEachIterationFailureKind::Cancelled,
  }
}

fn recover_open_for_each_failure(
  transaction: &Transaction<'_>,
  workflow: &CompiledWorkflowDefinition,
  events: &mut Vec<RunEvent>,
  run_id: &str,
  event_schema_version: u32,
  now: DateTime<Utc>,
  forced_failure: Option<AttemptFailure>,
) -> Result<Option<(usize, AttemptFailure)>, DurableStoreError> {
  let projection = fold_events(events)?;
  if !matches!(
    event_schema_version,
    crate::RUN_EVENT_SCHEMA_VERSION_V15 | crate::RUN_EVENT_SCHEMA_VERSION_V16
  ) || projection.status == RunStatus::Cancelling
  {
    return Ok(None);
  }
  let force_settlement = forced_failure.is_some();
  let Some(descriptor) = workflow
    .graph
    .for_each
    .as_deref()
    .unwrap_or_default()
    .iter()
    .find(|descriptor| {
      projection
        .for_each
        .get(&descriptor.for_each_id)
        .is_some_and(|loop_state| {
          loop_state.status == ForEachStatus::Open
            && (force_settlement
              || loop_state
                .iterations
                .values()
                .any(|status| matches!(status, ForEachIterationStatus::Failed { .. }))
              || loop_state.executions.values().any(|execution| {
                execution
                  .attempts
                  .iter()
                  .any(|attempt| attempt.status == AttemptStatus::Started)
                  || execution.operations.values().any(|operation| {
                    operation.status == crate::projection::OperationStatus::Started
                  })
                  || execution.lifecycle_hooks.values().any(|hook| {
                    matches!(
                      hook.status,
                      crate::projection::LifecycleHookStatus::Requested
                        | crate::projection::LifecycleHookStatus::Running
                    )
                  })
                  || execution.reusable_lifecycle_hooks.values().any(|hook| {
                    matches!(
                      hook.status,
                      crate::projection::ReusableLifecycleStatus::Requested
                        | crate::projection::ReusableLifecycleStatus::Running
                    )
                  })
              }))
        })
    })
  else {
    return Ok(None);
  };

  let loop_state = projection
    .for_each
    .get(&descriptor.for_each_id)
    .expect("the selected loop exists");
  let mut interrupted_attempts = 0_usize;
  let mut primary_failure = forced_failure.clone().or_else(|| {
    loop_state.executions.values().find_map(|execution| {
      execution.attempts.iter().rev().find_map(|attempt| {
        if let AttemptStatus::Failed { failure } = &attempt.status {
          Some(failure.clone())
        } else {
          None
        }
      })
    })
  });

  for (index, execution) in &loop_state.executions {
    let scope = ForEachIterationScope {
      for_each_id: descriptor.for_each_id.clone(),
      index: *index,
    };
    for hook in execution.reusable_lifecycle_hooks.values().filter(|hook| {
      matches!(
        hook.status,
        crate::projection::ReusableLifecycleStatus::Requested
          | crate::projection::ReusableLifecycleStatus::Running
      )
    }) {
      if let Some(action_id) = &hook.active_action_id {
        append_to_history_scoped(
          transaction,
          events,
          run_id,
          generated_event_id(),
          now,
          event_schema_version,
          Some(scope.clone()),
          RunEventPayload::ReusableLifecycleActionFailed(
            crate::event::ReusableLifecycleActionFailedData {
              invocation_id: hook.invocation_id.clone(),
              definition_digest: hook.definition_digest.clone(),
              hook: hook.hook,
              action_id: action_id.clone(),
              outcome: crate::event::ReusableLifecycleOutcome::Failed,
              warning_code: "WOML_REUSABLE_LIFECYCLE_INTERRUPTED".to_string(),
            },
          ),
        )?;
      }
      append_to_history_scoped(
        transaction,
        events,
        run_id,
        generated_event_id(),
        now,
        event_schema_version,
        Some(scope.clone()),
        RunEventPayload::ReusableLifecycleCompleted(crate::event::ReusableLifecycleCompletedData {
          invocation_id: hook.invocation_id.clone(),
          definition_digest: hook.definition_digest.clone(),
          hook: hook.hook,
          outcome: crate::event::ReusableLifecycleOutcome::CompletedWithWarnings,
        }),
      )?;
      primary_failure.get_or_insert_with(interrupted_failure);
    }
    for hook in execution.lifecycle_hooks.values().filter(|hook| {
      matches!(
        hook.status,
        crate::projection::LifecycleHookStatus::Requested
          | crate::projection::LifecycleHookStatus::Running
      )
    }) {
      let compiled = workflow.lifecycle_hook(&hook.hook_id).ok_or_else(|| {
        DurableStoreError::Contract(format!(
          "Recovered iteration hook {:?} is absent from Model v16.",
          hook.hook_id
        ))
      })?;
      let failed_actions = compiled
        .actions
        .iter()
        .filter(|action| {
          !matches!(
            hook
              .actions
              .get(&action.action_id)
              .map(|action| action.status),
            Some(crate::projection::LifecycleActionStatus::Succeeded)
          )
        })
        .count() as u32;
      for action in &compiled.actions {
        match hook
          .actions
          .get(&action.action_id)
          .map(|action| action.status)
        {
          Some(crate::projection::LifecycleActionStatus::Succeeded)
          | Some(crate::projection::LifecycleActionStatus::Failed) => continue,
          None => {
            append_to_history_scoped(
              transaction,
              events,
              run_id,
              generated_event_id(),
              now,
              event_schema_version,
              Some(scope.clone()),
              RunEventPayload::LifecycleActionAttemptStarted(
                crate::event::LifecycleActionIdentityData {
                  hook_invocation_id: hook.hook_invocation_id.clone(),
                  action_id: action.action_id.clone(),
                  attempt: 1,
                },
              ),
            )?;
          }
          Some(crate::projection::LifecycleActionStatus::Started) => {}
        }
        append_to_history_scoped(
          transaction,
          events,
          run_id,
          generated_event_id(),
          now,
          event_schema_version,
          Some(scope.clone()),
          RunEventPayload::LifecycleActionFailed(crate::event::LifecycleActionFailedData {
            hook_invocation_id: hook.hook_invocation_id.clone(),
            action_id: action.action_id.clone(),
            attempt: 1,
            failure: crate::event::LifecycleFailure {
              kind: crate::event::LifecycleFailureKind::Interrupted,
              code: "WOML_LIFECYCLE_ACTION_INTERRUPTED".to_string(),
              message: "Recovery failed closed for an unfinished loop-owned lifecycle action."
                .to_string(),
            },
          }),
        )?;
      }
      append_to_history_scoped(
        transaction,
        events,
        run_id,
        generated_event_id(),
        now,
        event_schema_version,
        Some(scope.clone()),
        RunEventPayload::LifecycleHookCompleted(crate::event::LifecycleHookCompletedData {
          hook_invocation_id: hook.hook_invocation_id.clone(),
          status: crate::event::LifecycleHookCompletionStatus::CompletedWithWarnings,
          failed_actions,
        }),
      )?;
      primary_failure.get_or_insert_with(interrupted_failure);
    }
    for operation in execution
      .operations
      .values()
      .filter(|operation| operation.status == crate::projection::OperationStatus::Started)
    {
      append_to_history_scoped(
        transaction,
        events,
        run_id,
        generated_event_id(),
        now,
        event_schema_version,
        Some(scope.clone()),
        RunEventPayload::OperationFailed(OperationFailedData {
          node_id: operation.node_id.clone(),
          attempt_number: operation.attempt_number,
          invocation_id: operation.identity.invocation_id.clone(),
          call_id: operation.identity.call_id.clone(),
          operation_key: operation.operation_key.clone(),
          capability: operation.capability.clone(),
          operation: operation.operation.clone(),
          execution_mode: operation.execution_mode,
          metadata: interrupted_operation_metadata(operation),
          duration_ms: 0.0,
          failure: interrupted_operation_failure(operation, false, true),
        }),
      )?;
    }
    for attempt in execution
      .attempts
      .iter()
      .filter(|attempt| attempt.status == AttemptStatus::Started)
    {
      let failure = forced_failure.clone().unwrap_or_else(interrupted_failure);
      primary_failure.get_or_insert_with(|| failure.clone());
      interrupted_attempts += 1;
      append_to_history_scoped(
        transaction,
        events,
        run_id,
        generated_event_id(),
        now,
        event_schema_version,
        Some(scope.clone()),
        RunEventPayload::StepAttemptFailed(StepAttemptFailedData {
          node_id: attempt.identity.node_id.clone(),
          attempt: attempt.identity.attempt,
          invocation_id: attempt.identity.invocation_id.clone(),
          failure,
        }),
      )?;
    }
    for group in execution
      .parallel_groups
      .values()
      .filter(|group| group.status == ParallelGroupStatus::Started)
    {
      let start_id = format!("__woml_parallel__{}__start", group.parallel_id);
      let cancelled_node_ids = descriptor
        .body
        .edges
        .iter()
        .filter(|edge| {
          edge.from == start_id && edge.parallel_id.as_deref() == Some(group.parallel_id.as_str())
        })
        .map(|edge| edge.to.clone())
        .collect::<Vec<_>>();
      append_to_history_scoped(
        transaction,
        events,
        run_id,
        generated_event_id(),
        now,
        event_schema_version,
        Some(scope.clone()),
        RunEventPayload::ParallelGroupCompleted(ParallelGroupCompletedData {
          parallel_id: group.parallel_id.clone(),
          outcome: ParallelGroupOutcome::Failed,
          failed_node_ids: Vec::new(),
          cancelled_node_ids,
        }),
      )?;
    }
  }

  let recovered = fold_events(events)?;
  let loop_state = recovered
    .for_each
    .get(&descriptor.for_each_id)
    .expect("the recovering loop exists");
  let active_indexes = loop_state
    .iterations
    .iter()
    .filter_map(|(index, status)| {
      matches!(status, ForEachIterationStatus::Started).then_some(*index)
    })
    .collect::<Vec<_>>();
  for index in active_indexes {
    let projection = fold_events(events)?;
    let failure = projection
      .for_each
      .get(&descriptor.for_each_id)
      .and_then(|loop_state| loop_state.executions.get(&index))
      .and_then(|execution| {
        execution.attempts.iter().rev().find_map(|attempt| {
          if let AttemptStatus::Failed { failure } = &attempt.status {
            Some((attempt.identity.node_id.clone(), failure.clone()))
          } else {
            None
          }
        })
      });
    let (failed_node_id, failure_kind) = failure
      .map(|(node_id, failure)| (node_id, for_each_recovery_failure_kind(failure.kind)))
      .unwrap_or_else(|| {
        (
          descriptor.body.terminal_node_id.clone(),
          forced_failure
            .as_ref()
            .map_or(ForEachIterationFailureKind::Interrupted, |failure| {
              for_each_recovery_failure_kind(failure.kind)
            }),
        )
      });
    append_to_history_scoped(
      transaction,
      events,
      run_id,
      generated_event_id(),
      now,
      event_schema_version,
      Some(ForEachIterationScope {
        for_each_id: descriptor.for_each_id.clone(),
        index,
      }),
      RunEventPayload::ForEachIterationFailed(ForEachIterationFailedData {
        for_each_id: descriptor.for_each_id.clone(),
        failed_node_id,
        failure_kind,
      }),
    )?;
  }

  let recovered = fold_events(events)?;
  let loop_state = recovered
    .for_each
    .get(&descriptor.for_each_id)
    .expect("the recovering loop exists");
  let pending_indexes = (0..loop_state.total)
    .filter(|index| !loop_state.iterations.contains_key(index))
    .collect::<Vec<_>>();
  for index in pending_indexes {
    append_to_history_scoped(
      transaction,
      events,
      run_id,
      generated_event_id(),
      now,
      event_schema_version,
      Some(ForEachIterationScope {
        for_each_id: descriptor.for_each_id.clone(),
        index,
      }),
      RunEventPayload::ForEachIterationSkipped(ForEachIterationSkippedData {
        for_each_id: descriptor.for_each_id.clone(),
        reason: ForEachSkipReason::Failure,
      }),
    )?;
  }

  let recovered = fold_events(events)?;
  let loop_state = recovered
    .for_each
    .get(&descriptor.for_each_id)
    .expect("the recovering loop exists");
  let succeeded = loop_state
    .iterations
    .values()
    .filter(|status| matches!(status, ForEachIterationStatus::Succeeded { .. }))
    .count() as u32;
  let failed = loop_state
    .iterations
    .values()
    .filter(|status| matches!(status, ForEachIterationStatus::Failed { .. }))
    .count() as u32;
  let skipped = loop_state
    .iterations
    .values()
    .filter(|status| matches!(status, ForEachIterationStatus::Skipped))
    .count() as u32;
  let aggregate = serde_json::json!({
    "total": loop_state.total,
    "succeeded": succeeded,
    "failed": failed,
    "skipped": skipped,
  });
  append_to_history(
    transaction,
    events,
    run_id,
    generated_event_id(),
    now,
    event_schema_version,
    RunEventPayload::ForEachFailed(ForEachSettledData {
      for_each_id: descriptor.for_each_id.clone(),
      total: loop_state.total,
      succeeded,
      failed,
      skipped,
      aggregate_digest: sha256_prefixed(&canonical_json(&aggregate)?),
    }),
  )?;
  Ok(Some((
    interrupted_attempts,
    primary_failure.unwrap_or_else(interrupted_failure),
  )))
}

fn parallel_failure_message(policy: ParallelFailurePolicy, failed_count: usize) -> String {
  match policy {
    ParallelFailurePolicy::FailFast => {
      "A parallel child failed; active siblings were cancelled.".to_string()
    }
    ParallelFailurePolicy::WaitAll if failed_count == 1 => "One parallel child failed.".to_string(),
    ParallelFailurePolicy::WaitAll => format!("{failed_count} parallel children failed."),
  }
}

fn attempt_run_failed_data(
  event_schema_version: u32,
  node_id: String,
  attempt: u32,
  invocation_id: String,
  failure: AttemptFailure,
) -> RunFailedData {
  match event_schema_version {
    RUN_EVENT_SCHEMA_VERSION_V1 => RunFailedData::V1(RunFailedDataV1 {
      node_id: Some(node_id),
      attempt: Some(attempt),
      invocation_id: Some(invocation_id),
      failure,
    }),
    RUN_EVENT_SCHEMA_VERSION_V2
    | RUN_EVENT_SCHEMA_VERSION_V3
    | RUN_EVENT_SCHEMA_VERSION_V4
    | RUN_EVENT_SCHEMA_VERSION_V5
    | RUN_EVENT_SCHEMA_VERSION_V6
    | RUN_EVENT_SCHEMA_VERSION_V7
    | RUN_EVENT_SCHEMA_VERSION_V8
    | RUN_EVENT_SCHEMA_VERSION_V9
    | crate::RUN_EVENT_SCHEMA_VERSION_V10
    | crate::RUN_EVENT_SCHEMA_VERSION_V11
    | crate::RUN_EVENT_SCHEMA_VERSION_V12
    | crate::RUN_EVENT_SCHEMA_VERSION_V13
    | crate::RUN_EVENT_SCHEMA_VERSION_V14
    | crate::RUN_EVENT_SCHEMA_VERSION_V15
    | crate::RUN_EVENT_SCHEMA_VERSION_V16
    | crate::RUN_EVENT_SCHEMA_VERSION_V17
    | crate::RUN_EVENT_SCHEMA_VERSION_V18
    | crate::RUN_EVENT_SCHEMA_VERSION_V19
    | crate::RUN_EVENT_SCHEMA_VERSION_V20 => RunFailedData::V2(RunFailedDataV2::Attempt {
      node_id,
      attempt,
      invocation_id,
      failure,
    }),
    _ => unreachable!("event versions are validated before recovery"),
  }
}

fn generated_event_id() -> String {
  format!("evt_{}", Uuid::new_v4().simple())
}

#[derive(Debug, Error)]
pub enum DurableEngineError {
  #[error(transparent)]
  Store(#[from] DurableStoreError),
  #[error(transparent)]
  InvalidModel(#[from] ModelValidationError),
  #[error("{0}")]
  Contract(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepFailureDisposition {
  RetryScheduled {
    next_attempt: u32,
    scheduled_at: DateTime<Utc>,
  },
  StepFailed,
  RunFailureDeferred,
  RunFailed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepFailureCommit {
  pub disposition: StepFailureDisposition,
  pub projection: RunProjection,
}

#[derive(Debug)]
pub struct DurableDagEngine {
  workflow: CompiledWorkflowDefinition,
  definition_hash: String,
  store: DurableEventStore,
}

impl DurableDagEngine {
  pub fn new(
    workflow: CompiledWorkflowDefinition,
    definition_hash: impl Into<String>,
    mut store: DurableEventStore,
  ) -> Result<Self, DurableEngineError> {
    workflow.validate_for_durable_execution()?;
    let definition_hash = definition_hash.into();
    store.register_definition(&workflow, &definition_hash)?;
    Ok(Self {
      workflow,
      definition_hash,
      store,
    })
  }

  pub fn new_for_event_history(
    workflow: CompiledWorkflowDefinition,
    definition_hash: impl Into<String>,
    mut store: DurableEventStore,
  ) -> Result<Self, DurableEngineError> {
    workflow.validate_structure()?;
    let definition_hash = definition_hash.into();
    store.register_definition(&workflow, &definition_hash)?;
    Ok(Self {
      workflow,
      definition_hash,
      store,
    })
  }

  pub fn resume(store: DurableEventStore, run_id: &str) -> Result<Self, DurableEngineError> {
    let binding = store.run_binding(run_id)?;
    let workflow = store.definition(&binding.definition_hash)?;
    if workflow.workflow_id != binding.workflow_id {
      return Err(DurableEngineError::Contract(
        "Stored run binding does not match its compiled definition.".to_string(),
      ));
    }
    Ok(Self {
      workflow,
      definition_hash: binding.definition_hash,
      store,
    })
  }

  pub fn workflow(&self) -> &CompiledWorkflowDefinition {
    &self.workflow
  }

  pub fn definition_hash(&self) -> &str {
    &self.definition_hash
  }

  pub fn correction_action_externally_resolved(
    &self,
    run_id: &str,
    saga_id: &str,
    action_id: &str,
  ) -> Result<bool, DurableEngineError> {
    Ok(
      self
        .store
        .correction_action_externally_resolved(run_id, saga_id, action_id)?,
    )
  }

  pub fn decide_run_outcome(
    &mut self,
    run_id: &str,
    outcome: crate::event::RunOutcomeDecidedData,
    occurred_at: DateTime<Utc>,
  ) -> Result<RunProjection, DurableEngineError> {
    Ok(
      self
        .store
        .decide_run_outcome(run_id, outcome, occurred_at)?,
    )
  }

  pub fn settle_run_timeout(
    &mut self,
    run_id: &str,
    occurred_at: DateTime<Utc>,
  ) -> Result<RunTimeoutSettlement, DurableEngineError> {
    Ok(self.store.settle_run_timeout(run_id, occurred_at)?)
  }

  pub fn start_run(
    &mut self,
    event_id: impl Into<String>,
    run_id: impl Into<String>,
    occurred_at: DateTime<Utc>,
    trigger: Map<String, Value>,
  ) -> Result<RunProjection, DurableEngineError> {
    let (_, projection) = self.store.start_run(
      event_id,
      run_id,
      occurred_at,
      self.workflow.workflow_id.clone(),
      self.definition_hash.clone(),
      trigger,
    )?;
    Ok(projection)
  }

  pub fn append_payload(
    &mut self,
    event_id: impl Into<String>,
    run_id: &str,
    occurred_at: DateTime<Utc>,
    payload: RunEventPayload,
  ) -> Result<RunProjection, DurableEngineError> {
    let event_id = event_id.into();
    if matches!(
      run_event_schema_version_for_model(self.workflow.schema_version),
      crate::RUN_EVENT_SCHEMA_VERSION_V10
        | crate::RUN_EVENT_SCHEMA_VERSION_V11
        | crate::RUN_EVENT_SCHEMA_VERSION_V12
        | crate::RUN_EVENT_SCHEMA_VERSION_V13
        | crate::RUN_EVENT_SCHEMA_VERSION_V14
        | crate::RUN_EVENT_SCHEMA_VERSION_V15
        | crate::RUN_EVENT_SCHEMA_VERSION_V16
        | crate::RUN_EVENT_SCHEMA_VERSION_V17
        | crate::RUN_EVENT_SCHEMA_VERSION_V18
        | crate::RUN_EVENT_SCHEMA_VERSION_V19
        | crate::RUN_EVENT_SCHEMA_VERSION_V20
    ) && matches!(
      &payload,
      RunEventPayload::RunSucceeded(_) | RunEventPayload::RunFailed(_)
    ) {
      let translated = expand_model_v11_payload(&self.workflow, run_id, payload)?;
      return Ok(
        self.store.append_payloads_atomically(
          run_id,
          translated
            .into_iter()
            .enumerate()
            .map(|(index, payload)| {
              (
                if index == 0 {
                  event_id.clone()
                } else {
                  generated_event_id()
                },
                occurred_at,
                payload,
              )
            })
            .collect(),
        )?,
      );
    }
    validate_payload_against_definition(&self.workflow, &self.definition_hash, &payload)
      .map_err(DurableEngineError::Contract)?;
    if let RunEventPayload::StepAttemptStarted(data) = &payload {
      let ready = self.ready_node_ids(run_id)?;
      if !ready.iter().any(|node_id| node_id == &data.node_id) {
        return Err(DurableEngineError::Contract(format!(
          "Node {:?} is not ready for execution.",
          data.node_id
        )));
      }
    }
    if self.workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V6
      && matches!(
        payload,
        RunEventPayload::StepAttemptFailed(_) | RunEventPayload::StepRetryScheduled(_)
      )
    {
      return Err(DurableEngineError::Contract(
        "Model v6 step failures and retry schedules must use the atomic failure API.".to_string(),
      ));
    }
    let (_, projection) = self
      .store
      .append_payload(run_id, event_id, occurred_at, payload)?;
    Ok(projection)
  }

  pub fn append_scoped_payload(
    &mut self,
    event_id: impl Into<String>,
    run_id: &str,
    occurred_at: DateTime<Utc>,
    iteration: crate::event::ForEachIterationScope,
    payload: RunEventPayload,
  ) -> Result<RunProjection, DurableEngineError> {
    validate_scoped_payload_against_definition(
      &self.workflow,
      &self.definition_hash,
      run_id,
      &iteration,
      &payload,
    )
    .map_err(DurableEngineError::Contract)?;
    let (_, projection) =
      self
        .store
        .append_payload_scoped(run_id, event_id, occurred_at, Some(iteration), payload)?;
    Ok(projection)
  }

  pub fn record_step_attempt_failure(
    &mut self,
    run_id: &str,
    failed_at: DateTime<Utc>,
    failure: StepAttemptFailedData,
  ) -> Result<StepFailureCommit, DurableEngineError> {
    if self.workflow.schema_version < crate::COMPILED_MODEL_SCHEMA_VERSION_V6 {
      return Err(DurableEngineError::Contract(
        "The atomic retry failure API requires compiled model v6.".to_string(),
      ));
    }
    let node = self.workflow.node(&failure.node_id).ok_or_else(|| {
      DurableEngineError::Contract(format!(
        "Attempt failure references unknown node {:?}.",
        failure.node_id
      ))
    })?;
    let parallel_child = self
      .workflow
      .parallel_group_for_child(&failure.node_id)
      .is_some();
    let fork_child = self.workflow.fork_branch_owner(&failure.node_id).is_some();
    let reusable_lifecycle =
      self
        .workflow
        .reusable_definitions
        .iter()
        .flatten()
        .any(|definition| {
          matches!(
            definition,
            crate::model::CompiledReusableInvocation::Step {
              node_id,
              lifecycle: Some(_),
              ..
            } if node_id == &failure.node_id
          )
        });
    let lock_body = self
      .workflow
      .graph
      .locks
      .as_deref()
      .unwrap_or_default()
      .iter()
      .any(|lock| {
        let mut pending = vec![lock.body_entry_node_id.as_str()];
        let mut visited = HashSet::new();
        while let Some(candidate) = pending.pop() {
          if !visited.insert(candidate) {
            continue;
          }
          if candidate == failure.node_id {
            return true;
          }
          if candidate == lock.body_terminal_node_id {
            continue;
          }
          pending.extend(
            self
              .workflow
              .graph
              .edges
              .iter()
              .filter(|edge| edge.from == candidate)
              .map(|edge| edge.to.as_str()),
          );
        }
        false
      });
    let saga_body = self.workflow.saga_for_body_node(&failure.node_id).is_some();
    let projection = self.projection(run_id)?;
    let fork_coordinated = self.workflow.schema_version >= crate::COMPILED_MODEL_SCHEMA_VERSION_V13
      && projection.forks.values().any(|fork| {
        fork.join_status == crate::projection::ForkJoinStatus::Pending
          || fork.branches.len()
            < self
              .workflow
              .graph
              .forks
              .as_deref()
              .unwrap_or_default()
              .iter()
              .find(|definition| definition.fork_id == fork.fork_id)
              .map_or(0, |definition| definition.branches.len())
      });
    if !matches!(
      projection.latest_attempt(&failure.node_id),
      Some(attempt)
        if attempt.identity.attempt == failure.attempt
          && attempt.identity.invocation_id == failure.invocation_id
          && attempt.status == AttemptStatus::Started
    ) {
      return Err(DurableEngineError::Contract(
        "Attempt failure does not close the active compiled step attempt.".to_string(),
      ));
    }

    let cancellation_settlement = projection.status == RunStatus::Cancelling
      && failure.failure.kind == AttemptFailureKind::InvocationCancelled;
    let retry = node.retry_policy.as_ref().filter(|policy| {
      !cancellation_settlement
        && failure.failure.kind == AttemptFailureKind::ScriptThrew
        && failure.attempt < policy.max_attempts
    });
    let (payloads, disposition) = if let Some(policy) = retry {
      let next_attempt = failure.attempt + 1;
      let delay_ms = policy.delay_before_attempt(next_attempt).ok_or_else(|| {
        DurableEngineError::Contract("Compiled retry delay is invalid.".to_string())
      })?;
      let delay = chrono::Duration::milliseconds(i64::try_from(delay_ms).map_err(|_| {
        DurableEngineError::Contract("Retry delay exceeds the clock range.".to_string())
      })?);
      let scheduled_at = failed_at + delay;
      (
        vec![
          (
            generated_event_id(),
            failed_at,
            RunEventPayload::StepAttemptFailed(failure.clone()),
          ),
          (
            generated_event_id(),
            failed_at,
            RunEventPayload::StepRetryScheduled(StepRetryScheduledData {
              node_id: failure.node_id.clone(),
              failed_attempt: failure.attempt,
              next_attempt,
              scheduled_at,
            }),
          ),
        ],
        StepFailureDisposition::RetryScheduled {
          next_attempt,
          scheduled_at,
        },
      )
    } else if parallel_child || fork_child || fork_coordinated || cancellation_settlement {
      (
        vec![(
          generated_event_id(),
          failed_at,
          RunEventPayload::StepAttemptFailed(failure.clone()),
        )],
        StepFailureDisposition::StepFailed,
      )
    } else if reusable_lifecycle || lock_body || saga_body {
      (
        vec![(
          generated_event_id(),
          failed_at,
          RunEventPayload::StepAttemptFailed(failure.clone()),
        )],
        StepFailureDisposition::RunFailureDeferred,
      )
    } else {
      (
        vec![
          (
            generated_event_id(),
            failed_at,
            RunEventPayload::StepAttemptFailed(failure.clone()),
          ),
          (
            generated_event_id(),
            failed_at,
            RunEventPayload::RunFailed(attempt_run_failed_data(
              run_event_schema_version_for_model(self.workflow.schema_version),
              failure.node_id.clone(),
              failure.attempt,
              failure.invocation_id.clone(),
              failure.failure.clone(),
            )),
          ),
        ],
        StepFailureDisposition::RunFailed,
      )
    };
    let projection = self.store.append_payloads_atomically(run_id, payloads)?;
    Ok(StepFailureCommit {
      disposition,
      projection,
    })
  }

  pub fn start_step_attempt(
    &mut self,
    run_id: &str,
    node_id: &str,
    attempt: u32,
    invocation_id: impl Into<String>,
    occurred_at: DateTime<Utc>,
  ) -> Result<RunProjection, DurableEngineError> {
    let node = self
      .workflow
      .node(node_id)
      .ok_or_else(|| DurableEngineError::Contract(format!("Unknown step node {node_id:?}.")))?;
    let idempotency_key = step_effect_idempotency_key(run_id, &self.definition_hash, node_id);
    self.append_payload(
      generated_event_id(),
      run_id,
      occurred_at,
      RunEventPayload::StepAttemptStarted(crate::event::StepAttemptStartedData {
        node_id: node_id.to_string(),
        attempt,
        invocation_id: invocation_id.into(),
        handler: node.handler.clone(),
        idempotency_key: Some(idempotency_key),
      }),
    )
  }

  /// Commits a successful forward effect and its compensation eligibility as
  /// one Store v17 transaction. A validation or SQLite failure rolls back both
  /// records, so replay can never observe success without registration.
  pub fn record_step_success_with_compensation(
    &mut self,
    run_id: &str,
    success: crate::StepAttemptSucceededData,
    saga_id: &str,
    action_id: &str,
  ) -> Result<RunProjection, DurableEngineError> {
    let completion_sequence = self.events(run_id)?.len() as u64 + 1;
    self.append_payloads_atomically(
      run_id,
      vec![
        RunEventPayload::StepAttemptSucceeded(success.clone()),
        RunEventPayload::CompensationRegistered(crate::CompensationRegisteredData {
          saga_id: saga_id.to_string(),
          action_id: action_id.to_string(),
          for_step_id: success.node_id,
          completion_sequence,
        }),
      ],
    )
  }

  pub fn request_approval(
    &mut self,
    run_id: &str,
    occurred_at: DateTime<Utc>,
    request: ApprovalRequestedData,
  ) -> Result<IssuedApprovalToken, DurableEngineError> {
    let ready = self.ready_node_ids(run_id)?;
    if !ready.iter().any(|node_id| node_id == &request.approval_id) {
      return Err(DurableEngineError::Contract(format!(
        "Approval {:?} is not ready to enter waiting state.",
        request.approval_id
      )));
    }
    let (_, token) =
      self
        .store
        .request_approval_atomically(run_id, generated_event_id(), occurred_at, request)?;
    Ok(token)
  }

  pub fn request_rich_approval(
    &mut self,
    run_id: &str,
    occurred_at: DateTime<Utc>,
    binding: &BoundRichApprovalReview,
  ) -> Result<RunProjection, DurableEngineError> {
    let ready = self.ready_node_ids(run_id)?;
    if !ready.iter().any(|node_id| node_id == &binding.approval_id) {
      return Err(DurableEngineError::Contract(format!(
        "Rich approval {:?} is not ready to enter waiting state.",
        binding.approval_id
      )));
    }
    Ok(
      self
        .store
        .create_rich_approval_request(binding, occurred_at)?,
    )
  }

  pub fn inspect_rich_approval(
    &self,
    run_id: &str,
    request_id: &str,
  ) -> Result<RichApprovalInspectionV1, DurableEngineError> {
    Ok(self.store.inspect_rich_approval(
      run_id,
      request_id,
      RICH_APPROVAL_INSPECTION_DECISION_LIMIT,
    )?)
  }

  pub fn settle_pending_rich_approval(
    &mut self,
    run_id: &str,
    reason: RichApprovalResolutionReason,
    settled_at: DateTime<Utc>,
  ) -> Result<bool, DurableEngineError> {
    let projection = self.projection(run_id)?;
    let Some(request) = projection
      .rich_approval_requests
      .values()
      .find(|request| request.status == crate::RichApprovalRequestStatus::Waiting)
      .cloned()
    else {
      return Ok(false);
    };
    self.store.settle_rich_approval_without_vote(
      run_id,
      &request.request_id,
      &request.review_digest,
      reason,
      settled_at,
    )?;
    Ok(true)
  }

  pub fn register_durable_wait(
    &mut self,
    run_id: &str,
    occurred_at: DateTime<Utc>,
    registration: crate::WaitRegisteredData,
  ) -> Result<RunProjection, DurableEngineError> {
    let ready = self.ready_node_ids(run_id)?;
    if !ready.iter().any(|node_id| node_id == &registration.node_id) {
      return Err(DurableEngineError::Contract(format!(
        "Durable wait {:?} is not ready to enter waiting state.",
        registration.wait_id
      )));
    }
    Ok(self.store.register_durable_wait_atomically(
      run_id,
      generated_event_id(),
      occurred_at,
      registration,
    )?)
  }

  pub fn register_lock_waiter(
    &mut self,
    run_id: &str,
    lock_id: &str,
    key: &str,
    occurred_at: DateTime<Utc>,
    deadline: DateTime<Utc>,
  ) -> Result<RunProjection, DurableEngineError> {
    let descriptor = self
      .workflow
      .durable_lock(lock_id)
      .ok_or_else(|| DurableEngineError::Contract("Unknown Model v18 lock.".to_string()))?;
    let ready = self.ready_node_ids(run_id)?;
    if !ready
      .iter()
      .any(|node_id| node_id == &descriptor.acquire_node_id)
    {
      return Err(DurableEngineError::Contract(format!(
        "Lock {lock_id:?} is not ready to acquire."
      )));
    }
    Ok(self.store.register_lock_waiter_atomically(
      run_id,
      lock_id,
      key,
      &descriptor.acquire_node_id,
      occurred_at,
      deadline,
    )?)
  }

  pub fn release_lock(
    &mut self,
    run_id: &str,
    lock_id: &str,
    result: Value,
    occurred_at: DateTime<Utc>,
  ) -> Result<RunProjection, DurableEngineError> {
    Ok(
      self
        .store
        .release_lock_atomically(run_id, lock_id, result, occurred_at)?,
    )
  }

  pub fn timeout_lock_waiter(
    &mut self,
    run_id: &str,
    lock_id: &str,
    occurred_at: DateTime<Utc>,
  ) -> Result<RunProjection, DurableEngineError> {
    Ok(
      self
        .store
        .timeout_lock_waiter_atomically(run_id, lock_id, occurred_at)?,
    )
  }

  pub fn renew_lock_lease(
    &mut self,
    run_id: &str,
    lock_id: &str,
    now: DateTime<Utc>,
  ) -> Result<LockLeaseMaintenance, DurableEngineError> {
    Ok(
      self
        .store
        .renew_lock_lease_atomically(run_id, lock_id, now)?,
    )
  }

  pub fn renew_iteration_lock_lease(
    &mut self,
    run_id: &str,
    lock_id: &str,
    iteration: ForEachIterationScope,
    now: DateTime<Utc>,
  ) -> Result<LockLeaseMaintenance, DurableEngineError> {
    Ok(
      self
        .store
        .renew_lock_lease_atomically_scoped(run_id, lock_id, Some(iteration), now)?,
    )
  }

  pub fn cancel_lock_waiters(
    &mut self,
    run_id: &str,
    cancellation_id: &str,
    now: DateTime<Utc>,
  ) -> Result<RunProjection, DurableEngineError> {
    Ok(
      self
        .store
        .cancel_lock_waiters_atomically(run_id, cancellation_id, now)?,
    )
  }

  pub fn register_iteration_lock_waiter(
    &mut self,
    run_id: &str,
    lock_id: &str,
    key: &str,
    iteration: ForEachIterationScope,
    occurred_at: DateTime<Utc>,
    deadline: DateTime<Utc>,
  ) -> Result<RunProjection, DurableEngineError> {
    let descriptor = self
      .workflow
      .durable_lock(lock_id)
      .ok_or_else(|| DurableEngineError::Contract("Unknown Model v18 lock.".to_string()))?;
    let execution_identity = format!(
      "{}:{}:{}",
      iteration.for_each_id, iteration.index, descriptor.acquire_node_id
    );
    Ok(self.store.register_lock_waiter_atomically_scoped(
      run_id,
      lock_id,
      key,
      &execution_identity,
      Some(iteration),
      occurred_at,
      deadline,
    )?)
  }

  pub fn release_iteration_lock(
    &mut self,
    run_id: &str,
    lock_id: &str,
    iteration: ForEachIterationScope,
    result: Value,
    occurred_at: DateTime<Utc>,
  ) -> Result<RunProjection, DurableEngineError> {
    Ok(self.store.release_lock_atomically_scoped(
      run_id,
      lock_id,
      Some(iteration),
      result,
      occurred_at,
    )?)
  }

  pub fn timeout_iteration_lock_waiter(
    &mut self,
    run_id: &str,
    lock_id: &str,
    iteration: ForEachIterationScope,
    occurred_at: DateTime<Utc>,
  ) -> Result<RunProjection, DurableEngineError> {
    Ok(self.store.timeout_lock_waiter_atomically_scoped(
      run_id,
      lock_id,
      Some(iteration),
      occurred_at,
    )?)
  }

  pub fn settle_timer_wait(
    &mut self,
    run_id: &str,
    wait_id: &str,
    now: DateTime<Utc>,
  ) -> Result<TimerWaitSettlement, DurableEngineError> {
    Ok(self.store.settle_timer_wait(run_id, wait_id, now)?)
  }

  pub fn reissue_waiting_approval_token(
    &mut self,
    run_id: &str,
    approval_id: &str,
    request_id: &str,
    issued_at: DateTime<Utc>,
  ) -> Result<IssuedApprovalToken, DurableEngineError> {
    Ok(
      self
        .store
        .reissue_approval_token(run_id, approval_id, request_id, issued_at)?,
    )
  }

  pub fn publish_pure_result(
    &mut self,
    run_id: &str,
    node_id: &str,
    invocation_id: &str,
    output: Value,
  ) -> Result<RunProjection, DurableEngineError> {
    let node = self.workflow.node(node_id).ok_or_else(|| {
      DurableEngineError::Contract(format!("Unknown pure result node {node_id:?}."))
    })?;
    if !matches!(
      node.handler.as_str(),
      "engine.branch-result" | "engine.choice-result" | "engine.saga-result"
    ) {
      return Err(DurableEngineError::Contract(format!(
        "Node {node_id:?} is not a supported pure-result operation."
      )));
    }
    let ready = self.ready_node_ids(run_id)?;
    if !ready.iter().any(|ready_id| ready_id == node_id) {
      return Err(DurableEngineError::Contract(format!(
        "Node {node_id:?} is not ready for pure result publication."
      )));
    }
    Ok(self.store.append_payloads_atomically(
      run_id,
      vec![
        (
          generated_event_id(),
          Utc::now(),
          RunEventPayload::StepAttemptStarted(crate::event::StepAttemptStartedData {
            node_id: node_id.to_string(),
            attempt: 1,
            invocation_id: invocation_id.to_string(),
            handler: node.handler.clone(),
            idempotency_key: (self.workflow.schema_version
              >= crate::COMPILED_MODEL_SCHEMA_VERSION_V6)
              .then(|| {
                step_effect_idempotency_key(run_id, &self.definition_hash, node_id)
              }),
          }),
        ),
        (
          generated_event_id(),
          Utc::now(),
          RunEventPayload::StepAttemptSucceeded(crate::event::StepAttemptSucceededData {
            node_id: node_id.to_string(),
            attempt: 1,
            invocation_id: invocation_id.to_string(),
            output,
          }),
        ),
      ],
    )?)
  }

  pub fn projection(&self, run_id: &str) -> Result<RunProjection, DurableEngineError> {
    Ok(self.store.projection(run_id)?)
  }

  pub fn events(&self, run_id: &str) -> Result<Vec<RunEvent>, DurableEngineError> {
    Ok(self.store.events(run_id)?)
  }

  pub fn ready_node_ids(&self, run_id: &str) -> Result<Vec<String>, DurableEngineError> {
    let projection = self.projection(run_id)?;
    ready_node_ids_for_projection(&self.workflow, &self.definition_hash, &projection)
      .map_err(DurableEngineError::Contract)
  }

  pub fn ready_node_ids_at(
    &self,
    run_id: &str,
    now: DateTime<Utc>,
  ) -> Result<Vec<String>, DurableEngineError> {
    let projection = self.projection(run_id)?;
    ready_node_ids_for_projection_at(&self.workflow, &self.definition_hash, &projection, now)
      .map_err(DurableEngineError::Contract)
  }

  pub(crate) fn append_payloads_atomically(
    &mut self,
    run_id: &str,
    payloads: Vec<RunEventPayload>,
  ) -> Result<RunProjection, DurableEngineError> {
    Ok(
      self.store.append_payloads_atomically(
        run_id,
        payloads
          .into_iter()
          .map(|payload| (generated_event_id(), Utc::now(), payload))
          .collect(),
      )?,
    )
  }

  pub(crate) fn append_scoped_payloads_atomically(
    &mut self,
    run_id: &str,
    iteration: crate::event::ForEachIterationScope,
    payloads: Vec<RunEventPayload>,
  ) -> Result<RunProjection, DurableEngineError> {
    Ok(
      self.store.append_payloads_scoped_atomically(
        run_id,
        Some(iteration),
        payloads
          .into_iter()
          .map(|payload| (generated_event_id(), Utc::now(), payload))
          .collect(),
      )?,
    )
  }

  pub fn recover_interrupted_runs(&mut self) -> Result<RecoveryReport, DurableEngineError> {
    Ok(self.store.recover_interrupted_runs()?)
  }

  pub fn into_store(self) -> DurableEventStore {
    self.store
  }
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
