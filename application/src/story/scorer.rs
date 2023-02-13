use serde::{Deserialize, Serialize};

use progscrape_scrapers::{
    ExtractedScrapeCollection, ScrapeCore, ScrapeSource, StoryDate, StoryDuration, TypedScrape,
    TypedScrapeMap,
};

use super::Story;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct StoryScoreMultiSourceConfig {
    power: f32,
    factor: f32,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct StoryScoreConfig {
    age_breakpoint_days: [u32; 2],
    hour_scores: [f32; 3],
    service_rank: TypedScrapeMap<f32>,
    multi_source: StoryScoreMultiSourceConfig,
}

pub enum StoryScoreType {
    Base,
    AgedFrom(StoryDate),
}

#[derive(Debug)]
pub enum StoryScore {
    Age,
    Random,
    SourceCount,
    LongRedditTitle,
    LongTitle,
    ImageLink,
    SelfLink,
    PoorUpvoteRatio,
    UpvoteCount,
    CommentCount,
    Position(ScrapeSource),
}

impl Serialize for StoryScore {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        format!("{:?}", self).serialize(serializer)
    }
}

pub struct StoryScorer {
    config: StoryScoreConfig,
}

trait ServiceScorer {}

// impl ServiceScorer for Generic

impl StoryScorer {
    pub fn new(config: &StoryScoreConfig) -> Self {
        Self {
            config: config.clone(),
        }
    }

    /// Re-scores stories w/age score.
    pub fn resort_stories<S>(&self, relative_to: StoryDate, stories: &mut [Story<S>]) {
        let new_score =
            move |story: &Story<S>| story.score + self.score_age(relative_to - story.date);

        stories.sort_by_cached_key(|story| (new_score(story) * -100000.0) as i64);
    }

    #[inline(always)]
    pub fn score_age(&self, age: StoryDuration) -> f32 {
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

    /// Score a single scrape so that we can evaluate which of multiple stories we want to
    /// choose.
    #[inline(always)]
    fn score_single<T: FnMut(StoryScore, f32)>(
        &self,
        scrape: &TypedScrape,
        core: &ScrapeCore,
        mut accum: T,
    ) {
        use StoryScore::*;

        let url = core.url;

        let source = scrape.id.source;
        if let Some(rank) = core.rank {
            accum(
                Position(source),
                (30.0 - rank.clamp(0, 30) as f32) * self.config.service_rank.get(source),
            );
        }

        if url.host().contains("gfycat")
            || url.host().contains("imgur")
            || url.host().contains("i.reddit.com")
        {
            if source == ScrapeSource::HackerNews {
                accum(ImageLink, -5.0);
            } else {
                accum(ImageLink, -10.0);
            }
        }

        match scrape {
            TypedScrape::HackerNews(hn) => {
                if hn.data.comments > 100 {
                    accum(CommentCount, 5.0);
                }
            }
            TypedScrape::Reddit(reddit) => {
                // Penalize Reddit self links
                if url.host().contains("reddit.com") {
                    accum(SelfLink, -20.0);
                }

                // Penalize a long title if reddit is a source
                if core.title.len() > 130 {
                    accum(LongRedditTitle, -5.0);
                }

                if reddit.data.upvote_ratio < 0.6 {
                    accum(PoorUpvoteRatio, -20.0);
                }
                if reddit.data.upvotes < 10 {
                    accum(UpvoteCount, -20.0);
                } else if reddit.data.upvotes > 10 {
                    accum(UpvoteCount, 10.0);
                } else if reddit.data.upvotes > 100 {
                    accum(UpvoteCount, 15.0);
                }
                if reddit.data.num_comments < 10 {
                    accum(CommentCount, -5.0);
                } else if reddit.data.num_comments > 10 {
                    accum(CommentCount, 5.0);
                }
            }
            TypedScrape::Lobsters(lobsters) => {
                // This won't get triggered until we start scraping lobsters comment counts
                if lobsters.data.num_comments > 100 {
                    accum(CommentCount, 5.0);
                }
            }
            TypedScrape::Slashdot(slashdot) => {
                if slashdot.data.num_comments > 100 {
                    accum(CommentCount, 5.0);
                }
            }
        }
    }

    #[inline(always)]
    fn score_impl<T: FnMut(StoryScore, f32)>(
        &self,
        scrapes: &ExtractedScrapeCollection,
        best: TypedScrapeMap<Option<(&TypedScrape, &ScrapeCore, f32)>>,
        mut accum: T,
    ) {
        use StoryScore::*;

        let title = scrapes.title;
        let url = scrapes.url();

        // Small random shuffle for stories to mix up the front page a bit
        accum(
            Random,
            (url.normalization().hash() % 6000000) as f32 / 1000000.0,
        );

        accum(
            SourceCount,
            (scrapes.scrapes.len() as f32).powf(self.config.multi_source.power)
                * self.config.multi_source.factor,
        );

        for (scrape, core, _) in best.values().flatten() {
            self.score_single(scrape, core, &mut accum);
        }

        // Penalize a really long title regardless of source
        if title.len() > 250 {
            accum(LongTitle, -15.0);
        }
    }

    fn calculate_best_scrapes<'a, 'b>(
        &self,
        scrapes: &'a ExtractedScrapeCollection<'b>,
    ) -> TypedScrapeMap<Option<(&'a TypedScrape, &'a ScrapeCore<'b>, f32)>> {
        let mut service_scrapes = TypedScrapeMap::new();
        for (id, (core, scrape)) in &scrapes.scrapes {
            let mut score_total = 0_f32;
            let accum = |_, score| score_total += score;
            self.score_single(scrape, core, accum);
            if let Some((_, _, existing_score)) = service_scrapes.get(id.source) {
                if *existing_score > score_total {
                    continue;
                }
            }
            service_scrapes.set(id.source, Some((*scrape, core, score_total)));
        }
        service_scrapes
    }

    pub fn score(&self, scrapes: &ExtractedScrapeCollection) -> f32 {
        let best = self.calculate_best_scrapes(scrapes);
        let mut score_total = 0_f32;
        let accum = |_, score| score_total += score;
        self.score_impl(scrapes, best, accum);
        score_total
    }

    pub fn score_detail(
        &self,
        scrapes: &ExtractedScrapeCollection,
        now: StoryDate,
    ) -> Vec<(StoryScore, f32)> {
        let best = self.calculate_best_scrapes(scrapes);
        let mut score_bits = vec![];
        let mut accum = |score_type, score| score_bits.push((score_type, score));
        accum(StoryScore::Age, self.score_age(now - scrapes.earliest));
        self.score_impl(scrapes, best, accum);
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
            service_rank: TypedScrapeMap::new_with_all(1.0),
            multi_source: StoryScoreMultiSourceConfig {
                power: 2.0,
                factor: 10.0,
            },
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
