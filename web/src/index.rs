use std::{collections::HashMap, path::Path, time::Instant};

use crate::{
    resource::BlogPost,
    web::{HostParams, WebError, BLOG_SEARCH},
};
use itertools::Itertools;
use keepcalm::{Shared, SharedMut};
use progscrape_application::{
    BackerUpper, BackupResult, IntoStoryQuery, PersistError, PersistLocation, ScrapePersistResult,
    SearchSummary, Shard, Storage, StorageFetch, StorageSummary, StorageWriter, Story,
    StoryEvaluator, StoryIdentifier, StoryIndex, StoryQuery, StoryRender, StoryScrapePayload,
};
use progscrape_scrapers::{StoryDate, StoryUrl, TypedScrape};
use serde::{Deserialize, Serialize};
use tracing::Level;

pub struct HotSet {
    stories: Vec<Story<Shard>>,
    top_tags: Vec<(String, usize)>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct IndexConfig {
    pub hot_set: HotSetConfig,
    pub max_count: usize,
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct HotSetConfig {
    /// The size of the hot set we keep resident
    pub size: usize,
    /// The max amount of jitter we may add to each story's score when fetching the hot set
    pub jitter: f32,
}

pub struct Index<S: StorageWriter> {
    pub pinned_story: SharedMut<Option<StoryUrl>>,
    storage: SharedMut<S>,
    hot_set: SharedMut<HotSet>,
    eval: Shared<StoryEvaluator>,
    blog: Shared<Vec<BlogPost>>,
    config: Shared<IndexConfig>,
}

impl<S: StorageWriter> Clone for Index<S> {
    fn clone(&self) -> Self {
        Self {
            storage: self.storage.clone(),
            hot_set: self.hot_set.clone(),
            pinned_story: self.pinned_story.clone(),
            eval: self.eval.clone(),
            blog: self.blog.clone(),
            config: self.config.clone(),
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
        blog: Shared<Vec<BlogPost>>,
        config: Shared<IndexConfig>,
    ) -> Result<Index<StoryIndex>, WebError> {
        let index = StoryIndex::new(PersistLocation::Path(path.as_ref().to_owned()))?;
        Ok(Index {
            storage: SharedMut::new(index),
            hot_set: SharedMut::new(HotSet {
                stories: vec![],
                top_tags: vec![],
            }),
            pinned_story: SharedMut::new(None),
            blog,
            eval,
            config,
        })
    }

    fn compute_hot_set(&self, stories: Vec<Story<Shard>>, now: StoryDate) -> HotSet {
        // First we'll sort these stories
        let scorer = &self.eval.read().scorer;
        let lock = self.pinned_story.read();
        let pinned = (*lock).as_ref();
        let (mut pinned, mut stories) = stories
            .into_iter()
            .partition::<Vec<Story<Shard>>, _>(|s| Some(&s.url) == pinned);
        pinned.truncate(1);
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

        pinned.append(&mut stories);
        HotSet {
            stories: pinned,
            top_tags,
        }
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

        // Fetch
        let max = self.config.read().hot_set.size;
        let v = async_run!(self.storage, |storage: &StoryIndex| {
            storage.fetch::<Shard>(&StoryQuery::FrontPage, max)
        })?;

        // TODO: We should only add this if it doesn't exist
        // for pinned in self.pinned_story.read().iter() {
        //     v.append(&mut self.fetch(StoryQuery::UrlSearch(pinned.clone()), 1).await?);
        // }
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
        host: &HostParams,
        raw_stories: impl Iterator<Item = &'a Story<Shard>>,
        offset: usize,
        count: usize,
    ) -> Vec<S> {
        raw_stories
            .skip(offset)
            .take(count)
            .enumerate()
            .map(|(index, story)| self.render(host, story, index).map(|s| s.into()))
            .filter_map(|story| story)
            .collect_vec()
    }

    fn render<'a>(
        &self,
        host: &HostParams,
        story: &'a Story<Shard>,
        order: usize,
    ) -> Option<StoryRender> {
        let mut render = story.render(&self.eval.read(), order);
        // TODO: This is a bit hacky
        if story.url.host() == "progscrape.com" {
            for blog in &*self.blog.read() {
                if blog.url == story.url {
                    render.html = blog.html.clone();
                    // render.tags = blog.tags;
                    render.tags.insert(0, host.host.to_string());
                    // TODO: Would be nice if StoryUrl preserved the URL parts
                    render.url = story.url.raw().replace(
                        "http://progscrape/",
                        &format!("{}://{}/", host.protocol, host.host),
                    );
                    return Some(render);
                }
            }
            None
        } else {
            Some(render)
        }
    }

    pub fn parse_query(&self, query: impl IntoStoryQuery) -> Result<StoryQuery, PersistError> {
        // Special case for blog
        if query.search_text() == BLOG_SEARCH {
            Ok("host:progscrape.com".into_story_query(&self.eval.read().tagger))
        } else {
            Ok(query.into_story_query(&self.eval.read().tagger))
        }
    }

    pub async fn stories_by_shard(&self, query: StoryQuery) -> Result<SearchSummary, PersistError> {
        async_run!(self.storage, |storage: &StoryIndex| {
            storage.fetch_count_by_shard(&query)
        })
    }

    pub async fn stories<S: From<StoryRender>>(
        &self,
        host: &HostParams,
        query: StoryQuery,
        offset: usize,
        count: usize,
    ) -> Result<Vec<S>, PersistError> {
        let stories = if let StoryQuery::FrontPage = query {
            self.filter_and_render(host, self.hot_set.read().stories.iter(), offset, count)
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
            self.filter_and_render(host, stories.iter(), offset, count)
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
        // Clamp to the configured max
        let max = self.config.read().max_count.min(max);
        async_run!(self.storage, |storage: &StoryIndex| {
            storage.fetch::<S>(&query, max)
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
            storage.fetch_one::<S>(&query)
        })
    }

    pub async fn fetch_detail_one(
        &self,
        id: StoryIdentifier,
    ) -> Result<Option<HashMap<String, Vec<String>>>, PersistError> {
        async_run!(self.storage, |storage: &StoryIndex| {
            storage.fetch_detail_one(&StoryQuery::ById(id))
        })
    }
}
