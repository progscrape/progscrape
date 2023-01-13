use std::collections::HashMap;

use serde::Serialize;

use super::{scrape, ScrapeConfig, ScrapeError, ScrapeSource, TypedScrape};

/// Accumulates the URLs required to scrape for all the services.
#[derive(Serialize)]
pub struct WebScrapeInput {
    pub scrapes: HashMap<ScrapeSource, Vec<String>>,
}

#[derive(Serialize)]
pub enum WebScrapeHttpResult {
    HTTPError(u16, String),
    Ok(String),
}

#[derive(Serialize)]
pub struct WebScrapeURLs {
    pub urls: Vec<String>,
}

#[derive(Serialize)]
pub enum WebScrapeURLResult {
    Err(WebScrapeHttpResult, String),
    Ok(String, Vec<TypedScrape>),
}

#[derive(Serialize)]
pub struct WebScrapeResult {
    pub url: String,
    pub result: Vec<WebScrapeURLResult>,
}

pub struct WebScraper {}

impl WebScraper {
    /// Compute the list of all possible scrapes from all sources and subsources.
    pub fn compute_all_scrapes(config: &ScrapeConfig) -> WebScrapeInput {
        let mut scrapes = HashMap::new();
        for source in ScrapeSource::all() {
            if let Some(config) = config.get(*source) {
                let subsources = config.subsources();
                scrapes.insert(*source, subsources);
            }
        }
        WebScrapeInput { scrapes }
    }

    /// Given a source and subsources, compute the set of URLs to fetch.
    pub fn compute_urls(
        config: &ScrapeConfig,
        source: ScrapeSource,
        subsources: Vec<String>,
    ) -> Vec<String> {
        if let Some(scrape) = config.get(source) {
            scrape.provide_urls(subsources)
        } else {
            vec![]
        }
    }

    /// Given the result of fetching a URL, returns the scraped stories.
    pub fn scrape(
        config: &ScrapeConfig,
        source: ScrapeSource,
        input: WebScrapeHttpResult,
    ) -> WebScrapeURLResult {
        match input {
            WebScrapeHttpResult::Ok(s) => match scrape(config, source, &s) {
                Ok((scrapes, warnings)) => WebScrapeURLResult::Ok(s, scrapes),
                Err(e) => WebScrapeURLResult::Err(WebScrapeHttpResult::Ok(s), format!("{:?}", e)),
            },
            error @ WebScrapeHttpResult::HTTPError(..) => {
                WebScrapeURLResult::Err(error, "HTTP Error".to_string())
            }
        }
    }
}
