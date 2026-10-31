//! Private remote worker journal identity.

use sha2::Digest;
use sha2::Sha256;

pub(in crate::remote_worker_journal) fn sha256(bytes: &[u8]) -> String {
  format!("sha256:{:x}", Sha256::digest(bytes))
}
