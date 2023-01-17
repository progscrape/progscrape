use std::{borrow::Cow, fmt::Debug};

use crate::story::{StoryDate, StoryUrl};
use enumset::{EnumSetType};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod hacker_news;
mod html;
mod id;
pub mod legacy_import;
pub mod lobsters;
pub mod reddit;
pub mod slashdot;
pub mod web_scraper;

pub use id::ScrapeId;

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

macro_rules! scrapers {
    ($($package:ident :: $name:ident ,)*) => {
        pub fn scrape(
            config: &ScrapeConfig,
            source: ScrapeSource,
            input: &str,
        ) -> Result<(Vec<TypedScrape>, Vec<String>), ScrapeError> {
            match source {
                $(
                    ScrapeSource::$name => {
                        let scraper = <$package::$name as ScrapeSourceDef>::Scraper::default();
                        let (res, warnings) = scraper.scrape(&config.$package, input)?;
                        Ok((res.into_iter().map(|x| x.into()).collect(), warnings))
                    },
                )*
                ScrapeSource::Other => unreachable!(),
            }
        }

        /// Configuration for all scrapers.
        #[derive(Clone, Default, Serialize, Deserialize)]
        pub struct ScrapeConfig {
            $($package : <$package :: $name as ScrapeSourceDef>::Config),*
        }

        impl ScrapeConfig {
            pub fn get(&self, source: ScrapeSource) -> Option<&dyn ScrapeConfigSource> {
                match source {
                    $( ScrapeSource::$name => Some(&self.$package), )*
                    ScrapeSource::Other => None,
                }
            }
        }

        #[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord, Deserialize, Serialize)]
        pub enum ScrapeSource {
            $($name,)*
            Other,
        }

        impl ScrapeSource {
            pub fn into_str(&self) -> &'static str {
                match self {
                    $(Self::$name => stringify!($package),)*
                    Self::Other => "other",
                }
            }

            pub fn try_from_str(s: &str) -> Option<Self> {
                match s {
                    $(stringify!($package) => Some(Self::$name),)*
                    "other" => Some(Self::Other),
                    _ => None,
                }
            }

            pub const fn all() -> &'static [ScrapeSource] {
                &[$(Self::$name),*]
            }
        }

        #[derive(Clone, Debug, Deserialize, Serialize)]
        pub enum TypedScrape {
            $( $name (<$package::$name as ScrapeSourceDef>::Scrape), )*
        }

        impl TypedScrape {
            pub fn merge(&mut self, b: Self) {
                match (self, b) {
                    $( (Self::$name(a), Self::$name(b)) => a.merge(b), )*
                    (_a, _b) => {
                        tracing::warn!(
                            "Unable to merge incompatible scrapes, ignoring",
                        );
                    }
                }
            }

            fn extract(&self, config: &ScrapeConfig) -> ScrapeCore {
                match self {
                    $(
                        Self::$name(a) => {
                            let scraper = <$package::$name as ScrapeSourceDef>::Scraper::default();
                            scraper.extract_core(&config.$package, a)
                        }
                    )*
                }
            }
        }

        $(
            impl From<<$package::$name as ScrapeSourceDef>::Scrape> for TypedScrape {
                fn from(x: <$package::$name as ScrapeSourceDef>::Scrape) -> Self {
                    TypedScrape::$name(x)
                }
            }
        )*
    };
}

scrapers! {
    hacker_news::HackerNews,
    slashdot::Slashdot,
    lobsters::Lobsters,
    reddit::Reddit,
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

#[derive(EnumSetType, Debug)]
pub enum ScrapeMergeResult {
    Date,
    Title,
    URL,
}

impl<'a> ScrapeCore<'a> {
    // pub fn merge(&mut self, other: Self) -> EnumSet<ScrapeMergeResult> {
    //     let mut changes = EnumSet::empty();
    //     if self.date != other.date {
    //         self.date = std::cmp::min(self.date, other.date);
    //         changes |= ScrapeMergeResult::Date;
    //     }
    //     let (other, other_data) = (other.core, other.data);
    //     if self.title != other.title {
    //         self.title = other.title;
    //         changes |= ScrapeMergeResult::Title;
    //     }
    //     if self.url != other.url {
    //         self.url = other.url;
    //         changes |= ScrapeMergeResult::URL;
    //     }
    //     self.data.merge(other_data);
    //     changes
    // }
}

#[cfg(test)]
pub mod test {
    use super::*;
    use std::fs::read_to_string;
    use std::path::PathBuf;
    use std::str::FromStr;

    pub fn slashdot_files() -> Vec<&'static str> {
        vec!["slashdot1.html", "slashdot2.html", "slashdot3.html"]
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

    pub fn files_by_source(source: ScrapeSource) -> Vec<&'static str> {
        match source {
            ScrapeSource::HackerNews => hacker_news_files(),
            ScrapeSource::Slashdot => slashdot_files(),
            ScrapeSource::Reddit => reddit_files(),
            ScrapeSource::Lobsters => lobsters_files(),
            ScrapeSource::Other => vec![],
        }
    }

    pub fn scrape_all() -> Vec<TypedScrape> {
        let mut v = vec![];
        let config = ScrapeConfig::default();
        for source in [
            ScrapeSource::HackerNews,
            ScrapeSource::Lobsters,
            ScrapeSource::Reddit,
            ScrapeSource::Slashdot,
        ] {
            for file in files_by_source(source) {
                let mut res = scrape(&config, source, &load_file(file))
                    .unwrap_or_else(|_| panic!("Scrape of {:?} failed", source));
                v.append(&mut res.0);
            }
        }
        v
    }

    pub fn load_file(f: &str) -> String {
        let mut path = PathBuf::from_str("src/scrapers/testdata").unwrap();
        path.push(f);
        read_to_string(path).unwrap()
    }

    #[test]
    fn test_scrape_all() {
        let extractor = ScrapeExtractor::new(&ScrapeConfig::default());
        for scrape in scrape_all() {
            let scrape = extractor.extract(&scrape);
            // Sanity check the scrapes
            assert!(
                !scrape.title.contains("&amp")
                    && !scrape.title.contains("&quot")
                    && !scrape.title.contains("&squot")
            );
            assert!(!scrape.url.raw().contains("&amp"));
            assert!(scrape.date.year() == 2023 || scrape.date.year() == 2022);
        }
    }
}
