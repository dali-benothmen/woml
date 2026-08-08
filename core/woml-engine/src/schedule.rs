use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Datelike, Duration, Timelike, Utc};
use chrono_tz::Tz;
use serde::Serialize;
use thiserror::Error;

pub const SCHEDULE_PROGRESS_CONTRACT: &str = "woml.schedule-progress";
pub const SCHEDULE_PROGRESS_CONTRACT_VERSION: u32 = 1;
const MAX_SEARCH_MINUTES: i64 = 60 * 24 * 366 * 8;

pub trait ScheduleClock: Send + Sync {
  fn now(&self) -> DateTime<Utc>;

  fn sleep_until(&self, deadline: DateTime<Utc>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

#[derive(Debug, Default)]
pub struct SystemScheduleClock;

impl ScheduleClock for SystemScheduleClock {
  fn now(&self) -> DateTime<Utc> {
    Utc::now()
  }

  fn sleep_until(&self, deadline: DateTime<Utc>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
    let duration = deadline
      .signed_duration_since(Utc::now())
      .to_std()
      .unwrap_or(StdDuration::ZERO);
    Box::pin(tokio::time::sleep(duration))
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleMisfirePolicy {
  Skip,
  RunOnce,
}

impl ScheduleMisfirePolicy {
  pub fn parse(value: &str) -> Result<Self, ScheduleError> {
    match value {
      "skip" => Ok(Self::Skip),
      "run-once" => Ok(Self::RunOnce),
      _ => Err(ScheduleError::InvalidMisfirePolicy(value.to_string())),
    }
  }

  pub const fn as_str(self) -> &'static str {
    match self {
      Self::Skip => "skip",
      Self::RunOnce => "run-once",
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WomlSchedule {
  source: String,
  timezone_name: String,
  timezone: Tz,
  fields: [CronField; 5],
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CronField {
  values: BTreeSet<u32>,
  unrestricted: bool,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ScheduleError {
  #[error("cron must contain exactly five fields separated by one ASCII space")]
  InvalidFieldCount,
  #[error("cron field {field} contains invalid WOML Cron v1 syntax")]
  InvalidFieldSyntax { field: usize },
  #[error("cron field {field} value {value} is outside {minimum}-{maximum}")]
  FieldOutOfBounds {
    field: usize,
    value: u32,
    minimum: u32,
    maximum: u32,
  },
  #[error("cron field {field} has a zero step")]
  ZeroStep { field: usize },
  #[error("cron field {field} contains a wrapping or descending range")]
  DescendingRange { field: usize },
  #[error("timezone {0:?} is not a canonical IANA timezone identifier")]
  InvalidTimezone(String),
  #[error("on-missed policy {0:?} is not supported")]
  InvalidMisfirePolicy(String),
  #[error("no matching schedule instant was found inside the supported search window")]
  SearchExhausted,
  #[error("schedule time exceeds the supported clock range")]
  ClockRange,
}

impl WomlSchedule {
  pub fn parse(cron: &str, timezone: &str) -> Result<Self, ScheduleError> {
    if cron.is_empty()
      || cron.len() > 256
      || cron.contains("  ")
      || cron
        .bytes()
        .any(|byte| byte.is_ascii_whitespace() && byte != b' ')
    {
      return Err(ScheduleError::InvalidFieldCount);
    }
    let parts = cron.split(' ').collect::<Vec<_>>();
    if parts.len() != 5 || parts.iter().any(|part| part.is_empty()) {
      return Err(ScheduleError::InvalidFieldCount);
    }
    let bounds = [(0, 59), (0, 23), (1, 31), (1, 12), (0, 7)];
    let parsed = parts
      .iter()
      .enumerate()
      .map(|(index, value)| parse_field(index, value, bounds[index].0, bounds[index].1))
      .collect::<Result<Vec<_>, _>>()?;
    let timezone_value = canonical_timezone(timezone)?;
    Ok(Self {
      source: cron.to_string(),
      timezone_name: timezone.to_string(),
      timezone: timezone_value,
      fields: parsed.try_into().expect("five parsed cron fields"),
    })
  }

  pub fn cron(&self) -> &str {
    &self.source
  }

  pub fn timezone(&self) -> &str {
    &self.timezone_name
  }

  pub fn matches(&self, instant: DateTime<Utc>) -> bool {
    let local = instant.with_timezone(&self.timezone);
    let minute = local.minute();
    let hour = local.hour();
    let day = local.day();
    let month = local.month();
    let weekday = local.weekday().num_days_from_sunday();
    let minute_matches = self.fields[0].values.contains(&minute);
    let hour_matches = self.fields[1].values.contains(&hour);
    let month_matches = self.fields[3].values.contains(&month);
    let day_of_month_matches = self.fields[2].values.contains(&day);
    let day_of_week_matches = self.fields[4].values.contains(&weekday);
    let day_matches = match (self.fields[2].unrestricted, self.fields[4].unrestricted) {
      (false, false) => day_of_month_matches || day_of_week_matches,
      (false, true) => day_of_month_matches,
      (true, false) => day_of_week_matches,
      (true, true) => true,
    };
    minute_matches && hour_matches && month_matches && day_matches
  }

  pub fn next_at_or_after(&self, instant: DateTime<Utc>) -> Result<DateTime<Utc>, ScheduleError> {
    let mut candidate = floor_minute(instant)?;
    if candidate < instant {
      candidate = add_minute(candidate, 1)?;
    }
    self.search(candidate, 1)
  }

  pub fn next_after(&self, instant: DateTime<Utc>) -> Result<DateTime<Utc>, ScheduleError> {
    self.search(add_minute(floor_minute(instant)?, 1)?, 1)
  }

  pub fn latest_at_or_before(
    &self,
    instant: DateTime<Utc>,
  ) -> Result<DateTime<Utc>, ScheduleError> {
    self.search(floor_minute(instant)?, -1)
  }

  pub fn occurrences_between(
    &self,
    start_inclusive: DateTime<Utc>,
    end_exclusive: DateTime<Utc>,
  ) -> Result<Vec<DateTime<Utc>>, ScheduleError> {
    if end_exclusive <= start_inclusive {
      return Ok(Vec::new());
    }
    let mut occurrence = self.next_at_or_after(start_inclusive)?;
    let mut occurrences = Vec::new();
    while occurrence < end_exclusive {
      occurrences.push(occurrence);
      occurrence = self.next_after(occurrence)?;
    }
    Ok(occurrences)
  }

  fn search(&self, start: DateTime<Utc>, direction: i64) -> Result<DateTime<Utc>, ScheduleError> {
    let mut candidate = start;
    for _ in 0..MAX_SEARCH_MINUTES {
      if self.matches(candidate) {
        return Ok(candidate);
      }
      candidate = add_minute(candidate, direction)?;
    }
    Err(ScheduleError::SearchExhausted)
  }
}

fn canonical_timezone(value: &str) -> Result<Tz, ScheduleError> {
  if value == "UTC" {
    return Ok(chrono_tz::UTC);
  }
  let legacy_prefix = [
    "Africa/Asmera",
    "America/Atka",
    "America/Buenos_Aires",
    "America/Ensenada",
    "America/Fort_Wayne",
    "America/Indianapolis",
    "America/Knox_IN",
    "America/Louisville",
    "America/Porto_Acre",
    "America/Rosario",
    "Asia/Calcutta",
    "Asia/Katmandu",
    "Asia/Rangoon",
    "Asia/Saigon",
    "Australia/ACT",
    "Australia/NSW",
    "Brazil/",
    "Canada/",
    "Chile/",
    "Etc/",
    "Europe/Belfast",
    "Mexico/",
    "SystemV/",
    "US/",
  ];
  if !value.contains('/') || legacy_prefix.iter().any(|prefix| value.starts_with(prefix)) {
    return Err(ScheduleError::InvalidTimezone(value.to_string()));
  }
  Tz::from_str(value).map_err(|_| ScheduleError::InvalidTimezone(value.to_string()))
}

fn parse_field(
  field: usize,
  source: &str,
  minimum: u32,
  maximum: u32,
) -> Result<CronField, ScheduleError> {
  let mut values = BTreeSet::new();
  for item in source.split(',') {
    if item.is_empty() {
      return Err(ScheduleError::InvalidFieldSyntax { field });
    }
    let (base, step) = match item.split_once('/') {
      Some((base, step)) if !base.is_empty() && !step.is_empty() && !step.contains('/') => {
        let step = parse_number(field, step, 1, u32::MAX)?;
        if step == 0 {
          return Err(ScheduleError::ZeroStep { field });
        }
        (base, step)
      }
      Some(_) => return Err(ScheduleError::InvalidFieldSyntax { field }),
      None => (item, 1),
    };
    let (start, end) = if base == "*" {
      (minimum, maximum)
    } else if let Some((start, end)) = base.split_once('-') {
      if start.is_empty() || end.is_empty() || end.contains('-') {
        return Err(ScheduleError::InvalidFieldSyntax { field });
      }
      let start = parse_number(field, start, minimum, maximum)?;
      let end = parse_number(field, end, minimum, maximum)?;
      if start > end {
        return Err(ScheduleError::DescendingRange { field });
      }
      (start, end)
    } else {
      let start = parse_number(field, base, minimum, maximum)?;
      let end = if item.contains('/') { maximum } else { start };
      (start, end)
    };
    let mut value = start;
    while value <= end {
      values.insert(if field == 4 && value == 7 { 0 } else { value });
      let Some(next) = value.checked_add(step) else {
        break;
      };
      value = next;
    }
  }
  if values.is_empty() {
    return Err(ScheduleError::InvalidFieldSyntax { field });
  }
  let every_value = (minimum..=maximum)
    .map(|value| if field == 4 && value == 7 { 0 } else { value })
    .collect::<BTreeSet<_>>();
  Ok(CronField {
    unrestricted: values == every_value,
    values,
  })
}

fn parse_number(
  field: usize,
  source: &str,
  minimum: u32,
  maximum: u32,
) -> Result<u32, ScheduleError> {
  if source.is_empty() || !source.bytes().all(|byte| byte.is_ascii_digit()) {
    return Err(ScheduleError::InvalidFieldSyntax { field });
  }
  let value = source
    .parse::<u32>()
    .map_err(|_| ScheduleError::InvalidFieldSyntax { field })?;
  if value < minimum || value > maximum {
    return Err(ScheduleError::FieldOutOfBounds {
      field,
      value,
      minimum,
      maximum,
    });
  }
  Ok(value)
}

fn floor_minute(instant: DateTime<Utc>) -> Result<DateTime<Utc>, ScheduleError> {
  instant
    .with_second(0)
    .and_then(|value| value.with_nanosecond(0))
    .ok_or(ScheduleError::ClockRange)
}

fn add_minute(instant: DateTime<Utc>, amount: i64) -> Result<DateTime<Utc>, ScheduleError> {
  instant
    .checked_add_signed(Duration::minutes(amount))
    .ok_or(ScheduleError::ClockRange)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleProgressReason {
  Initialized,
  Restarted,
  Advanced,
  MisfireSkipped,
  MisfireRunOnce,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type")]
pub enum ScheduleProgress {
  #[serde(rename = "next_due", rename_all = "camelCase")]
  NextDue {
    contract: &'static str,
    contract_version: u32,
    workflow_id: String,
    trigger_id: String,
    timezone: String,
    next_scheduled_at: DateTime<Utc>,
    reason: ScheduleProgressReason,
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

pub type ScheduleProgressReporter = Arc<dyn Fn(ScheduleProgress) + Send + Sync>;
