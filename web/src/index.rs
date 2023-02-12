use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, RwLock},
};

use itertools::Itertools;
use progscrape_application::{
    BackerUpper, BackupResult, PersistError, PersistLocation, Shard, Storage, StorageFetch,
    StorageSummary, StorageWriter, Story, StoryEvaluator, StoryIndex, StoryQuery, StoryRender,
    StoryScrapePayload,
};
use progscrape_scrapers::{StoryDate, TypedScrape};

use crate::web::WebError;

pub struct HotSet {
    stories: Vec<Story<Shard>>,
    top_tags: Vec<String>,
}

pub struct Index<S: StorageWriter> {
    pub storage: Arc<RwLock<S>>,
    pub hot_set: Arc<RwLock<HotSet>>,
}

impl<S: StorageWriter> Clone for Index<S> {
    fn clone(&self) -> Self {
        Self {
            storage: self.storage.clone(),
            hot_set: self.hot_set.clone(),
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
        let stories = index.fetch(StoryQuery::FrontPage(), 500)?;
        let hot_set = Self::compute_hot_set(stories);
        Ok(Index {
            storage: Arc::new(RwLock::new(index)),
            hot_set: Arc::new(RwLock::new(hot_set)),
        })
    }

    fn compute_hot_set(stories: Vec<Story<Shard>>) -> HotSet {
        // Count each item
        let mut tag_counts = HashMap::new();
        for story in &stories {
            // We won't count tags from any self posts because these tend to dominate the trending tags in two
            // way: first, by spamming the source's domain, and second in cases like Python/Rust where there are
            // lots of self posts (with low quality in some cases).
            if story.is_likely_self_post() {
                continue;
            }
            for tag in story.raw_tags() {
                tag_counts
                    .entry(tag)
                    .and_modify(|x| *x += 1)
                    .or_insert(1_usize);
            }
        }

        // Naive sort and truncate (fine for the number of tags we're dealing with)
        let top_tags = tag_counts
            .into_iter()
            .filter(|(_, count)| *count > 1)
            .sorted_by_cached_key(|(_, count)| -((*count) as i64))
            .take(50)
            .map(|(tag, _)| tag)
            .collect_vec();

        HotSet { stories, top_tags }
    }

    /// Back up the current index to the given path. The return value of this function is a little convoluted because we
    /// don't necessarily want to fail the whole operation.
    pub fn backup(
        &self,
        backup_path: &Path,
    ) -> Result<Vec<(Shard, Result<BackupResult, PersistError>)>, PersistError> {
        let backup = BackerUpper::new(backup_path);
        let storage = self.storage.read().expect("Poisoned");
        let shard_range = storage.shard_range()?;
        let results = storage.with_scrapes(|scrapes| backup.backup_range(scrapes, shard_range));
        for (shard, result) in &results {
            match result {
                Ok(res) => tracing::info!("Backed up shard {}: {:?}", shard.to_string(), res),
                Err(e) => {
                    tracing::error!("Backed up shard {}: FAILED {:?}", shard.to_string(), e)
                }
            }
        }
        Ok(results)
    }

    pub async fn refresh_hot_set(&self) -> Result<(), PersistError> {
        let v = self.fetch(StoryQuery::FrontPage(), 500).await?;
        *self.hot_set.write().expect("Failed to lock hot set") = Self::compute_hot_set(v);
        Ok(())
    }

    pub async fn stories<S: From<StoryRender>>(
        &self,
        search: Option<impl AsRef<str>>,
        eval: &StoryEvaluator,
        count: usize,
    ) -> Result<Vec<S>, PersistError> {
        let stories = if let Some(search) = search {
            self.fetch::<Shard>(StoryQuery::from_search(&eval.tagger, search.as_ref()), 30)
                .await?
                .iter()
                .enumerate()
                .map(|(index, story)| story.render(eval, index).into())
                .collect_vec()
        } else {
            self.hot_set
                .read()
                .expect("Failed to lock hot set")
                .stories
                .iter()
                .enumerate()
                .map(|(index, story)| story.render(eval, index).into())
                .collect_vec()
        };
        Ok(stories)
    }

    pub async fn hot_set(&self) -> Result<Vec<Story<Shard>>, PersistError> {
        let v = self
            .hot_set
            .read()
            .expect("Failed to lock hot set")
            .stories
            .clone();
        Ok(v)
    }

    pub async fn top_tags(&self) -> Result<Vec<String>, PersistError> {
        Ok(self
            .hot_set
            .read()
            .expect("Failed to lock hot set")
            .top_tags
            .clone())
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
