use super::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct LobstersStory {
    pub id: String,
    pub title: String,
    pub url: String,
    pub num_comments: u32,
    pub score: u32,
    pub date: StoryDate,
}

impl ScrapeData for LobstersStory {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn title(&self) -> String {
        return self.title.clone();
    }

    fn url(&self) -> String {
        return self.url.clone();
    }

    fn comments_url(&self) -> String {
        unimplemented!()
    }

    fn source(&self) -> super::ScrapeSource {
        return ScrapeSource::Lobsters;
    }

    fn date(&self) -> StoryDate {
        unimplemented!()
    }
}
