use serde::{Deserialize, Serialize};
use url::Url;

use crate::scrapers::{Scrape, ScrapeData, ScrapeId};
use std::{
    collections::{hash_map::DefaultHasher, HashMap},
    hash::{Hash, Hasher},
};

mod date;
pub use date::StoryDate;

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
            domain: Url::parse(&self.url())
                .ok()
                .and_then(|u| u.host().map(|h| h.to_string()))
                .unwrap_or_default(),
            title: self.title(),
            date: self.date(),
            tags: vec![],
            comment_links: HashMap::new(),
            scrapes: HashMap::new(),
        }
    }
}
