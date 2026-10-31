//! Private remote dispatch identity.

use crate::remote_dispatch::validate_request;
use crate::remote_dispatch::RemoteDispatchError;
use crate::remote_dispatch::RemoteDispatchRequestV1;
use chrono::DateTime;
use chrono::Utc;
use serde_json::json;
use serde_json_canonicalizer::to_vec as canonical_json;
use sha2::Digest;
use sha2::Sha256;

pub fn derive_remote_dispatch_id(
  request: &RemoteDispatchRequestV1,
) -> Result<String, RemoteDispatchError> {
  validate_request(request)?;
  let identity = json!({
    "domain": "woml.remote-dispatch/v1",
    "attempt": request.attempt,
    "invocationId": request.invocation_id,
    "iterationIdentity": request.iteration_identity,
    "nodeId": request.node_id,
    "runId": request.run_id,
    "target": request.target,
    "targetDigest": request.target_digest,
  });
  let bytes = canonical_json(&identity).map_err(|_| RemoteDispatchError::Invalid)?;
  Ok(format!("dispatch_{:x}", Sha256::digest(bytes)))
}

pub(in crate::remote_dispatch) fn digest_json(
  value: &serde_json::Value,
) -> Result<String, RemoteDispatchError> {
  let bytes = canonical_json(value).map_err(|_| RemoteDispatchError::Invalid)?;
  Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

pub(in crate::remote_dispatch) fn parse_time(
  value: &str,
) -> Result<DateTime<Utc>, RemoteDispatchError> {
  DateTime::parse_from_rfc3339(value)
    .map(|value| value.with_timezone(&Utc))
    .map_err(|_| RemoteDispatchError::Corrupt)
}

pub(in crate::remote_dispatch) fn parse_optional_time(
  value: Option<String>,
) -> Result<Option<DateTime<Utc>>, RemoteDispatchError> {
  value.map(|value| parse_time(&value)).transpose()
}

pub(in crate::remote_dispatch) fn valid_id(value: &str) -> bool {
  !value.is_empty()
    && value.len() <= 256
    && value
      .bytes()
      .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'-'))
}

pub(in crate::remote_dispatch) fn valid_digest(value: &str) -> bool {
  value.len() == 71
    && value.starts_with("sha256:")
    && value[7..]
      .bytes()
      .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
