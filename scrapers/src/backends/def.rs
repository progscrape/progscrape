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
    ) -> Result<(Vec<GenericScrape<Self::Output>>, Vec<String>), ScrapeError>;

    /// Extract the core scrape elements from the raw scrape.
    fn extract_core<'a>(
        &self,
        args: &Self::Config,
        input: &'a GenericScrape<Self::Output>,
    ) -> ScrapeCore<'a>;
}

pub trait ScrapeConfigSource {
    fn subsources(&self) -> Vec<String>;
    fn provide_urls(&self, subsources: Vec<String>) -> Vec<String>;
}

#[derive(Clone, Debug)]
pub struct ScrapeCore<'a> {
    /// The scrape source ID.
    pub source: &'a ScrapeId,

    /// Story title from this scrape source, potentially edited based on source (stripping suffixes, etc).
    pub title: &'a str,

    /// Story URL.
    pub url: &'a StoryUrl,

    /// Story date/time.
    pub date: StoryDate,

    /// Story tags from scrape source.
    pub tags: Vec<Cow<'a, str>>,

    /// If this story has a rank, lower is better.
    pub rank: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScrapeShared {
    pub id: ScrapeId,
    pub url: StoryUrl,
    pub raw_title: String,
    pub date: StoryDate,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenericScrape<T: ScrapeStory> {
    #[serde(flatten)]
    pub shared: ScrapeShared,
    #[serde(flatten)]
    pub data: T,
}

impl<T: ScrapeStory> std::ops::Deref for GenericScrape<T> {
    type Target = ScrapeShared;
    fn deref(&self) -> &Self::Target {
        &self.shared
    }
}

impl<T: ScrapeStory> std::ops::DerefMut for GenericScrape<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.shared
    }
}

impl<T: ScrapeStory> GenericScrape<T> {
    pub fn merge_generic(&mut self, other: Self) {}
}

macro_rules! scrape_story {
    ( $name:ident { $( $id:ident : $type:ty ),* $(,)? } ) => {
        #[derive(Serialize, Deserialize, Clone, Debug, Default)]
        pub struct $name {
            $( pub $id : $type ),*
        }

        impl $name {
            pub fn new(id: String, subsource: Option<String>, date: StoryDate, raw_title: String, url: StoryUrl, $( $id: $type ),*) -> GenericScrape<$name> {
                GenericScrape {
                    shared: ScrapeShared {
                        id: ScrapeId::new(<$name as ScrapeStory>::TYPE, subsource, id), date, raw_title, url
                    },
                    data: $name {
                        $($id),*
                    }
                }
            }

            pub fn new_with_defaults(id: String, subsource: Option<String>, date: StoryDate, raw_title: String, url: StoryUrl) -> GenericScrape<$name> {
                GenericScrape {
                    shared: ScrapeShared {
                        id: ScrapeId::new(<$name as ScrapeStory>::TYPE, subsource, id), date, raw_title, url
                    },
                    data: $name {
                        $($id : Default::default() ),*
                    }
                }
            }
        }
    };
}

pub(crate) use scrape_story;
