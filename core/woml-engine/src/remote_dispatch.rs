//! Coordinator-owned durable dispatch authority.
//!
//! Private transaction methods extend the existing DurableEventStore; they do
//! not create a per-feature store or independent worker scheduling authority.

mod contracts;
mod identity;
mod schema;
mod transactions;
mod validation;

pub use contracts::{
  RemoteDispatchError, RemoteDispatchInspectionV1, RemoteDispatchRecordV1, RemoteDispatchRequestV1,
  RemoteDispatchResolutionV1, RemoteDispatchStateV1, RemoteOutputBindingV1,
  REMOTE_DISPATCH_HEARTBEAT_MAX_BYTES, REMOTE_DISPATCH_RECOVERY_BATCH_LIMIT,
};
pub use identity::derive_remote_dispatch_id;
pub(in crate::remote_dispatch) use identity::{
  digest_json, parse_optional_time, parse_time, valid_digest, valid_id,
};
pub(crate) use schema::{validate_remote_dispatch_schema, CREATE_REMOTE_DISPATCH_SCHEMA_V20};
pub(in crate::remote_dispatch) use validation::{
  validate_request, validate_request_against_definition,
};
