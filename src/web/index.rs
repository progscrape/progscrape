use std::sync::Arc;

use crate::persist::{Storage, StorageWriter, MemIndex, StoryIndex};

use super::WebError;

#[derive(Clone)]
pub struct Global {
    pub storage: Arc<dyn Storage>,
}

pub fn initialize_with_testing_data() -> Result<Global, WebError> {
    let stories = crate::scrapers::legacy_import::import_legacy().expect("Failed to read scrapes");
    let mut index = MemIndex::default();
    index.insert_scrapes(stories.into_iter()).expect("Failed to insert scrapes");
    Ok(Global { storage: Arc::new(index) })
}

pub fn initialize_with_persistence() -> Result<Global, WebError> {
    unimplemented!()
}
