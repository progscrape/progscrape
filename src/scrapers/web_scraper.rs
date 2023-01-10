use std::collections::HashMap;

use url::Url;

use super::{ScrapeConfig, ScrapeConfigSource, ScrapeSource};

/// Accumulates the URLs required to scrape for all the services.
pub struct WebScrapeInput {
    pub scrapes: HashMap<String, Vec<Url>>,
}

pub struct WebScraper {
}

impl WebScraper {
    pub fn calculate_inputs(config: &ScrapeConfig) -> WebScrapeInput {
        unimplemented!()
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
