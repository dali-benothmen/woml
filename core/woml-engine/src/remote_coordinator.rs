//! Authenticated RET3 worker-session authority.
//!
//! The TLS adapter supplies the verified leaf-certificate digest. This module
//! binds it to the operator registry and owns replay, compatibility, capacity,
//! target-membership, renewal, and shutdown checks. It cannot return work.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

use crate::{
  remote_protocol::{
    WorkerHelloV1, WorkerPlatformV1, WorkerPollResultV1, WorkerPollV1, WorkerSessionV1,
    WorkerShutdownV1, WORKER_PROTOCOL_PROFILE, WORKER_PROTOCOL_VERSION,
  },
  PreparedTargetRegistryV1,
};

pub const REMOTE_SESSION_LIFETIME_SECONDS: i64 = 60;
pub const REMOTE_MESSAGE_CLOCK_SKEW_SECONDS: i64 = 300;
pub const REMOTE_MAX_SESSIONS: usize = 1024;
pub const REMOTE_MAX_REPLAY_IDS: usize = 100_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedWorkerIdentityV1 {
  pub worker_id: String,
  pub certificate_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSessionRecordV1 {
  pub session_id: String,
  pub worker_id: String,
  pub certificate_digest: String,
  pub worker_version: String,
  pub platform: WorkerPlatformV1,
  pub targets: BTreeSet<String>,
  pub capacity: u32,
  pub active_dispatches: u32,
  pub capabilities: BTreeMap<String, u32>,
  pub created_at: DateTime<Utc>,
  pub last_message_at: DateTime<Utc>,
  pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteWorkerHealthStatusV1 {
  Eligible,
  Saturated,
  Draining,
  Offline,
  Incompatible,
  Unauthorized,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteWorkerHealthV1 {
  pub profile: &'static str,
  pub target: String,
  pub worker_id: String,
  pub status: RemoteWorkerHealthStatusV1,
  pub active_dispatches: u32,
  pub capacity: u32,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub last_heartbeat_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RemoteCoordinatorError {
  #[error("the authenticated worker is not authorized")]
  Unauthorized,
  #[error("the worker is incompatible with the target")]
  Incompatible,
  #[error("the worker message was replayed or conflicts with its session")]
  ReplayOrConflict,
  #[error("the worker session is unavailable")]
  Unavailable,
  #[error("the worker session limit was reached")]
  Capacity,
}

impl RemoteCoordinatorError {
  pub const fn code(&self) -> &'static str {
    match self {
      Self::Unauthorized => "WOML_TARGET_UNAUTHORIZED",
      Self::Incompatible => "WOML_TARGET_INCOMPATIBLE",
      Self::ReplayOrConflict => "WOML_REMOTE_PROTOCOL_INVALID",
      Self::Unavailable | Self::Capacity => "WOML_TARGET_UNAVAILABLE",
    }
  }
}

#[derive(Debug)]
pub struct RemoteSessionAuthorityV1 {
  registry: PreparedTargetRegistryV1,
  trust: HashMap<String, String>,
  supported_capabilities: BTreeMap<String, u32>,
  sessions: HashMap<String, RemoteSessionRecordV1>,
  replay_ids: HashMap<String, DateTime<Utc>>,
  draining_targets: BTreeSet<String>,
  draining_workers: BTreeSet<String>,
}

impl RemoteSessionAuthorityV1 {
  pub fn new(
    registry: PreparedTargetRegistryV1,
    trust: impl IntoIterator<Item = TrustedWorkerIdentityV1>,
    supported_capabilities: impl IntoIterator<Item = (String, u32)>,
  ) -> Result<Self, RemoteCoordinatorError> {
    let mut trusted = HashMap::new();
    for identity in trust {
      if trusted
        .insert(identity.worker_id, identity.certificate_digest)
        .is_some()
      {
        return Err(RemoteCoordinatorError::ReplayOrConflict);
      }
    }
    Ok(Self {
      registry,
      trust: trusted,
      supported_capabilities: supported_capabilities.into_iter().collect(),
      sessions: HashMap::new(),
      replay_ids: HashMap::new(),
      draining_targets: BTreeSet::new(),
      draining_workers: BTreeSet::new(),
    })
  }

  pub fn establish(
    &mut self,
    hello: WorkerHelloV1,
    authenticated_certificate_digest: &str,
    now: DateTime<Utc>,
  ) -> Result<WorkerSessionV1, RemoteCoordinatorError> {
    self.expire(now);
    if self.sessions.len() >= REMOTE_MAX_SESSIONS {
      return Err(RemoteCoordinatorError::Capacity);
    }
    let trusted_digest = self
      .trust
      .get(&hello.worker_id)
      .ok_or(RemoteCoordinatorError::Unauthorized)?;
    if trusted_digest != authenticated_certificate_digest {
      return Err(RemoteCoordinatorError::Unauthorized);
    }
    self.accept_message(&hello.message_id, hello.sent_at, now)?;
    let capabilities = hello
      .capabilities
      .iter()
      .map(|capability| (capability.id.clone(), capability.version))
      .collect::<BTreeMap<_, _>>();
    for (capability, version) in &capabilities {
      if self.supported_capabilities.get(capability) != Some(version) {
        return Err(RemoteCoordinatorError::Incompatible);
      }
    }
    let mut maximum_capacity = u32::MAX;
    for target_id in &hello.targets {
      let target = self
        .registry
        .registry
        .targets
        .iter()
        .find(|target| &target.id == target_id)
        .ok_or(RemoteCoordinatorError::Unauthorized)?;
      if !target.allowed_worker_identities.contains(&hello.worker_id)
        || compare_semver(&hello.worker_version, &target.minimum_worker_version).is_lt()
        || target
          .required_capabilities
          .iter()
          .any(|required| !capabilities.contains_key(required))
      {
        return Err(RemoteCoordinatorError::Incompatible);
      }
      maximum_capacity = maximum_capacity.min(target.maximum_in_flight);
    }
    if hello.capacity > maximum_capacity {
      return Err(RemoteCoordinatorError::Capacity);
    }
    let session_id = format!("session_{}", Uuid::new_v4().simple());
    let expires_at = now + Duration::seconds(REMOTE_SESSION_LIFETIME_SECONDS);
    let record = RemoteSessionRecordV1 {
      session_id: session_id.clone(),
      worker_id: hello.worker_id.clone(),
      certificate_digest: authenticated_certificate_digest.to_string(),
      worker_version: hello.worker_version,
      platform: hello.platform,
      targets: hello.targets.iter().cloned().collect(),
      capacity: hello.capacity,
      active_dispatches: 0,
      capabilities,
      created_at: now,
      last_message_at: hello.sent_at,
      expires_at,
    };
    self.sessions.insert(session_id.clone(), record);
    Ok(WorkerSessionV1 {
      profile: "woml.worker-session/v1",
      protocol_version: WORKER_PROTOCOL_VERSION,
      session_id,
      worker_id: hello.worker_id,
      targets: hello.targets,
      expires_at,
      dispatch_enabled: false,
    })
  }

  pub fn poll(
    &mut self,
    poll: WorkerPollV1,
    authenticated_certificate_digest: &str,
    now: DateTime<Utc>,
  ) -> Result<WorkerPollResultV1, RemoteCoordinatorError> {
    self.expire(now);
    let session = self
      .sessions
      .get(&poll.session_id)
      .ok_or(RemoteCoordinatorError::Unavailable)?;
    if session.worker_id != poll.worker_id
      || session.certificate_digest != authenticated_certificate_digest
      || !session.targets.contains(&poll.target)
    {
      return Err(RemoteCoordinatorError::Unauthorized);
    }
    if poll.sent_at < session.last_message_at {
      return Err(RemoteCoordinatorError::ReplayOrConflict);
    }
    self.accept_message(&poll.message_id, poll.sent_at, now)?;
    let session = self
      .sessions
      .get_mut(&poll.session_id)
      .ok_or(RemoteCoordinatorError::Unavailable)?;
    session.last_message_at = poll.sent_at;
    session.expires_at = now + Duration::seconds(REMOTE_SESSION_LIFETIME_SECONDS);
    let draining = self.draining_workers.contains(&poll.worker_id)
      || self.draining_targets.contains(&poll.target);
    Ok(WorkerPollResultV1 {
      profile: "woml.worker-poll-result/v1",
      session_id: session.session_id.clone(),
      status: if draining { "draining" } else { "idle" },
      dispatch: None,
      expires_at: session.expires_at,
    })
  }

  pub fn shutdown(
    &mut self,
    message: WorkerShutdownV1,
    authenticated_certificate_digest: &str,
    now: DateTime<Utc>,
  ) -> Result<(), RemoteCoordinatorError> {
    let session = self
      .sessions
      .get(&message.session_id)
      .ok_or(RemoteCoordinatorError::Unavailable)?;
    if session.worker_id != message.worker_id
      || session.certificate_digest != authenticated_certificate_digest
    {
      return Err(RemoteCoordinatorError::Unauthorized);
    }
    if message.sent_at < session.last_message_at {
      return Err(RemoteCoordinatorError::ReplayOrConflict);
    }
    self.accept_message(&message.message_id, message.sent_at, now)?;
    self.sessions.remove(&message.session_id);
    Ok(())
  }

  pub fn inspect(&self, session_id: &str) -> Option<&RemoteSessionRecordV1> {
    self.sessions.get(session_id)
  }

  pub fn begin_drain(&mut self, target_or_worker: &str) -> Result<(), RemoteCoordinatorError> {
    if self
      .registry
      .registry
      .targets
      .iter()
      .any(|target| target.id == target_or_worker)
    {
      self.draining_targets.insert(target_or_worker.to_string());
      return Ok(());
    }
    if self.trust.contains_key(target_or_worker) {
      self.draining_workers.insert(target_or_worker.to_string());
      return Ok(());
    }
    Err(RemoteCoordinatorError::Unauthorized)
  }

  pub fn end_drain(&mut self, target_or_worker: &str) -> Result<(), RemoteCoordinatorError> {
    if self.draining_targets.remove(target_or_worker)
      || self.draining_workers.remove(target_or_worker)
    {
      Ok(())
    } else {
      Err(RemoteCoordinatorError::Unavailable)
    }
  }

  pub fn reserve_dispatch(
    &mut self,
    session_id: &str,
    target: &str,
  ) -> Result<(), RemoteCoordinatorError> {
    let session = self
      .sessions
      .get_mut(session_id)
      .ok_or(RemoteCoordinatorError::Unavailable)?;
    if !session.targets.contains(target)
      || self.draining_targets.contains(target)
      || self.draining_workers.contains(&session.worker_id)
    {
      return Err(RemoteCoordinatorError::Unavailable);
    }
    if session.active_dispatches >= session.capacity {
      return Err(RemoteCoordinatorError::Capacity);
    }
    session.active_dispatches += 1;
    Ok(())
  }

  pub fn release_dispatch(&mut self, session_id: &str) -> Result<(), RemoteCoordinatorError> {
    let session = self
      .sessions
      .get_mut(session_id)
      .ok_or(RemoteCoordinatorError::Unavailable)?;
    session.active_dispatches = session.active_dispatches.saturating_sub(1);
    Ok(())
  }

  pub fn target_health(
    &self,
    target_id: &str,
  ) -> Result<Vec<RemoteWorkerHealthV1>, RemoteCoordinatorError> {
    let target = self
      .registry
      .registry
      .targets
      .iter()
      .find(|target| target.id == target_id)
      .ok_or(RemoteCoordinatorError::Unavailable)?;
    Ok(
      target
        .allowed_worker_identities
        .iter()
        .map(|worker_id| {
          let session = self
            .sessions
            .values()
            .find(|session| session.worker_id == *worker_id && session.targets.contains(target_id));
          let (status, active_dispatches, capacity, last_heartbeat_at) =
            if !self.trust.contains_key(worker_id) {
              (RemoteWorkerHealthStatusV1::Unauthorized, 0, 0, None)
            } else if let Some(session) = session {
              let status = if self.draining_targets.contains(target_id)
                || self.draining_workers.contains(worker_id)
              {
                RemoteWorkerHealthStatusV1::Draining
              } else if session.active_dispatches >= session.capacity {
                RemoteWorkerHealthStatusV1::Saturated
              } else {
                RemoteWorkerHealthStatusV1::Eligible
              };
              (
                status,
                session.active_dispatches,
                session.capacity,
                Some(session.last_message_at),
              )
            } else {
              (RemoteWorkerHealthStatusV1::Offline, 0, 0, None)
            };
          RemoteWorkerHealthV1 {
            profile: "woml.remote-worker-health/v1",
            target: target_id.to_string(),
            worker_id: worker_id.clone(),
            status,
            active_dispatches,
            capacity,
            last_heartbeat_at,
          }
        })
        .collect(),
    )
  }

  fn accept_message(
    &mut self,
    message_id: &str,
    sent_at: DateTime<Utc>,
    now: DateTime<Utc>,
  ) -> Result<(), RemoteCoordinatorError> {
    let oldest = now - Duration::seconds(REMOTE_MESSAGE_CLOCK_SKEW_SECONDS);
    self.replay_ids.retain(|_, seen_at| *seen_at >= oldest);
    if (now - sent_at).num_seconds().unsigned_abs() > REMOTE_MESSAGE_CLOCK_SKEW_SECONDS as u64
      || self.replay_ids.len() >= REMOTE_MAX_REPLAY_IDS
      || self.replay_ids.contains_key(message_id)
    {
      return Err(RemoteCoordinatorError::ReplayOrConflict);
    }
    self.replay_ids.insert(message_id.to_string(), now);
    Ok(())
  }

  fn expire(&mut self, now: DateTime<Utc>) {
    self.sessions.retain(|_, session| session.expires_at > now);
  }
}

fn compare_semver(left: &str, right: &str) -> std::cmp::Ordering {
  fn core(value: &str) -> [u64; 3] {
    let value = value.split_once('-').map_or(value, |(core, _)| core);
    let mut parts = value
      .split('.')
      .map(|part| part.parse::<u64>().unwrap_or(0));
    [
      parts.next().unwrap_or(0),
      parts.next().unwrap_or(0),
      parts.next().unwrap_or(0),
    ]
  }
  core(left).cmp(&core(right))
}

pub fn remote_transport_profile() -> (&'static str, u32) {
  (WORKER_PROTOCOL_PROFILE, WORKER_PROTOCOL_VERSION)
}
