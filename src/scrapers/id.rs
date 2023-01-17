use serde::{Deserialize, Serialize};
use std::{
    fmt::{Debug, Display},
    marker::PhantomData,
};

use super::ScrapeSource;

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
                Ok(ScrapeId::new(
                    source,
                    Some(subsource.to_owned()),
                    id.to_owned(),
                ))
            } else {
                Ok(ScrapeId::new(source, None, rest.to_owned()))
            }
        } else {
            Err(serde::de::Error::custom("Invalid format"))
        }
    }
}
