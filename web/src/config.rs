use serde::{Deserialize, Serialize};

/// Root configuration for the application.
#[derive(Default, Serialize, Deserialize)]
pub struct Config {
    pub score: progscrape_application::StoryScoreConfig,
    pub tagger: progscrape_application::TaggerConfig,
    pub scrape: progscrape_scrapers::ScrapeConfig,
    pub cron: crate::cron::CronConfig,
}
