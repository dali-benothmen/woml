//! Minimal bounded mTLS HTTP adapter for the RET3 session authority.
//!
//! The listener exposes session establishment and idle long-polling only. It
//! intentionally has no route capable of creating or executing a dispatch.

use std::{
  collections::VecDeque,
  fs::File,
  io::{BufReader, Read, Write},
  net::{SocketAddr, TcpListener, TcpStream},
  path::PathBuf,
  sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
  },
  thread::{self, JoinHandle},
  time::{Duration, Instant},
};

use chrono::Utc;
use rustls::{
  pki_types::{CertificateDer, PrivateKeyDer},
  server::WebPkiClientVerifier,
  RootCertStore, ServerConfig, ServerConnection, StreamOwned,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
  decode_worker_hello, decode_worker_poll, decode_worker_shutdown, RemoteCoordinatorError,
  RemoteSessionAuthorityV1, WORKER_PROTOCOL_MAX_MESSAGE_BYTES,
};

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
              let active = Arc::clone(&active);
              let _ = thread::Builder::new()
                .name("woml-remote-session".to_string())
                .spawn(move || {
                  let _guard = ActiveConnection(active);
                  let _ = handle_connection(stream, tls, authority);
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

fn handle_connection(
  stream: TcpStream,
  tls: Arc<ServerConfig>,
  authority: Arc<Mutex<RemoteSessionAuthorityV1>>,
) -> Result<(), ()> {
  stream
    .set_read_timeout(Some(Duration::from_secs(
      REMOTE_COORDINATOR_IO_TIMEOUT_SECONDS,
    )))
    .map_err(|_| ())?;
  stream
    .set_write_timeout(Some(Duration::from_secs(
      REMOTE_COORDINATOR_IO_TIMEOUT_SECONDS,
    )))
    .map_err(|_| ())?;
  let connection = ServerConnection::new(tls).map_err(|_| ())?;
  let mut stream = StreamOwned::new(connection, stream);
  let request = match read_request(&mut stream) {
    Ok(request) => request,
    Err(status) => {
      let _ = write_error(&mut stream, status, "WOML_REMOTE_PROTOCOL_INVALID");
      return Ok(());
    }
  };
  let certificate_digest = stream
    .conn
    .peer_certificates()
    .and_then(|certificates| certificates.first())
    .map(|certificate| format!("sha256:{:x}", Sha256::digest(certificate.as_ref())))
    .ok_or(())?;
  let now = Utc::now();
  let result = if request.method == "POST" && request.path == "/woml/v1/workers/session" {
    decode_worker_hello(&request.body)
      .map_err(|_| RemoteCoordinatorError::ReplayOrConflict)
      .and_then(|hello| {
        authority
          .lock()
          .map_err(|_| RemoteCoordinatorError::Unavailable)?
          .establish(hello, &certificate_digest, now)
      })
      .and_then(|response| json_bytes(&response))
  } else if request.method == "POST" && request.path == "/woml/v1/workers/poll" {
    decode_worker_poll(&request.body)
      .map_err(|_| RemoteCoordinatorError::ReplayOrConflict)
      .and_then(|poll| {
        authority
          .lock()
          .map_err(|_| RemoteCoordinatorError::Unavailable)?
          .poll(poll, &certificate_digest, now)
      })
      .and_then(|response| json_bytes(&response))
  } else if request.method == "POST" && request.path == "/woml/v1/workers/shutdown" {
    decode_worker_shutdown(&request.body)
      .map_err(|_| RemoteCoordinatorError::ReplayOrConflict)
      .and_then(|message| {
        authority
          .lock()
          .map_err(|_| RemoteCoordinatorError::Unavailable)?
          .shutdown(message.clone(), &certificate_digest, now)?;
        json_bytes(&serde_json::json!({
          "profile": "woml.worker-protocol/v1",
          "sessionId": message.session_id,
          "status": "closed"
        }))
      })
  } else {
    write_error(&mut stream, 404, "WOML_TARGET_UNAVAILABLE").map_err(|_| ())?;
    return Ok(());
  };
  match result {
    Ok(body) => write_response(&mut stream, 200, &body).map_err(|_| ())?,
    Err(error) => {
      let status = match error {
        RemoteCoordinatorError::Unauthorized => 403,
        RemoteCoordinatorError::Incompatible | RemoteCoordinatorError::ReplayOrConflict => 400,
        RemoteCoordinatorError::Unavailable | RemoteCoordinatorError::Capacity => 503,
      };
      write_error(&mut stream, status, error.code()).map_err(|_| ())?;
    }
  }
  Ok(())
}

struct HttpRequest {
  method: String,
  path: String,
  body: Vec<u8>,
}

fn read_request(stream: &mut StreamOwned<ServerConnection, TcpStream>) -> Result<HttpRequest, u16> {
  let mut bytes = Vec::new();
  let mut buffer = [0u8; 4096];
  let header_end = loop {
    let read = stream.read(&mut buffer).map_err(|_| 408u16)?;
    if read == 0 {
      return Err(400);
    }
    bytes.extend_from_slice(&buffer[..read]);
    if bytes.len() > REMOTE_COORDINATOR_MAX_HEADER_BYTES + WORKER_PROTOCOL_MAX_MESSAGE_BYTES {
      return Err(413);
    }
    if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
      break position + 4;
    }
    if bytes.len() > REMOTE_COORDINATOR_MAX_HEADER_BYTES {
      return Err(431);
    }
  };
  let header = std::str::from_utf8(&bytes[..header_end]).map_err(|_| 400u16)?;
  let mut lines = header.split("\r\n");
  let request_line = lines.next().ok_or(400u16)?;
  let mut parts = request_line.split(' ');
  let method = parts.next().ok_or(400u16)?.to_string();
  let path = parts.next().ok_or(400u16)?.to_string();
  if parts.next() != Some("HTTP/1.1") || parts.next().is_some() {
    return Err(400);
  }
  let mut content_length = None;
  let mut content_type = None;
  for line in lines.filter(|line| !line.is_empty()) {
    let (name, value) = line.split_once(':').ok_or(400u16)?;
    if name.eq_ignore_ascii_case("content-length") {
      if content_length.is_some() {
        return Err(400);
      }
      content_length = Some(value.trim().parse::<usize>().map_err(|_| 400u16)?);
    } else if name.eq_ignore_ascii_case("content-type") {
      if content_type.is_some() {
        return Err(400);
      }
      content_type = Some(value.trim());
    } else if name.eq_ignore_ascii_case("transfer-encoding") {
      return Err(400);
    }
  }
  if content_type != Some("application/json") {
    return Err(415);
  }
  let content_length = content_length.ok_or(411u16)?;
  if content_length > WORKER_PROTOCOL_MAX_MESSAGE_BYTES {
    return Err(413);
  }
  while bytes.len() - header_end < content_length {
    let read = stream.read(&mut buffer).map_err(|_| 408u16)?;
    if read == 0 {
      return Err(400);
    }
    bytes.extend_from_slice(&buffer[..read]);
    if bytes.len() - header_end > content_length {
      return Err(400);
    }
  }
  Ok(HttpRequest {
    method,
    path,
    body: bytes[header_end..].to_vec(),
  })
}

fn json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, RemoteCoordinatorError> {
  serde_json::to_vec(value).map_err(|_| RemoteCoordinatorError::Unavailable)
}

fn write_error(
  stream: &mut StreamOwned<ServerConnection, TcpStream>,
  status: u16,
  code: &str,
) -> std::io::Result<()> {
  let body = serde_json::to_vec(&serde_json::json!({
    "error": { "code": code, "message": "The remote worker request was rejected." }
  }))
  .map_err(std::io::Error::other)?;
  write_response(stream, status, &body)
}

fn write_response(
  stream: &mut StreamOwned<ServerConnection, TcpStream>,
  status: u16,
  body: &[u8],
) -> std::io::Result<()> {
  let reason = match status {
    200 => "OK",
    400 => "Bad Request",
    403 => "Forbidden",
    404 => "Not Found",
    408 => "Request Timeout",
    411 => "Length Required",
    413 => "Content Too Large",
    415 => "Unsupported Media Type",
    431 => "Request Header Fields Too Large",
    _ => "Service Unavailable",
  };
  write!(
    stream,
    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
    body.len()
  )?;
  stream.write_all(body)?;
  stream.flush()
}

fn server_configuration(
  options: &RemoteCoordinatorTlsOptionsV1,
) -> Result<ServerConfig, RemoteCoordinatorServerError> {
  let server_certificates = certificates(&options.certificate_path)?;
  let private_key = private_key(&options.private_key_path)?;
  let client_roots = certificates(&options.client_ca_path)?;
  let mut roots = RootCertStore::empty();
  for certificate in client_roots {
    roots
      .add(certificate)
      .map_err(|_| RemoteCoordinatorServerError::TlsConfiguration)?;
  }
  let verifier = WebPkiClientVerifier::builder(Arc::new(roots))
    .build()
    .map_err(|_| RemoteCoordinatorServerError::TlsConfiguration)?;
  ServerConfig::builder()
    .with_client_cert_verifier(verifier)
    .with_single_cert(server_certificates, private_key)
    .map_err(|_| RemoteCoordinatorServerError::TlsConfiguration)
}

fn certificates(
  path: &PathBuf,
) -> Result<Vec<CertificateDer<'static>>, RemoteCoordinatorServerError> {
  let file = File::open(path).map_err(|_| RemoteCoordinatorServerError::TlsConfiguration)?;
  let certificates = rustls_pemfile::certs(&mut BufReader::new(file))
    .collect::<Result<Vec<_>, _>>()
    .map_err(|_| RemoteCoordinatorServerError::TlsConfiguration)?;
  if certificates.is_empty() {
    return Err(RemoteCoordinatorServerError::TlsConfiguration);
  }
  Ok(certificates)
}

fn private_key(path: &PathBuf) -> Result<PrivateKeyDer<'static>, RemoteCoordinatorServerError> {
  let file = File::open(path).map_err(|_| RemoteCoordinatorServerError::TlsConfiguration)?;
  rustls_pemfile::private_key(&mut BufReader::new(file))
    .map_err(|_| RemoteCoordinatorServerError::TlsConfiguration)?
    .ok_or(RemoteCoordinatorServerError::TlsConfiguration)
}
