use crate::story::{StoryDate, StoryUrl};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod hacker_news;
mod html;
pub mod legacy_import;
pub mod lobsters;
pub mod reddit_json;
pub mod slashdot;
pub mod web_scraper;

/// Our scrape sources, and the associated data types for each.
pub trait ScrapeSource2 {
    type Config: ScrapeConfigSource;
    type Scrape: ScrapeData;
    type Scraper: Scraper<Self::Config, Self::Scrape>;
    const TYPE: ScrapeSource;
}

#[derive(Default, Serialize, Deserialize)]
pub struct ScrapeConfig {
    hacker_news: hacker_news::HackerNewsConfig,
    slashdot: slashdot::SlashdotConfig,
    lobsters: lobsters::LobstersConfig,
    reddit: reddit_json::RedditConfig,
}

pub trait ScrapeConfigSource {
    fn subsources(&self) -> Vec<String>;
    fn provide_urls(&self, subsources: Vec<String>) -> Vec<String>;
}

#[derive(Error, Debug)]
pub enum ScrapeError {
    #[error("I/O error")]
    IO(#[from] std::io::Error),
    #[error("JSON parse error")]
    JSON(#[from] serde_json::Error),
    #[error("HTML parse error")]
    HTML(#[from] tl::ParseError),
    #[error("Structure error")]
    StructureError(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord, Deserialize, Serialize)]
pub enum ScrapeSource {
    HackerNews,
    Reddit,
    Lobsters,
    Slashdot,
    Other,
}

impl Into<&'static str> for &ScrapeSource {
    fn into(self) -> &'static str {
        use ScrapeSource::*;
        match self {
            HackerNews => "hacker_news",
            Reddit => "reddit",
            Lobsters => "lobsters",
            Slashdot => "slashdot",
            Other => "other",
        }
    }
}

/// Identify a scrape by source an ID.
#[derive(Clone, Debug, Eq, PartialEq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ScrapeId {
    pub source: ScrapeSource,
    pub subsource: Option<String>,
    pub id: String,
}

impl ScrapeId {
    pub fn new(source: ScrapeSource, subsource: Option<String>, id: String) -> Self {
        Self {
            source,
            subsource,
            id,
        }
    }

    pub fn as_str(&self) -> String {
        let source: &'static str = (&self.source).into();
        if let Some(subsource) = &self.subsource {
            format!("{}-{}-{}", source, subsource, self.id)
        } else {
            format!("{}-{}", source, self.id)
        }
    }
}

pub trait ScrapeData {
    /// Retrieve the scrape title.
    fn title(&self) -> String;

    /// Retrieve the scrape URL.
    fn url(&self) -> StoryUrl;

    /// Retrieve the scrape comments URL.
    fn comments_url(&self) -> String;

    /// Retrieve the scrape source.
    fn source(&self) -> ScrapeId;

    /// Retrieve the scrape date.
    fn date(&self) -> StoryDate;
}

/// Core partial initialization method.
pub trait ScrapeDataInit<T: ScrapeData> {
    fn initialize_required(id: String, title: String, url: StoryUrl, date: StoryDate) -> T;
    fn merge(&mut self, other: T);
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum Scrape {
    HackerNews(hacker_news::HackerNewsStory),
    Reddit(reddit_json::RedditStory),
    Lobsters(lobsters::LobstersStory),
    Slashdot(slashdot::SlashdotStory),
}

impl From<hacker_news::HackerNewsStory> for Scrape {
    fn from(story: hacker_news::HackerNewsStory) -> Self {
        Self::HackerNews(story)
    }
}

impl From<reddit_json::RedditStory> for Scrape {
    fn from(story: reddit_json::RedditStory) -> Self {
        Self::Reddit(story)
    }
}

impl From<lobsters::LobstersStory> for Scrape {
    fn from(story: lobsters::LobstersStory) -> Self {
        Self::Lobsters(story)
    }
}

impl From<slashdot::SlashdotStory> for Scrape {
    fn from(story: slashdot::SlashdotStory) -> Self {
        Self::Slashdot(story)
    }
}

impl AsRef<dyn ScrapeData + 'static> for Scrape {
    fn as_ref(&self) -> &(dyn ScrapeData + 'static) {
        match self {
            &Scrape::HackerNews(ref x) => x,
            &Scrape::Reddit(ref x) => x,
            &Scrape::Lobsters(ref x) => x,
            &Scrape::Slashdot(ref x) => x,
        }
    }
}

impl ScrapeData for Scrape {
    fn url(&self) -> StoryUrl {
        self.as_ref().url()
    }

    fn title(&self) -> String {
        self.as_ref().title()
    }

    fn date(&self) -> StoryDate {
        self.as_ref().date()
    }

    fn comments_url(&self) -> String {
        self.as_ref().comments_url()
    }

    fn source(&self) -> ScrapeId {
        self.as_ref().source()
    }
}

pub trait Scraper<Config: ScrapeConfigSource, Output: ScrapeData> {
    fn scrape(&self, args: Config, input: String) -> Result<(Vec<Output>, Vec<String>), ScrapeError>;
}

#[cfg(test)]
pub mod test {
    use super::*;
    use rstest::*;
    use std::fs::read_to_string;
    use std::path::PathBuf;
    use std::str::FromStr;

    pub fn slashdot_files() -> Vec<&'static str> {
        vec!["slashdot1.html", "slashdot2.html"]
    }

    pub fn hacker_news_files() -> Vec<&'static str> {
        vec!["hn1.html", "hn2.html", "hn3.html", "hn4.html"]
    }

    pub fn reddit_files() -> Vec<&'static str> {
        vec![
            "reddit-prog-tag1.json",
            "reddit-prog-tag2.json",
            "reddit-prog1.json",
            "reddit-science1.json",
            "reddit-science2.json",
        ]
    }

    pub fn scrape_all() -> Vec<Scrape> {
        let mut v = vec![];
        v.extend(
            super::hacker_news::test::scrape_all()
                .into_iter()
                .map(Scrape::HackerNews),
        );
        v.extend(
            super::reddit_json::test::scrape_all()
                .into_iter()
                .map(Scrape::Reddit),
        );
        v.extend(
            super::slashdot::test::scrape_all()
                .into_iter()
                .map(Scrape::Slashdot),
        );
        v
    }

    pub fn load_file(f: &str) -> String {
        let mut path = PathBuf::from_str("src/scrapers/testdata").unwrap();
        path.push(f);
        read_to_string(path).unwrap()
    }

    #[test]
    fn test_scrape_all() {
        scrape_all();
    }
}
