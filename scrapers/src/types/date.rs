use chrono::{DateTime, Datelike, Duration, Months, NaiveDateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::{fmt::Display, ops::Sub, time::SystemTime};

/// Story-specific date that wraps all of the operations we're interested in. This is a thin wrapper on top
/// of `DateTime<Utc>` and other `chrono` utilities for now.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StoryDate {
    internal_date: DateTime<Utc>,
}

impl StoryDate {
    pub const MAX: StoryDate = Self::new(DateTime::<Utc>::MAX_UTC);
    pub const MIN: StoryDate = Self::new(DateTime::<Utc>::MIN_UTC);

    pub const fn new(internal_date: DateTime<Utc>) -> Self {
        Self { internal_date }
    }
    pub fn now() -> Self {
        Self::new(DateTime::<Utc>::from(SystemTime::now()))
    }
    pub fn from_millis(millis: i64) -> Option<Self> {
        Utc.timestamp_millis_opt(millis).earliest().map(Self::new)
    }
    pub fn from_string(date: &str, s: &str) -> Option<Self> {
        let date = NaiveDateTime::parse_from_str(date, s).ok();
        date.map(|x| Self::new(Utc.from_utc_datetime(&x)))
    }
    pub fn parse_from_rfc3339(date: &str) -> Option<Self> {
        DateTime::parse_from_rfc3339(date)
            .ok()
            .map(|x| Self::new(x.into()))
    }
    pub fn parse_from_rfc2822(date: &str) -> Option<Self> {
        DateTime::parse_from_rfc2822(date)
            .ok()
            .map(|x| Self::new(x.into()))
    }
    pub fn year(&self) -> i32 {
        self.internal_date.year()
    }
    pub fn month(&self) -> u32 {
        self.internal_date.month()
    }
    pub fn month0(&self) -> u32 {
        self.internal_date.month0()
    }
    pub fn day(&self) -> u32 {
        self.internal_date.day()
    }
    pub fn day0(&self) -> u32 {
        self.internal_date.day0()
    }
    pub fn timestamp(&self) -> i64 {
        self.internal_date.timestamp()
    }
    pub fn checked_add_months(&self, months: u32) -> Option<Self> {
        self.internal_date
            .checked_add_months(Months::new(months))
            .map(StoryDate::new)
    }
    pub fn checked_sub_months(&self, months: u32) -> Option<Self> {
        self.internal_date
            .checked_sub_months(Months::new(months))
            .map(StoryDate::new)
    }
}

impl Sub for StoryDate {
    type Output = StoryDuration;
    fn sub(self, rhs: Self) -> Self::Output {
        StoryDuration {
            duration: self.internal_date - rhs.internal_date,
        }
    }
}

impl Serialize for StoryDate {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        chrono::serde::ts_seconds::serialize(&self.internal_date, serializer)
    }
}

impl<'de> Deserialize<'de> for StoryDate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        chrono::serde::ts_seconds::deserialize(deserializer).map(Self::new)
    }
}

impl Display for StoryDate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.internal_date.fmt(f)
    }
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct StoryDuration {
    duration: chrono::Duration,
}

impl StoryDuration {
    pub fn days(days: i64) -> Self {
        Self {
            duration: chrono::Duration::days(days),
        }
    }

    pub fn hours(hours: i64) -> Self {
        Self {
            duration: chrono::Duration::hours(hours),
        }
    }

    pub fn num_milliseconds(&self) -> i64 {
        self.duration.num_milliseconds()
    }

    pub fn num_hours(&self) -> i64 {
        self.duration.num_hours()
    }
}

impl Sub for StoryDuration {
    type Output = <chrono::Duration as Sub>::Output;

    fn sub(self, rhs: Self) -> Self::Output {
        self.duration - rhs.duration
    }
}
