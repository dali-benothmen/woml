//! Managed artifact binding and logical-store opening/connection ownership. One selected durable database retains metadata authority.

use crate::artifacts::artifact_access_denied;
use crate::artifacts::artifact_failure;
use crate::artifacts::artifact_not_found;
use crate::artifacts::backend_unavailable;
use crate::artifacts::cleanup_directory_contents;
use crate::artifacts::ensure_private_directory;
use crate::artifacts::ensure_store_identity;
use crate::artifacts::harden_file_permissions;
use crate::artifacts::initialize_schema;
use crate::artifacts::materialization_unavailable;
use crate::artifacts::validate_schema;
use crate::artifacts::ArtifactBackend;
use crate::artifacts::ArtifactReferenceV1;
use crate::artifacts::LocalArtifactBackend;
use crate::artifacts::PreparedArtifactBackend;
use crate::artifacts::PreparedArtifactPolicy;
use crate::artifacts::S3ArtifactBackend;
use crate::artifacts::ARTIFACT_REFERENCE_CONTRACT;
use crate::artifacts::ARTIFACT_REFERENCE_CONTRACT_VERSION;
use crate::CapabilityFailure;
use crate::CapabilityFailureKind;
use crate::DurableEventStore;
use rusqlite::Connection;
use rusqlite::OpenFlags;
use rusqlite::OptionalExtension;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

const ARTIFACT_BUSY_TIMEOUT_MS: u64 = 5_000;

#[derive(Default)]
pub struct ManagedArtifactStore {
  pub(in crate::artifacts) store: Mutex<Option<Arc<ArtifactStore>>>,
}

impl std::fmt::Debug for ManagedArtifactStore {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    formatter
      .debug_struct("ManagedArtifactStore")
      .field(
        "configured",
        &self.store.lock().map(|v| v.is_some()).unwrap_or(false),
      )
      .finish()
  }
}

impl ManagedArtifactStore {
  pub(crate) fn configure_for_state(
    &self,
    state_path: &Path,
    policy: Arc<PreparedArtifactPolicy>,
  ) -> Result<(), CapabilityFailure> {
    let store = Arc::new(ArtifactStore::open(state_path, policy)?);
    let mut configured = self.store.lock().map_err(|_| backend_unavailable())?;
    if let Some(existing) = configured.as_ref() {
      if existing.state_path == store.state_path
        && existing.backend.identity() == store.backend.identity()
      {
        return Ok(());
      }
      return Err(artifact_failure(
        CapabilityFailureKind::ServiceRejected,
        "WOML_ARTIFACT_BACKEND_UNAVAILABLE",
        "The artifact backend is already bound to another durable state store.",
        false,
        false,
      ));
    }
    *configured = Some(store);
    Ok(())
  }

  pub(in crate::artifacts) fn configured(&self) -> Result<Arc<ArtifactStore>, CapabilityFailure> {
    self
      .store
      .lock()
      .map_err(|_| backend_unavailable())?
      .clone()
      .ok_or_else(backend_unavailable)
  }

  pub(crate) fn cleanup_invocation(&self, invocation_id: &str) {
    let Ok(store) = self.configured() else {
      return;
    };
    let _ = store.cleanup_materialization(invocation_id);
  }

  pub(crate) fn operational_redactions(&self, invocation_id: &str) -> Vec<String> {
    let Ok(store) = self.configured() else {
      return Vec::new();
    };
    vec![store
      .invocation_materialization_directory(invocation_id)
      .to_string_lossy()
      .into_owned()]
  }

  /// Verifies an artifact reference for an immutable rich-approval review.
  /// This intentionally exposes metadata only; artifact bytes never enter the
  /// review event or provider payload.
  pub fn verify_review_reference(
    &self,
    run_id: &str,
    supplied: &ArtifactReferenceV1,
  ) -> Result<ArtifactReferenceV1, CapabilityFailure> {
    let store = self.configured()?;
    let artifact = store.authorize_reference(run_id, supplied)?;
    store
      .backend
      .verify(&artifact.reference.sha256, artifact.reference.size)?;
    Ok(artifact.reference)
  }

  /// Revalidates an artifact pinned by the durable rich-approval authority.
  /// The caller supplies only the immutable ID and digest stored in the pin;
  /// every remaining field, run grant, retention state, and backend byte
  /// digest is recovered and checked by the trusted artifact store.
  pub(crate) fn verify_policy_evidence(
    &self,
    run_id: &str,
    artifact_id: &str,
    expected_sha256: &str,
  ) -> Result<ArtifactReferenceV1, CapabilityFailure> {
    let store = self.configured()?;
    let connection = store.connection()?;
    validate_schema(&connection)?;
    let supplied = connection
      .query_row(
        "SELECT artifact_id, name, size, sha256, content_type
         FROM woml_artifacts WHERE artifact_id = ?1",
        [artifact_id],
        |row| {
          Ok(ArtifactReferenceV1 {
            contract: ARTIFACT_REFERENCE_CONTRACT.to_string(),
            contract_version: ARTIFACT_REFERENCE_CONTRACT_VERSION,
            id: row.get(0)?,
            name: row.get(1)?,
            size: row.get(2)?,
            sha256: row.get(3)?,
            content_type: row.get(4)?,
          })
        },
      )
      .optional()
      .map_err(|_| backend_unavailable())?
      .ok_or_else(artifact_not_found)?;
    if supplied.sha256 != expected_sha256 {
      return Err(artifact_access_denied());
    }
    let artifact = store.authorize_reference(run_id, &supplied)?;
    store
      .backend
      .verify(&artifact.reference.sha256, artifact.reference.size)?;
    Ok(artifact.reference)
  }
}

pub(in crate::artifacts) struct ArtifactStore {
  pub(in crate::artifacts) state_path: PathBuf,
  pub(in crate::artifacts) policy: Arc<PreparedArtifactPolicy>,
  pub(in crate::artifacts) backend: Arc<dyn ArtifactBackend>,
  pub(in crate::artifacts) materializations: PathBuf,
}

impl std::fmt::Debug for ArtifactStore {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    formatter
      .debug_struct("ArtifactStore")
      .field("state_path", &self.state_path)
      .field("backend", &self.backend.identity())
      .finish()
  }
}

impl ArtifactStore {
  pub(in crate::artifacts) fn open(
    state_path: &Path,
    policy: Arc<PreparedArtifactPolicy>,
  ) -> Result<Self, CapabilityFailure> {
    Self::open_with_startup_cleanup(state_path, policy, true)
  }

  pub(in crate::artifacts) fn open_with_startup_cleanup(
    state_path: &Path,
    policy: Arc<PreparedArtifactPolicy>,
    startup_cleanup: bool,
  ) -> Result<Self, CapabilityFailure> {
    DurableEventStore::open(state_path).map_err(|_| backend_unavailable())?;
    let state_path = fs::canonicalize(state_path).map_err(|_| backend_unavailable())?;
    harden_file_permissions(&state_path)?;
    ensure_private_directory(&policy.materialization_directory)?;
    let materializations = fs::canonicalize(&policy.materialization_directory)
      .map_err(|_| materialization_unavailable())?;
    if startup_cleanup {
      cleanup_directory_contents(&materializations)?;
    }
    let backend: Arc<dyn ArtifactBackend> = match &policy.backend {
      PreparedArtifactBackend::Local { directory } => {
        Arc::new(LocalArtifactBackend::open(directory)?)
      }
      PreparedArtifactBackend::S3(config) => Arc::new(S3ArtifactBackend::open(
        config.as_ref().clone(),
        &materializations,
      )?),
    };
    let store = Self {
      state_path,
      policy,
      backend,
      materializations,
    };
    let connection = store.connection()?;
    initialize_schema(&connection)?;
    validate_schema(&connection)?;
    let store_id = ensure_store_identity(&connection)?;
    store.backend.bind_store(&connection, &store_id)?;
    if startup_cleanup {
      store.backend.cleanup_startup(&connection)?;
    }
    Ok(store)
  }

  pub(in crate::artifacts) fn connection(&self) -> Result<Connection, CapabilityFailure> {
    let connection = Connection::open_with_flags(
      &self.state_path,
      OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|_| backend_unavailable())?;
    connection
      .busy_timeout(Duration::from_millis(ARTIFACT_BUSY_TIMEOUT_MS))
      .map_err(|_| backend_unavailable())?;
    connection
      .execute_batch("PRAGMA foreign_keys = ON;")
      .map_err(|_| backend_unavailable())?;
    Ok(connection)
  }
}
