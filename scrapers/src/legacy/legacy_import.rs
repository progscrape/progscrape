use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter},
};

use flate2::bufread::GzDecoder;
use serde_json::Value;

use super::{
    hacker_news::{self},
    lobsters::{self},
    reddit::{self},
    TypedScrape,
};
use crate::scrapers::html::unescape_entities;
use crate::story::StoryDate;
use crate::story::StoryUrl;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LegacyError {
    #[error("I/O error")]
    IOError(#[from] std::io::Error),
    #[error("UTF8 error")]
    UTF8Error(#[from] std::string::FromUtf8Error),
    #[error("JSON error")]
    JSONError(#[from] serde_json::Error),
    #[error("Field was missing or invalid")]
    MissingField,
    #[error("Field {0} was missing or invalid ({1:?})")]
    InvalidField(&'static str, Option<String>),
    #[error("CBOR error")]
    CBORError(#[from] serde_cbor::Error),
}

fn make_hacker_news(
    id: String,
    title: String,
    url: StoryUrl,
    date: StoryDate,
) -> hacker_news::HackerNewsStory {
    hacker_news::HackerNewsStory {
        id,
        title,
        url,
        date,
        comments: Default::default(),
        points: Default::default(),
        position: Default::default(),
    }
}

fn make_reddit(id: String, title: String, url: StoryUrl, date: StoryDate) -> reddit::RedditStory {
    reddit::RedditStory {
        id,
        title,
        url,
        date,
        downvotes: Default::default(),
        flair: Default::default(),
        num_comments: Default::default(),
        position: Default::default(),
        score: Default::default(),
        subreddit: Default::default(),
        upvote_ratio: Default::default(),
        upvotes: Default::default(),
    }
}

fn make_lobsters(
    id: String,
    title: String,
    url: StoryUrl,
    date: StoryDate,
) -> lobsters::LobstersStory {
    lobsters::LobstersStory {
        id,
        title,
        url,
        date,
        num_comments: Default::default(),
        position: Default::default(),
        score: Default::default(),
        tags: Default::default(),
    }
}

fn import_legacy_1() -> Result<impl Iterator<Item = TypedScrape>, LegacyError> {
    let f = BufReader::new(File::open("import/old.json.gz")?);
    let mut decoder = BufReader::new(GzDecoder::new(f));
    let mut out = vec![];
    loop {
        let mut buf = vec![];
        let read = decoder.read_until(b'\n', &mut buf)?;
        if read == 0 {
            break;
        }
        let json = String::from_utf8(buf)?;
        let root: Value = serde_json::from_str(&json)?;
        let date = root["date"].as_str().ok_or(LegacyError::MissingField)?;
        let date = StoryDate::from_string(date, "%Y-%m-%d %H:%M:%S%.3f")
            .ok_or(LegacyError::MissingField)?;
        let title = unescape_entities(root["title"].as_str().ok_or(LegacyError::MissingField)?);
        let mut url = unescape_entities(root["url"].as_str().ok_or(LegacyError::MissingField)?);
        if url.contains("&amp") {
            url = url
                .split_once('?')
                .ok_or(LegacyError::MissingField)?
                .0
                .to_owned();
            tracing::info!("Fixed up: {}", url);
        }
        if url.ends_with(" /") {
            let old = url.clone();
            url = url.replace(' ', "");
            tracing::info!("Fixed up (removed spaces): {}->{}", old, url);
        }
        let url = StoryUrl::parse(&url).ok_or(LegacyError::InvalidField("url", Some(url)))?;
        let id = root["redditProgId"].as_str().unwrap_or("None").to_owned();
        if id != "None" {
            out.push(make_reddit(id, title.clone(), url.clone(), date).into());
        }
        let id = root["redditTechId"].as_str().unwrap_or("None").to_owned();
        if id != "None" {
            out.push(make_reddit(id, title.clone(), url.clone(), date).into());
        }
        let id = root["hackerNewsId"].as_str().unwrap_or("None").to_owned();
        if id != "None" {
            out.push(make_hacker_news(id, title.clone(), url.clone(), date).into());
        }
    }

    Ok(out.into_iter())
}

fn import_legacy_2() -> Result<impl Iterator<Item = TypedScrape>, LegacyError> {
    let f = BufReader::new(File::open("import/stories-progscrape-hr.gz")?);
    let mut decoder = BufReader::new(GzDecoder::new(f));
    let mut out = vec![];
    'outer: loop {
        let mut buf = vec![];
        while !buf.ends_with("}\n".as_bytes()) {
            let read = decoder.read_until(b'\n', &mut buf)?;
            if read == 0 {
                break 'outer;
            }
        }
        let json = String::from_utf8(buf)?;
        let root: Value = serde_json::from_str(&json)?;
        let date = StoryDate::from_millis(root["date"].as_i64().ok_or(LegacyError::MissingField)?)
            .ok_or(LegacyError::MissingField)?;
        let title = unescape_entities(root["title"].as_str().ok_or(LegacyError::MissingField)?);
        let mut url = unescape_entities(root["url"].as_str().ok_or(LegacyError::MissingField)?);
        if url.contains("&amp") {
            url = url
                .split_once('?')
                .ok_or(LegacyError::MissingField)?
                .0
                .to_owned();
            tracing::info!("Fixed up: {}", url);
        }
        let url = StoryUrl::parse(&url).ok_or(LegacyError::InvalidField("url", Some(url)))?;
        let id = root["hn"].as_str().unwrap_or("None").to_owned();
        if id != "None" {
            out.push(make_hacker_news(id, title.clone(), url.clone(), date).into());
        }
        let mut reddit = root["reddit"]
            .as_array()
            .ok_or(LegacyError::MissingField)?
            .clone();
        while let Some(value) = reddit.pop() {
            let id = value.as_str().unwrap_or("None").to_owned();
            if id != "None" {
                out.push(make_reddit(id, title.clone(), url.clone(), date).into());
            }
        }
        let id = root["lobsters"].as_str().unwrap_or("None").to_owned();
        if id != "None" {
            out.push(make_lobsters(id, title.clone(), url.clone(), date).into());
        }
    }
    Ok(out.into_iter())
}

pub fn import_legacy() -> Result<Vec<TypedScrape>, LegacyError> {
    let cache_file = "target/legacycache.bin";
    if let Ok(f) = File::open(cache_file) {
        tracing::info!("Reading cache '{}'...", cache_file);
        if let Ok(value) = serde_cbor::from_reader::<Vec<_>, _>(BufReader::new(f)) {
            tracing::info!("Cache OK");
            return Ok(value);
        }
        tracing::info!("Cache not OK");
    }
    let _ = std::fs::remove_file(cache_file);
    let v: Vec<_> = import_legacy_1()?
        .chain(import_legacy_2()?)
        .collect::<Vec<_>>();
    let f = File::create(cache_file)?;
    serde_cbor::to_writer(BufWriter::new(f), &v)?;
    Ok(v)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_read_legacy_1() -> Result<(), Box<dyn std::error::Error>> {
        assert!(import_legacy_1()?.count() > 0);
        Ok(())
    }

    #[test]
    fn test_read_legacy_2() -> Result<(), Box<dyn std::error::Error>> {
        assert!(import_legacy_2()?.count() > 0);
        Ok(())
    }

    #[test]
    fn test_read_legacy_all() -> Result<(), Box<dyn std::error::Error>> {
        assert!(!import_legacy()?.is_empty());
        Ok(())
    }
}
