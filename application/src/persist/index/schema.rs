use tantivy::schema::{Field, Schema, FAST, INDEXED, STORED, STRING, TEXT};

#[derive(Clone)]
pub struct StorySchema {
    pub schema: Schema,
    pub id_field: Field,
    pub url_field: Field,
    pub url_norm_field: Field,
    pub url_norm_hash_field: Field,
    pub host_field: Field,
    pub score_field: Field,
    pub title_field: Field,
    pub date_field: Field,
    pub scrape_field: Field,
    pub tags_field: Field,
}

impl StorySchema {
    pub fn instantiate_global_schema() -> Self {
        let mut schema_builder = Schema::builder();
        let date_field = schema_builder.add_i64_field("date", FAST | STORED);
        let id_field = schema_builder.add_text_field("id", STRING | STORED);
        let url_field = schema_builder.add_text_field("url", STRING | STORED);
        let url_norm_field = schema_builder.add_text_field("url_norm", FAST | STRING);
        let url_norm_hash_field = schema_builder.add_i64_field("url_norm_hash", FAST | INDEXED);
        let host_field = schema_builder.add_text_field("host", TEXT | STORED);
        let title_field = schema_builder.add_text_field("title", TEXT | STORED);
        let scrape_field = schema_builder.add_text_field("scrapes", TEXT | STORED);
        let score_field = schema_builder.add_f64_field("score", FAST | STORED);
        let tags_field = schema_builder.add_text_field("tags", TEXT | STORED);
        let schema = schema_builder.build();

        Self {
            schema,
            id_field,
            host_field,
            url_field,
            url_norm_field,
            url_norm_hash_field,
            score_field,
            title_field,
            date_field,
            scrape_field,
            tags_field,
        }
    }
}
