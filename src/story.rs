use chrono::{DateTime, Utc, Datelike, Months, TimeZone, NaiveDateTime};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::scrapers::{Scrape, ScrapeData, ScrapeId, ScrapeSource};
use std::{
    collections::{hash_map::DefaultHasher, HashMap},
    hash::{Hash, Hasher}, time::SystemTime,
};

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
        Utc
            .timestamp_millis_opt(millis)
            .earliest().map(Self::new)
    }
    pub fn from_string(date: &str, s: &str) -> Option<Self> {
        let date = NaiveDateTime::parse_from_str(
            date,
            s,
        ).ok();
        date.map(|x| Self::new(Utc.from_utc_datetime(&x)))
    }
    pub fn parse_from_rfc3339(date: &str) -> Option<Self> {
        DateTime::parse_from_rfc3339(date).ok().map(|x| Self::new(x.into()))
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
    pub fn timestamp(&self) -> i64 {
        self.internal_date.timestamp()
    }
    pub fn checked_add_months(&self, months: u32) -> Option<Self> {
        self.internal_date.checked_add_months(Months::new(months)).map(StoryDate::new)
    }
    pub fn checked_sub_months(&self, months: u32) -> Option<Self> {
        self.internal_date.checked_sub_months(Months::new(months)).map(StoryDate::new)
    }
}

impl Serialize for StoryDate {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer {
                chrono::serde::ts_seconds::serialize(&self.internal_date, serializer)
            }
}

impl <'de> Deserialize<'de> for StoryDate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de> {
        chrono::serde::ts_seconds::deserialize(deserializer).map(Self::new)
    }
}

/// Rendered story with all properties hydrated from the underlying scrapes. Extraneous data is removed at this point.
#[derive(Clone, Default, Deserialize, Serialize)]
pub struct StoryRender {
    pub url: String,
    pub domain: String,
    pub title: String,
    pub date: StoryDate,
    pub tags: Vec<String>,
    pub comment_links: HashMap<String, String>,
    pub scrapes: HashMap<String, Scrape>,
}

/// Story scrape w/information from underlying sources.
#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Story {
    pub normalized_url: String,
    pub scrapes: HashMap<ScrapeId, Scrape>,
}

impl Story {
    pub fn new(normalized_url: String, scrape: Scrape) -> Self {
        let id = ScrapeId::new(scrape.source(), scrape.id());
        Self {
            normalized_url,
            scrapes: HashMap::from_iter([(id, scrape)]),
        }
    }

    pub fn normalized_url_hash(&self) -> i64 {
        let mut hasher = DefaultHasher::new();
        self.normalized_url.hash(&mut hasher);
        let url_norm_hash = hasher.finish() as i64;
        return url_norm_hash;
    }

    pub fn merge(&mut self, scrape: Scrape) {
        // This logic can be improved when try_insert is stabilized
        // TODO: We need to merge duplicate scrapes
        let id = ScrapeId::new(scrape.source(), scrape.id());
        self.scrapes.insert(id, scrape);
    }

    pub fn title(&self) -> String {
        self.scrapes
            .values()
            .next()
            .expect("Expected at least one")
            .title()
    }

    pub fn url(&self) -> String {
        self.scrapes
            .values()
            .next()
            .expect("Expected at least one")
            .url()
    }

    pub fn date(&self) -> StoryDate {
        self.scrapes
            .values()
            .next()
            .expect("Expected at least one")
            .date()
    }

    pub fn render(&self) -> StoryRender {
        StoryRender {
            url: self.url(),
            domain: Url::parse(&self.url()).ok().and_then(|u| u.host().map(|h| h.to_string())).unwrap_or_default(),
            title: self.title(),
            date: self.date(),
            tags: vec![],
            comment_links: HashMap::new(),
            scrapes: HashMap::new(),
        }
    }
}
