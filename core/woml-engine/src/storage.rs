//! Storage v1 capability with a Rust-owned local object backend.

use std::{
  fs::{self, File, OpenOptions},
  io::{Read, Seek, SeekFrom, Write},
  path::{Path, PathBuf},
  sync::{Arc, Mutex},
};

use base64::{
  engine::general_purpose::{STANDARD as BASE64, URL_SAFE_NO_PAD},
  Engine as _,
};
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use serde_json_canonicalizer::to_vec as canonical_json;
use sha2::{Digest, Sha256};

use crate::{
  CapabilityCallRequest, CapabilityCancellationToken, CapabilityDescriptor, CapabilityEffect,
  CapabilityFailure, CapabilityFailureKind, CapabilityHandler,
};

const STORAGE_CONTRACT: &str = "woml.storage";
const STORAGE_CONTRACT_VERSION: u32 = 1;
const OBJECT_CONTRACT: &str = "woml.storage-object";
const OBJECT_CONTRACT_VERSION: u32 = 1;
const STORAGE_OPERATIONS: [&str; 5] = ["put", "get", "head", "list", "delete"];
const CONTAINER_MAGIC: &[u8; 8] = b"WOMLOBJ1";
const HEADER_REGION_BYTES: usize = 4_096;
const MAX_OBJECT_BYTES: u64 = 67_108_864;
const MAX_SCRIPT_BODY_BYTES: u64 = 3_000_000;
const DEFAULT_LIST_LIMIT: usize = 100;
const MAX_LIST_LIMIT: usize = 1_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StorageObjectReference {
  pub contract: String,
  pub contract_version: u32,
  pub key: String,
  pub version: String,
  pub checksum: StorageChecksum,
  pub size: u64,
  pub content_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StorageChecksum {
  pub algorithm: String,
  pub value: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StorageRequest {
  contract: String,
  contract_version: u32,
  kind: String,
  operation: String,
  input: Value,
}

#[derive(Debug, Clone)]
struct StorageRoot {
  path: PathBuf,
  lock_path: PathBuf,
}

#[derive(Default)]
pub struct ManagedStorageStore {
  root: Mutex<Option<StorageRoot>>,
}

impl std::fmt::Debug for ManagedStorageStore {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    formatter
      .debug_struct("ManagedStorageStore")
      .field(
        "configured",
        &self.root.lock().map(|root| root.is_some()).unwrap_or(false),
      )
      .finish()
  }
}

impl ManagedStorageStore {
  pub(crate) fn validate_upload_target(
    key: &str,
    content_type: Option<&str>,
    overwrite: bool,
    if_version: Option<&str>,
  ) -> Result<(), CapabilityFailure> {
    validate_key(key)?;
    if let Some(content_type) = content_type {
      validate_content_type(content_type)?;
    }
    validate_version(if_version)?;
    if overwrite && if_version.is_some() {
      return Err(storage_input(
        "Storage overwrite and ifVersion are mutually exclusive.",
      ));
    }
    Ok(())
  }

  pub fn configure_for_state(&self, state_path: &Path) -> Result<(), CapabilityFailure> {
    let parent = state_path.parent().unwrap_or_else(|| Path::new("."));
    let parent = fs::canonicalize(parent).map_err(|_| storage_unavailable())?;
    self.configure_root(parent.join("objects-v1"))
  }

  pub fn configure_root(&self, path: PathBuf) -> Result<(), CapabilityFailure> {
    if let Ok(metadata) = fs::symlink_metadata(&path) {
      if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(storage_path_unsafe());
      }
    } else {
      fs::create_dir(&path).map_err(|_| storage_unavailable())?;
    }
    let path = fs::canonicalize(path).map_err(|_| storage_unavailable())?;
    let lock_path = path.join(".storage-v1.lock");
    if fs::symlink_metadata(&lock_path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
      return Err(storage_path_unsafe());
    }
    let _ = OpenOptions::new()
      .create(true)
      .read(true)
      .write(true)
      .open(&lock_path)
      .map_err(|_| storage_unavailable())?;
    let root = StorageRoot { path, lock_path };
    let mut configured = self.root.lock().expect("storage root lock");
    if configured
      .as_ref()
      .is_some_and(|existing| existing.path != root.path)
    {
      return Err(storage_failure(
        CapabilityFailureKind::ServiceRejected,
        "WOML_STORAGE_ROOT_CONFLICT",
        "The runtime storage root is already bound to another state location.",
        false,
        false,
      ));
    }
    *configured = Some(root);
    Ok(())
  }

  fn root(&self) -> Result<StorageRoot, CapabilityFailure> {
    self
      .root
      .lock()
      .expect("storage root lock")
      .clone()
      .ok_or_else(storage_unavailable)
  }

  pub(crate) fn begin_upload(
    self: &Arc<Self>,
    key: String,
    content_type: String,
    overwrite: bool,
    if_version: Option<String>,
  ) -> Result<StorageUpload, CapabilityFailure> {
    Self::validate_upload_target(&key, Some(&content_type), overwrite, if_version.as_deref())?;
    let root = self.root()?;
    ensure_safe_root(&root)?;
    let temp_path = root
      .path
      .join(format!(".upload-{}.tmp", uuid::Uuid::new_v4().simple()));
    let mut file = OpenOptions::new()
      .create_new(true)
      .read(true)
      .write(true)
      .open(&temp_path)
      .map_err(|_| storage_unavailable())?;
    file
      .write_all(CONTAINER_MAGIC)
      .and_then(|_| file.write_all(&vec![0_u8; HEADER_REGION_BYTES]))
      .map_err(|_| storage_unavailable())?;
    Ok(StorageUpload {
      root,
      temp_path,
      file: Some(file),
      key,
      content_type,
      overwrite,
      if_version,
      digest: Sha256::new(),
      size: 0,
      committed: false,
    })
  }

  fn put_bytes(
    self: &Arc<Self>,
    input: &Map<String, Value>,
    cancellation: &CapabilityCancellationToken,
  ) -> Result<Value, CapabilityFailure> {
    let key = required_string(input, "key")?;
    let content_type = put_content_type(input)?;
    let overwrite = input
      .get("overwrite")
      .and_then(Value::as_bool)
      .unwrap_or(false);
    let if_version = optional_string(input, "ifVersion")?;
    let body = put_body(input)?;
    let mut upload = self.begin_upload(key, content_type, overwrite, if_version)?;
    upload.write_chunk(&body, cancellation)?;
    let object = upload.finish(cancellation)?;
    serde_json::to_value(object).map_err(|_| storage_corrupt())
  }

  fn get(&self, input: &Map<String, Value>) -> Result<Value, CapabilityFailure> {
    let key = required_string(input, "key")?;
    validate_key(&key)?;
    let response_type = input
      .get("responseType")
      .and_then(Value::as_str)
      .ok_or_else(|| storage_input("Storage get requires responseType."))?;
    if !matches!(response_type, "json" | "text" | "bytes") {
      return Err(storage_input("Storage responseType is unsupported."));
    }
    let if_version = optional_string(input, "ifVersion")?;
    validate_version(if_version.as_deref())?;
    let root = self.root()?;
    let _lock = StoreLock::shared(&root)?;
    let (object, body) = read_complete_object(&root, &key)?
      .ok_or_else(|| storage_not_found("The requested storage object does not exist."))?;
    if if_version
      .as_ref()
      .is_some_and(|version| *version != object.version)
    {
      return Err(storage_conflict(
        "The storage object does not match ifVersion.",
      ));
    }
    if object.size > MAX_SCRIPT_BODY_BYTES {
      return Err(storage_failure(
        CapabilityFailureKind::ResultTooLarge,
        "WOML_STORAGE_GET_TOO_LARGE",
        "The object is too large to return to a script; use head or keep its reference.",
        false,
        false,
      ));
    }
    let data = match response_type {
      "json" => serde_json::from_slice(&body).map_err(|_| {
        storage_failure(
          CapabilityFailureKind::InvalidResult,
          "WOML_STORAGE_JSON_INVALID",
          "The stored object is not valid JSON.",
          false,
          false,
        )
      })?,
      "text" => Value::String(String::from_utf8(body).map_err(|_| {
        storage_failure(
          CapabilityFailureKind::InvalidResult,
          "WOML_STORAGE_TEXT_INVALID",
          "The stored object is not valid UTF-8 text.",
          false,
          false,
        )
      })?),
      "bytes" => json!({ "bytesBase64": BASE64.encode(body) }),
      _ => unreachable!("response type validated"),
    };
    Ok(json!({ "object": object, "data": data }))
  }

  fn head(&self, input: &Map<String, Value>) -> Result<Value, CapabilityFailure> {
    let key = required_string(input, "key")?;
    validate_key(&key)?;
    let root = self.root()?;
    let _lock = StoreLock::shared(&root)?;
    read_object_header(&root, &key)
      .map(|object| serde_json::to_value(object).unwrap_or(Value::Null))
  }

  fn list(&self, input: &Map<String, Value>) -> Result<Value, CapabilityFailure> {
    let prefix = input.get("prefix").and_then(Value::as_str).unwrap_or("");
    validate_prefix(prefix)?;
    let limit = input
      .get("limit")
      .and_then(Value::as_u64)
      .unwrap_or(DEFAULT_LIST_LIMIT as u64);
    if !(1..=MAX_LIST_LIMIT as u64).contains(&limit) {
      return Err(storage_input(
        "Storage list limit must be between 1 and 1000.",
      ));
    }
    let cursor = input
      .get("cursor")
      .map(|value| {
        value
          .as_str()
          .ok_or_else(|| storage_input("Storage list cursor must be a string."))
          .and_then(decode_cursor)
      })
      .transpose()?;
    let root = self.root()?;
    let _lock = StoreLock::shared(&root)?;
    ensure_safe_root(&root)?;
    let mut objects = Vec::new();
    for entry in fs::read_dir(&root.path).map_err(|_| storage_unavailable())? {
      let entry = entry.map_err(|_| storage_unavailable())?;
      let name = entry.file_name();
      let name = name.to_string_lossy();
      if !name.ends_with(".wobj") {
        continue;
      }
      if entry
        .file_type()
        .map_err(|_| storage_unavailable())?
        .is_symlink()
      {
        return Err(storage_path_unsafe());
      }
      let object = read_header_path(&entry.path())?;
      if object.key.starts_with(prefix) && cursor.as_ref().is_none_or(|cursor| object.key > *cursor)
      {
        objects.push(object);
      }
    }
    objects.sort_by(|left, right| left.key.cmp(&right.key));
    let has_more = objects.len() > limit as usize;
    objects.truncate(limit as usize);
    let next_cursor = has_more
      .then(|| objects.last().map(|object| encode_cursor(&object.key)))
      .flatten();
    Ok(json!({ "objects": objects, "nextCursor": next_cursor }))
  }

  fn delete(&self, input: &Map<String, Value>) -> Result<Value, CapabilityFailure> {
    let key = required_string(input, "key")?;
    validate_key(&key)?;
    let if_version = optional_string(input, "ifVersion")?;
    validate_version(if_version.as_deref())?;
    let root = self.root()?;
    let _lock = StoreLock::exclusive(&root)?;
    let object = read_object_header(&root, &key)?;
    let Some(object) = object else {
      if if_version.is_some() {
        return Err(storage_conflict(
          "The conditional storage delete found no current object.",
        ));
      }
      return Ok(json!({ "deleted": false, "object": null }));
    };
    if if_version
      .as_ref()
      .is_some_and(|version| *version != object.version)
    {
      return Err(storage_conflict(
        "The storage object does not match ifVersion.",
      ));
    }
    let path = object_path(&root, &key);
    reject_symlink(&path)?;
    fs::remove_file(path).map_err(|_| storage_unavailable())?;
    sync_directory(&root.path)?;
    Ok(json!({ "deleted": true, "object": object }))
  }
}

pub(crate) struct StorageUpload {
  root: StorageRoot,
  temp_path: PathBuf,
  file: Option<File>,
  key: String,
  content_type: String,
  overwrite: bool,
  if_version: Option<String>,
  digest: Sha256,
  size: u64,
  committed: bool,
}

impl StorageUpload {
  pub(crate) fn write_chunk(
    &mut self,
    chunk: &[u8],
    cancellation: &CapabilityCancellationToken,
  ) -> Result<(), CapabilityFailure> {
    if cancellation.is_cancelled() {
      return Err(storage_cancelled());
    }
    self.size = self.size.saturating_add(chunk.len() as u64);
    if self.size > MAX_OBJECT_BYTES {
      return Err(storage_failure(
        CapabilityFailureKind::ResultTooLarge,
        "WOML_STORAGE_OBJECT_TOO_LARGE",
        "The storage object exceeds 64 MiB.",
        false,
        false,
      ));
    }
    self.digest.update(chunk);
    self
      .file
      .as_mut()
      .expect("active storage upload")
      .write_all(chunk)
      .map_err(|_| storage_unavailable())
  }

  pub(crate) fn finish(
    mut self,
    cancellation: &CapabilityCancellationToken,
  ) -> Result<StorageObjectReference, CapabilityFailure> {
    if cancellation.is_cancelled() {
      return Err(storage_cancelled());
    }
    let checksum = hex::encode(self.digest.clone().finalize());
    let version = object_version(&self.content_type, &checksum);
    let object = StorageObjectReference {
      contract: OBJECT_CONTRACT.to_string(),
      contract_version: OBJECT_CONTRACT_VERSION,
      key: self.key.clone(),
      version,
      checksum: StorageChecksum {
        algorithm: "sha256".to_string(),
        value: checksum,
      },
      size: self.size,
      content_type: self.content_type.clone(),
    };
    let header = serde_json::to_vec(&object).map_err(|_| storage_corrupt())?;
    if header.len() + 4 > HEADER_REGION_BYTES {
      return Err(storage_corrupt());
    }
    let file = self.file.as_mut().expect("active storage upload");
    file
      .seek(SeekFrom::Start(CONTAINER_MAGIC.len() as u64))
      .and_then(|_| file.write_all(&(header.len() as u32).to_be_bytes()))
      .and_then(|_| file.write_all(&header))
      .and_then(|_| file.sync_all())
      .map_err(|_| storage_unavailable())?;

    let _lock = StoreLock::exclusive(&self.root)?;
    let current = read_object_header(&self.root, &self.key)?;
    match (&current, &self.if_version, self.overwrite) {
      (Some(current), Some(expected), _) if current.version != *expected => {
        return Err(storage_conflict(
          "The storage object does not match ifVersion.",
        ));
      }
      (None, Some(_), _) => {
        return Err(storage_conflict(
          "The conditional storage write found no current object.",
        ));
      }
      (Some(current), None, false) if current == &object => {
        return Ok(current.clone());
      }
      (Some(_), None, false) => {
        return Err(storage_conflict(
          "The storage key already exists; use overwrite or ifVersion.",
        ));
      }
      _ => {}
    }
    if cancellation.is_cancelled() {
      return Err(storage_cancelled());
    }
    self.file.take();
    let target = object_path(&self.root, &self.key);
    if target.exists() {
      reject_symlink(&target)?;
    }
    fs::rename(&self.temp_path, &target).map_err(|_| storage_unavailable())?;
    sync_directory(&self.root.path)?;
    self.committed = true;
    Ok(object)
  }
}

impl Drop for StorageUpload {
  fn drop(&mut self) {
    if !self.committed {
      self.file.take();
      let _ = fs::remove_file(&self.temp_path);
    }
  }
}

struct StoreLock {
  file: File,
}

impl StoreLock {
  fn shared(root: &StorageRoot) -> Result<Self, CapabilityFailure> {
    Self::open(root, false)
  }

  fn exclusive(root: &StorageRoot) -> Result<Self, CapabilityFailure> {
    Self::open(root, true)
  }

  fn open(root: &StorageRoot, exclusive: bool) -> Result<Self, CapabilityFailure> {
    ensure_safe_root(root)?;
    reject_symlink(&root.lock_path)?;
    let file = OpenOptions::new()
      .read(true)
      .write(true)
      .open(&root.lock_path)
      .map_err(|_| storage_unavailable())?;
    if exclusive {
      fs2::FileExt::lock_exclusive(&file).map_err(|_| storage_unavailable())?;
    } else {
      fs2::FileExt::lock_shared(&file).map_err(|_| storage_unavailable())?;
    }
    Ok(Self { file })
  }
}

impl Drop for StoreLock {
  fn drop(&mut self) {
    let _ = fs2::FileExt::unlock(&self.file);
  }
}

#[derive(Debug)]
pub struct ManagedStorageHandler {
  store: Arc<ManagedStorageStore>,
  operation: &'static str,
}

impl ManagedStorageHandler {
  pub fn handlers(store: Arc<ManagedStorageStore>) -> Vec<Arc<Self>> {
    STORAGE_OPERATIONS
      .iter()
      .map(|operation| {
        Arc::new(Self {
          store: Arc::clone(&store),
          operation,
        })
      })
      .collect()
  }
}

impl CapabilityHandler for ManagedStorageHandler {
  fn descriptor(&self) -> CapabilityDescriptor {
    CapabilityDescriptor {
      capability: "storage".to_string(),
      operation: self.operation.to_string(),
      input_contract_version: 1,
      result_contract_version: 1,
      effect: if matches!(self.operation, "get" | "head" | "list") {
        CapabilityEffect::Read
      } else {
        CapabilityEffect::IdempotentWrite
      },
      supports_cancellation: true,
      supports_provider_idempotency: false,
    }
  }

  fn validate_request(&self, request: &CapabilityCallRequest) -> Result<(), CapabilityFailure> {
    parse_request(&request.input, self.operation).map(|_| ())
  }

  fn safe_metadata(&self, input: &Value) -> Map<String, Value> {
    let key = input
      .get("input")
      .and_then(|input| input.get("key"))
      .and_then(Value::as_str);
    key
      .map(|key| Map::from_iter([("key".to_string(), Value::String(key.to_string()))]))
      .unwrap_or_default()
  }

  fn safe_result_metadata(&self, result: &Value) -> Map<String, Value> {
    let data = result.get("data");
    let object = match self.operation {
      "put" | "head" => data,
      "get" => data.and_then(|data| data.get("object")),
      "delete" => data.and_then(|data| data.get("object")),
      _ => None,
    };
    if let Some(object) = object.filter(|object| !object.is_null()) {
      return Map::from_iter([
        (
          "version".to_string(),
          object.get("version").cloned().unwrap_or(Value::Null),
        ),
        (
          "size".to_string(),
          object.get("size").cloned().unwrap_or(Value::Null),
        ),
        (
          "contentType".to_string(),
          object.get("contentType").cloned().unwrap_or(Value::Null),
        ),
      ]);
    }
    if self.operation == "list" {
      return Map::from_iter([(
        "objectCount".to_string(),
        Value::from(
          data
            .and_then(|data| data.get("objects"))
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0),
        ),
      )]);
    }
    Map::new()
  }

  fn execute(
    &self,
    input: Value,
    cancellation: CapabilityCancellationToken,
  ) -> BoxFuture<'static, Result<Value, CapabilityFailure>> {
    let request = match parse_request(&input, self.operation) {
      Ok(request) => request,
      Err(error) => return Box::pin(async move { Err(error) }),
    };
    let operation = self.operation.to_string();
    let store = Arc::clone(&self.store);
    Box::pin(async move {
      let task = tokio::task::spawn_blocking(move || {
        if cancellation.is_cancelled() {
          return Err(storage_cancelled());
        }
        let object = request.input.as_object().expect("validated storage input");
        let data = match operation.as_str() {
          "put" => store.put_bytes(object, &cancellation),
          "get" => store.get(object),
          "head" => store.head(object),
          "list" => store.list(object),
          "delete" => store.delete(object),
          _ => unreachable!("registered storage operation"),
        }?;
        Ok(json!({
          "contract": STORAGE_CONTRACT,
          "contractVersion": STORAGE_CONTRACT_VERSION,
          "kind": "result",
          "operation": operation,
          "data": data,
        }))
      });
      task.await.map_err(|_| {
        storage_failure(
          CapabilityFailureKind::HandlerCrashed,
          "WOML_STORAGE_HANDLER_CRASHED",
          "The storage handler stopped unexpectedly.",
          false,
          false,
        )
      })?
    })
  }
}

fn parse_request(input: &Value, operation: &str) -> Result<StorageRequest, CapabilityFailure> {
  let request: StorageRequest = serde_json::from_value(input.clone())
    .map_err(|_| storage_input("Storage input does not match contract v1."))?;
  if request.contract != STORAGE_CONTRACT
    || request.contract_version != STORAGE_CONTRACT_VERSION
    || request.kind != "request"
    || request.operation != operation
    || !STORAGE_OPERATIONS.contains(&request.operation.as_str())
  {
    return Err(storage_input("Storage input does not match contract v1."));
  }
  let object = request
    .input
    .as_object()
    .ok_or_else(|| storage_input("Storage operation input must be an object."))?;
  validate_operation(operation, object)?;
  Ok(request)
}

fn validate_operation(
  operation: &str,
  input: &Map<String, Value>,
) -> Result<(), CapabilityFailure> {
  let (allowed, required): (&[&str], &[&str]) = match operation {
    "put" => (
      &[
        "key",
        "value",
        "text",
        "bytesBase64",
        "contentType",
        "overwrite",
        "ifVersion",
      ],
      &["key"],
    ),
    "get" => (
      &["key", "responseType", "ifVersion"],
      &["key", "responseType"],
    ),
    "head" => (&["key"], &["key"]),
    "list" => (&["prefix", "limit", "cursor"], &[]),
    "delete" => (&["key", "ifVersion"], &["key"]),
    _ => return Err(storage_input("Storage operation is unsupported.")),
  };
  if input.keys().any(|field| !allowed.contains(&field.as_str()))
    || required.iter().any(|field| !input.contains_key(*field))
  {
    return Err(storage_input(
      "Storage operation input has missing or unsupported fields.",
    ));
  }
  match operation {
    "put" => {
      validate_key(&required_string(input, "key")?)?;
      let body_count = ["value", "text", "bytesBase64"]
        .iter()
        .filter(|field| input.contains_key(**field))
        .count();
      if body_count != 1 {
        return Err(storage_input(
          "Storage put requires exactly one of value, text, or bytesBase64.",
        ));
      }
      if input.contains_key("overwrite") && input.contains_key("ifVersion") {
        return Err(storage_input(
          "Storage overwrite and ifVersion are mutually exclusive.",
        ));
      }
      if input
        .get("overwrite")
        .is_some_and(|value| value.as_bool().is_none())
      {
        return Err(storage_input("Storage overwrite must be a Boolean."));
      }
      validate_version(optional_string(input, "ifVersion")?.as_deref())?;
      validate_content_type(&put_content_type(input)?)?;
      let _ = put_body(input)?;
    }
    "get" => {
      validate_key(&required_string(input, "key")?)?;
      if !matches!(
        input.get("responseType").and_then(Value::as_str),
        Some("json" | "text" | "bytes")
      ) {
        return Err(storage_input("Storage responseType is unsupported."));
      }
      validate_version(optional_string(input, "ifVersion")?.as_deref())?;
    }
    "head" | "delete" => {
      validate_key(&required_string(input, "key")?)?;
      validate_version(optional_string(input, "ifVersion")?.as_deref())?;
    }
    "list" => {
      validate_prefix(input.get("prefix").and_then(Value::as_str).unwrap_or(""))?;
      if input
        .get("limit")
        .is_some_and(|value| !matches!(value.as_u64(), Some(1..=1000)))
      {
        return Err(storage_input(
          "Storage list limit must be between 1 and 1000.",
        ));
      }
      if let Some(cursor) = input.get("cursor") {
        decode_cursor(
          cursor
            .as_str()
            .ok_or_else(|| storage_input("Storage list cursor must be a string."))?,
        )?;
      }
    }
    _ => unreachable!(),
  }
  Ok(())
}

fn put_body(input: &Map<String, Value>) -> Result<Vec<u8>, CapabilityFailure> {
  if let Some(value) = input.get("value") {
    return canonical_json(value).map_err(|_| storage_input("Storage JSON value is invalid."));
  }
  if let Some(text) = input.get("text") {
    return text
      .as_str()
      .map(|text| text.as_bytes().to_vec())
      .ok_or_else(|| storage_input("Storage text must be a string."));
  }
  if let Some(bytes) = input.get("bytesBase64") {
    return bytes
      .as_str()
      .and_then(|bytes| BASE64.decode(bytes).ok())
      .ok_or_else(|| storage_input("Storage bytesBase64 is invalid."));
  }
  Err(storage_input("Storage put body is missing."))
}

fn put_content_type(input: &Map<String, Value>) -> Result<String, CapabilityFailure> {
  if let Some(content_type) = input.get("contentType") {
    return content_type
      .as_str()
      .map(str::to_string)
      .ok_or_else(|| storage_input("Storage contentType must be a string."));
  }
  if input.contains_key("value") {
    Ok("application/json".to_string())
  } else if input.contains_key("text") {
    Ok("text/plain; charset=utf-8".to_string())
  } else {
    Ok("application/octet-stream".to_string())
  }
}

fn required_string(input: &Map<String, Value>, field: &str) -> Result<String, CapabilityFailure> {
  input
    .get(field)
    .and_then(Value::as_str)
    .map(str::to_string)
    .ok_or_else(|| storage_input("A required storage string is missing or invalid."))
}

fn optional_string(
  input: &Map<String, Value>,
  field: &str,
) -> Result<Option<String>, CapabilityFailure> {
  input
    .get(field)
    .map(|value| {
      value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| storage_input("An optional storage string is invalid."))
    })
    .transpose()
}

fn validate_key(key: &str) -> Result<(), CapabilityFailure> {
  if key.is_empty()
    || key.len() > 512
    || key.starts_with('/')
    || key.ends_with('/')
    || key.contains('\\')
    || key
      .chars()
      .any(|character| character == '\0' || character.is_control())
    || key
      .split('/')
      .any(|segment| segment.is_empty() || matches!(segment, "." | "..") || segment.len() > 128)
  {
    return Err(storage_input("The storage key is invalid or unsafe."));
  }
  Ok(())
}

fn validate_prefix(prefix: &str) -> Result<(), CapabilityFailure> {
  if prefix.is_empty() {
    return Ok(());
  }
  if prefix.len() > 512
    || prefix.starts_with('/')
    || prefix.contains('\\')
    || prefix
      .chars()
      .any(|character| character == '\0' || character.is_control())
    || prefix
      .trim_end_matches('/')
      .split('/')
      .any(|segment| segment.is_empty() || matches!(segment, "." | "..") || segment.len() > 128)
  {
    return Err(storage_input("The storage prefix is invalid or unsafe."));
  }
  Ok(())
}

fn validate_content_type(content_type: &str) -> Result<(), CapabilityFailure> {
  if content_type.is_empty()
    || content_type.len() > 256
    || !content_type.contains('/')
    || content_type.chars().any(|character| character.is_control())
  {
    return Err(storage_input("The storage contentType is invalid."));
  }
  Ok(())
}

fn validate_version(version: Option<&str>) -> Result<(), CapabilityFailure> {
  if version.is_some_and(|version| {
    !version.strip_prefix("v1:").is_some_and(|digest| {
      digest.len() == 64
        && digest
          .bytes()
          .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
  }) {
    return Err(storage_input("The storage version is invalid."));
  }
  Ok(())
}

fn object_version(content_type: &str, checksum: &str) -> String {
  let mut digest = Sha256::new();
  digest.update(b"woml.storage-object\0v1\0");
  digest.update(content_type.as_bytes());
  digest.update(b"\0");
  digest.update(checksum.as_bytes());
  format!("v1:{}", hex::encode(digest.finalize()))
}

fn key_digest(key: &str) -> String {
  let mut digest = Sha256::new();
  digest.update(b"woml.storage-key\0v1\0");
  digest.update(key.as_bytes());
  hex::encode(digest.finalize())
}

fn object_path(root: &StorageRoot, key: &str) -> PathBuf {
  root.path.join(format!("{}.wobj", key_digest(key)))
}

fn ensure_safe_root(root: &StorageRoot) -> Result<(), CapabilityFailure> {
  let metadata = fs::symlink_metadata(&root.path).map_err(|_| storage_unavailable())?;
  if metadata.file_type().is_symlink() || !metadata.is_dir() {
    return Err(storage_path_unsafe());
  }
  Ok(())
}

fn reject_symlink(path: &Path) -> Result<(), CapabilityFailure> {
  if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
    return Err(storage_path_unsafe());
  }
  Ok(())
}

fn read_object_header(
  root: &StorageRoot,
  key: &str,
) -> Result<Option<StorageObjectReference>, CapabilityFailure> {
  let path = object_path(root, key);
  if !path.exists() {
    return Ok(None);
  }
  reject_symlink(&path)?;
  let object = read_header_path(&path)?;
  let expected_name = format!("{}.wobj", key_digest(key));
  if object.key != key
    || path.file_name().and_then(|name| name.to_str()) != Some(expected_name.as_str())
  {
    return Err(storage_corrupt());
  }
  Ok(Some(object))
}

fn read_header_path(path: &Path) -> Result<StorageObjectReference, CapabilityFailure> {
  reject_symlink(path)?;
  let mut file = File::open(path).map_err(|_| storage_unavailable())?;
  let mut magic = [0_u8; 8];
  file.read_exact(&mut magic).map_err(|_| storage_corrupt())?;
  if &magic != CONTAINER_MAGIC {
    return Err(storage_corrupt());
  }
  let mut region = vec![0_u8; HEADER_REGION_BYTES];
  file
    .read_exact(&mut region)
    .map_err(|_| storage_corrupt())?;
  let length = u32::from_be_bytes(region[0..4].try_into().expect("four bytes")) as usize;
  if length == 0 || length + 4 > HEADER_REGION_BYTES {
    return Err(storage_corrupt());
  }
  let object: StorageObjectReference =
    serde_json::from_slice(&region[4..4 + length]).map_err(|_| storage_corrupt())?;
  validate_stored_reference(&object)?;
  let actual = file.metadata().map_err(|_| storage_unavailable())?.len();
  if actual != (CONTAINER_MAGIC.len() + HEADER_REGION_BYTES) as u64 + object.size {
    return Err(storage_corrupt());
  }
  Ok(object)
}

fn read_complete_object(
  root: &StorageRoot,
  key: &str,
) -> Result<Option<(StorageObjectReference, Vec<u8>)>, CapabilityFailure> {
  let path = object_path(root, key);
  let Some(object) = read_object_header(root, key)? else {
    return Ok(None);
  };
  let mut file = File::open(path).map_err(|_| storage_unavailable())?;
  file
    .seek(SeekFrom::Start(
      (CONTAINER_MAGIC.len() + HEADER_REGION_BYTES) as u64,
    ))
    .map_err(|_| storage_corrupt())?;
  let mut body = Vec::with_capacity(object.size as usize);
  file.read_to_end(&mut body).map_err(|_| storage_corrupt())?;
  let checksum = hex::encode(Sha256::digest(&body));
  if checksum != object.checksum.value {
    return Err(storage_corrupt());
  }
  Ok(Some((object, body)))
}

fn validate_stored_reference(object: &StorageObjectReference) -> Result<(), CapabilityFailure> {
  validate_key(&object.key).map_err(|_| storage_corrupt())?;
  validate_version(Some(&object.version)).map_err(|_| storage_corrupt())?;
  validate_content_type(&object.content_type).map_err(|_| storage_corrupt())?;
  if object.contract != OBJECT_CONTRACT
    || object.contract_version != OBJECT_CONTRACT_VERSION
    || object.checksum.algorithm != "sha256"
    || object.checksum.value.len() != 64
    || !object
      .checksum
      .value
      .bytes()
      .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    || object.size > MAX_OBJECT_BYTES
    || object.version != object_version(&object.content_type, &object.checksum.value)
  {
    return Err(storage_corrupt());
  }
  Ok(())
}

fn encode_cursor(key: &str) -> String {
  URL_SAFE_NO_PAD.encode(key.as_bytes())
}

fn decode_cursor(cursor: &str) -> Result<String, CapabilityFailure> {
  if cursor.is_empty() || cursor.len() > 1_024 {
    return Err(storage_input("The storage list cursor is invalid."));
  }
  let bytes = URL_SAFE_NO_PAD
    .decode(cursor)
    .map_err(|_| storage_input("The storage list cursor is invalid."))?;
  let key =
    String::from_utf8(bytes).map_err(|_| storage_input("The storage list cursor is invalid."))?;
  validate_key(&key)?;
  Ok(key)
}

fn sync_directory(path: &Path) -> Result<(), CapabilityFailure> {
  File::open(path)
    .and_then(|directory| directory.sync_all())
    .map_err(|_| storage_unavailable())
}

fn storage_input(message: &str) -> CapabilityFailure {
  storage_failure(
    CapabilityFailureKind::InvalidInput,
    "WOML_STORAGE_INPUT_INVALID",
    message,
    false,
    false,
  )
}

fn storage_not_found(message: &str) -> CapabilityFailure {
  storage_failure(
    CapabilityFailureKind::ServiceRejected,
    "WOML_STORAGE_NOT_FOUND",
    message,
    false,
    false,
  )
}

fn storage_conflict(message: &str) -> CapabilityFailure {
  storage_failure(
    CapabilityFailureKind::ServiceRejected,
    "WOML_STORAGE_CONFLICT",
    message,
    false,
    false,
  )
}

fn storage_corrupt() -> CapabilityFailure {
  storage_failure(
    CapabilityFailureKind::InvalidResult,
    "WOML_STORAGE_CORRUPT",
    "The stored object failed its integrity checks.",
    false,
    false,
  )
}

fn storage_unavailable() -> CapabilityFailure {
  storage_failure(
    CapabilityFailureKind::TransportFailed,
    "WOML_STORAGE_UNAVAILABLE",
    "The local object store is unavailable.",
    true,
    false,
  )
}

fn storage_path_unsafe() -> CapabilityFailure {
  storage_failure(
    CapabilityFailureKind::ServiceRejected,
    "WOML_STORAGE_PATH_UNSAFE",
    "The local object store contains an unsafe filesystem path.",
    false,
    false,
  )
}

fn storage_cancelled() -> CapabilityFailure {
  storage_failure(
    CapabilityFailureKind::Cancelled,
    "WOML_STORAGE_CANCELLED",
    "The storage operation was cancelled.",
    false,
    false,
  )
}

fn storage_failure(
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

#[cfg(test)]
mod tests {
  use super::*;

  struct TestRoot(PathBuf);

  impl TestRoot {
    fn new(name: &str) -> Self {
      let path =
        std::env::temp_dir().join(format!("woml-sc9-{name}-{}", uuid::Uuid::new_v4().simple()));
      fs::create_dir(&path).unwrap();
      Self(path)
    }

    fn store(&self) -> Arc<ManagedStorageStore> {
      let store = Arc::new(ManagedStorageStore::default());
      store.configure_root(self.0.clone()).unwrap();
      store
    }
  }

  impl Drop for TestRoot {
    fn drop(&mut self) {
      let _ = fs::remove_dir_all(&self.0);
    }
  }

  fn put_input(key: &str, text: &str) -> Map<String, Value> {
    Map::from_iter([
      ("key".to_string(), Value::String(key.to_string())),
      ("text".to_string(), Value::String(text.to_string())),
    ])
  }

  #[test]
  fn key_and_prefix_grammar_rejects_traversal_and_ambiguous_segments() {
    for key in ["../secret", "/absolute", "a//b", "a/./b", "a/../b", "a\\b"] {
      assert!(validate_key(key).is_err(), "{key}");
    }
    for key in ["reports/day.json", "customer 42/avatar.png", "ümlaut/data"] {
      assert!(validate_key(key).is_ok(), "{key}");
    }
    assert!(validate_prefix("").is_ok());
    assert!(validate_prefix("reports/").is_ok());
  }

  #[test]
  fn content_version_covers_both_bytes_and_content_type() {
    let checksum = hex::encode(Sha256::digest(b"hello"));
    assert_eq!(
      object_version("text/plain", &checksum),
      object_version("text/plain", &checksum)
    );
    assert_ne!(
      object_version("text/plain", &checksum),
      object_version("application/octet-stream", &checksum)
    );
  }

  #[test]
  fn local_store_round_trips_lists_deletes_and_survives_restart() {
    let root = TestRoot::new("round-trip");
    let store = root.store();
    let cancellation = CapabilityCancellationToken::default();
    let object: StorageObjectReference = serde_json::from_value(
      store
        .put_bytes(&put_input("reports/day.txt", "hello"), &cancellation)
        .unwrap(),
    )
    .unwrap();
    assert_eq!(object.size, 5);
    assert_eq!(object.content_type, "text/plain; charset=utf-8");

    let restarted = root.store();
    let get = restarted
      .get(&Map::from_iter([
        ("key".to_string(), json!("reports/day.txt")),
        ("responseType".to_string(), json!("text")),
      ]))
      .unwrap();
    assert_eq!(get["data"], "hello");
    assert_eq!(get["object"]["version"], object.version);
    assert_eq!(
      restarted
        .head(&Map::from_iter([(
          "key".to_string(),
          json!("reports/day.txt"),
        )]))
        .unwrap()["checksum"],
      json!(object.checksum)
    );
    let list = restarted.list(&Map::new()).unwrap();
    assert_eq!(list["objects"].as_array().unwrap().len(), 1);
    assert_eq!(list["nextCursor"], Value::Null);
    let deleted = restarted
      .delete(&Map::from_iter([(
        "key".to_string(),
        json!("reports/day.txt"),
      )]))
      .unwrap();
    assert_eq!(deleted["deleted"], true);
    assert_eq!(
      restarted
        .head(&Map::from_iter([(
          "key".to_string(),
          json!("reports/day.txt"),
        )]))
        .unwrap(),
      Value::Null
    );
  }

  #[test]
  fn atomic_conditions_allow_exactly_one_competing_replacement() {
    let root = TestRoot::new("conditions");
    let store = root.store();
    let cancellation = CapabilityCancellationToken::default();
    let current: StorageObjectReference = serde_json::from_value(
      store
        .put_bytes(&put_input("shared.txt", "initial"), &cancellation)
        .unwrap(),
    )
    .unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let handles = ["first", "second"].map(|text| {
      let store = root.store();
      let barrier = Arc::clone(&barrier);
      let version = current.version.clone();
      std::thread::spawn(move || {
        let mut input = put_input("shared.txt", text);
        input.insert("ifVersion".to_string(), Value::String(version));
        barrier.wait();
        store.put_bytes(&input, &CapabilityCancellationToken::default())
      })
    });
    barrier.wait();
    let outcomes = handles.map(|handle| handle.join().unwrap());
    assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
    assert_eq!(
      outcomes
        .iter()
        .filter_map(|outcome| outcome.as_ref().err())
        .next()
        .unwrap()
        .code,
      "WOML_STORAGE_CONFLICT"
    );
  }

  #[test]
  fn partial_uploads_are_invisible_and_checksum_corruption_fails_closed() {
    let root = TestRoot::new("integrity");
    let store = root.store();
    fs::write(root.0.join(".upload-interrupted.tmp"), b"partial").unwrap();
    assert_eq!(
      store.list(&Map::new()).unwrap()["objects"]
        .as_array()
        .unwrap()
        .len(),
      0
    );
    store
      .put_bytes(
        &put_input("protected.txt", "original"),
        &CapabilityCancellationToken::default(),
      )
      .unwrap();
    let object_path = fs::read_dir(&root.0)
      .unwrap()
      .filter_map(Result::ok)
      .map(|entry| entry.path())
      .find(|path| {
        path
          .extension()
          .is_some_and(|extension| extension == "wobj")
      })
      .unwrap();
    let mut object = OpenOptions::new().write(true).open(object_path).unwrap();
    object
      .seek(SeekFrom::Start(
        (CONTAINER_MAGIC.len() + HEADER_REGION_BYTES) as u64,
      ))
      .unwrap();
    object.write_all(b"X").unwrap();
    let error = store
      .get(&Map::from_iter([
        ("key".to_string(), json!("protected.txt")),
        ("responseType".to_string(), json!("text")),
      ]))
      .unwrap_err();
    assert_eq!(error.code, "WOML_STORAGE_CORRUPT");
  }

  #[test]
  fn upload_limits_and_cancellation_remove_temporary_files() {
    let root = TestRoot::new("limits");
    let store = root.store();
    let cancellation = CapabilityCancellationToken::default();
    let mut upload = store
      .begin_upload(
        "large.bin".to_string(),
        "application/octet-stream".to_string(),
        false,
        None,
      )
      .unwrap();
    upload.size = MAX_OBJECT_BYTES;
    let error = upload.write_chunk(&[0], &cancellation).unwrap_err();
    assert_eq!(error.code, "WOML_STORAGE_OBJECT_TOO_LARGE");
    drop(upload);
    cancellation.cancel();
    let mut cancelled = store
      .begin_upload(
        "cancelled.bin".to_string(),
        "application/octet-stream".to_string(),
        false,
        None,
      )
      .unwrap();
    assert_eq!(
      cancelled.write_chunk(&[1], &cancellation).unwrap_err().code,
      "WOML_STORAGE_CANCELLED"
    );
    drop(cancelled);
    assert_eq!(
      fs::read_dir(&root.0)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(".upload-"))
        .count(),
      0
    );
  }

  #[cfg(unix)]
  #[test]
  fn symlinked_store_roots_are_rejected() {
    use std::os::unix::fs::symlink;

    let parent = TestRoot::new("symlink");
    let target = parent.0.join("real");
    fs::create_dir(&target).unwrap();
    let linked = parent.0.join("linked");
    symlink(&target, &linked).unwrap();
    let store = ManagedStorageStore::default();
    assert_eq!(
      store.configure_root(linked).unwrap_err().code,
      "WOML_STORAGE_PATH_UNSAFE"
    );
  }
}
