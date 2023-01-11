use itertools::Itertools;
use serde::{Deserialize, Serialize};
use tl::{HTMLTag, Parser, ParserOptions};

use crate::story::{StoryDate, StoryUrl};

use super::{
    html::*, ScrapeConfigSource, ScrapeData, ScrapeDataInit, ScrapeId, ScrapeSource, ScrapeSource2,
    Scraper,
};

pub struct Slashdot {}

impl ScrapeSource2 for Slashdot {
    type Config = SlashdotConfig;
    type Scrape = SlashdotStory;
    type Scraper = SlashdotScraper;
    const TYPE: ScrapeSource = ScrapeSource::Slashdot;
}

#[derive(Default, Serialize, Deserialize)]
pub struct SlashdotConfig {
    homepage: String,
}

impl ScrapeConfigSource for SlashdotConfig {
    fn subsources(&self) -> Vec<String> {
        vec![]
    }

    fn provide_urls(&self, _: Vec<String>) -> Vec<String> {
        vec![self.homepage.clone()]
    }
}

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
        ScrapeId::new(ScrapeSource::Slashdot, None, self.id.clone())
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
pub struct SlashdotScraper {}

impl SlashdotScraper {
    fn parse_time(date: &str) -> Result<StoryDate, String> {
        // Expected input: 'on Monday January 09, 2023 @08:25PM'

        // Clean up "on " prefix, @ signs and commas
        let date = date
            .trim_start_matches("on ")
            .replace(['@', ','], "")
            ;

        // Expected at point: 'Monday January 09 2023 08:25PM'

        // https://docs.rs/chrono/latest/chrono/format/strftime/index.html
        let day_of_week = ["%A ", ""];
        let day = ["%d", "%e"];
        let am_pm = ["%p", "%P"];

        // Attempt to use multiple patterns to parse
        for ((day_of_week, day), am_pm) in day_of_week
            .iter()
            .cartesian_product(day)
            .cartesian_product(am_pm)
        {
            let pattern = format!("{}%B {} %Y %I:%M{}", day_of_week, day, am_pm);
            if let Some(date) = StoryDate::from_string(&date, &pattern) {
                return Ok(date);
            }
        }

        Err(format!("Failed to parse date: {}", date))
    }

    fn map_story(p: &Parser, article: &HTMLTag) -> Result<SlashdotStory, String> {
        let title = find_first(p, article, ".story-title").ok_or("Missing .story-title")?;
        let mut links = html_tag_iterator(p, title.query_selector(p, "a"));
        let story_link = links.next().ok_or("Missing story link")?;
        let title = story_link.inner_text(p).to_string();
        if title.len() < 5 {
            return Err(format!("Title was too short: {}", title));
        }
        let story_url =
            get_attribute(p, story_link, "href").ok_or("Missing story href".to_string())?;
        let (_, b) = story_url
            .split_once("/story/")
            .ok_or(format!("Invalid link format: {}", story_url))?;
        let id = b.splitn(5, '/').take(4).collect::<Vec<_>>();
        if id.len() != 4 {
            return Err(format!("Invalid link format: {}", story_url));
        }
        let id = id.join("/");

        let external_link = links.next().ok_or("Missing external link")?;
        let href = get_attribute(p, external_link, "href").ok_or("Missing href".to_string())?;
        let url = StoryUrl::parse(&href).ok_or(format!("Invalid href: {}", href))?;

        // This doesn't appear if there are no comments on a story, so we need to be flexible
        let num_comments = if let Some(comments) = find_first(p, article, ".comment-bubble") {
            comments
                .inner_text(p)
                .parse()
                .map_err(|_e| "Failed to parse number of comments")?
        } else {
            0
        };

        let topics = find_first(p, article, ".topic").ok_or("Mising topics".to_string())?;
        let mut tags = vec![];
        for topic in html_tag_iterator(p, topics.query_selector(p, "img")) {
            tags.push(
                get_attribute(p, topic, "title")
                    .ok_or("Missing title on topic")?
                    .to_ascii_lowercase(),
            );
        }

        let date = find_first(p, article, "time").ok_or("Could not locate time".to_string())?;
        let date = Self::parse_time(&date.inner_text(p))?;

        Ok(SlashdotStory {
            date,
            id,
            num_comments,
            tags,
            title,
            url,
        })
    }
}

impl Scraper<SlashdotConfig, SlashdotStory> for SlashdotScraper {
    fn scrape(
        &self,
        _args: &SlashdotConfig,
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
    use rstest::*;

    pub fn scrape_all() -> Vec<SlashdotStory> {
        let mut all = vec![];
        let scraper = SlashdotScraper::default();
        for file in slashdot_files() {
            let stories = scraper
                .scrape(&SlashdotConfig::default(), load_file(file))
                .unwrap_or_else(|_| panic!("Failed to parse a story from {}", file));
            all.extend(stories.0);
        }
        all
    }

    #[rstest]
    #[case("on Monday January 09, 2023 @08:25PM")]
    #[case("on Wednesday January 1, 2020 @11:00AM")]
    #[case("on Wednesday January 1, 2020 @12:00AM")]
    #[case("on Wednesday January 1, 2020 @12:30PM")]
    #[case("on January 1, 2020 @12:30PM")]
    fn test_date_parse(#[case] s: &str) {
        SlashdotScraper::parse_time(s).expect("Expected this to parse");
    }

    #[test]
    fn test_parse_sample() {
        let scraper = SlashdotScraper::default();
        for file in slashdot_files() {
            let stories = scraper
                .scrape(&SlashdotConfig::default(), load_file(file))
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
