use std::{
    fs::File,
    io::{BufReader, BufWriter},
    path::Path,
    sync::Arc,
};

use crate::config::Config;

use progscrape_application::{MemIndex, Storage, StorageWriter, StoryEvaluator, StoryIndex, PersistLocation};

use crate::web::WebError;

#[derive(Clone)]
pub struct Index {
    pub storage: Arc<dyn Storage>,
}

impl Index {
    pub fn initialize_with_testing_data(root: &Path, config: &Config) -> Result<Index, WebError> {
        let cache_file = "target/testing_data.bin";
        if let Ok(f) = File::open(cache_file) {
            tracing::info!("Reading cache '{}'...", cache_file);
            if let Ok(index) = serde_cbor::from_reader::<MemIndex, _>(BufReader::new(f)) {
                tracing::info!("Cache OK");
                return Ok(Index {
                    storage: Arc::new(index),
                });
            }
            tracing::info!("Cache not OK");
        }
        let _ = std::fs::remove_file(cache_file);

        // Filter to just 2017 for performance
        let scrapes = progscrape_scrapers::import_legacy(root).expect("Failed import");
        // scrapes.retain(|x| x.date.year() == 2017);

        let mut index = MemIndex::default();
        let eval = StoryEvaluator::new(&config.tagger, &config.score, &config.scrape);
        index.insert_scrapes(&eval, scrapes.into_iter())?;
        let f = File::create(cache_file)?;
        serde_cbor::to_writer(BufWriter::new(f), &index)?;
        Ok(Index {
            storage: Arc::new(index),
        })
    }

    pub fn initialize_with_persistence<P: AsRef<Path>>(path: P) -> Result<Index, WebError> {
        let index = StoryIndex::new(PersistLocation::Path(path.as_ref().to_owned()))?;
        Ok(Index {
            storage: Arc::new(index)
        })
    }
}
