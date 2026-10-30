//! Scoped credential generation, expiry, and strict token parsing. Secret-bearing values remain outside inspection and event history.

use crate::durable::DurableStoreError;
use chrono::DateTime;
use chrono::Utc;
use getrandom::getrandom;

const DEFAULT_APPROVAL_CREDENTIAL_LIFETIME_HOURS: i64 = 24;

pub(in crate::durable) fn random_hex(byte_count: usize) -> Result<String, DurableStoreError> {
  let mut bytes = vec![0_u8; byte_count];
  getrandom(&mut bytes).map_err(|_| {
    DurableStoreError::Contract(
      "The operating system could not generate a secure approval credential.".to_string(),
    )
  })?;
  Ok(hex::encode(bytes))
}

pub(in crate::durable) fn approval_credential_expiry(
  issued_at: DateTime<Utc>,
  approval_deadline: Option<DateTime<Utc>>,
) -> Result<DateTime<Utc>, DurableStoreError> {
  let default_expiry = issued_at
    .checked_add_signed(chrono::Duration::hours(
      DEFAULT_APPROVAL_CREDENTIAL_LIFETIME_HOURS,
    ))
    .ok_or_else(|| {
      DurableStoreError::Contract("Approval credential expiry exceeds the clock range.".to_string())
    })?;
  let credential_expires_at =
    approval_deadline.map_or(default_expiry, |deadline| deadline.min(default_expiry));
  if credential_expires_at <= issued_at {
    return Err(DurableStoreError::Contract(
      "Cannot issue an approval credential at or after the request deadline.".to_string(),
    ));
  }
  Ok(credential_expires_at)
}

pub(in crate::durable) fn parse_approval_token(
  token: &str,
) -> Result<(&str, &str), DurableStoreError> {
  let Some(body) = token.strip_prefix("apr_") else {
    return Err(DurableStoreError::InvalidApprovalToken);
  };
  let Some((token_id, secret)) = body.split_once('.') else {
    return Err(DurableStoreError::InvalidApprovalToken);
  };
  if token_id.len() != 32
    || secret.len() != 64
    || token_id.contains('.')
    || secret.contains('.')
    || !token_id.bytes().all(|byte| byte.is_ascii_hexdigit())
    || !secret.bytes().all(|byte| byte.is_ascii_hexdigit())
  {
    return Err(DurableStoreError::InvalidApprovalToken);
  }
  Ok((token_id, secret))
}

pub(in crate::durable) fn parse_notification_capability(
  token: &str,
) -> Result<(&str, &str), DurableStoreError> {
  let Some(token) = token.strip_prefix("ncap_") else {
    return Err(DurableStoreError::InvalidApprovalToken);
  };
  let Some((id, secret)) = token.split_once('.') else {
    return Err(DurableStoreError::InvalidApprovalToken);
  };
  let compact = id.len() == 16 && secret.len() == 32;
  let legacy = id.len() == 32 && secret.len() == 64;
  if (!compact && !legacy)
    || !id
      .bytes()
      .chain(secret.bytes())
      .all(|byte| byte.is_ascii_hexdigit())
  {
    return Err(DurableStoreError::InvalidApprovalToken);
  }
  Ok((id, secret))
}
