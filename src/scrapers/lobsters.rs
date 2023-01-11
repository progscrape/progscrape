use super::*;
use serde::{Deserialize, Serialize};

pub struct Lobsters {}

impl ScrapeSource2 for Lobsters {
    type Config = LobstersConfig;
    type Scrape = LobstersStory;
    type Scraper = LobstersScraper;
    const TYPE: ScrapeSource = ScrapeSource::Lobsters;
}

#[derive(Default, Serialize, Deserialize)]
pub struct LobstersConfig {
    feed: String,
}

impl ScrapeConfigSource for LobstersConfig {
    fn subsources(&self) -> Vec<String> {
        vec![]
    }

    fn provide_urls(&self, _: Vec<String>) -> Vec<String> {
        vec![self.feed.clone()]
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LobstersStory {
    pub id: String,
    pub title: String,
    pub url: StoryUrl,
    pub num_comments: u32,
    pub score: u32,
    pub date: StoryDate,
    pub tags: Vec<String>,
}

impl ScrapeData for LobstersStory {
    fn title(&self) -> String {
        return self.title.clone();
    }

    fn url(&self) -> StoryUrl {
        return self.url.clone();
    }

    fn comments_url(&self) -> String {
        unimplemented!()
    }

    fn source(&self) -> ScrapeId {
        ScrapeId::new(ScrapeSource::Lobsters, None, self.id.clone())
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
            tags: Default::default(),
        }
    }

    fn merge(&mut self, other: LobstersStory) {
        self.title = other.title;
        self.url = other.url;
        self.date = std::cmp::min(self.date, other.date);
        self.score = std::cmp::max(self.score, other.score);
        self.num_comments = std::cmp::max(self.num_comments, other.num_comments);
    }
}

#[derive(Default)]
pub struct LobstersScraper {}

impl Scraper<LobstersConfig, LobstersStory> for LobstersScraper {
    fn scrape(
        &self,
        _args: &LobstersConfig,
        _input: String,
    ) -> Result<(Vec<LobstersStory>, Vec<String>), ScrapeError> {
        unimplemented!()
    }
}
