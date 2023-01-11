use std::{collections::{HashMap, HashSet}, time::SystemTime};

use chrono::{DateTime, Utc, Duration};
use rand::Rng;
use serde::{Serialize, Deserialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum CronInterval {
    Week,
    Day,
    Hour,
    Minute,
    Second,
}

impl CronInterval {
    pub fn as_duration(&self, count: usize) -> Duration {
        let count = count as i64;
        match self {
            Self::Second => Duration::seconds(count),
            Self::Minute => Duration::minutes(count),
            Self::Hour => Duration::hours(count),
            Self::Day => Duration::days(count),
            Self::Week => Duration::weeks(count),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct CronJob {
    url: String,
    interval: (usize, CronInterval),
}

#[derive(Default, Serialize, Deserialize)]
pub struct CronConfig {
    jobs: HashMap<String, CronJob>,
}

pub struct Cron {
    queue: Vec<CronTask>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CronTask {
    name: String,
    url: String,
    next: DateTime<Utc>,
}

fn now() -> DateTime<Utc> {
    SystemTime::now().into()
}

fn jitter(percent: u8, interval: (usize, CronInterval)) -> Duration {
    // Jitter is a percentage of the expected interval
    let jitter = interval.1.as_duration(interval.0).num_milliseconds();
    let jitter = jitter * (percent as i64) / 100;
    Duration::milliseconds(rand::thread_rng().gen_range(0..jitter))
}

fn next_cron(interval: (usize, CronInterval)) -> DateTime<Utc> {
    // Jitter is 20% of the expected interval
    now() + interval.1.as_duration(interval.0) + jitter(20, interval)
}

impl Cron {
    pub fn initialize(config: &CronConfig) -> Self {
        let mut new = Self { queue: vec![] };
        let _ = new.tick(config);
        new
    }

    pub fn tick(&mut self, config: &CronConfig) -> Vec<String> {
        let now = now();

        // Drain the queue of any ready items
        let mut ready = vec![];
        let mut remaining = HashMap::<_, _>::from_iter(config.jobs.iter());
        self.queue.retain(|task| {
            if task.next > now {
                ready.push(task.url.clone());
                false
            } else {
                remaining.remove(&task.name);
                true
            }
        });

        // If we find a job in the config and it isn't already in the queue, add it in
        for (name, job) in remaining {
            self.queue.push(CronTask { name: name.clone(), url: job.url.clone(), next: next_cron(job.interval) });
        }

        ready
    }

    pub fn inspect(&self) -> Vec<CronTask> {
        self.queue.iter().cloned().collect()
    }
}
