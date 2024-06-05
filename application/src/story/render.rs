use std::collections::HashMap;

use progscrape_scrapers::{ScrapeId, StoryDate, TypedScrape, TypedScrapeMap};
use serde::{Deserialize, Serialize};

/// Rendered story with all properties hydrated from the underlying scrapes. Extraneous data is removed at this point.
#[derive(Clone, Default, Debug, Deserialize, Serialize)]
pub struct StoryRender {
    /// Natural story order in its container list.
    pub order: usize,
    /// An ID useful for pulling the full information for this story.
    pub id: String,
    pub url: String,
    pub domain: String,
    pub title: String,
    pub date: StoryDate,
    pub score: f32,
    pub tags: Vec<String>,
    /// Only for our blog posts
    pub html: String,
    pub sources: TypedScrapeMap<Option<ScrapeId>>,
}
