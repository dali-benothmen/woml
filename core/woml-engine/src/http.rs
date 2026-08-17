//! Managed HTTP v1 capability executed by Rust.

use std::{
  collections::HashMap,
  sync::{Arc, Mutex},
  time::Duration,
};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use futures_util::future::BoxFuture;
use reqwest::{
  header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE},
  redirect::Policy,
  Client, Method, Url,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::{
  CapabilityCallRequest, CapabilityCancellationToken, CapabilityDescriptor, CapabilityEffect,
  CapabilityFailure, CapabilityFailureKind, CapabilityHandler, DEFAULT_CAPABILITY_RESULT_BYTES,
};

const MANAGED_HTTP_CONTRACT: &str = "woml.managed-http";
const MANAGED_HTTP_CONTRACT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
enum RedirectMode {
  Follow,
  Manual,
  Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ResponseType {
  Json,
  Text,
  Bytes,
  Storage,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HttpStorageTarget {
  key: String,
  #[serde(default)]
  content_type: Option<String>,
  #[serde(default)]
  overwrite: bool,
  #[serde(default)]
  if_version: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AcceptedStatus {
  minimum: u16,
  maximum: u16,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExternalIdempotency {
  header: String,
  value: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManagedHttpRequest {
  contract: String,
  contract_version: u32,
  kind: String,
  method: String,
  url: String,
  headers: HashMap<String, String>,
  #[serde(default)]
  query: Option<HashMap<String, Value>>,
  #[serde(default)]
  #[serde(rename = "json")]
  _json: Option<Value>,
  #[serde(default)]
  text: Option<String>,
  #[serde(default)]
  bytes_base64: Option<String>,
  response_type: ResponseType,
  #[serde(default)]
  storage: Option<HttpStorageTarget>,
  timeout_ms: u64,
  accepted_status: AcceptedStatus,
  redirect: RedirectMode,
  maximum_redirects: usize,
  #[serde(default)]
  idempotency: Option<ExternalIdempotency>,
}

pub struct ManagedHttpHandler {
  clients: Mutex<HashMap<(RedirectMode, usize), Client>>,
  storage: Option<Arc<crate::ManagedStorageStore>>,
}

impl std::fmt::Debug for ManagedHttpHandler {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    formatter.debug_struct("ManagedHttpHandler").finish()
  }
}

impl Default for ManagedHttpHandler {
  fn default() -> Self {
    Self {
      clients: Mutex::new(HashMap::new()),
      storage: None,
    }
  }
}

impl ManagedHttpHandler {
  pub fn with_storage(storage: Arc<crate::ManagedStorageStore>) -> Self {
    Self {
      clients: Mutex::new(HashMap::new()),
      storage: Some(storage),
    }
  }

  fn client(&self, mode: RedirectMode, maximum: usize) -> Result<Client, CapabilityFailure> {
    let key = (mode, maximum);
    let mut clients = self.clients.lock().expect("managed HTTP client cache");
    if let Some(client) = clients.get(&key) {
      return Ok(client.clone());
    }
    let policy = match mode {
      RedirectMode::Follow => Policy::limited(maximum),
      RedirectMode::Manual => Policy::none(),
      RedirectMode::Error => Policy::custom(|attempt| attempt.error("redirects are disabled")),
    };
    let client = Client::builder().redirect(policy).build().map_err(|_| {
      failure(
        CapabilityFailureKind::HandlerCrashed,
        "WOML_HTTP_CLIENT_UNAVAILABLE",
        "The managed HTTP client could not be initialized.",
        false,
        false,
      )
    })?;
    clients.insert(key, client.clone());
    Ok(client)
  }
}

impl CapabilityHandler for ManagedHttpHandler {
  fn descriptor(&self) -> CapabilityDescriptor {
    CapabilityDescriptor {
      capability: "http".to_string(),
      operation: "request".to_string(),
      input_contract_version: 1,
      result_contract_version: 1,
      effect: CapabilityEffect::UnsafeWrite,
      supports_cancellation: true,
      supports_provider_idempotency: true,
    }
  }

  fn validate_request(&self, request: &CapabilityCallRequest) -> Result<(), CapabilityFailure> {
    let parsed = parse_request(&request.input)?;
    match (
      &parsed.idempotency,
      &request.identity.provider_idempotency_key,
    ) {
      (None, None) => Ok(()),
      (Some(idempotency), Some(identity)) if idempotency.value == *identity => Ok(()),
      _ => Err(failure(
        CapabilityFailureKind::InvalidInput,
        "WOML_HTTP_IDEMPOTENCY_MISMATCH",
        "Managed HTTP idempotency input does not match its capability identity.",
        false,
        false,
      )),
    }
  }

  fn effect(&self, input: &Value) -> CapabilityEffect {
    let Ok(request) = parse_request(input) else {
      return CapabilityEffect::UnsafeWrite;
    };
    if request.response_type == ResponseType::Storage {
      CapabilityEffect::IdempotentWrite
    } else if is_read_method(&request.method) {
      CapabilityEffect::Read
    } else if request.idempotency.is_some() {
      CapabilityEffect::IdempotentWrite
    } else {
      CapabilityEffect::UnsafeWrite
    }
  }

  fn safe_metadata(&self, input: &Value) -> Map<String, Value> {
    let Ok(request) = parse_request(input) else {
      return Map::new();
    };
    let Ok(url) = Url::parse(&request.url) else {
      return Map::new();
    };
    Map::from_iter([
      ("method".to_string(), Value::String(request.method)),
      (
        "origin".to_string(),
        Value::String(url.origin().ascii_serialization()),
      ),
    ])
  }

  fn safe_result_metadata(&self, result: &Value) -> Map<String, Value> {
    result
      .get("status")
      .and_then(Value::as_u64)
      .map(|status| Map::from_iter([("status".to_string(), Value::from(status))]))
      .unwrap_or_default()
  }

  fn execute(
    &self,
    input: Value,
    cancellation: CapabilityCancellationToken,
  ) -> BoxFuture<'static, Result<Value, CapabilityFailure>> {
    let request = match parse_request(&input) {
      Ok(request) => request,
      Err(error) => return Box::pin(async move { Err(error) }),
    };
    let json_body = input.get("json").cloned();
    let client = match self.client(request.redirect, request.maximum_redirects) {
      Ok(client) => client,
      Err(error) => return Box::pin(async move { Err(error) }),
    };
    let storage = self.storage.clone();
    Box::pin(
      async move { execute_request(client, request, json_body, cancellation, storage).await },
    )
  }
}

fn parse_request(input: &Value) -> Result<ManagedHttpRequest, CapabilityFailure> {
  let object = input
    .as_object()
    .ok_or_else(|| invalid_input("Managed HTTP input must be an object."))?;
  let body_count = ["json", "text", "bytesBase64"]
    .iter()
    .filter(|field| object.contains_key(**field))
    .count();
  if body_count > 1 {
    return Err(invalid_input(
      "Managed HTTP accepts only one of json, text, or bytesBase64.",
    ));
  }
  let request: ManagedHttpRequest = serde_json::from_value(input.clone())
    .map_err(|_| invalid_input("Managed HTTP input does not match contract v1."))?;
  if request.contract != MANAGED_HTTP_CONTRACT
    || request.contract_version != MANAGED_HTTP_CONTRACT_VERSION
    || request.kind != "request"
    || request.method.is_empty()
    || request.method.len() > 32
    || !request.method.bytes().all(|byte| byte.is_ascii_uppercase())
    || !(1..=86_400_000).contains(&request.timeout_ms)
    || request.maximum_redirects > 20
    || !(100..=599).contains(&request.accepted_status.minimum)
    || !(100..=599).contains(&request.accepted_status.maximum)
    || request.accepted_status.minimum > request.accepted_status.maximum
  {
    return Err(invalid_input(
      "Managed HTTP input does not match contract v1.",
    ));
  }
  let url =
    Url::parse(&request.url).map_err(|_| invalid_input("Managed HTTP requires a valid URL."))?;
  if !matches!(url.scheme(), "http" | "https") || request.url.len() > 8_192 {
    return Err(invalid_input(
      "Managed HTTP supports only bounded HTTP and HTTPS URLs.",
    ));
  }
  request
    .method
    .parse::<Method>()
    .map_err(|_| invalid_input("Managed HTTP method is invalid."))?;
  validate_headers(&request.headers)?;
  validate_query(request.query.as_ref())?;
  if let Some(idempotency) = &request.idempotency {
    if idempotency.header.is_empty()
      || idempotency.header.len() > 128
      || idempotency.value.is_empty()
      || idempotency.value.len() > 256
    {
      return Err(invalid_input(
        "Managed HTTP idempotency metadata exceeds its contract limits.",
      ));
    }
    let name = HeaderName::from_bytes(idempotency.header.as_bytes())
      .map_err(|_| invalid_input("Managed HTTP idempotency header is invalid."))?;
    HeaderValue::from_str(&idempotency.value)
      .map_err(|_| invalid_input("Managed HTTP idempotency value is invalid."))?;
    if request
      .headers
      .keys()
      .any(|header| header.eq_ignore_ascii_case(name.as_str()))
    {
      return Err(invalid_input(
        "Managed HTTP idempotency header must not also appear in headers.",
      ));
    }
  }
  if let Some(encoded) = &request.bytes_base64 {
    BASE64
      .decode(encoded)
      .map_err(|_| invalid_input("Managed HTTP bytesBase64 is not valid base64."))?;
  }
  match (request.response_type, request.storage.as_ref()) {
    (ResponseType::Storage, Some(target)) => {
      crate::ManagedStorageStore::validate_upload_target(
        &target.key,
        target.content_type.as_deref(),
        target.overwrite,
        target.if_version.as_deref(),
      )?;
    }
    (ResponseType::Storage, None) => {
      return Err(invalid_input(
        "Managed HTTP storage response mode requires a storage target.",
      ));
    }
    (_, Some(_)) => {
      return Err(invalid_input(
        "Managed HTTP storage is valid only with responseType storage.",
      ));
    }
    (_, None) => {}
  }
  Ok(request)
}

async fn execute_request(
  client: Client,
  request: ManagedHttpRequest,
  json_body: Option<Value>,
  cancellation: CapabilityCancellationToken,
  storage: Option<Arc<crate::ManagedStorageStore>>,
) -> Result<Value, CapabilityFailure> {
  let method = request.method.parse::<Method>().expect("validated method");
  let read = is_read_method(&request.method);
  let externally_idempotent = request.idempotency.is_some();
  let safe_to_retry = read || externally_idempotent;
  let mut url = Url::parse(&request.url).expect("validated URL");
  if let Some(query) = &request.query {
    let mut pairs = url.query_pairs_mut();
    for (key, value) in query {
      match value {
        Value::Array(values) => {
          for value in values {
            pairs.append_pair(key, &query_scalar(value));
          }
        }
        value => {
          pairs.append_pair(key, &query_scalar(value));
        }
      }
    }
  }
  let original_url = url.clone();
  let mut headers = HeaderMap::new();
  for (name, value) in &request.headers {
    headers.insert(
      HeaderName::from_bytes(name.as_bytes()).expect("validated header name"),
      HeaderValue::from_str(value).expect("validated header value"),
    );
  }
  if let Some(idempotency) = &request.idempotency {
    headers.insert(
      HeaderName::from_bytes(idempotency.header.as_bytes()).expect("validated idempotency header"),
      HeaderValue::from_str(&idempotency.value).expect("validated idempotency value"),
    );
  }
  let mut builder = client
    .request(method, url)
    .headers(headers)
    .timeout(Duration::from_millis(request.timeout_ms));
  if let Some(json) = json_body {
    builder = builder.json(&json);
  } else if let Some(text) = &request.text {
    builder = builder.body(text.clone());
  } else if let Some(bytes) = &request.bytes_base64 {
    builder = builder.body(BASE64.decode(bytes).expect("validated base64"));
  }

  let mut response = tokio::select! {
    _ = cancellation.cancelled() => return Err(failure(
      CapabilityFailureKind::Cancelled,
      "WOML_HTTP_CANCELLED",
      "The managed HTTP request was cancelled.",
      false,
      !read,
    )),
    response = builder.send() => response.map_err(|error| request_error(error, safe_to_retry, !read))?,
  };
  let status = response.status().as_u16();
  let final_url = response.url().clone();
  if final_url.as_str().len() > 8_192 {
    return Err(invalid_result(
      "The managed HTTP response URL exceeds its contract limit.",
    ));
  }
  let redirected = final_url != original_url;
  if status < request.accepted_status.minimum || status > request.accepted_status.maximum {
    let mut details = Map::new();
    details.insert("method".to_string(), Value::String(request.method));
    details.insert(
      "origin".to_string(),
      Value::String(final_url.origin().ascii_serialization()),
    );
    details.insert("status".to_string(), Value::from(status));
    return Err(CapabilityFailure {
      kind: CapabilityFailureKind::ServiceRejected,
      code: "WOML_HTTP_STATUS_REJECTED".to_string(),
      message: "The managed HTTP response status is outside the accepted range.".to_string(),
      retryable: safe_to_retry && (status == 429 || status >= 500),
      ambiguous: false,
      details: Some(details),
    });
  }

  let result_headers = response_headers(response.headers())?;
  if request.response_type == ResponseType::Storage {
    let target = request.storage.expect("validated HTTP storage target");
    let store = storage.ok_or_else(|| {
      failure(
        CapabilityFailureKind::HandlerCrashed,
        "WOML_HTTP_STORAGE_UNAVAILABLE",
        "Managed HTTP storage mode is unavailable in this runtime.",
        false,
        false,
      )
    })?;
    let content_type = target.content_type.unwrap_or_else(|| {
      response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string()
    });
    let mut upload = store.begin_upload(
      target.key,
      content_type,
      target.overwrite,
      target.if_version,
    )?;
    loop {
      let chunk = tokio::select! {
        _ = cancellation.cancelled() => return Err(failure(
          CapabilityFailureKind::Cancelled,
          "WOML_HTTP_CANCELLED",
          "The managed HTTP response was cancelled while storing its body.",
          false,
          !read,
        )),
        chunk = response.chunk() => chunk.map_err(|_| failure(
          CapabilityFailureKind::TransportFailed,
          "WOML_HTTP_RESPONSE_FAILED",
          "The managed HTTP response body could not be stored.",
          false,
          !read,
        ))?,
      };
      let Some(chunk) = chunk else { break };
      upload.write_chunk(&chunk, &cancellation)?;
    }
    let object = upload.finish(&cancellation)?;
    return Ok(json!({
      "contract": MANAGED_HTTP_CONTRACT,
      "contractVersion": MANAGED_HTTP_CONTRACT_VERSION,
      "kind": "result",
      "status": status,
      "ok": (200..=299).contains(&status),
      "headers": result_headers,
      "data": object,
      "url": final_url.to_string(),
      "redirected": redirected,
    }));
  }
  let mut body = Vec::new();
  loop {
    let chunk = tokio::select! {
      _ = cancellation.cancelled() => return Err(failure(
        CapabilityFailureKind::Cancelled,
        "WOML_HTTP_CANCELLED",
        "The managed HTTP response was cancelled while reading its body.",
        false,
        !read,
      )),
      chunk = response.chunk() => chunk.map_err(|_| failure(
        CapabilityFailureKind::TransportFailed,
        "WOML_HTTP_RESPONSE_FAILED",
        "The managed HTTP response body could not be read.",
        false,
        !read,
      ))?,
    };
    let Some(chunk) = chunk else { break };
    if body.len().saturating_add(chunk.len()) > DEFAULT_CAPABILITY_RESULT_BYTES as usize {
      return Err(failure(
        CapabilityFailureKind::ResultTooLarge,
        "WOML_HTTP_RESPONSE_TOO_LARGE",
        "The managed HTTP response exceeds its configured byte limit.",
        false,
        !read,
      ));
    }
    body.extend_from_slice(&chunk);
  }

  let data = match request.response_type {
    ResponseType::Json => serde_json::from_slice(&body).map_err(|_| {
      failure(
        CapabilityFailureKind::InvalidResult,
        "WOML_HTTP_RESPONSE_JSON_INVALID",
        "The managed HTTP response is not valid JSON.",
        false,
        !read,
      )
    })?,
    ResponseType::Text => Value::String(String::from_utf8(body).map_err(|_| {
      failure(
        CapabilityFailureKind::InvalidResult,
        "WOML_HTTP_RESPONSE_TEXT_INVALID",
        "The managed HTTP response is not valid UTF-8 text.",
        false,
        !read,
      )
    })?),
    ResponseType::Bytes => Value::String(BASE64.encode(body)),
    ResponseType::Storage => unreachable!("storage responses return while streaming"),
  };
  Ok(json!({
    "contract": MANAGED_HTTP_CONTRACT,
    "contractVersion": MANAGED_HTTP_CONTRACT_VERSION,
    "kind": "result",
    "status": status,
    "ok": (200..=299).contains(&status),
    "headers": result_headers,
    "data": data,
    "url": final_url.to_string(),
    "redirected": redirected,
  }))
}

fn response_headers(headers: &HeaderMap) -> Result<Map<String, Value>, CapabilityFailure> {
  if headers.keys().count() > 128 {
    return Err(invalid_result(
      "The managed HTTP response has too many headers.",
    ));
  }
  let mut result = Map::new();
  for name in headers.keys() {
    let values = headers
      .get_all(name)
      .iter()
      .map(|value| value.to_str())
      .collect::<Result<Vec<_>, _>>()
      .map_err(|_| invalid_result("The managed HTTP response contains a non-text header."))?;
    let value = values.join(", ");
    if value.len() > 16_384 {
      return Err(invalid_result(
        "A managed HTTP response header is too large.",
      ));
    }
    result.insert(name.as_str().to_string(), Value::String(value));
  }
  Ok(result)
}

fn validate_headers(headers: &HashMap<String, String>) -> Result<(), CapabilityFailure> {
  if headers.len() > 128 {
    return Err(invalid_input("Managed HTTP accepts at most 128 headers."));
  }
  for (name, value) in headers {
    if name.len() > 128
      || value.len() > 16_384
      || HeaderName::from_bytes(name.as_bytes()).is_err()
      || HeaderValue::from_str(value).is_err()
    {
      return Err(invalid_input("Managed HTTP contains an invalid header."));
    }
  }
  Ok(())
}

fn validate_query(query: Option<&HashMap<String, Value>>) -> Result<(), CapabilityFailure> {
  let Some(query) = query else { return Ok(()) };
  if query.len() > 128 {
    return Err(invalid_input(
      "Managed HTTP accepts at most 128 query fields.",
    ));
  }
  for value in query.values() {
    let values = value
      .as_array()
      .map(Vec::as_slice)
      .unwrap_or(std::slice::from_ref(value));
    if values.iter().any(|value| {
      !matches!(
        value,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
      )
    }) {
      return Err(invalid_input(
        "Managed HTTP query values must be scalar or scalar arrays.",
      ));
    }
  }
  Ok(())
}

fn query_scalar(value: &Value) -> String {
  match value {
    Value::Null => String::new(),
    Value::String(value) => value.clone(),
    value => value.to_string(),
  }
}

fn is_read_method(method: &str) -> bool {
  matches!(method, "GET" | "HEAD" | "OPTIONS")
}

fn request_error(error: reqwest::Error, safe_to_retry: bool, write: bool) -> CapabilityFailure {
  let (kind, code, message, ambiguous) = if error.is_timeout() {
    (
      CapabilityFailureKind::TimedOut,
      "WOML_HTTP_TIMED_OUT",
      "The managed HTTP request exceeded its deadline.",
      write,
    )
  } else if error.is_redirect() {
    (
      CapabilityFailureKind::ServiceRejected,
      "WOML_HTTP_REDIRECT_REJECTED",
      "The managed HTTP redirect policy rejected the response.",
      write,
    )
  } else {
    (
      CapabilityFailureKind::TransportFailed,
      "WOML_HTTP_TRANSPORT_FAILED",
      "The managed HTTP request could not be completed.",
      write && !error.is_connect(),
    )
  };
  failure(kind, code, message, safe_to_retry, ambiguous)
}

fn invalid_input(message: &str) -> CapabilityFailure {
  failure(
    CapabilityFailureKind::InvalidInput,
    "WOML_HTTP_INPUT_INVALID",
    message,
    false,
    false,
  )
}

fn invalid_result(message: &str) -> CapabilityFailure {
  failure(
    CapabilityFailureKind::InvalidResult,
    "WOML_HTTP_RESULT_INVALID",
    message,
    false,
    false,
  )
}

fn failure(
  kind: CapabilityFailureKind,
  code: &str,
  message: &str,
  retryable: bool,
  ambiguous: bool,
) -> CapabilityFailure {
  CapabilityFailure {
    kind,
    code: code.to_string(),
    message: message.to_string(),
    retryable,
    ambiguous,
    details: None,
  }
}
