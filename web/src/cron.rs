use std::{
    collections::{BTreeMap, HashMap, HashSet},
    ops::RangeInclusive,
    time::{Duration, Instant, SystemTime},
};

use rand::Rng;
use serde::{Deserialize, Serialize};

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
        let count = count as u64;
        match self {
            Self::Second => Duration::from_secs(count),
            Self::Minute => Duration::from_secs(count * 60),
            Self::Hour => Duration::from_secs(count * 60 * 60),
            Self::Day => Duration::from_secs(count * 60 * 60 * 24),
            Self::Week => Duration::from_secs(count * 60 * 60 * 24 * 7),
        }
    }

    pub fn as_duration_f32(&self, count: f32) -> Duration {
        match self {
            Self::Second => Duration::from_secs_f32(count),
            Self::Minute => Duration::from_secs_f32(count * 60.0),
            Self::Hour => Duration::from_secs_f32(count * 60.0 * 60.0),
            Self::Day => Duration::from_secs_f32(count * 60.0 * 60.0 * 24.0),
            Self::Week => Duration::from_secs_f32(count * 60.0 * 60.0 * 24.0 * 7.0),
        }
    }
}

fn default_cron_job_enabled() -> bool {
    true
}

#[derive(Serialize, Deserialize)]
pub struct CronJob {
    url: String,
    interval: (usize, CronInterval),
    /// Eagerly queue this cron job's first run regardless of interval.
    #[serde(default)]
    eager: bool,
    /// Set to `false` to disable a job without removing it from the config.
    #[serde(default = "default_cron_job_enabled")]
    enabled: bool,
}

#[derive(Serialize, Deserialize)]
pub struct CronConfig {
    pub jobs: HashMap<String, CronJob>,
    pub jitter: (i8, i8),
    pub history_age: (usize, CronInterval),
    pub history_count: usize,
}

impl Default for CronConfig {
    fn default() -> Self {
        Self {
            jobs: Default::default(),
            jitter: (0, 0),
            history_age: (1, CronInterval::Minute),
            history_count: 10,
        }
    }
}

#[derive(Default)]
pub struct CronHistory {
    history: HashMap<String, BTreeMap<Instant, (u16, String)>>,
}

impl CronHistory {
    pub fn insert(
        &mut self,
        max_age: (usize, CronInterval),
        max_count: usize,
        service: String,
        status_code: u16,
        output: String,
    ) {
        let now = Instant::now();
        let map = self.history.entry(service).or_default();
        map.insert(now, (status_code, output));
        let cutoff = now.checked_sub(max_age.1.as_duration(max_age.0));
        if let Some(cutoff) = cutoff {
            while let (len, Some(entry)) = (map.len(), map.first_entry()) {
                if len > max_count || *entry.key() < cutoff {
                    map.pop_first();
                } else {
                    break;
                }
            }
        } else {
            // This _may_ fail when the computer is recently rebooted as the instant might be relative
            // to that particular time.
            tracing::error!("Failed to compute the cutoff, perhaps this computer was rebooted recently? now={:?} duration={} {:?}", now, max_age.0, max_age.1);
        }
    }

    pub fn entries(&self) -> Vec<(u64, String, u16, String)> {
        let mut out = vec![];
        for (service, entries) in &self.history {
            for (time, (status, output)) in entries {
                out.push((
                    approximate_instant_as_unix_time(*time),
                    service.clone(),
                    *status,
                    output.clone(),
                ));
            }
        }
        out.sort_by_cached_key(|k| k.0);
        out
    }
}

/// A very basic cron system that allows us to schedule tasks that will be triggered as URL POSTs.
pub struct Cron {
    queue: Vec<CronTask>,
    jitter_fraction: Option<RangeInclusive<f32>>,
}

#[derive(Clone)]
pub struct CronTask {
    name: String,
    url: String,
    last: Option<Instant>,
    next: Instant,
}

fn approximate_instant_as_unix_time(when: Instant) -> u64 {
    let now = Instant::now();
    if when > now {
        (SystemTime::now() + (when - now))
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    } else {
        (SystemTime::now() - (now - when))
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}

impl Serialize for CronTask {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        #[derive(Serialize)]
        struct Temp<'a> {
            name: &'a str,
            url: &'a str,
            next: u64,
            last: u64,
        }
        Temp {
            name: &self.name,
            url: &self.url,
            next: approximate_instant_as_unix_time(self.next),
            last: self
                .last
                .map(approximate_instant_as_unix_time)
                .unwrap_or_default(),
        }
        .serialize(serializer)
    }
}

impl Cron {
    /// Create a new `Cron` system with the specified jitter. The system is empty until ticked once.
    pub fn new_with_jitter(jitter: RangeInclusive<i8>) -> Self {
        // No empty ranges for this function (use new())
        assert!(!jitter.is_empty());
        // Less than -100 is problematic, obviously, though we'll handle negative jitter without totally falling over
        assert!(*jitter.start() >= -100);

        // Convert percentage (-20..=+20) to fraction (0.8..=1.2)
        let start = *(jitter).start() as f32 / 100.0 + 1.0;
        let end = *(jitter).end() as f32 / 100.0 + 1.0;

        Self {
            queue: vec![],
            jitter_fraction: Some(start..=end),
        }
    }

    /// Create a new `Cron` system. The system is empty until ticked once.
    #[cfg(test)]
    pub fn new() -> Self {
        Self {
            queue: vec![],
            jitter_fraction: None,
        }
    }

    fn jitter(&self, interval: (usize, CronInterval)) -> Duration {
        if let Some(jitter) = &self.jitter_fraction {
            // Perform jitter math in floating point
            let fraction = rand::thread_rng().gen_range(jitter.clone());
            // For sanity, clamp fraction from EPSILON..2.0
            let fraction = fraction.clamp(f32::EPSILON, 2.0);
            interval.1.as_duration_f32(interval.0 as f32 * fraction)
        } else {
            interval.1.as_duration(interval.0)
        }
    }

    /// Trigger a task to run at the next call to `tick`.
    pub fn trigger(&mut self, job_name: String) -> bool {
        for job in self.queue.iter_mut() {
            if job.name == job_name {
                job.next = Instant::now() - Duration::from_secs(1);
                return true;
            }
        }
        false
    }

    pub fn tick(&mut self, jobs: &HashMap<String, CronJob>, now: Instant) -> Vec<String> {
        // Drain the queue of any ready items
        let mut ready = HashSet::new();
        let mut ret = vec![];
        let mut remaining = HashMap::<_, _>::from_iter(jobs.iter().filter(|job| job.1.enabled));
        self.queue.retain(|task| {
            let entry = remaining.get(&task.name);
            if entry.is_none() {
                false
            } else if task.next <= now {
                ready.insert(task.name.clone());
                ret.push(task.url.clone());
                false
            } else {
                remaining.remove(&task.name);
                true
            }
        });

        // If we find a job in the config and it isn't already in the queue, add it in
        for (name, job) in remaining {
            if !job.enabled {
                continue;
            }
            let last = if ready.contains(name) {
                Some(now)
            } else {
                None
            };
            let jitter = if job.eager && last.is_none() {
                Duration::from_secs(1)
            } else {
                self.jitter(job.interval)
            };
            self.queue.push(CronTask {
                name: name.clone(),
                url: job.url.clone(),
                next: now + jitter,
                last,
            });
        }

        ret
    }

    pub fn inspect(&self) -> Vec<CronTask> {
        self.queue.to_vec()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_cron() {
        let mut jobs = HashMap::new();
        jobs.insert(
            "job".into(),
            CronJob {
                url: "/1".into(),
                interval: (1, CronInterval::Minute),
                eager: false,
                enabled: true,
            },
        );
        let mut cron = Cron::new();
        let mut now = Instant::now();
        // No tasks available yet
        assert_eq!(cron.inspect().len(), 0);
        // No tasks ready yet, but we'll pick up one task
        assert_eq!(cron.tick(&jobs, now).len(), 0);
        // The one task is available...
        assert_eq!(cron.inspect().len(), 1);
        // ... but not ready
        assert_eq!(cron.tick(&jobs, now).len(), 0);

        // Not ready after one second either
        now = now.checked_add(Duration::from_secs(1)).expect("Add");
        assert_eq!(cron.tick(&jobs, now).len(), 0);

        // In one minute we'll have one task ready (we use 61 seconds to guarantee we ticked over it)
        now = now.checked_add(Duration::from_secs(61)).expect("Add");
        assert_eq!(cron.tick(&jobs, now).len(), 1);
        // But it can only be picked up once
        assert_eq!(cron.tick(&jobs, now).len(), 0);
    }

    #[test]
    fn test_history() {
        let mut history = CronHistory::default();
        history.insert(
            (1, CronInterval::Minute),
            10,
            "service".into(),
            200,
            "".into(),
        );
    }
}
