use std::{
    borrow::{Borrow, Cow},
    collections::{hash_map::Entry, HashMap, HashSet},
};

use itertools::Itertools;
use serde::{Deserialize, Serialize};

use crate::{
    backends::ScrapeCore, ScrapeExtractor, ScrapeId, ScrapeSource, StoryDate, StoryUrl, TypedScrape,
};

/// Collection of scrapes, which can also extract the best title, etc.
#[derive(Serialize, Deserialize)]
pub struct ScrapeCollection {
    pub earliest: StoryDate,

    // TODO: We need to clone the scrape ID because we can't use a reference to the key, and making this a hash set
    // prevents mutation/
    pub scrapes: HashMap<ScrapeId, TypedScrape>,
}

impl ScrapeCollection {
    pub fn new_from_one(scrape: TypedScrape) -> Self {
        Self {
            earliest: scrape.date,
            scrapes: HashMap::from_iter([(scrape.id.clone(), scrape)]),
        }
    }

    pub fn new_from_iter(scrapes: impl Iterator<Item = TypedScrape>) -> Self {
        let scrapes = HashMap::from_iter(scrapes.map(|s| (s.id.clone(), s)));
        let earliest = scrapes
            .values()
            .map(|x| x.date)
            .min()
            .expect("Requires at least one TypedScrape");
        Self { earliest, scrapes }
    }

    /// Takes and merges another `TypedScrape`.
    pub fn merge(&mut self, scrape: TypedScrape) {
        self.earliest = self.earliest.min(scrape.date);
        match self.scrapes.entry(scrape.id.clone()) {
            Entry::Occupied(mut x) => {
                x.get_mut().merge(scrape);
            }
            Entry::Vacant(x) => {
                x.insert(scrape);
            }
        }
    }

    /// Takes and merges all the `TypedScrape`s from the other `ScrapeCollection`.
    pub fn merge_all(&mut self, other: Self) {
        for scrape in other.scrapes.into_values() {
            self.merge(scrape)
        }
    }

    pub fn url(&self) -> &StoryUrl {
        &self
            .scrapes
            .values()
            .next()
            .expect("Requires at least one TypedScrape")
            .url
    }

    pub fn extract<'a>(&'a self, extractor: &ScrapeExtractor) -> ExtractedScrapeCollection<'a> {
        let title_score = |source: &ScrapeSource| {
            match source {
                // HN is moderated and titles are high quality
                ScrapeSource::HackerNews => 0,
                ScrapeSource::Lobsters => 1,
                ScrapeSource::Slashdot => 2,
                // User-submitted titles are generally just OK
                ScrapeSource::Reddit => 3,
                ScrapeSource::Other => 99,
            }
        };

        let iter = self
            .scrapes
            .iter()
            .map(|(k, v)| (k, (extractor.extract(v), v)));
        let scrapes = HashMap::from_iter(iter);
        let mut title_story = *scrapes
            .iter()
            .next()
            .expect("Expected at least one scrape")
            .0;
        let mut max_title_score = i32::MAX;
        for (id, (_, _)) in &scrapes {
            let this_score = title_score(&id.source);
            if this_score < max_title_score {
                max_title_score = this_score;
                title_story = *id;
            }
        }

        ExtractedScrapeCollection {
            earliest: self.earliest,
            title_story,
            scrapes,
        }
    }
}

/// Collection of scrape data that has been extracted from a `ScrapeCollection`.
pub struct ExtractedScrapeCollection<'a> {
    pub earliest: StoryDate,
    title_story: &'a ScrapeId,
    pub scrapes: HashMap<&'a ScrapeId, (ScrapeCore<'a>, &'a TypedScrape)>,
}

impl<'a> ExtractedScrapeCollection<'a> {
    pub fn title(&'a self) -> &'a str {
        &self
            .scrapes
            .get(self.title_story)
            .expect("Expected the title story to be in the scrape collection")
            .0
            .title
    }

    pub fn url(&'a self) -> &'a StoryUrl {
        self.scrapes
            .iter()
            .next()
            .expect("Expected at least one scrape")
            .1
             .0
            .url
    }

    pub fn tags<'b>(&'b self) -> Vec<Cow<'a, str>> {
        let mut tags = HashSet::new();
        for (_, (scrape, _)) in &self.scrapes {
            tags.extend(&scrape.tags);
        }
        tags.into_iter().cloned().collect_vec()
    }
    // /// Choose a title based on source priority, with preference for shorter titles if the priority is the same.
    // fn title_choice(&self) -> (ScrapeSource, Cow<str>) {
    //     let title_score = |source: &ScrapeSource| {
    //         match source {
    //             // HN is moderated and titles are high quality
    //             ScrapeSource::HackerNews => 0,
    //             ScrapeSource::Lobsters => 1,
    //             ScrapeSource::Slashdot => 2,
    //             // User-submitted titles are generally just OK
    //             ScrapeSource::Reddit => 3,
    //             ScrapeSource::Other => 99,
    //         }
    //     };
    //     let mut best_title = (99, &ScrapeSource::Other, Cow::Borrowed("Unknown title"));
    //     for (id, scrape) in &self.scrapes {
    //         let scrape = extractor.extract(scrape);
    //         let score = title_score(&id.source);
    //         if score < best_title.0 {
    //             best_title = (score, &id.source, scrape.title);
    //             continue;
    //         }
    //         let title = &scrape.title;
    //         if score == best_title.0 && title.len() < best_title.2.len() {
    //             best_title = (score, &id.source, scrape.title);
    //             continue;
    //         }
    //     }
    //     (*best_title.1, best_title.2)
    // }
}
