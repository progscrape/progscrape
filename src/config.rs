use serde::{Deserialize, Serialize};

/// Root configuration for the application.
#[derive(Default, Serialize, Deserialize)]
pub struct Config {
    pub score: crate::story::StoryScoreConfig,
    pub scrape: crate::scrapers::ScrapeConfig,
    pub cron: crate::web::cron::CronConfig,
}
