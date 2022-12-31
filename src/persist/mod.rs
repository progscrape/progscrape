use crate::story::Story;
use crate::scrapers::Scrape;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use thiserror::Error;

mod index;
mod db;

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

/// The underlying storage engine.
trait Storage {
    /// Insert a set of scrapes, merging with existing stories if necessary.
    fn insert_scrapes<'a, IT: AsRef<dyn Scrape>, I: Iterator<Item = IT> + 'a>(&mut self, scrape: I) -> Result<(), PersistError>;

    /// Query the current front page, scored mainly by "hotness".
    fn query_frontpage(&self, max_count: usize) -> Result<Vec<Story>, PersistError>;

    /// Query a search, scored mostly by date but may include some "hotness".
    fn query_search(&self, search: String, max_count: usize) -> Result<Vec<Story>, PersistError>;
}
