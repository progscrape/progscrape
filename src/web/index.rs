use std::{
    fs::File,
    io::{BufReader, BufWriter},
    sync::Arc,
};

use crate::{
    persist::{MemIndex, Storage, StorageWriter},
    scrapers::ScrapeData, config::Config, story::StoryDate,
};

use super::WebError;

#[derive(Clone)]
pub struct Global {
    pub storage: Arc<dyn Storage>,
}

pub fn initialize_with_testing_data(config: &Config) -> Result<Global, WebError> {
    let cache_file = "target/testing_data.bin";
    if let Ok(f) = File::open(cache_file) {
        tracing::info!("Reading cache '{}'...", cache_file);
        if let Ok(index) = serde_cbor::from_reader::<MemIndex, _>(BufReader::new(f)) {
            tracing::info!("Cache OK");
            return Ok(Global {
                storage: Arc::new(index),
            });
        }
        tracing::info!("Cache not OK");
    }
    let _ = std::fs::remove_file(cache_file);

    // Filter to just 2017 for performance
    let mut scrapes = crate::scrapers::legacy_import::import_legacy().expect("Failed import");
    scrapes.retain(|x| x.date().year() == 2017);

    let mut index = MemIndex::default();
    index
        .insert_scrapes(&config.score, scrapes.into_iter())?;
    let f = File::create(cache_file)?;
    serde_cbor::to_writer(BufWriter::new(f), &index)?;
    Ok(Global {
        storage: Arc::new(index),
    })
}

pub fn initialize_with_persistence() -> Result<Global, WebError> {
    unimplemented!()
}
