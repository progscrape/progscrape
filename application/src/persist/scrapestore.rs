use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use progscrape_scrapers::{ScrapeId, StoryDate, TypedScrape};
use serde::{Deserialize, Serialize};

use crate::{story::StoryScrapeId, PersistError};

use super::{db::DB, shard::Shard, PersistLocation};

pub const SCRAPE_STORE_VERSION: usize = 1;

/// Long-term persistence for raw scrape data.
pub struct ScrapeStore {
    location: PersistLocation,
    shards: RwLock<HashMap<Shard, Arc<DB>>>,
}

/// Summary information for a given scrape store, useful for debugging and determining if a scrape store has been modified.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrapeStoreStats {
    /// The backup version, defaulting to zero
    #[serde(default)]
    pub version: usize,

    pub earliest: StoryDate,
    pub latest: StoryDate,
    pub count: usize,
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

    fn open_shard<'a>(&'a self, shard: Shard) -> Result<Arc<DB>, PersistError> {
        let mut lock = self.shards.write().expect("Poisoned");
        let db = if let Some(db) = lock.get(&shard) {
            db
        } else {
            let db = match self.location.join(shard.to_string()) {
                PersistLocation::Memory => DB::open(":memory:")?,
                PersistLocation::Path(ref path) => {
                    std::fs::create_dir_all(path)?;
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
        self.insert_scrape_batch([scrape])
    }

    pub fn insert_scrape_batch<'a, I: IntoIterator<Item = &'a TypedScrape>>(
        &self,
        iter: I,
    ) -> Result<(), PersistError> {
        let mut per_shard: HashMap<Shard, Vec<&TypedScrape>> = HashMap::new();
        for item in iter {
            let shard = Shard::from_date_time(item.date);
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
        shard: Shard,
        id: &ScrapeId,
    ) -> Result<Option<TypedScrape>, PersistError> {
        let db = self.open_shard(shard)?;
        let scrape = db.load::<ScrapeCacheEntry>(id.to_string())?;
        if let Some(scrape) = scrape {
            let typed_scrape = serde_json::from_str(&scrape.json)?;
            Ok(Some(typed_scrape))
        } else {
            Ok(None)
        }
    }

    pub fn fetch_scrape_batch<'a, I: IntoIterator<Item = StoryScrapeId>>(
        &self,
        iter: I,
    ) -> Result<HashMap<ScrapeId, Option<TypedScrape>>, PersistError> {
        let mut map = HashMap::new();
        for id in iter {
            let db = self.open_shard(id.shard)?;
            let scrape = db.load::<ScrapeCacheEntry>(id.id.to_string())?;
            if let Some(scrape) = scrape {
                let typed_scrape = serde_json::from_str(&scrape.json)?;
                map.insert(id.id.clone(), typed_scrape);
            } else {
                map.insert(id.id.clone(), None);
            }
        }
        Ok(map)
    }

    /// Fetch all the scrapes, passing them to a given callback (or the error to an error callback).
    pub fn fetch_all<F: FnMut(TypedScrape) -> Result<(), PersistError>, FE: FnMut(PersistError)>(
        &self,
        shard: Shard,
        mut f: F,
        mut fe: FE,
    ) -> Result<(), PersistError> {
        let db = self.open_shard(shard)?;
        let sql = format!(
            "select * from {} order by date, id",
            DB::table_for::<ScrapeCacheEntry>()
        );
        db.query_raw_callback(&sql, |scrape: ScrapeCacheEntry| {
            match serde_json::from_str(&scrape.json) {
                Ok(typed_scrape) => f(typed_scrape)?,
                Err(e) => fe(e.into()),
            }
            Ok(())
        })?;
        Ok(())
    }

    /// Get the stats for a given shard.
    pub fn stats(&self, shard: Shard) -> Result<ScrapeStoreStats, PersistError> {
        let db = self.open_shard(shard)?;
        // Fetch the stats object from a virtual view of that table
        let sql = format!(
            "select {} version, count(*) count, coalesce(min(date), 0) as earliest, coalesce(max(date), 0) as latest from {}",
            SCRAPE_STORE_VERSION,
            DB::table_for::<ScrapeCacheEntry>()
        );
        if let Some(stats) = db.query_raw::<ScrapeStoreStats>(&sql)?.into_iter().next() {
            Ok(stats)
        } else {
            Err(PersistError::UnexpectedError(
                "Failed to fetch single row for query".into(),
            ))
        }
    }
}

#[cfg(test)]
mod test {
    use progscrape_scrapers::ScrapeConfig;
    use rstest::rstest;

    use crate::test::enable_tracing;

    use super::*;

    #[rstest]
    fn test_insert(_enable_tracing: &bool) -> Result<(), Box<dyn std::error::Error>> {
        let store = ScrapeStore::new(PersistLocation::Memory)?;

        let samples = progscrape_scrapers::load_sample_scrapes(&ScrapeConfig::default());
        let first = &samples[0..100];

        // No items
        let stats = store.stats(Shard::from_date_time(first[0].date))?;
        assert_eq!(stats.count, 0);

        for scrape in first {
            store.insert_scrape(scrape)?;
        }
        for scrape in first {
            let loaded_scrape = store
                .fetch_scrape(Shard::from_date_time(scrape.date), &scrape.id)?
                .unwrap();
            assert_eq!(scrape.id, loaded_scrape.id);
        }

        // At least one item
        let stats = store.stats(Shard::from_date_time(first[0].date))?;
        assert!(stats.count >= 1);

        Ok(())
    }
}
