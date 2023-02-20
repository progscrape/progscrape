use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter},
    path::Path,
};

use flate2::bufread::GzDecoder;
use serde_json::Value;

use super::export::*;
use super::utils::html::unescape_entities;
use super::{GenericScrape, TypedScrape};
use crate::{
    hacker_news::HackerNewsStory, lobsters::LobstersStory, reddit::RedditStory,
    slashdot::SlashdotStory, types::*,
};
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
    raw_title: String,
    url: StoryUrl,
    date: StoryDate,
) -> GenericScrape<hacker_news::HackerNewsStory> {
    hacker_news::HackerNewsStory::new_with_defaults(id, date, raw_title, url)
}

fn make_reddit(
    id: String,
    raw_title: String,
    url: StoryUrl,
    date: StoryDate,
) -> GenericScrape<reddit::RedditStory> {
    reddit::RedditStory::new_with_defaults(id, date, raw_title, url)
}

fn make_lobsters(
    id: String,
    raw_title: String,
    url: StoryUrl,
    date: StoryDate,
) -> GenericScrape<lobsters::LobstersStory> {
    lobsters::LobstersStory::new_with_defaults(id, date, raw_title, url)
}

fn import_legacy_1(root: &Path) -> Result<impl Iterator<Item = TypedScrape>, LegacyError> {
    let f = BufReader::new(File::open(root.join("scrapers/import/old.json.gz"))?);
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

fn import_legacy_2(root: &Path) -> Result<impl Iterator<Item = TypedScrape>, LegacyError> {
    let f = BufReader::new(File::open(
        root.join("scrapers/import/stories-progscrape-hr.gz"),
    )?);
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
        let mut title = unescape_entities(root["title"].as_str().ok_or(LegacyError::MissingField)?);
        for error in ["AT&T;", "P&G;", "S&P;", "Q&A;", "H&R;", "AT&To;", "C&C;"] {
            if title.contains(error) {
                let old_title = title;
                title = old_title.replace(error, &error[..error.len() - 1]);
                tracing::info!("Fixed up title: {} -> {}", old_title, title);
            }
        }
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

pub fn import_legacy_3(root: &Path) -> Result<impl Iterator<Item = TypedScrape>, LegacyError> {
    let file = root.join("scrapers/import/allstories.json.gz");
    if !file.exists() {
        return Ok(vec![].into_iter());
    }
    let f = BufReader::new(File::open(file)?);
    let mut decoder = BufReader::new(GzDecoder::new(f));
    let mut out: Vec<TypedScrape> = vec![];
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
        let date = StoryDate::from_millis(
            root["date"]
                .as_i64()
                .ok_or(LegacyError::InvalidField("date", None))?,
        )
        .ok_or(LegacyError::InvalidField("date", None))?;
        let mut url = unescape_entities(
            root["url"]
                .as_str()
                .ok_or(LegacyError::InvalidField("url", None))?,
        );
        if url.contains("Guix to GNU/Hurd") {
            // https://news.ycombinator.com/item?id=10090379
            // http://[GSoC update] Porting Guix to GNU/Hurd
            tracing::info!("Fixed up: {}", url);
            url = "https://lists.gnu.org/archive/html/guix-devel/2015-08/msg00379.html".into();
        }
        if url.ends_with("&amp") {
            tracing::info!("Fixed up: {}", url);
            url = url.strip_suffix("&amp").expect("Missing suffix").into();
        }
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
        let scrapes = root["scrapes"]
            .as_array()
            .ok_or(LegacyError::InvalidField("scrapes", None))?;
        for scrape in scrapes {
            let scrape: Value = serde_json::from_str(
                scrape
                    .as_str()
                    .ok_or(LegacyError::InvalidField("scrapes", None))?,
            )?;
            let source = scrape
                .get("source")
                .ok_or(LegacyError::InvalidField("source", None))?
                .as_str()
                .ok_or(LegacyError::InvalidField("source", None))?;
            let index = scrape
                .get("index")
                .ok_or(LegacyError::InvalidField("index", None))?
                .as_i64()
                .ok_or(LegacyError::InvalidField("index", None))?;
            let title = scrape
                .get("title")
                .ok_or(LegacyError::InvalidField("title", None))?
                .as_str()
                .ok_or(LegacyError::InvalidField("title", None))?;
            let id = scrape
                .get("id")
                .ok_or(LegacyError::InvalidField("id", None))?
                .as_str()
                .ok_or(LegacyError::InvalidField("id", None))?;
            let subcategory = if let Some(subcategory) = scrape.get("subcategory") {
                subcategory.as_str()
            } else {
                None
            };

            let story: TypedScrape = match source {
                "hackernews" => {
                    assert!(subcategory.is_none());
                    HackerNewsStory::new_with_defaults(id, date, title, url.clone()).into()
                }
                "lobsters" => {
                    assert!(subcategory.is_none());
                    LobstersStory::new_with_defaults(id, date, title, url.clone()).into()
                }
                "reddit.prog" | "reddit.tech" => {
                    if let Some(sub) = subcategory {
                        RedditStory::new_subsource_with_defaults(id, sub, date, title, url.clone())
                            .into()
                    } else {
                        RedditStory::new_with_defaults(id, date, title, url.clone()).into()
                    }
                }
                "slashdot" => {
                    assert!(subcategory.is_none());
                    SlashdotStory::new_with_defaults(id, date, title, url.clone()).into()
                }
                _ => {
                    return Err(LegacyError::InvalidField("source", Some(source.into())));
                }
            };
            out.push(story);
        }
    }
    Ok(out.into_iter())
}

pub fn import_legacy(root: &Path) -> Result<Vec<TypedScrape>, LegacyError> {
    let cache_file = root.to_owned().join("target/legacycache.bin");
    tracing::info!("Reading cache '{:?}'...", cache_file);
    if let Ok(f) = File::open(&cache_file) {
        if let Ok(value) = serde_cbor::from_reader::<Vec<_>, _>(BufReader::new(f)) {
            tracing::info!("Cache OK");
            return Ok(value);
        }
        tracing::info!("Cache not OK");
    }
    let _ = std::fs::remove_file(&cache_file);
    let v: Vec<_> = import_legacy_1(root)?
        .chain(import_legacy_2(root)?)
        .chain(import_legacy_3(root)?)
        .collect::<Vec<_>>();
    let f = File::create(&cache_file)?;
    serde_cbor::to_writer(BufWriter::new(f), &v)?;
    Ok(v)
}

#[cfg(test)]
mod test {
    use super::*;
    use rstest::*;

    #[test]
    fn test_read_legacy_1() -> Result<(), Box<dyn std::error::Error>> {
        assert!(import_legacy_1(Path::new(".."))?.count() > 0);
        Ok(())
    }

    #[test]
    fn test_read_legacy_2() -> Result<(), Box<dyn std::error::Error>> {
        assert!(import_legacy_2(Path::new(".."))?.count() > 0);
        Ok(())
    }

    #[test]
    fn test_read_legacy_3() -> Result<(), Box<dyn std::error::Error>> {
        import_legacy_3(Path::new(".."))?.count();
        Ok(())
    }

    #[test]
    fn test_read_legacy_all() -> Result<(), Box<dyn std::error::Error>> {
        assert!(!import_legacy(Path::new(".."))?.is_empty());
        Ok(())
    }
}
