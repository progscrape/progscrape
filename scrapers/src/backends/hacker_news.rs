use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::{
    borrow::{Borrow, Cow},
    collections::HashMap,
};
use tl::{HTMLTag, Parser, ParserOptions};

use super::{
    scrape_story, utils::html::*, GenericScrape, ScrapeConfigSource, ScrapeCore, ScrapeShared,
    ScrapeSource, ScrapeSourceDef, ScrapeStory, Scraper,
};
use crate::types::*;

pub struct HackerNews {}

impl ScrapeSourceDef for HackerNews {
    type Config = HackerNewsConfig;
    type Scrape = HackerNewsStory;
    type Scraper = HackerNewsScraper;

    fn comments_url(id: &str, _subsource: Option<&str>) -> String {
        format!("https://news.ycombinator.com/item?id={}", id)
    }

    fn is_comments_host(host: &str) -> bool {
        host.ends_with("news.ycombinator.com")
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct HackerNewsConfig {
    homepage: String,
    pages: Vec<String>,
}

impl ScrapeConfigSource for HackerNewsConfig {
    fn subsources(&self) -> Vec<String> {
        vec![]
    }

    fn provide_urls(&self, _: Vec<String>) -> Vec<String> {
        self.pages
            .iter()
            .map(|s| format!("{}{}", self.homepage, s))
            .collect_vec()
    }
}

scrape_story! {
    HackerNewsStory {
        points: u32,
        comments: u32,
        position: u32,
    }
}

impl ScrapeStory for HackerNewsStory {
    const TYPE: ScrapeSource = ScrapeSource::HackerNews;

    fn merge(&mut self, other: HackerNewsStory) {
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
            return Err("Story table cannot contain other tables".to_string());
        }

        fn extract_number(s: &str) -> Result<u32, String> {
            str::parse(&s.replace(|c| !('0'..='9').contains(&c), ""))
                .map_err(|_| format!("Failed to parse number: '{}'", s))
        }

        return if let Some(titleline) = find_first(p, node, ".titleline") {
            if find_first(p, node, ".votelinks").is_none() {
                return Err("Missing votelinks".to_string());
            }
            let first_link = find_first(p, titleline, "a")
                .ok_or_else(|| "Failed to query first link".to_string())?;
            let title = unescape_entities(first_link.inner_text(p).borrow());
            let mut url = unescape_entities(
                &get_attribute(p, first_link, "href")
                    .ok_or_else(|| "Failed to get href".to_string())?,
            );
            if url.starts_with("item?") {
                url.insert_str(0, "https://news.ycombinator.com/");
            }
            let url = StoryUrl::parse(&url).ok_or(format!("Failed to parse URL {}", url))?;
            let id =
                get_attribute(p, node, "id").ok_or_else(|| "Failed to get id node".to_string())?;
            let rank =
                find_first(p, node, ".rank").ok_or_else(|| "Failed to get rank".to_string())?;
            let position = rank
                .inner_text(p)
                .trim_end_matches('.')
                .parse()
                .or(Err("Failed to parse rank".to_string()))?;
            Ok(HackerNewsNode::StoryLine(HackerNewsStoryLine {
                id,
                position,
                url,
                title,
            }))
        } else if let Some(..) = find_first(p, node, ".subtext") {
            let age_node =
                find_first(p, node, ".age").ok_or_else(|| "Failed to query .age".to_string())?;
            let date = get_attribute(p, age_node, "title")
                .ok_or_else(|| "Failed to get age title".to_string())?
                + "Z";
            let date = StoryDate::parse_from_rfc3339(&date)
                .ok_or_else(|| "Failed to map date".to_string())?;
            let mut comments = None;
            for node in html_tag_iterator(p, node.query_selector(p, "a")) {
                let text = node.inner_text(p);
                if text.contains("comment") {
                    comments = Some(extract_number(text.borrow())?);
                } else if text.contains("discuss") {
                    comments = Some(0);
                }
            }
            let score_node = find_first(p, node, ".score")
                .ok_or_else(|| "Failed to query .score".to_string())?;
            let id = get_attribute(p, score_node, "id")
                .ok_or_else(|| "Missing ID on score node".to_string())?
                .trim_start_matches("score_")
                .into();
            let points = extract_number(score_node.inner_text(p).borrow())?;
            let comments = comments.ok_or_else(|| "Missing comment count".to_string())?;
            Ok(HackerNewsNode::InfoLine(HackerNewsInfoLine {
                id,
                comments,
                points,
                date,
            }))
        } else {
            Err("Unknown node type".to_string())
        };
    }

    fn tags_from_title(
        &self,
        _args: &<HackerNews as ScrapeSourceDef>::Config,
        title: &str,
    ) -> Vec<&'static str> {
        let mut tags = vec![];
        // TODO: Strip years [ie: (2005)] from end of title
        if title.starts_with("Show HN") {
            tags.push("show");
        }
        if title.starts_with("Ask HN") {
            tags.push("ask");
        }
        if title.ends_with("[pdf]") {
            tags.push("pdf");
        }
        if title.ends_with("[video]") {
            tags.push("video");
        }
        tags
    }
}

impl Scraper for HackerNewsScraper {
    type Config = <HackerNews as ScrapeSourceDef>::Config;
    type Output = <HackerNews as ScrapeSourceDef>::Scrape;

    fn scrape(
        &self,
        _args: &HackerNewsConfig,
        input: &str,
    ) -> Result<(Vec<GenericScrape<Self::Output>>, Vec<String>), ScrapeError> {
        let dom = tl::parse(input, ParserOptions::default())?;
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
                let HackerNewsStoryLine {
                    url,
                    title: raw_title,
                    position,
                    ..
                } = v;
                let HackerNewsInfoLine {
                    date,
                    points,
                    comments,
                    ..
                } = info;
                let id = k;
                stories.push(HackerNewsStory::new(
                    id, date, raw_title, url, points, comments, position,
                ));
            } else {
                errors.push(format!("Unmatched story/info for id {}", k));
            }
        }
        stories.sort_by_key(|x| x.data.position);
        Ok((stories, errors))
    }

    fn extract_core<'a>(
        &self,
        args: &Self::Config,
        input: &'a GenericScrape<Self::Output>,
    ) -> ScrapeCore<'a> {
        let tags = self
            .tags_from_title(args, &input.shared.raw_title)
            .into_iter()
            .map(Cow::Borrowed)
            .collect();
        ScrapeCore {
            source: &input.shared.id,
            title: &input.shared.raw_title,
            url: &input.shared.url,
            date: input.shared.date,
            rank: (input.data.position as usize).checked_sub(1),
            tags,
        }
    }
}
