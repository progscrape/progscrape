use std::collections::HashMap;

use itertools::Itertools;

use progscrape_scrapers::{ScrapeCollection, StoryUrlNorm};

use super::{shard::Shard, *};

/// Builds an index of stories in memory, useful for pre-aggregation of scrapes into normalized URL collections.
#[derive(Default, Serialize, Deserialize)]
pub struct MemIndex {
    /// A map from year/month to normalized story URL, to scrape source/ID to scrape.
    stories: HashMap<Shard, HashMap<StoryUrlNorm, ScrapeCollection>>,
}

impl MemIndex {
    pub fn get_all_stories(self) -> impl DoubleEndedIterator<Item = ScrapeCollection> {
        let mut out = vec![];
        for (_shard, stories) in self.stories.into_iter().sorted_by_cached_key(|f| f.0) {
            for (_, story) in stories {
                out.push(story);
            }
        }
        out.into_iter()
    }

    fn map_mut(&mut self, shard: Shard) -> &mut HashMap<StoryUrlNorm, ScrapeCollection> {
        self.stories.entry(shard).or_default()
    }

    fn map(&self, shard: &Shard) -> Option<&HashMap<StoryUrlNorm, ScrapeCollection>> {
        self.stories.get(shard)
    }

    pub fn insert_scrapes<I: IntoIterator<Item = TypedScrape>>(
        &mut self,
        scrapes: I,
    ) -> Result<(), PersistError> {
        'outer: for scrape in scrapes {
            let date = Shard::from_date_time(scrape.date);
            let normalized_url = scrape.url.normalization();
            // Try to pin it to an existing item
            for n in -2..=2 {
                let map0 = self.map_mut(date.plus_months(n));
                if let Some((key, mut scrapes)) = map0.remove_entry(normalized_url) {
                    // Merge and then re-insert the story in the correct shard
                    scrapes.merge(scrape);
                    self.map_mut(Shard::from_date_time(scrapes.earliest))
                        .insert(key, scrapes);
                    continue 'outer;
                }
            }

            // Not found!
            if let Some(_old) = self.map_mut(date).insert(
                normalized_url.clone(),
                ScrapeCollection::new_from_one(scrape),
            ) {
                // TODO: We need to merge duplicate scrapes
                println!("Unexpected");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use progscrape_scrapers::ScrapeConfig;

    use super::*;

    #[test]
    fn test_year_month() {
        let date = Shard::from_year_month(2000, 12);
        assert_eq!(Shard::from_year_month(2001, 1), date.plus_months(1));
        assert_eq!(Shard::from_year_month(2001, 12), date.plus_months(12));
        assert_eq!(Shard::from_year_month(1999, 12), date.sub_months(12));
        assert_eq!(Shard::from_year_month(2000, 1), date.sub_months(11));

        assert_eq!(
            date,
            Shard::from_string(&date.to_string()).expect("Failed to parse")
        );
    }

    #[test]
    fn test_index_lots() {
        let stories = progscrape_scrapers::load_sample_scrapes(&ScrapeConfig::default());
        let mut index = MemIndex::default();

        let _eval = StoryEvaluator::new_for_test();
        index
            .insert_scrapes(stories)
            .expect("Failed to insert scrapes");
    }
}
