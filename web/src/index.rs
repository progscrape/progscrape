use std::{collections::HashMap, path::Path, time::Instant};

use crate::web::WebError;
use itertools::Itertools;
use keepcalm::{Shared, SharedMut};
use progscrape_application::{
    BackerUpper, BackupResult, IntoStoryQuery, PersistError, PersistLocation, ScrapePersistResult,
    Shard, Storage, StorageFetch, StorageSummary, StorageWriter, Story, StoryEvaluator,
    StoryIdentifier, StoryIndex, StoryQuery, StoryRender, StoryScrapePayload,
};
use progscrape_scrapers::{StoryDate, TypedScrape};
use tracing::Level;

pub struct HotSet {
    stories: Vec<Story<Shard>>,
    top_tags: Vec<(String, usize)>,
}

pub struct Index<S: StorageWriter> {
    pub storage: SharedMut<S>,
    pub hot_set: SharedMut<HotSet>,
    pub eval: Shared<StoryEvaluator>,
}

impl<S: StorageWriter> Clone for Index<S> {
    fn clone(&self) -> Self {
        Self {
            storage: self.storage.clone(),
            hot_set: self.hot_set.clone(),
            eval: self.eval.clone(),
        }
    }
}

macro_rules! async_run {
    ($storage:expr, $block:expr) => {{
        let storage = $storage.clone();
        #[allow(clippy::redundant_closure_call)]
        tokio::task::spawn_blocking(move || {
            let storage = storage.read();
            $block(&storage)
        })
        .await
        .map_err(|_| PersistError::UnexpectedError("Storage fetch panicked".into()))?
    }};
}

macro_rules! async_run_write {
    ($storage:expr, $block:expr) => {{
        let storage = $storage.clone();
        #[allow(clippy::redundant_closure_call)]
        tokio::task::spawn_blocking(move || {
            let mut storage = storage.write();
            $block(&mut storage)
        })
        .await
        .map_err(|_| PersistError::UnexpectedError("Storage fetch panicked".into()))?
    }};
}

impl Index<StoryIndex> {
    pub fn initialize_with_persistence<P: AsRef<Path>>(
        path: P,
        eval: Shared<StoryEvaluator>,
    ) -> Result<Index<StoryIndex>, WebError> {
        let index = StoryIndex::new(PersistLocation::Path(path.as_ref().to_owned()))?;
        Ok(Index {
            storage: SharedMut::new(index),
            hot_set: SharedMut::new(HotSet {
                stories: vec![],
                top_tags: vec![],
            }),
            eval,
        })
    }

    fn compute_hot_set(&self, mut stories: Vec<Story<Shard>>, now: StoryDate) -> HotSet {
        // First we'll sort these stories
        let scorer = &self.eval.read().scorer;
        stories
            .sort_by_cached_key(|x| ((x.score + scorer.score_age(now - x.date)) * -1000.0) as i32);

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
        let storage = self.storage.read();
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
        let now = self.storage.read().most_recent_story()?;
        let v = self.fetch(StoryQuery::FrontPage, 500).await?;
        *self.hot_set.write() = self.compute_hot_set(v, now);
        Ok(())
    }

    /// Borrows the hot set
    fn with_hot_set<T>(
        &self,
        f: impl FnOnce(&Vec<Story<Shard>>) -> Result<T, PersistError>,
    ) -> Result<T, PersistError> {
        f(&self.hot_set.read().stories)
    }

    /// Re-index and refresh the hot set using the most current configuration.
    pub async fn reindex_hot_set(&self) -> Result<Vec<ScrapePersistResult>, PersistError> {
        // Get the hot set story IDs
        let story_ids = self.with_hot_set(|hot_set| {
            Ok(hot_set.iter().map(|story| story.id.clone()).collect_vec())
        })?;

        // Reindex the stories
        let eval_clone = self.eval.clone();
        let res = async_run_write!(self.storage, |storage: &mut StoryIndex| {
            storage.reinsert_stories(&eval_clone.read(), story_ids)
        })?;

        // Refresh the hot set from the index
        self.refresh_hot_set().await?;

        Ok(res)
    }

    fn filter_and_render<'a, S: From<StoryRender>>(
        &self,
        raw_stories: impl Iterator<Item = &'a Story<Shard>>,
        offset: usize,
        count: usize,
    ) -> Vec<S> {
        raw_stories
            .skip(offset)
            .take(count)
            .enumerate()
            .map(|(index, story)| story.render(&self.eval.read(), index).into())
            .collect_vec()
    }

    pub fn parse_query(&self, query: impl IntoStoryQuery) -> Result<StoryQuery, PersistError> {
        Ok(query.into_story_query(&self.eval.read().tagger))
    }

    pub async fn stories<S: From<StoryRender>>(
        &self,
        query: StoryQuery,
        offset: usize,
        count: usize,
    ) -> Result<Vec<S>, PersistError> {
        let stories = if let StoryQuery::FrontPage = query {
            self.filter_and_render(self.hot_set.read().stories.iter(), offset, count)
        } else {
            let start = Instant::now();
            let (query_log, query_text) = if tracing::enabled!(Level::INFO) {
                (
                    Some(format!("{query:?}")),
                    Some(format!("{:?}", query.query_text().to_string())),
                )
            } else {
                (None, None)
            };
            let stories = self.fetch::<Shard>(query, 100).await?;
            let elapsed_ms = start.elapsed().as_millis();
            tracing::info!(
                "Search query_text={} search_time={elapsed_ms}ms query={}",
                query_text.unwrap_or_default(),
                query_log.unwrap_or_default()
            );
            self.filter_and_render(stories.iter(), offset, count)
        };

        Ok(stories)
    }

    pub fn top_tags(&self, limit: usize) -> Result<Vec<(String, usize)>, PersistError> {
        let top_tags = &self.hot_set.read().top_tags;
        let tagger = &self.eval.read().tagger;
        Ok(top_tags
            .iter()
            .take(limit)
            .map(|(s, count)| (tagger.make_display_tag(s), *count))
            .collect_vec())
    }

    pub async fn insert_scrapes<I: IntoIterator<Item = TypedScrape> + Send + 'static>(
        &self,
        scrapes: I,
    ) -> Result<Vec<ScrapePersistResult>, PersistError> {
        let eval = self.eval.clone();
        async_run_write!(self.storage, move |storage: &mut StoryIndex| {
            storage.insert_scrapes(&eval.read(), scrapes)
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
