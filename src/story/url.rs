use std::{
    collections::hash_map::DefaultHasher,
    fmt::Display,
    hash::{Hash, Hasher},
};

use serde::{Deserialize, Serialize};
use url::Url;

use crate::datasci::urlnormalizer::url_normalization_string;

/// Story-specific URL that caches the normalization information and other important parts of the URL.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StoryUrl {
    url: String,
    host: String,
    norm_str: StoryUrlNorm,
}

impl Serialize for StoryUrl {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let tuple: (&String, &String, &String) = (&self.url, &self.host, &self.norm_str.norm);
        tuple.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for StoryUrl {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let res: Result<(String, String, String), D::Error> =
            Deserialize::deserialize(deserializer);
        res.map(|(url, host, norm)| StoryUrl {
            url,
            host,
            norm_str: StoryUrlNorm { norm },
        })
    }
}

impl Display for StoryUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.url.fmt(f)
    }
}

impl StoryUrl {
    pub fn parse<S: AsRef<str>>(s: S) -> Option<Self> {
        if let Ok(url) = Url::parse(s.as_ref()) {
            if let Some(host) = url.host_str() {
                let host = host.to_owned();
                let norm_str = StoryUrlNorm {
                    norm: url_normalization_string(&url),
                };
                let url = url.into();
                return Some(Self {
                    url,
                    host,
                    norm_str,
                });
            }
        }
        None
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn raw(&self) -> &str {
        &self.url
    }

    pub fn normalization(&self) -> &StoryUrlNorm {
        &self.norm_str
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StoryUrlNorm {
    norm: String,
}

impl Serialize for StoryUrlNorm {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.norm.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for StoryUrlNorm {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let res: Result<String, _> = Deserialize::deserialize(deserializer);
        res.map(|norm| StoryUrlNorm { norm })
    }
}

impl StoryUrlNorm {
    /// Re-create a story norm, if you know what you're doing.
    pub fn from_string(norm: String) -> Self {
        Self { norm }
    }

    pub fn hash(&self) -> i64 {
        let mut hasher = DefaultHasher::new();
        self.norm.hash(&mut hasher);
        let url_norm_hash = hasher.finish() as i64;
        return url_norm_hash;
    }

    pub fn string(&self) -> &str {
        &self.norm
    }
}
