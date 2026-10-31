//! runtime cleanup.

use super::RuntimeExecutionOptions;
use std::sync::Arc;

pub(super) struct ArtifactInvocationCleanup {
  store: Option<Arc<crate::ManagedArtifactStore>>,
  invocation_id: String,
}

impl ArtifactInvocationCleanup {
  pub(super) fn new(options: &RuntimeExecutionOptions, invocation_id: &str) -> Self {
    Self {
      store: options.managed_artifact_store.clone(),
      invocation_id: invocation_id.to_string(),
    }
  }

  pub(super) fn from_store(
    store: Option<Arc<crate::ManagedArtifactStore>>,
    invocation_id: &str,
  ) -> Self {
    Self {
      store,
      invocation_id: invocation_id.to_string(),
    }
  }
}

impl Drop for ArtifactInvocationCleanup {
  fn drop(&mut self) {
    if let Some(store) = &self.store {
      store.cleanup_invocation(&self.invocation_id);
    }
  }
}
