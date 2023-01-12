use super::*;
use roxmltree::Document;
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
    pub position: u32,
    pub score: u32,
    pub date: StoryDate,
    pub tags: Vec<String>,
}

impl ScrapeData for LobstersStory {
    fn title(&self) -> String {
        self.title.clone()
    }

    fn url(&self) -> StoryUrl {
        self.url.clone()
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
            position: Default::default(),
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
        input: String,
    ) -> Result<(Vec<LobstersStory>, Vec<String>), ScrapeError> {
        let doc = Document::parse(&input)?;
        let rss = doc.root_element();
        let mut warnings = vec![];
        let mut stories = vec![];
        for channel in rss.children() {
            if channel.tag_name().name() == "channel" {
                for (position, item) in channel.children().filter(|item| item.tag_name().name() == "item").enumerate() {
                    let mut title = None;
                    let mut id = None;
                    let mut url = None;
                    let mut date = None;
                    let mut tags = vec![];
                    for subitem in item.children() {
                        if !subitem.is_element() {
                            continue;
                        }
                        match subitem.tag_name().name() {
                            "title" => title = subitem.text().map(|s| s.to_owned()),
                            "guid" => id = subitem.text().map(|s| s.trim_start_matches("https://lobste.rs/s/").to_owned()),
                            "link" => url = subitem.text().and_then(StoryUrl::parse),
                            "author" => {},
                            "pubDate" => date = subitem.text().and_then(StoryDate::parse_from_rfc2822),
                            "comments" => {},
                            "category" => drop(subitem.text().map(|s| tags.push(s.to_owned()))),
                            "description" => {},
                            x => {
                                warnings.push(format!("Unknown sub-node '{}'", x))
                            }
                        }
                    }
                    if let (Some(title), Some(id), Some(url), Some(date)) = (title, id, url, date) {
                        stories.push(LobstersStory {
                            title,
                            id,
                            date,
                            num_comments: 0,
                            position: position as u32,
                            score: 0,
                            tags,
                            url,
                        });
                    } else {
                        warnings.push(format!("Story did not contain all required fields"));
                    }
                }
            }
        }
        Ok((stories, warnings))
    }
}

#[cfg(test)]
pub mod test {
    use super::super::test::*;
    use super::*;

    pub fn scrape_all() -> Vec<LobstersStory> {
        let mut all = vec![];
        let scraper = LobstersScraper::default();
        for file in lobsters_files() {
            let stories = scraper
                .scrape(&LobstersConfig::default(), load_file(file))
                .unwrap_or_else(|_| panic!("Failed to parse a story from {}", file));
            all.extend(stories.0);
        }
        all
    }

    #[test]
    fn test_parse_sample() {
        let scraper = LobstersScraper::default();
        for file in lobsters_files() {
            let stories = scraper
                .scrape(&LobstersConfig::default(), load_file(file))
                .unwrap();
            assert!(stories.0.len() >= 25);
            for story in stories.0 {
                println!(
                    "{}. [{}] {} ({})",
                    story.position, story.id, story.title, story.url
                );
            }
        }
    }
}
