use itertools::Itertools;

use tantivy::collector::TopDocs;
use tantivy::directory::{MmapDirectory, RamDirectory};
use tantivy::query::{AllQuery, BooleanQuery, Occur, PhraseQuery, Query, RangeQuery, TermQuery};
use tantivy::tokenizer::{PreTokenizedString, SimpleTokenizer, Tokenizer};
use tantivy::{doc, Index};
use tantivy::{
    schema::*, Directory, DocAddress, IndexSettings, IndexSortByField, IndexWriter, Searcher,
};

use progscrape_scrapers::{ScrapeCore, ScrapeId, StoryDate, StoryUrl, TypedScrape};

use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::ops::RangeBounds;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use crate::story::{StoryCollector, TagSet};

use super::scrapestore::ScrapeStore;
use super::shard::{Shard, ShardOrder, ShardRange};
use super::*;

const MEMORY_ARENA_SIZE: usize = 50_000_000;
const STORY_INDEXING_CHUNK_SIZE: usize = 10000;
const SCRAPE_PROCESSING_CHUNK_SIZE: usize = 1000;

/// For performance, we shard stories by time period to allow for more efficient lookup of normalized URLs.
struct StoryIndexShard {
    index: Index,
    id_field: Field,
    url_field: Field,
    url_norm_field: Field,
    url_norm_hash_field: Field,
    host_field: Field,
    score_field: Field,
    title_field: Field,
    date_field: Field,
    scrape_field: Field,
    tags_field: Field,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
struct StoryLookupId {
    url_norm_hash: i64,
    date: i64,
}

#[derive(Debug, PartialEq, Eq)]
enum StoryLookup {
    Unfound(StoryLookupId),
    Found(StoryLookupId, DocAddress),
}

#[derive(Default)]
struct StoryInsert {
    id: String,
    host: String,
    url: String,
    url_norm: String,
    url_norm_hash: i64,
    title: String,
    date: i64,
    score: f64,
    tags: TagSet,
    scrape_ids: Vec<String>,
}

#[derive(Debug)]
struct StoryFetch {
    url: String,
    title: String,
    date: i64,
    score: f64,
    tags: Vec<String>,
    scrape_ids: Vec<String>,
}

/// The `StoryIndexShard` manages a single shard of the index.
impl StoryIndexShard {
    pub fn initialize(location: PersistLocation, shard: Shard) -> Result<Self, PersistError> {
        let mut schema_builder = Schema::builder();
        let date_field = schema_builder.add_i64_field("date", FAST | STORED);
        let id_field = schema_builder.add_text_field("id", STRING | STORED);
        let url_field = schema_builder.add_text_field("url", STRING | STORED);
        let url_norm_field = schema_builder.add_text_field("url_norm", FAST | STRING);
        let url_norm_hash_field = schema_builder.add_i64_field("url_norm_hash", FAST | INDEXED);
        let host_field = schema_builder.add_text_field("host", TEXT | STORED);
        let title_field = schema_builder.add_text_field("title", TEXT | STORED);
        let scrape_field = schema_builder.add_text_field("scrapes", TEXT | STORED);
        let score_field = schema_builder.add_f64_field("score", FAST | STORED);
        let tags_field = schema_builder.add_text_field("tags", TEXT | STORED);
        let schema = schema_builder.build();
        let settings = IndexSettings {
            sort_by_field: Some(IndexSortByField {
                field: "date".to_owned(),
                order: tantivy::Order::Asc,
            }),
            ..Default::default()
        };
        let (directory, exists): (Box<dyn Directory>, bool) = match location {
            PersistLocation::Memory => (Box::new(RamDirectory::create()), false),
            PersistLocation::Path(path) => {
                let path = path.join(format!("{}/index", shard.to_string()));
                tracing::info!("Opening index at {}", path.to_string_lossy());
                std::fs::create_dir_all(&path)?;
                let dir = MmapDirectory::open(path)?;
                let exists = Index::exists(&dir).unwrap_or(false);
                (Box::new(dir), exists)
            }
        };
        let index = Index::builder()
            .settings(settings)
            .schema(schema)
            .open_or_create(directory)?;
        if exists {
            let meta = index.load_metas()?;
            tracing::info!(
                "Loaded existing index with {} doc(s)",
                meta.segments.iter().fold(0, |a, b| a + b.num_docs())
            );
        } else {
            tracing::info!("Created and initialized new index");
        }
        Ok(Self {
            index,
            id_field,
            host_field,
            url_field,
            url_norm_field,
            url_norm_hash_field,
            score_field,
            title_field,
            date_field,
            scrape_field,
            tags_field,
        })
    }

    fn most_recent_story(&self) -> Result<StoryDate, PersistError> {
        let searcher = self.index.reader()?.searcher();
        let mut recent = 0;
        for (_segment_ord, segment_reader) in searcher.segment_readers().iter().enumerate() {
            let date = segment_reader.fast_fields().i64(self.date_field)?;
            recent = recent.max(date.max_value());
        }
        Ok(StoryDate::from_seconds(recent).unwrap_or_default())
    }

    fn total_docs(&self) -> Result<usize, PersistError> {
        let meta = self.index.load_metas()?;
        Ok(meta.segments.iter().fold(0, |a, b| a + b.num_docs()) as usize)
    }

    fn insert_story_document(
        &mut self,
        writer: &mut IndexWriter,
        doc: StoryInsert,
    ) -> Result<ScrapePersistResult, PersistError> {
        let mut new_doc = doc! {
            self.id_field => doc.id,
            self.url_field => doc.url,
            self.url_norm_field => doc.url_norm,
            self.url_norm_hash_field => doc.url_norm_hash,
            self.title_field => doc.title,
            self.date_field => doc.date,
            self.score_field => doc.score,
        };
        for id in doc.scrape_ids {
            new_doc.add_text(self.scrape_field, id);
        }
        for tag in doc.tags {
            new_doc.add_text(self.tags_field, tag);
        }

        let tokens = {
            let mut token_stream = SimpleTokenizer.token_stream(&doc.host);
            let mut tokens = vec![];
            while token_stream.advance() {
                tokens.push(token_stream.token().clone());
            }
            tokens
        };
        new_doc.add_pre_tokenized_text(
            self.host_field,
            PreTokenizedString {
                text: doc.host,
                tokens,
            },
        );
        writer.add_document(new_doc)?;
        Ok(ScrapePersistResult::NewStory)
    }

    fn add_scrape_id(
        &mut self,
        writer: &mut IndexWriter,
        searcher: &Searcher,
        doc_address: DocAddress,
        mut scrape_ids: HashSet<String>,
    ) -> Result<ScrapePersistResult, PersistError> {
        let mut doc = searcher.doc(doc_address)?;

        // Fast exit if these scrapes have already been added
        for value in doc.get_all(self.scrape_field) {
            if let Some(id) = value.as_text() {
                scrape_ids.remove(id);
                if scrape_ids.len() == 0 {
                    return Ok(ScrapePersistResult::AlreadyPartOfExistingStory);
                }
            }
        }

        let id = doc
            .get_first(self.id_field)
            .ok_or(PersistError::UnexpectedError(
                "No ID field in document".into(),
            ))?;
        let id = id
            .as_text()
            .ok_or(PersistError::UnexpectedError(
                "Unable to convert ID field to string".into(),
            ))?
            .to_string();
        writer.delete_term(Term::from_field_text(self.id_field, &id));
        for id in scrape_ids {
            doc.add_text(self.scrape_field, id);
        }

        // Re-add the norm hash
        let norm = searcher
            .segment_reader(doc_address.segment_ord)
            .fast_fields()
            .i64(self.url_norm_hash_field)?;
        doc.add_i64(self.url_norm_hash_field, norm.get_val(doc_address.doc_id));

        writer.add_document(doc)?;
        Ok(ScrapePersistResult::MergedWithExistingStory)
    }

    fn create_norm_query(
        &self,
        _url_norm: &str,
        url_norm_hash: i64,
        date: StoryDate,
    ) -> Result<impl Query, PersistError> {
        if let (Some(start), Some(end)) = (date.checked_sub_months(1), date.checked_add_months(1)) {
            let url_query = Box::new(TermQuery::new(
                Term::from_field_i64(self.url_norm_hash_field, url_norm_hash),
                IndexRecordOption::Basic,
            ));
            let date_range_query = Box::new(RangeQuery::new_i64(
                self.date_field,
                start.timestamp()..end.timestamp(),
            ));
            Ok(BooleanQuery::new(vec![
                (Occur::Must, url_query),
                (Occur::Must, date_range_query),
            ]))
        } else {
            // Extremely unlikely
            Err(PersistError::UnexpectedError(
                "Could not map date range".into(),
            ))
        }
    }

    fn text_value(&self, doc: &Document, field: Field) -> String {
        if let Some(val) = doc.get_first(field) {
            val.as_text().unwrap_or_default().to_owned()
        } else {
            "".to_owned()
        }
    }

    fn text_values(&self, doc: &Document, field: Field) -> Vec<String> {
        let mut v = vec![];
        for value in doc.get_all(field) {
            if let Some(s) = value.as_text() {
                v.push(s.into());
            }
        }
        v
    }

    fn i64_value(&self, doc: &Document, field: Field) -> i64 {
        if let Some(val) = doc.get_first(field) {
            val.as_i64().unwrap_or_default().to_owned()
        } else {
            0
        }
    }

    fn f64_value(&self, doc: &Document, field: Field) -> f64 {
        if let Some(val) = doc.get_first(field) {
            val.as_f64().unwrap_or_default().to_owned()
        } else {
            0.0
        }
    }

    fn lookup_story(
        &self,
        searcher: &Searcher,
        doc_address: DocAddress,
    ) -> Result<StoryFetch, PersistError> {
        let doc = searcher.doc(doc_address)?;
        let url = self.text_value(&doc, self.url_field);
        let title = self.text_value(&doc, self.title_field);
        let date = self.i64_value(&doc, self.date_field);
        let score = self.f64_value(&doc, self.score_field);
        let scrape_ids = self.text_values(&doc, self.scrape_field);
        let tags = self.text_values(&doc, self.tags_field);
        Ok(StoryFetch {
            url,
            title,
            date,
            score,
            scrape_ids,
            tags,
        })
    }

    fn doc_fields(
        &self,
        searcher: &Searcher,
        doc_address: DocAddress,
    ) -> Result<NamedFieldDocument, PersistError> {
        let doc = searcher.doc(doc_address)?;
        let named_doc = searcher.schema().to_named_doc(&doc);
        Ok(named_doc)
    }

    /// Given a set of `StoryLookupId`s, computes the documents that match them.
    fn lookup_stories(
        &self,
        searcher: &Searcher,
        mut stories: HashSet<StoryLookupId>,
        date_range: impl RangeBounds<i64>,
    ) -> Result<Vec<StoryLookup>, PersistError> {
        let mut result = vec![];
        for (segment_ord, segment_reader) in searcher.segment_readers().iter().enumerate() {
            let index = segment_reader.fast_fields().i64(self.url_norm_hash_field)?;
            let date = segment_reader.fast_fields().i64(self.date_field)?;
            let (min, max) = (index.min_value(), index.max_value());
            stories.retain(|story| {
                if min <= story.url_norm_hash && max >= story.url_norm_hash {
                    for i in segment_reader.doc_ids_alive() {
                        if index.get_val(i) == story.url_norm_hash {
                            let date = date.get_val(i) - story.date;
                            if !date_range.contains(&date) {
                                return true;
                            }
                            result.push(StoryLookup::Found(
                                *story,
                                DocAddress::new(segment_ord as u32, i),
                            ));
                            return false;
                        }
                    }
                }
                true
            });
            // Early exit optimization
            if stories.is_empty() {
                break;
            }
        }
        result.extend(stories.into_iter().map(StoryLookup::Unfound));
        Ok(result)
    }
}

struct IndexCache {
    cache: HashMap<Shard, Arc<RwLock<StoryIndexShard>>>,
    range: ShardRange,
}

pub struct StoryIndex {
    index_cache: RwLock<IndexCache>,
    location: PersistLocation,
    scrape_db: ScrapeStore,
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
        };

        Ok(new)
    }

    pub fn shards(&self) -> ShardRange {
        self.index_cache.read().expect("Poisoned").range
    }

    fn get_shard_for_date(
        &self,
        date: StoryDate,
    ) -> Result<Arc<RwLock<StoryIndexShard>>, PersistError> {
        self.get_shard(Shard::from_date_time(date))
    }

    fn get_shard(&self, shard: Shard) -> Result<Arc<RwLock<StoryIndexShard>>, PersistError> {
        let mut lock = self.index_cache.write().expect("Poisoned");
        if let Some(shard) = lock.cache.get(&shard) {
            Ok(shard.clone())
        } else {
            tracing::info!("Creating shard {}", shard.to_string());
            let new_shard = StoryIndexShard::initialize(self.location.clone(), shard)?;
            lock.range.include(shard);
            Ok(lock
                .cache
                .entry(shard)
                .or_insert(Arc::new(RwLock::new(new_shard)))
                .clone())
        }
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
        tracing::info!("Preparing to commit {} writer(s)", writer_count);
        let mut v = vec![];
        for writer in writers.values_mut() {
            v.push(writer.prepare_commit()?);
        }
        tracing::info!("Committing {} writer(s)", writer_count);
        for writer in v {
            writer.commit()?;
        }

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
        for writer in writers.values_mut() {
            writer.commit()?;
        }
        for (_, writer) in writers {
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
        index: &StoryIndexShard,
        searcher: &Searcher,
        doc_address: DocAddress,
    ) -> Result<Story, PersistError> {
        let story = index.lookup_story(searcher, doc_address)?;
        let url = StoryUrl::parse(story.url).expect("Failed to parse URL");
        let date = StoryDate::from_seconds(story.date).expect("Failed to re-parse date");
        let score = story.score as f32;
        let mut scrapes = HashSet::new();
        for id in story.scrape_ids {
            if let Some((_, b)) = id.split_once(':') {
                if let Some(id) = ScrapeId::from_string(b.into()) {
                    scrapes.insert(id);
                }
            }
        }
        Ok(Story::new_from_parts(
            story.title,
            url,
            date,
            score,
            story.tags,
            scrapes,
        ))
    }

    fn get_story_doc(
        &self,
        id: &StoryIdentifier,
    ) -> Result<Option<NamedFieldDocument>, PersistError> {
        let shard = self.get_shard(Shard::from_year_month(id.year(), id.month()))?;
        let shard = shard.read().expect("Poisoned");
        let query = TermQuery::new(
            Term::from_field_text(shard.id_field, &id.to_base64()),
            IndexRecordOption::Basic,
        );
        let searcher = shard.index.reader()?.searcher();
        let docs = searcher.search(&query, &TopDocs::with_limit(1))?;
        for (_, doc_address) in docs {
            let doc = shard.doc_fields(&searcher, doc_address)?;
            return Ok(Some(doc));
        }
        Ok(None)
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
        for shard in self.shards().iterate(shard::ShardOrder::OldestFirst) {
            let index = self.get_shard(shard)?;
            let subtotal = index.read().expect("Poisoned").total_docs()?;
            summary.by_shard.push((shard.to_string(), subtotal));
            summary.total += subtotal;
        }
        Ok(summary)
    }

    fn get_story(&self, _id: &StoryIdentifier) -> Option<(Story, ScrapeCollection)> {
        unimplemented!()
    }

    fn stories_by_shard(&self, shard: &str) -> Result<Vec<Story>, PersistError> {
        if let Some(shard) = Shard::from_string(shard) {
            let index = self.get_shard(shard)?;
            let index = index.read().expect("Poisoned");
            let searcher = index.index.reader()?.searcher();

            let mut v = vec![];
            let now = Instant::now();
            for (idx, segment_reader) in searcher.segment_readers().iter().enumerate() {
                for doc_id in segment_reader.doc_ids_alive() {
                    let doc_address = DocAddress::new(idx as u32, doc_id);
                    v.push(Self::lookup_story(&index, &searcher, doc_address)?);
                }
            }
            tracing::info!(
                "Loaded {} stories from shard {:?} in {}ms",
                v.len(),
                shard,
                now.elapsed().as_millis()
            );
            Ok(v)
        } else {
            Ok(vec![])
        }
    }

    fn query_frontpage_hot_set(&self, max_count: usize) -> Result<Vec<Story>, PersistError> {
        let mut story_collector = StoryCollector::new(max_count);
        let mut processed = 0;
        let processing_target = max_count * 2;
        for shard in self.shards().iterate(shard::ShardOrder::NewestFirst) {
            // Process at least twice as many stories as requested
            if processed >= processing_target {
                break;
            }

            let index = self.get_shard(shard)?;
            let index = index.read().expect("Poisoned");
            let searcher = index.index.reader()?.searcher();
            let query = AllQuery {};
            let date = index.date_field;

            let top =
                TopDocs::with_limit(processing_target - processed).order_by_fast_field::<i64>(date);
            let docs = searcher.search(&query, &top)?;
            tracing::info!("Got {} doc(s) from shard {:?}", docs.len(), shard);
            for (_, doc_address) in docs {
                processed += 1;
                let score = searcher
                    .segment_reader(doc_address.segment_ord)
                    .fast_fields()
                    .f64(index.score_field)?
                    .get_val(doc_address.doc_id) as f32;
                if story_collector.would_accept(score) {
                    story_collector.accept(Self::lookup_story(&index, &searcher, doc_address)?);
                }
            }
        }
        tracing::info!(
            "Got {}/{} docs for front page",
            story_collector.len(),
            processed
        );
        Ok(story_collector.to_sorted())
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
        let mut vec = vec![];
        for shard in self.shards().iterate(shard::ShardOrder::NewestFirst) {
            let index = self.get_shard(shard)?;
            let index = index.read().expect("Poisoned");
            // println!("Found shard {}", shard);
            let searcher = index.index.reader()?.searcher();

            // This isn't terribly smart, buuuuut it allows us to search either a tag or site
            let docs = if let Some(tag) = tagger.check_tag_search(search) {
                let query = TermQuery::new(
                    Term::from_field_text(index.tags_field, tag),
                    IndexRecordOption::Basic,
                );
                tracing::debug!("Tag symbol query = {:?}", query);
                searcher.search(&query, &TopDocs::with_limit(max_count))?
            } else if search.contains('.') {
                let host_field = index.host_field;
                let phrase = search
                    .split(|c: char| !c.is_alphanumeric())
                    .map(|s| Term::from_field_text(host_field, s))
                    .enumerate()
                    .collect_vec();
                let query = PhraseQuery::new_with_offset(phrase);
                tracing::debug!("Phrase query = {:?}", query);
                searcher.search(&query, &TopDocs::with_limit(max_count))?
            } else {
                let query = TermQuery::new(
                    Term::from_field_text(index.title_field, search),
                    IndexRecordOption::Basic,
                );
                tracing::debug!("Term query = {:?}", query);
                searcher.search(&query, &TopDocs::with_limit(max_count))?
            };

            for (_score, doc_address) in docs {
                vec.push(Self::lookup_story(&index, &searcher, doc_address)?);
            }
        }

        Ok(vec)
    }
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use super::*;
    use progscrape_scrapers::{hacker_news::*, reddit::*, ScrapeSource, StoryUrl};

    use crate::{story::TagSet, test::*};
    use rstest::*;

    fn populate_shard(
        ids: impl Iterator<Item = (i64, i64)>,
    ) -> Result<StoryIndexShard, PersistError> {
        let mut shard = StoryIndexShard::initialize(PersistLocation::Memory, Shard::default())?;
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
        assert_eq!(
            HashSet::from_iter([
                HackerNews.id("story1"),
                Reddit.subsource_id("rust", "story1")
            ]),
            story.scrapes
        );
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
        assert_eq!(
            HashSet::from_iter([
                HackerNews.id("story1"),
                Reddit.subsource_id("rust", "story1")
            ]),
            story.scrapes
        );
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
        let doc = index.get_story_doc(&StoryIdentifier::new(date, url.normalization()))?;

        index.insert_scrapes(
            &eval,
            [reddit_story("story-3", "subreddit", date, "Title 3", &url)].into_iter(),
        )?;

        let doc = index.get_story_doc(&StoryIdentifier::new(date, url.normalization()))?;

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
