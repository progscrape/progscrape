use itertools::Itertools;

use tantivy::collector::TopDocs;
use tantivy::directory::{RamDirectory, MmapDirectory};
use tantivy::query::{BooleanQuery, Occur, Query, RangeQuery, TermQuery, AllQuery};
use tantivy::{doc, Index, DateTime, SegmentReader};
use tantivy::{
    schema::*, Directory, DocAddress, IndexSettings, IndexSortByField, IndexWriter, Searcher,
};

use progscrape_scrapers::{StoryDate, TypedScrape, ScrapeId, StoryUrl};

use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::ops::{RangeBounds, Deref};
use std::sync::{RwLock, Arc, RwLockReadGuard};
use std::time::{Duration, Instant};

use super::*;
use super::scrapestore::ScrapeStore;
use super::shard::{Shard, ShardRange, ShardOrder};

const MEMORY_ARENA_SIZE: usize = 50_000_000;
const STORY_INDEXING_CHUNK_SIZE: usize = 10000;
const SCRAPE_PROCESSING_CHUNK_SIZE: usize = 1000;

/// For performance, we shard stories by time period to allow for more efficient lookup of normalized URLs.
struct StoryIndexShard {
    index: Index,
    url_field: Field,
    url_norm_field: Field,
    url_norm_hash_field: Field,
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
struct StoryInsert<'a> {
    url: &'a str,
    url_norm: &'a str,
    url_norm_hash: i64,
    title: &'a str,
    date: i64,
    score: f64,
    tags: Vec<String>,
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
        let url_field = schema_builder.add_text_field("url", STRING | STORED);
        let url_norm_field = schema_builder.add_text_field("url_norm", FAST | STRING);
        let url_norm_hash_field = schema_builder.add_i64_field("url_norm_hash", FAST | INDEXED);
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
        let directory: Box<dyn Directory> = match location {
            PersistLocation::Memory => Box::new(RamDirectory::create()),
            PersistLocation::Path(path) => {
                let path = path.join(format!("{}/index", shard.to_string()));
                tracing::info!("Opening index at {}", path.to_string_lossy());
                std::fs::create_dir_all(&path)?;
                Box::new(MmapDirectory::open(path)?)
            }
        };
        let index = Index::builder().settings(settings).schema(schema).open_or_create(directory)?;
        let meta = index.load_metas()?;
        tracing::info!("Loaded index with {} doc(s)", meta.segments.iter().fold(0, |a, b| a + b.num_docs()));
        Ok(Self {
            index,
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
        for (segment_ord, segment_reader) in searcher.segment_readers().iter().enumerate() {
            let date = segment_reader.fast_fields().i64(self.date_field)?;
            recent = recent.max(date.max_value());
        }
        Ok(StoryDate::from_millis(recent).unwrap_or_default())
    }

    fn total_docs(&self) -> Result<usize, PersistError> {
        let meta = self.index.load_metas()?;
        Ok(meta.segments.iter().fold(0, |a, b| a + b.num_docs()) as usize)
    }

    fn insert_story_document(
        &mut self,
        writer: &mut IndexWriter,
        doc: StoryInsert,
    ) -> Result<(), PersistError> {
        let mut new_doc = doc! {
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
        writer.add_document(new_doc)?;
        Ok(())
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
            Err(PersistError::Unmappable())
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

    fn lookup_story(&self, searcher: &Searcher, doc_address: DocAddress) -> Result<StoryFetch, PersistError> {
        let doc = searcher.doc(doc_address)?;
        let url = self.text_value(&doc, self.url_field);
        let title = self.text_value(&doc, self.title_field);
        let date = self.i64_value(&doc, self.date_field);
        let score = self.f64_value(&doc, self.score_field);
        let scrape_ids = self.text_values(&doc, self.scrape_field);
        let tags = self.text_values(&doc, self.tags_field);
        Ok(StoryFetch { url, title, date, score, scrape_ids, tags })
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
                    for i in 0..segment_reader.num_docs() {
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
            for r in std::fs::read_dir(path)? {
                if let Ok(d) = r {
                    if let Some(s) = d.file_name().to_str() {
                        if let Some(shard) = Shard::from_string(s) {
                            range.include(shard);
                        }
                    }
                }
            }
        }

        tracing::info!("Found shards {:?}", range);

        let new = Self {
            index_cache: RwLock::new(IndexCache { cache: HashMap::new(), range }),
            location,
            scrape_db,
        };

        Ok(new)
    }
    
    pub fn shards(&self) -> ShardRange {
        self.index_cache.read().expect("Poisoned").range
    }

    fn get_shard_for_date(&self, date: StoryDate) -> Result<Arc<RwLock<StoryIndexShard>>, PersistError> {
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
            Ok(lock.cache.entry(shard).or_insert(Arc::new(RwLock::new(new_shard))).clone())
        }
    }

    fn insert_scrape_batch<'a, I: Iterator<Item = TypedScrape> + 'a>(
        &mut self,
        eval: &StoryEvaluator,
        scrapes: I,
    ) -> Result<(), PersistError> {
        let one_month = Duration::from_secs(60 * 60 * 24 * 30).as_secs() as i64;
        let mut writers = HashMap::new();

        for scrape in scrapes {
            let shard = Shard::from_date_time(scrape.date);
            let shard_index = self.get_shard_for_date(scrape.date)?;
            let mut shard_index = shard_index.write().expect("Poisoned");
            let writer = if let Some(writer) = writers.get_mut(&shard) {
                writer
            } else {
                let writer = shard_index.index.writer(MEMORY_ARENA_SIZE)?;
                writers.entry(shard).or_insert(writer)
            };

            let searcher = shard_index.index.reader()?.searcher();
            let lookup = StoryLookupId {
                url_norm_hash: scrape.url.normalization().hash(),
                date: scrape.date.timestamp(),
            };
            let lookup = HashSet::from_iter([lookup]);
            let result = shard_index.lookup_stories(&searcher, lookup, (-one_month)..one_month)?;
            let lookup = result.into_iter().next().expect("TODO");
            match lookup {
                StoryLookup::Found(id, doc) => {
                    let story = shard_index.lookup_story(&searcher, doc)?;
                    println!("{:?}", story);
                },
                StoryLookup::Unfound(id) => {
                    let story = Story::new(eval, scrape);
                    let mut scrape_ids = vec![];
                    for scrape in &story.scrapes {
                        scrape_ids.push(format!("{}:{}", Shard::from_date_time(scrape.1.date).to_string(), scrape.0.to_string()));
                    }
                    shard_index.insert_story_document(writer, StoryInsert {
                        url: story.url.raw(),
                        url_norm: story.url.normalization().string(),
                        url_norm_hash: story.url.normalization().hash(),
                        score: story.score as f64,
                        date: story.date.timestamp(),
                        title: &story.title,
                        scrape_ids,
                        tags: story.tags.collect()
                    })?;
                }
            };
        }

        let writer_count = writers.len();
        tracing::info!("Preparing to commit {} writer(s)", writer_count);
        let mut v = vec![];
        for (_, writer) in &mut writers {
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

    fn insert_stories<I: Iterator<Item = Story>>(
        &mut self,
        stories: I
    ) -> Result<(), PersistError> {
        let mut writers = HashMap::new();
        let start = Instant::now();
        let mut total = 0;
        for stories in &stories.chunks(STORY_INDEXING_CHUNK_SIZE) {
            tracing::info!("Indexing chunk...");
            let start_chunk = Instant::now();
            let mut count = 0;
            for story in stories {
                count += 1;
                let shard = Shard::from_date_time(story.date);
                let shard_index = self.get_shard_for_date(story.date)?;
                let mut shard_index = shard_index.write().expect("Poisoned");
                let writer = if let Some(writer) = writers.get_mut(&shard) {
                    writer
                } else {
                    let writer = shard_index.index.writer(MEMORY_ARENA_SIZE)?;
                    writers.entry(shard).or_insert(writer)
                };
                let mut scrape_ids = vec![];
                for scrape in &story.scrapes {
                    scrape_ids.push(format!("{}:{}", Shard::from_date_time(scrape.1.date).to_string(), scrape.0.to_string()));
                }
                shard_index.insert_story_document(writer, StoryInsert {
                    url: story.url.raw(),
                    url_norm: story.url.normalization().string(),
                    url_norm_hash: story.url.normalization().hash(),
                    score: story.score as f64,
                    date: story.date.timestamp(),
                    title: &story.title,
                    scrape_ids,
                    tags: story.tags.collect()
                })?;
            }
            total += count;
            tracing::info!("Indexed chunk of {} stories in {} second(s)...", count, start_chunk.elapsed().as_secs());
        }
        tracing::info!("Indexed total of {} stories in {} second(s)...", total, start.elapsed().as_secs());

        let writer_count = writers.len();
        tracing::info!("Commiting {} writer(s)", writer_count);
        let commit_start = Instant::now();
        for (_, writer) in &mut writers {
            writer.commit()?;
        }
        for (_, writer) in writers {
            writer.wait_merging_threads()?;
        }
        tracing::info!("Committed {} writer(s) in {} second(s)...", writer_count, commit_start.elapsed().as_secs());
        Ok(())
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

    fn insert_stories<I: Iterator<Item = Story>>(
            &mut self,
            stories: I
        ) -> Result<(), PersistError> {
        self.insert_stories(stories)
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

    fn get_story(&self, _id: &StoryIdentifier) -> Option<Story> {
        unimplemented!()
    }

    fn stories_by_shard(&self, _shard: &str) -> Result<Vec<Story>, PersistError> {
        unimplemented!()
    }

    fn query_frontpage_hot_set(&self, max_count: usize) -> Result<Vec<Story>, PersistError> {
        let mut vec = vec![];
        for shard in self.shards().iterate(shard::ShardOrder::NewestFirst) {
            if vec.len() > max_count {
                break;
            }
            let index = self.get_shard(shard)?;
            let index = index.read().expect("Poisoned");
            let searcher = index.index.reader()?.searcher();
            let query = AllQuery {};
            let date = index.date_field;
            let top = TopDocs::with_limit(max_count).tweak_score(move |segment_reader: &SegmentReader| {
                let date = segment_reader.fast_fields().i64(date).expect("Failed to get date field");
                move |doc, original_score| {
                    original_score + date.get_val(doc) as f32
                }
            });
            let docs = searcher.search(&query, &top)?;
            for (score, doc_address) in docs {
                let story = index.lookup_story(&searcher, doc_address)?;
                let url = StoryUrl::parse(story.url).expect("Failed to parse URL");
                let date = StoryDate::from_millis(story.date).expect("Failed to re-parse date");
                vec.push(Story::new_from_parts(story.title, url, date, story.tags));
            }
        }
        Ok(vec)
    }

    fn query_search(&self, search: &str, max_count: usize) -> Result<Vec<Story>, PersistError> {
        let vec = vec![];
        for shard in self.shards().iterate(shard::ShardOrder::NewestFirst) {
            let index = self.get_shard(shard)?;
            let index = index.read().expect("Poisoned");
            // println!("Found shard {}", shard);
            let searcher = index.index.reader()?.searcher();
            let query = TermQuery::new(
                Term::from_field_text(index.title_field, &search),
                IndexRecordOption::Basic,
            );
            let docs = searcher.search(&query, &TopDocs::with_limit(max_count))?;
            for doc in docs {
                let doc = searcher.doc(doc.1)?;
                println!("{}", doc.get_first(index.title_field).and_then(|x| x.as_text()).unwrap_or_default());
            }
        }

        Ok(vec)
    }
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use progscrape_scrapers::{hacker_news, StoryUrl, reddit};

    use super::*;

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

    #[test]
    fn test_index_shard() {
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

    #[test]
    fn test_index_basic() -> Result<(), Box<dyn std::error::Error>> {
        tracing_subscriber::fmt().with_max_level(tracing_subscriber::filter::LevelFilter::INFO).init();

        let mut index = StoryIndex::new(PersistLocation::Memory)?;
        let eval = StoryEvaluator::new_for_test();
        let url = StoryUrl::parse("http://example.com").expect("URL");
        let date = StoryDate::year_month_day(2020, 1, 1).expect("Date failed");
        index.insert_scrapes(&eval, [
            hacker_news::HackerNewsStory::new_with_defaults("story1".into(), None, date, "I love Rust".into(), url.clone()).into()
        ].into_iter())?;

        let counts = index.story_count()?;
        assert_eq!(counts.total, 1);

        index.insert_scrapes(&eval, [
            reddit::RedditStory::new_with_defaults("story1".into(), Some("rust".into()), date, "I love Rust".into(), url.clone()).into()
        ].into_iter())?;

        let counts = index.story_count()?;
        assert_eq!(counts.total, 1);

        Ok(())
    }

    #[test]
    fn test_index_stories() -> Result<(), Box<dyn std::error::Error>> {
        tracing_subscriber::fmt().with_max_level(tracing_subscriber::filter::LevelFilter::INFO).init();

        let mut memindex = MemIndex::default();
        let eval = StoryEvaluator::new_for_test();
        let url = StoryUrl::parse("http://example.com").expect("URL");
        let date = StoryDate::year_month_day(2020, 1, 1).expect("Date failed");
        memindex.insert_scrapes(&eval, [
            hacker_news::HackerNewsStory::new_with_defaults("story1".into(), None, date, "I love Rust".into(), url.clone()).into()
        ].into_iter())?;
        memindex.insert_scrapes(&eval, [
            reddit::RedditStory::new_with_defaults("story1".into(), Some("rust".into()), date, "I love Rust".into(), url.clone()).into()
        ].into_iter())?;

        let mut index = StoryIndex::new(PersistLocation::Memory)?;
        index.insert_stories(memindex.get_all_stories())?;

        let counts = index.story_count()?;
        assert_eq!(counts.total, 1);

        Ok(())
    }

    #[test]
    fn test_index_lots() -> Result<(), Box<dyn std::error::Error>> {
        tracing_subscriber::fmt().with_max_level(tracing_subscriber::filter::LevelFilter::INFO).init();

        let path = "/tmp/indextest";
        std::fs::create_dir_all(path)?;
        let mut index = StoryIndex::new(PersistLocation::Path(path.into()))?;

        let scrapes =
            progscrape_scrapers::import_legacy(Path::new(".."))?;
        let eval = StoryEvaluator::new_for_test();
        let mut memindex = MemIndex::default();

        // First, build an in-memory index quickly        
        memindex.insert_scrapes(&eval, scrapes.into_iter())?;

        index.insert_stories(memindex.get_all_stories())?;

        // Query the new index
        for story in index.query_search("rust", 10)? {
            println!("{:?}", story);
        }

        Ok(())
    }
}
