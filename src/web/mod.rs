use std::{collections::HashMap, net::SocketAddr};

use axum::{
    extract::{Path, Query, State},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use hyper::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};
use tera::Context;
use thiserror::Error;

use crate::{
    persist::{PersistError, StorageSummary},
    scrapers::{web_scraper::{WebScrapeInput, WebScraper}, ScrapeSource, Scrape},
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
    #[error("Scrape error")]
    ScrapeError(#[from] crate::scrapers::ScrapeError),
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
    #[error("Reqwest error")]
    ReqwestError(#[from] reqwest::Error),
    #[error("Item not found")]
    NotFound,
}

impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        let body = format!("Error: {:?}", self);
        (StatusCode::INTERNAL_SERVER_ERROR, body).into_response()
    }
}

pub fn admin_routes<S>(resources: Resources, global: index::Global) -> Router<S> {
    Router::new()
        .route("/", get(admin))
        .route("/scrape/", get(admin_scrape))
        .route("/scrape/test", post(admin_scrape_test))
        .route("/index/", get(admin_index_status))
        .route("/index/frontpage/", get(admin_status_frontpage))
        .route("/index/shard/:shard/", get(admin_status_shard))
        .route("/index/story/:story/", get(admin_status_story))
        .with_state((global, resources))
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
        .nest("/admin", admin_routes(resources.clone(), global.clone()))
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

/// Render a context with a given template name.
fn render(
    resources: &Resources,
    template_name: &str,
    context: Context,
) -> Result<Html<String>, WebError> {
    Ok(resources
        .templates()
        .render(template_name, &context)?
        .into())
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
    render(
        &resources,
        "index2.html",
        context!(
            top_tags: Vec<String> = top_tags,
            stories: Vec<StoryRender> = stories
        ),
    )
}

async fn admin(
    State((state, resources)): State<(index::Global, Resources)>,
) -> Result<Html<String>, WebError> {
    render(
        &resources,
        "admin/admin.html",
        context!(config: std::sync::Arc<crate::config::Config> = resources.config().clone()),
    )
}

async fn admin_scrape(
    State((state, resources)): State<(index::Global, Resources)>,
) -> Result<Html<String>, WebError> {
    let config = resources.config().clone();
    render(
        &resources,
        "admin/scrape.html",
        context!(
            config: std::sync::Arc<crate::config::Config> = config.clone(),
            scrapes: WebScrapeInput = WebScraper::calculate_inputs(&config.scrape),
            endpoint: &'static str = "/admin/scrape/test"
        ),
    )
}

#[derive(Deserialize)]
struct AdminScrapeTestParams {
    /// Which source do we want to scrape?
    source: ScrapeSource,
    subsources: Vec<String>,
}

async fn admin_scrape_test(
    State((state, resources)): State<(index::Global, Resources)>,
    Json(params): Json<AdminScrapeTestParams>,
) -> Result<Html<String>, WebError> {
    let config = resources.config().clone();
    let urls = WebScraper::compute_urls(&config.scrape, params.source, params.subsources);
    let mut results = vec![];
    for url in urls {
        let text = reqwest::get(&url).await?.text().await?;
        results.push((url, WebScraper::scrape(&config.scrape, params.source, text)?));
    }
    render(&resources, "admin/scrape_test.html", context!(
        scrapes: Vec<(String, (Vec<Scrape>, Vec<String>))> = results
    ))
}

async fn admin_index_status(
    State((state, resources)): State<(index::Global, Resources)>,
) -> Result<Html<String>, WebError> {
    render(
        &resources,
        "admin/status.html",
        context!(
            storage: StorageSummary = state.storage.story_count()?,
            config: std::sync::Arc<crate::config::Config> = resources.config().clone()
        ),
    )
}

async fn admin_status_frontpage(
    State((state, resources)): State<(index::Global, Resources)>,
    sort: Query<HashMap<String, String>>,
) -> Result<Html<String>, WebError> {
    let now = now(&state);
    let sort = sort.get("sort").map(|x| x.clone()).unwrap_or_default();
    render(
        &resources,
        "admin/frontpage.html",
        context!(
            stories: Vec<StoryRender> =
                render_stories(hot_set(now, &state, &resources.config())?.iter(),),
            sort: String = sort
        ),
    )
}

async fn admin_status_shard(
    State((state, resources)): State<(index::Global, Resources)>,
    Path(shard): Path<String>,
    sort: Query<HashMap<String, String>>,
) -> Result<Html<String>, WebError> {
    let sort = sort.get("sort").map(|x| x.clone()).unwrap_or_default();
    render(
        &resources,
        "admin/shard.html",
        context!(
            shard: String = shard.clone(),
            stories: Vec<StoryRender> =
                render_stories(state.storage.stories_by_shard(&shard)?.iter(),),
            sort: String = sort
        ),
    )
}

async fn admin_status_story(
    State((state, resources)): State<(index::Global, Resources)>,
    Path(id): Path<String>,
) -> Result<Html<String>, WebError> {
    let id = StoryIdentifier::from_base64(id).ok_or(WebError::NotFound)?;
    let now = now(&state);
    tracing::info!("Loading story = {:?}", id);
    let story = state.storage.get_story(&id).ok_or(WebError::NotFound)?;
    render(
        &resources,
        "admin/story.html",
        context!(
            story: StoryRender = story.render(0),
            score: Vec<(String, f32)> =
                story.score_detail(&resources.config().score, StoryScoreType::AgedFrom(now))
        ),
    )
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
