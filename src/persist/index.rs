use itertools::Itertools;
use tantivy::collector::TopDocs;

use tantivy::query::{BooleanQuery, Occur, Query, RangeQuery, TermQuery};
use tantivy::{doc, Index};
use tantivy::{
    schema::*, Directory, DocAddress, IndexSettings, IndexSortByField, IndexWriter,
    Searcher,
};


use crate::scrapers::{Scrape};
use crate::story::StoryDate;

use std::collections::{HashMap, HashSet};
use std::hash::{Hash};
use std::ops::{RangeBounds};
use std::time::{Duration};

use super::*;

const MEMORY_ARENA_SIZE: usize = 50_000_000;
const STORY_INDEXING_CHUNK_SIZE: usize = 10000;

/// For performance, we shard stories by time period to allow for more efficient lookup of normalized URLs.
struct StoryIndexShard {
    index: Index,
    url_field: Field,
    url_norm_field: Field,
    url_norm_hash_field: Field,
    title_field: Field,
    date_field: Field,
    scrape_field: Field,
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
}

impl StoryIndexShard {
    pub fn initialize<DIR: Into<Box<dyn Directory>>>(directory: DIR) -> Result<Self, PersistError> {
        let mut schema_builder = Schema::builder();
        let date_field = schema_builder.add_i64_field("date", FAST | INDEXED);
        let url_field = schema_builder.add_text_field("url", STRING | STORED);
        let url_norm_field = schema_builder.add_text_field("url_norm", FAST | STRING);
        let url_norm_hash_field = schema_builder.add_i64_field("url_norm_hash", FAST | INDEXED);
        let title_field = schema_builder.add_text_field("title", TEXT | STORED);
        let scrape_field = schema_builder.add_json_field("scrapes", TEXT | STORED);
        let schema = schema_builder.build();
        let settings = IndexSettings {
            sort_by_field: Some(IndexSortByField {
                field: "date".to_owned(),
                order: tantivy::Order::Asc,
            }),
            ..Default::default()
        };
        let index = Index::create(directory, schema.clone(), settings)?;
        Ok(Self {
            index,
            url_field,
            url_norm_field,
            url_norm_hash_field,
            title_field,
            date_field,
            scrape_field,
        })
    }

    fn insert_story_document(
        &mut self,
        writer: &mut IndexWriter,
        doc: StoryInsert,
    ) -> Result<(), PersistError> {
        writer.add_document(doc! {
            self.url_field => doc.url,
            self.url_norm_field => doc.url_norm,
            self.url_norm_hash_field => doc.url_norm_hash,
            self.title_field => doc.title,
            self.date_field => doc.date,
        })?;
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
                return true;
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

pub struct StoryIndex {
    index_cache: HashMap<u32, StoryIndexShard>,
    directory_fn: Box<dyn Fn(u32) -> Box<dyn Directory> + Send + Sync>,
    start_date: StoryDate,
}

impl StoryIndex {
    pub fn initialize<DIR: Directory>(
        start_date: StoryDate,
        directory_fn: fn(u32) -> DIR,
    ) -> Result<Self, PersistError> {
        let directory_fn: Box<dyn Fn(u32) -> Box<dyn Directory> + Send + Sync> =
            Box::new(move |date| Box::new(directory_fn(date)));
        Ok(Self {
            index_cache: HashMap::new(),
            directory_fn,
            start_date,
        })
    }

    fn index_for_date(&self, date: StoryDate) -> u32 {
        (date.year() as u32) * 12 + date.month0()
    }

    fn ensure_shard<'a>(&mut self, shard: u32) -> Result<(), PersistError> {
        if !self.index_cache.contains_key(&shard) {
            println!("Creating shard {}", shard);
            let new_shard = StoryIndexShard::initialize((self.directory_fn)(shard))?;
            self.index_cache.insert(shard, new_shard);
        }
        Ok(())
    }

    fn insert_scrape_batch<'a, I: Iterator<Item = Story> + 'a>(
        &mut self,
        scrapes: I,
    ) -> Result<(), PersistError> {
        // Split stories by index shard
        let mut sharded = HashMap::<u32, Vec<Story>>::new();
        for scrape in scrapes {
            sharded
                .entry(self.index_for_date(scrape.date()))
                .or_default()
                .push(scrape);
        }

        for (shard, stories) in sharded {
            self.ensure_shard(shard)?;
            let index = self.index_cache.get_mut(&shard).expect("Shard was missing");
            let mut writer = index.index.writer(MEMORY_ARENA_SIZE)?;
            let searcher = index.index.reader()?.searcher();
            let iter = stories.into_iter().enumerate().map(|(_i, story)| {
                (
                    StoryLookupId {
                        url_norm_hash: story.normalized_url_hash(),
                        date: story.date().timestamp(),
                    },
                    story,
                )
            });
            let stories = HashMap::<_, _>::from_iter(iter);
            let story_ids = HashSet::from_iter(stories.keys().map(|x| *x));
            let one_month = Duration::from_secs(60 * 60 * 24 * 30).as_secs() as i64;
            let result = index.lookup_stories(&searcher, story_ids, (-one_month)..one_month)?;
            for result in result {
                match result {
                    StoryLookup::Found(a, b) => {
                        let _story = stories.get(&a).expect("Didn't find a story we should have");
                        let _url = searcher
                            .doc(b)?
                            .get_first(index.url_field)
                            .unwrap()
                            .as_text()
                            .unwrap_or_default()
                            .to_owned();
                        let _title = searcher
                            .doc(b)?
                            .get_first(index.title_field)
                            .unwrap()
                            .as_text()
                            .unwrap_or_default()
                            .to_owned();
                    }
                    StoryLookup::Unfound(a) => {
                        let story = stories.get(&a).expect("Didn't find a story we should have");
                        index.insert_story_document(
                            &mut writer,
                            StoryInsert {
                                url: &story.url(),
                                url_norm: &story.normalized_url,
                                url_norm_hash: story.normalized_url_hash(),
                                title: &story.title(),
                                date: story.date().timestamp(),
                            },
                        )?;
                    }
                }
            }
            writer.commit()?;
        }

        Ok(())
    }

    /// Insert a list of scrapes into the index.
    fn insert_scrapes<'a, I: Iterator<Item = Scrape> + 'a>(
        &mut self,
        scrapes: I,
    ) -> Result<(), PersistError> {
        let mut memindex = memindex::MemIndex::default();
        memindex.insert_scrapes(scrapes)?;

        for chunk in &memindex.get_all_stories().chunks(STORY_INDEXING_CHUNK_SIZE) {
            println!("Chunk");
            self.insert_scrape_batch(chunk)?;
        }

        Ok(())
    }
}

impl StorageWriter for StoryIndex {
    fn insert_scrapes<'a, I: Iterator<Item = Scrape> + 'a>(
        &mut self,
        scrapes: I,
    ) -> Result<(), PersistError> {
        self.insert_scrapes(scrapes)
    }
}

impl Storage for StoryIndex {
    fn story_count(&self) -> Result<StorageSummary, PersistError> {
        let now = StoryDate::now();
        let mut summary = StorageSummary::default();
        for shard in (self.index_for_date(self.start_date)..self.index_for_date(now)).rev() {
            let index = self.index_cache.get(&shard);
            let mut subtotal = 0;
            if let Some(index) = index {
                let meta = index.index.load_metas()?;
                subtotal += meta.segments.iter().fold(0, |a, b| a + b.num_docs()) as usize;
            }
            summary.by_shard.push((shard.to_string(), subtotal));
            summary.total += subtotal;
        }
        Ok(summary)
    }

    fn stories_by_shard(&self, _shard: &str) -> Result<Vec<Story>, PersistError> {
        unimplemented!()
    }

    fn query_frontpage(&self, _max_count: usize) -> Result<Vec<Story>, PersistError> {
        unimplemented!()
    }

    fn query_search(&self, search: String, max_count: usize) -> Result<Vec<Story>, PersistError> {
        let now = StoryDate::now();
        let vec = vec![];
        for shard in (self.index_for_date(self.start_date)..self.index_for_date(now)).rev() {
            let index = self.index_cache.get(&shard);
            if let Some(index) = index {
                // println!("Found shard {}", shard);
                let searcher = index.index.reader()?.searcher();
                let query = TermQuery::new(
                    Term::from_field_text(index.title_field, &search),
                    IndexRecordOption::Basic,
                );
                let docs = searcher.search(&query, &TopDocs::with_limit(max_count))?;
                for doc in docs {
                    let _doc = searcher.doc(doc.1)?;
                    // println!("{}", doc.get_first(index.title_field).and_then(|x| x.as_text()).unwrap_or_default());
                }
            }
        }

        Ok(vec)
    }
}

#[cfg(test)]
mod test {
    use tantivy::directory::RamDirectory;

    use crate::scrapers::ScrapeData;

    use super::*;

    fn populate_shard(
        ids: impl Iterator<Item = (i64, i64)>,
    ) -> Result<StoryIndexShard, PersistError> {
        let dir = RamDirectory::create();
        let mut shard = StoryIndexShard::initialize(dir)?;
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
                .collect::<Vec<_>>()
                .len()
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
    fn test_index_lots() {
        let stories = crate::scrapers::legacy_import::import_legacy()
            .expect("Failed to read scrapes")
            .collect::<Vec<_>>();
        let start_date = stories
            .iter()
            .fold(StoryDate::MAX, |a, b| std::cmp::min(a, b.date()));
        // let stories = crate::scrapers::test::scrape_all();
        // let dir = MmapDirectory::open("/tmp/index").expect("Failed to get mmap dir");
        let _dir = RamDirectory::create();
        let mut index = StoryIndex::initialize(start_date, |_n| RamDirectory::create())
            .expect("Failed to initialize index");
        index
            .insert_scrapes(stories.into_iter())
            .expect("Failed to insert scrapes");
        index.query_search("rust".to_owned(), 10);
    }
}
