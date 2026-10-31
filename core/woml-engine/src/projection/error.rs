//! projection error.

use crate::event::EventValidationError;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum FoldError {
  #[error(transparent)]
  InvalidEvent(#[from] EventValidationError),
  #[error("{0}")]
  InvalidHistory(String),
}
