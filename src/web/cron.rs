use std::collections::HashMap;

use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
pub enum CronInterval {
    Week,
    Day,
    Hour,
    Minute,
    Second,
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
