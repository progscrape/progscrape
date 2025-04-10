//! Stories begin as a `ScrapeCollection`, and we progressively analyze that collection to add further metdata,
//! including tags, scores, and post-processing of the provided titles.
use itertools::Itertools;
use serde::{Deserialize, Serialize};

use progscrape_scrapers::{
    ScrapeConfig, ScrapeExtractor, ScrapeId, StoryDate, StoryUrl, TypedScrapeMap,
};
use std::collections::{HashMap, HashSet};

mod collector;
mod id;
mod render;
mod scorer;
mod tagger;

use crate::persist::Shard;

pub use self::{
    collector::StoryCollector,
    id::StoryIdentifier,
    render::StoryRender,
    scorer::{StoryScore, StoryScoreConfig, StoryScorer},
    tagger::{StoryTagger, TaggerConfig},
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

#[derive(Clone, Debug)]
pub struct StoryScrapeId {
    pub id: ScrapeId,
    pub shard: Shard,
}

impl From<StoryScrapeId> for (ScrapeId, Shard) {
    fn from(val: StoryScrapeId) -> Self {
        (val.id, val.shard)
    }
}

/// Story scrape w/information from underlying sources.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Story<S> {
    pub id: StoryIdentifier,
    pub score: f32,
    pub date: StoryDate,
    pub url: StoryUrl,
    pub title: String,
    pub tags: TagSet,
    pub scrapes: HashMap<ScrapeId, S>,
}

impl<S> Story<S> {
    pub fn new_from_parts(
        title: String,
        url: StoryUrl,
        date: StoryDate,
        score: f32,
        tags: impl IntoIterator<Item = String>,
        scrapes: impl IntoIterator<Item = impl Into<(ScrapeId, S)>>,
    ) -> Self {
        Self {
            id: StoryIdentifier::new(date, url.normalization()),
            tags: TagSet::from_iter(tags),
            title,
            url,
            date,
            score,
            scrapes: HashMap::from_iter(scrapes.into_iter().map(|x| x.into())),
        }
    }

    /// Compares two stories, ordering by score.
    pub fn compare_score(&self, other: &Self) -> std::cmp::Ordering {
        // Sort by score, but fall back to date if score is somehow a NaN (it shouldn't be, but we'll just be robust here)
        f32::partial_cmp(&self.score, &other.score).unwrap_or_else(|| self.date.cmp(&other.date))
    }

    /// Compares two stories, ordering by date.
    pub fn compare_date(&self, other: &Self) -> std::cmp::Ordering {
        self.date.cmp(&other.date)
    }

    /// Is this story likely a self-post? See the description of `ScrapeId::is_likely_self_post` for details of the
    /// heuristic and caveats.
    pub fn is_likely_self_post(&self) -> bool {
        for scrape_id in self.scrapes.keys() {
            if scrape_id.is_likely_self_post(&self.url) {
                return true;
            }
        }
        false
    }

    pub fn raw_tags(&self) -> Vec<String> {
        let mut tags = vec![self.url.host().to_owned()];
        tags.extend(self.tags.dump());
        tags
    }

    pub fn render_tags(&self, tagger: &StoryTagger) -> Vec<String> {
        let host = self.url.host();
        let mut tags = vec![];
        // This is mainly for our blog entries that explicitly use "progscrape" as a host
        if host.contains('.') {
            tags.push(host.to_owned());
        }
        tags.extend(tagger.make_display_tags(self.tags.dump()));
        tags
    }

    pub fn render(&self, eval: &StoryEvaluator, order: usize) -> StoryRender {
        let mut sources = TypedScrapeMap::new();
        for id in self.scrapes.keys() {
            sources.set(id.source, Some(id.clone()));
        }
        StoryRender {
            order,
            id: self.id.to_base64(),
            score: self.score,
            url: self.url.to_string(),
            domain: self.url.host().to_owned(),
            title: self.title.to_owned(),
            date: self.date,
            tags: self.render_tags(&eval.tagger),
            html: Default::default(),
            sources,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagSet {
    set: HashSet<String>,
}

impl TagSet {
    pub fn new() -> Self {
        Self {
            set: HashSet::new(),
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

    pub fn dump(&self) -> impl Iterator<Item = String> + '_ {
        self.set.iter().sorted().cloned()
    }

    pub fn iter(&self) -> impl Iterator<Item = &String> {
        self.set.iter()
    }
}

impl <T: AsRef<str>> FromIterator<T> for TagSet {
    fn from_iter<U: IntoIterator<Item = T>>(iter: U) -> Self {
        Self {
            set: HashSet::from_iter(iter.into_iter().map(|s| s.as_ref().to_owned())),
        }
    }
}

impl<'a> IntoIterator for &'a TagSet {
    type IntoIter = <&'a HashSet<String> as IntoIterator>::IntoIter;
    type Item = <&'a HashSet<String> as IntoIterator>::Item;

    fn into_iter(self) -> Self::IntoIter {
        self.set.iter()
    }
}

impl IntoIterator for TagSet {
    type IntoIter = <HashSet<String> as IntoIterator>::IntoIter;
    type Item = <HashSet<String> as IntoIterator>::Item;

    fn into_iter(self) -> Self::IntoIter {
        self.set.into_iter()
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
