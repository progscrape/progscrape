use std::{collections::HashMap, net::SocketAddr};

use axum::{
    extract::{Path, Query, State},
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use hyper::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};
use tera::Context;
use thiserror::Error;

use crate::{
    persist::StorageSummary,
    story::{Story, StoryIdentifier, StoryRender},
};

use self::generate::GeneratedSource;

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
    #[error("Item not found")]
    NotFound,
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
        .route("/admin/status/", get(status))
        .with_state((global.clone(), generated.clone()))
        .route("/admin/status/frontpage/", get(status_frontpage))
        .with_state((global.clone(), generated.clone()))
        .route("/admin/status/shard/:shard/", get(status_shard))
        .with_state((global.clone(), generated.clone()))
        .route("/admin/status/story/:story/", get(status_story))
        .with_state((global.clone(), generated.clone()))
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

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum StorySort {
    None,
    Title,
    Date,
    Domain,
}

impl StorySort {
    pub fn from_key(s: &str) -> Self {
        match s {
            "title" => Self::Title,
            "date" => Self::Date,
            "domain" => Self::Domain,
            _ => Self::None,
        }
    }
}

fn render_stories(iter: impl Iterator<Item = Story>, sort: StorySort) -> Vec<StoryRender> {
    let mut v = iter.map(|x| x.render()).collect::<Vec<_>>();
    match sort {
        StorySort::None => {}
        StorySort::Title => v.sort_by(|a, b| a.title.cmp(&b.title)),
        StorySort::Date => v.sort_by(|a, b| a.date.cmp(&b.date)),
        StorySort::Domain => v.sort_by(|a, b| a.domain.cmp(&b.domain)),
    }
    v
}

// basic handler that responds with a static string
async fn root(
    State((state, generated)): State<(index::Global, GeneratedSource)>,
) -> Result<Html<String>, WebError> {
    let stories = render_stories(
        state.storage.query_frontpage(0, 30)?.into_iter(),
        StorySort::None,
    );
    let top_tags = vec![
        "github.com",
        "rust",
        "amazon",
        "java",
        "health",
        "wsj.com",
        "security",
        "apple",
        "theverge.com",
        "python",
        "kernel",
        "google",
        "arstechnica.com",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let context = Context::from_serialize(&FrontPage { top_tags, stories })?;
    Ok(generated
        .templates()
        .render("index2.html", &context)?
        .into())
}

async fn status(
    State((state, generated)): State<(index::Global, GeneratedSource)>,
) -> Result<Html<String>, WebError> {
    #[derive(Serialize)]
    struct Status {
        storage: StorageSummary,
    }
    let context = Context::from_serialize(&Status {
        storage: state.storage.story_count()?,
    })?;
    Ok(generated
        .templates()
        .render("admin_status.html", &context)?
        .into())
}

async fn status_frontpage(
    State((state, generated)): State<(index::Global, GeneratedSource)>,
) -> Result<Html<String>, WebError> {
    #[derive(Serialize)]
    struct Status {
        stories: Vec<StoryRender>,
    }
    let context = Context::from_serialize(&Status {
        stories: render_stories(
            state.storage.query_frontpage(0, 500)?.into_iter(),
            StorySort::None,
        ),
    })?;
    Ok(generated
        .templates()
        .render("admin_frontpage.html", &context)?
        .into())
}

async fn status_shard(
    State((state, generated)): State<(index::Global, GeneratedSource)>,
    Path(shard): Path<String>,
    sort: Query<HashMap<String, String>>,
) -> Result<Html<String>, WebError> {
    #[derive(Serialize)]
    struct ShardStatus {
        shard: String,
        stories: Vec<StoryRender>,
    }
    let sort = StorySort::from_key(&sort.get("sort").map(|x| x.clone()).unwrap_or_default());
    let context = Context::from_serialize(&ShardStatus {
        shard: shard.clone(),
        stories: render_stories(state.storage.stories_by_shard(&shard)?.into_iter(), sort),
    })?;
    Ok(generated
        .templates()
        .render("admin_shard.html", &context)?
        .into())
}

async fn status_story(
    State((state, generated)): State<(index::Global, GeneratedSource)>,
    Path(id): Path<String>,
) -> Result<Html<String>, WebError> {
    #[derive(Serialize)]
    struct StoryStatus {
        story: StoryRender,
        score: Vec<(String, f32)>,
    }
    let id = StoryIdentifier::from_base64(id).ok_or(WebError::NotFound)?;
    tracing::info!("Loading story = {:?}", id);
    let story = state
        .storage
        .get_story(&id)
        .ok_or(WebError::NotFound)?;
    let context = Context::from_serialize(&StoryStatus { story: story.render(), score: story.score_detail() })?;
    Ok(generated
        .templates()
        .render("admin_story.html", &context)?
        .into())
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
