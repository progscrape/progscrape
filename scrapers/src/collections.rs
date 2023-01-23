use std::{
    collections::{hash_map::Entry, HashMap},
};

use serde::{Deserialize, Serialize};

use crate::{backends::ScrapeCore, ScrapeExtractor, ScrapeId, StoryDate, StoryUrl, TypedScrape};

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

    pub fn merge(&mut self, scrape: TypedScrape) {
        match self.scrapes.entry(scrape.id.clone()) {
            Entry::Occupied(mut x) => {
                x.get_mut().merge(scrape);
            }
            Entry::Vacant(x) => {
                x.insert(scrape);
            }
        }
    }

    pub fn extract<'a>(&'a self, extractor: &ScrapeExtractor) -> ExtractedScrapeCollection<'a> {
        let iter = self.scrapes.iter().map(|(k, v)| (k, extractor.extract(v)));
        ExtractedScrapeCollection {
            scrapes: HashMap::from_iter(iter),
        }
    }
}

/// Collection of scrape data that has been extracted from a `ScrapeCollection`.
pub struct ExtractedScrapeCollection<'a> {
    pub scrapes: HashMap<&'a ScrapeId, ScrapeCore<'a>>,
}

impl<'a> ExtractedScrapeCollection<'a> {
    pub fn title(&'a self) -> &'a str {
        // TODO: Best title
        self
            .scrapes
            .iter()
            .next()
            .expect("Expected at least one scrape")
            .1
            .title
    }

    pub fn url(&'a self) -> &'a StoryUrl {
        self
            .scrapes
            .iter()
            .next()
            .expect("Expected at least one scrape")
            .1
            .url
    }

    pub fn tags<'b>(&'b self) -> Vec<String> {
        // TODO: Fill this in
        vec![]
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
