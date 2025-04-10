use chrono::{
    DateTime, Datelike, Days, Months, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc,
};
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
    pub fn year_month_day(year: i32, month: u32, day: u32) -> Option<Self> {
        match (
            NaiveDate::from_ymd_opt(year, month, day),
            NaiveTime::from_hms_opt(0, 0, 0),
        ) {
            (Some(d), Some(t)) => {
                let dt = d.and_time(t);
                Some(Self::new(Utc.from_utc_datetime(&dt)))
            }
            _ => None,
        }
    }
    pub fn now() -> Self {
        Self::new(DateTime::<Utc>::from(SystemTime::now()))
    }
    pub fn from_millis(millis: i64) -> Option<Self> {
        Utc.timestamp_millis_opt(millis).earliest().map(Self::new)
    }
    pub fn from_seconds(seconds: i64) -> Option<Self> {
        Self::from_millis(seconds * 1_000)
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
    pub fn parse_from_rfc3339_loose(date: &str) -> Option<Self> {
        // Try as actual RFC3339
        // 2024-10-26T14:38:11Z
        if let Some(date) = Self::parse_from_rfc3339(date) {
            return Some(date);
        }
        // Try chopping off most of the date and just putting a Z
        // 2024-10-26T14:38:11
        if date.len() >= 19 {
            if let Some(date) = Self::parse_from_rfc3339(&format!("{}Z", &date[..19])) {
                return Some(date);
            }
            // Try combining the first and second parts with a T and Z
            if let Some(date) =
                Self::parse_from_rfc3339(&format!("{}T{}Z", &date[..10], &date[11..19]))
            {
                return Some(date);
            }
        }
        // Try combining the first part with midnight
        if let Some(date) = Self::parse_from_rfc3339(&format!("{}T00:00:00Z", &date[..10])) {
            return Some(date);
        }

        return None;
    }
    pub fn to_rfc3339(&self) -> String {
        self.internal_date.to_rfc3339()
    }
    pub fn to_rfc2822(&self) -> String {
        self.internal_date.to_rfc2822()
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
    pub fn checked_add_days(&self, days: u64) -> Option<Self> {
        self.internal_date
            .checked_add_days(Days::new(days))
            .map(StoryDate::new)
    }
    pub fn checked_sub_days(&self, days: u64) -> Option<Self> {
        self.internal_date
            .checked_sub_days(Days::new(days))
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

macro_rules! duration_unit {
    ($unit:ident, $num_unit:ident, $num_unit_f32:ident) => {
        #[inline(always)]
        #[allow(dead_code)]
        pub fn $unit($unit: i64) -> Self {
            Self {
                duration: chrono::Duration::$unit($unit),
            }
        }

        #[inline(always)]
        #[allow(dead_code)]
        pub fn $num_unit(&self) -> i64 {
            self.duration.$num_unit()
        }

        #[inline(always)]
        #[allow(dead_code)]
        pub fn $num_unit_f32(&self) -> f32 {
            self.duration.num_milliseconds() as f32
                / Self::$unit(1).duration.num_milliseconds() as f32
        }
    };
}

impl StoryDuration {
    duration_unit!(days, num_days, num_days_f32);
    duration_unit!(hours, num_hours, num_hours_f32);
    duration_unit!(minutes, num_minutes, num_minutes_f32);
    duration_unit!(seconds, num_seconds, num_seconds_f32);
    duration_unit!(milliseconds, num_milliseconds, num_milliseconds_f32);
}

impl Sub for StoryDuration {
    type Output = <chrono::Duration as Sub>::Output;

    fn sub(self, rhs: Self) -> Self::Output {
        self.duration - rhs.duration
    }
}

#[cfg(test)]
mod test {
    use crate::StoryDate;

    #[test]
    fn test_serialize() {
        let date = StoryDate::year_month_day(2000, 1, 1).expect("Date is valid");
        let json = serde_json::to_string(&date).expect("Serialize");
        let date2 = serde_json::from_str::<StoryDate>(&json).expect("Deserialize");
        assert_eq!(date, date2);

        let date_from_seconds = str::parse::<i64>(&json).expect("Parse");
        assert_eq!(
            date,
            StoryDate::from_seconds(date_from_seconds).expect("From seconds")
        );
    }

    #[test]
    fn test_parse_from_rfc3339_loose() {
        let actual_date = StoryDate::parse_from_rfc3339("2024-10-26T14:38:11Z").unwrap();

        for variations in [
            "2024-10-26T14:38:11",
            "2024-10-26T14:38:11ZZ",
            "2024-10-26 14:38:11",
        ] {
            assert_eq!(
                StoryDate::parse_from_rfc3339_loose(&variations),
                Some(actual_date),
                "{variations}"
            );
        }
    }
}
