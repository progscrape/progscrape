use std::{net::SocketAddr, sync::Arc};

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use hyper::{header, HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};
use tera::{Context, Tera};
use thiserror::Error;

use crate::{
    persist::StorageSummary,
    story::{Story, StoryRender},
};

use self::{generate::GeneratedSource};

mod filters;
mod generate;
mod index;
mod serve_static_files;
mod static_files;

#[derive(Debug, Error)]
pub enum WebError {
    #[error("Template error")]
    TeraTemplateError(#[from] tera::Error),
    #[error("Web error")]
    HyperError(#[from] hyper::Error),
    #[error("Persistence error")]
    PersistError(#[from] crate::persist::PersistError),
    #[error("I/O error")]
    IOError(#[from] std::io::Error),
    #[error("Invalid header")]
    InvalidHeader(#[from] hyper::header::InvalidHeaderValue),
    #[error("CSS error")]
    CssError(#[from] Box<grass::Error>),
    #[error("FS notify error")]
    NotifyError(#[from] notify::Error),
    #[error("CBOR error")]
    CBORError(#[from] serde_cbor::Error),
}

impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        let body = format!("Error: {:?}", self);
        (StatusCode::INTERNAL_SERVER_ERROR, body).into_response()
    }
}

pub async fn start_server() -> Result<(), WebError> {
    tracing_subscriber::fmt::init();
    let generated = generate::start_watcher().await?;

    let global = index::initialize_with_testing_data()?;

    // build our application with a route
    let app = Router::new()
        .route("/", get(root))
        .with_state((global.clone(), generated.clone()))
        .route("/static/:file", get(serve_static_files_immutable))
        .with_state(generated.clone())
        .route("/admin/status", get(status))
        .with_state((global.clone(), generated.clone()))
        .route("/admin/templates/reload", get(reload_templates))
        .route(
            "/:file",
            get(serve_static_files_well_known).with_state(generated.clone()),
        );
    // run our app with hyper
    // `axum::Server` is a re-export of `hyper::Server`
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::info!("listening on http://{}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

#[derive(Serialize, Deserialize)]
struct FrontPage {
    top_tags: Vec<String>,
    stories: Vec<StoryRender>,
}

// basic handler that responds with a static string
async fn root(State((state, generated)): State<(index::Global, GeneratedSource)>) -> Result<Html<String>, WebError> {
    let stories = state
        .storage
        .query_frontpage(30)?
        .iter()
        .map(|x| x.render())
        .collect();
    let top_tags = vec!["github.com", "rust", "amazon", "java", "health", "wsj.com", "security", "apple", "theverge.com", "python", "kernel", "google", "arstechnica.com"].into_iter().map(str::to_owned).collect();
    let context = Context::from_serialize(&FrontPage { top_tags, stories })?;
    Ok(generated.templates().render("index2.html", &context)?.into())
}

async fn status(State((state, generated)): State<(index::Global, GeneratedSource)>) -> Result<Html<String>, WebError> {
    let context = Context::from_serialize(&Status {
        storage: state.storage.story_count()?,
    })?;
    Ok(generated.templates().render("status.html", &context)?.into())
}

async fn reload_templates() -> &'static str {
    // TEMPLATES.full_reload();
    "Reloaded templates!"
}

pub async fn serve_static_files_immutable(
    headers_in: HeaderMap,
    Path(key): Path<String>,
    State(generated): State<GeneratedSource>,
) -> Result<impl IntoResponse, WebError> {
    serve_static_files::immutable(headers_in, key, generated.static_files()).await
}

pub async fn serve_static_files_well_known(
    headers_in: HeaderMap,
    Path(file): Path<String>,
    State(generated): State<GeneratedSource>,
) -> Result<impl IntoResponse, WebError> {
    serve_static_files::well_known(headers_in, file, generated.static_files_root()).await
}

#[derive(Serialize)]
struct Status {
    storage: StorageSummary,
}
