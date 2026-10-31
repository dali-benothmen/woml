//! Private remote coordinator server http.

use crate::remote_coordinator_server::REMOTE_COORDINATOR_MAX_HEADER_BYTES;
use crate::WORKER_PROTOCOL_MAX_MESSAGE_BYTES;
use rustls::ServerConnection;
use rustls::StreamOwned;
use std::io::Read;
use std::io::Write;
use std::net::Shutdown;
use std::net::TcpStream;

pub(in crate::remote_coordinator_server) struct HttpRequest {
  pub(in crate::remote_coordinator_server) method: String,
  pub(in crate::remote_coordinator_server) path: String,
  pub(in crate::remote_coordinator_server) body: Vec<u8>,
}

pub(in crate::remote_coordinator_server) fn read_request(
  stream: &mut StreamOwned<ServerConnection, TcpStream>,
) -> Result<HttpRequest, u16> {
  let mut bytes = Vec::new();
  let mut buffer = [0u8; 4096];
  let header_end = loop {
    let read = stream.read(&mut buffer).map_err(request_read_error)?;
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
    let read = stream.read(&mut buffer).map_err(request_read_error)?;
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

fn request_read_error(error: std::io::Error) -> u16 {
  #[cfg(debug_assertions)]
  eprintln!(
    "WOML remote coordinator request read failed: {:?}",
    error.kind()
  );
  #[cfg(not(debug_assertions))]
  let _ = error;
  408
}

pub(in crate::remote_coordinator_server) fn connection_io_error(
  stage: &'static str,
  error: std::io::Error,
) -> &'static str {
  #[cfg(debug_assertions)]
  eprintln!("WOML remote coordinator {stage} failed: {:?}", error.kind());
  #[cfg(not(debug_assertions))]
  let _ = error;
  stage
}

pub(in crate::remote_coordinator_server) fn write_error(
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

pub(in crate::remote_coordinator_server) fn write_response(
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
  let response_header = format!(
    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
    body.len()
  );
  stream.conn.writer().write_all(response_header.as_bytes())?;
  stream.conn.writer().write_all(body)?;
  stream.conn.send_close_notify();
  while stream.conn.wants_write() {
    if stream.conn.write_tls(&mut stream.sock)? == 0 {
      return Err(std::io::ErrorKind::WriteZero.into());
    }
  }
  stream.sock.shutdown(Shutdown::Write)
}
