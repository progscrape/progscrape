use itertools::Itertools;

use tantivy::directory::{MmapDirectory, RamDirectory};
use tantivy::tokenizer::{PreTokenizedString, SimpleTokenizer, Tokenizer};
use tantivy::{doc, Index, IndexReader};
use tantivy::{
    schema::*, Directory, DocAddress, IndexSettings, IndexSortByField, IndexWriter, Searcher,
};

use progscrape_scrapers::{ScrapeId, StoryDate};

use std::collections::HashSet;
use std::hash::Hash;
use std::ops::RangeBounds;
use std::sync::RwLock;
use std::time::Instant;

use crate::persist::{ScrapePersistResult, Shard};
use crate::story::{StoryScrapeId, TagSet};
use crate::{PersistError, PersistLocation};

use super::schema::StorySchema;

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct StoryLookupId {
    pub url_norm_hash: i64,
    pub date: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum StoryLookup {
    Unfound(StoryLookupId),
    Found(StoryLookupId, DocAddress),
}

#[derive(Default)]
pub struct StoryInsert {
    pub id: String,
    pub host: String,
    pub url: String,
    pub url_norm: String,
    pub url_norm_hash: i64,
    pub title: String,
    pub date: i64,
    pub score: f64,
    pub tags: TagSet,
    pub scrape_ids: Vec<String>,
}

#[derive(Debug)]
pub struct StoryFetch {
    pub url: String,
    pub title: String,
    pub date: i64,
    pub score: f64,
    pub tags: Vec<String>,
    pub scrape_ids: Vec<StoryScrapeId>,
}

/// For performance, we shard stories by time period to allow for more efficient lookup of normalized URLs.
pub struct StoryIndexShard {
    shard: Shard,
    index: Index,
    maybe_searcher: RwLock<Option<(IndexReader, Searcher)>>,
    schema: StorySchema,
}

const MEMORY_ARENA_SIZE: usize = 50_000_000;

/// The `StoryIndexShard` manages a single shard of the index.
impl StoryIndexShard {
    pub(crate) fn initialize(
        location: PersistLocation,
        shard: Shard,
        schema: StorySchema,
    ) -> Result<Self, PersistError> {
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
            .schema(schema.schema.clone())
            .open_or_create(directory)?;
        if exists {
            let meta = index.load_metas()?;
            tracing::info!(
                "Loaded existing index for {shard:?} with {} doc(s)",
                meta.segments.iter().fold(0, |a, b| a + b.num_docs())
            );
        } else {
            tracing::info!("Created and initialized new index for {shard:?}");
        }

        Ok(Self {
            shard,
            index,
            maybe_searcher: Default::default(),
            schema,
        })
    }

    /// Provides a valid searcher and schema temporarily for the callback function.
    #[inline(always)]
    pub fn with_searcher<F: FnOnce(&Searcher, &StorySchema) -> Result<T, PersistError>, T>(
        &self,
        f: F,
    ) -> Result<T, PersistError> {
        // We only ever transition from None -> Some so this never deadlocks
        loop {
            if let Some((_, searcher)) = &*self.maybe_searcher.read().expect("Poisoned") {
                break f(searcher, &self.schema);
            }
            // Check to see if we won the write race
            let mut lock = self.maybe_searcher.write().expect("Poisoned");
            if lock.is_none() {
                *lock = {
                    let now = Instant::now();
                    let reader = self.index.reader()?;
                    let searcher = reader.searcher();
                    tracing::info!(
                        "Created reader and searcher for {:?} in {}ms",
                        self.shard,
                        now.elapsed().as_millis()
                    );
                    Some((reader, searcher))
                }
                // TODO: if we switch to parking lot, we get downgrading for free, otherwise loop around and take just the
                // read lock
            }
        }
    }

    pub fn writer(&self) -> Result<IndexWriter, PersistError> {
        Ok(self.index.writer(MEMORY_ARENA_SIZE)?)
    }

    pub fn commit_writer(&mut self, mut writer: IndexWriter) -> Result<(), PersistError> {
        writer.commit()?;
        // Reload reader and search iff they were loaded
        if let Some((reader, searcher)) = &mut *self.maybe_searcher.write().expect("Poisoned") {
            reader.reload()?;
            *searcher = reader.searcher();
        }
        writer.wait_merging_threads()?;
        Ok(())
    }

    pub fn most_recent_story(&self) -> Result<StoryDate, PersistError> {
        let searcher = self.index.reader()?.searcher();
        let mut recent = 0;
        for (_segment_ord, segment_reader) in searcher.segment_readers().iter().enumerate() {
            let date = segment_reader.fast_fields().i64(self.schema.date_field)?;
            recent = recent.max(date.max_value());
        }
        Ok(StoryDate::from_seconds(recent).unwrap_or_default())
    }

    pub fn total_docs(&self) -> Result<usize, PersistError> {
        let meta = self.index.load_metas()?;
        Ok(meta.segments.iter().fold(0, |a, b| a + b.num_docs()) as usize)
    }

    /// Re-insert a story document, deleting the old one first.
    pub fn reinsert_story_document(
        &self,
        writer: &mut IndexWriter,
        doc: StoryInsert,
    ) -> Result<ScrapePersistResult, PersistError> {
        writer.delete_term(Term::from_field_text(self.schema.id_field, &doc.id));
        self.insert_story_document(writer, doc)?;
        Ok(ScrapePersistResult::MergedWithExistingStory)
    }

    /// Insert a brand-new story document.
    pub fn insert_story_document(
        &self,
        writer: &mut IndexWriter,
        doc: StoryInsert,
    ) -> Result<ScrapePersistResult, PersistError> {
        let mut new_doc = doc! {
            self.schema.id_field => doc.id,
            self.schema.url_field => doc.url,
            self.schema.url_norm_field => doc.url_norm,
            self.schema.url_norm_hash_field => doc.url_norm_hash,
            self.schema.title_field => doc.title,
            self.schema.date_field => doc.date,
            self.schema.score_field => doc.score,
        };
        for id in doc.scrape_ids {
            new_doc.add_text(self.schema.scrape_field, id);
        }
        for tag in doc.tags {
            new_doc.add_text(self.schema.tags_field, tag);
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
            self.schema.host_field,
            PreTokenizedString {
                text: doc.host,
                tokens,
            },
        );
        writer.add_document(new_doc)?;
        Ok(ScrapePersistResult::NewStory)
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

    /// Does the tricky work of converting indexed `StoryScrapeId`s to full ones.
    pub fn extract_scrape_ids_from_doc(&self, doc: &Document) -> Vec<StoryScrapeId> {
        self.text_values(doc, self.schema.scrape_field)
            .into_iter()
            .filter_map(|id| {
                if let Some((a, b)) = id.split_once(':') {
                    if let (Some(shard), Some(id)) =
                        (Shard::from_string(a), ScrapeId::from_string(b))
                    {
                        return Some(StoryScrapeId { id, shard });
                    }
                }
                None
            })
            .collect_vec()
    }

    pub fn lookup_story(&self, doc_address: DocAddress) -> Result<StoryFetch, PersistError> {
        let doc = self.doc(doc_address)?;
        let url = self.text_value(&doc, self.schema.url_field);
        let title = self.text_value(&doc, self.schema.title_field);
        let date = self.i64_value(&doc, self.schema.date_field);
        let score = self.f64_value(&doc, self.schema.score_field);
        let scrape_ids = self.extract_scrape_ids_from_doc(&doc);
        let tags = self.text_values(&doc, self.schema.tags_field);
        Ok(StoryFetch {
            url,
            title,
            date,
            score,
            scrape_ids,
            tags,
        })
    }

    pub fn doc(&self, doc_address: DocAddress) -> Result<Document, PersistError> {
        self.with_searcher(|searcher, _| Ok(searcher.doc(doc_address)?))
    }

    pub fn doc_fields(&self, doc_address: DocAddress) -> Result<NamedFieldDocument, PersistError> {
        let doc = self.doc(doc_address)?;
        let named_doc = self.schema.schema.to_named_doc(&doc);
        Ok(named_doc)
    }

    /// Given a set of `StoryLookupId`s, computes the documents that match them.
    pub fn lookup_stories(
        &self,
        mut stories: HashSet<StoryLookupId>,
        date_range: impl RangeBounds<i64>,
    ) -> Result<Vec<StoryLookup>, PersistError> {
        self.with_searcher(move |searcher, _| {
            let mut result = vec![];
            for (segment_ord, segment_reader) in searcher.segment_readers().iter().enumerate() {
                let index = segment_reader
                    .fast_fields()
                    .i64(self.schema.url_norm_hash_field)?;
                let date = segment_reader.fast_fields().i64(self.schema.date_field)?;
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
        })
    }
}
