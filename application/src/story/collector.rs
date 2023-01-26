use std::{cmp::Ordering, collections::BinaryHeap};

use crate::Story;

pub struct StoryCollector {
    stories: BinaryHeap<StoryWrapper>,
    capacity: usize,
}

struct StoryWrapper(Story);

impl Ord for StoryWrapper {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.score.total_cmp(&other.0.score).reverse()
    }
}

impl PartialOrd for StoryWrapper {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.0.score.total_cmp(&other.0.score).reverse())
    }
}

impl PartialEq for StoryWrapper {
    fn eq(&self, other: &Self) -> bool {
        self.0.score.total_cmp(&other.0.score) == Ordering::Equal
    }
}

impl Eq for StoryWrapper {}

impl StoryCollector {
    pub fn new(capacity: usize) -> Self {
        Self {
            stories: BinaryHeap::with_capacity(capacity + 1),
            capacity,
        }
    }

    pub fn len(&self) -> usize {
        self.stories.len()
    }

    pub fn min_score(&self) -> f32 {
        self.stories.peek().map(|x| x.0.score).unwrap_or(f32::MIN)
    }

    #[inline(always)]
    pub fn would_accept(&self, score: f32) -> bool {
        self.stories.len() < self.capacity || score > self.min_score()
    }

    pub fn accept(&mut self, story: Story) -> bool {
        if !self.would_accept(story.score) {
            return false;
        }
        self.stories.push(StoryWrapper(story));
        while self.stories.len() > self.capacity {
            self.stories.pop();
        }
        true
    }

    pub fn to_sorted(mut self) -> Vec<Story> {
        // This will be easier w/.drain_sorted()
        let mut v = Vec::with_capacity(self.stories.len());
        while let Some(story) = self.stories.pop() {
            v.push(story.0);
        }
        v.reverse();
        v
    }

    #[cfg(test)]
    pub fn scores(&self) -> Vec<f32> {
        use itertools::Itertools;

        let mut v = self.stories.iter().map(|x| x.0.score).collect_vec();
        v.sort_by(|a, b| a.total_cmp(b));
        v
    }
}

#[cfg(test)]
mod test {
    use std::collections::{HashMap, HashSet};

    use progscrape_scrapers::{StoryDate, StoryUrl};

    use super::*;

    fn make_story_with_score(score: f32) -> Story {
        Story::new_from_parts(
            "title".into(),
            StoryUrl::parse("http://example.com").expect("url"),
            StoryDate::year_month_day(2000, 1, 1).expect("date"),
            score,
            vec![],
            vec![],
        )
    }

    #[test]
    fn test_collect_lower() {
        let mut collector = StoryCollector::new(10);
        collector.accept(make_story_with_score(10.0));
        collector.accept(make_story_with_score(9.0));

        assert_eq!(collector.len(), 2);
    }

    #[test]
    fn test_collector() {
        let mut collector = StoryCollector::new(10);

        // Empty collector will accept all stories
        assert!(collector.would_accept(1.0));
        assert!(collector.would_accept(-1000000.0));

        // Will accept all scores when not at capacity
        for i in 0..10 {
            assert!(collector.accept(make_story_with_score(i as f32 * 10.0)));
        }

        assert_eq!(collector.min_score() as i32, 0);

        // Won't accept scores below or equal to the min
        assert!(!collector.would_accept(-10.0));
        assert!(!collector.accept(make_story_with_score(-10.0)));
        assert!(!collector.would_accept(0.0));
        assert!(!collector.accept(make_story_with_score(0.0)));

        // Will accept
        assert!(collector.accept(make_story_with_score(1.0)));
        // Will not accept (1.0 is now the minimum)
        assert!(!collector.accept(make_story_with_score(1.0)));
    }
}
