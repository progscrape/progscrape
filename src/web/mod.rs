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
    persist::{PersistError, StorageSummary},
    story::{rescore_stories, Story, StoryDate, StoryIdentifier, StoryRender, StoryScoreType},
};

use self::resource::Resources;

mod filters;
mod index;
mod resource;
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
    #[error("JSON error")]
    JSONError(#[from] serde_json::Error),
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
    let resources = resource::start_watcher().await?;

    let global = index::initialize_with_testing_data(&resources.config())?;

    // build our application with a route
    let app = Router::new()
        .route("/", get(root))
        .with_state((global.clone(), resources.clone()))
        .route("/static/:file", get(serve_static_files_immutable))
        .with_state(resources.clone())
        .route("/admin/status/", get(status))
        .with_state((global.clone(), resources.clone()))
        .route("/admin/status/frontpage/", get(status_frontpage))
        .with_state((global.clone(), resources.clone()))
        .route("/admin/status/shard/:shard/", get(status_shard))
        .with_state((global.clone(), resources.clone()))
        .route("/admin/status/story/:story/", get(status_story))
        .with_state((global.clone(), resources.clone()))
        .route(
            "/:file",
            get(serve_static_files_well_known).with_state(resources.clone()),
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

fn render_stories<'a>(iter: impl Iterator<Item = &'a Story>) -> Vec<StoryRender> {
    let v = iter
        .enumerate()
        .map(|(n, x)| x.render(n))
        .collect::<Vec<_>>();
    v
}

fn now(global: &index::Global) -> StoryDate {
    global.storage.most_recent_story()
}

fn hot_set(
    now: StoryDate,
    global: &index::Global,
    config: &crate::config::Config,
) -> Result<Vec<Story>, PersistError> {
    let mut hot_set = global.storage.query_frontpage_hot_set(500)?;
    rescore_stories(&config.score, now, &mut hot_set);
    hot_set.sort_by(|a, b| a.compare_score(b).reverse());
    Ok(hot_set)
}

macro_rules! context {
    ( $($id:ident : $typ:ty = $expr:expr),* ) => {
        {
            #[derive(Serialize)]
            struct TempStruct {
                $(
                    $id: $typ,
                )*
            }

            Context::from_serialize(&TempStruct {
                $(
                    $id: $expr,
                )*
            })?
        }
    };
}

// basic handler that responds with a static string
async fn root(
    State((state, resources)): State<(index::Global, Resources)>,
) -> Result<Html<String>, WebError> {
    let now = now(&state);
    let stories = render_stories(hot_set(now, &state, &resources.config())?[0..30].into_iter());
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
    let context = context!(
        top_tags: Vec<String> = top_tags,
        stories: Vec<StoryRender> = stories
    );
    Ok(resources
        .templates()
        .render("index2.html", &context)?
        .into())
}

async fn status(
    State((state, resources)): State<(index::Global, Resources)>,
) -> Result<Html<String>, WebError> {
    let context = context!(storage: StorageSummary = state.storage.story_count()?);
    Ok(resources
        .templates()
        .render("admin_status.html", &context)?
        .into())
}

async fn status_frontpage(
    State((state, resources)): State<(index::Global, Resources)>,
    sort: Query<HashMap<String, String>>,
) -> Result<Html<String>, WebError> {
    let now = now(&state);
    let sort = sort.get("sort").map(|x| x.clone()).unwrap_or_default();
    let context = context!(
        stories: Vec<StoryRender> =
            render_stories(hot_set(now, &state, &resources.config())?.iter(),),
        sort: String = sort
    );
    Ok(resources
        .templates()
        .render("admin_frontpage.html", &context)?
        .into())
}

async fn status_shard(
    State((state, resources)): State<(index::Global, Resources)>,
    Path(shard): Path<String>,
    sort: Query<HashMap<String, String>>,
) -> Result<Html<String>, WebError> {
    let sort = sort.get("sort").map(|x| x.clone()).unwrap_or_default();
    let context = context!(
        shard: String = shard.clone(),
        stories: Vec<StoryRender> = render_stories(state.storage.stories_by_shard(&shard)?.iter(),),
        sort: String = sort
    );
    Ok(resources
        .templates()
        .render("admin_shard.html", &context)?
        .into())
}

async fn status_story(
    State((state, resources)): State<(index::Global, Resources)>,
    Path(id): Path<String>,
) -> Result<Html<String>, WebError> {
    let id = StoryIdentifier::from_base64(id).ok_or(WebError::NotFound)?;
    let now = now(&state);
    tracing::info!("Loading story = {:?}", id);
    let story = state.storage.get_story(&id).ok_or(WebError::NotFound)?;
    let context = context!(
        story: StoryRender = story.render(0),
        score: Vec<(String, f32)> =
            story.score_detail(&resources.config().score, StoryScoreType::AgedFrom(now))
    );
    Ok(resources
        .templates()
        .render("admin_story.html", &context)?
        .into())
}

pub async fn serve_static_files_immutable(
    headers_in: HeaderMap,
    Path(key): Path<String>,
    State(resources): State<Resources>,
) -> Result<impl IntoResponse, WebError> {
    serve_static_files::immutable(headers_in, key, resources.static_files()).await
}

pub async fn serve_static_files_well_known(
    headers_in: HeaderMap,
    Path(file): Path<String>,
    State(resources): State<Resources>,
) -> Result<impl IntoResponse, WebError> {
    serve_static_files::well_known(headers_in, file, resources.static_files_root()).await
}
