use itertools::Itertools;

use tantivy::collector::TopDocs;
use tantivy::query::{AllQuery, PhraseQuery, Query, TermQuery};
use tantivy::{schema::*, DocAddress, Searcher};

use progscrape_scrapers::{ScrapeCollection, ScrapeCore, StoryDate, StoryUrl, TypedScrape};

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use crate::persist::index::indexshard::{StoryIndexShard, StoryLookup, StoryLookupId};
use crate::persist::scrapestore::ScrapeStore;
use crate::persist::shard::{ShardOrder, ShardRange};
use crate::persist::{Shard, StoryQuery};
use crate::story::{StoryCollector, StoryTagger, TagSet};
use crate::{
    MemIndex, PersistError, PersistLocation, Storage, StorageSummary, StorageWriter, Story,
    StoryEvaluator, StoryIdentifier,
};

use super::indexshard::StoryInsert;
use super::schema::StorySchema;

const MEMORY_ARENA_SIZE: usize = 50_000_000;
const STORY_INDEXING_CHUNK_SIZE: usize = 10000;
const SCRAPE_PROCESSING_CHUNK_SIZE: usize = 1000;

struct IndexCache {
    cache: HashMap<Shard, Arc<RwLock<StoryIndexShard>>>,
    range: ShardRange,
}

pub struct StoryIndex {
    index_cache: RwLock<IndexCache>,
    location: PersistLocation,
    scrape_db: ScrapeStore,
    schema: StorySchema,
}

impl StoryIndex {
    pub fn new(location: PersistLocation) -> Result<Self, PersistError> {
        // TODO: This start date needs to be dynamic
        let scrape_db = ScrapeStore::new(location.clone())?;
        tracing::info!("Initialized StoryIndex at {:?}", location);

        // Determine the min/max shard, if any
        let mut range = ShardRange::default();
        if let PersistLocation::Path(path) = &location {
            for d in std::fs::read_dir(path)?.flatten() {
                if let Some(s) = d.file_name().to_str() {
                    if let Some(shard) = Shard::from_string(s) {
                        range.include(shard);
                    }
                }
            }
        }

        tracing::info!("Found shards {:?}", range);

        let new = Self {
            index_cache: RwLock::new(IndexCache {
                cache: HashMap::new(),
                range,
            }),
            location,
            scrape_db,
            schema: StorySchema::instantiate_global_schema(),
        };

        Ok(new)
    }

    pub fn shards(&self) -> ShardRange {
        self.index_cache.read().expect("Poisoned").range
    }

    fn get_shard(&self, shard: Shard) -> Result<Arc<RwLock<StoryIndexShard>>, PersistError> {
        let mut lock = self.index_cache.write().expect("Poisoned");
        if let Some(shard) = lock.cache.get(&shard) {
            Ok(shard.clone())
        } else {
            tracing::info!("Creating shard {}", shard.to_string());
            let new_shard =
                StoryIndexShard::initialize(self.location.clone(), shard, self.schema.clone())?;
            lock.range.include(shard);
            Ok(lock
                .cache
                .entry(shard)
                .or_insert(Arc::new(RwLock::new(new_shard)))
                .clone())
        }
    }

    #[inline(always)]
    fn with_searcher<F: FnMut(Shard, &Searcher, &StorySchema) -> T, T>(
        &self,
        shard: Shard,
        mut f: F,
    ) -> Result<T, PersistError> {
        let shard_index = self.get_shard(shard)?;
        let shard_index = shard_index.read().expect("Poisoned");
        Ok(shard_index.with_searcher(|searcher, schema| f(shard, searcher, schema))?)
    }

    fn create_scrape_id_from_scrape_core(scrape_core: &ScrapeCore) -> String {
        format!(
            "{}:{}",
            Shard::from_date_time(scrape_core.date).to_string(),
            scrape_core.source
        )
    }

    fn create_scrape_id_from_scrape(scrape: &TypedScrape) -> String {
        format!(
            "{}:{}",
            Shard::from_date_time(scrape.date).to_string(),
            scrape.id
        )
    }

    fn create_story_insert<'a>(eval: &StoryEvaluator, story: &'a ScrapeCollection) -> StoryInsert {
        // TODO: We could be creating the doc directly here instead of allocating
        let extracted = story.extract(&eval.extractor);
        let score = eval.scorer.score(&extracted);
        let scrape_ids = extracted
            .scrapes
            .values()
            .map(Self::create_scrape_id_from_scrape_core)
            .collect_vec();
        let title = extracted.title().to_owned();
        let mut tags = TagSet::new();
        eval.tagger.tag(&title, &mut tags);
        for tag in extracted.tags() {
            tags.add(tag);
        }
        let url = extracted.url();
        let id = StoryIdentifier::new(story.earliest, extracted.url().normalization()).to_base64();
        let doc = StoryInsert {
            id,
            host: url.host().to_owned(),
            url: url.raw().to_owned(),
            url_norm: url.normalization().string().to_owned(),
            url_norm_hash: url.normalization().hash(),
            score: score as f64,
            date: story.earliest.timestamp(),
            title,
            scrape_ids,
            tags,
        };
        doc
    }

    fn insert_scrape_batch<'a, I: Iterator<Item = TypedScrape> + 'a>(
        &mut self,
        eval: &StoryEvaluator,
        scrapes: I,
    ) -> Result<(), PersistError> {
        let one_month = Duration::from_secs(60 * 60 * 24 * 30).as_secs() as i64;
        let mut writers = HashMap::new();

        let mut memindex = MemIndex::default();
        memindex.insert_scrapes(scrapes)?;

        for scrape in memindex.get_all_stories() {
            let shard = Shard::from_date_time(scrape.earliest);
            let shard_index = self.get_shard(shard)?;
            let mut shard_index = shard_index.write().expect("Poisoned");
            let writer = if let Some(writer) = writers.get_mut(&shard) {
                writer
            } else {
                let writer = shard_index.index.writer(MEMORY_ARENA_SIZE)?;
                writers.entry(shard).or_insert(writer)
            };

            let searcher = shard_index.index.reader()?.searcher();
            let lookup = StoryLookupId {
                url_norm_hash: scrape.url().normalization().hash(),
                date: scrape.earliest.timestamp(),
            };
            let lookup = HashSet::from_iter([lookup]);
            // TODO: Should be batching
            let result = shard_index.lookup_stories(&searcher, lookup, (-one_month)..one_month)?;
            let lookup = result.into_iter().next().expect("TODO");
            let insert_type = match lookup {
                StoryLookup::Found(_id, doc) => shard_index.add_scrape_id(
                    writer,
                    &searcher,
                    doc,
                    scrape
                        .scrapes
                        .values()
                        .map(Self::create_scrape_id_from_scrape)
                        .collect(),
                )?,
                StoryLookup::Unfound(_id) => {
                    let doc = Self::create_story_insert(eval, &scrape);
                    shard_index.insert_story_document(writer, doc)?
                }
            };
            tracing::debug!(
                "Inserted scrapes {:?}: {:?}",
                scrape.scrapes.keys(),
                insert_type
            );
        }

        let writer_count = writers.len();
        tracing::info!("Commiting {} writer(s)", writer_count);
        let commit_start = Instant::now();
        for (shard, writer) in writers.iter_mut() {
            writer.commit()?;
            let shard = self.get_shard(*shard)?;
            let mut shard = shard.write().expect("Poisoned");
            shard.reader.reload()?;
            shard.searcher = shard.reader.searcher();
        }
        for writer in writers.into_values() {
            writer.wait_merging_threads()?;
        }
        tracing::info!(
            "Committed {} writer(s) in {} second(s)...",
            writer_count,
            commit_start.elapsed().as_secs()
        );

        Ok(())
    }

    /// Insert a list of scrapes into the index.
    fn insert_scrapes<I: Iterator<Item = TypedScrape>>(
        &mut self,
        eval: &StoryEvaluator,
        scrapes: I,
    ) -> Result<(), PersistError> {
        let v = scrapes.collect_vec();

        tracing::info!("Storing raw scrapes...");
        self.scrape_db.insert_scrape_batch(v.iter())?;

        tracing::info!("Indexing scrapes...");
        self.insert_scrape_batch(eval, v.into_iter())?;

        Ok(())
    }

    fn insert_scrape_collections<I: Iterator<Item = ScrapeCollection>>(
        &mut self,
        eval: &StoryEvaluator,
        scrape_collections: I,
    ) -> Result<(), PersistError> {
        let mut writers = HashMap::new();
        let start = Instant::now();
        let mut total = 0;
        for scrape_collections in &scrape_collections.chunks(STORY_INDEXING_CHUNK_SIZE) {
            tracing::info!("Indexing chunk...");
            let start_chunk = Instant::now();
            let mut count = 0;
            let mut scrapes_batch = vec![];

            for story in scrape_collections {
                count += 1;
                let shard = Shard::from_date_time(story.earliest);
                let shard_index = self.get_shard(shard)?;
                let mut shard_index = shard_index.write().expect("Poisoned");
                let writer = if let Some(writer) = writers.get_mut(&shard) {
                    writer
                } else {
                    let writer = shard_index.index.writer(MEMORY_ARENA_SIZE)?;
                    writers.entry(shard).or_insert(writer)
                };
                let doc = Self::create_story_insert(eval, &story);
                shard_index.insert_story_document(writer, doc)?;

                let scrapes = story.scrapes.into_values();
                scrapes_batch.extend(scrapes);

                if scrapes_batch.len() > SCRAPE_PROCESSING_CHUNK_SIZE {
                    self.scrape_db.insert_scrape_batch(scrapes_batch.iter())?;
                    scrapes_batch.clear();
                }
            }
            self.scrape_db.insert_scrape_batch(scrapes_batch.iter())?;
            scrapes_batch.clear();
            total += count;
            tracing::info!(
                "Indexed chunk of {} stories in {} second(s)...",
                count,
                start_chunk.elapsed().as_secs()
            );
        }
        tracing::info!(
            "Indexed total of {} stories in {} second(s)...",
            total,
            start.elapsed().as_secs()
        );

        let writer_count = writers.len();
        tracing::info!("Commiting {} writer(s)", writer_count);
        let commit_start = Instant::now();
        for (shard, writer) in writers.iter_mut() {
            writer.commit()?;
            let shard = self.get_shard(*shard)?;
            let mut shard = shard.write().expect("Poisoned");
            shard.reader.reload()?;
            shard.searcher = shard.reader.searcher();
        }
        for writer in writers.into_values() {
            writer.wait_merging_threads()?;
        }
        tracing::info!(
            "Committed {} writer(s) in {} second(s)...",
            writer_count,
            commit_start.elapsed().as_secs()
        );
        Ok(())
    }

    fn lookup_story(
        &self,
        index: &StoryIndexShard,
        doc_address: DocAddress,
    ) -> Result<Story, PersistError> {
        let story = index.lookup_story(doc_address)?;
        let url = StoryUrl::parse(story.url).expect("Failed to parse URL");
        let date = StoryDate::from_seconds(story.date).expect("Failed to re-parse date");
        let score = story.score as f32;
        Ok(Story::new_from_parts(
            story.title,
            url,
            date,
            score,
            story.tags,
            story.scrape_ids,
        ))
    }

    fn lookup_story_and_scrapes(
        &self,
        index: &StoryIndexShard,
        doc_address: DocAddress,
    ) -> Result<(Story, ScrapeCollection), PersistError> {
        let story = index.lookup_story(doc_address)?;
        let url = StoryUrl::parse(story.url).expect("Failed to parse URL");
        let date = StoryDate::from_seconds(story.date).expect("Failed to re-parse date");
        let score = story.score as f32;
        let scrapes = self
            .scrape_db
            .fetch_scrape_batch(story.scrape_ids.clone())?;
        let scrapes = ScrapeCollection::new_from_iter(scrapes.into_values().flatten());
        let story =
            Story::new_from_parts(story.title, url, date, score, story.tags, story.scrape_ids);
        Ok((story, scrapes))
    }

    fn get_story_doc(
        &self,
        index: &StoryIndexShard,
        doc_address: DocAddress,
    ) -> Result<NamedFieldDocument, PersistError> {
        index.with_searcher(|searcher, _| index.doc_fields(searcher, doc_address))?
    }

    fn fetch_by_segment(
        &self,
    ) -> impl FnMut(Shard, &Searcher, &StorySchema) -> Vec<(Shard, DocAddress)> {
        move |shard, searcher, _schema| {
            let mut v = vec![];
            let now = Instant::now();
            for (idx, segment_reader) in searcher.segment_readers().iter().enumerate() {
                for doc_id in segment_reader.doc_ids_alive() {
                    let doc_address = DocAddress::new(idx as u32, doc_id);
                    v.push((shard, doc_address));
                }
            }
            tracing::info!(
                "Loaded {} stories from shard {:?} in {}ms",
                v.len(),
                shard,
                now.elapsed().as_millis()
            );
            v
        }
    }

    fn fetch_by_id(
        &self,
        id: &StoryIdentifier,
    ) -> impl FnMut(Shard, &Searcher, &StorySchema) -> Result<Vec<(Shard, DocAddress)>, PersistError>
    {
        let id = id.to_base64();
        move |shard, searcher, schema| {
            let query = TermQuery::new(
                Term::from_field_text(schema.id_field, &id),
                IndexRecordOption::Basic,
            );
            let docs = searcher.search(&query, &TopDocs::with_limit(1))?;
            for (_, doc_address) in docs {
                return Ok(vec![(shard, doc_address)]);
            }
            Ok(vec![])
        }
    }

    /// Incrementally fetch a query from multiple shards, up to max items
    fn fetch_search_query<Q: Query>(
        &self,
        query: Q,
        max: usize,
    ) -> Result<Vec<(Shard, DocAddress)>, PersistError> {
        let mut vec = vec![];
        let mut remaining = max;
        for shard in self.shards().iterate(ShardOrder::NewestFirst) {
            let docs = self.with_searcher(shard, |shard, searcher, _schema| {
                let docs = searcher.search(&query, &TopDocs::with_limit(remaining))?;
                Result::<_, PersistError>::Ok(docs.into_iter().map(move |x| (shard, x.1)))
            })??;
            vec.extend(docs);
            remaining = max - vec.len();
        }
        Ok(vec)
    }

    fn fetch_tag_search(
        &self,
        tag: &str,
        max: usize,
    ) -> Result<Vec<(Shard, DocAddress)>, PersistError> {
        let query = TermQuery::new(
            Term::from_field_text(self.schema.tags_field, tag),
            IndexRecordOption::Basic,
        );
        tracing::debug!("Tag symbol query = {:?}", query);
        self.fetch_search_query(query, max)
    }

    fn fetch_domain_search(
        &self,
        domain: &str,
        max: usize,
    ) -> Result<Vec<(Shard, DocAddress)>, PersistError> {
        let host_field = self.schema.host_field;
        let phrase = domain
            .split(|c: char| !c.is_alphanumeric())
            .map(|s| Term::from_field_text(host_field, s))
            .enumerate()
            .collect_vec();
        let query = PhraseQuery::new_with_offset(phrase);
        tracing::debug!("Domain phrase query = {:?}", query);
        self.fetch_search_query(query, max)
    }

    fn fetch_text_search(
        &self,
        search: &str,
        max: usize,
    ) -> Result<Vec<(Shard, DocAddress)>, PersistError> {
        let query = TermQuery::new(
            Term::from_field_text(self.schema.title_field, search),
            IndexRecordOption::Basic,
        );
        tracing::debug!("Term query = {:?}", query);
        self.fetch_search_query(query, max)
    }

    fn fetch_front_page(&self, max_count: usize) -> Result<Vec<(Shard, DocAddress)>, PersistError> {
        let mut story_collector: StoryCollector<(Shard, DocAddress)> =
            StoryCollector::new(max_count);
        let mut processed = 0;
        let processing_target = max_count * 2;

        // Limit how far back we go since the front page _should_ only be one or two shards unless our index is empty.
        for shard in self.shards().iterate(ShardOrder::NewestFirst).take(3) {
            // Process at least twice as many stories as requested
            if processed >= processing_target {
                break;
            }

            self.with_searcher(shard, |shard, searcher, _schema| {
                let top = TopDocs::with_limit(processing_target - processed)
                    .order_by_fast_field::<i64>(self.schema.date_field);
                let docs = searcher.search(&AllQuery {}, &top)?;
                tracing::info!("Got {} doc(s) from shard {:?}", docs.len(), shard);

                for (_, doc_address) in docs {
                    processed += 1;
                    let score = searcher
                        .segment_reader(doc_address.segment_ord)
                        .fast_fields()
                        .f64(self.schema.score_field)?
                        .get_val(doc_address.doc_id) as f32;
                    if story_collector.would_accept(score) {
                        story_collector.accept(score, (shard, doc_address));
                    }
                }

                Result::<_, PersistError>::Ok(())
            })??;
        }
        tracing::info!(
            "Got {}/{} docs for front page (processed {})",
            story_collector.len(),
            max_count,
            processed
        );
        Ok(story_collector.to_sorted())
    }

    fn fetch_doc_addresses(
        &self,
        query: StoryQuery,
        max: usize,
    ) -> Result<Vec<(Shard, DocAddress)>, PersistError> {
        match query {
            StoryQuery::ById(id) => self.with_searcher(
                Shard::from_year_month(id.year(), id.month()),
                self.fetch_by_id(&id),
            )?,
            StoryQuery::ByShard(shard) => Ok(self.with_searcher(shard, self.fetch_by_segment())?),
            StoryQuery::FrontPage() => self.fetch_front_page(max),
            StoryQuery::TagSearch(tag) => self.fetch_tag_search(&tag, max),
            StoryQuery::DomainSearch(domain) => self.fetch_domain_search(&domain, max),
            StoryQuery::TextSearch(text) => self.fetch_text_search(&text, max),
        }
    }
}

impl StorageWriter for StoryIndex {
    fn insert_scrapes<I: Iterator<Item = TypedScrape>>(
        &mut self,
        eval: &StoryEvaluator,
        scrapes: I,
    ) -> Result<(), PersistError> {
        self.insert_scrapes(eval, scrapes)
    }

    fn insert_scrape_collections<I: Iterator<Item = ScrapeCollection>>(
        &mut self,
        eval: &StoryEvaluator,
        scrape_collections: I,
    ) -> Result<(), PersistError> {
        self.insert_scrape_collections(eval, scrape_collections)
    }
}

impl Storage for StoryIndex {
    fn most_recent_story(&self) -> Result<StoryDate, PersistError> {
        if let Some(max) = self.shards().iterate(ShardOrder::NewestFirst).next() {
            let shard = self.get_shard(max)?;
            let index = shard.read().expect("Poisoned");
            Ok(index.most_recent_story()?)
        } else {
            Ok(StoryDate::MIN)
        }
    }

    fn story_count(&self) -> Result<StorageSummary, PersistError> {
        let mut summary = StorageSummary::default();
        for shard in self.shards().iterate(ShardOrder::OldestFirst) {
            let index = self.get_shard(shard)?;
            let subtotal = index.read().expect("Poisoned").total_docs()?;
            summary.by_shard.push((shard.to_string(), subtotal));
            summary.total += subtotal;
        }
        Ok(summary)
    }

    fn fetch(&self, query: StoryQuery, max: usize) -> Result<Vec<Story>, PersistError> {
        let mut v = vec![];
        for (shard, doc) in self.fetch_doc_addresses(query, max)? {
            let shard = self.get_shard(shard)?;
            let shard = shard.read().expect("Poisoned");
            v.push(self.lookup_story(&shard, doc)?);
        }
        Ok(v)
    }

    fn fetch_with_scrapes(
        &self,
        query: StoryQuery,
        max: usize,
    ) -> Result<Vec<(Story, ScrapeCollection)>, PersistError> {
        let mut v = vec![];
        for (shard, doc) in self.fetch_doc_addresses(query, max)? {
            let shard = self.get_shard(shard)?;
            let shard = shard.read().expect("Poisoned");
            v.push(self.lookup_story_and_scrapes(&shard, doc)?);
        }
        Ok(v)
    }

    fn get_story(
        &self,
        id: &StoryIdentifier,
    ) -> Result<Option<(Story, ScrapeCollection)>, PersistError> {
        Ok(self
            .fetch_with_scrapes(StoryQuery::ById(id.clone()), 1)?
            .into_iter()
            .next())
    }

    fn stories_by_shard(&self, shard: &str) -> Result<Vec<Story>, PersistError> {
        if let Some(shard) = Shard::from_string(shard) {
            self.fetch(StoryQuery::ByShard(shard), usize::MAX)
        } else {
            Ok(vec![])
        }
    }

    fn query_frontpage_hot_set(&self, max_count: usize) -> Result<Vec<Story>, PersistError> {
        self.fetch(StoryQuery::FrontPage(), max_count)
    }

    fn query_frontpage_hot_set_detail(
        &self,
        _max_count: usize,
    ) -> Result<Vec<(Story, ScrapeCollection)>, PersistError> {
        unimplemented!()
    }

    fn query_search(
        &self,
        tagger: &StoryTagger,
        search: &str,
        max_count: usize,
    ) -> Result<Vec<Story>, PersistError> {
        // This isn't terribly smart, buuuuut it allows us to search either a tag or site
        if let Some(tag) = tagger.check_tag_search(search) {
            self.fetch(StoryQuery::TagSearch(tag.to_string()), max_count)
        } else if search.contains('.') {
            self.fetch(StoryQuery::DomainSearch(search.to_string()), max_count)
        } else {
            self.fetch(StoryQuery::TextSearch(search.to_string()), max_count)
        }
    }
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use super::*;
    use progscrape_scrapers::{hacker_news::*, reddit::*, ScrapeSource, StoryUrl};

    use crate::{story::TagSet, test::*, MemIndex};
    use rstest::*;

    fn populate_shard(
        ids: impl Iterator<Item = (i64, i64)>,
    ) -> Result<StoryIndexShard, PersistError> {
        let mut shard = StoryIndexShard::initialize(
            PersistLocation::Memory,
            Shard::default(),
            StorySchema::instantiate_global_schema(),
        )?;
        let mut writer = shard.index.writer(MEMORY_ARENA_SIZE)?;
        for (url_norm_hash, date) in ids {
            shard.insert_story_document(
                &mut writer,
                StoryInsert {
                    url_norm_hash,
                    date,
                    ..Default::default()
                },
            )?;
        }
        writer.commit()?;
        Ok(shard)
    }

    fn hn_story(id: &str, date: StoryDate, title: &str, url: &StoryUrl) -> TypedScrape {
        HackerNewsStory::new_with_defaults(id, date, title, url.clone()).into()
    }

    fn reddit_story(
        id: &str,
        subreddit: &str,
        date: StoryDate,
        title: &str,
        url: &StoryUrl,
    ) -> TypedScrape {
        RedditStory::new_subsource_with_defaults(id, subreddit, date, title, url.clone()).into()
    }

    #[rstest]
    fn test_index_shard(_enable_tracing: &bool) {
        let ids1 = (0..100).into_iter().map(|x| (x, 0));
        let ids2 = (100..200).into_iter().map(|x| (x, 10));
        let shard = populate_shard(ids1.chain(ids2)).expect("Failed to initialize shard");
        let reader = shard.index.reader().expect("Failed to get reader");
        let searcher = reader.searcher();
        let count_found = |vec: Vec<StoryLookup>| {
            vec.iter()
                .filter(|x| matches!(x, StoryLookup::Found(..)))
                .count()
        };
        macro_rules! test_range {
            ($date:expr, $slop:expr, $expected:expr) => {
                let lookup = (95..110)
                    .into_iter()
                    .map(|n| StoryLookupId {
                        url_norm_hash: n,
                        date: $date,
                    })
                    .collect();
                let result = shard
                    .lookup_stories(&searcher, lookup, $slop)
                    .expect("Failed to look up");
                assert_eq!($expected, count_found(result));
            };
        }
        // No slop on date, date = 0, we only get 95..100
        test_range!(0, 0..=0, 5);
        // No slop on date, date = 10, we only get 100-110
        test_range!(10, 0..=0, 10);
        // 0..+10 slop on date, date = 0, we get everything
        test_range!(0, 0..=10, 15);
    }

    #[rstest]
    fn test_index_scrapes(_enable_tracing: &bool) -> Result<(), Box<dyn std::error::Error>> {
        use ScrapeSource::*;

        let mut index = StoryIndex::new(PersistLocation::Memory)?;
        let eval = StoryEvaluator::new_for_test();
        let url = StoryUrl::parse("http://example.com").expect("URL");
        let date = StoryDate::year_month_day(2020, 1, 1).expect("Date failed");
        index.insert_scrapes(
            &eval,
            [hn_story("story1", date, "I love Rust", &url)].into_iter(),
        )?;

        let counts = index.story_count()?;
        assert_eq!(counts.total, 1);

        index.insert_scrapes(
            &eval,
            [reddit_story("story1", "rust", date, "I love rust", &url)].into_iter(),
        )?;

        let counts = index.story_count()?;
        assert_eq!(counts.total, 1);

        let search = index.query_search(&eval.tagger, "rust", 10)?;
        assert_eq!(search.len(), 1);

        let story = &search[0];
        assert_eq!("I love Rust", story.title);
        assert!(itertools::equal(
            [
                &HackerNews.id("story1"),
                &Reddit.subsource_id("rust", "story1")
            ],
            story.scrapes.keys().sorted()
        ),);
        assert_eq!(TagSet::from_iter(["rust"]), story.tags);

        Ok(())
    }

    #[rstest]
    fn test_index_scrape_collections(
        _enable_tracing: &bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use ScrapeSource::*;

        let mut memindex = MemIndex::default();
        let eval = StoryEvaluator::new_for_test();
        let url = StoryUrl::parse("http://example.com").expect("URL");
        let date = StoryDate::year_month_day(2020, 1, 1).expect("Date failed");
        memindex.insert_scrapes([hn_story("story1", date, "I love Rust", &url)].into_iter())?;
        memindex.insert_scrapes(
            [reddit_story("story1", "rust", date, "I love Rust", &url)].into_iter(),
        )?;

        let mut index = StoryIndex::new(PersistLocation::Memory)?;
        index.insert_scrape_collections(&eval, memindex.get_all_stories())?;

        let counts = index.story_count()?;
        assert_eq!(counts.total, 1);

        let search = index.query_search(&eval.tagger, "rust", 10)?;
        assert_eq!(search.len(), 1);

        let story = &search[0];
        assert_eq!("I love Rust", story.title);
        assert!(itertools::equal(
            [
                &HackerNews.id("story1"),
                &Reddit.subsource_id("rust", "story1")
            ],
            story.scrapes.keys().sorted()
        ),);
        assert_eq!(TagSet::from_iter(["rust"]), story.tags);

        Ok(())
    }

    #[rstest]
    fn test_insert_batch(_enable_tracing: &bool) -> Result<(), Box<dyn std::error::Error>> {
        let mut batch = vec![];
        let date = StoryDate::year_month_day(2020, 1, 1).expect("Date failed");

        for i in 0..30 {
            let url = StoryUrl::parse(&format!("http://domain-{}.com/", i)).expect("URL");
            batch.push(hn_story(
                &format!("story-{}", i),
                date,
                &format!("Title {}", i),
                &url,
            ));
        }

        let mut index = StoryIndex::new(PersistLocation::Memory)?;
        let eval = StoryEvaluator::new_for_test();

        index.insert_scrapes(&eval, batch.clone().into_iter())?;

        // Cause a delete
        let url = StoryUrl::parse("http://domain-3.com/").expect("URL");

        index.insert_scrapes(
            &eval,
            [reddit_story("story-3", "subreddit", date, "Title 3", &url)].into_iter(),
        )?;

        index.insert_scrapes(&eval, batch.clone().into_iter())?;

        let front_page = index.query_frontpage_hot_set(100)?;
        assert_eq!(30, front_page.len());

        Ok(())
    }

    /// Ensure that a story is searchable by various terms.
    #[rstest]
    #[case("http://example.com", "I love Rust", &["rust", "love", "example.com"])]
    #[case("http://medium.com", "The pitfalls of C++", &["c++", "cplusplus"])]
    #[case("http://www.att.com", "New AT&T plans", &["at&t", "atandt", "att.com"])]
    #[case("http://example.com", "I love Go", &["golang", "love"])]
    #[case("http://example.com", "I love C", &["clanguage", "love"])]
    // TODO: This case doesn't work yet
    // #[case("http://youtube.com/?v=123", "A tutorial", &["video", "youtube", "tutorial"])]
    fn test_findable(
        #[case] url: &str,
        #[case] title: &str,
        #[case] search_terms: &[&str],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut index = StoryIndex::new(PersistLocation::Memory)?;
        let eval = StoryEvaluator::new_for_test();
        let url = StoryUrl::parse(url).expect("URL");
        let date = StoryDate::year_month_day(2020, 1, 1).expect("Date failed");
        index.insert_scrapes(&eval, [hn_story("story1", date, title, &url)].into_iter())?;

        let counts = index.story_count()?;
        assert_eq!(counts.total, 1);

        for term in search_terms {
            let search = index.query_search(&eval.tagger, term, 10)?;
            assert_eq!(
                1,
                search.len(),
                "Expected one search result when querying '{}' for title={} url={}",
                term,
                title,
                url
            );
        }

        Ok(())
    }

    #[rstest]
    fn test_index_lots(
        _enable_tracing: &bool,
        enable_slow_tests: &bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if !enable_slow_tests {
            tracing::error!("Ignoring test because enable_slow_tests is not set");
            return Ok(());
        }
        let path = "/tmp/indextest";
        std::fs::create_dir_all(path)?;
        let mut index = StoryIndex::new(PersistLocation::Path(path.into()))?;

        let scrapes = progscrape_scrapers::import_legacy(Path::new(".."))?;
        let eval = StoryEvaluator::new_for_test();
        let mut memindex = MemIndex::default();

        // First, build an in-memory index quickly
        memindex.insert_scrapes(scrapes.into_iter())?;

        index.insert_scrape_collections(&eval, memindex.get_all_stories())?;

        // Query the new index
        for story in index.query_search(&eval.tagger, "rust", 10)? {
            println!("{:?}", story);
        }

        Ok(())
    }
}
