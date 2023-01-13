use crate::story::{StoryDate, StoryUrl, TagSet};
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
    type Scrape: ScrapeStory;
    type Scraper: Scraper<Self::Config, Self::Scrape>;

    fn scrape(
        args: &Self::Config,
        input: String,
    ) -> Result<(Vec<Scrape<Self::Scrape>>, Vec<String>), ScrapeError> {
        Self::Scraper::default().scrape(args, input)
    }
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
    Json(#[from] serde_json::Error),
    #[error("HTML parse error")]
    Html(#[from] tl::ParseError),
    #[error("XML parse error")]
    Xml(#[from] roxmltree::Error),
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

pub trait ScrapeStory: Default {
    const TYPE: ScrapeSource;

    fn comments_url(&self) -> String;

    fn merge(&mut self, other: Self);
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ScrapeCore {
    pub title: String,
    pub url: StoryUrl,
    pub source: ScrapeId,
    pub date: StoryDate,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Scrape<T: ScrapeStory> {
    core: ScrapeCore,

    /// The additional underlying data from the scrape.
    pub data: T,
}

impl<T: ScrapeStory> core::ops::Deref for Scrape<T> {
    type Target = ScrapeCore;
    fn deref(&self) -> &Self::Target {
        &self.core
    }
}

impl<T: ScrapeStory> core::ops::DerefMut for Scrape<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.core
    }
}

impl<T: ScrapeStory> Scrape<T> {
    pub fn new(id: String, title: String, url: StoryUrl, date: StoryDate, data: T) -> Self {
        Self {
            core: ScrapeCore {
                source: ScrapeId {
                    source: T::TYPE,
                    subsource: None,
                    id,
                },
                title,
                url,
                date,
            },
            data,
        }
    }

    pub fn new_subsource(
        id: String,
        subsource: String,
        title: String,
        url: StoryUrl,
        date: StoryDate,
        data: T,
    ) -> Self {
        Self {
            core: ScrapeCore {
                source: ScrapeId {
                    source: T::TYPE,
                    subsource: Some(subsource),
                    id,
                },
                title,
                url,
                date,
            },
            data,
        }
    }

    pub fn merge(&mut self, other: Self) {
        self.date = std::cmp::min(self.date, other.date);
        let (other, other_data) = (other.core, other.data);
        self.title = other.title;
        self.url = other.url;
        self.data.merge(other_data);
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum TypedScrape {
    HackerNews(Scrape<hacker_news::HackerNewsStory>),
    Reddit(Scrape<reddit_json::RedditStory>),
    Lobsters(Scrape<lobsters::LobstersStory>),
    Slashdot(Scrape<slashdot::SlashdotStory>),
}

impl TypedScrape {
    pub fn merge(&mut self, b: Self) {
        match (self, b) {
            (Self::HackerNews(a), Self::HackerNews(b)) => a.merge(b),
            (Self::Reddit(a), Self::Reddit(b)) => a.merge(b),
            (Self::Lobsters(a), Self::Lobsters(b)) => a.merge(b),
            (Self::Slashdot(a), Self::Slashdot(b)) => a.merge(b),
            (a, b) => {
                tracing::warn!(
                    "Unable to merge incompatible scrapes {:?} and {:?}, ignoring",
                    &a.source,
                    &b.source
                );
            }
        }
    }
}

impl core::ops::Deref for TypedScrape {
    type Target = ScrapeCore;
    fn deref(&self) -> &Self::Target {
        use TypedScrape::*;
        match self {
            HackerNews(x) => &x.core,
            Reddit(x) => &x.core,
            Lobsters(x) => &x.core,
            Slashdot(x) => &x.core,
        }
    }
}

impl core::ops::DerefMut for TypedScrape {
    fn deref_mut(&mut self) -> &mut Self::Target {
        use TypedScrape::*;
        match self {
            HackerNews(x) => &mut x.core,
            Reddit(x) => &mut x.core,
            Lobsters(x) => &mut x.core,
            Slashdot(x) => &mut x.core,
        }
    }
}

impl From<Scrape<hacker_news::HackerNewsStory>> for TypedScrape {
    fn from(x: Scrape<hacker_news::HackerNewsStory>) -> Self {
        TypedScrape::HackerNews(x)
    }
}

impl From<Scrape<reddit_json::RedditStory>> for TypedScrape {
    fn from(x: Scrape<reddit_json::RedditStory>) -> Self {
        TypedScrape::Reddit(x)
    }
}

impl From<Scrape<lobsters::LobstersStory>> for TypedScrape {
    fn from(x: Scrape<lobsters::LobstersStory>) -> Self {
        TypedScrape::Lobsters(x)
    }
}

impl From<Scrape<slashdot::SlashdotStory>> for TypedScrape {
    fn from(x: Scrape<slashdot::SlashdotStory>) -> Self {
        TypedScrape::Slashdot(x)
    }
}

pub trait Scraper<Config: ScrapeConfigSource, Output: ScrapeStory>: Default {
    /// Given input in the correct format, scrapes raw stories.
    fn scrape(
        &self,
        args: &Config,
        input: String,
    ) -> Result<(Vec<Scrape<Output>>, Vec<String>), ScrapeError>;

    /// Given a scrape, processes the tags from it and adds them to the `TagSet`.
    fn provide_tags(
        &self,
        args: &Config,
        scrape: &Scrape<Output>,
        tags: &mut TagSet,
    ) -> Result<(), ScrapeError>;
}

#[cfg(test)]
pub mod test {
    use super::*;
    use std::fs::read_to_string;
    use std::path::PathBuf;
    use std::str::FromStr;

    pub fn slashdot_files() -> Vec<&'static str> {
        vec!["slashdot1.html", "slashdot2.html"]
    }

    pub fn hacker_news_files() -> Vec<&'static str> {
        vec!["hn1.html", "hn2.html", "hn3.html", "hn4.html"]
    }

    pub fn lobsters_files() -> Vec<&'static str> {
        vec!["lobsters1.rss", "lobsters2.rss"]
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

    pub fn scrape_all() -> Vec<TypedScrape> {
        let mut v = vec![];
        v.extend(
            super::hacker_news::test::scrape_all()
                .into_iter()
                .map(TypedScrape::HackerNews),
        );
        v.extend(
            super::reddit_json::test::scrape_all()
                .into_iter()
                .map(TypedScrape::Reddit),
        );
        v.extend(
            super::slashdot::test::scrape_all()
                .into_iter()
                .map(TypedScrape::Slashdot),
        );
        v
    }

    pub fn load_file(f: &str) -> String {
        let mut path = PathBuf::from_str("src/scrapers/testdata").unwrap();
        path.push(f);
        read_to_string(path).unwrap()
    }

    pub fn dump_story(_story: TypedScrape) {}

    #[test]
    fn test_scrape_all() {
        scrape_all();
    }
}
