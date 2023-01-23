use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use axum::{
    extract::{Path, Query, State},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use hyper::{HeaderMap, Request, StatusCode};
use serde::{Deserialize, Serialize};
use tera::Context;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::{
    auth::Auth,
    cron::{Cron, CronTask},
    index::{self, Index},
    resource::{self, Resources},
    serve_static_files,
};

use progscrape_application::{
    PersistError, StorageSummary, Story, StoryEvaluator, StoryIdentifier, StoryRender,
};
use progscrape_scrapers::{
    ScrapeSource, ScraperHttpResponseInput, ScraperHttpResult, ScraperPossibilities, StoryDate,
};

#[derive(Debug, Error)]
pub enum WebError {
    #[error("Template error")]
    TeraTemplateError(#[from] tera::Error),
    #[error("Web error")]
    HyperError(#[from] hyper::Error),
    #[error("Persistence error")]
    PersistError(#[from] progscrape_application::PersistError),
    #[error("Legacy error")]
    LegacyError(#[from] progscrape_scrapers::LegacyError),
    #[error("Scrape error")]
    ScrapeError(#[from] progscrape_scrapers::ScrapeError),
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
    #[error("Log setup error")]
    LogSetupError(#[from] tracing_subscriber::filter::ParseError),
    #[error("Log setup error")]
    LogSetup2Error(#[from] tracing_subscriber::filter::FromEnvError),
    #[error("Item not found")]
    NotFound,
    #[error("Invalid command-line arguments")]
    ArgumentsInvalid(String),
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
    index: index::Index,
    cron: Arc<Mutex<Cron>>,
}

#[derive(Clone, Serialize, Deserialize)]
struct CurrentUser {
    user: String,
}

async fn authorize<B>(
    State(auth): State<Auth>,
    mut req: Request<B>,
    next: Next<B>,
) -> Result<Response, StatusCode> {
    tracing::info!("Attempting authorization against auth = {:?}", auth);
    let user = match auth {
        Auth::None => None,
        Auth::Fixed(fixed) => Some(fixed.clone()),
        Auth::FromHeader(header) => req
            .headers()
            .get(header)
            .and_then(|header| header.to_str().ok().map(|s| s.to_string())),
    };

    match user {
        None => {
            tracing::error!("No user authorized for this path!");
            Ok((StatusCode::UNAUTHORIZED, ">progscrape: 403 ▒").into_response())
        }
        Some(user) => {
            req.extensions_mut().insert(CurrentUser { user });
            Ok(next.run(req).await)
        }
    }
}

async fn ensure_slash<B>(req: Request<B>, next: Next<B>) -> Result<Response, StatusCode> {
    let test_uri = "/admin";
    let final_uri = "/admin/";
    if req.uri().path() == test_uri {
        tracing::debug!("Redirecting {} -> {}", test_uri, final_uri);
        return Ok(Redirect::permanent(final_uri).into_response());
    }

    Ok(next.run(req).await)
}

async fn handle_404() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, ">progscrape: 404 ▒")
}

pub fn admin_routes<S: Clone + Send + Sync + 'static>(
    resources: Resources,
    index: index::Index,
    cron: Arc<Mutex<Cron>>,
    auth: Auth,
) -> Router<S> {
    Router::new()
        .route("/", get(admin))
        .route("/cron/", get(admin_cron))
        .route("/headers/", get(admin_headers))
        .route("/scrape/", get(admin_scrape))
        .route("/scrape/test", post(admin_scrape_test))
        .route("/index/", get(admin_index_status))
        .route("/index/frontpage/", get(admin_status_frontpage))
        .route("/index/shard/:shard/", get(admin_status_shard))
        .route("/index/story/:story/", get(admin_status_story))
        .fallback(handle_404)
        .with_state(AdminState {
            resources,
            index,
            cron,
        })
        .route_layer(middleware::from_fn_with_state(auth, authorize))
}

fn start_cron(cron: Arc<Mutex<Cron>>, resources: Resources) {
    tokio::spawn(async move {
        loop {
            cron.lock().await.tick(&resources.config().cron);
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });
}

pub async fn start_server(
    root_path: &std::path::Path,
    index: Index,
    auth: Auth,
) -> Result<(), WebError> {
    tracing::info!("Root path: {:?}", root_path);
    let resource_path = root_path.join("resource");

    let resources = resource::start_watcher(resource_path).await?;

    let cron = Arc::new(Mutex::new(Cron::initialize(&resources.config().cron)));
    start_cron(cron.clone(), resources.clone());

    // build our application with a route
    let app = Router::new()
        .route("/", get(root))
        .with_state((index.clone(), resources.clone()))
        .route("/static/:file", get(serve_static_files_immutable))
        .with_state(resources.clone())
        .nest(
            "/admin",
            admin_routes(resources.clone(), index.clone(), cron.clone(), auth),
        )
        .route_layer(middleware::from_fn(ensure_slash))
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

fn now(global: &index::Index) -> Result<StoryDate, PersistError> {
    global.storage.most_recent_story()
}

fn hot_set(
    now: StoryDate,
    global: &index::Index,
    eval: &StoryEvaluator,
) -> Result<Vec<Story>, PersistError> {
    let mut hot_set = global.storage.query_frontpage_hot_set(500)?;
    eval.scorer.resort_stories(now, &mut hot_set);
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
    State((index, resources)): State<(index::Index, Resources)>,
    query: Query<HashMap<String, String>>,
) -> Result<Html<String>, WebError> {
    let now = now(&index)?;
    let stories = if let Some(search) = query.get("search") {
        index
            .storage
            .query_search(&resources.story_evaluator().tagger, search, 30)?
    } else {
        let mut vec = hot_set(now, &index, &resources.story_evaluator())?;
        vec.truncate(30);
        vec
    };
    let stories = render_stories(stories.iter());
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
        "index.html",
        context!(
            top_tags: Vec<String> = top_tags,
            stories: Vec<StoryRender> = stories,
            now: StoryDate = now
        ),
    )
}

async fn admin(
    Extension(user): Extension<CurrentUser>,
    State(AdminState { resources, .. }): State<AdminState>,
) -> Result<Html<String>, WebError> {
    render(
        &resources,
        "admin/admin.html",
        context!(
            user: CurrentUser = user,
            config: std::sync::Arc<crate::config::Config> = resources.config()
        ),
    )
}

async fn admin_cron(
    Extension(user): Extension<CurrentUser>,
    State(AdminState {
        cron, resources, ..
    }): State<AdminState>,
) -> Result<Html<String>, WebError> {
    render(
        &resources,
        "admin/cron.html",
        context!(
            user: CurrentUser = user,
            config: std::sync::Arc<crate::config::Config> = resources.config(),
            cron: Vec<CronTask> = cron.lock().await.inspect()
        ),
    )
}

async fn admin_headers(
    Extension(user): Extension<CurrentUser>,
    State(AdminState { resources, .. }): State<AdminState>,
    Query(query): Query<HashMap<String, String>>,
    raw_headers: HeaderMap,
) -> Result<Html<String>, WebError> {
    let mut headers = HashMap::new();
    for (header, value) in raw_headers {
        let name = header.map(|h| h.to_string()).unwrap_or("(missing)".into());
        headers
            .entry(name)
            .or_insert(vec![])
            .push(String::from_utf8_lossy(value.as_bytes()).to_string());
    }
    render(
        &resources,
        "admin/headers.html",
        context!(
            user: CurrentUser = user,
            query: HashMap<String, String> = query,
            headers: HashMap<String, Vec<String>> = headers
        ),
    )
}

async fn admin_scrape(
    Extension(user): Extension<CurrentUser>,
    State(AdminState { resources, .. }): State<AdminState>,
) -> Result<Html<String>, WebError> {
    let config = resources.config();
    render(
        &resources,
        "admin/scrape.html",
        context!(
            user: CurrentUser = user,
            config: std::sync::Arc<crate::config::Config> = config,
            scrapes: ScraperPossibilities = resources.scrapers().compute_scrape_possibilities(),
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
    Extension(user): Extension<CurrentUser>,
    State(AdminState { resources, .. }): State<AdminState>,
    Json(params): Json<AdminScrapeTestParams>,
) -> Result<Html<String>, WebError> {
    let urls = resources
        .scrapers()
        .compute_scrape_url_demands(params.source, params.subsources);
    let mut map = HashMap::new();
    for url in urls {
        let resp = reqwest::Client::new()
            .get(&url)
            .header("User-Agent", "progscrape")
            .send()
            .await?;
        let status = resp.status();
        if status == StatusCode::OK {
            map.insert(url, ScraperHttpResponseInput::Ok(resp.text().await?));
        } else {
            map.insert(
                url,
                ScraperHttpResponseInput::HTTPError(status.as_u16(), status.as_str().to_owned()),
            );
        }
    }

    let scrapes = HashMap::from_iter(
        map.into_iter()
            .map(|(k, v)| (k, resources.scrapers().scrape_http_result(params.source, v))),
    );

    render(
        &resources,
        "admin/scrape_test.html",
        context!(
            user: CurrentUser = user,
            scrapes: HashMap<String, ScraperHttpResult> = scrapes
        ),
    )
}

async fn admin_index_status(
    Extension(user): Extension<CurrentUser>,
    State(AdminState {
        index, resources, ..
    }): State<AdminState>,
) -> Result<Html<String>, WebError> {
    render(
        &resources,
        "admin/status.html",
        context!(
            user: CurrentUser = user,
            storage: StorageSummary = index.storage.story_count()?,
            config: std::sync::Arc<crate::config::Config> = resources.config()
        ),
    )
}

async fn admin_status_frontpage(
    Extension(user): Extension<CurrentUser>,
    State(AdminState {
        index, resources, ..
    }): State<AdminState>,
    sort: Query<HashMap<String, String>>,
) -> Result<Html<String>, WebError> {
    let now = now(&index)?;
    let sort = sort.get("sort").cloned().unwrap_or_default();
    render(
        &resources,
        "admin/frontpage.html",
        context!(
            user: CurrentUser = user,
            stories: Vec<StoryRender> =
                render_stories(hot_set(now, &index, &resources.story_evaluator())?.iter(),),
            sort: String = sort
        ),
    )
}

async fn admin_status_shard(
    Extension(user): Extension<CurrentUser>,
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
            user: CurrentUser = user,
            shard: String = shard.clone(),
            stories: Vec<StoryRender> =
                render_stories(index.storage.stories_by_shard(&shard)?.iter(),),
            sort: String = sort
        ),
    )
}

async fn admin_status_story(
    Extension(user): Extension<CurrentUser>,
    State(AdminState {
        index, resources, ..
    }): State<AdminState>,
    Path(id): Path<String>,
) -> Result<Html<String>, WebError> {
    let id = StoryIdentifier::from_base64(id).ok_or(WebError::NotFound)?;
    let _now = now(&index)?;
    tracing::info!("Loading story = {:?}", id);
    let story = index.storage.get_story(&id).ok_or(WebError::NotFound)?;
    // let score_details = resources.story_evaluator().scorer.score_detail(&story, now);
    let score_details = vec![];
    let tags = Default::default(); // _details = resources.story_evaluator().tagger.tag_detail(&story);

    render(
        &resources,
        "admin/story.html",
        context!(
            user: CurrentUser = user,
            story: StoryRender = story.0.render(0),
            tags: HashMap<String, Vec<String>> = tags,
            score: Vec<(String, f32)> = score_details
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
