
use std::collections::HashMap;

use chrono::{Datelike, DateTime, Utc};
use itertools::Itertools;
use url::Url;

use crate::{scrapers::{ScrapeData, ScrapeId}, datasci::urlnormalizer::url_normalization_string};

use super::*;

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct YearMonth(u16, u8);

impl YearMonth {
    fn from_date_time(date: DateTime<Utc>) -> Self {
        Self(date.year() as u16, date.month0() as u8)
    }

    fn plus_months(&self, months: i8) -> Self {
        let ordinal = self.0 as i32 * 12 + self.1 as i32 + months as i32;
        YearMonth((ordinal / 12) as u16, (ordinal % 12) as u8)
    }

    fn sub_months(&self, months: i8) -> Self {
        self.plus_months(-months)
    }
}

/// Builds an index of stories in memory, useful for pre-aggregation of scrapes into normalized URL collections.
#[derive(Default)]
struct MemIndex {
    /// A map from year/month to normalized story URL, to scrape source/ID to scrape.
    stories: HashMap<YearMonth, HashMap<String, HashMap<ScrapeId, Scrape>>>
}

impl MemIndex {
    pub fn get_all_stories(&self) -> impl Iterator<Item = Story> {
        let mut out = vec![];
        for (month, stories) in self.stories.iter().sorted_by_cached_key(|f| f.0) {
            for story in stories {
                out.push(Story {
                    scrapes: story.1.clone()
                });
            }
        }
        out.into_iter()
    }
}

impl Storage for MemIndex {
    fn insert_scrapes<'a, I: Iterator<Item = Scrape> + 'a>(&mut self, scrapes: I) -> Result<(), PersistError> {
        'outer:
        for scrape in scrapes {
            let id = scrape.id();
            let date = YearMonth::from_date_time(scrape.date());
            let url = Url::parse(&scrape.url())?;
            let title = scrape.title();
            let normalized_url = url_normalization_string(&url);
            let source = scrape.source();
            let key = ScrapeId::new(source, id);
            // Try to pin it to an existing item
            for n in -2..2 {
                let map0 = self.stories.entry(date.plus_months(n)).or_default();
                if let Some(map1) = map0.get_mut(&normalized_url) {
                    // This logic can be improved when try_insert is stabilized
                    // TODO: We need to merge duplicate scrapes
                    map1.insert(key, scrape);
                    continue 'outer;
                }
            }

            // Not found!
            if let Some(old) = self.stories.entry(date).or_default().entry(normalized_url).or_default().insert(key, scrape) {
                // TODO: We need to merge duplicate scrapes
                println!("Unexpected");
            }
        }
        Ok(())
    }

    fn query_frontpage(&self, max_count: usize) -> Result<Vec<Story>, PersistError> {
        unimplemented!()   
    }

    fn query_search(&self, search: String, max_count: usize) -> Result<Vec<Story>, PersistError> {
        unimplemented!()   
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_year_month() {
        let date = YearMonth(2000, 11);
        assert_eq!(YearMonth(2001, 0), date.plus_months(1));
        assert_eq!(YearMonth(2001, 11), date.plus_months(12));
        assert_eq!(YearMonth(1999, 11), date.sub_months(12));
        assert_eq!(YearMonth(2000, 0), date.sub_months(11));
    }

    #[test]
    fn test_index_lots() {
        let stories = crate::scrapers::legacy_import::import_legacy().expect("Failed to read scrapes");
        let mut index = MemIndex::default();
        index.insert_scrapes(stories.into_iter()).expect("Failed to insert scrapes");
    }
}
