use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

use super::export::*;
use super::{GenericScrape, TypedScrape};
use crate::types::*;
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

/// Import a backup-formatted JSON file, which is a JSON file of `TypedScrape` object, separated by newlines.
pub fn import_backup(file: &Path) -> Result<Vec<TypedScrape>, LegacyError> {
    let mut f = BufReader::new(File::open(file)?);
    let mut out: Vec<TypedScrape> = vec![];
    'outer: loop {
        let mut buf = vec![];
        while !buf.ends_with("}\n".as_bytes()) {
            let read = f.read_until(b'\n', &mut buf)?;
            if read == 0 {
                break 'outer;
            }
        }
        let json = String::from_utf8(buf)?;
        let scrape = serde_json::from_str(&json)?;
        out.push(scrape);
    }

    Ok(out)
}
