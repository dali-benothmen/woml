//! Private bridge remote.

use chrono::Utc;
use woml_engine::prepare_remote_dispatch;
use woml_engine::PreparedRemoteDispatchV1;
use woml_engine::RemoteCoordinatorAuthorityV1;
use woml_engine::RemoteDispatchPackageV1;
use woml_engine::RemoteWorkerAuthorityV1;

pub(super) fn native_prepared_remote_dispatch(
  dispatch_package_json: &str,
  coordinator_authority_json: &str,
  worker_authority_json: &str,
  worker_id: &str,
  target: &str,
) -> napi::Result<PreparedRemoteDispatchV1> {
  let package: RemoteDispatchPackageV1 =
    serde_json::from_str(dispatch_package_json).map_err(|_| {
      napi::Error::from_reason("WOML_REMOTE_PROTOCOL_INVALID: invalid dispatch package")
    })?;
  let coordinator: RemoteCoordinatorAuthorityV1 = serde_json::from_str(coordinator_authority_json)
    .map_err(|_| {
      napi::Error::from_reason("WOML_REMOTE_PROTOCOL_INVALID: invalid coordinator authority")
    })?;
  let worker: RemoteWorkerAuthorityV1 =
    serde_json::from_str(worker_authority_json).map_err(|_| {
      napi::Error::from_reason("WOML_REMOTE_PROTOCOL_INVALID: invalid worker authority")
    })?;
  prepare_remote_dispatch(
    package,
    &coordinator,
    worker_id,
    target,
    &worker,
    Utc::now(),
  )
  .map_err(|error| napi::Error::from_reason(format!("{}: {error}", error.code())))
}
