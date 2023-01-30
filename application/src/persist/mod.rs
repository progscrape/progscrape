use std::path::PathBuf;

use crate::story::{Story, StoryEvaluator, StoryIdentifier, StoryTagger};
use progscrape_scrapers::{ScrapeCollection, StoryDate, TypedScrape};
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod db;
mod index;
mod memindex;
mod scrapestore;
mod shard;

pub use index::StoryIndex;
pub use memindex::MemIndex;
pub use shard::Shard;

#[derive(Error, Debug)]
pub enum PersistError {
    #[error("SQLite error")]
    SQLiteError(#[from] rusqlite::Error),
    #[error("Tantivy error")]
    TantivyError(#[from] tantivy::TantivyError),
    #[error("Tantivy error")]
    TantivyPathError(#[from] tantivy::directory::error::OpenDirectoryError),
    #[error("Tantivy query parser error")]
    TantivyQueryError(#[from] tantivy::query::QueryParserError),
    #[error("JSON error")]
    JsonError(#[from] serde_json::Error),
    #[error("Serialize/deserialize error")]
    SerdeError(#[from] serde_rusqlite::Error),
    #[error("I/O error")]
    IOError(#[from] std::io::Error),
    #[error("Unexpected error")]
    UnexpectedError(String),
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct StorageSummary {
    pub by_shard: Vec<(String, usize)>,
    pub total: usize,
}

/// The type of story fetch to perform.
pub enum StoryQuery {
    /// A single story.
    ById(StoryIdentifier),
    /// All stories from a given shard.
    ByShard(Shard),
    /// Front page stories.
    FrontPage(),
    /// Stories matching a tag query.
    TagSearch(String),
    /// Stories matching a domain query.
    DomainSearch(String),
    /// Stories matching a text search.
    TextSearch(String),
}

impl StoryQuery {
    pub fn from_search(tagger: &StoryTagger, search: &str) -> Self {
        // This isn't terribly smart, buuuuut it allows us to search either a tag or site
        if let Some(tag) = tagger.check_tag_search(search) {
            StoryQuery::TagSearch(tag.to_string())
        } else if search.contains('.') {
            StoryQuery::DomainSearch(search.to_string())
        } else {
            StoryQuery::TextSearch(search.to_string())
        }
    }
}

pub trait StoryScrapePayload: Send + Sync {}

impl StoryScrapePayload for () {}
impl StoryScrapePayload for Shard {}
impl StoryScrapePayload for TypedScrape {}

pub trait StorageFetch<S: StoryScrapePayload> {
    fn fetch_type(&self, query: StoryQuery, max: usize) -> Result<Vec<Story<S>>, PersistError>;
}

/// The underlying storage engine.
pub trait Storage: Send + Sync {
    fn most_recent_story(&self) -> Result<StoryDate, PersistError>;

    /// Count the docs in this index, breaking it out by index segment.
    fn story_count(&self) -> Result<StorageSummary, PersistError>;

    /// Count the docs matching the query, at most max.
    fn fetch_count(&self, query: StoryQuery, max: usize) -> Result<usize, PersistError>;

    /// Fetch a list of stories with the specified payload type.
    #[inline(always)]
    fn fetch<S: StoryScrapePayload>(
        &self,
        query: StoryQuery,
        max: usize,
    ) -> Result<Vec<Story<S>>, PersistError>
    where
        Self: StorageFetch<S>,
    {
        <Self as StorageFetch<S>>::fetch_type(self, query, max)
    }

    /// Fetch a single story with the specified payload type.
    #[inline(always)]
    fn fetch_one<S: StoryScrapePayload>(
        &self,
        query: StoryQuery,
    ) -> Result<Option<Story<S>>, PersistError>
    where
        Self: StorageFetch<S>,
    {
        Ok(<Self as StorageFetch<S>>::fetch_type(self, query, 1)?
            .into_iter()
            .next())
    }
}

pub trait StorageWriter: Storage {
    /// Insert a set of scrapes, merging with existing stories if necessary.
    fn insert_scrapes<I: Iterator<Item = TypedScrape>>(
        &mut self,
        eval: &StoryEvaluator,
        scrapes: I,
    ) -> Result<(), PersistError>;

    /// Insert a set of pre-digested stories. Assumes that the underlying story does not exist and no merging is required.
    fn insert_scrape_collections<I: Iterator<Item = ScrapeCollection>>(
        &mut self,
        eval: &StoryEvaluator,
        stories: I,
    ) -> Result<(), PersistError>;
}

#[derive(Debug)]
pub enum ScrapePersistResult {
    MergedWithExistingStory,
    AlreadyPartOfExistingStory,
    NewStory,
}

#[derive(Clone, Debug)]
/// Where is this persistence engine storing data?
pub enum PersistLocation {
    /// In-memory.
    Memory,
    /// At a given path.
    Path(PathBuf),
}

impl PersistLocation {
    pub fn join<P: AsRef<std::path::Path>>(&self, p: P) -> PersistLocation {
        match self {
            PersistLocation::Memory => PersistLocation::Memory,
            PersistLocation::Path(path) => PersistLocation::Path(path.join(p)),
        }
    }
}
