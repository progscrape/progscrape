use crate::scrapers::{Scrape, ScrapeSource, ScrapeId};
use std::collections::HashMap;

/// Rendered story with all properties hydrated from the underlying scrapes. Extraneous data is removed at this point.
pub struct StoryRender {
    url: String,
    title: String,
    tags: Vec<String>,
    comment_links: HashMap<String, String>,
}

/// Story scrape w/information from underlying sources.
pub struct Story {
    pub scrapes: HashMap<ScrapeId, Scrape>,
}

impl Story {
    pub fn title(&self) -> String {
        unimplemented!()
    }

    pub fn url(&self) -> String {
        unimplemented!()
    }
}
