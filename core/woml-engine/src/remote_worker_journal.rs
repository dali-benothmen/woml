//! Worker-local attempt evidence and execution facade.
//!
//! Preparation intersects authority; persistence journals acceptance/start/terminal
//! facts; execution owns the active slot and supervised host. None schedules a DAG.

mod artifacts;
mod contracts;
mod execution;
mod identity;
mod persistence;
mod preparation;
mod schema;
mod secrets;

pub use artifacts::verify_remote_artifact_transfer;
pub(in crate::remote_worker_journal) use artifacts::{
  artifact_grant_digest, validate_artifact_grants,
};
pub use contracts::{
  PreparedRemoteDispatchV1, RemoteWorkerAuthorityV1, RemoteWorkerCancellationTokenV1,
  RemoteWorkerError, RemoteWorkerTerminalV1, WorkerAttemptRecordV1, WorkerAttemptStateV1,
  REMOTE_CANCELLATION_CONFIRMATION_TIMEOUT_MS, WORKER_JOURNAL_RECOVERY_LIMIT,
};
pub use execution::RemoteWorkerAttemptRuntimeV1;
pub(in crate::remote_worker_journal) use identity::sha256;
pub use persistence::WorkerJournalV1;
pub use preparation::prepare_remote_dispatch;
pub(in crate::remote_worker_journal) use preparation::{
  maximum_attempts, script_source, valid_bound_id,
};
pub(in crate::remote_worker_journal) use schema::CREATE_SCHEMA;
pub use schema::WORKER_JOURNAL_SCHEMA_VERSION;
pub(in crate::remote_worker_journal) use secrets::{redact_json_value, valid_secret_name};
pub use secrets::{
  EnvironmentRemoteSecretProvider, RemoteSecretProvider, ResolvedRemoteSecretProvider,
};
