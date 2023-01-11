use std::collections::HashMap;

use serde::Serialize;

use super::{ScrapeConfig, ScrapeConfigSource, ScrapeSource, hacker_news::HackerNews, ScrapeSource2, ScrapeError, Scrape, reddit_json::Reddit, lobsters::Lobsters, slashdot::Slashdot};

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
            scrapes.insert(*source, subsources);
        }
        WebScrapeInput { scrapes }
    }

    pub fn compute_urls(config: &ScrapeConfig, source: ScrapeSource, subsources: Vec<String>) -> Vec<String> {
        if let Some(scrape) = Self::scrapes(config).get(&source) {
            scrape.provide_urls(subsources)
        } else {
            vec![]
        }
    }

    /// "Box" the `Scrape`s into the `Scrape` enum. 
    fn map<T>(input: (Vec<T>, Vec<String>)) -> (Vec<Scrape>, Vec<String>) where Scrape: From<T> {
        (input.0.into_iter().map(|x| x.into()).collect(), input.1)
    }

    pub fn scrape(config: &ScrapeConfig, source: ScrapeSource, input: String) -> Result<(Vec<Scrape>, Vec<String>), ScrapeError> {
        Ok(match source {
            ScrapeSource::HackerNews => Self::map(HackerNews::scrape(&config.hacker_news, input)?),
            ScrapeSource::Reddit => Self::map(Reddit::scrape(&config.reddit, input)?),
            ScrapeSource::Lobsters => Self::map(Lobsters::scrape(&config.lobsters, input)?),
            ScrapeSource::Slashdot => Self::map(Slashdot::scrape(&config.slashdot, input)?),
            ScrapeSource::Other => unreachable!()
        })
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
