use serde::{Deserialize, Serialize};

use crate::scrapers::{Scrape, ScrapeData, ScrapeId};
use std::{
    collections::{HashMap},
};

mod date;
mod url;

pub use self::{date::StoryDate, url::{StoryUrl, StoryUrlNorm}};

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
    pub scrapes: HashMap<ScrapeId, Scrape>,
}

impl Story {
    pub fn new(scrape: Scrape) -> Self {
        let id = ScrapeId::new(scrape.source(), scrape.id());
        Self {
            scrapes: HashMap::from_iter([(id, scrape)]),
        }
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

    pub fn url(&self) -> StoryUrl {
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
            url: self.url().to_string(),
            domain: self.url().host().to_owned(),
            title: self.title(),
            date: self.date(),
            tags: vec![],
            comment_links: HashMap::new(),
            scrapes: HashMap::new(),
        }
    }
}
