//! event errors.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EventValidationError {
  #[error("unsupported run event schema version {0}")]
  UnsupportedSchemaVersion(u32),
  #[error("{0}")]
  Invalid(String),
}
