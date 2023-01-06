use super::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LobstersStory {
    pub id: String,
    pub title: String,
    pub url: StoryUrl,
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

    fn url(&self) -> StoryUrl {
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

impl ScrapeDataInit<LobstersStory> for LobstersStory {
    fn initialize_required(
        id: String,
        title: String,
        url: StoryUrl,
        date: StoryDate,
    ) -> LobstersStory {
        LobstersStory {
            id,
            title,
            url,
            date,
            num_comments: Default::default(),
            score: Default::default(),
        }
    }
}
