use std::{
    borrow::{Borrow, Cow},
    collections::HashSet,
    time::SystemTime,
};

use chrono::{DateTime, TimeZone, Utc};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use tl::{HTMLTag, Parser, ParserOptions};
use url::Url;

use crate::types::*;

use super::{
    scrape_story, utils::html::*, GenericScrape, ScrapeConfigSource, ScrapeCore, ScrapeSource,
    ScrapeSourceDef, ScrapeStory, Scraper,
};

pub struct Slashdot {}

impl ScrapeSourceDef for Slashdot {
    type Config = SlashdotConfig;
    type Scrape = SlashdotStory;
    type Scraper = SlashdotScraper;

    fn comments_url(id: &str, _subsource: Option<&str>) -> String {
        format!("https://tech.slashdot.org/story/{}/", id)
    }

    fn id_from_comments_url(url: &str) -> Option<(&str, Option<&str>)> {
        let url = url.trim_end_matches('/');
        Some((url.strip_prefix("https://tech.slashdot.org/story/")?, None))
    }

    fn is_comments_host(host: &str) -> bool {
        host.ends_with("slashdot.org")
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct SlashdotConfig {
    homepage: String,
    tag_allowlist: HashSet<String>,
}

impl ScrapeConfigSource for SlashdotConfig {
    fn subsources(&self) -> Vec<String> {
        vec![]
    }

    fn provide_urls(&self, _: Vec<String>) -> Vec<String> {
        vec![self.homepage.clone()]
    }
}

scrape_story! {
    SlashdotStory {
        num_comments: u32,
        tags: Vec<String>,
    }
}

impl ScrapeStory for SlashdotStory {
    const TYPE: ScrapeSource = ScrapeSource::Slashdot;

    fn merge(&mut self, other: Self) {
        self.num_comments = std::cmp::max(self.num_comments, other.num_comments);
    }
}

#[derive(Default)]
pub struct SlashdotScraper {}

impl SlashdotScraper {
    fn parse_time(date: &str) -> Result<StoryDate, String> {
        // Slashdot runs in EST5EDT (according to the latest slashcode release) if you
        // are logged out. If we wanted to be more accurate, we could scrape the times from the
        // RSS feed and correlate with the website but we're going to make do here instead.

        let tz = chrono_tz::US::Eastern;
        let res = tz.from_utc_datetime(&DateTime::<Utc>::from(SystemTime::now()).naive_utc());

        // Clean up "on " prefix, @ signs and commas, then add the offset
        let date = format!(
            "{} {}",
            date.trim_start_matches("on ").replace(['@', ','], ""),
            res.format("%z")
        );

        // Expected at point: 'Monday January 09 2023 08:25PM -0500'

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
            let pattern = format!("{}%B {} %Y %I:%M{} %z", day_of_week, day, am_pm);
            if let Some(date) = StoryDate::from_string(&date, &pattern) {
                return Ok(date);
            }
        }

        Err(format!("Failed to parse date: {}", date))
    }

    fn parse_topic(href: &str) -> Option<String> {
        let base = Url::parse("https://slashdot.org").expect("Failed to parse base URL");
        let url = base.join(href);
        if let Ok(url) = url {
            if let Some((_, value)) = url.query_pairs().find(|(k, _)| k == "fhfilter") {
                return Some(value.into());
            }
        }
        None
    }

    fn map_story(
        p: &Parser,
        article: &HTMLTag,
    ) -> Result<GenericScrape<<Self as Scraper>::Output>, String> {
        let title = find_first(p, article, ".story-title").ok_or("Missing .story-title")?;
        let mut links = html_tag_iterator(p, title.query_selector(p, "a"));
        let story_link = links.next().ok_or("Missing story link")?;
        let raw_title = unescape_entities(story_link.inner_text(p).borrow());
        if raw_title.len() < 5 {
            return Err(format!("Title was too short: {}", raw_title));
        }
        let story_url =
            get_attribute(p, story_link, "href").ok_or_else(|| "Missing story href".to_string())?;
        let (_, b) = story_url
            .split_once("/story/")
            .ok_or(format!("Invalid link format: {}", story_url))?;
        let id = b.splitn(5, '/').take(4).collect::<Vec<_>>();
        if id.len() != 4 {
            return Err(format!("Invalid link format: {}", story_url));
        }
        let id = id.join("/");

        let external_link = links.next().ok_or("Missing external link")?;
        let href = unescape_entities(
            &get_attribute(p, external_link, "href").ok_or_else(|| "Missing href".to_string())?,
        );
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

        let topics = find_first(p, article, ".topic").ok_or_else(|| "Mising topics".to_string())?;
        let mut tags = vec![];
        for topic in html_tag_iterator(p, topics.query_selector(p, "a")) {
            if let Some(topic_href) = get_attribute(p, topic, "href") {
                if let Some(topic) = Self::parse_topic(&topic_href) {
                    tags.push(topic);
                }
            }
        }

        let date =
            find_first(p, article, "time").ok_or_else(|| "Could not locate time".to_string())?;
        let date = Self::parse_time(&date.inner_text(p))?;

        Ok(SlashdotStory::new(
            id,
            date,
            raw_title,
            url,
            num_comments,
            tags,
        ))
    }
}

impl Scraper for SlashdotScraper {
    type Config = <Slashdot as ScrapeSourceDef>::Config;
    type Output = <Slashdot as ScrapeSourceDef>::Scrape;

    fn scrape(
        &self,
        _args: &Self::Config,
        input: &str,
    ) -> Result<(Vec<GenericScrape<Self::Output>>, Vec<String>), ScrapeError> {
        let dom = tl::parse(input, ParserOptions::default())?;
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

    fn extract_core<'a>(
        &self,
        args: &Self::Config,
        input: &'a GenericScrape<Self::Output>,
    ) -> ScrapeCore<'a> {
        let mut tags = vec![];
        for tag in &input.data.tags {
            if args.tag_allowlist.contains(tag) {
                tags.push(Cow::Borrowed(tag.as_str()));
            }
        }

        ScrapeCore {
            source: &input.shared.id,
            date: input.shared.date,
            title: Cow::Borrowed(&input.shared.raw_title),
            url: &input.shared.url,
            rank: None,
            tags,
        }
    }
}

#[cfg(test)]
pub mod test {
    use super::*;
    use rstest::*;

    #[rstest]
    #[case("on Monday January 09, 2023 @08:25PM")]
    #[case("on Wednesday January 1, 2020 @11:00AM")]
    #[case("on Wednesday January 1, 2020 @12:00AM")]
    #[case("on Wednesday January 1, 2020 @12:30PM")]
    #[case("on January 1, 2020 @12:30PM")]
    fn test_date_parse(#[case] s: &str) {
        SlashdotScraper::parse_time(s).expect("Expected this to parse");
    }

    /// Test that we can extract the fhfilter tag in all cases.
    #[rstest]
    #[case("https://slashdot.org/index2.pl?fhfilter=business", Some("business"))]
    #[case("//slashdot.org/index2.pl?fhfilter=business", Some("business"))]
    #[case(
        "//web.archive.org/web/20180313000356/https://slashdot.org/index2.pl?fhfilter=business",
        Some("business")
    )]
    fn test_extract_topic(#[case] a: &str, #[case] b: Option<&str>) {
        assert_eq!(b.map(String::from), SlashdotScraper::parse_topic(a));
    }
}
