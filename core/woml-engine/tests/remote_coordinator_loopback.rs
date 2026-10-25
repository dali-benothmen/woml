use std::{
  io::{Read, Write},
  net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
  path::{Path, PathBuf},
  sync::Arc,
};

use chrono::{Duration, Utc};
use rcgen::{
  BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
  KeyPair, KeyUsagePurpose,
};
use rustls::{
  pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer, ServerName},
  ClientConfig, ClientConnection, RootCertStore, StreamOwned,
};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use woml_engine::{
  derive_remote_dispatch_id, prepare_target_registry, CompiledWorkflowDefinition,
  DurableEventStore, RemoteCoordinatorAuthorityV1, RemoteCoordinatorServerV1,
  RemoteCoordinatorTlsOptionsV1, RemoteDispatchPackageV1, RemoteDispatchRequestV1,
  RemoteDispatchStateV1, RemoteDispatchTransportV1, RemoteSessionAuthorityV1,
  TargetRegistryEntryV1, TargetRegistryV1, TriggerAdmissionRequest, TrustedWorkerIdentityV1,
  WorkerCapabilityV1,
};

const MODEL: &str =
  include_str!("../../../woml/tests/fixtures/remote-execution-targets/model.v21.reviewed.json");
const DEFINITION_HASH: &str =
  "sha256:2121212121212121212121212121212121212121212121212121212121212121";
const POLICY_DIGEST: &str =
  "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
  fn new() -> Self {
    let path =
      std::env::temp_dir().join(format!("woml-remote-loopback-{}", Uuid::new_v4().simple()));
    std::fs::create_dir_all(&path).unwrap();
    Self(path)
  }

  fn path(&self) -> &Path {
    &self.0
  }
}

impl Drop for TemporaryDirectory {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir_all(&self.0);
  }
}

struct TlsFixture {
  server_certificate: PathBuf,
  server_key: PathBuf,
  ca_certificate: PathBuf,
  client_certificate_der: rustls::pki_types::CertificateDer<'static>,
  client_key_der: Vec<u8>,
  roots: RootCertStore,
}

fn tls_fixture(directory: &Path) -> TlsFixture {
  let ca_key = KeyPair::generate().unwrap();
  let mut ca_params = CertificateParams::new(vec!["woml-loopback-ca".to_string()]).unwrap();
  ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
  ca_params.key_usages = vec![
    KeyUsagePurpose::KeyCertSign,
    KeyUsagePurpose::DigitalSignature,
  ];
  let ca = ca_params.self_signed(&ca_key).unwrap();

  let server_key = KeyPair::generate().unwrap();
  let mut server_params = CertificateParams::new(vec!["127.0.0.1".to_string()]).unwrap();
  let mut server_name = DistinguishedName::new();
  server_name.push(DnType::CommonName, "127.0.0.1");
  server_params.distinguished_name = server_name;
  server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
  let server = server_params.signed_by(&server_key, &ca, &ca_key).unwrap();

  let client_key = KeyPair::generate().unwrap();
  let mut client_params = CertificateParams::new(Vec::<String>::new()).unwrap();
  let mut client_name = DistinguishedName::new();
  client_name.push(DnType::CommonName, "production-runner-01");
  client_params.distinguished_name = client_name;
  client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
  let client = client_params.signed_by(&client_key, &ca, &ca_key).unwrap();

  let server_certificate = directory.join("server.pem");
  let server_key_path = directory.join("server.key");
  let ca_certificate = directory.join("ca.pem");
  std::fs::write(&server_certificate, server.pem()).unwrap();
  std::fs::write(&server_key_path, server_key.serialize_pem()).unwrap();
  std::fs::write(&ca_certificate, ca.pem()).unwrap();
  let mut roots = RootCertStore::empty();
  roots.add(ca.der().clone()).unwrap();
  TlsFixture {
    server_certificate,
    server_key: server_key_path,
    ca_certificate,
    client_certificate_der: client.der().clone(),
    client_key_der: client_key.serialize_der(),
    roots,
  }
}

fn post(address: SocketAddr, tls: &TlsFixture, route: &str, body: Value) -> (u16, Value) {
  let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(tls.client_key_der.clone()));
  let config = ClientConfig::builder()
    .with_root_certificates(tls.roots.clone())
    .with_client_auth_cert(vec![tls.client_certificate_der.clone()], key)
    .unwrap();
  let connection = ClientConnection::new(
    Arc::new(config),
    ServerName::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST).into()),
  )
  .unwrap();
  let tcp = TcpStream::connect(address).unwrap();
  let mut stream = StreamOwned::new(connection, tcp);
  let body = serde_json::to_vec(&body).unwrap();
  let mut request = format!(
    "POST {route} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
    body.len()
  )
  .into_bytes();
  request.extend_from_slice(&body);
  stream.write_all(&request).unwrap();
  if let Err(error) = stream.flush() {
    // Some TLS/socket implementations report the peer's orderly HTTP close as
    // a failed flush even though the complete request was accepted and the
    // response is available to read. Preserve every other transport failure.
    if !matches!(
      error.kind(),
      std::io::ErrorKind::BrokenPipe
        | std::io::ErrorKind::ConnectionAborted
        | std::io::ErrorKind::ConnectionReset
    ) {
      panic!("loopback request flush failed: {error}");
    }
  }
  let mut response = Vec::new();
  let mut buffer = [0u8; 4096];
  loop {
    match stream.read(&mut buffer) {
      Ok(0) => break,
      Ok(read) => response.extend_from_slice(&buffer[..read]),
      Err(error)
        if matches!(
          error.kind(),
          std::io::ErrorKind::UnexpectedEof
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
        ) && !response.is_empty() =>
      {
        break;
      }
      Err(error) => panic!("loopback response failed: {error}"),
    }
  }
  let split = response
    .windows(4)
    .position(|window| window == b"\r\n\r\n")
    .unwrap();
  let header = std::str::from_utf8(&response[..split]).unwrap();
  let status = header
    .split_whitespace()
    .nth(1)
    .unwrap()
    .parse::<u16>()
    .unwrap();
  let value = serde_json::from_slice(&response[split + 4..]).unwrap();
  (status, value)
}

fn setup_store(path: &Path) -> (String, CompiledWorkflowDefinition) {
  let workflow: CompiledWorkflowDefinition = serde_json::from_str(MODEL).unwrap();
  let mut store = DurableEventStore::open(path).unwrap();
  store
    .register_definition(&workflow, DEFINITION_HASH)
    .unwrap();
  let run_id = store
    .admit_trigger_occurrence(TriggerAdmissionRequest {
      workflow_id: workflow.workflow_id.clone(),
      definition_hash: DEFINITION_HASH.to_string(),
      trigger_id: "start".to_string(),
      trigger_handler: "trigger.manual".to_string(),
      source_identity: "remote-loopback-test".to_string(),
      payload: Map::new(),
      received_at: Utc::now(),
    })
    .unwrap()
    .run_id;
  (run_id, workflow)
}

fn package(
  run_id: &str,
  workflow: &CompiledWorkflowDefinition,
  target_digest: &str,
) -> RemoteDispatchPackageV1 {
  let now = Utc::now();
  let mut package = RemoteDispatchPackageV1 {
    profile: "woml.dispatch-package/v1".to_string(),
    version: 1,
    dispatch_id: String::new(),
    run_id: run_id.to_string(),
    node_id: "applyRelease".to_string(),
    iteration_key: None,
    attempt: 1,
    invocation_id: format!("inv_{}", Uuid::new_v4().simple()),
    target: "production-private".to_string(),
    target_registry_digest: target_digest.to_string(),
    infrastructure_policy_digest: Some(POLICY_DIGEST.to_string()),
    definition_package_root_hash: DEFINITION_HASH.to_string(),
    node: workflow.node("applyRelease").unwrap().clone(),
    context: woml_engine::WorkflowContext {
      trigger: Map::new(),
      steps: Map::new(),
      lock: None,
    },
    required_capabilities: vec!["runtime.script".to_string()],
    required_secrets: vec![],
    artifact_grants: vec![],
    deadline: now + Duration::minutes(5),
    package_digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
      .to_string(),
  };
  package.dispatch_id = derive_remote_dispatch_id(&RemoteDispatchRequestV1 {
    run_id: package.run_id.clone(),
    node_id: package.node_id.clone(),
    iteration_identity: String::new(),
    attempt: package.attempt,
    invocation_id: package.invocation_id.clone(),
    target: package.target.clone(),
    target_digest: package.target_registry_digest.clone(),
    package_digest: package.package_digest.clone(),
    requested_at: now,
  })
  .unwrap();
  package.package_digest = package.computed_digest().unwrap();
  package
}

fn base_message(
  message_type: &str,
  session_id: &str,
  dispatch_id: &str,
) -> serde_json::Map<String, Value> {
  serde_json::Map::from_iter([
    ("profile".to_string(), json!("woml.worker-protocol/v1")),
    ("protocolVersion".to_string(), json!(1)),
    (
      "messageId".to_string(),
      json!(format!("{message_type}_{}", Uuid::new_v4().simple())),
    ),
    ("messageType".to_string(), json!(message_type)),
    ("workerId".to_string(), json!("production-runner-01")),
    ("sessionId".to_string(), json!(session_id)),
    ("sentAt".to_string(), json!(Utc::now())),
    ("target".to_string(), json!("production-private")),
    ("dispatchId".to_string(), json!(dispatch_id)),
  ])
}

#[test]
fn mtls_loopback_dispatches_and_commits_authenticated_worker_facts() {
  let temporary = TemporaryDirectory::new();
  let tls = tls_fixture(temporary.path());
  let database = temporary.path().join("state.sqlite");
  let (run_id, workflow) = setup_store(&database);
  let registry = prepare_target_registry(TargetRegistryV1 {
    profile: "woml.target-registry/v1".to_string(),
    version: 1,
    targets: vec![TargetRegistryEntryV1 {
      id: "production-private".to_string(),
      kind: "worker-pool".to_string(),
      allowed_worker_identities: vec!["production-runner-01".to_string()],
      required_capabilities: vec!["runtime.script".to_string()],
      minimum_worker_version: "1.0.10".to_string(),
      maximum_in_flight: 1,
    }],
  })
  .unwrap();
  let certificate_digest = format!(
    "sha256:{:x}",
    Sha256::digest(tls.client_certificate_der.as_ref())
  );
  let authority = RemoteSessionAuthorityV1::new_dispatching(
    registry.clone(),
    [TrustedWorkerIdentityV1 {
      worker_id: "production-runner-01".to_string(),
      certificate_digest,
    }],
    [("runtime.script".to_string(), 1)],
  )
  .unwrap();
  let transport = RemoteDispatchTransportV1::new(&database);
  let dispatch = package(&run_id, &workflow, &registry.digest);
  transport
    .enqueue(
      dispatch.clone(),
      RemoteCoordinatorAuthorityV1 {
        policy_digest: POLICY_DIGEST.to_string(),
        allowed_capabilities: vec![WorkerCapabilityV1 {
          id: "runtime.script".to_string(),
          version: 1,
        }],
        allowed_secret_names: vec![],
      },
      Utc::now(),
    )
    .unwrap();
  let server = RemoteCoordinatorServerV1::start_dispatching(
    authority,
    transport.clone(),
    RemoteCoordinatorTlsOptionsV1 {
      bind_address: "127.0.0.1:0".parse().unwrap(),
      certificate_path: tls.server_certificate.clone(),
      private_key_path: tls.server_key.clone(),
      client_ca_path: tls.ca_certificate.clone(),
    },
  )
  .unwrap();
  let address = server.local_address();
  let (_, session) = post(
    address,
    &tls,
    "/woml/v1/workers/session",
    json!({
      "profile": "woml.worker-protocol/v1",
      "protocolVersion": 1,
      "messageId": format!("hello_{}", Uuid::new_v4().simple()),
      "messageType": "hello",
      "workerId": "production-runner-01",
      "sentAt": Utc::now(),
      "workerVersion": "1.0.10",
      "platform": {"os": "linux", "arch": "x64"},
      "targets": ["production-private"],
      "capacity": 1,
      "capabilities": [{"id": "runtime.script", "version": 1}]
    }),
  );
  assert_eq!(session["dispatchEnabled"], true);
  let session_id = session["sessionId"].as_str().unwrap();
  let (status, poll) = post(
    address,
    &tls,
    "/woml/v1/workers/poll",
    json!({
      "profile": "woml.worker-protocol/v1",
      "protocolVersion": 1,
      "messageId": format!("poll_{}", Uuid::new_v4().simple()),
      "messageType": "poll",
      "workerId": "production-runner-01",
      "sessionId": session_id,
      "sentAt": Utc::now(),
      "target": "production-private",
      "dispatchId": "none"
    }),
  );
  assert_eq!(status, 200);
  assert_eq!(poll["status"], "offered");
  assert_eq!(
    poll["dispatch"]["dispatch"]["dispatchId"],
    dispatch.dispatch_id
  );

  for message_type in ["accepted", "started"] {
    let (_, ack) = post(
      address,
      &tls,
      &format!("/woml/v1/workers/{message_type}"),
      Value::Object(base_message(
        message_type,
        session_id,
        &dispatch.dispatch_id,
      )),
    );
    assert_eq!(ack["status"], message_type);
  }
  let mut terminal = base_message("terminal", session_id, &dispatch.dispatch_id);
  terminal.insert("outcome".to_string(), json!("succeeded"));
  terminal.insert("result".to_string(), json!({"remote": true}));
  terminal.insert("finalOutputSequence".to_string(), json!(0));
  terminal.insert(
    "journalDigest".to_string(),
    json!("sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"),
  );
  let (_, receipt) = post(
    address,
    &tls,
    "/woml/v1/workers/terminal",
    Value::Object(terminal),
  );
  assert_eq!(receipt["state"], "succeeded");
  assert_eq!(
    DurableEventStore::open(&database)
      .unwrap()
      .remote_dispatch_record(&dispatch.dispatch_id)
      .unwrap()
      .state,
    RemoteDispatchStateV1::Succeeded
  );
  server.shutdown();
}
