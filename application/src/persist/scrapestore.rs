use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, RwLock},
};

use progscrape_scrapers::{ScrapeId, StoryDate, TypedScrape};
use serde::{Deserialize, Serialize};

use crate::PersistError;

use super::{db::DB, PersistLocation};

/// Long-term persistence for raw scrape data.
pub struct ScrapeStore {
    location: PersistLocation,
    shards: RwLock<HashMap<String, Arc<DB>>>,
}

#[derive(Default, Serialize, Deserialize)]
struct ScrapeCacheEntry {
    date: StoryDate,
    id: String,
    json: String,
}

impl ScrapeStore {
    pub fn new(location: PersistLocation) -> Result<Self, PersistError> {
        tracing::info!("Initialized ScrapeStore at {:?}", location);
        Ok(Self {
            location,
            shards: RwLock::new(HashMap::new()),
        })
    }

    fn open_shard<'a>(&'a self, shard: String) -> Result<Arc<DB>, PersistError> {
        let mut lock = self.shards.write().expect("Poisoned");
        let db = if let Some(db) = lock.get(&shard) {
            db
        } else {
            let db = match self.location.join(&shard) {
                PersistLocation::Memory => DB::open(":memory:")?,
                PersistLocation::Path(ref path) => {
                    std::fs::create_dir_all(&path)?;
                    let path = path.join("scrapes.sqlite3");
                    tracing::info!("Opening scrape database at {}", path.to_string_lossy());
                    let db = DB::open(path)?;
                    // Force each DB into WAL mode
                    db.execute_raw("PRAGMA journal_mode = WAL")?;
                    db
                }
            };
            lock.entry(shard).or_insert(Arc::new(db))
        };
        db.create_table::<ScrapeCacheEntry>()?;
        db.create_unique_index::<ScrapeCacheEntry>("idx_id", &["id"])?;
        Ok(db.clone())
    }

    pub fn insert_scrape(&self, scrape: &TypedScrape) -> Result<(), PersistError> {
        self.insert_scrape_batch([scrape].into_iter())
    }

    pub fn insert_scrape_batch<'a, I: Iterator<Item = &'a TypedScrape>>(
        &self,
        iter: I,
    ) -> Result<(), PersistError> {
        let mut per_shard: HashMap<String, Vec<&TypedScrape>> = HashMap::new();
        for item in iter {
            let shard = format!("{:04}-{:02}", item.date.year(), item.date.month());
            per_shard.entry(shard).or_default().push(item);
        }
        for (shard, stories) in per_shard {
            let db = self.open_shard(shard)?;
            let mut batch = vec![];
            for item in stories {
                let json = serde_json::to_string(item)?;
                batch.push(ScrapeCacheEntry {
                    date: item.date,
                    id: item.id.to_string(),
                    json,
                });
            }
            db.store_batch(batch)?;
        }
        Ok(())
    }

    pub fn fetch_scrape(
        &self,
        id: &ScrapeId,
        date: StoryDate,
    ) -> Result<Option<TypedScrape>, PersistError> {
        let shard = format!("{:04}-{:02}", date.year(), date.month());
        let db = self.open_shard(shard)?;
        let scrape = db.load::<ScrapeCacheEntry>(id.to_string())?;
        if let Some(scrape) = scrape {
            let typed_scrape = serde_json::from_str(&scrape.json)?;
            Ok(Some(typed_scrape))
        } else {
            Ok(None)
        }
    }

    pub fn fetch_scrape_batch<'a, I: Iterator<Item = (&'a ScrapeId, StoryDate)>>(
        &self,
        iter: I,
    ) -> Result<HashMap<ScrapeId, Option<TypedScrape>>, PersistError> {
        let mut map = HashMap::new();
        for (id, date) in iter {
            let shard = format!("{:04}-{:02}", date.year(), date.month());
            let db = self.open_shard(shard)?;
            let scrape = db.load::<ScrapeCacheEntry>(id.to_string())?;
            if let Some(scrape) = scrape {
                let typed_scrape = serde_json::from_str(&scrape.json)?;
                map.insert(id.clone(), typed_scrape);
            } else {
                map.insert(id.clone(), None);
            }
        }
        Ok(map)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_insert() -> Result<(), Box<dyn std::error::Error>> {
        let mut store = ScrapeStore::new(PersistLocation::Memory)?;
        let legacy = progscrape_scrapers::import_legacy(Path::new(".."))?;
        let first = &legacy[0..100];
        for scrape in first {
            store.insert_scrape(scrape)?;
        }
        for scrape in first {
            let loaded_scrape = store.fetch_scrape(&scrape.id, scrape.date)?.unwrap();
            assert_eq!(scrape.id, loaded_scrape.id);
        }
        Ok(())
    }
}
