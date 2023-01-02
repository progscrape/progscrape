use chrono::{DateTime, Utc};

use crate::scrapers::{Scrape, ScrapeSource, ScrapeId, ScrapeData};
use std::collections::HashMap;

/// Rendered story with all properties hydrated from the underlying scrapes. Extraneous data is removed at this point.
pub struct StoryRender {
    url: String,
    title: String,
    tags: Vec<String>,
    comment_links: HashMap<String, String>,
}

/// Story scrape w/information from underlying sources.
#[derive(Clone, Default)]
pub struct Story {
    pub normalized_url: String,
    pub scrapes: HashMap<ScrapeId, Scrape>,
}

impl Story {
    pub fn new(normalized_url: String, scrape: Scrape) -> Self {
        let id = ScrapeId::new(scrape.source(), scrape.id());
        Self {
            normalized_url,
            scrapes: HashMap::from_iter([(id, scrape)])
        }
    }

    pub fn merge(&mut self, scrape: Scrape) {
        // This logic can be improved when try_insert is stabilized
        // TODO: We need to merge duplicate scrapes
        let id = ScrapeId::new(scrape.source(), scrape.id());
        self.scrapes.insert(id, scrape);
    }

    pub fn title(&self) -> String {
        self.scrapes.values().next().expect("Expected at least one").title()
    }

    pub fn url(&self) -> String {
        self.scrapes.values().next().expect("Expected at least one").url()
    }

    pub fn date(&self) -> DateTime<Utc> {
        self.scrapes.values().next().expect("Expected at least one").date()
    }
}
