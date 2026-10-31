//! Private remote coordinator server tls.

use crate::remote_coordinator_server::RemoteCoordinatorServerError;
use crate::remote_coordinator_server::RemoteCoordinatorTlsOptionsV1;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::PrivateKeyDer;
use rustls::server::WebPkiClientVerifier;
use rustls::RootCertStore;
use rustls::ServerConfig;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::Arc;

pub(in crate::remote_coordinator_server) fn server_configuration(
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

pub(in crate::remote_coordinator_server) fn certificates(
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
