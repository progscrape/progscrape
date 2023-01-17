use std::sync::Arc;

use num_format::ToFormattedString;
use serde_json::Value;

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

pub struct StaticFileFilter {
    static_files: Arc<StaticFileRegistry>,
}

impl StaticFileFilter {
    pub fn new(static_files: Arc<StaticFileRegistry>) -> Self {
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
