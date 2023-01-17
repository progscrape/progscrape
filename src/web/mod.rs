use std::{collections::HashMap, net::SocketAddr, sync::Arc};

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
use tokio::sync::Mutex;

use crate::{
    persist::{PersistError, StorageSummary},
    scrapers::{
        self,
        web_scraper::{WebScrapeHttpResult, WebScrapeInput, WebScrapeURLResult, WebScraper},
        ScrapeSource, TypedScrape,
    },
    story::{rescore_stories, Story, StoryDate, StoryIdentifier, StoryRender, StoryScoreType, TagSet},
    web::cron::Cron,
};

use self::resource::Resources;

pub mod cron;
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

#[derive(Clone)]
struct AdminState {
    resources: Resources,
    index: index::Global,
    cron: Arc<Mutex<Cron>>,
}

pub fn admin_routes<S>(
    resources: Resources,
    index: index::Global,
    cron: Arc<Mutex<Cron>>,
) -> Router<S> {
    Router::new()
        .route("/", get(admin))
        .route("/cron/", get(admin_cron))
        .route("/scrape/", get(admin_scrape))
        .route("/scrape/test", post(admin_scrape_test))
        .route("/index/", get(admin_index_status))
        .route("/index/frontpage/", get(admin_status_frontpage))
        .route("/index/shard/:shard/", get(admin_status_shard))
        .route("/index/story/:story/", get(admin_status_story))
        .with_state(AdminState {
            resources,
            index,
            cron,
        })
}

fn start_cron(cron: Arc<Mutex<Cron>>, resources: Resources) {
    tokio::spawn(async move {
        loop {
            cron.lock().await.tick(&resources.config().cron);
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });
}

pub async fn start_server() -> Result<(), WebError> {
    tracing_subscriber::fmt::init();
    let resources = resource::start_watcher().await?;

    let global = index::initialize_with_testing_data(&resources.config())?;

    let cron = Arc::new(Mutex::new(Cron::initialize(&resources.config().cron)));
    start_cron(cron.clone(), resources.clone());

    // build our application with a route
    let app = Router::new()
        .route("/", get(root))
        .with_state((global.clone(), resources.clone()))
        .route("/static/:file", get(serve_static_files_immutable))
        .with_state(resources.clone())
        .nest(
            "/admin",
            admin_routes(resources.clone(), global.clone(), cron.clone()),
        )
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
    iter.enumerate()
        .map(|(n, x)| x.render(n))
        .collect::<Vec<_>>()
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

            #[allow(clippy::redundant_field_names)]
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
    State((index, resources)): State<(index::Global, Resources)>,
) -> Result<Html<String>, WebError> {
    let now = now(&index);
    let stories = render_stories(hot_set(now, &index, &resources.config())?[0..30].iter());
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
    State(AdminState { resources, .. }): State<AdminState>,
) -> Result<Html<String>, WebError> {
    render(
        &resources,
        "admin/admin.html",
        context!(config: std::sync::Arc<crate::config::Config> = resources.config()),
    )
}

async fn admin_cron(
    State(AdminState {
        cron, resources, ..
    }): State<AdminState>,
) -> Result<Html<String>, WebError> {
    render(
        &resources,
        "admin/cron.html",
        context!(
            config: std::sync::Arc<crate::config::Config> = resources.config(),
            cron: Vec<cron::CronTask> = cron.lock().await.inspect()
        ),
    )
}

async fn admin_scrape(
    State(AdminState { resources, .. }): State<AdminState>,
) -> Result<Html<String>, WebError> {
    let config = resources.config();
    render(
        &resources,
        "admin/scrape.html",
        context!(
            config: std::sync::Arc<crate::config::Config> = config.clone(),
            scrapes: WebScrapeInput = WebScraper::compute_all_scrapes(&config.scrape),
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
    State(AdminState { resources, .. }): State<AdminState>,
    Json(params): Json<AdminScrapeTestParams>,
) -> Result<Html<String>, WebError> {
    let config = resources.config();
    let urls = WebScraper::compute_urls(&config.scrape, params.source, params.subsources);
    let mut map = HashMap::new();
    for url in urls {
        let resp = reqwest::Client::new()
            .get(&url)
            .header("User-Agent", "progscrape")
            .send()
            .await?;
        let status = resp.status();
        if status == StatusCode::OK {
            map.insert(url, WebScrapeHttpResult::Ok(resp.text().await?));
        } else {
            map.insert(
                url,
                WebScrapeHttpResult::HTTPError(status.as_u16(), status.as_str().to_owned()),
            );
        }
    }

    let scrapes = HashMap::from_iter(
        map.into_iter()
            .map(|(k, v)| (k, WebScraper::scrape(&config.scrape, params.source, v))),
    );

    render(
        &resources,
        "admin/scrape_test.html",
        context!(scrapes: HashMap<String, WebScrapeURLResult> = scrapes),
    )
}

async fn admin_index_status(
    State(AdminState {
        index, resources, ..
    }): State<AdminState>,
) -> Result<Html<String>, WebError> {
    render(
        &resources,
        "admin/status.html",
        context!(
            storage: StorageSummary = index.storage.story_count()?,
            config: std::sync::Arc<crate::config::Config> = resources.config()
        ),
    )
}

async fn admin_status_frontpage(
    State(AdminState {
        index, resources, ..
    }): State<AdminState>,
    sort: Query<HashMap<String, String>>,
) -> Result<Html<String>, WebError> {
    let now = now(&index);
    let sort = sort.get("sort").cloned().unwrap_or_default();
    render(
        &resources,
        "admin/frontpage.html",
        context!(
            stories: Vec<StoryRender> =
                render_stories(hot_set(now, &index, &resources.config())?.iter(),),
            sort: String = sort
        ),
    )
}

async fn admin_status_shard(
    State(AdminState {
        index, resources, ..
    }): State<AdminState>,
    Path(shard): Path<String>,
    sort: Query<HashMap<String, String>>,
) -> Result<Html<String>, WebError> {
    let sort = sort.get("sort").cloned().unwrap_or_default();
    render(
        &resources,
        "admin/shard.html",
        context!(
            shard: String = shard.clone(),
            stories: Vec<StoryRender> =
                render_stories(index.storage.stories_by_shard(&shard)?.iter(),),
            sort: String = sort
        ),
    )
}

async fn admin_status_story(
    State(AdminState {
        index, resources, ..
    }): State<AdminState>,
    Path(id): Path<String>,
) -> Result<Html<String>, WebError> {
    let id = StoryIdentifier::from_base64(id).ok_or(WebError::NotFound)?;
    let now = now(&index);
    tracing::info!("Loading story = {:?}", id);
    let story = index.storage.get_story(&id).ok_or(WebError::NotFound)?;
    let mut tags = HashMap::new();
    let mut tag_set = TagSet::new();
    resources.tagger().tag(story.title(), &mut tag_set);
    tags.insert("title".to_owned(), tag_set.collect());
    for (id, scrape) in &story.scrapes {
        let mut tag_set = TagSet::new();
        scrape.tag(&resources.config().scrape, &mut tag_set)?;
        tags.insert(format!("scrape {:?}", id), tag_set.collect());
    }
    render(
        &resources,
        "admin/story.html",
        context!(
            story: StoryRender = story.render(0),
            tags: HashMap<String, Vec<String>> = tags,
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
