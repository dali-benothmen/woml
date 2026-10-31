//! model policies.

use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledRuntimePolicy {
  pub profile_version: u32,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub concurrency: Option<u32>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub timeout_ms: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub rate_limit: Option<CompiledRateLimitPolicy>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub queue: Option<CompiledQueuePolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledRateLimitPolicy {
  pub count: u32,
  pub window_ms: u64,
  pub algorithm: RateLimitAlgorithm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitAlgorithm {
  RollingWindow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledQueuePolicy {
  pub name: String,
  pub discipline: QueueDiscipline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueDiscipline {
  WorkConservingFifo,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetryPolicy {
  pub max_attempts: u32,
  pub backoff: BackoffPolicy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum BackoffPolicy {
  None,
  Fixed {
    #[serde(rename = "delayMs")]
    delay_ms: u64,
  },
  Exponential {
    #[serde(rename = "initialDelayMs")]
    initial_delay_ms: u64,
    multiplier: f64,
    #[serde(rename = "maximumDelayMs")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    maximum_delay_ms: Option<u64>,
  },
}

impl RetryPolicy {
  pub const MAX_ATTEMPTS: u32 = 10;
  pub const MAX_DELAY_MS: u64 = 86_400_000;

  pub fn delay_before_attempt(&self, next_attempt: u32) -> Option<u64> {
    if next_attempt < 2 || next_attempt > self.max_attempts {
      return None;
    }
    match self.backoff {
      BackoffPolicy::None => None,
      BackoffPolicy::Fixed { delay_ms } => Some(delay_ms),
      BackoffPolicy::Exponential {
        initial_delay_ms,
        multiplier,
        maximum_delay_ms,
      } => {
        let exponent = i32::try_from(next_attempt - 2).ok()?;
        let calculated = (initial_delay_ms as f64) * multiplier.powi(exponent);
        let capped = calculated.min(maximum_delay_ms? as f64);
        if !capped.is_finite() || capped < 1.0 || capped > u64::MAX as f64 {
          return None;
        }
        Some(capped as u64)
      }
    }
  }
}
