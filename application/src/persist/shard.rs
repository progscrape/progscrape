use std::ops::Range;

use progscrape_scrapers::StoryDate;
use serde::{Serialize, Deserialize};

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Shard(u16);

impl ToString for Shard {
    fn to_string(&self) -> String {
        format!("{:04}-{:02}", self.0 / 12, self.0 % 12 + 1)
    }
}

impl Default for Shard {
    fn default() -> Self {
        Shard::from_year_month(2000, 1)
    }
}

pub trait ShardRange {
    fn rev(&self) -> Vec<Shard>;
}

impl ShardRange for Range<Shard> {
    fn rev(&self) -> Vec<Shard> {
        let mut v = vec![];
        for shard in self.start.0..self.end.0 {
            v.push(Shard(shard));
        }
        v.reverse();
        v
    }
}

impl Shard {
    pub const MIN: Shard = Shard(u16::MIN);
    pub const MAX: Shard = Shard(u16::MAX);

    pub fn from_year_month(year: u16, month: u8) -> Self {
        assert!(month > 0);
        Shard(year * 12 + month as u16 - 1)
    }

    pub fn from_string(s: &str) -> Option<Self> {
        if let Some((a, b)) = s.split_once('-') {
            if let (Ok(a), Ok(b)) = (str::parse(a), str::parse(b)) {
                return Some(Self::from_year_month(a, b));
            }
        }
        None
    }

    pub fn from_date_time(date: StoryDate) -> Self {
        Self::from_year_month(date.year() as u16, date.month() as u8)
    }

    pub fn plus_months(&self, months: i8) -> Self {
        let ordinal = self.0 as i16 + months as i16;
        Self(ordinal as u16)
    }

    pub fn sub_months(&self, months: i8) -> Self {
        self.plus_months(-months)
    }
}
