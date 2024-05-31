use progscrape_application::StoryRender;
use progscrape_scrapers::{StoryDate, StoryUrl, TypedScrapeMap};
use serde::{Deserialize, Serialize};

/// The older-style feed.json story. This will be replaced by a more modern
/// data model in the future.
#[derive(Serialize, Deserialize)]
pub struct FeedStory {
    date: String,
    href: String,
    title: String,
    tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reddit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hnews: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lobsters: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    slashdot: Option<String>,
}

impl FeedStory {
    /// Return the feed story's comment URLs in the modern [`TypedScrapeMap`] format.
    pub fn comment_urls(&self) -> TypedScrapeMap<Option<&str>> {
        TypedScrapeMap {
            hacker_news: self.hnews.as_deref(),
            slashdot: self.slashdot.as_deref(),
            lobsters: self.lobsters.as_deref(),
            reddit: self.reddit.as_deref(),
            feed: None,
            other: None,
        }
    }
}

impl From<StoryRender> for FeedStory {
    fn from(story: StoryRender) -> Self {
        let comments = story
            .sources
            .into_with_map(|_, id| id.map(|id| id.comments_url()));
        FeedStory {
            date: story.date.to_rfc3339(),
            href: story.url,
            title: story.title,
            tags: story.tags,
            reddit: comments.reddit,
            hnews: comments.hacker_news,
            lobsters: comments.lobsters,
            slashdot: comments.slashdot,
        }
    }
}

impl TryInto<StoryRender> for FeedStory {
    type Error = String;
    fn try_into(self) -> Result<StoryRender, Self::Error> {
        let sources = self.comment_urls().into_with_map_fallible(|source, url| {
            if let Some(url) = url {
                Ok::<_, String>(Some(
                    source
                        .id_from_comments_url(url)
                        .ok_or(format!("Invalid {source:?} URL"))?,
                ))
            } else {
                Ok(None)
            }
        })?;
        let url = StoryUrl::parse(self.href).ok_or("Invalid url")?;
        Ok(StoryRender {
            date: StoryDate::parse_from_rfc3339(&self.date).ok_or("Invalid date")?,
            url: url.to_string(),
            title: self.title,
            tags: self.tags,
            domain: url.host().to_string(),
            id: "".to_owned(),
            order: 0,
            score: 0.0,
            html: "".to_owned(),
            sources,
        })
    }
}

#[cfg(test)]
mod tests {
    use progscrape_application::StoryRender;
    use progscrape_scrapers::{ScrapeId, ScrapeSource, StoryDate, StoryUrl, TypedScrapeMap};

    use super::FeedStory;

    #[test]
    fn test_feed_story() {
        let mut sources = TypedScrapeMap::new();
        sources.hacker_news = Some(ScrapeId::new(
            ScrapeSource::HackerNews,
            None,
            "1".to_string(),
        ));
        sources.reddit = Some(ScrapeId::new(ScrapeSource::Reddit, None, "2".to_string()));
        sources.lobsters = Some(ScrapeId::new(ScrapeSource::Lobsters, None, "3".to_string()));
        sources.slashdot = Some(ScrapeId::new(ScrapeSource::Slashdot, None, "4".to_string()));
        let url = StoryUrl::parse("http://example.com").unwrap();
        let story = StoryRender {
            id: "".to_string(),
            date: StoryDate::year_month_day(2024, 1, 1).unwrap(),
            domain: "example.com".to_string(),
            order: 0,
            score: 0.0,
            sources,
            tags: vec!["a".to_string()],
            title: "Title".to_string(),
            url: url.to_string(),
            html: "".to_string(),
        };

        let feed_story: FeedStory = story.clone().into();
        let story2: StoryRender = feed_story.try_into().unwrap();
        assert_eq!(format!("{story:?}"), format!("{story2:?}"));
    }
}
