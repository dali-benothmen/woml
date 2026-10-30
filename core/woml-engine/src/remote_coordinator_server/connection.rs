//! Private remote coordinator server connection.

use crate::decode_worker_dispatch_fact;
use crate::decode_worker_hello;
use crate::decode_worker_poll;
use crate::decode_worker_shutdown;
use crate::decode_worker_terminal;
use crate::remote_coordinator_server::connection_io_error;
use crate::remote_coordinator_server::read_request;
use crate::remote_coordinator_server::write_error;
use crate::remote_coordinator_server::write_response;
use crate::remote_coordinator_server::REMOTE_COORDINATOR_IO_TIMEOUT_SECONDS;
use crate::RemoteCoordinatorError;
use crate::RemoteDispatchTransportV1;
use crate::RemoteSessionAuthorityV1;
use crate::RemoteTransportError;
use chrono::Utc;
use rustls::ServerConfig;
use rustls::ServerConnection;
use rustls::StreamOwned;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

pub(in crate::remote_coordinator_server) fn handle_connection(
  stream: TcpStream,
  tls: Arc<ServerConfig>,
  authority: Arc<Mutex<RemoteSessionAuthorityV1>>,
  transport: Option<Arc<RemoteDispatchTransportV1>>,
) -> Result<(), &'static str> {
  // The listener is nonblocking for shutdown polling. Accepted sockets can
  // inherit that mode on macOS and Windows, but this worker uses blocking TLS
  // reads with a bounded timeout. Set the mode explicitly on every platform.
  stream
    .set_nonblocking(false)
    .map_err(|error| connection_io_error("set_blocking", error))?;
  stream
    .set_read_timeout(Some(Duration::from_secs(
      REMOTE_COORDINATOR_IO_TIMEOUT_SECONDS,
    )))
    .map_err(|error| connection_io_error("set_read_timeout", error))?;
  stream
    .set_write_timeout(Some(Duration::from_secs(
      REMOTE_COORDINATOR_IO_TIMEOUT_SECONDS,
    )))
    .map_err(|error| connection_io_error("set_write_timeout", error))?;
  let connection = ServerConnection::new(tls).map_err(|_| "tls_connection")?;
  let mut stream = StreamOwned::new(connection, stream);
  let request = match read_request(&mut stream) {
    Ok(request) => request,
    Err(status) => {
      #[cfg(debug_assertions)]
      eprintln!("WOML remote coordinator rejected request with HTTP {status}");
      write_error(&mut stream, status, "WOML_REMOTE_PROTOCOL_INVALID")
        .map_err(|error| connection_io_error("write_request_error", error))?;
      return Ok(());
    }
  };
  let certificate_digest = stream
    .conn
    .peer_certificates()
    .and_then(|certificates| certificates.first())
    .map(|certificate| format!("sha256:{:x}", Sha256::digest(certificate.as_ref())))
    .ok_or("peer_certificate_missing")?;
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
        let mut authority = authority
          .lock()
          .map_err(|_| RemoteCoordinatorError::Unavailable)?;
        let mut response = authority.poll(poll.clone(), &certificate_digest, now)?;
        if response.status == "idle" {
          if let Some(transport) = &transport {
            authority.reserve_dispatch(&poll.session_id, &poll.target)?;
            match transport.offer_next(&poll.worker_id, &poll.session_id, &poll.target, now) {
              Ok(Some(offer)) => {
                response.status = "offered";
                response.dispatch =
                  Some(RemoteDispatchTransportV1::offer_json(offer).map_err(map_transport_error)?);
              }
              Ok(None) => {
                authority.release_dispatch(&poll.session_id)?;
              }
              Err(error) => {
                let _ = authority.release_dispatch(&poll.session_id);
                return Err(map_transport_error(error));
              }
            }
          }
        }
        Ok(response)
      })
      .and_then(|response| json_bytes(&response))
  } else if request.method == "POST" && request.path == "/woml/v1/workers/accepted" {
    dispatch_fact(
      &request.body,
      "accepted",
      &certificate_digest,
      now,
      &authority,
      transport.as_deref(),
      |transport, fact| transport.accept(fact),
    )
  } else if request.method == "POST" && request.path == "/woml/v1/workers/started" {
    dispatch_fact(
      &request.body,
      "started",
      &certificate_digest,
      now,
      &authority,
      transport.as_deref(),
      |transport, fact| transport.start(fact),
    )
  } else if request.method == "POST" && request.path == "/woml/v1/workers/terminal" {
    decode_worker_terminal(&request.body)
      .map_err(|_| RemoteCoordinatorError::ReplayOrConflict)
      .and_then(|terminal| {
        let mut authority = authority
          .lock()
          .map_err(|_| RemoteCoordinatorError::Unavailable)?;
        authority.authorize_dispatch_fact(
          &terminal.worker_id,
          &terminal.session_id,
          &terminal.target,
          &terminal.message_id,
          terminal.sent_at,
          &certificate_digest,
          now,
        )?;
        let session_id = terminal.session_id.clone();
        let receipt = transport
          .as_deref()
          .ok_or(RemoteCoordinatorError::Unavailable)?
          .terminal(terminal)
          .map_err(map_transport_error)?;
        authority.release_dispatch(&session_id)?;
        json_bytes(&receipt)
      })
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
    write_error(&mut stream, 404, "WOML_TARGET_UNAVAILABLE")
      .map_err(|error| connection_io_error("write_not_found", error))?;
    return Ok(());
  };
  match result {
    Ok(body) => write_response(&mut stream, 200, &body)
      .map_err(|error| connection_io_error("write_response", error))?,
    Err(error) => {
      let status = match error {
        RemoteCoordinatorError::Unauthorized => 403,
        RemoteCoordinatorError::Incompatible | RemoteCoordinatorError::ReplayOrConflict => 400,
        RemoteCoordinatorError::Unavailable | RemoteCoordinatorError::Capacity => 503,
      };
      write_error(&mut stream, status, error.code())
        .map_err(|error| connection_io_error("write_authority_error", error))?;
    }
  }
  Ok(())
}

#[allow(clippy::too_many_arguments)]
fn dispatch_fact<F>(
  body: &[u8],
  expected_type: &str,
  certificate_digest: &str,
  now: chrono::DateTime<Utc>,
  authority: &Arc<Mutex<RemoteSessionAuthorityV1>>,
  transport: Option<&RemoteDispatchTransportV1>,
  transition: F,
) -> Result<Vec<u8>, RemoteCoordinatorError>
where
  F: FnOnce(
    &RemoteDispatchTransportV1,
    &crate::WorkerDispatchFactV1,
  ) -> Result<(), RemoteTransportError>,
{
  let fact = decode_worker_dispatch_fact(body, expected_type)
    .map_err(|_| RemoteCoordinatorError::ReplayOrConflict)?;
  authority
    .lock()
    .map_err(|_| RemoteCoordinatorError::Unavailable)?
    .authorize_dispatch_fact(
      &fact.worker_id,
      &fact.session_id,
      &fact.target,
      &fact.message_id,
      fact.sent_at,
      certificate_digest,
      now,
    )?;
  transition(transport.ok_or(RemoteCoordinatorError::Unavailable)?, &fact)
    .map_err(map_transport_error)?;
  json_bytes(&serde_json::json!({
    "profile": "woml.remote-dispatch-ack/v1",
    "dispatchId": fact.dispatch_id,
    "status": expected_type
  }))
}

fn map_transport_error(error: RemoteTransportError) -> RemoteCoordinatorError {
  match error {
    RemoteTransportError::Capacity => RemoteCoordinatorError::Capacity,
    RemoteTransportError::Unavailable => RemoteCoordinatorError::Unavailable,
    RemoteTransportError::Invalid | RemoteTransportError::Dispatch(_) => {
      RemoteCoordinatorError::ReplayOrConflict
    }
  }
}

fn json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, RemoteCoordinatorError> {
  serde_json::to_vec(value).map_err(|_| RemoteCoordinatorError::Unavailable)
}
