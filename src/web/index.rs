use std::{
    fs::File,
    io::{BufReader, BufWriter},
    sync::Arc,
};

use crate::{
    persist::{MemIndex, Storage, StorageWriter},
    scrapers::ScrapeData,
};

use super::WebError;

#[derive(Clone)]
pub struct Global {
    pub storage: Arc<dyn Storage>,
}

pub fn initialize_with_testing_data() -> Result<Global, WebError> {
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
    let stories = crate::scrapers::legacy_import::import_legacy().expect("Failed to read scrapes");
    let mut index = MemIndex::default();
    index
        .insert_scrapes(stories.filter(|x| x.date().year() >= 2017).into_iter())
        .expect("Failed to insert scrapes");
    let f = File::create(cache_file)?;
    serde_cbor::to_writer(BufWriter::new(f), &index)?;
    Ok(Global {
        storage: Arc::new(index),
    })
}

pub fn initialize_with_persistence() -> Result<Global, WebError> {
    unimplemented!()
}
