//! Coordinator-owned RET7 streaming and loss boundary.
//!
//! Remote output is authenticated against the durable dispatch binding, then
//! persisted only in Operational Output Store v1. The worker sequence is
//! committed in the same output-store transaction as the record, which makes
//! duplicate delivery safe across coordinator restarts.

use chrono::{DateTime, Utc};
use serde_json::json;
use thiserror::Error;

use crate::{
  DurableEventStore, OperationalBodyV1, OperationalIdentityV1, OperationalOutputError,
  OperationalOutputStore, OperationalRecordDraftV1, RemoteDispatchError,
  RemoteOperationalAppendOutcomeV1, WorkerOutputBatchV1,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteOutputIngestReportV1 {
  pub dispatch_id: String,
  pub accepted: usize,
  pub duplicates: usize,
  pub last_remote_sequence: u64,
  pub last_local_sequence: u64,
}

#[derive(Debug, Error)]
pub enum RemoteExecutionError {
  #[error(transparent)]
  Dispatch(#[from] RemoteDispatchError),
  #[error(transparent)]
  Output(#[from] OperationalOutputError),
  #[error("the remote output body is not legal on the worker stream")]
  InvalidOutputBody,
}

impl RemoteExecutionError {
  pub fn code(&self) -> &'static str {
    match self {
      Self::Dispatch(error) => error.code(),
      Self::Output(error) => error.code(),
      Self::InvalidOutputBody => "WOML_REMOTE_PROTOCOL_INVALID",
    }
  }
}

pub fn ingest_remote_output_batch(
  store: &mut DurableEventStore,
  output: &OperationalOutputStore,
  batch: &WorkerOutputBatchV1,
  now: DateTime<Utc>,
) -> Result<RemoteOutputIngestReportV1, RemoteExecutionError> {
  let binding =
    store.authorize_remote_output(&batch.dispatch_id, &batch.worker_id, &batch.session_id)?;
  let mut accepted = 0usize;
  let mut duplicates = 0usize;
  let mut last_remote_sequence = 0u64;
  let mut last_local_sequence = 0u64;
  for remote in &batch.records {
    if matches!(
      remote.body,
      OperationalBodyV1::Closed { .. } | OperationalBodyV1::Gap { .. }
    ) {
      return Err(RemoteExecutionError::InvalidOutputBody);
    }
    let outcome = output.append_remote_bounded(
      &batch.dispatch_id,
      remote.sequence,
      &remote.record_digest,
      OperationalRecordDraftV1 {
        identity: OperationalIdentityV1 {
          workflow_id: binding.workflow_id.clone(),
          run_id: binding.run_id.clone(),
          node_id: Some(binding.node_id.clone()),
          attempt: Some(binding.attempt),
          invocation_id: Some(binding.invocation_id.clone()),
          phase: binding.phase,
        },
        body: remote.body.clone(),
      },
    )?;
    match outcome {
      RemoteOperationalAppendOutcomeV1::Appended { local_sequence } => {
        accepted += 1;
        last_local_sequence = local_sequence;
      }
      RemoteOperationalAppendOutcomeV1::Duplicate { local_sequence } => {
        duplicates += 1;
        last_local_sequence = local_sequence;
      }
    }
    last_remote_sequence = remote.sequence;
  }
  store.heartbeat_remote_dispatch(
    &batch.dispatch_id,
    &batch.worker_id,
    &batch.session_id,
    last_remote_sequence,
    &json!({
      "outputSequence": last_remote_sequence,
      "records": batch.records.len(),
    }),
    now,
  )?;
  Ok(RemoteOutputIngestReportV1 {
    dispatch_id: batch.dispatch_id.clone(),
    accepted,
    duplicates,
    last_remote_sequence,
    last_local_sequence,
  })
}
