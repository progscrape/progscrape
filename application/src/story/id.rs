use serde::{Deserialize, Serialize};

use std::fmt::Display;

use progscrape_scrapers::{StoryDate, StoryUrlNorm};

use crate::Shard;

/// Uniquely identifies a story within the index.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
pub struct StoryIdentifier {
    pub norm: StoryUrlNorm,
    date: (u16, u8, u8),
}

impl Display for StoryIdentifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "{}:{}:{}:{}",
            self.date.0,
            self.date.1,
            self.date.2,
            self.norm.string()
        ))
    }
}

impl StoryIdentifier {
    const BASE64_CONFIG: base64::engine::GeneralPurpose =
        base64::engine::general_purpose::URL_SAFE_NO_PAD;

    pub fn new(date: StoryDate, norm: &StoryUrlNorm) -> Self {
        Self {
            norm: norm.clone(),
            date: (date.year() as u16, date.month() as u8, date.day() as u8),
        }
    }

    pub fn update_date(&mut self, date: StoryDate) {
        self.date = (date.year() as u16, date.month() as u8, date.day() as u8);
    }

    pub fn matches_date(&self, date: StoryDate) -> bool {
        (self.date.0, self.date.1, self.date.2)
            == (date.year() as u16, date.month() as u8, date.day() as u8)
    }

    pub fn to_base64(&self) -> String {
        use base64::Engine;
        Self::BASE64_CONFIG.encode(self.to_string().as_bytes())
    }

    pub fn from_base64<T: AsRef<[u8]>>(s: T) -> Option<Self> {
        // Use an inner function so we can make use of ? (is there an easier way?)
        fn from_base64_res<T: AsRef<[u8]>>(s: T) -> Result<StoryIdentifier, ()> {
            use base64::Engine;
            let s = StoryIdentifier::BASE64_CONFIG.decode(s).map_err(drop)?;
            let s = String::from_utf8(s).map_err(drop)?;
            let mut bits = s.splitn(4, ':');
            let year = bits.next().ok_or(())?;
            let month = bits.next().ok_or(())?;
            let day = bits.next().ok_or(())?;
            let norm = bits.next().ok_or(())?.to_owned();
            Ok(StoryIdentifier {
                norm: StoryUrlNorm::from_string(norm),
                date: (
                    year.parse().map_err(drop)?,
                    month.parse().map_err(drop)?,
                    day.parse().map_err(drop)?,
                ),
            })
        }

        from_base64_res(s).ok()
    }

    pub fn shard(&self) -> Shard {
        Shard::from_year_month(self.year(), self.month())
    }

    fn year(&self) -> u16 {
        self.date.0
    }

    fn month(&self) -> u8 {
        self.date.1
    }

    fn day(&self) -> u8 {
        self.date.2
    }
}

#[cfg(test)]
mod test {
    use crate::story::{StoryDate, StoryUrl};

    use super::*;

    #[test]
    fn test_story_identifier() {
        let url = StoryUrl::parse("https://google.com/?q=foo").expect("Failed to parse URL");
        let id = StoryIdentifier::new(StoryDate::now(), url.normalization());
        let base64 = id.to_base64();
        assert_eq!(
            id,
            StoryIdentifier::from_base64(base64).expect("Failed to decode ID")
        );
    }
}
