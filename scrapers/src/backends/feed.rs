use std::borrow::Cow;

use itertools::Itertools;
use serde::{Deserialize, Serialize};

use crate::{ScrapeCore, ScrapeSource};

use super::{
    scrape_story, GenericScrape, ScrapeConfigSource, ScrapeSourceDef, ScrapeStory, Scraper,
};

pub struct Feed {}

impl ScrapeSourceDef for Feed {
    type Config = FeedConfig;
    type Scrape = FeedStory;
    type Scraper = FeedScraper;

    fn comments_url(id: &str, subsource: Option<&str>) -> String {
        "".to_string()
    }

    fn id_from_comments_url(url: &str) -> Option<(&str, Option<&str>)> {
        None
    }

    fn is_comments_host(host: &str) -> bool {
        false
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct FeedConfig {}

impl ScrapeConfigSource for FeedConfig {
    fn provide_urls(&self, subsources: Vec<String>) -> Vec<String> {
        vec![]
    }

    fn subsources(&self) -> Vec<String> {
        vec![]
    }
}

scrape_story! {
    FeedStory {
        tags: Vec<String>
    }
}

impl ScrapeStory for FeedStory {
    const TYPE: ScrapeSource = ScrapeSource::Feed;

    fn merge(&mut self, other: Self) {}
}

#[derive(Default)]
pub struct FeedScraper {}

impl Scraper for FeedScraper {
    type Config = <Feed as ScrapeSourceDef>::Config;
    type Output = <Feed as ScrapeSourceDef>::Scrape;

    fn extract_core<'a>(
        &self,
        args: &Self::Config,
        input: &'a super::GenericScrape<Self::Output>,
    ) -> ScrapeCore<'a> {
        let tags = input
            .data
            .tags
            .iter()
            .map(|tag| Cow::Borrowed(tag.as_str()))
            .collect_vec();
        ScrapeCore {
            source: &input.shared.id,
            title: Cow::Borrowed(&input.shared.raw_title),
            url: &input.shared.url,
            date: input.shared.date,
            rank: None,
            tags,
        }
    }

    fn scrape(
        &self,
        args: &Self::Config,
        input: &str,
    ) -> Result<(Vec<super::GenericScrape<Self::Output>>, Vec<String>), crate::ScrapeError> {
        unimplemented!()
    }
}
