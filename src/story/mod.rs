use chrono::Duration;
use serde::{Deserialize, Serialize};

use crate::scrapers::{Scrape, ScrapeData, ScrapeDataInit, ScrapeId, ScrapeSource};
use std::{
    collections::{hash_map::Entry, HashMap},
    fmt::Display,
};

mod date;
mod url;

pub use self::{
    date::StoryDate,
    url::{StoryUrl, StoryUrlNorm},
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
    pub scrapes: HashMap<String, Scrape>,
}

/// Uniquely identifies a story within the index.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
pub struct StoryIdentifier {
    pub norm: StoryUrlNorm,
    date: (u16, u8, u8),
}

impl Display for StoryIdentifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "{}:{}:{}:{}",
            self.date.0,
            self.date.1,
            self.date.2,
            self.norm.string()
        ))
    }
}

impl StoryIdentifier {
    const BASE64_CONFIG: base64::engine::GeneralPurpose =
        base64::engine::general_purpose::URL_SAFE_NO_PAD;

    pub fn new(date: StoryDate, norm: &StoryUrlNorm) -> Self {
        Self {
            norm: norm.clone(),
            date: (date.year() as u16, date.month() as u8, date.day() as u8),
        }
    }

    pub fn update_date(&mut self, date: StoryDate) {
        self.date = (date.year() as u16, date.month() as u8, date.day() as u8);
    }

    pub fn matches_date(&self, date: StoryDate) -> bool {
        (self.date.0, self.date.1, self.date.2)
            == (date.year() as u16, date.month() as u8, date.day() as u8)
    }

    pub fn to_base64(&self) -> String {
        use base64::Engine;
        Self::BASE64_CONFIG.encode(self.to_string().as_bytes())
    }

    pub fn from_base64<T: AsRef<[u8]>>(s: T) -> Option<Self> {
        // Use an inner function so we can make use of ? (is there an easier way?)
        fn from_base64_res<T: AsRef<[u8]>>(s: T) -> Result<StoryIdentifier, ()> {
            use base64::Engine;
            let s = StoryIdentifier::BASE64_CONFIG.decode(s).map_err(drop)?;
            let s = String::from_utf8(s).map_err(drop)?;
            let mut bits = s.splitn(4, ':');
            let year = bits.next().ok_or(())?;
            let month = bits.next().ok_or(())?;
            let day = bits.next().ok_or(())?;
            let norm = bits.next().ok_or(())?.to_owned();
            Ok(StoryIdentifier {
                norm: StoryUrlNorm::from_string(norm),
                date: (
                    year.parse().map_err(drop)?,
                    month.parse().map_err(drop)?,
                    day.parse().map_err(drop)?,
                ),
            })
        }

        from_base64_res(s).ok()
    }

    pub fn year(&self) -> u16 {
        self.date.0
    }
    pub fn month(&self) -> u8 {
        self.date.1
    }
    pub fn day(&self) -> u8 {
        self.date.2
    }
}

/// Story scrape w/information from underlying sources.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Story {
    pub id: StoryIdentifier,
    pub score: f32,
    pub scrapes: HashMap<ScrapeId, Scrape>,
}

impl Story {
    pub fn new(score_config: &StoryScoreConfig, scrape: Scrape) -> Self {
        let id = StoryIdentifier::new(scrape.date(), scrape.url().normalization());
        let scrape_id = scrape.source();
        // This is a bit awkward as we should probably be scoring from the raw scrapes rather than the story itself
        let mut story = Self {
            id,
            score: 0.0,
            scrapes: HashMap::from_iter([(scrape_id, scrape)]),
        };
        story.score = StoryScorer::score(score_config, &story, StoryScoreType::Base);
        story
    }

    pub fn merge(&mut self, score_config: &StoryScoreConfig, scrape: Scrape) {
        let scrape_id = scrape.source();
        match self.scrapes.entry(scrape_id) {
            Entry::Occupied(mut x) => {
                Self::merge_scrape(x.get_mut(), scrape);
            }
            Entry::Vacant(x) => {
                x.insert(scrape);
            }
        }
        // Re-score this story
        self.score = StoryScorer::score(score_config, self, StoryScoreType::Base);
        // The ID may change if the date changes
        self.id.update_date(self.date());
    }

    fn merge_scrape(a: &mut Scrape, b: Scrape) {
        use Scrape::*;

        match (a, b) {
            (HackerNews(a), HackerNews(b)) => a.merge(b),
            (Reddit(a), Reddit(b)) => a.merge(b),
            (Lobsters(a), Lobsters(b)) => a.merge(b),
            (Slashdot(a), Slashdot(b)) => a.merge(b),
            (a, b) => {
                tracing::warn!(
                    "Unable to merge incompatible scrapes {:?} and {:?}, ignoring",
                    a.source(),
                    b.source()
                );
            }
        }
    }

    /// Compares two stories, ordering by score.
    pub fn compare_score(&self, other: &Story) -> std::cmp::Ordering {
        // Sort by score, but fall back to date if score is somehow a NaN (it shouldn't be, but we'll just be robust here)
        f32::partial_cmp(&self.score, &other.score)
            .unwrap_or_else(|| self.date().cmp(&other.date()))
    }

    /// Compares two stories, ordering by date.
    pub fn compare_date(&self, other: &Story) -> std::cmp::Ordering {
        self.date().cmp(&other.date())
    }

    pub fn title(&self) -> String {
        self.title_choice().1
    }

    pub fn score(&self, config: &StoryScoreConfig, score_type: StoryScoreType) -> f32 {
        StoryScorer::score(config, self, score_type)
    }

    pub fn score_detail(
        &self,
        config: &StoryScoreConfig,
        score_type: StoryScoreType,
    ) -> Vec<(String, f32)> {
        StoryScorer::score_detail(config, self, score_type)
    }

    /// Choose a title based on source priority, with preference for shorter titles if the priority is the same.
    fn title_choice(&self) -> (ScrapeSource, String) {
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
        let mut best_title = (99, &ScrapeSource::Other, "Unknown title".to_owned());
        for (id, scrape) in &self.scrapes {
            let score = title_score(&id.source);
            if score < best_title.0 {
                best_title = (score, &id.source, scrape.title());
                continue;
            }
            let title = scrape.title();
            if score == best_title.0 && title.len() < best_title.2.len() {
                best_title = (score, &id.source, scrape.title());
                continue;
            }
        }
        (*best_title.1, best_title.2)
    }

    pub fn url(&self) -> StoryUrl {
        self.scrapes
            .values()
            .next()
            .expect("Expected at least one")
            .url()
    }

    /// Returns the date of this story, which is always the earliest scrape date.
    pub fn date(&self) -> StoryDate {
        self.scrapes
            .values()
            .map(|s| s.date())
            .min()
            .unwrap_or_default()
    }

    pub fn render(&self, order: usize) -> StoryRender {
        let scrapes = HashMap::from_iter(self.scrapes.iter().map(|(k, v)| (k.as_str(), v.clone())));
        StoryRender {
            order,
            id: self.id.to_base64(),
            score: self.score,
            url: self.url().to_string(),
            url_norm: self.url().normalization().string().to_owned(),
            url_norm_hash: self.url().normalization().hash(),
            domain: self.url().host().to_owned(),
            title: self.title(),
            date: self.date(),
            tags: vec![],
            comment_links: HashMap::new(),
            scrapes,
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct StoryScoreConfig {
    age_breakpoint_days: [u32; 2],
    hour_scores: [f32; 3],
}

pub enum StoryScoreType {
    Base,
    AgedFrom(StoryDate),
}

#[derive(Debug)]
enum StoryScore {
    Age,
    Random,
    SourceCount,
    LongRedditTitle,
    LongTitle,
    ImageLink,
    HNPosition,
    RedditPosition,
    LobstersPosition,
}

/// Re-scores stories w/age score.
pub fn rescore_stories(config: &StoryScoreConfig, relative_to: StoryDate, stories: &mut [Story]) {
    for story in stories.iter_mut() {
        story.score += StoryScorer::score_age(config, relative_to - story.date());
    }
}

struct StoryScorer {}

impl StoryScorer {
    #[inline(always)]
    fn score_age(config: &StoryScoreConfig, age: chrono::Duration) -> f32 {
        let breakpoint1 = Duration::days(config.age_breakpoint_days[0] as i64);
        let breakpoint2 = Duration::days(config.age_breakpoint_days[1] as i64);
        let hour_score0 = config.hour_scores[0];
        let hour_score1 = config.hour_scores[1];
        let hour_score2 = config.hour_scores[2];

        // Equivalent to Duration::hours(1).num_milliseconds() as f32;
        const MILLIS_TO_HOURS: f32 = 60.0 * 60.0 * 1000.0;

        // Fractional hours, clamped to zero
        let fractional_hours = f32::max(0.0, age.num_milliseconds() as f32 / MILLIS_TO_HOURS);

        if age < breakpoint1 {
            fractional_hours * hour_score0
        } else if age < breakpoint2 {
            breakpoint1.num_hours() as f32 * hour_score0
                + (fractional_hours - breakpoint1.num_hours() as f32) * hour_score1
        } else {
            breakpoint1.num_hours() as f32 * hour_score0
                + (breakpoint2 - breakpoint1).num_hours() as f32 * hour_score1
                + (fractional_hours - breakpoint2.num_hours() as f32) * hour_score2
        }
    }

    #[inline(always)]
    fn score_impl<T: FnMut(StoryScore, f32)>(
        config: &StoryScoreConfig,
        story: &Story,
        score_type: StoryScoreType,
        mut accum: T,
    ) {
        use StoryScore::*;

        let title = story.title();
        let url = story.url();

        // Small random shuffle for stories to mix up the front page a bit
        accum(
            Random,
            (url.normalization().hash() % 6000000) as f32 / 1000000.0,
        );

        // Story age decay
        if let StoryScoreType::AgedFrom(relative_date) = score_type {
            let age = relative_date - story.date();
            accum(StoryScore::Age, Self::score_age(config, age));
        }

        let mut reddit = None;
        let mut hn = None;
        let mut lobsters = None;
        let mut slashdot = None;

        // Pick out the first source we find for each source
        for (_, scrape) in &story.scrapes {
            match scrape {
                Scrape::HackerNews(x) => {
                    if x.position != 0 {
                        accum(HNPosition, (30.0 - x.position as f32) * 1.2)
                    };
                    hn = Some(x)
                }
                Scrape::Reddit(x) => reddit = Some(x),
                Scrape::Lobsters(x) => lobsters = Some(x),
                Scrape::Slashdot(x) => slashdot = Some(x),
            }
        }

        accum(
            SourceCount,
            (hn.is_some() as u8
                + reddit.is_some() as u8
                + lobsters.is_some() as u8
                + slashdot.is_some() as u8) as f32
                * 5.0,
        );

        // Penalize a long title if reddit is a source
        if title.len() > 130 && reddit.is_some() {
            accum(LongRedditTitle, -5.0);
        }

        // Penalize a really long title regardless of source
        if title.len() > 250 {
            accum(LongTitle, -15.0);
        }

        if url.host().contains("gfycat")
            || url.host().contains("imgur")
            || url.host().contains("i.reddit.com")
        {
            if hn.is_some() {
                accum(ImageLink, -5.0);
            } else {
                accum(ImageLink, -10.0);
            }
        }
    }

    pub fn score(config: &StoryScoreConfig, story: &Story, score_type: StoryScoreType) -> f32 {
        let mut score_total = 0_f32;
        let accum = |_, score| score_total += score;
        Self::score_impl(config, story, score_type, accum);
        score_total
    }

    pub fn score_detail(
        config: &StoryScoreConfig,
        story: &Story,
        score_type: StoryScoreType,
    ) -> Vec<(String, f32)> {
        let mut score_bits = vec![];
        let accum = |score_type, score| score_bits.push((format!("{:?}", score_type), score));
        Self::score_impl(config, story, score_type, accum);
        score_bits
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use chrono::Duration;

    /// Make sure that the scores are decreasing.
    #[test]
    fn test_age_score() {
        let config = StoryScoreConfig {
            age_breakpoint_days: [1, 30],
            hour_scores: [-5.0, -3.0, -0.1],
        };
        let mut last_score = f32::MAX;
        for i in 0..Duration::days(60).num_hours() {
            let score = StoryScorer::score_age(&config, Duration::hours(i));
            assert!(score < last_score, "{} < {}", score, last_score);
            last_score = score;
        }
    }

    #[test]
    fn test_story_identifier() {
        let url = StoryUrl::parse("https://google.com/?q=foo").expect("Failed to parse URL");
        let id = StoryIdentifier::new(StoryDate::now(), url.normalization());
        let base64 = id.to_base64();
        assert_eq!(
            id,
            StoryIdentifier::from_base64(base64).expect("Failed to decode ID")
        );
    }
}
