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
}
