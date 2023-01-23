use std::collections::HashMap;

use progscrape_scrapers::{StoryDate, ScrapeId, TypedScrape};
use serde::{Deserialize, Serialize};

/// Rendered story with all properties hydrated from the underlying scrapes. Extraneous data is removed at this point.
#[derive(Clone, Default, Deserialize, Serialize)]
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
    pub comment_links: HashMap<String, String>,
}

/// Fully-rendered story, suitable for display on admin screens.
#[derive(Clone, Default, Deserialize, Serialize)]
pub struct StoryFullRender {
    /// Base render flattened into this structure at display time by serde.
    #[serde(flatten)]
    pub base: StoryRender,

    pub url_norm: String,
    pub url_norm_hash: i64,

    /// Fully-detailed scrapes
    pub scrapes: HashMap<ScrapeId, TypedScrape>,
}
