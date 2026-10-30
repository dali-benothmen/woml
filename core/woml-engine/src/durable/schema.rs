//! Frozen store version markers and exact schema/migration DDL literals.
//!
//! Relocation is not a schema change. Migrations and startup validation retain
//! their existing versions, ordering, constraints, indexes, and SQL bytes.

pub const DURABLE_STORE_SCHEMA_VERSION: u32 = 20;

pub(in crate::durable) const STORE_SCHEMA_VERSION_V1: &str = "1";

pub(in crate::durable) const STORE_SCHEMA_VERSION_V2: &str = "2";

pub(in crate::durable) const STORE_SCHEMA_VERSION_V3: &str = "3";

pub(in crate::durable) const STORE_SCHEMA_VERSION_V4: &str = "4";

pub(in crate::durable) const STORE_SCHEMA_VERSION_V5: &str = "5";

pub(in crate::durable) const STORE_SCHEMA_VERSION_V6: &str = "6";

pub(in crate::durable) const STORE_SCHEMA_VERSION_V7: &str = "7";

pub(in crate::durable) const STORE_SCHEMA_VERSION_V8: &str = "8";

pub(in crate::durable) const STORE_SCHEMA_VERSION_V9: &str = "9";

pub(in crate::durable) const STORE_SCHEMA_VERSION_V10: &str = "10";

pub(in crate::durable) const STORE_SCHEMA_VERSION_V11: &str = "11";

pub(in crate::durable) const STORE_SCHEMA_VERSION_V12: &str = "12";

pub(in crate::durable) const STORE_SCHEMA_VERSION_V13: &str = "13";

pub(in crate::durable) const STORE_SCHEMA_VERSION_V14: &str = "14";

pub(in crate::durable) const STORE_SCHEMA_VERSION_V15: &str = "15";

pub(in crate::durable) const STORE_SCHEMA_VERSION_V16: &str = "16";

pub(in crate::durable) const STORE_SCHEMA_VERSION_V17: &str = "17";

pub(in crate::durable) const STORE_SCHEMA_VERSION_V18: &str = "18";

pub(in crate::durable) const STORE_SCHEMA_VERSION_V19: &str = "19";

pub(in crate::durable) const STORE_SCHEMA_VERSION_V20: &str = "20";

pub(in crate::durable) const CREATE_INFRASTRUCTURE_POLICY_SCHEMA_V19: &str = r#"
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

pub(in crate::durable) const CREATE_RUN_SUMMARY_SCHEMA_V11: &str = r#"
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

pub(in crate::durable) const MIGRATE_RUNTIME_POLICY_SCHEMA_V12: &str = r#"
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

pub(in crate::durable) const CREATE_DURABLE_STATE_SCHEMA_V13: &str = r#"
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

pub(in crate::durable) const CREATE_PRODUCTION_RUNTIME_SCHEMA_V14: &str = r#"
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

pub(in crate::durable) const CREATE_DURABLE_WAIT_SCHEMA_V15: &str = r#"
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

pub(in crate::durable) const CREATE_LOCK_SCHEMA_V16: &str = r#"
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

pub(in crate::durable) const CREATE_CORRECTION_SCHEMA_V17: &str = r#"
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

pub(in crate::durable) const CREATE_RICH_APPROVAL_SCHEMA_V18: &str = r#"
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

pub(in crate::durable) const CREATE_SCHEMA: &str = r#"
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

pub(in crate::durable) const CREATE_APPROVAL_SCHEMA_V2: &str = r#"
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

pub(in crate::durable) const CREATE_NOTIFICATION_SCHEMA_V3: &str = r#"
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

pub(in crate::durable) const CREATE_TRIGGER_SCHEMA_V4: &str = r#"
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

pub(in crate::durable) const CREATE_SCHEDULE_SCHEMA_V5: &str = r#"
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

pub(in crate::durable) const CREATE_INTERVAL_SCHEMA_V6: &str = r#"
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

pub(in crate::durable) const CREATE_INTERNAL_EVENT_SCHEMA_V7: &str = r#"
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

pub(in crate::durable) const CREATE_MODULE_ARTIFACT_SCHEMA_V8: &str = r#"
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

pub(in crate::durable) const CREATE_WORKFLOW_CALL_SCHEMA_V9: &str = r#"
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

pub(in crate::durable) const CREATE_WORKFLOW_RUNTIME_SCHEMA_V10: &str = r#"
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
