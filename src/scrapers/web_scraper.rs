use std::collections::HashMap;

use serde::Serialize;

use super::{ScrapeConfig, ScrapeConfigSource, ScrapeSource};

/// Accumulates the URLs required to scrape for all the services.
#[derive(Serialize)]
pub struct WebScrapeInput {
    pub scrapes: HashMap<ScrapeSource, Vec<String>>,
}

pub struct WebScraper {}

impl WebScraper {
    pub fn calculate_inputs(config: &ScrapeConfig) -> WebScrapeInput {
        let sources = Self::scrapes(config);
        let mut scrapes = HashMap::new();
        for (source, scrape_config) in &sources {
            let subsources = scrape_config.subsources();
            let urls = scrape_config.provide_urls(subsources);
            scrapes.insert(*source, urls);
        }
        WebScrapeInput { scrapes }
    }

    pub fn scrape(config: &ScrapeConfig, source: ScrapeSource, input: String) {
        // match source {
        //     ScrapeSource::HackerNews => Hack
        // }
    }

    fn scrapes(config: &ScrapeConfig) -> HashMap<ScrapeSource, &dyn ScrapeConfigSource> {
        use ScrapeSource::*;
        HashMap::from_iter([
            (HackerNews, &config.hacker_news as &dyn ScrapeConfigSource),
            (Lobsters, &config.lobsters as &dyn ScrapeConfigSource),
            (Slashdot, &config.slashdot as &dyn ScrapeConfigSource),
            (Reddit, &config.reddit as &dyn ScrapeConfigSource),
        ])
    }
}
