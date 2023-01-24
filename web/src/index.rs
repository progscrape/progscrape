use std::{path::Path, sync::Arc};

use progscrape_application::{PersistLocation, StorageWriter, StoryIndex};
use tokio::sync::RwLock;

use crate::web::WebError;

pub struct Index<S: StorageWriter> {
    pub storage: Arc<RwLock<S>>,
}

impl<S: StorageWriter> Clone for Index<S> {
    fn clone(&self) -> Self {
        Self {
            storage: self.storage.clone(),
        }
    }
}

impl Index<StoryIndex> {
    pub fn initialize_with_persistence<P: AsRef<Path>>(
        path: P,
    ) -> Result<Index<StoryIndex>, WebError> {
        let index = StoryIndex::new(PersistLocation::Path(path.as_ref().to_owned()))?;
        Ok(Index {
            storage: Arc::new(RwLock::new(index)),
        })
    }
}
