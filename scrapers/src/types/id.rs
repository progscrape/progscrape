use serde::{Deserialize, Serialize};
use std::{
    fmt::{Debug, Display},
    marker::PhantomData,
};

use crate::{StoryUrl, backends::ScrapeSource};

/// Identify a scrape by source an ID.
#[derive(Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ScrapeId {
    pub source: ScrapeSource,
    pub subsource: Option<String>,
    pub id: String,
    _noinit: PhantomData<()>,
}

impl ScrapeId {
    pub fn new(source: ScrapeSource, subsource: Option<String>, id: String) -> Self {
        Self {
            source,
            subsource,
            id,
            _noinit: Default::default(),
        }
    }

    /// Given a URL, determines if that URL would make this story a self-post. The current heuristic for
    /// this is whether the url's host looks like a comments host, and the url itself contains the scrape's
    /// ID. The latter heuristic isn't perfect, but the failure modes are pretty harmless.
    pub fn is_likely_self_post(&self, url: &StoryUrl) -> bool {
        self.source.is_comments_host(url.host()) && url.raw().contains(&self.id)
    }

    /// Generate a comments URL for this scrape.
    pub fn comments_url(&self) -> String {
        self.source
            .comments_url(&self.id, self.subsource.as_deref())
    }

    pub fn from_string<S: AsRef<str>>(s: S) -> Option<Self> {
        if let Some((head, rest)) = s.as_ref().split_once('-') {
            if let Some(source) = ScrapeSource::try_from_str(head) {
                if let Some((subsource, id)) = rest.split_once('-') {
                    Some(source.subsource_id(subsource, id))
                } else {
                    Some(source.id(rest))
                }
            } else {
                None
            }
        } else {
            None
        }
    }
}

impl Display for ScrapeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(subsource) = &self.subsource {
            f.write_fmt(format_args!(
                "{}-{}-{}",
                self.source.into_str(),
                subsource,
                self.id
            ))
        } else {
            f.write_fmt(format_args!("{}-{}", self.source.into_str(), self.id))
        }
    }
}

impl Debug for ScrapeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        <Self as Display>::fmt(self, f)
    }
}

impl Serialize for ScrapeId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if let Some(subsource) = &self.subsource {
            format!("{}-{}-{}", self.source.into_str(), subsource, self.id)
        } else {
            format!("{}-{}", self.source.into_str(), self.id)
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ScrapeId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        if let Some((head, rest)) = s.split_once('-') {
            let source = ScrapeSource::try_from_str(head)
                .ok_or(serde::de::Error::custom("Invalid source"))?;
            if let Some((subsource, id)) = rest.split_once('-') {
                Ok(source.subsource_id(subsource, id))
            } else {
                Ok(source.id(rest))
            }
        } else {
            Err(serde::de::Error::custom("Invalid format"))
        }
    }
}
