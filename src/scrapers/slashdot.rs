use serde::{Deserialize, Serialize};

use crate::story::{StoryDate, StoryUrl};

use super::{ScrapeData, ScrapeDataInit, ScrapeSource};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SlashdotStory {
    pub id: String,
    pub title: String,
    pub url: StoryUrl,
    pub num_comments: u32,
    pub score: u32,
    pub date: StoryDate,
    pub tags: Vec<String>,
}

impl ScrapeData for SlashdotStory {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn title(&self) -> String {
        return self.title.clone();
    }

    fn url(&self) -> StoryUrl {
        return self.url.clone();
    }

    fn comments_url(&self) -> String {
        unimplemented!()
    }

    fn source(&self) -> super::ScrapeSource {
        return ScrapeSource::Slashdot;
    }

    fn date(&self) -> StoryDate {
        unimplemented!()
    }
}

impl ScrapeDataInit<SlashdotStory> for SlashdotStory {
    fn initialize_required(
        id: String,
        title: String,
        url: StoryUrl,
        date: StoryDate,
    ) -> SlashdotStory {
        SlashdotStory {
            id,
            title,
            url,
            date,
            num_comments: Default::default(),
            score: Default::default(),
            tags: Default::default(),
        }
    }

    fn merge(&mut self, other: SlashdotStory) {
        self.title = other.title;
        self.url = other.url;
        self.date = std::cmp::min(self.date, other.date);
        self.score = std::cmp::max(self.score, other.score);
        self.num_comments = std::cmp::max(self.num_comments, other.num_comments);
    }
}
