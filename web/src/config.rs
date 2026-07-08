use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Root configuration for the application.
#[derive(Default, Serialize, Deserialize)]
pub struct Config {
    pub index: crate::index::IndexConfig,
    pub score: progscrape_application::StoryScoreConfig,
    pub tagger: progscrape_application::TaggerConfig,
    pub scrape: progscrape_scrapers::ScrapeConfig,
    pub cron: crate::cron::CronConfig,
    pub rate_limits: crate::rate_limits::RateLimitsConfig,
    /// `/admin/proxy/<name>/<path>` -> upstream base URL, e.g. { "scrape-vm": "http://scrape-vm:8080" }.
    #[serde(default)]
    pub proxy: HashMap<String, String>,
    /// Background task-dump interval (secs); 0 = off (dump() pauses the runtime).
    #[serde(default)]
    pub task_dump_interval_seconds: u64,
}
