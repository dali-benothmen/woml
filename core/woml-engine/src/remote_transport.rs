//! Packaged coordinator-to-worker dispatch transport.
//!
//! This adapter carries already-authorized attempts over the authenticated
//! worker session. It never schedules workflow nodes or settles runs: every
//! offer and worker fact is committed through `DurableEventStore` first.

use std::{
  collections::{BTreeMap, VecDeque},
  path::{Path, PathBuf},
  sync::{Arc, Mutex},
};

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

use crate::{
  decode_remote_dispatch_package, DurableEventStore, RemoteCoordinatorAuthorityV1,
  RemoteDispatchError, RemoteDispatchPackageV1, RemoteDispatchRequestV1, RemoteDispatchStateV1,
  WorkerDispatchFactV1, WorkerTerminalOutcomeV1, WorkerTerminalV1,
};

pub const REMOTE_OFFER_LIFETIME_SECONDS: i64 = 30;
pub const REMOTE_TRANSPORT_MAX_PENDING: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDispatchOfferV1 {
  pub profile: &'static str,
  pub offer_id: String,
  pub dispatch: RemoteDispatchPackageV1,
  pub coordinator_authority: RemoteCoordinatorAuthorityV1,
  pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDispatchTerminalReceiptV1 {
  pub profile: &'static str,
  pub dispatch_id: String,
  pub state: RemoteDispatchStateV1,
  pub terminal_digest: String,
  pub final_output_sequence: u64,
}

#[derive(Debug, Error)]
pub enum RemoteTransportError {
  #[error("the remote transport request is invalid")]
  Invalid,
  #[error("the remote transport queue is full")]
  Capacity,
  #[error("the remote transport is unavailable")]
  Unavailable,
  #[error(transparent)]
  Dispatch(#[from] RemoteDispatchError),
}

impl RemoteTransportError {
  pub const fn code(&self) -> &'static str {
    match self {
      Self::Invalid => "WOML_REMOTE_PROTOCOL_INVALID",
      Self::Capacity | Self::Unavailable => "WOML_TARGET_UNAVAILABLE",
      Self::Dispatch(error) => error.code(),
    }
  }
}

#[derive(Debug, Clone)]
struct PendingDispatchV1 {
  package: RemoteDispatchPackageV1,
  authority: RemoteCoordinatorAuthorityV1,
}

#[derive(Debug, Default)]
struct RemoteTransportStateV1 {
  pending: VecDeque<String>,
  packages: BTreeMap<String, PendingDispatchV1>,
  terminals: BTreeMap<String, WorkerTerminalV1>,
}

#[derive(Debug, Clone)]
pub struct RemoteDispatchTransportV1 {
  store_path: PathBuf,
  state: Arc<Mutex<RemoteTransportStateV1>>,
}

impl RemoteDispatchTransportV1 {
  pub fn new(store_path: impl Into<PathBuf>) -> Self {
    Self {
      store_path: store_path.into(),
      state: Arc::new(Mutex::new(RemoteTransportStateV1::default())),
    }
  }

  pub fn store_path(&self) -> &Path {
    &self.store_path
  }

  pub fn enqueue(
    &self,
    package: RemoteDispatchPackageV1,
    authority: RemoteCoordinatorAuthorityV1,
    now: DateTime<Utc>,
  ) -> Result<String, RemoteTransportError> {
    let encoded = serde_json::to_vec(&package).map_err(|_| RemoteTransportError::Invalid)?;
    let package =
      decode_remote_dispatch_package(&encoded).map_err(|_| RemoteTransportError::Invalid)?;
    if package.deadline <= now
      || authority.policy_digest.len() != 71
      || !authority.policy_digest.starts_with("sha256:")
    {
      return Err(RemoteTransportError::Invalid);
    }
    let request = RemoteDispatchRequestV1 {
      run_id: package.run_id.clone(),
      node_id: package.node_id.clone(),
      iteration_identity: package.iteration_key.clone().unwrap_or_default(),
      attempt: package.attempt,
      invocation_id: package.invocation_id.clone(),
      target: package.target.clone(),
      target_digest: package.target_registry_digest.clone(),
      package_digest: package.package_digest.clone(),
      requested_at: now,
    };
    let mut store =
      DurableEventStore::open(&self.store_path).map_err(|_| RemoteTransportError::Unavailable)?;
    let record = store.request_remote_dispatch(&request)?;
    let mut state = self
      .state
      .lock()
      .map_err(|_| RemoteTransportError::Unavailable)?;
    if state.packages.len() >= REMOTE_TRANSPORT_MAX_PENDING
      && !state.packages.contains_key(&record.dispatch_id)
    {
      return Err(RemoteTransportError::Capacity);
    }
    match state.packages.get(&record.dispatch_id) {
      Some(existing)
        if existing.package.package_digest != package.package_digest
          || existing.authority != authority =>
      {
        return Err(RemoteTransportError::Invalid)
      }
      Some(_) => return Ok(record.dispatch_id),
      None => {}
    }
    if record.state == RemoteDispatchStateV1::Requested {
      state.pending.push_back(record.dispatch_id.clone());
    }
    state.packages.insert(
      record.dispatch_id.clone(),
      PendingDispatchV1 { package, authority },
    );
    Ok(record.dispatch_id)
  }

  pub fn offer_next(
    &self,
    worker_id: &str,
    session_id: &str,
    target: &str,
    now: DateTime<Utc>,
  ) -> Result<Option<RemoteDispatchOfferV1>, RemoteTransportError> {
    let (dispatch_id, selected) = {
      let mut state = self
        .state
        .lock()
        .map_err(|_| RemoteTransportError::Unavailable)?;
      let Some(index) = state.pending.iter().position(|dispatch_id| {
        state
          .packages
          .get(dispatch_id)
          .is_some_and(|pending| pending.package.target == target && pending.package.deadline > now)
      }) else {
        return Ok(None);
      };
      let dispatch_id = state
        .pending
        .remove(index)
        .ok_or(RemoteTransportError::Unavailable)?;
      let selected = state
        .packages
        .get(&dispatch_id)
        .cloned()
        .ok_or(RemoteTransportError::Unavailable)?;
      (dispatch_id, selected)
    };
    let offer_id = format!("offer_{}", Uuid::new_v4().simple());
    let expires_at = now + Duration::seconds(REMOTE_OFFER_LIFETIME_SECONDS);
    let mut store =
      DurableEventStore::open(&self.store_path).map_err(|_| RemoteTransportError::Unavailable)?;
    if let Err(error) = store.offer_remote_dispatch(
      &selected.package.dispatch_id,
      worker_id,
      session_id,
      &offer_id,
      expires_at,
      now,
    ) {
      let mut state = self
        .state
        .lock()
        .map_err(|_| RemoteTransportError::Unavailable)?;
      if state.packages.contains_key(&dispatch_id) && !state.pending.contains(&dispatch_id) {
        state.pending.push_front(dispatch_id);
      }
      return Err(error.into());
    }
    Ok(Some(RemoteDispatchOfferV1 {
      profile: "woml.remote-dispatch-offer/v1",
      offer_id,
      dispatch: selected.package,
      coordinator_authority: selected.authority,
      expires_at,
    }))
  }

  pub fn accept(&self, fact: &WorkerDispatchFactV1) -> Result<(), RemoteTransportError> {
    let mut store =
      DurableEventStore::open(&self.store_path).map_err(|_| RemoteTransportError::Unavailable)?;
    let record = store.remote_dispatch_record(&fact.dispatch_id)?;
    let offer_id = record.offer_id.ok_or(RemoteTransportError::Invalid)?;
    store.accept_remote_dispatch(
      &fact.dispatch_id,
      &fact.worker_id,
      &fact.session_id,
      &offer_id,
      fact.sent_at,
    )?;
    Ok(())
  }

  pub fn start(&self, fact: &WorkerDispatchFactV1) -> Result<(), RemoteTransportError> {
    let mut store =
      DurableEventStore::open(&self.store_path).map_err(|_| RemoteTransportError::Unavailable)?;
    store.start_remote_dispatch(
      &fact.dispatch_id,
      &fact.worker_id,
      &fact.session_id,
      fact.sent_at,
    )?;
    Ok(())
  }

  pub fn terminal(
    &self,
    fact: WorkerTerminalV1,
  ) -> Result<RemoteDispatchTerminalReceiptV1, RemoteTransportError> {
    let outcome = match fact.outcome {
      WorkerTerminalOutcomeV1::Succeeded => RemoteDispatchStateV1::Succeeded,
      WorkerTerminalOutcomeV1::Failed => RemoteDispatchStateV1::Failed,
      WorkerTerminalOutcomeV1::Cancelled => RemoteDispatchStateV1::Cancelled,
    };
    let mut store =
      DurableEventStore::open(&self.store_path).map_err(|_| RemoteTransportError::Unavailable)?;
    let record = store.terminal_remote_dispatch(
      &fact.dispatch_id,
      &fact.worker_id,
      &fact.session_id,
      outcome,
      &fact.journal_digest,
      fact.sent_at,
    )?;
    let mut state = self
      .state
      .lock()
      .map_err(|_| RemoteTransportError::Unavailable)?;
    match state.terminals.get(&fact.dispatch_id) {
      Some(existing) if existing != &fact => return Err(RemoteTransportError::Invalid),
      Some(_) => {}
      None => {
        state
          .terminals
          .insert(fact.dispatch_id.clone(), fact.clone());
      }
    }
    state.packages.remove(&fact.dispatch_id);
    Ok(RemoteDispatchTerminalReceiptV1 {
      profile: "woml.remote-dispatch-terminal-receipt/v1",
      dispatch_id: fact.dispatch_id,
      state: record.state,
      terminal_digest: fact.journal_digest,
      final_output_sequence: fact.final_output_sequence,
    })
  }

  pub fn terminal_fact(
    &self,
    dispatch_id: &str,
  ) -> Result<Option<WorkerTerminalV1>, RemoteTransportError> {
    Ok(
      self
        .state
        .lock()
        .map_err(|_| RemoteTransportError::Unavailable)?
        .terminals
        .get(dispatch_id)
        .cloned(),
    )
  }

  pub fn mark_lost(
    &self,
    due_before: DateTime<Utc>,
    now: DateTime<Utc>,
  ) -> Result<Vec<String>, RemoteTransportError> {
    let mut store =
      DurableEventStore::open(&self.store_path).map_err(|_| RemoteTransportError::Unavailable)?;
    let records = store.mark_lost_remote_dispatches(
      due_before,
      now,
      crate::REMOTE_DISPATCH_RECOVERY_BATCH_LIMIT,
    )?;
    let mut state = self
      .state
      .lock()
      .map_err(|_| RemoteTransportError::Unavailable)?;
    for record in &records {
      if record.state == RemoteDispatchStateV1::Requested
        && state.packages.contains_key(&record.dispatch_id)
        && !state.pending.contains(&record.dispatch_id)
      {
        state.pending.push_back(record.dispatch_id.clone());
      }
    }
    Ok(
      records
        .into_iter()
        .map(|record| record.dispatch_id)
        .collect(),
    )
  }

  pub fn offer_json(offer: RemoteDispatchOfferV1) -> Result<Value, RemoteTransportError> {
    serde_json::to_value(offer).map_err(|_| RemoteTransportError::Invalid)
  }
}
