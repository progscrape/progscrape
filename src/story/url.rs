use std::{fmt::Display, collections::hash_map::DefaultHasher, hash::{Hasher, Hash}};

use serde::{Deserialize, Serialize};
use url::Url;

use crate::datasci::urlnormalizer::url_normalization_string;

/// Story-specific URL that caches the normalization information and other important parts of the URL.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct StoryUrl {
    url: String,
    host: String,
    norm_str: StoryUrlNorm,
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
                let norm_str = StoryUrlNorm { norm: url_normalization_string(&url) };
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

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct StoryUrlNorm {
    norm: String,
}

impl StoryUrlNorm {
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
