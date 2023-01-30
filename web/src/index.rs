use std::{
    path::Path,
    sync::{Arc, RwLock},
};

use progscrape_application::{
    PersistError, PersistLocation, Storage, StorageFetch, StorageSummary, StorageWriter, Story,
    StoryEvaluator, StoryIndex, StoryQuery, StoryScrapePayload,
};
use progscrape_scrapers::{StoryDate, TypedScrape};

use crate::web::WebError;

pub struct Index<S: StorageWriter> {
    pub storage: Arc<RwLock<S>>,
}

impl<S: StorageWriter> Clone for Index<S> {
    fn clone(&self) -> Self {
        Self {
            storage: self.storage.clone(),
        }
    }
}

macro_rules! async_run {
    ($storage:expr, $block:expr) => {{
        let storage = $storage.clone();
        tokio::task::spawn_blocking(move || {
            let storage = storage.read().expect("Failed to lock storage for read");
            $block(&storage)
        })
        .await
        .map_err(|_| PersistError::UnexpectedError("Storage fetch panicked".into()))?
    }};
}

macro_rules! async_run_write {
    ($storage:expr, $block:expr) => {{
        let storage = $storage.clone();
        tokio::task::spawn_blocking(move || {
            let mut storage = storage.write().expect("Failed to lock storage for write");
            $block(&mut storage)
        })
        .await
        .map_err(|_| PersistError::UnexpectedError("Storage fetch panicked".into()))?
    }};
}

impl Index<StoryIndex> {
    pub fn initialize_with_persistence<P: AsRef<Path>>(
        path: P,
    ) -> Result<Index<StoryIndex>, WebError> {
        let index = StoryIndex::new(PersistLocation::Path(path.as_ref().to_owned()))?;
        Ok(Index {
            storage: Arc::new(RwLock::new(index)),
        })
    }

    pub async fn insert_scrapes<I: Iterator<Item = TypedScrape> + Send + 'static>(
        &self,
        eval: Arc<StoryEvaluator>,
        scrapes: I,
    ) -> Result<(), PersistError> {
        async_run_write!(self.storage, move |storage: &mut StoryIndex| {
            storage.insert_scrapes(&eval, scrapes)
        })
    }

    pub async fn most_recent_story(&self) -> Result<StoryDate, PersistError> {
        async_run!(self.storage, |storage: &StoryIndex| {
            storage.most_recent_story()
        })
    }

    pub async fn story_count(&self) -> Result<StorageSummary, PersistError> {
        async_run!(self.storage, |storage: &StoryIndex| {
            storage.story_count()
        })
    }

    pub async fn fetch<S: StoryScrapePayload + 'static>(
        &self,
        query: StoryQuery,
        max: usize,
    ) -> Result<Vec<Story<S>>, PersistError>
    where
        StoryIndex: StorageFetch<S>,
    {
        async_run!(self.storage, |storage: &StoryIndex| {
            storage.fetch::<S>(query, max)
        })
    }

    pub async fn fetch_one<S: StoryScrapePayload + 'static>(
        &self,
        query: StoryQuery,
    ) -> Result<Option<Story<S>>, PersistError>
    where
        StoryIndex: StorageFetch<S>,
    {
        async_run!(self.storage, |storage: &StoryIndex| {
            storage.fetch_one::<S>(query)
        })
    }
}
