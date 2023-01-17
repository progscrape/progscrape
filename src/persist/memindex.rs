use std::collections::HashMap;

use itertools::Itertools;

use crate::story::{StoryDate, StoryUrlNorm, StoryEvaluator};

use super::*;

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
struct YearMonth(u16);

impl ToString for YearMonth {
    fn to_string(&self) -> String {
        format!("{:04}-{:02}", self.0 / 12, self.0 % 12 + 1)
    }
}

impl YearMonth {
    fn from_year_month(year: u16, month: u8) -> Self {
        assert!(month > 0);
        YearMonth(year * 12 + month as u16 - 1)
    }

    fn from_string(s: &str) -> Option<Self> {
        if let Some((a, b)) = s.split_once('-') {
            if let (Ok(a), Ok(b)) = (str::parse(a), str::parse(b)) {
                return Some(Self::from_year_month(a, b));
            }
        }
        None
    }

    fn from_date_time(date: StoryDate) -> Self {
        Self::from_year_month(date.year() as u16, date.month() as u8)
    }

    fn plus_months(&self, months: i8) -> Self {
        let ordinal = self.0 as i16 + months as i16;
        Self(ordinal as u16)
    }

    fn sub_months(&self, months: i8) -> Self {
        self.plus_months(-months)
    }
}

/// Builds an index of stories in memory, useful for pre-aggregation of scrapes into normalized URL collections.
#[derive(Default, Serialize, Deserialize)]
pub struct MemIndex {
    most_recent_story: StoryDate,

    /// A map from year/month to normalized story URL, to scrape source/ID to scrape.
    stories: HashMap<YearMonth, HashMap<StoryUrlNorm, Story>>,
}

impl MemIndex {
    pub fn get_all_stories(&self) -> impl DoubleEndedIterator<Item = Story> {
        let mut out = vec![];
        for (shard, stories) in self.stories.iter().sorted_by_cached_key(|f| f.0) {
            for (_, story) in stories {
                out.push(story.clone());
                assert_eq!(*shard, YearMonth::from_date_time(story.date));
            }
        }
        out.sort_by(|a, b| a.compare_date(b));
        out.into_iter()
    }

    fn map_mut(&mut self, shard: YearMonth) -> &mut HashMap<StoryUrlNorm, Story> {
        self.stories.entry(shard).or_default()
    }

    fn map(&self, shard: YearMonth) -> Option<&HashMap<StoryUrlNorm, Story>> {
        self.stories.get(&shard)
    }

    #[cfg(test)]
    fn ensure_consistency(&self) {
        for (shard, stories) in &self.stories {
            for (norm, story) in stories {
                assert_eq!(YearMonth::from_date_time(story.date), *shard);
                assert!(story.id.matches_date(story.date));
                self.get_story(&story.id).unwrap_or_else(|| {
                    panic!(
                        "Expected to find a story by its ID ({:?}), shard {}, norm '{:?}'",
                        &story.id,
                        shard.to_string(),
                        norm
                    )
                });
            }
        }
    }
}

impl StorageWriter for MemIndex {
    fn insert_scrapes<I: Iterator<Item = TypedScrape>>(
        &mut self,
        eval: &StoryEvaluator,
        scrapes: I,
    ) -> Result<(), PersistError> {
        'outer: for scrape in scrapes {
            let scrape_core = eval.extractor.extract(&scrape);
            let date = YearMonth::from_date_time(scrape_core.date);
            let normalized_url = scrape_core.url.normalization();
            // Try to pin it to an existing item
            for n in -2..=2 {
                let map0 = self.map_mut(date.plus_months(n));
                if let Some((key, mut story)) = map0.remove_entry(normalized_url) {
                    // Merge and then re-insert the story in the correct shard
                    story.merge(eval, scrape);
                    self.most_recent_story = self.most_recent_story.max(story.date);
                    self.map_mut(YearMonth::from_date_time(story.date))
                        .insert(key, story);
                    continue 'outer;
                }
            }

            self.most_recent_story = self.most_recent_story.max(scrape_core.date);
            // Not found!
            if let Some(_old) = self
                .map_mut(date)
                .insert(normalized_url.clone(), Story::new(eval, scrape))
            {
                // TODO: We need to merge duplicate scrapes
                println!("Unexpected");
            }
        }
        Ok(())
    }
}

impl Storage for MemIndex {
    fn most_recent_story(&self) -> StoryDate {
        self.most_recent_story
    }

    fn story_count(&self) -> Result<StorageSummary, PersistError> {
        let mut summary = StorageSummary::default();
        summary.by_shard = self
            .stories
            .iter()
            .sorted_by_cached_key(|f| f.0)
            .map(|f| (f.0.to_string(), f.1.len()))
            .filter(|f| f.1 > 0)
            .collect();
        summary.total = summary.by_shard.iter().map(|x| x.1).sum();
        Ok(summary)
    }

    fn get_story(&self, id: &StoryIdentifier) -> Option<Story> {
        let shard = YearMonth::from_year_month(id.year(), id.month());
        if let Some(map) = self.map(shard) {
            map.get(&id.norm).map(Clone::clone)
        } else {
            tracing::warn!("Shard {:?} not found for story {:?}", shard, id);
            None
        }
    }

    fn stories_by_shard(&self, shard: &str) -> Result<Vec<Story>, PersistError> {
        if let Some(shard) = YearMonth::from_string(shard) {
            if let Some(map) = self.map(shard) {
                Ok(map.values().cloned().collect_vec())
            } else {
                Ok(vec![])
            }
        } else {
            Ok(vec![])
        }
    }

    fn query_frontpage_hot_set(&self, max_count: usize) -> Result<Vec<Story>, PersistError> {
        const LIMIT: usize = 500;
        let rev = self.get_all_stories().rev();
        Ok(rev.take(max_count).collect::<Vec<_>>())
    }

    fn query_search(&self, _search: String, _max_count: usize) -> Result<Vec<Story>, PersistError> {
        unimplemented!()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_year_month() {
        let date = YearMonth::from_year_month(2000, 12);
        assert_eq!(YearMonth::from_year_month(2001, 1), date.plus_months(1));
        assert_eq!(YearMonth::from_year_month(2001, 12), date.plus_months(12));
        assert_eq!(YearMonth::from_year_month(1999, 12), date.sub_months(12));
        assert_eq!(YearMonth::from_year_month(2000, 1), date.sub_months(11));

        assert_eq!(
            date,
            YearMonth::from_string(&date.to_string()).expect("Failed to parse")
        );
    }

    #[test]
    fn test_index_lots() {
        let stories =
            crate::scrapers::legacy_import::import_legacy().expect("Failed to read scrapes");
        let mut index = MemIndex::default();

        let eval = StoryEvaluator::new_for_test();
        index
            .insert_scrapes(&eval, stories.into_iter())
            .expect("Failed to insert scrapes");
        index.ensure_consistency();
    }
}
