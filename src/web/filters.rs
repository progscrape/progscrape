use num_format::ToFormattedString;
use serde_json::Value;

#[derive(Default)]
pub struct CommaFilter {}

impl tera::Filter for CommaFilter {
    fn filter(&self, value: &Value, args: &std::collections::HashMap<String, Value>) -> tera::Result<Value> {
        Ok(value.as_i64().unwrap_or_default().to_formatted_string(&num_format::Locale::en).into())
    }
}
