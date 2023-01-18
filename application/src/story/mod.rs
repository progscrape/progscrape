use itertools::Itertools;
use serde::{Deserialize, Serialize};

use progscrape_scrapers::{
    ScrapeConfig, ScrapeExtractor, ScrapeId, ScrapeSource, StoryDate, StoryUrl, TypedScrape,
};
use std::{
    borrow::Cow,
    collections::{hash_map::Entry, HashMap, HashSet},
};

mod id;
mod scorer;
mod tagger;

use self::scorer::StoryScoreType;
pub use self::{
    id::StoryIdentifier,
    scorer::{StoryScoreConfig, StoryScorer},
    tagger::{StoryTagger, TaggerConfig},
};

/// Rendered story with all properties hydrated from the underlying scrapes. Extraneous data is removed at this point.
#[derive(Clone, Default, Deserialize, Serialize)]
pub struct StoryRender {
    /// Natural story order in its container list.
    pub order: usize,
    pub id: String,
    pub url: String,
    pub url_norm: String,
    pub url_norm_hash: i64,
    pub domain: String,
    pub title: String,
    pub date: StoryDate,
    pub score: f32,
    pub tags: Vec<String>,
    pub comment_links: HashMap<String, String>,
    pub scrapes: HashMap<ScrapeId, TypedScrape>,
}

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
            &TaggerConfig::default(),
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
    pub scrapes: HashMap<ScrapeId, TypedScrape>,
}

impl Story {
    pub fn new(eval: &StoryEvaluator, scrape: TypedScrape) -> Self {
        let scrape_core = eval.extractor.extract(&scrape);
        let id = StoryIdentifier::new(scrape_core.date, scrape_core.url.normalization());
        let scrape_id = scrape_core.source.clone();
        // This is a bit awkward as we should probably be scoring from the raw scrapes rather than the story itself
        let mut story = Self {
            id,
            tags: TagSet::new(),
            title: scrape_core.title.to_string(),
            url: scrape_core.url.clone(),
            date: scrape_core.date,
            score: 0.0,
            scrapes: HashMap::from_iter([(scrape_id, scrape)]),
        };
        story.score = eval.scorer.score(&story, StoryScoreType::Base);
        eval.tagger.tag(&story.title, &mut story.tags);
        story
    }

    pub fn merge(&mut self, eval: &StoryEvaluator, scrape: TypedScrape) {
        let scrape_core = eval.extractor.extract(&scrape);
        let scrape_id = scrape_core.source.clone();
        self.date = std::cmp::min(self.date, scrape_core.date);

        match self.scrapes.entry(scrape_id) {
            Entry::Occupied(mut x) => {
                x.get_mut().merge(scrape);
            }
            Entry::Vacant(x) => {
                x.insert(scrape);
            }
        }

        self.title = self.title_choice(&eval.extractor).1.to_string();

        // Re-score this story
        self.score = eval.scorer.score(self, StoryScoreType::Base);
        // The ID may change if the date changes
        self.id.update_date(self.date);
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

    /// Choose a title based on source priority, with preference for shorter titles if the priority is the same.
    fn title_choice(&self, extractor: &ScrapeExtractor) -> (ScrapeSource, Cow<str>) {
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
        let mut best_title = (99, &ScrapeSource::Other, Cow::Borrowed("Unknown title"));
        for (id, scrape) in &self.scrapes {
            let scrape = extractor.extract(scrape);
            let score = title_score(&id.source);
            if score < best_title.0 {
                best_title = (score, &id.source, scrape.title);
                continue;
            }
            let title = &scrape.title;
            if score == best_title.0 && title.len() < best_title.2.len() {
                best_title = (score, &id.source, scrape.title);
                continue;
            }
        }
        (*best_title.1, best_title.2)
    }

    pub fn render(&self, order: usize) -> StoryRender {
        let scrapes = HashMap::from_iter(self.scrapes.iter().map(|(k, v)| (k.clone(), v.clone())));
        let mut tags = vec![self.url.host().to_owned()];
        tags.extend(self.tags.dump());
        StoryRender {
            order,
            id: self.id.to_base64(),
            score: self.score,
            url: self.url.to_string(),
            url_norm: self.url.normalization().string().to_owned(),
            url_norm_hash: self.url.normalization().hash(),
            domain: self.url.host().to_owned(),
            title: self.title.to_owned(),
            date: self.date,
            tags,
            comment_links: HashMap::new(),
            scrapes,
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
