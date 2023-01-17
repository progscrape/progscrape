use crate::ScrapeId;

use super::*;

/// Our scrape sources, and the associated data types for each.
pub trait ScrapeSourceDef {
    type Config: ScrapeConfigSource;
    type Scrape: ScrapeStory;
    type Scraper: Scraper<Config = Self::Config, Output = Self::Scrape>;
}

pub trait ScrapeStory {
    const TYPE: ScrapeSource;

    fn comments_url(&self) -> String;

    fn merge(&mut self, other: Self);
}

pub trait Scraper: Default {
    type Config: ScrapeConfigSource;
    type Output: ScrapeStory;

    /// Given input in the correct format, scrapes raw stories.
    fn scrape(
        &self,
        args: &Self::Config,
        input: &str,
    ) -> Result<(Vec<Self::Output>, Vec<String>), ScrapeError>;

    /// Extract the core scrape elements from the raw scrape.
    fn extract_core<'a>(&self, args: &Self::Config, input: &'a Self::Output) -> ScrapeCore<'a>;
}

pub trait ScrapeConfigSource {
    fn subsources(&self) -> Vec<String>;
    fn provide_urls(&self, subsources: Vec<String>) -> Vec<String>;
}

#[derive(Clone, Debug)]
pub struct ScrapeCore<'a> {
    /// The scrape source ID.
    pub source: ScrapeId,

    /// Story title from this scrape source, potentially edited based on source (stripping suffixes, etc).
    pub title: Cow<'a, str>,

    /// Story URL.
    pub url: &'a StoryUrl,

    /// Story date/time.
    pub date: StoryDate,

    /// Story tags from scrape source.
    pub tags: Vec<Cow<'a, str>>,

    /// If this story has a rank, lower is better.
    pub rank: Option<usize>,
}


