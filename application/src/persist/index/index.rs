use itertools::Itertools;

use tantivy::collector::TopDocs;
use tantivy::query::{AllQuery, PhraseQuery, Query, QueryParser, TermQuery};
use tantivy::tokenizer::TokenizerManager;
use tantivy::{schema::*, DocAddress, IndexWriter, Searcher};

use progscrape_scrapers::{ScrapeCollection, StoryDate, StoryUrl, TypedScrape};

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::persist::index::indexshard::{StoryIndexShard, StoryLookup, StoryLookupId};
use crate::persist::scrapestore::ScrapeStore;
use crate::persist::shard::{ShardOrder, ShardRange};
use crate::persist::{ScrapePersistResult, Shard, ShardSummary, StorageFetch, StoryQuery};
use crate::story::{StoryCollector, TagSet};
use crate::{
    timer_end, timer_start, MemIndex, PersistError, PersistLocation, Storage, StorageSummary,
    StorageWriter, Story, StoryEvaluator, StoryIdentifier,
};

use super::indexshard::StoryInsert;
use super::schema::StorySchema;

const STORY_INDEXING_CHUNK_SIZE: usize = 10000;
const SCRAPE_PROCESSING_CHUNK_SIZE: usize = 1000;

struct IndexCache {
    cache: HashMap<Shard, Arc<RwLock<StoryIndexShard>>>,
    location: PersistLocation,
    range: ShardRange,
    schema: StorySchema,
    most_recent_story: Option<StoryDate>,
}

impl IndexCache {
    fn get_shard(&mut self, shard: Shard) -> Result<Arc<RwLock<StoryIndexShard>>, PersistError> {
        if let Some(shard) = self.cache.get(&shard) {
            Ok(shard.clone())
        } else {
            tracing::info!("Creating shard {}", shard.to_string());
            let new_shard =
                StoryIndexShard::initialize(self.location.clone(), shard, self.schema.clone())?;
            self.range.include(shard);
            Ok(self
                .cache
                .entry(shard)
                .or_insert(Arc::new(RwLock::new(new_shard)))
                .clone())
        }
    }
}

pub struct StoryIndex {
    index_cache: Arc<RwLock<IndexCache>>,
    scrape_db: ScrapeStore,
    schema: StorySchema,
}

struct WriterProvider {
    writers: HashMap<Shard, IndexWriter>,
    index: Arc<RwLock<IndexCache>>,
}

impl WriterProvider {
    fn provide<T>(
        &mut self,
        shard: Shard,
        f: impl FnOnce(Shard, &StoryIndexShard, &'_ mut IndexWriter) -> Result<T, PersistError>,
    ) -> Result<T, PersistError> {
        let shard_index = self.index.write().expect("Poisoned").get_shard(shard)?;
        let shard_index = shard_index.write().expect("Poisoned");
        let writer = if let Some(writer) = self.writers.get_mut(&shard) {
            writer
        } else {
            let writer = shard_index.writer()?;
            self.writers.entry(shard).or_insert(writer)
        };

        f(shard, &shard_index, writer)
    }
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
        let schema = StorySchema::instantiate_global_schema();
        let new = Self {
            index_cache: Arc::new(RwLock::new(IndexCache {
                cache: HashMap::new(),
                location,
                range,
                schema: schema.clone(),
                most_recent_story: None,
            })),
            scrape_db,
            schema,
        };

        Ok(new)
    }

    pub fn shards(&self) -> ShardRange {
        self.index_cache.read().expect("Poisoned").range
    }

    fn get_shard(&self, shard: Shard) -> Result<Arc<RwLock<StoryIndexShard>>, PersistError> {
        let mut lock = self.index_cache.write().expect("Poisoned");
        lock.get_shard(shard)
    }

    /// Borrow the scrape database for a period of time.
    pub fn with_scrapes<F: FnOnce(&ScrapeStore) -> T, T>(&self, f: F) -> T {
        f(&self.scrape_db)
    }

    #[inline(always)]
    fn with_searcher<F: FnMut(Shard, &Searcher, &StorySchema) -> T, T>(
        &self,
        shard: Shard,
        mut f: F,
    ) -> Result<T, PersistError> {
        let shard_index = self.get_shard(shard)?;
        let shard_index = shard_index.read().expect("Poisoned");
        shard_index.with_searcher(|searcher, schema| f(shard, searcher, schema))
    }

    #[inline(always)]
    fn with_index<F: FnMut(Shard, &StoryIndexShard) -> T, T>(
        &self,
        shard: Shard,
        mut f: F,
    ) -> Result<T, PersistError> {
        let shard_index = self.get_shard(shard)?;
        let shard_index = shard_index.read().expect("Poisoned");
        Ok(f(shard, &shard_index))
    }

    /// This is a complicated function that gives you access to a function that gives you access
    /// to writers. The function manages the writers until the completion of the outer closure.
    fn with_writers<
        TOuter,
        WriterOuterClosure: FnOnce(&mut WriterProvider) -> Result<TOuter, PersistError>,
    >(
        &self,
        f: WriterOuterClosure,
    ) -> Result<TOuter, PersistError> {
        let mut provider = WriterProvider {
            writers: Default::default(),
            index: self.index_cache.clone(),
        };
        let res = f(&mut provider);
        let WriterProvider { writers, .. } = provider;

        let writer_count = writers.len();
        if res.is_ok() {
            tracing::info!("Commiting {} writer(s)", writer_count);
            let commit_start = timer_start!();
            for (shard, writer) in writers.into_iter() {
                let shard = self.get_shard(shard)?;
                let mut shard = shard.write().expect("Poisoned");
                shard.commit_writer(writer)?;
            }
            timer_end!(commit_start, "Committed {} writer(s).", writer_count);
            self.index_cache
                .write()
                .expect("Poisoned")
                .most_recent_story = None;
        } else {
            // We'll just have to do our best here...
            for mut writer in writers.into_values() {
                if let Err(e) = writer.rollback() {
                    tracing::error!("Ignoring nested error in writer rollback: {:?}", e);
                }
            }
        }
        res
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
            .map(|x| x.1)
            .map(Self::create_scrape_id_from_scrape)
            .collect_vec();
        let title = extracted.title.to_owned();
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

    fn insert_scrape_batch<'a, I: IntoIterator<Item = TypedScrape> + 'a>(
        &mut self,
        eval: &StoryEvaluator,
        scrapes: I,
    ) -> Result<Vec<ScrapePersistResult>, PersistError> {
        let one_month = Duration::from_secs(60 * 60 * 24 * 30).as_secs() as i64;

        let mut memindex = MemIndex::default();
        memindex.insert_scrapes(scrapes)?;

        self.with_writers(|provider| {
            let mut res = vec![];
            for story in memindex.get_all_stories() {
                let shard = Shard::from_date_time(story.earliest);
                // TODO: Should be searching multiple shards
                provider.provide(shard, |_, index, writer| {
                    let lookup = StoryLookupId {
                        url_norm_hash: story.url().normalization().hash(),
                        date: story.earliest.timestamp(),
                    };
                    let lookup = HashSet::from_iter([lookup]);
                    // TODO: Should be batching
                    let result = index.lookup_stories(lookup, (-one_month)..one_month)?;
                    let lookup = result.into_iter().next().expect("TODO");
                    let _insert_type = match lookup {
                        StoryLookup::Found(_id, doc) => {
                            let doc = index.with_searcher(|searcher, _| searcher.doc(doc))??;
                            let ids = index.extract_scrape_ids_from_doc(&doc);
                            let scrapes = self.scrape_db.fetch_scrape_batch(ids)?;
                            let mut orig_story =
                                ScrapeCollection::new_from_iter(scrapes.into_values().flatten());
                            orig_story.merge_all(story);
                            let doc = Self::create_story_insert(eval, &orig_story);
                            res.push(ScrapePersistResult::MergedWithExistingStory);
                            index.reinsert_story_document(writer, doc)?
                        }
                        StoryLookup::Unfound(_id) => {
                            let doc = Self::create_story_insert(eval, &story);
                            res.push(ScrapePersistResult::NewStory);
                            index.insert_story_document(writer, doc)?
                        }
                    };

                    Ok(())
                })?;
            }
            Ok(res)
        })
    }

    /// Insert a list of scrapes into the index.
    fn insert_scrapes<I: IntoIterator<Item = TypedScrape>>(
        &mut self,
        eval: &StoryEvaluator,
        scrapes: I,
    ) -> Result<Vec<ScrapePersistResult>, PersistError> {
        let v = scrapes.into_iter().collect_vec();

        tracing::info!("Storing raw scrapes...");
        self.scrape_db.insert_scrape_batch(v.iter())?;

        tracing::info!("Indexing scrapes...");
        Ok(self.insert_scrape_batch(eval, v)?)
    }

    fn insert_scrape_collections<I: IntoIterator<Item = ScrapeCollection>>(
        &mut self,
        eval: &StoryEvaluator,
        scrape_collections: I,
    ) -> Result<Vec<ScrapePersistResult>, PersistError> {
        self.with_writers(|provider| {
            let mut res = vec![];
            let start = timer_start!();
            let mut total = 0;
            for scrape_collections in &scrape_collections
                .into_iter()
                .chunks(STORY_INDEXING_CHUNK_SIZE)
            {
                tracing::info!("Indexing chunk...");
                let start_chunk = timer_start!();
                let mut count = 0;
                let mut scrapes_batch = vec![];

                for story in scrape_collections {
                    count += 1;
                    res.push(ScrapePersistResult::NewStory);
                    let doc = Self::create_story_insert(eval, &story);
                    let scrapes = story.scrapes.into_values();
                    scrapes_batch.extend(scrapes);
                    provider.provide(
                        Shard::from_date_time(story.earliest),
                        move |_, index, writer| {
                            index.insert_story_document(writer, doc)?;
                            Ok(())
                        },
                    )?;

                    if scrapes_batch.len() > SCRAPE_PROCESSING_CHUNK_SIZE {
                        self.scrape_db.insert_scrape_batch(scrapes_batch.iter())?;
                        scrapes_batch.clear();
                    }
                }
                self.scrape_db.insert_scrape_batch(scrapes_batch.iter())?;
                scrapes_batch.clear();
                total += count;
                timer_end!(start_chunk, "Indexed chunk of {} stories", count);
            }
            timer_end!(start, "Indexed total of {} stories", total);
            Ok(res)
        })
    }

    fn reinsert_stories<I: IntoIterator<Item = StoryIdentifier>>(
        &mut self,
        eval: &StoryEvaluator,
        stories: I,
    ) -> Result<Vec<ScrapePersistResult>, PersistError> {
        self.with_writers(|provider| {
            let mut res = vec![];
            for id in stories {
                let searcher = self.fetch_by_id(&id);
                let docs = self.with_searcher(id.shard(), searcher)??;
                if let Some((shard, doc)) = docs.first() {
                    provider.provide(*shard, |_, index, writer| {
                        let doc = index.with_searcher(|searcher, _| searcher.doc(*doc))??;
                        let ids = index.extract_scrape_ids_from_doc(&doc);
                        let scrapes = self.scrape_db.fetch_scrape_batch(ids)?;
                        let orig_story =
                            ScrapeCollection::new_from_iter(scrapes.into_values().flatten());
                        let doc = Self::create_story_insert(eval, &orig_story);
                        index.reinsert_story_document(writer, doc)?;
                        Ok(())
                    })?;
                    res.push(ScrapePersistResult::MergedWithExistingStory);
                } else {
                    res.push(ScrapePersistResult::NotFound)
                }
            }
            Ok(res)
        })
    }

    fn get_story_doc(
        &self,
        id: &StoryIdentifier,
    ) -> Result<Option<NamedFieldDocument>, PersistError> {
        let shard = id.shard();
        let id = self
            .with_searcher(shard, self.fetch_by_id(id))??
            .first()
            .map(Clone::clone);
        if let Some((_, doc_address)) = id {
            let res = self.with_index(shard, |_, index| index.doc_fields(doc_address))??;
            Ok(Some(res))
        } else {
            Ok(None)
        }
    }

    fn fetch_by_segment(
        &self,
    ) -> impl FnMut(Shard, &Searcher, &StorySchema) -> Vec<(Shard, DocAddress)> {
        move |shard, searcher, _schema| {
            let mut v = vec![];
            let now = timer_start!();
            for (idx, segment_reader) in searcher.segment_readers().iter().enumerate() {
                for doc_id in segment_reader.doc_ids_alive() {
                    let doc_address = DocAddress::new(idx as u32, doc_id);
                    v.push((shard, doc_address));
                }
            }
            timer_end!(now, "Loaded {} stories from shard {:?}", v.len(), shard);
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
            if remaining == 0 {
                break;
            }
            let docs = self.with_searcher(shard, |shard, searcher, _schema| {
                let docs = searcher.search(&query, &TopDocs::with_limit(remaining))?;
                Result::<_, PersistError>::Ok(docs.into_iter().map(move |x| (shard, x.1)))
            })??;
            vec.extend(docs);
            remaining = max.saturating_sub(vec.len());
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
        // TODO: We probably don't want to re-parse this as a URL, but it's the fastest way to normalize it
        if let Some(url) = StoryUrl::parse(format!("http://{}", domain)) {
            let phrase = url
                .host()
                .split(|c: char| !c.is_alphanumeric())
                .map(|s| Term::from_field_text(host_field, s))
                .collect_vec();
            let query = PhraseQuery::new(phrase);
            tracing::debug!("Domain phrase query = {:?}", query);
            self.fetch_search_query(query, max)
        } else {
            Err(PersistError::UnexpectedError("Invalid domain".into()))
        }
    }

    fn fetch_text_search(
        &self,
        search: &str,
        max: usize,
    ) -> Result<Vec<(Shard, DocAddress)>, PersistError> {
        let query_parser = QueryParser::new(
            self.schema.schema.clone(),
            vec![self.schema.title_field, self.schema.tags_field],
            TokenizerManager::default(),
        );
        let query = query_parser.parse_query(search)?;
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
            StoryQuery::ById(id) => self.with_searcher(id.shard(), self.fetch_by_id(&id))?,
            StoryQuery::ByShard(shard) => Ok(self.with_searcher(shard, self.fetch_by_segment())?),
            StoryQuery::FrontPage() => self.fetch_front_page(max),
            StoryQuery::TagSearch(tag) => self.fetch_tag_search(&tag, max),
            StoryQuery::DomainSearch(domain) => self.fetch_domain_search(&domain, max),
            StoryQuery::TextSearch(text) => self.fetch_text_search(&text, max),
        }
    }
}

impl StorageWriter for StoryIndex {
    /// Inserts individual scrapes, assuming that there is no story overlap in the input scrapes.
    /// If there are matching scrapes in stories in the index, those stories are updated with
    /// the new scrapes.
    fn insert_scrapes<I: IntoIterator<Item = TypedScrape>>(
        &mut self,
        eval: &StoryEvaluator,
        scrapes: I,
    ) -> Result<Vec<ScrapePersistResult>, PersistError> {
        self.insert_scrapes(eval, scrapes)
    }

    /// Inserts a set of pre-existing scrape collections, assuming that these stories do
    /// not already exist in the index. This is the fastest way to populate an index.
    fn insert_scrape_collections<I: IntoIterator<Item = ScrapeCollection>>(
        &mut self,
        eval: &StoryEvaluator,
        scrape_collections: I,
    ) -> Result<Vec<ScrapePersistResult>, PersistError> {
        self.insert_scrape_collections(eval, scrape_collections)
    }

    /// Re-insert a set of stories, assuming that they are in the index. This must only be
    /// used with a story sourced from this index. Note that the
    fn reinsert_stories<I: IntoIterator<Item = StoryIdentifier>>(
        &mut self,
        eval: &StoryEvaluator,
        stories: I,
    ) -> Result<Vec<ScrapePersistResult>, PersistError> {
        self.reinsert_stories(eval, stories)
    }
}

impl StorageFetch<Shard> for StoryIndex {
    fn fetch_type(&self, query: StoryQuery, max: usize) -> Result<Vec<Story<Shard>>, PersistError> {
        let mut v = vec![];
        for (shard, doc) in self.fetch_doc_addresses(query, max)? {
            let doc = self.with_index(shard, |_, index| {
                let story = index.lookup_story(doc)?;
                let url = StoryUrl::parse(story.url).expect("Failed to parse URL");
                let date = StoryDate::from_seconds(story.date).expect("Failed to re-parse date");
                let score = story.score as f32;
                Result::<_, PersistError>::Ok(Story::new_from_parts(
                    story.title,
                    url,
                    date,
                    score,
                    story.tags,
                    story.scrape_ids,
                ))
            })??;

            v.push(doc);
        }
        Ok(v)
    }
}

impl StorageFetch<TypedScrape> for StoryIndex {
    fn fetch_type(
        &self,
        query: StoryQuery,
        max: usize,
    ) -> Result<Vec<Story<TypedScrape>>, PersistError> {
        let mut v = vec![];
        for (shard, doc) in self.fetch_doc_addresses(query, max)? {
            let doc = self.with_index(shard, |_, index| {
                let story = index.lookup_story(doc)?;
                let url = StoryUrl::parse(story.url).expect("Failed to parse URL");
                let date = StoryDate::from_seconds(story.date).expect("Failed to re-parse date");
                let score = story.score as f32;

                let scrapes = self
                    .scrape_db
                    .fetch_scrape_batch(story.scrape_ids.clone())?;
                let story = Story::new_from_parts(
                    story.title,
                    url,
                    date,
                    score,
                    story.tags,
                    scrapes.into_values().flatten(),
                );

                Result::<_, PersistError>::Ok(story)
            })??;

            v.push(doc);
        }
        Ok(v)
    }
}

impl Storage for StoryIndex {
    fn most_recent_story(&self) -> Result<StoryDate, PersistError> {
        if let Some(most_recent_story) =
            self.index_cache.read().expect("Poisoned").most_recent_story
        {
            return Ok(most_recent_story);
        }

        if let Some(max) = self.shards().iterate(ShardOrder::NewestFirst).next() {
            let shard = self.get_shard(max)?;
            let index = shard.read().expect("Poisoned");
            let result = index.most_recent_story()?;
            self.index_cache
                .write()
                .expect("Poisoned")
                .most_recent_story = Some(result);
            Ok(result)
        } else {
            Ok(StoryDate::MIN)
        }
    }

    fn shard_range(&self) -> Result<ShardRange, PersistError> {
        Ok(self.shards())
    }

    fn story_count(&self) -> Result<StorageSummary, PersistError> {
        let mut summary = StorageSummary::default();
        for shard in self.shards().iterate(ShardOrder::OldestFirst) {
            let index = self.get_shard(shard)?;
            let subtotal = index.read().expect("Poisoned").total_docs()?;
            let scrape_subtotal = self.scrape_db.stats(shard)?.count;
            summary.by_shard.push((
                shard.to_string(),
                ShardSummary {
                    story_count: subtotal,
                    scrape_count: scrape_subtotal,
                },
            ));
            summary.total.story_count += subtotal;
            summary.total.scrape_count += scrape_subtotal;
        }
        Ok(summary)
    }

    fn fetch_count(&self, query: StoryQuery, max: usize) -> Result<usize, PersistError> {
        Ok(self.fetch_doc_addresses(query, max)?.len())
    }
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use super::*;
    use progscrape_scrapers::{
        hacker_news::*, lobsters::LobstersStory, reddit::*, ScrapeSource, StoryUrl,
    };

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
        shard.with_writer(move |shard, writer, _| {
            for (url_norm_hash, date) in ids {
                shard.insert_story_document(
                    writer,
                    StoryInsert {
                        url_norm_hash,
                        date,
                        ..Default::default()
                    },
                )?;
            }
            Ok(())
        })?;
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

    fn lobsters_story(
        id: &str,
        date: StoryDate,
        title: &str,
        url: &StoryUrl,
        tags: Vec<String>,
    ) -> TypedScrape {
        let mut lobsters = LobstersStory::new_with_defaults(id, date, title, url.clone());
        lobsters.data.tags = tags;
        lobsters.into()
    }

    fn rust_story_hn() -> TypedScrape {
        let url = StoryUrl::parse("http://example.com").expect("URL");
        let date = StoryDate::year_month_day(2020, 1, 1).expect("Date failed");
        hn_story("story1", date, "I love Rust", &url)
    }

    fn rust_story_reddit() -> TypedScrape {
        let url = StoryUrl::parse("http://example.com").expect("URL");
        let date = StoryDate::year_month_day(2020, 1, 1).expect("Date failed");
        reddit_story("story1", "rust", date, "I love rust", &url)
    }

    fn rust_story_lobsters() -> TypedScrape {
        let url = StoryUrl::parse("http://example.com").expect("URL");
        let date = StoryDate::year_month_day(2020, 1, 1).expect("Date failed");
        lobsters_story(
            "story1",
            date,
            "Type inference in Rust",
            &url,
            vec!["plt".to_string()],
        )
    }

    #[rstest]
    fn test_index_shard(_enable_tracing: &bool) {
        let ids1 = (0..100).into_iter().map(|x| (x, 0));
        let ids2 = (100..200).into_iter().map(|x| (x, 10));
        let shard = populate_shard(ids1.chain(ids2)).expect("Failed to initialize shard");
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
                    .lookup_stories(lookup, $slop)
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
        index.insert_scrapes(&eval, [rust_story_hn()])?;

        let counts = index.story_count()?;
        assert_eq!(counts.total.story_count, 1);

        index.insert_scrapes(&eval, [rust_story_reddit()])?;

        let counts = index.story_count()?;
        assert_eq!(counts.total.story_count, 1);

        let search = index.fetch::<Shard>(StoryQuery::from_search(&eval.tagger, "rust"), 10)?;
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
        memindex.insert_scrapes([rust_story_hn(), rust_story_reddit()])?;

        let mut index = StoryIndex::new(PersistLocation::Memory)?;
        index.insert_scrape_collections(&eval, memindex.get_all_stories())?;

        let counts = index.story_count()?;
        assert_eq!(counts.total.story_count, 1);

        let search = index.fetch::<Shard>(StoryQuery::from_search(&eval.tagger, "rust"), 10)?;
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

    /// Does re-indexing a story work correctly?
    #[test]
    fn test_reindex_story() -> Result<(), Box<dyn std::error::Error>> {
        // Load a story
        let mut memindex = MemIndex::default();
        let eval = StoryEvaluator::new_for_test();
        memindex.insert_scrapes([rust_story_hn(), rust_story_reddit()])?;
        let mut index = StoryIndex::new(PersistLocation::Memory)?;
        index.insert_scrape_collections(&eval, memindex.get_all_stories())?;

        // Ask the index for this story
        let story = index
            .fetch_one::<Shard>(StoryQuery::from_search(&eval.tagger, "rust"))?
            .expect("Missing story");

        // Re-insert it and make sure it comes back with the right info
        assert_eq!(
            index.reinsert_stories(&eval, [story.id])?,
            vec![ScrapePersistResult::MergedWithExistingStory]
        );
        let story = index
            .fetch_one::<Shard>(StoryQuery::from_search(&eval.tagger, "rust"))?
            .expect("Missing story");
        assert_eq!(story.title, "I love Rust");

        let counts = index.story_count()?;
        assert_eq!(counts.total.story_count, 1);

        Ok(())
    }

    #[rstest]
    fn test_insert_batch(_enable_tracing: &bool) -> Result<(), Box<dyn std::error::Error>> {
        let mut batch = vec![];
        let date = StoryDate::year_month_day(2020, 1, 1).expect("Date failed");

        for i in 0..30 {
            let url = StoryUrl::parse(format!("http://domain-{}.com/", i)).expect("URL");
            batch.push(hn_story(
                &format!("story-{}", i),
                date,
                &format!("Title {}", i),
                &url,
            ));
        }

        let mut index = StoryIndex::new(PersistLocation::Memory)?;
        let eval = StoryEvaluator::new_for_test();

        index.insert_scrapes(&eval, batch.clone())?;

        // Cause a delete
        let url = StoryUrl::parse("http://domain-3.com/").expect("URL");

        index.insert_scrapes(
            &eval,
            [reddit_story("story-3", "subreddit", date, "Title 3", &url)],
        )?;

        index.insert_scrapes(&eval, batch.clone())?;

        let front_page = index.fetch_count(StoryQuery::FrontPage(), 100)?;
        assert_eq!(30, front_page);

        Ok(())
    }

    #[test]
    fn test_findable_by_extracted_tag() -> Result<(), Box<dyn std::error::Error>> {
        let mut index = StoryIndex::new(PersistLocation::Memory)?;
        let eval = StoryEvaluator::new_for_test();
        let story = rust_story_lobsters();
        index.insert_scrapes(&eval, [story.clone()])?;

        let counts = index.story_count()?;
        assert_eq!(counts.total.story_count, 1);

        for term in ["plt", "type", "inference"] {
            let search = index.fetch_count(StoryQuery::from_search(&eval.tagger, term), 10)?;
            let doc = index
                .get_story_doc(&StoryIdentifier::new(story.date, story.url.normalization()))?;
            assert_eq!(
                1, search,
                "Expected one search result when querying '{}' for title={} url={} doc={:?}",
                term, story.raw_title, story.url, doc
            );
        }
        Ok(())
    }

    /// Ensure that a story is searchable by various terms.
    #[rstest]
    #[case("http://example.com", "I love Rust", &["rust", "love", "example.com"])]
    #[case("http://medium.com", "The Pitfalls of C++", &["c++", "cplusplus", "pitfalls", "Pitfalls"])]
    #[case("http://www.att.com", "New AT&T plans", &["at&t", "atandt", "att.com"])]
    #[case("http://example.com", "I love Go", &["golang", "love"])]
    #[case("http://example.com", "I love C", &["clanguage", "love"])]
    #[case("http://www3.xyz.imperial.co.uk", "Why England is England", &["england", "www3.xyz.imperial.co.uk", "xyz.imperial.co.uk",  "co.uk"])]
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
        index.insert_scrapes(&eval, [hn_story("story1", date, title, &url)])?;

        let counts = index.story_count()?;
        assert_eq!(counts.total.story_count, 1);

        for term in search_terms {
            let search = index.fetch_count(StoryQuery::from_search(&eval.tagger, term), 10)?;
            let doc = index.get_story_doc(&StoryIdentifier::new(date, url.normalization()))?;
            assert_eq!(
                1, search,
                "Expected one search result when querying '{}' for title={} url={} doc={:?}",
                term, title, url, doc
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
        memindex.insert_scrapes(scrapes)?;

        index.insert_scrape_collections(&eval, memindex.get_all_stories())?;

        // Query the new index
        for story in index.fetch::<Shard>(StoryQuery::from_search(&eval.tagger, "rust"), 10)? {
            println!("{:?}", story);
        }

        Ok(())
    }
}
