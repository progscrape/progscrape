use serde::{Deserialize, Serialize};
use tl::{HTMLTag, Parser, ParserOptions};

use crate::story::{StoryDate, StoryUrl};

use super::{html::*, ScrapeData, ScrapeDataInit, ScrapeSource, Scraper};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SlashdotStory {
    pub id: String,
    pub title: String,
    pub url: StoryUrl,
    pub num_comments: u32,
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
            tags: Default::default(),
        }
    }

    fn merge(&mut self, other: SlashdotStory) {
        self.title = other.title;
        self.url = other.url;
        self.date = std::cmp::min(self.date, other.date);
        self.num_comments = std::cmp::max(self.num_comments, other.num_comments);
    }
}

#[derive(Default)]
pub struct SlashdotArgs {}

#[derive(Default)]
pub struct SlashdotScraper {}

impl SlashdotScraper {
    fn map_story(p: &Parser, article: &HTMLTag) -> Result<SlashdotStory, String> {
        let title = find_first(p, article, ".story-title").ok_or("Missing .story-title")?;
        let mut links = html_tag_iterator(p, title.query_selector(p, "a"));
        let story_link = links.next().ok_or("Missing story link")?;
        let title = story_link.inner_text(p).to_string();
        if title.len() < 5 {
            return Err(format!("Title was too short: {}", title));
        }
        let story_url =
            get_attribute(p, story_link, "href").ok_or(format!("Missing story href"))?;
        let (_, b) = story_url
            .split_once("/story/")
            .ok_or(format!("Invalid link format: {}", story_url))?;
        let id = b.splitn(5, '/').take(4).collect::<Vec<_>>();
        if id.len() != 4 {
            return Err(format!("Invalid link format: {}", story_url));
        }
        let id = id.join("/");

        let external_link = links.next().ok_or("Missing external link")?;
        let href = get_attribute(p, external_link, "href").ok_or(format!("Missing href"))?;
        let url = StoryUrl::parse(&href).ok_or(format!("Invalid href: {}", href))?;

        // This doesn't appear if there are no comments on a story, so we need to be flexible
        let num_comments = if let Some(comments) = find_first(p, article, ".comment-bubble") {
            comments
                .inner_text(p)
                .parse()
                .map_err(|e| "Failed to parse number of comments")?
        } else {
            0
        };

        let topics = find_first(p, article, ".topic").ok_or(format!("Mising topics"))?;
        let mut tags = vec![];
        for topic in html_tag_iterator(p, topics.query_selector(p, "img")) {
            tags.push(
                get_attribute(p, topic, "title")
                    .ok_or("Missing title on topic")?
                    .to_ascii_lowercase(),
            );
        }

        Ok(SlashdotStory {
            date: StoryDate::now(),
            id,
            num_comments,
            tags,
            title,
            url,
        })
    }
}

impl Scraper<SlashdotArgs, SlashdotStory> for SlashdotScraper {
    fn scrape(
        &self,
        args: SlashdotArgs,
        input: String,
    ) -> Result<(Vec<SlashdotStory>, Vec<String>), super::ScrapeError> {
        let dom = tl::parse(&input, ParserOptions::default())?;
        let p = dom.parser();
        let mut errors = vec![];
        let mut v = vec![];

        for article in html_tag_iterator(p, dom.query_selector("article.article")) {
            match Self::map_story(p, article) {
                Ok(s) => v.push(s),
                Err(e) => errors.push(e),
            }
        }

        Ok((v, errors))
    }
}

#[cfg(test)]
pub mod test {
    use super::super::test::*;
    use super::*;

    pub fn scrape_all() -> Vec<SlashdotStory> {
        let mut all = vec![];
        let scraper = SlashdotScraper::default();
        for file in slashdot_files() {
            let stories = scraper
                .scrape(SlashdotArgs::default(), load_file(file))
                .expect(&format!("Failed to parse a story from {}", file));
            all.extend(stories.0);
        }
        all
    }

    #[test]
    fn test_parse_sample() {
        let scraper = SlashdotScraper::default();
        for file in slashdot_files() {
            let stories = scraper
                .scrape(SlashdotArgs::default(), load_file(file))
                .unwrap();
            for error in stories.1 {
                println!("{}", error);
            }
            for story in stories.0 {
                println!(
                    "[{}] {} ({}) c={} t={:?}",
                    story.id, story.title, story.url, story.num_comments, story.tags
                );
            }
        }
    }
}
