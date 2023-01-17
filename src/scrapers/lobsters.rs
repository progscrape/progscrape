use std::collections::HashSet;

use super::*;
use roxmltree::Document;
use serde::{Deserialize, Serialize};

pub struct Lobsters {}

impl ScrapeSourceDef for Lobsters {
    type Config = LobstersConfig;
    type Scrape = LobstersStory;
    type Scraper = LobstersScraper;
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct LobstersConfig {
    feed: String,
    tag_denylist: HashSet<String>,
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
    pub date: StoryDate,
    pub num_comments: u32,
    pub position: u32,
    pub score: u32,
    pub tags: Vec<String>,
}

impl ScrapeStory for LobstersStory {
    const TYPE: ScrapeSource = ScrapeSource::Lobsters;

    fn comments_url(&self) -> String {
        unimplemented!()
    }

    fn merge(&mut self, other: LobstersStory) {
        self.score = std::cmp::max(self.score, other.score);
        self.num_comments = std::cmp::max(self.num_comments, other.num_comments);
    }
}

#[derive(Default)]
pub struct LobstersScraper {}

impl Scraper for LobstersScraper {
    type Config = <Lobsters as ScrapeSourceDef>::Config;
    type Output = <Lobsters as ScrapeSourceDef>::Scrape; 

    fn scrape(
            &self,
            args: &Self::Config,
            input: &str,
        ) -> Result<(Vec<Self::Output>, Vec<String>), ScrapeError> {
        let doc = Document::parse(&input)?;
        let rss = doc.root_element();
        let mut warnings = vec![];
        let mut stories = vec![];
        for channel in rss.children() {
            if channel.tag_name().name() == "channel" {
                for (position, item) in channel
                    .children()
                    .filter(|item| item.tag_name().name() == "item")
                    .enumerate()
                {
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
                            "guid" => {
                                id = subitem.text().map(|s| {
                                    s.trim_start_matches("https://lobste.rs/s/").to_owned()
                                })
                            }
                            "link" => url = subitem.text().and_then(StoryUrl::parse),
                            "author" => {}
                            "pubDate" => {
                                date = subitem.text().and_then(StoryDate::parse_from_rfc2822)
                            }
                            "comments" => {}
                            "category" => drop(subitem.text().map(|s| tags.push(s.to_owned()))),
                            "description" => {}
                            x => warnings.push(format!("Unknown sub-node '{}'", x)),
                        }
                    }
                    if let (Some(title), Some(id), Some(url), Some(date)) = (title, id, url, date) {
                        stories.push(LobstersStory {
                            id,
                            title,
                            url,
                            date,
                            num_comments: 0,
                            position: position as u32 + 1,
                            score: 0,
                            tags,
                        });
                    } else {
                        warnings.push("Story did not contain all required fields".to_string());
                    }
                }
            }
        }
        Ok((stories, warnings))
    }

    fn extract_core<'a>(&self, args: &Self::Config, input: &'a Self::Output) -> ScrapeCore<'a> {
        let mut tags = Vec::new();
        for tag in &input.tags {
            if args.tag_denylist.contains(tag) {
                continue;
            }
            tags.push(Cow::Borrowed(tag.as_str()));
        }

        ScrapeCore { 
            source: ScrapeId::new(ScrapeSource::Lobsters, None, input.id.clone()), 
            title: Cow::Borrowed(input.title.as_str()), 
            url: &input.url, 
            date: input.date, 
            tags, 
            rank: (input.position as usize).checked_sub(1) 
        }
    }
}
