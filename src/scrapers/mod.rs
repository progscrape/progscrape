use std::marker::PhantomData;

use crate::story::{StoryDate, StoryUrl, TagSet};
use enumset::{EnumSetType, EnumSet};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod hacker_news;
mod html;
pub mod legacy_import;
pub mod lobsters;
pub mod reddit;
pub mod slashdot;
pub mod web_scraper;

/// Our scrape sources, and the associated data types for each.
pub trait ScrapeSourceDef {
    type Config: ScrapeConfigSource;
    type Scrape: ScrapeStory;
    type Scraper: Scraper<Self::Config, Self::Scrape>;
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
        #[derive(Default, Serialize, Deserialize)]
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
            $( $name (Scrape<<$package::$name as ScrapeSourceDef>::Scrape>), )*
        }

        impl TypedScrape {
            pub fn merge(&mut self, b: Self) -> EnumSet<ScrapeMergeResult> {
                match (self, b) {
                    $( (Self::$name(a), Self::$name(b)) => a.merge(b), )*
                    (a, b) => {
                        tracing::warn!(
                            "Unable to merge incompatible scrapes {:?} and {:?}, ignoring",
                            &a.source,
                            &b.source
                        );
                        EnumSet::empty()
                    }
                }
            }
        }

        impl core::ops::Deref for TypedScrape {
            type Target = ScrapeCore;
            fn deref(&self) -> &Self::Target {
                match self {
                    $( TypedScrape::$name(x) => &x.core, )*
                }
            }
        }

        impl core::ops::DerefMut for TypedScrape {
            fn deref_mut(&mut self) -> &mut Self::Target {
                match self {
                    $( TypedScrape::$name(x) => &mut x.core, )*
                }
            }
        }

        $(
            impl From<Scrape<<$package::$name as ScrapeSourceDef>::Scrape>> for TypedScrape {
                fn from(x: Scrape<<$package::$name as ScrapeSourceDef>::Scrape>) -> Self {
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

/// Identify a scrape by source an ID.
#[derive(Clone, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ScrapeId {
    pub source: ScrapeSource,
    pub subsource: Option<String>,
    pub id: String,
    _noinit: PhantomData<()>,
}

impl ScrapeId {
    pub fn new(source: ScrapeSource, subsource: Option<String>, id: String) -> Self {
        Self {
            source,
            subsource,
            id,
            _noinit: Default::default(),
        }
    }
}

impl Serialize for ScrapeId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if let Some(subsource) = &self.subsource {
            format!("{}-{}-{}", self.source.into_str(), subsource, self.id)
        } else {
            format!("{}-{}", self.source.into_str(), self.id)
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ScrapeId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        if let Some((head, rest)) = s.split_once('-') {
            let source = ScrapeSource::try_from_str(head)
                .ok_or(serde::de::Error::custom("Invalid source"))?;
            if let Some((subsource, id)) = rest.split_once('-') {
                Ok(ScrapeId::new(
                    source,
                    Some(subsource.to_owned()),
                    id.to_owned(),
                ))
            } else {
                Ok(ScrapeId::new(source, None, rest.to_owned()))
            }
        } else {
            Err(serde::de::Error::custom("Invalid format"))
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
    #[serde(flatten)]
    core: ScrapeCore,

    /// The additional underlying data from the scrape.
    #[serde(flatten)]
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

#[derive(EnumSetType, Debug)]
pub enum ScrapeMergeResult {
    Date,
    Title,
    URL,
}

impl<T: ScrapeStory> Scrape<T> {
    pub fn new(id: String, title: String, url: StoryUrl, date: StoryDate, data: T) -> Self {
        Self {
            core: ScrapeCore {
                source: ScrapeId::new(T::TYPE, None, id),
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
                source: ScrapeId::new(T::TYPE, Some(subsource), id),
                title,
                url,
                date,
            },
            data,
        }
    }

    pub fn merge(&mut self, other: Self) -> EnumSet<ScrapeMergeResult> {
        let mut changes = EnumSet::empty();
        if self.date != other.date {
            self.date = std::cmp::min(self.date, other.date);
            changes |= ScrapeMergeResult::Date;
        }
        let (other, other_data) = (other.core, other.data);
        if self.title != other.title {
            self.title = other.title;
            changes |= ScrapeMergeResult::Title;
        }
        if self.url != other.url {
            self.url = other.url;
            changes |= ScrapeMergeResult::URL;
        }
        self.data.merge(other_data);
        changes
    }
}

pub trait Scraper<Config: ScrapeConfigSource, Output: ScrapeStory>: Default {
    /// Given input in the correct format, scrapes raw stories.
    fn scrape(
        &self,
        args: &Config,
        input: &str,
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
                    .expect(&format!("Scrape of {:?} failed", source));
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
        for scrape in scrape_all() {
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
