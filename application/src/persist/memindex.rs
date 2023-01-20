use std::collections::HashMap;

use itertools::Itertools;

use progscrape_scrapers::{StoryDate, StoryUrlNorm};

use super::{*, shard::Shard};

/// Builds an index of stories in memory, useful for pre-aggregation of scrapes into normalized URL collections.
#[derive(Default, Serialize, Deserialize)]
pub struct MemIndex {
    most_recent_story: StoryDate,

    /// A map from year/month to normalized story URL, to scrape source/ID to scrape.
    stories: HashMap<Shard, HashMap<StoryUrlNorm, Story>>,
}

impl MemIndex {
    pub fn get_all_stories(&self) -> impl DoubleEndedIterator<Item = Story> {
        let mut out = vec![];
        for (shard, stories) in self.stories.iter().sorted_by_cached_key(|f| f.0) {
            for (_, story) in stories {
                out.push(story.clone());
                assert_eq!(*shard, Shard::from_date_time(story.date));
            }
        }
        out.sort_by(|a, b| a.compare_date(b));
        out.into_iter()
    }

    fn map_mut(&mut self, shard: Shard) -> &mut HashMap<StoryUrlNorm, Story> {
        self.stories.entry(shard).or_default()
    }

    fn map(&self, shard: &Shard) -> Option<&HashMap<StoryUrlNorm, Story>> {
        self.stories.get(shard)
    }

    #[cfg(test)]
    fn ensure_consistency(&self) {
        for (shard, stories) in &self.stories {
            for (norm, story) in stories {
                assert_eq!(Shard::from_date_time(story.date), *shard);
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
            let date = Shard::from_date_time(scrape_core.date);
            let normalized_url = scrape_core.url.normalization();
            // Try to pin it to an existing item
            for n in -2..=2 {
                let map0 = self.map_mut(date.plus_months(n));
                if let Some((key, mut story)) = map0.remove_entry(normalized_url) {
                    // Merge and then re-insert the story in the correct shard
                    story.merge(eval, scrape);
                    self.most_recent_story = self.most_recent_story.max(story.date);
                    self.map_mut(Shard::from_date_time(story.date))
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

    fn insert_stories<I: Iterator<Item = Story>>(
            &mut self,
            stories: I
        ) -> Result<(), PersistError> {
        unimplemented!()
    }
}

impl Storage for MemIndex {
    fn most_recent_story(&self) -> Result<StoryDate, PersistError> {
        Ok(self.most_recent_story)
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
        let shard = Shard::from_year_month(id.year(), id.month());
        if let Some(map) = self.map(&shard) {
            map.get(&id.norm).map(Clone::clone)
        } else {
            tracing::warn!("Shard {:?} not found for story {:?}", shard, id);
            None
        }
    }

    fn stories_by_shard(&self, shard: &str) -> Result<Vec<Story>, PersistError> {
        if let Some(shard) = Shard::from_string(shard) {
            if let Some(map) = self.map(&shard) {
                Ok(map.values().cloned().collect_vec())
            } else {
                Ok(vec![])
            }
        } else {
            Ok(vec![])
        }
    }

    fn query_frontpage_hot_set(&self, max_count: usize) -> Result<Vec<Story>, PersistError> {
        let rev = self.get_all_stories().rev();
        Ok(rev.take(max_count).collect::<Vec<_>>())
    }

    fn query_search(&self, search: &str, max_count: usize) -> Result<Vec<Story>, PersistError> {
        let mut v = vec![];
        let search = search.to_lowercase().trim().to_owned();
        'outer:
        for shard in self.stories.keys().sorted().rev() {
            if let Some(map) = self.map(shard) {
                for story in map.values().sorted_by_key(|s| s.date).rev() {
                    if story.tags.contains(&search) {
                        v.push(story.clone());
                    } else if story.title.to_lowercase().contains(&search) {
                        v.push(story.clone());
                    }
                    if v.len() > max_count {
                        break 'outer;
                    }
                }
            }
        }
        Ok(v)
    }
}

#[cfg(test)]
mod test {
    use std::path::Path;

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
        let stories =
            progscrape_scrapers::import_legacy(&Path::new("..")).expect("Failed to read scrapes");
        let mut index = MemIndex::default();

        let eval = StoryEvaluator::new_for_test();
        index
            .insert_scrapes(&eval, stories.into_iter())
            .expect("Failed to insert scrapes");
        index.ensure_consistency();
    }
}
