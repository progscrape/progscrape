use std::{borrow::Cow, collections::HashMap};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    scrape_story, utils::html::unescape_entities, GenericScrape, ScrapeConfigSource, ScrapeCore,
    ScrapeShared, ScrapeSource, ScrapeSourceDef, ScrapeStory, Scraper,
};
use crate::types::*;

pub struct Reddit {}

impl ScrapeSourceDef for Reddit {
    type Config = RedditConfig;
    type Scrape = RedditStory;
    type Scraper = RedditScraper;

    fn comments_url(id: &str, subsource: Option<&str>) -> String {
        if let Some(subsource) = subsource {
            format!("https://www.reddit.com/r/{}/comments/{}/", subsource, id)
        } else {
            format!("https://www.reddit.com/comments/{}/", id)
        }
    }

    fn is_comments_host(host: &str) -> bool {
        host.ends_with("reddit.com")
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct RedditConfig {
    api: String,
    subreddit_batch: usize,
    limit: usize,
    subreddits: HashMap<String, SubredditConfig>,
}

impl ScrapeConfigSource for RedditConfig {
    fn subsources(&self) -> Vec<String> {
        self.subreddits.iter().map(|s| s.0.clone()).collect()
    }

    fn provide_urls(&self, subsources: Vec<String>) -> Vec<String> {
        let mut output = vec![];
        for chunk in subsources.chunks(self.subreddit_batch) {
            output.push(
                self.api.replace("${subreddits}", &chunk.join("+"))
                    + &format!("?limit={}", self.limit),
            )
        }
        output
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct SubredditConfig {
    #[serde(default)]
    is_tag: bool,
    #[serde(default)]
    flair_is_tag: bool,
}

#[derive(Default)]
pub struct RedditScraper {}

scrape_story! {
    RedditStory {
        flair: String,
        position: u32,
        upvotes: u32,
        downvotes: u32,
        num_comments: u32,
        score: u32,
        upvote_ratio: f32,
    }
}

impl ScrapeStory for RedditStory {
    const TYPE: ScrapeSource = ScrapeSource::Reddit;

    fn merge(&mut self, other: RedditStory) {
        self.position = std::cmp::max(self.position, other.position);
        self.upvotes = std::cmp::max(self.upvotes, other.upvotes);
        self.downvotes = std::cmp::max(self.downvotes, other.downvotes);
        self.num_comments = std::cmp::max(self.num_comments, other.num_comments);
        self.score = std::cmp::max(self.score, other.score);
        self.upvote_ratio = f32::max(self.upvote_ratio, other.upvote_ratio);
    }
}

impl RedditScraper {
    fn require_string(&self, data: &Value, key: &str) -> Result<String, String> {
        Ok(data[key]
            .as_str()
            .ok_or(format!("Missing field {:?}", key))?
            .to_owned())
    }

    fn optional_string(&self, data: &Value, key: &str) -> Result<String, String> {
        Ok(data[key].as_str().unwrap_or_default().to_owned())
    }

    fn require_integer<T: TryFrom<i64> + TryFrom<u64>>(
        &self,
        data: &Value,
        key: &str,
    ) -> Result<T, String> {
        if let Value::Number(n) = &data[key] {
            if let Some(n) = n.as_u64() {
                if let Ok(n) = n.try_into() {
                    return Ok(n);
                }
            }
            if let Some(n) = n.as_i64() {
                if let Ok(n) = n.try_into() {
                    return Ok(n);
                }
            }
            if let Some(n) = n.as_f64() {
                let n = n as i64;
                if let Ok(n) = n.try_into() {
                    return Ok(n);
                }
            }
            Err(format!(
                "Failed to parse {} as integer (value was {:?})",
                key, n
            ))
        } else {
            Err(format!(
                "Missing or invalid field {:?} (value was {:?})",
                key, data[key]
            ))
        }
    }

    fn require_float(&self, data: &Value, key: &str) -> Result<f64, String> {
        if let Value::Number(n) = &data[key] {
            if let Some(n) = n.as_u64() {
                return Ok(n as f64);
            }
            if let Some(n) = n.as_i64() {
                return Ok(n as f64);
            }
            if let Some(n) = n.as_f64() {
                return Ok(n);
            }
            Err(format!(
                "Failed to parse {} as float (value was {:?})",
                key, n
            ))
        } else {
            Err(format!(
                "Missing or invalid field {:?} (value was {:?})",
                key, data[key]
            ))
        }
    }

    fn map_story(
        &self,
        child: &Value,
        positions: &mut HashMap<String, u32>,
    ) -> Result<GenericScrape<<Self as Scraper>::Output>, String> {
        let kind = child["kind"].as_str();
        let data = if kind == Some("t3") {
            &child["data"]
        } else {
            return Err(format!("Unknown story type: {:?}", kind));
        };

        let id = self.require_string(data, "id")?;
        let subreddit = self.require_string(data, "subreddit")?.to_ascii_lowercase();
        if let Some(true) = data["stickied"].as_bool() {
            return Err(format!("Ignoring stickied story {}/{}", subreddit, id));
        }
        let position = *positions
            .entry(subreddit.clone())
            .and_modify(|n| *n += 1)
            .or_default()
            + 1;
        let seconds: i64 = self.require_integer(data, "created_utc")?;
        let millis = seconds * 1000;
        let date = StoryDate::from_millis(millis).ok_or_else(|| "Unmappable date".to_string())?;
        let url = StoryUrl::parse(unescape_entities(&self.require_string(data, "url")?))
            .ok_or_else(|| "Unmappable URL".to_string())?;
        let raw_title = unescape_entities(&self.require_string(data, "title")?);
        let num_comments = self.require_integer(data, "num_comments")?;
        let score = self.require_integer(data, "score")?;
        let downvotes = self.require_integer(data, "downs")?;
        let upvotes = self.require_integer(data, "ups")?;
        let upvote_ratio = self.require_float(data, "upvote_ratio")? as f32;
        let flair = unescape_entities(&self.optional_string(data, "link_flair_text")?);
        let story = RedditStory::new_subsource(
            id,
            subreddit,
            date,
            raw_title,
            url,
            flair,
            position,
            upvotes,
            downvotes,
            num_comments,
            score,
            upvote_ratio,
        );
        Ok(story)
    }
}

impl Scraper for RedditScraper {
    type Config = <Reddit as ScrapeSourceDef>::Config;
    type Output = <Reddit as ScrapeSourceDef>::Scrape;

    fn scrape(
        &self,
        _args: &RedditConfig,
        input: &str,
    ) -> Result<(Vec<GenericScrape<Self::Output>>, Vec<String>), ScrapeError> {
        let root: Value = serde_json::from_str(input)?;
        let mut value = &root;
        for path in ["data", "children"] {
            if let Some(object) = value.as_object() {
                if let Some(nested_value) = object.get(path) {
                    value = nested_value;
                } else {
                    return Err(ScrapeError::StructureError(
                        "Failed to parse Reddit JSON data.children".to_owned(),
                    ));
                }
            }
        }

        if let Some(children) = value.as_array() {
            let mut vec = vec![];
            let mut errors = vec![];
            let mut positions = HashMap::new();
            for child in children {
                match self.map_story(child, &mut positions) {
                    Ok(story) => vec.push(story),
                    Err(e) => errors.push(e),
                }
            }
            Ok((vec, errors))
        } else {
            Err(ScrapeError::StructureError(
                "Missing children element".to_owned(),
            ))
        }
    }

    fn extract_core<'a>(
        &self,
        args: &Self::Config,
        input: &'a GenericScrape<Self::Output>,
    ) -> ScrapeCore<'a> {
        let mut tags = vec![];
        if let Some(ref subreddit) = input.shared.id.subsource {
            if let Some(config) = args.subreddits.get(subreddit) {
                if config.flair_is_tag && !input.data.flair.contains(' ') {
                    tags.push(Cow::Owned(input.data.flair.to_lowercase()));
                }
                if config.is_tag {
                    tags.push(Cow::Borrowed(subreddit.as_str()));
                }
            }
        }

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
