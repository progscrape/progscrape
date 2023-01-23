use std::{
    fs::File,
    io::{BufReader, BufWriter},
    path::Path,
    sync::Arc,
};

use crate::config::Config;

use progscrape_application::{
    MemIndex, PersistLocation, Storage, StorageWriter, StoryEvaluator, StoryIndex,
};

use crate::web::WebError;

#[derive(Clone)]
pub struct Index {
    pub storage: Arc<dyn Storage>,
}

impl Index {
    pub fn initialize_with_persistence<P: AsRef<Path>>(path: P) -> Result<Index, WebError> {
        let index = StoryIndex::new(PersistLocation::Path(path.as_ref().to_owned()))?;
        Ok(Index {
            storage: Arc::new(index),
        })
    }
}
