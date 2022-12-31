
use std::collections::HashMap;

use itertools::Itertools;
use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, TermQuery, QueryParser};
use tantivy::schema::*;
use tantivy::{doc, DocId, Index, Score, SegmentReader};
use url::Url;

use crate::datasci::urlnormalizer::url_normalization_string;
use crate::scrapers::Scrape;

use super::*;

const MEMORY_ARENA_SIZE: usize = 50_000_000;
const STORY_INDEXING_CHUNK_SIZE: usize = 100;

struct StoryIndex {
    index: Index,
    url_field: Field,
    url_norm_field: Field,
    title_field: Field,
}

impl StoryIndex {
    pub fn initialize() -> Self {
        let mut schema_builder = Schema::builder();
        let url_field = schema_builder.add_text_field("url", STRING | STORED);  
        let url_norm_field = schema_builder.add_text_field("url_norm", STRING | STORED);  
        let title_field = schema_builder.add_text_field("title", TEXT | STORED);  
        let schema = schema_builder.build();
        let index = Index::create_in_ram(schema.clone());
        Self {
            index,
            url_field,
            url_norm_field,
            title_field
        }
    }

    /// Insert a list of scrapes into the index.
    pub fn insert_scrapes<'a, IT: AsRef<dyn Scrape>, I: Iterator<Item = IT> + 'a>(&mut self, scrape: I) -> Result<(), PersistError> {
        let mut writer = self.index.writer(MEMORY_ARENA_SIZE)?;
        let reader = self.index.reader()?;
        let parser = QueryParser::for_index(&self.index, vec![self.url_field]);

        for chunk in &scrape.chunks(STORY_INDEXING_CHUNK_SIZE) {
            let mut batch = HashMap::new();
            let mut searcher = reader.searcher();
            for item in scrape {
                let scrape = item.as_ref();
                if let Ok(url) = Url::parse(&scrape.url()) {
                    let url_normalized = url_normalization_string(&url);
    
                    let term = TermQuery::new(Term::from_field_text(self.url_norm_field, &url_normalized), IndexRecordOption::Basic);
                    let docs2 = searcher.search(&term, &TopDocs::with_limit(1))?;
    x
                    writer.add_document(doc! {
                        self.url_field => scrape.url(),
                        self.url_norm_field => url_normalized,
                        self.title_field => scrape.title(),
                    })?;
                }
            }
            writer.commit()?;
            reader.reload()?;
        }

        Ok(())
    }
}

impl Storage for StoryIndex {
    fn insert_scrapes<'a, IT: AsRef<dyn Scrape>, I: Iterator<Item = IT> + 'a>(&mut self, scrape: I) -> Result<(), PersistError> {
        self.insert_scrapes(scrape)
    }

    fn query_frontpage(&self, max_count: usize) -> Result<Vec<Story>, PersistError> {
        unimplemented!()
    }

    fn query_search(&self, search: String, max_count: usize) -> Result<Vec<Story>, PersistError> {
        unimplemented!()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_index_lots() {
        let stories = crate::scrapers::test::scrape_all();
        let mut index = StoryIndex::initialize();
        index.insert_scrapes(stories.into_iter()).expect("Failed to insert scrapes");

    }
}

