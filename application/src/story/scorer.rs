use serde::{Deserialize, Serialize};

use progscrape_scrapers::{TypedScrape, StoryDate, StoryDuration};

use super::{Story};

#[derive(Clone, Default, Serialize, Deserialize)]
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

pub struct StoryScorer {
    config: StoryScoreConfig,
}

impl StoryScorer {
    pub fn new(config: &StoryScoreConfig) -> Self {
        Self {
            config: config.clone(),
        }
    }

    /// Re-scores stories w/age score.
    pub fn resort_stories(&self, relative_to: StoryDate, stories: &mut [Story]) {
        stories.sort_by_cached_key(|story| {
            ((story.score + self.score_age(relative_to - story.date)) * 100000.0) as i64
        });
    }

    #[inline(always)]
    fn score_age(&self, age: StoryDuration) -> f32 {
        let breakpoint1 = StoryDuration::days(self.config.age_breakpoint_days[0] as i64);
        let breakpoint2 = StoryDuration::days(self.config.age_breakpoint_days[1] as i64);
        let hour_score0 = self.config.hour_scores[0];
        let hour_score1 = self.config.hour_scores[1];
        let hour_score2 = self.config.hour_scores[2];

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
        &self,
        story: &Story,
        score_type: StoryScoreType,
        mut accum: T,
    ) {
        use StoryScore::*;

        let title = &story.title;
        let url = &story.url;

        // Small random shuffle for stories to mix up the front page a bit
        accum(
            Random,
            (url.normalization().hash() % 6000000) as f32 / 1000000.0,
        );

        // Story age decay
        if let StoryScoreType::AgedFrom(relative_date) = score_type {
            let age = relative_date - story.date;
            accum(StoryScore::Age, self.score_age(age));
        }

        let mut reddit = None;
        let mut hn = None;
        let mut lobsters = None;
        let mut slashdot = None;

        use TypedScrape::*;
        // Pick out the first source we find for each source
        for (_, scrape) in &story.scrapes {
            match scrape {
                HackerNews(x) => {
                    // TODO: Rank
                    // if x.data.position != 0 {
                    //     accum(HNPosition, (30.0 - x.data.position as f32) * 1.2)
                    // };
                    hn = Some(x)
                }
                Reddit(x) => reddit = Some(x),
                Lobsters(x) => lobsters = Some(x),
                Slashdot(x) => slashdot = Some(x),
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

    pub fn score(&self, story: &Story, score_type: StoryScoreType) -> f32 {
        let mut score_total = 0_f32;
        let accum = |_, score| score_total += score;
        self.score_impl(story, score_type, accum);
        score_total
    }

    pub fn score_detail(&self, story: &Story, now: StoryDate) -> Vec<(String, f32)> {
        let mut score_bits = vec![];
        let accum = |score_type, score| score_bits.push((format!("{:?}", score_type), score));
        self.score_impl(story, StoryScoreType::AgedFrom(now), accum);
        score_bits
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// Make sure that the scores are decreasing.
    #[test]
    fn test_age_score() {
        let config = StoryScoreConfig {
            age_breakpoint_days: [1, 30],
            hour_scores: [-5.0, -3.0, -0.1],
        };
        let mut last_score = f32::MAX;
        let scorer = StoryScorer::new(&config);
        for i in 0..StoryDuration::days(60).num_hours() {
            let score = scorer.score_age(StoryDuration::hours(i));
            assert!(score < last_score, "{} < {}", score, last_score);
            last_score = score;
        }
    }
}