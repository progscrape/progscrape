use std::{fmt::Debug, ops::RangeInclusive};

use progscrape_scrapers::StoryDate;
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Shard(u16);

impl ToString for Shard {
    fn to_string(&self) -> String {
        format!("{:04}-{:02}", self.0 / 12, self.0 % 12 + 1)
    }
}

impl Debug for Shard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{:04}-{:02}", self.0 / 12, self.0 % 12 + 1))
    }
}

impl Default for Shard {
    fn default() -> Self {
        Shard::from_year_month(2000, 1)
    }
}

impl std::ops::Add<u16> for Shard {
    type Output = Shard;
    fn add(self, rhs: u16) -> Self::Output {
        Shard(self.0 + rhs)
    }
}

impl std::ops::Sub<u16> for Shard {
    type Output = Shard;
    fn sub(self, rhs: u16) -> Self::Output {
        Shard(self.0 - rhs)
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub enum ShardOrder {
    OldestFirst,
    NewestFirst,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ShardRange {
    range: Option<(Shard, Shard)>,
}

impl ShardRange {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn new_from(range: RangeInclusive<Shard>) -> Self {
        Self {
            range: Some((*range.start(), *range.end())),
        }
    }

    pub fn iterate(&self, order: ShardOrder) -> impl Iterator<Item = Shard> {
        let (mut start, end) = self.range.unwrap_or((Shard::MAX, Shard::MIN));
        let orig_start = start;
        std::iter::from_fn(move || {
            if start > end {
                None
            } else {
                let next = Some(if order == ShardOrder::OldestFirst {
                    start
                } else {
                    Shard((end.0 - start.0) + orig_start.0)
                });
                start = start + 1;
                next
            }
        })
    }

    pub fn include(&mut self, shard: Shard) {
        if let Some(range) = self.range {
            self.range = Some((range.0.min(shard), range.1.max(shard)))
        } else {
            self.range = Some((shard, shard))
        }
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

#[cfg(test)]
mod test {
    use super::*;
    use itertools::Itertools;

    #[test]
    fn test_year_month() {
        let date = Shard::from_year_month(2000, 12);
        assert_eq!(Shard::from_year_month(2001, 1), date.plus_months(1));
        assert_eq!(Shard::from_year_month(2001, 12), date.plus_months(12));
        assert_eq!(Shard::from_year_month(1999, 12), date.sub_months(12));
        assert_eq!(Shard::from_year_month(2000, 1), date.sub_months(11));

        assert_eq!(
            date,
            Shard::from_string(&date.to_string()).expect("Failed to parse")
        );
    }

    #[test]
    fn test_shard_iterator() {
        let range = ShardRange::new_from(
            Shard::from_year_month(2000, 1)..=Shard::from_year_month(2000, 12),
        );
        assert_eq!(range.iterate(ShardOrder::OldestFirst).count(), 12);
        assert_eq!(range.iterate(ShardOrder::NewestFirst).count(), 12);

        let in_order = range.iterate(ShardOrder::OldestFirst).collect_vec();
        let mut rev_order = range.iterate(ShardOrder::NewestFirst).collect_vec();
        rev_order.reverse();

        assert_eq!(in_order, rev_order);
    }
}
