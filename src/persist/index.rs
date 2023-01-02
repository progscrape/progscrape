
use std::collections::HashMap;
use std::hash::Hash;

use chrono::{DateTime, Utc, Months};
use itertools::Itertools;
use tantivy::collector::TopDocs;
use tantivy::directory::MmapDirectory;
use tantivy::query::{BooleanQuery, TermQuery, QueryParser, Occur, RangeQuery};
use tantivy::{schema::*, IndexSettings, IndexSortByField, Directory};
use tantivy::{doc, DocId, Index, Score, SegmentReader};
use url::Url;

use crate::datasci::urlnormalizer::url_normalization_string;
use crate::scrapers::{Scrape, ScrapeSource};

use super::*;

const MEMORY_ARENA_SIZE: usize = 50_000_000;
const STORY_INDEXING_CHUNK_SIZE: usize = 10000;

struct StoryIndex {
    index: Index,
    url_field: Field,
    url_norm_field: Field,
    title_field: Field,
    date_field: Field,
}

impl StoryIndex {
    pub fn initialize<DIR: Directory>(directory: DIR) -> Result<Self, PersistError> {
        let mut schema_builder = Schema::builder();
        let date_field = schema_builder.add_i64_field("date", FAST | INDEXED);
        let url_field = schema_builder.add_text_field("url", STRING | STORED);  
        let url_norm_field = schema_builder.add_text_field("url_norm", STRING | STORED);  
        let title_field = schema_builder.add_text_field("title", TEXT | STORED);  
        let schema = schema_builder.build();
        let settings = IndexSettings { sort_by_field: Some(IndexSortByField {
            field: "date".to_owned(),
            order: tantivy::Order::Asc,
        }), ..Default::default() };
        let index = Index::create(directory, schema.clone(), settings)?;
        Ok(Self {
            index,
            url_field,
            url_norm_field,
            title_field,
            date_field,
        })
    }

    /// Insert a list of scrapes into the index.
    fn insert_scrapes<'a, I: Iterator<Item = Scrape> + 'a>(&mut self, scrapes: I) -> Result<(), PersistError> {
        let mut writer = self.index.writer(MEMORY_ARENA_SIZE)?;
        let reader = self.index.reader()?;
        let searcher = reader.searcher();
        let parser = QueryParser::for_index(&self.index, vec![self.url_field]);

        let mut memindex = memindex::MemIndex::default();
        memindex.insert_scrapes(scrapes)?;

        for story in &memindex.get_all_stories().chunks(STORY_INDEXING_CHUNK_SIZE) {
            println!("Chunk");
            for story in story {
                if let (Some(start), Some(end)) = (story.date().checked_sub_months(Months::new(1)), story.date().checked_add_months(Months::new(1))) {
                    let url_query = Box::new(TermQuery::new(Term::from_field_text(self.url_norm_field, &story.normalized_url), IndexRecordOption::Basic));
                    let date_range_query = Box::new(RangeQuery::new_i64(self.date_field, start.timestamp()..end.timestamp()));
                    let query = BooleanQuery::new(vec![
                        (Occur::Must, url_query),
                        (Occur::Must, date_range_query),
                    ]);
                    let docs = searcher.search(&query, &TopDocs::with_limit(1))?;
                    if let Some(doc) = docs.first() {
                        // Need to update
                        let doc = searcher.doc(doc.1)?;
                        let url = doc.get_first(self.url_field);
                        if let Some(url) = url.and_then(|x| x.as_text()) {
                            println!("Update: {} {}", url, story.url());
                        }
                    } else {
                        // Insert new
                        writer.add_document(doc! {
                            self.url_field => story.url(),
                            self.url_norm_field => story.normalized_url.clone(),
                            self.title_field => story.title(),
                            self.date_field => story.date().timestamp(),
                        })?;
                    }
                }
            }
        }
        writer.commit()?;
        reader.reload()?;

        Ok(())
    }
}

impl Storage for StoryIndex {
    fn insert_scrapes<'a, I: Iterator<Item = Scrape> + 'a>(&mut self, scrapes: I) -> Result<(), PersistError> {
        self.insert_scrapes(scrapes)
    }

    fn query_frontpage(&self, max_count: usize) -> Result<Vec<Story>, PersistError> {
        unimplemented!()
    }

    fn query_search(&self, search: String, max_count: usize) -> Result<Vec<Story>, PersistError> {
        let query = TermQuery::new(Term::from_field_text(self.title_field, &search), IndexRecordOption::Basic);
        let searcher = self.index.reader()?.searcher();
        let docs = searcher.search(&query, &TopDocs::with_limit(max_count))?;
        let mut vec = vec![];
        for doc in docs {
            let doc = searcher.doc(doc.1)?;
            println!("{}", doc.get_first(self.title_field).and_then(|x| x.as_text()).unwrap_or_default());
        }
        Ok(vec)
    }
}

#[cfg(test)]
mod test {
    use tantivy::directory::RamDirectory;

    use super::*;

    #[test]
    fn test_index_lots() {
        let stories = crate::scrapers::legacy_import::import_legacy().expect("Failed to read scrapes");
        // let stories = crate::scrapers::test::scrape_all();
        // let dir = MmapDirectory::open("/tmp/index").expect("Failed to get mmap dir");
        let dir = RamDirectory::create();
        let mut index = StoryIndex::initialize(dir).expect("Failed to initialize index");
        index.insert_scrapes(stories.into_iter()).expect("Failed to insert scrapes");
        index.query_search("rust".to_owned(), 10);
    }
}
