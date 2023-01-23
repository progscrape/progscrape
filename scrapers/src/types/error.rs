use thiserror::Error;

#[derive(Error, Debug)]
pub enum ScrapeError {
    #[error("I/O error")]
    IO(#[from] std::io::Error),
    #[error("JSON parse error")]
    Json(#[from] serde_json::Error),
    #[error("HTML parse error")]
    Html(#[from] tl::ParseError),
    #[error("XML parse error")]
    Xml(#[from] roxmltree::Error),
    #[error("Structure error")]
    StructureError(String),
}
