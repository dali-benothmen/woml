//! Authenticated loopback coordinator server and listener lifecycle.
//!
//! Keep shutdown, listener ownership and connection capacity here. Private
//! connection/HTTP/TLS modules carry facts to the existing dispatch authority.

mod connection;
mod contracts;
mod http;
mod tls;

pub(in crate::remote_coordinator_server) use connection::handle_connection;
pub use contracts::{
  RemoteCoordinatorServerError, RemoteCoordinatorTlsOptionsV1,
  REMOTE_COORDINATOR_IO_TIMEOUT_SECONDS, REMOTE_COORDINATOR_MAX_CONNECTIONS,
  REMOTE_COORDINATOR_MAX_CONNECTIONS_PER_SECOND, REMOTE_COORDINATOR_MAX_HEADER_BYTES,
};
pub(in crate::remote_coordinator_server) use http::{
  connection_io_error, read_request, write_error, write_response,
};
pub(in crate::remote_coordinator_server) use tls::server_configuration;

use crate::RemoteDispatchTransportV1;
use crate::RemoteSessionAuthorityV1;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::net::TcpStream;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;

pub struct RemoteCoordinatorServerV1 {
  local_address: SocketAddr,
  shutdown: Arc<AtomicBool>,
  thread: Option<JoinHandle<()>>,
}

impl RemoteCoordinatorServerV1 {
  pub fn start(
    authority: RemoteSessionAuthorityV1,
    options: RemoteCoordinatorTlsOptionsV1,
  ) -> Result<Self, RemoteCoordinatorServerError> {
    Self::start_inner(authority, None, options)
  }

  pub fn start_dispatching(
    authority: RemoteSessionAuthorityV1,
    transport: RemoteDispatchTransportV1,
    options: RemoteCoordinatorTlsOptionsV1,
  ) -> Result<Self, RemoteCoordinatorServerError> {
    Self::start_inner(authority, Some(transport), options)
  }

  fn start_inner(
    authority: RemoteSessionAuthorityV1,
    transport: Option<RemoteDispatchTransportV1>,
    options: RemoteCoordinatorTlsOptionsV1,
  ) -> Result<Self, RemoteCoordinatorServerError> {
    let tls = Arc::new(server_configuration(&options)?);
    let listener = TcpListener::bind(options.bind_address)
      .map_err(|_| RemoteCoordinatorServerError::Unavailable)?;
    let local_address = listener
      .local_addr()
      .map_err(|_| RemoteCoordinatorServerError::Unavailable)?;
    listener
      .set_nonblocking(true)
      .map_err(|_| RemoteCoordinatorServerError::Unavailable)?;
    let shutdown = Arc::new(AtomicBool::new(false));
    let server_shutdown = Arc::clone(&shutdown);
    let authority = Arc::new(Mutex::new(authority));
    let transport = transport.map(Arc::new);
    let active = Arc::new(AtomicUsize::new(0));
    let connection_rate = Arc::new(Mutex::new(VecDeque::new()));
    let thread = thread::Builder::new()
      .name("woml-remote-coordinator".to_string())
      .spawn(move || {
        while !server_shutdown.load(Ordering::Acquire) {
          match listener.accept() {
            Ok((stream, _)) => {
              if active.fetch_add(1, Ordering::AcqRel) >= REMOTE_COORDINATOR_MAX_CONNECTIONS {
                active.fetch_sub(1, Ordering::AcqRel);
                continue;
              }
              let admitted = connection_rate
                .lock()
                .map(|mut arrivals| {
                  let now = Instant::now();
                  while arrivals
                    .front()
                    .is_some_and(|arrival| now.duration_since(*arrival) >= Duration::from_secs(1))
                  {
                    arrivals.pop_front();
                  }
                  if arrivals.len() >= REMOTE_COORDINATOR_MAX_CONNECTIONS_PER_SECOND {
                    false
                  } else {
                    arrivals.push_back(now);
                    true
                  }
                })
                .unwrap_or(false);
              if !admitted {
                active.fetch_sub(1, Ordering::AcqRel);
                continue;
              }
              let tls = Arc::clone(&tls);
              let authority = Arc::clone(&authority);
              let transport = transport.clone();
              let active = Arc::clone(&active);
              let _ = thread::Builder::new()
                .name("woml-remote-session".to_string())
                .spawn(move || {
                  let _guard = ActiveConnection(active);
                  if let Err(stage) = handle_connection(stream, tls, authority, transport) {
                    #[cfg(debug_assertions)]
                    eprintln!("WOML remote coordinator connection failed at {stage}");
                    #[cfg(not(debug_assertions))]
                    let _ = stage;
                  }
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
              thread::sleep(Duration::from_millis(10));
            }
            Err(_) => break,
          }
        }
      })
      .map_err(|_| RemoteCoordinatorServerError::Unavailable)?;
    Ok(Self {
      local_address,
      shutdown,
      thread: Some(thread),
    })
  }

  pub const fn local_address(&self) -> SocketAddr {
    self.local_address
  }

  pub fn shutdown(mut self) {
    self.shutdown.store(true, Ordering::Release);
    let _ = TcpStream::connect_timeout(&self.local_address, Duration::from_millis(100));
    if let Some(thread) = self.thread.take() {
      let _ = thread.join();
    }
  }
}

impl Drop for RemoteCoordinatorServerV1 {
  fn drop(&mut self) {
    self.shutdown.store(true, Ordering::Release);
    let _ = TcpStream::connect_timeout(&self.local_address, Duration::from_millis(100));
    if let Some(thread) = self.thread.take() {
      let _ = thread.join();
    }
  }
}

struct ActiveConnection(Arc<AtomicUsize>);

impl Drop for ActiveConnection {
  fn drop(&mut self) {
    self.0.fetch_sub(1, Ordering::AcqRel);
  }
}
