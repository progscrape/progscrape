use std::{
    hash::Hash,
    time::{Duration, Instant},
};

use bloom::{ASMS, CountingBloomFilter};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct RateLimitsConfig {
    pub enabled: bool,
    pub ip: BucketConfig,
    pub bot: BucketConfig,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct BucketConfig {
    pub hard: f32,
    pub hash: HashConfig,
    pub minute: u32,
    pub ten_minute: u32,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct HashConfig {
    pub item_count: u32,
    pub false_positive_rate: f32,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LimitState {
    None,
    Soft,
    Hard,
}

impl LimitState {
    fn max(self, other: Self) -> Self {
        if self == LimitState::Hard || other == LimitState::Hard {
            LimitState::Hard
        } else if self == LimitState::Soft || other == LimitState::Soft {
            LimitState::Soft
        } else {
            LimitState::None
        }
    }
}

struct Blooms {
    start: Instant,
    size: Duration,
    prev: CountingBloomFilter,
    curr: CountingBloomFilter,
    soft: u32,
    hard: u32,
}

impl Blooms {
    pub fn new(
        size: Duration,
        expected_num_items: u32,
        false_positive_rate: f32,
        soft: u32,
        hard: u32,
    ) -> Self {
        Self {
            start: Instant::now(),
            size,
            prev: CountingBloomFilter::with_rate(
                CountingBloomFilter::bits_for_max(hard * 2),
                false_positive_rate,
                expected_num_items,
            ),
            curr: CountingBloomFilter::with_rate(
                CountingBloomFilter::bits_for_max(hard * 2),
                false_positive_rate,
                expected_num_items,
            ),
            soft,
            hard,
        }
    }

    pub fn accumulate(&mut self, now: Instant, h: &impl Hash) -> LimitState {
        // Time to roll the buckets?
        if now.duration_since(self.start) > self.size {
            std::mem::swap(&mut self.prev, &mut self.curr);
            self.curr.clear();
            self.start = now;
        }

        // Use curr + prev and double limits
        let count = self.curr.estimate_count(h) + self.prev.estimate_count(h);
        if count >= self.hard * 2 {
            LimitState::Hard
        } else if count >= self.soft * 2 {
            self.curr.insert_get_count(h);
            LimitState::Soft
        } else {
            self.curr.insert_get_count(h);
            LimitState::None
        }
    }
}

pub struct RateLimits {
    pub enabled: bool,
    ip_minute: Blooms,
    ip_ten_minute: Blooms,
    bot_minute: Blooms,
    bot_ten_minute: Blooms,
}

impl RateLimits {
    pub fn new(config: &RateLimitsConfig) -> Self {
        Self {
            enabled: config.enabled,
            ip_minute: Blooms::new(
                Duration::from_secs(60),
                config.ip.hash.item_count,
                config.ip.hash.false_positive_rate,
                config.ip.minute,
                (config.ip.minute as f32 * config.ip.hard) as u32,
            ),
            ip_ten_minute: Blooms::new(
                Duration::from_secs(10 * 60),
                config.ip.hash.item_count,
                config.ip.hash.false_positive_rate,
                config.ip.ten_minute,
                (config.ip.ten_minute as f32 * config.ip.hard) as u32,
            ),
            bot_minute: Blooms::new(
                Duration::from_secs(60),
                config.bot.hash.item_count,
                config.bot.hash.false_positive_rate,
                config.bot.ten_minute,
                (config.bot.ten_minute as f32 * config.bot.hard) as u32,
            ),
            bot_ten_minute: Blooms::new(
                Duration::from_secs(10 * 60),
                config.bot.hash.item_count,
                config.bot.hash.false_positive_rate,
                config.bot.ten_minute,
                (config.bot.ten_minute as f32 * config.bot.hard) as u32,
            ),
        }
    }

    pub fn accumulate(
        &mut self,
        now: Instant,
        ip: impl Hash,
        bot_ua: Option<impl Hash>,
    ) -> LimitState {
        let mut state = LimitState::None;
        state = state.max(self.ip_minute.accumulate(now, &ip));
        if state == LimitState::Hard {
            return state;
        }
        state = state.max(self.ip_ten_minute.accumulate(now, &ip));
        if state == LimitState::Hard {
            return state;
        }
        if let Some(bot_ua) = bot_ua {
            state = state.max(self.bot_minute.accumulate(now, &bot_ua));
            if state == LimitState::Hard {
                return state;
            }
            state = state.max(self.bot_ten_minute.accumulate(now, &bot_ua));
        }
        state
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    #[test]
    pub fn test_rate_limits() {
        let mut limits = RateLimits::new(&RateLimitsConfig {
            enabled: true,
            bot: BucketConfig {
                hard: 2.0,
                hash: HashConfig {
                    item_count: 1000,
                    false_positive_rate: 0.01,
                },
                ten_minute: 100,
                minute: 10,
            },
            ip: BucketConfig {
                hard: 2.0,
                hash: HashConfig {
                    item_count: 1000,
                    false_positive_rate: 0.01,
                },
                ten_minute: 100,
                minute: 10,
            },
        });

        let ip = "127.0.0.1";
        for i in 0..20 {
            assert_eq!(
                limits.accumulate(Instant::now(), ip, None::<String>),
                LimitState::None,
                "at {i}"
            );
        }

        for i in 20..40 {
            assert_eq!(
                limits.accumulate(Instant::now(), ip, None::<String>),
                LimitState::Soft,
                "at {i}"
            );
        }

        for i in 40..50 {
            assert_eq!(
                limits.accumulate(Instant::now(), ip, None::<String>),
                LimitState::Hard,
                "at {i}"
            );
        }

        // This one is OK
        assert_eq!(
            limits.accumulate(Instant::now(), "255.255.255.255", None::<String>),
            LimitState::None
        );
    }
}
