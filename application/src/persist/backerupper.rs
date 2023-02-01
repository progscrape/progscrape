use std::{
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use progscrape_scrapers::StoryDate;
use serde::{Deserialize, Serialize};

use crate::{persist::scrapestore::ScrapeStoreStats, timer_end, timer_start, PersistError, Shard};

use super::{
    scrapestore::ScrapeStore,
    shard::{ShardOrder, ShardRange},
};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum BackupResult {
    Empty,
    NoChange,
    Success(usize),
}

pub struct BackerUpper {
    path: PathBuf,
}

impl BackerUpper {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_owned(),
        }
    }

    fn trace_error<E: core::fmt::Debug>(error: E) -> E {
        tracing::error!("Ignoring error in metadata read: {:?}", error);
        error
    }

    pub fn backup(
        &self,
        name: &str,
        shard: Shard,
        scrapes: &ScrapeStore,
    ) -> Result<BackupResult, PersistError> {
        let stats = scrapes.stats(shard)?;
        if stats.count == 0 {
            return Ok(BackupResult::Empty);
        }

        // Metadata read intentionally drops some errors - we'll intentionally do more work if it's corrupt
        let meta = self.path.join(format!("{}.meta.json", name));
        let meta_temp = self.path.join(format!(".{}.meta.json", name));
        if meta.exists() {
            if let Ok(file) = std::fs::File::open(&meta).map_err(Self::trace_error) {
                if let Ok(current_stats) = serde_json::from_reader(file).map_err(Self::trace_error)
                {
                    if stats == current_stats {
                        return Ok(BackupResult::NoChange);
                    }
                }
            }
        }

        let output = self.path.join(format!("{}.json", name));
        let temp = self.path.join(format!(".{}.temp", name));
        let file = std::fs::File::create(&temp)?;

        let time = timer_start!();

        // Write each scrape to the file, with a newline separating them
        let mut w = BufWriter::new(file);
        const NEWLINE: [u8; 1] = [b'\n'];
        let mut earliest = StoryDate::MAX;
        let mut latest = StoryDate::MIN;
        let mut count = 0;
        scrapes.fetch_all(
            shard,
            |scrape| {
                count += 1;
                earliest = earliest.min(scrape.date);
                latest = latest.max(scrape.date);
                w.write_all(serde_json::to_string(&scrape)?.as_bytes())?;
                w.write(&NEWLINE)?;
                Ok(())
            },
            |error| {
                tracing::error!("Error fetching scrape: {:?}", error);
            },
        )?;

        let computed_stats = ScrapeStoreStats {
            count,
            earliest,
            latest,
        };

        if computed_stats != stats {
            tracing::info!(
                "Scrape store stats changed during backup: was {:?}, computed {:?}",
                stats,
                computed_stats
            );
        }

        // Note that we write our computed stats, not the ones we used for checking backup freshness!
        serde_json::to_writer(std::fs::File::create(&meta_temp)?, &computed_stats)?;

        // Atomic rename from temp to output for data files and meta
        std::fs::rename(temp, &output)?;
        std::fs::rename(meta_temp, meta)?;

        timer_end!(
            time,
            "Successfully backed up {} stories to {}",
            count,
            output.to_string_lossy()
        );

        Ok(BackupResult::Success(count))
    }

    pub fn backup_range(
        &self,
        scrapes: &ScrapeStore,
        shard_range: ShardRange,
    ) -> Vec<(Shard, Result<BackupResult, PersistError>)> {
        let mut v = vec![];
        for shard in shard_range.iterate(ShardOrder::OldestFirst) {
            v.push((shard, self.backup(&shard.to_string(), shard, scrapes)))
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test::enable_tracing;
    use crate::PersistLocation;
    use rstest::*;

    #[ignore]
    #[rstest]
    fn test_insert(_enable_tracing: &bool) -> Result<(), Box<dyn std::error::Error>> {
        let store = ScrapeStore::new(PersistLocation::Memory)?;

        let legacy = progscrape_scrapers::import_legacy(Path::new(".."))?;
        let first = &legacy[0..100];

        for scrape in first {
            store.insert_scrape(scrape)?;
        }

        let backup = BackerUpper::new("/tmp/backup");
        backup.backup("2015-01", Shard::from_year_month(2015, 1), &store)?;

        Ok(())
    }
}
