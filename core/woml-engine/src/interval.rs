use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use thiserror::Error;

pub const MIN_INTERVAL_MS: u64 = 1_000;
pub const MAX_INTERVAL_MS: u64 = 30 * 24 * 60 * 60 * 1_000;
pub const INTERVAL_PROGRESS_CONTRACT: &str = "woml.interval-progress";
pub const INTERVAL_PROGRESS_CONTRACT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WomlInterval {
  every_ms: u64,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum IntervalError {
  #[error("interval must be a whole duration from 1s through 30d")]
  InvalidDuration,
  #[error("interval sequence must begin at 1")]
  InvalidSequence,
  #[error("interval time exceeds the supported clock range")]
  ClockRange,
  #[error("no interval occurrence is due at this instant")]
  NoDueOccurrence,
}

impl WomlInterval {
  pub fn new(every_ms: u64) -> Result<Self, IntervalError> {
    if !(MIN_INTERVAL_MS..=MAX_INTERVAL_MS).contains(&every_ms) {
      return Err(IntervalError::InvalidDuration);
    }
    Ok(Self { every_ms })
  }

  pub const fn every_ms(self) -> u64 {
    self.every_ms
  }

  pub fn normalize_anchor(instant: DateTime<Utc>) -> Result<DateTime<Utc>, IntervalError> {
    DateTime::from_timestamp_millis(instant.timestamp_millis()).ok_or(IntervalError::ClockRange)
  }

  pub fn planned_at(
    self,
    anchor: DateTime<Utc>,
    sequence: u64,
  ) -> Result<DateTime<Utc>, IntervalError> {
    if sequence == 0 {
      return Err(IntervalError::InvalidSequence);
    }
    let offset = self
      .every_ms
      .checked_mul(sequence)
      .and_then(|value| i64::try_from(value).ok())
      .ok_or(IntervalError::ClockRange)?;
    anchor
      .checked_add_signed(Duration::milliseconds(offset))
      .ok_or(IntervalError::ClockRange)
  }

  pub fn next_sequence_after(
    self,
    anchor: DateTime<Utc>,
    instant: DateTime<Utc>,
  ) -> Result<u64, IntervalError> {
    let elapsed = instant
      .timestamp_millis()
      .checked_sub(anchor.timestamp_millis())
      .ok_or(IntervalError::ClockRange)?;
    if elapsed < 0 {
      return Ok(1);
    }
    u64::try_from(elapsed)
      .ok()
      .and_then(|elapsed| elapsed.checked_div(self.every_ms))
      .and_then(|sequence| sequence.checked_add(1))
      .ok_or(IntervalError::ClockRange)
  }

  pub fn latest_sequence_at_or_before(
    self,
    anchor: DateTime<Utc>,
    instant: DateTime<Utc>,
  ) -> Result<u64, IntervalError> {
    let elapsed = instant
      .timestamp_millis()
      .checked_sub(anchor.timestamp_millis())
      .ok_or(IntervalError::ClockRange)?;
    if elapsed < i64::try_from(self.every_ms).map_err(|_| IntervalError::ClockRange)? {
      return Err(IntervalError::NoDueOccurrence);
    }
    u64::try_from(elapsed)
      .ok()
      .and_then(|elapsed| elapsed.checked_div(self.every_ms))
      .filter(|sequence| *sequence >= 1)
      .ok_or(IntervalError::ClockRange)
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntervalProgressReason {
  Initialized,
  Restarted,
  Advanced,
  MisfireSkipped,
  MisfireRunOnce,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type")]
pub enum IntervalProgress {
  #[serde(rename = "next_due", rename_all = "camelCase")]
  NextDue {
    contract: &'static str,
    contract_version: u32,
    workflow_id: String,
    trigger_id: String,
    every_ms: u64,
    anchor_at: DateTime<Utc>,
    next_sequence: u64,
    next_scheduled_at: DateTime<Utc>,
    reason: IntervalProgressReason,
    occurred_at: DateTime<Utc>,
  },
  #[serde(rename = "scheduler_error", rename_all = "camelCase")]
  SchedulerError {
    contract: &'static str,
    contract_version: u32,
    workflow_id: String,
    trigger_id: String,
    code: String,
    message: String,
    occurred_at: DateTime<Utc>,
  },
}

pub type IntervalProgressReporter = Arc<dyn Fn(IntervalProgress) + Send + Sync>;
