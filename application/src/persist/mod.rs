use crate::scrapers::TypedScrape;
use crate::story::{Story, StoryDate, StoryEvaluator, StoryIdentifier};
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod db;
mod index;
mod memindex;

pub use index::StoryIndex;
pub use memindex::MemIndex;

#[derive(Error, Debug)]
pub enum PersistError {
    #[error("SQLite error")]
    SQLiteError(#[from] rusqlite::Error),
    #[error("Tantivy error")]
    TantivyError(#[from] tantivy::TantivyError),
    #[error("Tantivy query parser error")]
    TantivyQueryError(#[from] tantivy::query::QueryParserError),
    #[error("Serialize/deserialize error")]
    SerdeError(#[from] serde_rusqlite::Error),
    #[error("Unmappable column")]
    Unmappable(),
}

#[derive(Default, Serialize, Deserialize)]
pub struct StorageSummary {
    by_shard: Vec<(String, usize)>,
    total: usize,
}

/// The underlying storage engine.
pub trait Storage: Send + Sync {
    fn most_recent_story(&self) -> StoryDate;

    /// Count the docs in this index, breaking it out by index segment.
    fn story_count(&self) -> Result<StorageSummary, PersistError>;

    /// Retrieves a single, unique story from the index.
    fn get_story(&self, id: &StoryIdentifier) -> Option<Story>;

    /// Retrieves all stories in a shard.
    fn stories_by_shard(&self, shard: &str) -> Result<Vec<Story>, PersistError>;

    /// Query the current front page hot set, sorted by overall base score.
    fn query_frontpage_hot_set(&self, max_count: usize) -> Result<Vec<Story>, PersistError>;

    /// Query a search, scored mostly by date but may include some "hotness".
    fn query_search(&self, search: String, max_count: usize) -> Result<Vec<Story>, PersistError>;
}

pub trait StorageWriter: Storage {
    /// Insert a set of scrapes, merging with existing stories if necessary.
    fn insert_scrapes<I: Iterator<Item = TypedScrape>>(
        &mut self,
        eval: &StoryEvaluator,
        scrapes: I,
    ) -> Result<(), PersistError>;
}
