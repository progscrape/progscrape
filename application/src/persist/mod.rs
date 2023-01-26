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

/// The underlying storage engine.
pub trait Storage: Send + Sync {
    fn most_recent_story(&self) -> Result<StoryDate, PersistError>;

    /// Count the docs in this index, breaking it out by index segment.
    fn story_count(&self) -> Result<StorageSummary, PersistError>;

    /// Retrieves a single, unique story from the index.
    fn get_story(
        &self,
        id: &StoryIdentifier,
    ) -> Result<Option<(Story, ScrapeCollection)>, PersistError>;

    /// Retrieves all stories in a shard.
    fn stories_by_shard(&self, shard: &str) -> Result<Vec<Story>, PersistError>;

    /// Query the current front page hot set, sorted by overall base score.
    fn query_frontpage_hot_set(&self, max_count: usize) -> Result<Vec<Story>, PersistError>;

    /// Query the current front page hot set, sorted by overall base score.
    fn query_frontpage_hot_set_detail(
        &self,
        max_count: usize,
    ) -> Result<Vec<(Story, ScrapeCollection)>, PersistError>;

    /// Query a search, scored mostly by date but may include some "hotness".
    fn query_search(
        &self,
        tagger: &StoryTagger,
        search: &str,
        max_count: usize,
    ) -> Result<Vec<Story>, PersistError>;
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
