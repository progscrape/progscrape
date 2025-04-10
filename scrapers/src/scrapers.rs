//! Public interface for the collection of scrapers.
use std::collections::HashMap;

use serde::Serialize;

use crate::{ScrapeConfig, ScrapeSource, TypedScrape, backends::scrape};

/// Accumulates the URLs required to scrape for all the services.
#[derive(Serialize)]
pub struct ScraperPossibilities {
    pub scrapes: HashMap<ScrapeSource, Vec<String>>,
}

#[derive(Serialize)]
pub enum ScraperHttpResponseInput {
    HTTPError(u16, String),
    Ok(String),
}

#[derive(Serialize)]
pub enum ScraperHttpResult {
    Err(ScraperHttpResponseInput, String),
    Ok(String, Vec<TypedScrape>),
}

pub struct Scrapers {
    config: ScrapeConfig,
}

/// Interface to the collection of scrapers in this library.
impl Scrapers {
    pub fn new(config: &ScrapeConfig) -> Self {
        Self {
            config: config.clone(),
        }
    }

    /// Compute the list of all possible scrapes from all sources and subsources.
    pub fn compute_scrape_possibilities(&self) -> ScraperPossibilities {
        let mut scrapes = HashMap::new();
        for source in ScrapeSource::all() {
            if let Some(config) = self.config.get(*source) {
                let subsources = config.subsources();
                scrapes.insert(*source, subsources);
            }
        }
        ScraperPossibilities { scrapes }
    }

    /// Compute the list of all possible scrapes from all sources and subsources.
    pub fn compute_scrape_subsources(&self, source: ScrapeSource) -> Vec<String> {
        if let Some(config) = self.config.get(source) {
            let subsources = config.subsources();
            return subsources;
        }
        vec![]
    }

    /// Given a source and subsources, compute the set of URLs to fetch.
    pub fn compute_scrape_url_demands(
        &self,
        source: ScrapeSource,
        subsources: Vec<String>,
    ) -> Vec<String> {
        if let Some(scrape) = self.config.get(source) {
            scrape.provide_urls(subsources)
        } else {
            vec![]
        }
    }

    /// Given the result of fetching a URL, returns the scraped stories.
    pub fn scrape_http_result(
        &self,
        source: ScrapeSource,
        input: ScraperHttpResponseInput,
    ) -> ScraperHttpResult {
        match input {
            ScraperHttpResponseInput::Ok(s) => match scrape(&self.config, source, &s) {
                Ok((scrapes, _warnings)) => ScraperHttpResult::Ok(s, scrapes),
                Err(e) => {
                    ScraperHttpResult::Err(ScraperHttpResponseInput::Ok(s), format!("{:?}", e))
                }
            },
            error @ ScraperHttpResponseInput::HTTPError(..) => {
                ScraperHttpResult::Err(error, "HTTP Error".to_string())
            }
        }
    }
}
