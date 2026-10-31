//! Original artifact identifiers, domain-separated canonical digests, and timestamp encoding.

use crate::artifacts::artifact_path_invalid;
use crate::artifacts::backend_unavailable;
use crate::CapabilityFailure;
use chrono::SecondsFormat;
use chrono::Utc;
use serde_json::Value;
use serde_json_canonicalizer::to_vec as canonical_json;
use sha2::Digest;
use sha2::Sha256;

pub(in crate::artifacts) const CROCKFORD: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

pub(in crate::artifacts) fn new_artifact_id() -> Result<String, CapabilityFailure> {
  let mut bytes = [0_u8; 16];
  getrandom::getrandom(&mut bytes).map_err(|_| backend_unavailable())?;
  let mut encoded = String::with_capacity(26);
  for group in 0..26 {
    let mut value = 0_u8;
    for offset in 0..5 {
      let bit = group * 5 + offset;
      value <<= 1;
      if bit >= 2 {
        let source = bit - 2;
        value |= (bytes[source / 8] >> (7 - source % 8)) & 1;
      }
    }
    encoded.push(CROCKFORD[value as usize] as char);
  }
  Ok(format!("artifact_{encoded}"))
}

pub(in crate::artifacts) fn canonical_digest(
  domain: &[u8],
  value: &Value,
) -> Result<String, CapabilityFailure> {
  let encoded = canonical_json(value).map_err(|_| artifact_path_invalid())?;
  let mut digest = Sha256::new();
  digest.update(domain);
  digest.update(encoded);
  Ok(format!("sha256:{}", hex::encode(digest.finalize())))
}

pub(in crate::artifacts) fn timestamp() -> String {
  Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}
