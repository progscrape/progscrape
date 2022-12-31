use super::*;
use serde::{Deserialize, Serialize};
use chrono::{serde::ts_seconds, DateTime, Utc};

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct LobstersStory {
    pub id: String,
    pub title: String,
    pub url: String,
    pub num_comments: u32,
    pub score: u32,
    #[serde(with = "ts_seconds")]
    pub date: DateTime<Utc>,
}

impl Scrape for LobstersStory {
    fn title(&self) -> String {
        return self.title.clone();
    }

    fn url(&self) -> String {
        return self.url.clone();
    }

    fn comments_url(&self) -> String {
        unimplemented!()
    }

    fn source(&self) -> super::ScrapeSource {
        return ScrapeSource::Lobsters;
    }
}
