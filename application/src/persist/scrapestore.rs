use std::collections::HashMap;

use progscrape_scrapers::{TypedScrape, ScrapeId, StoryDate};
use serde::{Serialize, Deserialize};

use crate::PersistError;

use super::db::DB;

/// Long-term persistence for raw scrape data.
pub struct ScrapeStore {
    db: DB
}

#[derive(Default, Serialize, Deserialize)]
struct ScrapeCacheEntry {
    date: StoryDate,
    id: String,
    json: String,
}

impl ScrapeStore {
    pub fn new(location: &str) -> Result<Self, PersistError> {
        let db = DB::open(location)?;
        db.create_table::<ScrapeCacheEntry>()?;
        db.create_unique_index::<ScrapeCacheEntry>("idx_id", &["id"])?;
        Ok(Self {
            db
        })
    }

    pub fn insert_scrape(&mut self, scrape: &TypedScrape) -> Result<(), PersistError> {
        self.insert_scrape_batch([scrape].into_iter())
    }

    pub fn insert_scrape_batch<'a, I: Iterator<Item = &'a TypedScrape>>(&mut self, iter: I) -> Result<(), PersistError> {
        for item in iter {
            let json = serde_json::to_string(item)?;
            self.db.store(&ScrapeCacheEntry {
                date: item.date,
                id: item.id.to_string(),
                json
            })?;
        }
        Ok(())
    }

    pub fn fetch_scrape(&self, id: &ScrapeId) -> Result<Option<TypedScrape>, PersistError> {
        let scrape = self.db.load::<ScrapeCacheEntry>(id.to_string())?;
        if let Some(scrape) = scrape {
            let typed_scrape = serde_json::from_str(&scrape.json)?;
            Ok(Some(typed_scrape))
        } else {
            Ok(None)
        }
    }

    pub fn fetch_scrape_batch<'a, I: Iterator<Item = &'a ScrapeId>>(&self, iter: I) -> Result<HashMap<ScrapeId, Option<TypedScrape>>, PersistError> {
        let mut map = HashMap::new();
        for item in iter {
            let scrape = self.db.load::<ScrapeCacheEntry>(item.to_string())?;
            if let Some(scrape) = scrape {
                let typed_scrape = serde_json::from_str(&scrape.json)?;
                map.insert(item.clone(), typed_scrape);
            } else {
                map.insert(item.clone(), None);
            }
        }
        Ok(map)
    }
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use super::ScrapeStore;

    #[test]
    fn test_insert() -> Result<(), Box<dyn std::error::Error>> {
        let mut store = ScrapeStore::new(":memory:")?;
        let legacy = progscrape_scrapers::import_legacy(Path::new(".."))?;
        let first = &legacy[0..100];
        for scrape in first {
            store.insert_scrape(scrape)?;
        }
        for scrape in first {
            let loaded_scrape = store.fetch_scrape(&scrape.id)?.unwrap();
            assert_eq!(scrape.id, loaded_scrape.id);
        }
        Ok(())
    }
}
