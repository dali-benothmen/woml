//! Private remote worker journal schema.

pub const WORKER_JOURNAL_SCHEMA_VERSION: u32 = 1;

pub(in crate::remote_worker_journal) const CREATE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS woml_worker_metadata (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS woml_worker_dispatches (
  dispatch_id TEXT PRIMARY KEY,
  package_digest TEXT NOT NULL,
  worker_id TEXT NOT NULL,
  run_id TEXT NOT NULL,
  node_id TEXT NOT NULL,
  iteration_identity TEXT NOT NULL,
  attempt INTEGER NOT NULL,
  invocation_id TEXT NOT NULL UNIQUE,
  target TEXT NOT NULL,
  authority_digest TEXT NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('accepted','started','succeeded','failed','cancelled','ambiguous')),
  terminal_json TEXT,
  terminal_digest TEXT,
  accepted_at TEXT NOT NULL,
  started_at TEXT,
  terminal_at TEXT,
  CHECK ((terminal_json IS NULL) = (terminal_digest IS NULL)),
  CHECK ((state IN ('succeeded','failed','cancelled')) = (terminal_json IS NOT NULL))
);
CREATE TABLE IF NOT EXISTS woml_worker_journal (
  local_sequence INTEGER PRIMARY KEY AUTOINCREMENT,
  dispatch_id TEXT NOT NULL,
  worker_id TEXT NOT NULL,
  session_id TEXT NOT NULL,
  package_digest TEXT NOT NULL,
  attempt_identity_digest TEXT NOT NULL,
  authority_digest TEXT NOT NULL,
  fact_digest TEXT NOT NULL,
  record_type TEXT NOT NULL CHECK (record_type IN ('accepted','started','succeeded','failed','cancelled','ambiguous')),
  recorded_at TEXT NOT NULL,
  previous_record_digest TEXT NOT NULL,
  record_digest TEXT NOT NULL UNIQUE,
  FOREIGN KEY (dispatch_id) REFERENCES woml_worker_dispatches(dispatch_id)
);
CREATE INDEX IF NOT EXISTS woml_worker_dispatch_state
  ON woml_worker_dispatches(state, accepted_at, dispatch_id);
"#;
