//! Stories begin as a `ScrapeCollection`, and we progressively analyze that collection to add further metdata,
//! including tags, scores, and post-processing of the provided titles.
use itertools::Itertools;
use serde::{Deserialize, Serialize};

use progscrape_scrapers::{
    ScrapeConfig, ScrapeExtractor, ScrapeId, ScrapeSource, StoryDate, StoryUrl, TypedScrape, ScrapeCollection,
};
use std::{
    borrow::Cow,
    collections::{hash_map::Entry, HashMap, HashSet},
};

mod id;
mod collector;
mod render;
mod scorer;
mod tagger;

use self::scorer::StoryScoreType;
pub use self::{
    id::StoryIdentifier,
    collector::StoryCollector,
    scorer::{StoryScoreConfig, StoryScorer},
    tagger::{StoryTagger, TaggerConfig},
    render::{StoryRender, StoryFullRender},
};

/// Required services to evaulate a story.
pub struct StoryEvaluator {
    pub tagger: StoryTagger,
    pub scorer: StoryScorer,
    pub extractor: ScrapeExtractor,
}

impl StoryEvaluator {
    pub fn new(tagger: &TaggerConfig, scorer: &StoryScoreConfig, scrape: &ScrapeConfig) -> Self {
        Self {
            tagger: StoryTagger::new(tagger),
            scorer: StoryScorer::new(scorer),
            extractor: ScrapeExtractor::new(scrape),
        }
    }

    #[cfg(test)]
    pub fn new_for_test() -> Self {
        Self::new(
            &crate::story::tagger::test::tagger_config(),
            &StoryScoreConfig::default(),
            &ScrapeConfig::default(),
        )
    }
}

/// Story scrape w/information from underlying sources.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Story {
    pub id: StoryIdentifier,
    pub score: f32,
    pub date: StoryDate,
    pub url: StoryUrl,
    pub title: String,
    pub tags: TagSet,
    pub scrapes: HashSet<ScrapeId>,
}

impl Story {
    pub fn new_from_parts(title: String, url: StoryUrl, date: StoryDate, score: f32, tags: Vec<String>, scrapes: HashSet<ScrapeId>) -> Self {
        Self {
            id: StoryIdentifier::new(date, url.normalization()),
            tags: TagSet::from_iter(tags.into_iter()),
            title,
            url,
            date,
            score,
            scrapes,
        }
    }

    /// Compares two stories, ordering by score.
    pub fn compare_score(&self, other: &Story) -> std::cmp::Ordering {
        // Sort by score, but fall back to date if score is somehow a NaN (it shouldn't be, but we'll just be robust here)
        f32::partial_cmp(&self.score, &other.score).unwrap_or_else(|| self.date.cmp(&other.date))
    }

    /// Compares two stories, ordering by date.
    pub fn compare_date(&self, other: &Story) -> std::cmp::Ordering {
        self.date.cmp(&other.date)
    }

    pub fn render(&self, order: usize) -> StoryRender {
        let mut tags = vec![self.url.host().to_owned()];
        tags.extend(self.tags.dump());
        StoryRender {
            order,
            id: self.id.to_base64(),
            score: self.score,
            url: self.url.to_string(),
            domain: self.url.host().to_owned(),
            title: self.title.to_owned(),
            date: self.date,
            tags,
            comment_links: HashMap::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TagSet {
    set: HashSet<String>,
}

impl TagSet {
    pub fn new() -> Self {
        Self {
            set: HashSet::new(),
        }
    }

    pub fn from_iter<S: AsRef<str>>(iter: impl Iterator<Item = S>) -> Self {
        Self {
            set: HashSet::from_iter(iter.map(|s| s.as_ref().to_owned()))
        }
    }

    pub fn contains(&self, tag: impl AsRef<str>) -> bool {
        self.set.contains(tag.as_ref())
    }

    pub fn add(&mut self, tag: impl AsRef<str>) {
        self.set.insert(tag.as_ref().to_ascii_lowercase());
    }

    pub fn collect(&self) -> Vec<String> {
        self.dump().collect()
    }

    pub fn dump<'a>(&'a self) -> impl Iterator<Item = String> + 'a {
        self.set.iter().sorted().cloned()
    }
}

impl TagAcceptor for TagSet {
    fn tag(&mut self, s: &str) {
        self.add(s);
    }
}

pub trait TagAcceptor {
    fn tag(&mut self, s: &str);
}
