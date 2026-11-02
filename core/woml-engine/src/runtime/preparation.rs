//! Private selection of owned request opening; invalid preparation never falls
//! back to initialization. Options do not carry mutable projections or decisions.

use super::{RuntimeExecutionError, RuntimeExecutionOptions};
use crate::durable::{DurableEventStore, PreparedScope};
use chrono::{DateTime, Utc};
use std::path::Path;

impl RuntimeExecutionOptions {
  pub(crate) fn prepare_owned_state_binding(
    &self,
    path: &Path,
  ) -> Result<(), RuntimeExecutionError> {
    // StateService's existing first binding validates durable state and opens
    // the engine store. Do it once behind startup ownership, before publishing
    // readiness, rather than running global initialization in the first request.
    if let Some(state) = &self.managed_durable_state_store {
      state
        .configure_for_state(path)
        .map_err(|failure| RuntimeExecutionError::InvalidConfiguration(failure.message))?;
    }
    Ok(())
  }
  pub(super) fn require_unprepared_entrypoint(&self) -> Result<(), RuntimeExecutionError> {
    if self.prepared_runtime.is_some() {
      return Err(RuntimeExecutionError::InvalidConfiguration(
        "Owned runtime options require the admitted execution path, not one-shot recovery.".into(),
      ));
    }
    Ok(())
  }
  pub(crate) fn open_ingress_store(
    &self,
    path: &Path,
    scope: PreparedScope,
  ) -> Result<DurableEventStore, crate::DurableStoreError> {
    match &self.prepared_runtime {
      Some(context) => context.open(path, scope, self.prepared_run.as_deref()),
      None => DurableEventStore::open_ready(path),
    }
  }
  pub(crate) fn open_runtime_store(
    &self,
    path: &Path,
    scope: PreparedScope,
  ) -> Result<DurableEventStore, crate::DurableStoreError> {
    match &self.prepared_runtime {
      Some(context) => context.open(path, scope, self.prepared_run.as_deref()),
      None => DurableEventStore::open(path),
    }
  }

  pub(crate) fn for_admitted_run(
    mut self,
    path: &Path,
    run_id: &str,
  ) -> Result<Self, RuntimeExecutionError> {
    if self.prepared_run.as_deref() != Some(run_id) {
      if let Some(context) = &self.prepared_runtime {
        context.bind_run(path, run_id)?;
      }
      self.prepared_run = Some(run_id.to_string());
    }
    Ok(self)
  }

  /// Rust-only owner maintenance, available only on options returned by the
  /// prepared trigger server. No caller-selected bypass or serialized token.
  pub fn renew_owned_runtime_lease(
    &self,
    path: &Path,
    now: DateTime<Utc>,
    expires: DateTime<Utc>,
  ) -> Result<bool, crate::DurableStoreError> {
    self
      .prepared_runtime
      .as_ref()
      .ok_or_else(|| {
        crate::DurableStoreError::Contract(
          "WOML_RUNTIME_PREPARATION_INVALID: no owned lifetime".into(),
        )
      })?
      .renew(path, now, expires)
  }
}
