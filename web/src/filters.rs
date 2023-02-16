use num_format::ToFormattedString;
use progscrape_scrapers::{ScrapeId, StoryDate, StoryDuration};
use serde_json::Value;

use crate::types::Shared;

use super::static_files::StaticFileRegistry;

#[derive(Default)]
pub struct CommaFilter {}

impl tera::Filter for CommaFilter {
    fn filter(
        &self,
        value: &Value,
        _args: &std::collections::HashMap<String, Value>,
    ) -> tera::Result<Value> {
        Ok(value
            .as_i64()
            .unwrap_or_else(|| {
                tracing::warn!("Invalid input to comma filter");
                0
            })
            .to_formatted_string(&num_format::Locale::en)
            .into())
    }
}

#[derive(Default)]
pub struct AbsoluteTimeFilter {}

impl tera::Filter for AbsoluteTimeFilter {
    fn filter(
        &self,
        value: &Value,
        _args: &std::collections::HashMap<String, Value>,
    ) -> tera::Result<Value> {
        let date = value.as_i64().and_then(StoryDate::from_seconds);
        if let Some(date) = date {
            Ok(format!("{}", date).into())
        } else {
            Err("Invalid date arguments".to_string().into())
        }
    }
}

#[derive(Default)]
pub struct RFC3339Filter {}

impl tera::Filter for RFC3339Filter {
    fn filter(
        &self,
        value: &Value,
        _args: &std::collections::HashMap<String, Value>,
    ) -> tera::Result<Value> {
        let date = value.as_i64().and_then(StoryDate::from_seconds);
        if let Some(date) = date {
            Ok(date.to_rfc3339().into())
        } else {
            Err("Invalid date arguments".to_string().into())
        }
    }
}

#[derive(Default)]
pub struct RelativeTimeFilter {}

impl tera::Filter for RelativeTimeFilter {
    fn filter(
        &self,
        value: &Value,
        args: &std::collections::HashMap<String, Value>,
    ) -> tera::Result<Value> {
        let date = value.as_i64().and_then(StoryDate::from_seconds);
        let now = args
            .get("now")
            .and_then(Value::as_i64)
            .and_then(StoryDate::from_seconds);
        if let (Some(date), Some(now)) = (date, now) {
            let relative = now - date;
            if relative > StoryDuration::days(60) {
                Ok(format!("{} months ago", relative.num_days() / 30).into())
            } else if relative > StoryDuration::days(2) {
                Ok(format!("{} days ago", relative.num_days()).into())
            } else if relative > StoryDuration::minutes(120) {
                Ok(format!("{} hours ago", relative.num_hours()).into())
            } else if relative > StoryDuration::minutes(60) {
                Ok("an hour ago".into())
            } else {
                Ok("recently added".into())
            }
        } else {
            Err("Invalid date arguments".to_string().into())
        }
    }
}

#[derive(Default)]
pub struct ApproxTimeFilter {}

impl tera::Filter for ApproxTimeFilter {
    fn filter(
        &self,
        value: &Value,
        _args: &std::collections::HashMap<String, Value>,
    ) -> tera::Result<Value> {
        let date = value.as_i64().and_then(StoryDate::from_seconds);
        let now = StoryDate::now();

        if let Some(date) = date {
            Ok(if now > date {
                let relative = now - date;
                if relative > StoryDuration::days(1) {
                    format!("{:.2} day(s) ago", relative.num_days_f32())
                } else if relative > StoryDuration::hours(1) {
                    format!("{:.2} hour(s) ago", relative.num_hours_f32())
                } else if relative > StoryDuration::minutes(1) {
                    format!("{:.2} minutes(s) ago", relative.num_minutes_f32())
                } else if relative > StoryDuration::seconds(1) {
                    format!("{} second(s) ago", relative.num_seconds())
                } else {
                    "just now".to_string()
                }
            } else {
                let relative = date - now;
                if relative > StoryDuration::days(1) {
                    format!("in {:.2} day(s)", relative.num_days_f32())
                } else if relative > StoryDuration::hours(1) {
                    format!("in {:.2} hour(s)", relative.num_hours_f32())
                } else if relative > StoryDuration::minutes(1) {
                    format!("in {:.2} minutes(s)", relative.num_minutes_f32())
                } else if relative > StoryDuration::seconds(1) {
                    format!("in {} second(s)", relative.num_seconds())
                } else {
                    "now".to_string()
                }
            }
            .into())
        } else {
            Err("Invalid date arguments".to_string().into())
        }
    }
}

#[derive(Default)]
pub struct CommentLinkFilter {}

impl tera::Filter for CommentLinkFilter {
    fn filter(
        &self,
        value: &Value,
        _args: &std::collections::HashMap<String, Value>,
    ) -> tera::Result<Value> {
        let id = value.as_str().unwrap_or_else(|| {
            tracing::warn!("Invalid input to comment_link filter");
            ""
        });

        let s = if let Some(id) = ScrapeId::from_string(id) {
            id.comments_url()
        } else {
            tracing::warn!("Invalid scrape ID for comment_link filter");
            "<invalid>".to_owned()
        };

        Ok(s.into())
    }
}

pub struct StaticFileFilter {
    static_files: Shared<StaticFileRegistry>,
}

impl StaticFileFilter {
    pub fn new(static_files: Shared<StaticFileRegistry>) -> Self {
        Self { static_files }
    }
}

impl tera::Filter for StaticFileFilter {
    fn filter(
        &self,
        value: &Value,
        _args: &std::collections::HashMap<String, Value>,
    ) -> tera::Result<Value> {
        let key = value.as_str().unwrap_or_else(|| {
            tracing::warn!("Invalid input to static filter");
            ""
        });
        let s = format!(
            "/static/{}",
            self.static_files.lookup_key(key).unwrap_or_else(|| {
                tracing::warn!("Static file not found: {}", key);
                "<invalid>"
            })
        );
        Ok(s.into())
    }
}
