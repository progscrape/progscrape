use serde::{Deserialize, Serialize};
use std::{borrow::Borrow, collections::HashMap};
use tl::{HTMLTag, Parser, ParserOptions};

use super::{
    html::*, unescape_entities, ScrapeData, ScrapeDataInit, ScrapeError, ScrapeSource, Scraper,
};
use crate::story::{StoryDate, StoryUrl};

#[derive(Default, Serialize, Deserialize)]
pub struct HackerNewsConfig {
    homepage: String,
}

#[derive(Debug, Default)]
pub struct HackerNewsArgs {}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HackerNewsStory {
    pub title: String,
    pub url: StoryUrl,
    pub id: String,
    pub points: u32,
    pub comments: u32,
    pub position: u32,
    pub date: StoryDate,
}

impl ScrapeData for HackerNewsStory {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn title(&self) -> String {
        self.title.clone()
    }

    fn url(&self) -> StoryUrl {
        self.url.clone()
    }

    fn source(&self) -> ScrapeSource {
        ScrapeSource::HackerNews
    }

    fn comments_url(&self) -> String {
        unimplemented!()
    }

    fn date(&self) -> StoryDate {
        self.date
    }
}

impl ScrapeDataInit<HackerNewsStory> for HackerNewsStory {
    fn initialize_required(
        id: String,
        title: String,
        url: StoryUrl,
        date: StoryDate,
    ) -> HackerNewsStory {
        HackerNewsStory {
            title,
            url,
            id,
            date,
            points: Default::default(),
            comments: Default::default(),
            position: Default::default(),
        }
    }

    fn merge(&mut self, other: HackerNewsStory) {
        self.title = other.title;
        self.url = other.url;
        self.date = std::cmp::min(self.date, other.date);
        self.points = std::cmp::max(self.points, other.points);
        self.comments = std::cmp::max(self.comments, other.comments);
    }
}

#[derive(Default)]
pub struct HackerNewsScraper {}

#[derive(Debug)]
struct HackerNewsStoryLine {
    id: String,
    position: u32,
    url: StoryUrl,
    title: String,
}

#[derive(Debug)]
struct HackerNewsInfoLine {
    id: String,
    comments: u32,
    points: u32,
    date: StoryDate,
}

#[derive(Debug)]
enum HackerNewsNode {
    StoryLine(HackerNewsStoryLine),
    InfoLine(HackerNewsInfoLine),
}

impl HackerNewsScraper {
    fn map_node_to_story(&self, p: &Parser, node: &HTMLTag) -> Result<HackerNewsNode, String> {
        if find_first(p, node, "table").is_some() {
            return Err(format!("Story table cannot contain other tables"));
        }

        fn extract_number(s: &str) -> Result<u32, String> {
            str::parse(&s.replace(|c| c < '0' || c > '9', ""))
                .map_err(|_| format!("Failed to parse number: '{}'", s))
        }

        return if let Some(titleline) = find_first(p, node, ".titleline") {
            if find_first(p, node, ".votelinks").is_none() {
                return Err(format!("Missing votelinks"));
            }
            let first_link =
                find_first(p, titleline, "a").ok_or(format!("Failed to query first link"))?;
            let title = unescape_entities(first_link.inner_text(p).borrow());
            let mut url = unescape_entities(
                &get_attribute(p, first_link, "href").ok_or(format!("Failed to get href"))?,
            );
            if url.starts_with("item?") {
                url.insert_str(0, "https://news.ycombinator.com/");
            }
            let url = StoryUrl::parse(&url).ok_or(format!("Failed to parse URL {}", url))?;
            let id = get_attribute(p, node, "id").ok_or(format!("Failed to get id node"))?;
            let rank = find_first(p, node, ".rank").ok_or(format!("Failed to get rank"))?;
            let position = rank
                .inner_text(p)
                .trim_end_matches('.')
                .parse()
                .or(Err(format!("Failed to parse rank")))?;
            Ok(HackerNewsNode::StoryLine(HackerNewsStoryLine {
                id,
                position,
                url,
                title,
            }))
        } else if let Some(..) = find_first(p, node, ".subtext") {
            let age_node = find_first(p, node, ".age").ok_or(format!("Failed to query .age"))?;
            let date = get_attribute(p, age_node, "title")
                .ok_or(format!("Failed to get age title"))?
                + "Z";
            let date = StoryDate::parse_from_rfc3339(&date).ok_or(format!("Failed to map date"))?;
            let mut comments = None;
            for node in html_tag_iterator(p, node.query_selector(p, "a")) {
                let text = node.inner_text(p);
                if text.contains("comment") {
                    comments = Some(extract_number(text.borrow())?);
                } else if text.contains("discuss") {
                    comments = Some(0);
                }
            }
            let score_node =
                find_first(p, node, ".score").ok_or(format!("Failed to query .score"))?;
            let id = get_attribute(p, score_node, "id")
                .ok_or(format!("Missing ID on score node"))?
                .trim_start_matches("score_")
                .into();
            let points = extract_number(score_node.inner_text(p).borrow())?;
            let comments = comments.ok_or(format!("Missing comment count"))?;
            Ok(HackerNewsNode::InfoLine(HackerNewsInfoLine {
                id,
                comments,
                points,
                date,
            }))
        } else {
            Err(format!("Unknown node type"))
        };
    }
}

impl Scraper<HackerNewsArgs, HackerNewsStory> for HackerNewsScraper {
    fn scrape(
        &self,
        _args: HackerNewsArgs,
        input: String,
    ) -> Result<(Vec<HackerNewsStory>, Vec<String>), ScrapeError> {
        let dom = tl::parse(&input, ParserOptions::default())?;
        let p = dom.parser();
        let mut errors = vec![];
        let mut story_lines = HashMap::new();
        let mut info_lines = HashMap::new();
        for node in html_tag_iterator(p, dom.query_selector("tr")) {
            match self.map_node_to_story(p, node) {
                Ok(HackerNewsNode::InfoLine(x)) => {
                    info_lines.insert(x.id.clone(), x);
                }
                Ok(HackerNewsNode::StoryLine(x)) => {
                    story_lines.insert(x.id.clone(), x);
                }
                Err(e) => {
                    errors.push(e);
                }
            }
        }
        let mut stories = vec![];
        for (k, v) in story_lines {
            let info = info_lines.remove(&k);
            if let Some(info) = info {
                stories.push(HackerNewsStory {
                    title: v.title,
                    url: v.url,
                    id: k,
                    points: info.points,
                    comments: info.comments,
                    date: info.date,
                    position: v.position,
                })
            } else {
                errors.push(format!("Unmatched story/info for id {}", k));
            }
        }
        stories.sort_by_key(|x| x.position);
        Ok((stories, errors))
    }
}

#[cfg(test)]
pub mod test {
    use super::super::test::*;
    use super::*;

    pub fn scrape_all() -> Vec<HackerNewsStory> {
        let mut all = vec![];
        let scraper = HackerNewsScraper::default();
        for file in hacker_news_files() {
            let stories = scraper
                .scrape(HackerNewsArgs::default(), load_file(file))
                .expect(&format!("Failed to parse a story from {}", file));
            all.extend(stories.0);
        }
        all
    }

    #[test]
    fn test_parse_sample() {
        let scraper = HackerNewsScraper::default();
        for file in hacker_news_files() {
            let stories = scraper
                .scrape(HackerNewsArgs::default(), load_file(file))
                .unwrap();
            assert!(stories.0.len() >= 25);
            for story in stories.0 {
                println!(
                    "{}. [{}] {} ({}) c={} p={}",
                    story.position, story.id, story.title, story.url, story.comments, story.points
                );
            }
        }
    }
}
