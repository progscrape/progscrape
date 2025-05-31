use std::{
    collections::hash_map::DefaultHasher,
    fmt::Display,
    hash::{Hash, Hasher},
};

use serde::{Deserialize, Serialize};
use url::Url;
use urlnorm::UrlNormalizer;

lazy_static::lazy_static! {
    static ref URL_NORMALIZER: UrlNormalizer = UrlNormalizer::default();
}

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
        // The StoryUrl can be either a tuple with the underlying bits, or a raw URL that we need to parse
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum StoryUrlSerializationOptions {
            Raw(String),
            Bits((String, String, String)),
        }

        let res: Result<StoryUrlSerializationOptions, D::Error> =
            Deserialize::deserialize(deserializer);
        match res {
            Ok(StoryUrlSerializationOptions::Raw(raw)) => StoryUrl::parse(&raw).ok_or(
                serde::de::Error::custom(format!("Failed to parse URL '{raw}'")),
            ),
            Ok(StoryUrlSerializationOptions::Bits((url, host, norm))) => Ok(StoryUrl {
                url,
                host,
                norm_str: StoryUrlNorm { norm },
            }),
            Err(e) => Err(e),
        }
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
            if let Some(host) = URL_NORMALIZER.normalize_host(&url) {
                let host = host.to_owned();
                let norm_str = StoryUrlNorm {
                    norm: URL_NORMALIZER.compute_normalization_string(&url),
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

        hasher.finish() as i64
    }

    pub fn string(&self) -> &str {
        &self.norm
    }
}
