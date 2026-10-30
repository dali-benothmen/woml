//! runtime clock.

pub trait EngineClock: Send + Sync {
  fn now(&self) -> chrono::DateTime<chrono::Utc>;
}

#[derive(Debug, Default)]
pub struct SystemEngineClock;

impl EngineClock for SystemEngineClock {
  fn now(&self) -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now()
  }
}

#[derive(Debug, Clone)]
pub struct FixedEngineClock {
  now: chrono::DateTime<chrono::Utc>,
}

impl FixedEngineClock {
  pub const fn new(now: chrono::DateTime<chrono::Utc>) -> Self {
    Self { now }
  }
}

impl EngineClock for FixedEngineClock {
  fn now(&self) -> chrono::DateTime<chrono::Utc> {
    self.now
  }
}
