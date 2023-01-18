use crate::backends::{ScrapeConfig, ScrapeCore, TypedScrape};

pub struct ScrapeExtractor {
    config: ScrapeConfig,
}

impl ScrapeExtractor {
    pub fn new(config: &ScrapeConfig) -> Self {
        Self {
            config: config.clone(),
        }
    }

    pub fn extract<'a>(&self, scrape: &'a TypedScrape) -> ScrapeCore<'a> {
        scrape.extract(&self.config)
    }
}
