use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter},
};

use flate2::bufread::GzDecoder;
use serde_json::Value;
use url::Url;

use super::{hacker_news::HackerNewsStory, lobsters::LobstersStory, Scrape, ScrapeData, Scraper};
use crate::scrapers::reddit_json::RedditStory;
use crate::scrapers::unescape_entities;
use crate::story::StoryDate;
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
    #[error("CBOR error")]
    CBORError(#[from] serde_cbor::Error),
}

fn import_legacy_1() -> Result<impl Iterator<Item = Scrape>, LegacyError> {
    let f = BufReader::new(File::open("import/old.json.gz")?);
    let mut decoder = BufReader::new(GzDecoder::new(f));
    let mut out = vec![];
    loop {
        let mut buf = vec![];
        let read = decoder.read_until('\n' as u8, &mut buf)?;
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
        if let Err(e) = Url::parse(&url) {
            tracing::error!("Bad URL: {}", url);
            continue;
        }
        let id = root["redditProgId"].as_str().unwrap_or("None").to_owned();
        if id != "None" {
            out.push(
                RedditStory {
                    id,
                    title: title.clone(),
                    url: url.clone(),
                    date: date.clone(),
                    ..Default::default()
                }
                .into(),
            );
        }
        let id = root["redditTechId"].as_str().unwrap_or("None").to_owned();
        if id != "None" {
            out.push(
                RedditStory {
                    id,
                    title: title.clone(),
                    url: url.clone(),
                    date: date.clone(),
                    ..Default::default()
                }
                .into(),
            );
        }
        let id = root["hackerNewsId"].as_str().unwrap_or("None").to_owned();
        if id != "None" {
            out.push(
                HackerNewsStory {
                    id,
                    title: title.clone(),
                    url: url.clone(),
                    date,
                    ..Default::default()
                }
                .into(),
            );
        }
    }

    Ok(out.into_iter())
}

fn import_legacy_2() -> Result<impl Iterator<Item = Scrape>, LegacyError> {
    let f = BufReader::new(File::open("import/stories-progscrape-hr.gz")?);
    let mut decoder = BufReader::new(GzDecoder::new(f));
    let mut out = vec![];
    'outer: loop {
        let mut buf = vec![];
        while !buf.ends_with("}\n".as_bytes()) {
            let read = decoder.read_until('\n' as u8, &mut buf)?;
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
        if let Err(e) = Url::parse(&url) {
            tracing::error!("Bad URL: {}", url);
            continue;
        }
        let id = root["hn"].as_str().unwrap_or("None").to_owned();
        if id != "None" {
            out.push(
                HackerNewsStory {
                    id,
                    title: title.clone(),
                    url: url.clone(),
                    date: date.clone(),
                    ..Default::default()
                }
                .into(),
            );
        }
        let mut reddit = root["reddit"]
            .as_array()
            .ok_or(LegacyError::MissingField)?
            .clone();
        while let Some(value) = reddit.pop() {
            let id = value.as_str().unwrap_or("None").to_owned();
            if id != "None" {
                out.push(
                    RedditStory {
                        id,
                        title: title.clone(),
                        url: url.clone(),
                        date: date.clone(),
                        ..Default::default()
                    }
                    .into(),
                );
            }
        }
        let id = root["lobsters"].as_str().unwrap_or("None").to_owned();
        if id != "None" {
            out.push(
                LobstersStory {
                    id,
                    title: title.clone(),
                    url: url.clone(),
                    date: date.clone(),
                    ..Default::default()
                }
                .into(),
            );
        }
    }
    Ok(out.into_iter())
}

pub fn import_legacy() -> Result<impl Iterator<Item = Scrape>, LegacyError> {
    let cache_file = "target/legacycache.bin";
    if let Ok(f) = File::open(cache_file) {
        tracing::info!("Reading cache '{}'...", cache_file);
        if let Ok(value) = serde_cbor::from_reader::<Vec<Scrape>, _>(BufReader::new(f)) {
            tracing::info!("Cache OK");
            return Ok(value.into_iter());
        }
        tracing::info!("Cache not OK");
    }
    let _ = std::fs::remove_file(cache_file);
    let mut v: Vec<_> = import_legacy_1()?
        .chain(import_legacy_2()?)
        .collect::<Vec<_>>();
    v.sort_by_cached_key(|story| story.date());
    let f = File::create(cache_file)?;
    serde_cbor::to_writer(BufWriter::new(f), &v)?;
    Ok(v.into_iter())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_read_legacy_1() {
        for story in import_legacy_1().expect("Failed to import legacy stories") {
            println!("{:?} {} {}", story.source(), story.title(), story.url());
        }
    }

    #[test]
    fn test_read_legacy_2() {
        for story in import_legacy_2().expect("Failed to import legacy stories") {
            println!("{:?} {} {}", story.source(), story.title(), story.url());
        }
    }

    #[test]
    fn test_read_legacy_all() {
        for story in import_legacy().expect("Failed to import legacy stories") {
            println!("{:?} {} {}", story.source(), story.title(), story.url());
        }
    }
}
