use std::{collections::HashMap, path::Path};

use itertools::Itertools;
use progscrape_application::{
    BackerUpper, BackupResult, PersistError, PersistLocation, ScrapePersistResult, Shard, Storage,
    StorageFetch, StorageSummary, StorageWriter, Story, StoryEvaluator, StoryIdentifier,
    StoryIndex, StoryQuery, StoryRender, StoryScrapePayload,
};
use progscrape_scrapers::{StoryDate, TypedScrape};

use crate::{
    types::{Shared, SharedRW},
    web::WebError,
};

pub struct HotSet {
    stories: Vec<Story<Shard>>,
    top_tags: Vec<String>,
}

pub struct Index<S: StorageWriter> {
    pub storage: SharedRW<S>,
    pub hot_set: SharedRW<HotSet>,
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
            let storage = storage.lock_read();
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
            let mut storage = storage.lock_write();
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
            storage: SharedRW::new(index),
            hot_set: SharedRW::new(HotSet {
                stories: vec![],
                top_tags: vec![],
            }),
        })
    }

    fn compute_hot_set(
        mut stories: Vec<Story<Shard>>,
        now: StoryDate,
        eval: Shared<StoryEvaluator>,
    ) -> HotSet {
        // First we'll sort these stories
        stories.sort_by_cached_key(|x| {
            ((x.score + eval.scorer.score_age(now - x.date)) * -1000.0) as i32
        });

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
        let storage = self.storage.lock_read();
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

    pub async fn refresh_hot_set(&self, eval: Shared<StoryEvaluator>) -> Result<(), PersistError> {
        let now = self.storage.lock_read().most_recent_story()?;
        let v = self.fetch(StoryQuery::FrontPage(), 500).await?;
        *self.hot_set.lock_write() = Self::compute_hot_set(v, now, eval);
        Ok(())
    }

    /// Borrows the hot set
    fn with_hot_set<T>(
        &self,
        f: impl FnOnce(&Vec<Story<Shard>>) -> Result<T, PersistError>,
    ) -> Result<T, PersistError> {
        f(&self.hot_set.lock_read().stories)
    }

    /// Re-index and refresh the hot set using the most current configuration.
    pub async fn reindex_hot_set(
        &self,
        eval: Shared<StoryEvaluator>,
    ) -> Result<Vec<ScrapePersistResult>, PersistError> {
        // Get the hot set story IDs
        let story_ids = self.with_hot_set(|hot_set| {
            Ok(hot_set.iter().map(|story| story.id.clone()).collect_vec())
        })?;

        // Reindex the stories
        let eval_clone = eval.clone();
        let res = async_run_write!(self.storage, |storage: &mut StoryIndex| {
            storage.reinsert_stories(&eval_clone, story_ids)
        })?;

        // Refresh the hot set from the index
        self.refresh_hot_set(eval).await?;

        Ok(res)
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
                .sorted_by(|a, b| a.date.cmp(&b.date).reverse())
                .enumerate()
                .map(|(index, story)| story.render(eval, index).into())
                .collect_vec()
        } else {
            self.with_hot_set(|hot_set| {
                Ok(hot_set
                    .iter()
                    .take(count)
                    .enumerate()
                    .map(|(index, story)| story.render(eval, index).into())
                    .collect_vec())
            })?
        };
        Ok(stories)
    }

    pub async fn top_tags(&self) -> Result<Vec<String>, PersistError> {
        Ok(self.hot_set.lock_read().top_tags.clone())
    }

    pub async fn insert_scrapes<I: IntoIterator<Item = TypedScrape> + Send + 'static>(
        &self,
        eval: Shared<StoryEvaluator>,
        scrapes: I,
    ) -> Result<Vec<ScrapePersistResult>, PersistError> {
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

    pub async fn fetch_detail_one(
        &self,
        id: StoryIdentifier,
    ) -> Result<Option<HashMap<String, Vec<String>>>, PersistError> {
        async_run!(self.storage, |storage: &StoryIndex| {
            storage.fetch_detail_one(StoryQuery::ById(id))
        })
    }
}
