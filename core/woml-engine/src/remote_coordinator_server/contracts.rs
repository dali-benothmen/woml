//! Private remote coordinator server contracts.

use std::net::SocketAddr;
use std::path::PathBuf;
use thiserror::Error;

pub const REMOTE_COORDINATOR_MAX_CONNECTIONS: usize = 64;

pub const REMOTE_COORDINATOR_MAX_CONNECTIONS_PER_SECOND: usize = 256;

pub const REMOTE_COORDINATOR_MAX_HEADER_BYTES: usize = 16 * 1024;

pub const REMOTE_COORDINATOR_IO_TIMEOUT_SECONDS: u64 = 5;

#[derive(Debug, Clone)]
pub struct RemoteCoordinatorTlsOptionsV1 {
  pub bind_address: SocketAddr,
  pub certificate_path: PathBuf,
  pub private_key_path: PathBuf,
  pub client_ca_path: PathBuf,
}

#[derive(Debug, Error)]
pub enum RemoteCoordinatorServerError {
  #[error("the remote coordinator TLS configuration is invalid")]
  TlsConfiguration,
  #[error("the remote coordinator listener is unavailable")]
  Unavailable,
}
