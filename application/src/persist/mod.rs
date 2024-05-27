use std::{collections::HashMap, ops::AddAssign, path::PathBuf};

use crate::story::{Story, StoryEvaluator, StoryIdentifier, StoryTagger};
use progscrape_scrapers::{ScrapeCollection, StoryDate, TypedScrape};
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod backerupper;
mod db;
mod index;
mod memindex;
mod scrapestore;
mod shard;

pub use backerupper::{BackerUpper, BackupResult};
pub use index::StoryIndex;
pub use memindex::MemIndex;
pub use shard::Shard;

use self::shard::ShardRange;

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
pub struct ShardSummary {
    pub story_count: usize,
    pub scrape_count: usize,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct StorageSummary {
    pub by_shard: Vec<(String, ShardSummary)>,
    pub total: ShardSummary,
}

/// The type of story fetch to perform.
pub enum StoryQuery {
    /// A single story.
    ById(StoryIdentifier),
    /// All stories from a given shard.
    ByShard(Shard),
    /// Front page stories.
    FrontPage(),
    /// Stories matching a tag query (second item in tuple is the alternative).
    TagSearch(String, Option<String>),
    /// Stories matching a domain query.
    DomainSearch(String),
    /// Stories matching a text search.
    TextSearch(String),
}

impl StoryQuery {
    pub fn from_search(tagger: &StoryTagger, search: &str) -> Self {
        // This isn't terribly smart, buuuuut it allows us to search either a tag or site
        if let Some(tag) = tagger.check_tag_search(search) {
            let alt = if tag.eq_ignore_ascii_case(search) {
                None
            } else {
                Some(search.to_ascii_lowercase())
            };
            StoryQuery::TagSearch(tag.to_string(), alt)
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
    /// Returns the most recent story date.
    fn most_recent_story(&self) -> Result<StoryDate, PersistError>;

    /// Returns the range of shards for this index.
    fn shard_range(&self) -> Result<ShardRange, PersistError>;

    /// Count the docs in this index, breaking it out by index segment.
    fn story_count(&self) -> Result<StorageSummary, PersistError>;

    /// Count the docs matching the query, at most max.
    fn fetch_count(&self, query: StoryQuery, max: usize) -> Result<usize, PersistError>;

    /// Fetches the index-specific story details for a single story.
    fn fetch_detail_one(
        &self,
        query: StoryQuery,
    ) -> Result<Option<HashMap<String, Vec<String>>>, PersistError>;

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
    fn insert_scrapes<I: IntoIterator<Item = TypedScrape>>(
        &mut self,
        eval: &StoryEvaluator,
        scrapes: I,
    ) -> Result<Vec<ScrapePersistResult>, PersistError>;

    /// Insert a set of pre-digested stories. Assumes that the underlying story does not exist and no merging is required.
    fn insert_scrape_collections<I: IntoIterator<Item = ScrapeCollection>>(
        &mut self,
        eval: &StoryEvaluator,
        stories: I,
    ) -> Result<Vec<ScrapePersistResult>, PersistError>;

    /// Given a set of existing stories, re-inserts them into the index with updated scores and tags.
    fn reinsert_stories<I: IntoIterator<Item = StoryIdentifier>>(
        &mut self,
        eval: &StoryEvaluator,
        stories: I,
    ) -> Result<Vec<ScrapePersistResult>, PersistError>;
}

#[derive(Debug, Serialize, PartialEq, Eq, Ord, PartialOrd, Hash)]
pub enum ScrapePersistResult {
    /// The story was merged with an existing story whilst we tried to re-insert it.
    MergedWithExistingStory,
    /// The scrape has already been added.
    AlreadyPartOfExistingStory,
    /// This is a new story.
    NewStory,
    /// The story was not found whilst we tried to re-insert it.
    NotFound,
}

#[derive(Default, Debug, Serialize)]
pub struct ScrapePersistResultSummary {
    pub merged: usize,
    pub existing: usize,
    pub new: usize,
    pub not_found: usize,
}

impl AddAssign for ScrapePersistResultSummary {
    fn add_assign(&mut self, rhs: Self) {
        self.merged += rhs.merged;
        self.existing += rhs.existing;
        self.new += rhs.new;
        self.not_found += rhs.not_found;
    }
}

pub trait ScrapePersistResultSummarizer {
    fn summary(&self) -> ScrapePersistResultSummary;
}

impl ScrapePersistResultSummarizer for Vec<ScrapePersistResult> {
    fn summary(&self) -> ScrapePersistResultSummary {
        let mut summary = ScrapePersistResultSummary::default();
        for x in self {
            match x {
                &ScrapePersistResult::MergedWithExistingStory => summary.merged += 1,
                &ScrapePersistResult::AlreadyPartOfExistingStory => summary.existing += 1,
                &ScrapePersistResult::NewStory => summary.new += 1,
                &ScrapePersistResult::NotFound => summary.not_found += 1,
            }
        }
        summary
    }
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
