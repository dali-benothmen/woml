//! Existing encrypted S3-compatible transport, signing, bounded multipart/recovery, verification, inventory, and store binding.

use crate::artifacts::artifact_cancelled;
use crate::artifacts::artifact_corrupt;
use crate::artifacts::backend_unavailable;
use crate::artifacts::cleanup_directory_contents;
use crate::artifacts::ensure_private_directory;
use crate::artifacts::ArtifactBackend;
use crate::artifacts::ArtifactStage;
use crate::artifacts::PreparedS3Backend;
use crate::artifacts::S3ServerSideEncryption;
use crate::artifacts::VerifiedArtifactBlob;
use crate::artifacts::ARTIFACT_ORPHAN_SCAN_BATCH;
use crate::artifacts::STREAM_BUFFER_BYTES;
use crate::CapabilityCancellationToken;
use crate::CapabilityFailure;
use chrono::Utc;
use hmac::Hmac;
use hmac::Mac;
use reqwest::Method;
use reqwest::StatusCode;
use reqwest::Url;
use rusqlite::Connection;
use sha2::Digest;
use sha2::Sha256;
use std::collections::BTreeMap;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

const S3_MULTIPART_THRESHOLD_BYTES: u64 = 64 * 1024 * 1024;

const S3_MULTIPART_PART_BYTES: usize = 8 * 1024 * 1024;

const S3_RECOVERY_ATTEMPTS: usize = 4;

const S3_CONTROL_RESPONSE_MAX_BYTES: usize = 1024 * 1024;

#[derive(Clone)]
pub(in crate::artifacts) struct S3ArtifactBackend {
  pub(in crate::artifacts) config: PreparedS3Backend,
  pub(in crate::artifacts) client: reqwest::Client,
  pub(in crate::artifacts) staging: PathBuf,
  pub(in crate::artifacts) identity: String,
}

impl S3ArtifactBackend {
  pub(in crate::artifacts) fn open(
    config: PreparedS3Backend,
    materializations: &Path,
  ) -> Result<Self, CapabilityFailure> {
    let staging = materializations.join(".s3-staging");
    ensure_private_directory(&staging)?;
    let client = reqwest::Client::builder()
      .https_only(config.endpoint.scheme() == "https")
      .connect_timeout(Duration::from_secs(15))
      .timeout(Duration::from_secs(60 * 60))
      .redirect(reqwest::redirect::Policy::none())
      .build()
      .map_err(|_| backend_unavailable())?;
    let identity = format!(
      "s3://{}/{}@{}",
      config.bucket,
      config.prefix,
      config.endpoint.origin().ascii_serialization()
    );
    Ok(Self {
      config,
      client,
      staging,
      identity,
    })
  }

  pub(in crate::artifacts) fn object_key(&self, suffix: &str) -> String {
    format!("{}{}", self.config.prefix, suffix)
  }

  pub(in crate::artifacts) fn blob_key(&self, digest: &str) -> String {
    self.object_key(&format!("blobs/sha256/{}/{}", &digest[..2], &digest[2..]))
  }

  pub(in crate::artifacts) fn endpoint(
    &self,
    key: &str,
    query: &[(String, String)],
  ) -> Result<(Url, String, String), CapabilityFailure> {
    let mut url = self.config.endpoint.clone();
    let base = url.path().trim_end_matches('/').to_string();
    let path = if self.config.force_path_style {
      format!("{base}/{}/{key}", self.config.bucket)
    } else {
      let host = url.host_str().ok_or_else(backend_unavailable)?;
      url
        .set_host(Some(&format!("{}.{}", self.config.bucket, host)))
        .map_err(|_| backend_unavailable())?;
      format!("{base}/{key}")
    };
    url.set_path(&path);
    let mut encoded_query = query
      .iter()
      .map(|(name, value)| {
        format!(
          "{}={}",
          aws_percent_encode(name.as_bytes()),
          aws_percent_encode(value.as_bytes())
        )
      })
      .collect::<Vec<_>>();
    encoded_query.sort();
    let canonical_query = encoded_query.join("&");
    url.set_query((!canonical_query.is_empty()).then_some(&canonical_query));
    let canonical_uri = if url.path().is_empty() {
      "/".to_string()
    } else {
      url.path().to_string()
    };
    Ok((url, canonical_uri, canonical_query))
  }

  pub(in crate::artifacts) async fn send(
    &self,
    method: Method,
    key: &str,
    query: Vec<(String, String)>,
    payload_hash: String,
    body: Option<reqwest::Body>,
    mut extra_headers: BTreeMap<String, String>,
  ) -> Result<reqwest::Response, CapabilityFailure> {
    let (url, canonical_uri, canonical_query) = self.endpoint(key, &query)?;
    let now = Utc::now();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date = now.format("%Y%m%d").to_string();
    let host = match url.port() {
      Some(port) => format!("{}:{port}", url.host_str().ok_or_else(backend_unavailable)?),
      None => url.host_str().ok_or_else(backend_unavailable)?.to_string(),
    };
    extra_headers.insert("host".to_string(), host);
    extra_headers.insert("x-amz-content-sha256".to_string(), payload_hash.clone());
    extra_headers.insert("x-amz-date".to_string(), amz_date.clone());
    if let Some(token) = &self.config.credentials.session_token {
      extra_headers.insert("x-amz-security-token".to_string(), token.clone());
    }
    let canonical_headers = extra_headers
      .iter()
      .map(|(name, value)| format!("{name}:{}\n", value.trim()))
      .collect::<String>();
    let signed_headers = extra_headers.keys().cloned().collect::<Vec<_>>().join(";");
    let canonical_request = format!(
      "{}\n{}\n{}\n{}\n{}\n{}",
      method.as_str(),
      canonical_uri,
      canonical_query,
      canonical_headers,
      signed_headers,
      payload_hash
    );
    let scope = format!("{date}/{}/s3/aws4_request", self.config.region);
    let string_to_sign = format!(
      "AWS4-HMAC-SHA256\n{}\n{}\n{}",
      amz_date,
      scope,
      hex::encode(Sha256::digest(canonical_request.as_bytes()))
    );
    let date_key = hmac_sha256(
      format!("AWS4{}", self.config.credentials.secret_key).as_bytes(),
      date.as_bytes(),
    )?;
    let region_key = hmac_sha256(&date_key, self.config.region.as_bytes())?;
    let service_key = hmac_sha256(&region_key, b"s3")?;
    let signing_key = hmac_sha256(&service_key, b"aws4_request")?;
    let signature = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes())?);
    let authorization = format!(
      "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
      self.config.credentials.access_key, scope, signed_headers, signature
    );
    let mut request = self
      .client
      .request(method, url)
      .header("authorization", authorization);
    for (name, value) in extra_headers {
      request = request.header(name, value);
    }
    if let Some(body) = body {
      request = request.body(body);
    }
    request.send().await.map_err(|_| backend_unavailable())
  }

  pub(in crate::artifacts) fn encryption_headers(&self) -> BTreeMap<String, String> {
    match &self.config.server_side_encryption {
      S3ServerSideEncryption::Aes256 => BTreeMap::from([(
        "x-amz-server-side-encryption".to_string(),
        "AES256".to_string(),
      )]),
      S3ServerSideEncryption::AwsKms { key_id } => BTreeMap::from([
        (
          "x-amz-server-side-encryption".to_string(),
          "aws:kms".to_string(),
        ),
        (
          "x-amz-server-side-encryption-aws-kms-key-id".to_string(),
          key_id.clone(),
        ),
      ]),
    }
  }

  pub(in crate::artifacts) fn run<T, F>(&self, future: F) -> Result<T, CapabilityFailure>
  where
    T: Send + 'static,
    F: std::future::Future<Output = Result<T, CapabilityFailure>> + Send + 'static,
  {
    std::thread::spawn(move || {
      tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| backend_unavailable())?
        .block_on(future)
    })
    .join()
    .map_err(|_| backend_unavailable())?
  }

  pub(in crate::artifacts) fn upload(
    &self,
    path: &Path,
    key: &str,
    digest: &str,
    size: u64,
    cancellation: &CapabilityCancellationToken,
  ) -> Result<(), CapabilityFailure> {
    if size >= S3_MULTIPART_THRESHOLD_BYTES {
      return self.upload_multipart(path, key, size, cancellation);
    }
    for attempt in 0..S3_RECOVERY_ATTEMPTS {
      if cancellation.is_cancelled() {
        return Err(artifact_cancelled());
      }
      let backend = self.clone();
      let path = path.to_path_buf();
      let key = key.to_string();
      let digest = digest.to_string();
      let cancellation = cancellation.clone();
      let outcome = self.run(async move {
        let body = streaming_request_body(path, cancellation)?;
        let mut headers = backend.encryption_headers();
        headers.insert("if-none-match".to_string(), "*".to_string());
        headers.insert("content-length".to_string(), size.to_string());
        let response = backend
          .send(Method::PUT, &key, vec![], digest, Some(body), headers)
          .await?;
        if response.status().is_success() || response.status() == StatusCode::PRECONDITION_FAILED {
          Ok(())
        } else {
          Err(backend_unavailable())
        }
      });
      if outcome.is_ok() {
        return outcome;
      }
      if attempt + 1 < S3_RECOVERY_ATTEMPTS {
        std::thread::sleep(Duration::from_millis(25 * (attempt as u64 + 1)));
      }
    }
    Err(backend_unavailable())
  }

  pub(in crate::artifacts) fn upload_multipart(
    &self,
    path: &Path,
    key: &str,
    size: u64,
    cancellation: &CapabilityCancellationToken,
  ) -> Result<(), CapabilityFailure> {
    let backend = self.clone();
    let path = path.to_path_buf();
    let key = key.to_string();
    let cancellation = cancellation.clone();
    self.run(async move {
      let empty_hash = hex::encode(Sha256::digest([]));
      let initiated = backend
        .send(
          Method::POST,
          &key,
          vec![("uploads".to_string(), String::new())],
          empty_hash.clone(),
          None,
          backend.encryption_headers(),
        )
        .await?;
      if !initiated.status().is_success() {
        return Err(backend_unavailable());
      }
      let initiated = bounded_response_text(initiated, S3_CONTROL_RESPONSE_MAX_BYTES).await?;
      let upload_id = xml_values(&initiated, "UploadId")
        .into_iter()
        .next()
        .ok_or_else(backend_unavailable)?;
      let outcome = async {
        let mut source = File::open(&path).map_err(|_| backend_unavailable())?;
        let mut parts = Vec::new();
        let mut uploaded = 0_u64;
        let mut part_number = 1_u32;
        while uploaded < size {
          if cancellation.is_cancelled() {
            return Err(artifact_cancelled());
          }
          let remaining = usize::try_from((size - uploaded).min(S3_MULTIPART_PART_BYTES as u64))
            .map_err(|_| backend_unavailable())?;
          let mut bytes = vec![0_u8; remaining];
          source
            .read_exact(&mut bytes)
            .map_err(|_| backend_unavailable())?;
          let part_hash = hex::encode(Sha256::digest(&bytes));
          let response = backend
            .send(
              Method::PUT,
              &key,
              vec![
                ("partNumber".to_string(), part_number.to_string()),
                ("uploadId".to_string(), upload_id.clone()),
              ],
              part_hash,
              Some(reqwest::Body::from(bytes)),
              BTreeMap::from([("content-length".to_string(), remaining.to_string())]),
            )
            .await?;
          if !response.status().is_success() {
            return Err(backend_unavailable());
          }
          let etag = response
            .headers()
            .get("etag")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
            .ok_or_else(backend_unavailable)?;
          parts.push((part_number, etag));
          uploaded += remaining as u64;
          part_number += 1;
        }
        let completion = format!(
          "<CompleteMultipartUpload>{}</CompleteMultipartUpload>",
          parts
            .iter()
            .map(|(number, etag)| format!(
              "<Part><PartNumber>{number}</PartNumber><ETag>{}</ETag></Part>",
              xml_escape(etag)
            ))
            .collect::<String>()
        );
        let completion_hash = hex::encode(Sha256::digest(completion.as_bytes()));
        let response = backend
          .send(
            Method::POST,
            &key,
            vec![("uploadId".to_string(), upload_id.clone())],
            completion_hash,
            Some(reqwest::Body::from(completion)),
            BTreeMap::from([("content-type".to_string(), "application/xml".to_string())]),
          )
          .await?;
        if response.status().is_success() {
          Ok(())
        } else {
          Err(backend_unavailable())
        }
      }
      .await;
      if outcome.is_err() {
        let _ = backend
          .send(
            Method::DELETE,
            &key,
            vec![("uploadId".to_string(), upload_id)],
            empty_hash,
            None,
            BTreeMap::new(),
          )
          .await;
      }
      outcome
    })
  }

  pub(in crate::artifacts) fn download_verified(
    &self,
    key: &str,
    digest: &str,
    size: u64,
  ) -> Result<VerifiedArtifactBlob, CapabilityFailure> {
    let backend = self.clone();
    let key = key.to_string();
    let digest = digest.to_string();
    let path = self
      .staging
      .join(format!("{}.download", uuid::Uuid::new_v4().simple()));
    let download_path = path.clone();
    let outcome = self.run(async move {
      let empty_hash = hex::encode(Sha256::digest([]));
      let mut response = None;
      let mut final_status = None;
      for attempt in 0..S3_RECOVERY_ATTEMPTS {
        match backend
          .send(
            Method::GET,
            &key,
            vec![],
            empty_hash.clone(),
            None,
            BTreeMap::new(),
          )
          .await
        {
          Ok(candidate) if candidate.status().is_success() => {
            response = Some(candidate);
            break;
          }
          Ok(candidate) => final_status = Some(candidate.status()),
          Err(_) => final_status = None,
        }
        if attempt + 1 < S3_RECOVERY_ATTEMPTS {
          tokio::time::sleep(Duration::from_millis(25 * (attempt as u64 + 1))).await;
        }
      }
      let mut response = response.ok_or_else(|| {
        if final_status == Some(StatusCode::NOT_FOUND) {
          artifact_corrupt()
        } else {
          backend_unavailable()
        }
      })?;
      let mut options = OpenOptions::new();
      options.create_new(true).write(true);
      #[cfg(unix)]
      {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
      }
      let mut target = options
        .open(&download_path)
        .map_err(|_| backend_unavailable())?;
      let mut hasher = Sha256::new();
      let mut received = 0_u64;
      while let Some(chunk) = response.chunk().await.map_err(|_| backend_unavailable())? {
        received = received.saturating_add(chunk.len() as u64);
        if received > size {
          return Err(artifact_corrupt());
        }
        hasher.update(&chunk);
        target
          .write_all(&chunk)
          .map_err(|_| backend_unavailable())?;
      }
      target.sync_all().map_err(|_| backend_unavailable())?;
      if received != size || hex::encode(hasher.finalize()) != digest {
        return Err(artifact_corrupt());
      }
      Ok(())
    });
    if let Err(error) = outcome {
      let _ = fs::remove_file(&path);
      return Err(error);
    }
    Ok(VerifiedArtifactBlob {
      path,
      temporary: true,
    })
  }

  pub(in crate::artifacts) fn list_inventory(
    &self,
    limit: usize,
  ) -> Result<Vec<String>, CapabilityFailure> {
    let backend = self.clone();
    let prefix = self.object_key("blobs/sha256/");
    self.run(async move {
      let empty_hash = hex::encode(Sha256::digest([]));
      for attempt in 0..S3_RECOVERY_ATTEMPTS {
        let response = backend
          .send(
            Method::GET,
            "",
            vec![
              ("list-type".to_string(), "2".to_string()),
              ("max-keys".to_string(), limit.min(1_000).to_string()),
              ("prefix".to_string(), prefix.clone()),
            ],
            empty_hash.clone(),
            None,
            BTreeMap::new(),
          )
          .await;
        match response {
          Ok(response) if response.status().is_success() => {
            let body = bounded_response_text(response, S3_CONTROL_RESPONSE_MAX_BYTES).await?;
            let mut digests = xml_values(&body, "Key")
              .into_iter()
              .filter_map(|key| key.strip_prefix(&prefix).map(str::to_string))
              .map(|suffix| suffix.replace('/', ""))
              .filter(|digest| {
                digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
              })
              .take(limit)
              .collect::<Vec<_>>();
            digests.sort();
            return Ok(digests);
          }
          Ok(response)
            if !response.status().is_server_error()
              && response.status() != StatusCode::REQUEST_TIMEOUT
              && response.status() != StatusCode::TOO_MANY_REQUESTS =>
          {
            return Err(backend_unavailable());
          }
          Ok(_) | Err(_) => {}
        }
        if attempt + 1 < S3_RECOVERY_ATTEMPTS {
          tokio::time::sleep(Duration::from_millis(25 * (attempt as u64 + 1))).await;
        }
      }
      Err(backend_unavailable())
    })
  }
}

impl ArtifactBackend for S3ArtifactBackend {
  fn identity(&self) -> &str {
    &self.identity
  }

  fn bind_store(&self, _connection: &Connection, store_id: &str) -> Result<(), CapabilityFailure> {
    let backend = self.clone();
    let key = self.object_key("store-id");
    let expected = format!("woml.artifact-store/v1\n{store_id}\n");
    self.run(async move {
      let empty_hash = hex::encode(Sha256::digest([]));
      let digest = hex::encode(Sha256::digest(expected.as_bytes()));
      for attempt in 0..S3_RECOVERY_ATTEMPTS {
        let get = backend
          .send(
            Method::GET,
            &key,
            vec![],
            empty_hash.clone(),
            None,
            BTreeMap::new(),
          )
          .await;
        match get {
          Ok(response) if response.status().is_success() => {
            let body = bounded_response_text(response, 1024).await?;
            return if body == expected {
              Ok(())
            } else {
              Err(backend_unavailable())
            };
          }
          Ok(response) if response.status() == StatusCode::NOT_FOUND => {
            let mut headers = backend.encryption_headers();
            headers.insert("if-none-match".to_string(), "*".to_string());
            headers.insert("content-length".to_string(), expected.len().to_string());
            if let Ok(response) = backend
              .send(
                Method::PUT,
                &key,
                vec![],
                digest.clone(),
                Some(reqwest::Body::from(expected.clone())),
                headers,
              )
              .await
            {
              if response.status().is_success() {
                return Ok(());
              }
              if response.status() != StatusCode::PRECONDITION_FAILED {
                return Err(backend_unavailable());
              }
            }
          }
          Ok(_) | Err(_) => {}
        }
        if attempt + 1 < S3_RECOVERY_ATTEMPTS {
          tokio::time::sleep(Duration::from_millis(25 * (attempt as u64 + 1))).await;
        }
      }
      Err(backend_unavailable())
    })
  }

  fn begin_stage(&self) -> Result<ArtifactStage, CapabilityFailure> {
    let path = self
      .staging
      .join(format!("{}.part", uuid::Uuid::new_v4().simple()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true).read(true);
    #[cfg(unix)]
    {
      use std::os::unix::fs::OpenOptionsExt;
      options.mode(0o600);
    }
    let file = options.open(&path).map_err(|_| backend_unavailable())?;
    Ok(ArtifactStage {
      path,
      file: Some(file),
    })
  }

  fn publish(
    &self,
    mut stage: ArtifactStage,
    sha256: &str,
    size: u64,
    cancellation: &CapabilityCancellationToken,
  ) -> Result<(), CapabilityFailure> {
    let file = stage.file.take().ok_or_else(backend_unavailable)?;
    file.sync_all().map_err(|_| backend_unavailable())?;
    drop(file);
    self.upload(
      &stage.path,
      &self.blob_key(sha256),
      sha256,
      size,
      cancellation,
    )?;
    let verified = self.verify(sha256, size)?;
    drop(verified);
    fs::remove_file(&stage.path).map_err(|_| backend_unavailable())
  }

  fn cleanup_startup(&self, connection: &Connection) -> Result<(), CapabilityFailure> {
    cleanup_directory_contents(&self.staging)?;
    for digest in self.inventory(ARTIFACT_ORPHAN_SCAN_BATCH)? {
      let reachable: bool = connection
        .query_row(
          "SELECT EXISTS(SELECT 1 FROM woml_artifact_blobs WHERE sha256 = ?1)",
          [&digest],
          |row| row.get(0),
        )
        .map_err(|_| backend_unavailable())?;
      if !reachable {
        self.delete(&digest)?;
      }
    }
    Ok(())
  }

  fn verify(&self, sha256: &str, size: u64) -> Result<VerifiedArtifactBlob, CapabilityFailure> {
    self.download_verified(&self.blob_key(sha256), sha256, size)
  }

  fn delete(&self, sha256: &str) -> Result<(), CapabilityFailure> {
    let backend = self.clone();
    let key = self.blob_key(sha256);
    self.run(async move {
      let empty_hash = hex::encode(Sha256::digest([]));
      for attempt in 0..S3_RECOVERY_ATTEMPTS {
        if let Ok(response) = backend
          .send(
            Method::DELETE,
            &key,
            vec![],
            empty_hash.clone(),
            None,
            BTreeMap::new(),
          )
          .await
        {
          if response.status().is_success() || response.status() == StatusCode::NOT_FOUND {
            return Ok(());
          }
        }
        if attempt + 1 < S3_RECOVERY_ATTEMPTS {
          tokio::time::sleep(Duration::from_millis(25 * (attempt as u64 + 1))).await;
        }
      }
      Err(backend_unavailable())
    })
  }

  fn inventory(&self, limit: usize) -> Result<Vec<String>, CapabilityFailure> {
    self.list_inventory(limit)
  }
}

fn streaming_request_body(
  path: PathBuf,
  cancellation: CapabilityCancellationToken,
) -> Result<reqwest::Body, CapabilityFailure> {
  let source = File::open(path).map_err(|_| backend_unavailable())?;
  let stream = futures_util::stream::try_unfold(
    (source, cancellation),
    |(mut source, cancellation)| async move {
      if cancellation.is_cancelled() {
        return Err(std::io::Error::new(
          std::io::ErrorKind::Interrupted,
          "artifact upload cancelled",
        ));
      }
      let mut buffer = vec![0_u8; STREAM_BUFFER_BYTES];
      let read = source.read(&mut buffer)?;
      if read == 0 {
        return Ok(None);
      }
      buffer.truncate(read);
      Ok(Some((bytes::Bytes::from(buffer), (source, cancellation))))
    },
  );
  Ok(reqwest::Body::wrap_stream(stream))
}

async fn bounded_response_text(
  mut response: reqwest::Response,
  maximum_bytes: usize,
) -> Result<String, CapabilityFailure> {
  let mut bytes = Vec::new();
  while let Some(chunk) = response.chunk().await.map_err(|_| backend_unavailable())? {
    if bytes.len().saturating_add(chunk.len()) > maximum_bytes {
      return Err(backend_unavailable());
    }
    bytes.extend_from_slice(&chunk);
  }
  String::from_utf8(bytes).map_err(|_| backend_unavailable())
}

fn hmac_sha256(key: &[u8], value: &[u8]) -> Result<Vec<u8>, CapabilityFailure> {
  let mut mac = Hmac::<Sha256>::new_from_slice(key).map_err(|_| backend_unavailable())?;
  mac.update(value);
  Ok(mac.finalize().into_bytes().to_vec())
}

fn aws_percent_encode(value: &[u8]) -> String {
  let mut encoded = String::new();
  for byte in value {
    if byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_' | b'.' | b'~') {
      encoded.push(*byte as char);
    } else {
      encoded.push_str(&format!("%{byte:02X}"));
    }
  }
  encoded
}

fn xml_values(source: &str, tag: &str) -> Vec<String> {
  let start = format!("<{tag}>");
  let end = format!("</{tag}>");
  let mut values = Vec::new();
  let mut remaining = source;
  while let Some(offset) = remaining.find(&start) {
    let value_start = offset + start.len();
    let Some(value_end) = remaining[value_start..].find(&end) else {
      break;
    };
    values.push(xml_unescape(
      &remaining[value_start..value_start + value_end],
    ));
    remaining = &remaining[value_start + value_end + end.len()..];
  }
  values
}

fn xml_escape(value: &str) -> String {
  value
    .replace('&', "&amp;")
    .replace('<', "&lt;")
    .replace('>', "&gt;")
    .replace('"', "&quot;")
    .replace('\'', "&apos;")
}

fn xml_unescape(value: &str) -> String {
  value
    .replace("&lt;", "<")
    .replace("&gt;", ">")
    .replace("&quot;", "\"")
    .replace("&apos;", "'")
    .replace("&amp;", "&")
}
