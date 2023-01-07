use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{unescape_entities, ScrapeData, ScrapeDataInit, ScrapeError, ScrapeSource, Scraper};
use crate::story::{StoryDate, StoryUrl};

#[derive(Default)]
pub struct RedditArgs {}

#[derive(Default)]
pub struct RedditScraper {}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RedditStory {
    pub title: String,
    pub url: StoryUrl,
    pub subreddit: String,
    pub flair: String,
    pub id: String,
    pub position: u32,
    pub upvotes: u32,
    pub downvotes: u32,
    pub num_comments: u32,
    pub score: u32,
    pub upvote_ratio: f32,
    pub date: StoryDate,
}

impl ScrapeData for RedditStory {
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
        return ScrapeSource::Reddit(self.subreddit.clone());
    }

    fn date(&self) -> StoryDate {
        self.date
    }
}

impl ScrapeDataInit<RedditStory> for RedditStory {
    fn initialize_required(
        id: String,
        title: String,
        url: StoryUrl,
        date: StoryDate,
    ) -> RedditStory {
        Self {
            title,
            url,
            id,
            date,
            subreddit: Default::default(),
            flair: Default::default(),
            position: Default::default(),
            upvotes: Default::default(),
            downvotes: Default::default(),
            num_comments: Default::default(),
            score: Default::default(),
            upvote_ratio: Default::default(),
        }
    }

    fn merge(&mut self, other: RedditStory) {
        self.title = other.title;
        self.url = other.url;
        self.date = std::cmp::min(self.date, other.date);
        self.flair = other.flair;
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
            return Err(format!(
                "Failed to parse {} as integer (value was {:?})",
                key, n
            ));
        } else {
            return Err(format!(
                "Missing or invalid field {:?} (value was {:?})",
                key, data[key]
            ));
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
            return Err(format!(
                "Failed to parse {} as float (value was {:?})",
                key, n
            ));
        } else {
            return Err(format!(
                "Missing or invalid field {:?} (value was {:?})",
                key, data[key]
            ));
        }
    }

    fn map_story(&self, child: &Value, position: u32) -> Result<RedditStory, String> {
        let kind = child["kind"].as_str();
        let data;
        if kind == Some("t3") {
            data = &child["data"];
        } else {
            return Err(format!("Unknown story type: {:?}", kind));
        }

        let millis = self.require_integer(data, "created_utc")?;
        let date = StoryDate::from_millis(millis).ok_or(format!("Unmappable date"))?;
        let url = StoryUrl::parse(unescape_entities(&self.require_string(data, "url")?))
            .ok_or(format!("Unmappable URL"))?;
        let story = RedditStory {
            title: unescape_entities(&self.require_string(data, "title")?),
            url,
            num_comments: self.require_integer(data, "num_comments")?,
            score: self.require_integer(data, "score")?,
            downvotes: self.require_integer(data, "downs")?,
            upvotes: self.require_integer(data, "ups")?,
            upvote_ratio: self.require_float(data, "upvote_ratio")? as f32,
            subreddit: self.require_string(data, "subreddit")?,
            flair: self.optional_string(data, "link_flair_text")?,
            id: self.require_string(data, "id")?,
            date,
            position,
        };
        return Ok(story);
    }
}

impl Scraper<RedditArgs, RedditStory> for RedditScraper {
    fn scrape(
        &self,
        _args: RedditArgs,
        input: String,
    ) -> Result<(Vec<RedditStory>, Vec<String>), ScrapeError> {
        let value: Value = serde_json::from_str(&input)?;
        if let Some(object) = value.as_object() {
            if let Some(children) = object["data"]["children"].as_array() {
                let mut vec = vec![];
                let mut errors = vec![];
                for (position, child) in children.iter().enumerate() {
                    match self.map_story(child, position as u32) {
                        Ok(story) => vec.push(story),
                        Err(e) => errors.push(e),
                    }
                }
                return Ok((vec, errors));
            } else {
                return Err(ScrapeError::StructureError(
                    "Missing children element".to_owned(),
                ));
            }
        } else {
            return Err(ScrapeError::StructureError(
                "Failed to parse Reddit JSON".to_owned(),
            ));
        }
    }
}

#[cfg(test)]
pub mod test {
    use super::super::test::*;
    use super::*;

    pub fn scrape_all() -> Vec<RedditStory> {
        let mut all = vec![];
        let scraper = RedditScraper::default();
        for file in reddit_files() {
            let stories = scraper
                .scrape(RedditArgs::default(), load_file(file))
                .expect(&format!("Failed to parse a story from {}", file));
            all.extend(stories.0);
        }
        all
    }

    #[test]
    fn test_parse_sample() {
        let scraper = RedditScraper::default();
        for file in reddit_files() {
            let stories = scraper
                .scrape(RedditArgs::default(), load_file(file))
                .unwrap();
            for story in stories.0 {
                println!("[{}] {} ({})", story.subreddit, story.title, story.url);
            }
        }
    }
}
