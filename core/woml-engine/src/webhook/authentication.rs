//! Private webhook authentication.

use actix_web::http::header;
use actix_web::HttpRequest;
use sha2::Digest;
use sha2::Sha256;
use subtle::ConstantTimeEq;

pub(in crate::webhook) enum WebhookAuthentication {
  None,
  Bearer { token_digest: [u8; 32] },
}

pub(in crate::webhook) fn authorized(
  request: &HttpRequest,
  authentication: &WebhookAuthentication,
) -> bool {
  match authentication {
    WebhookAuthentication::None => true,
    WebhookAuthentication::Bearer { token_digest } => authorized_bearer(request, token_digest),
  }
}

pub(in crate::webhook) fn authorized_bearer(
  request: &HttpRequest,
  token_digest: &[u8; 32],
) -> bool {
  let Some(value) = request
    .headers()
    .get(header::AUTHORIZATION)
    .and_then(|value| value.to_str().ok())
  else {
    return false;
  };
  let Some((scheme, presented)) = value.split_once(' ') else {
    return false;
  };
  scheme.eq_ignore_ascii_case("bearer")
    && !presented.is_empty()
    && bearer_token_matches(token_digest, presented)
}

pub(in crate::webhook) fn bearer_token_matches(
  expected_digest: &[u8; 32],
  presented: &str,
) -> bool {
  let presented_digest: [u8; 32] = Sha256::digest(presented.as_bytes()).into();
  bool::from(expected_digest.ct_eq(&presented_digest))
}
