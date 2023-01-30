use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Instant};

use axum::{
    body::HttpBody,
    extract::{Path, Query, State},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use hyper::{service::Service, Body, HeaderMap, Method, Request, StatusCode};
use serde::{Deserialize, Serialize};
use tera::Context;
use thiserror::Error;
use tokio::sync::Mutex;
use unwrap_infallible::UnwrapInfallible;

use crate::{
    auth::Auth,
    cron::{Cron, CronHistory},
    index::Index,
    resource::{self, Resources},
    serve_static_files,
};
use progscrape_application::{
    PersistError, Shard, Storage, StorageWriter, Story, StoryEvaluator, StoryIdentifier,
    StoryIndex, StoryQuery, StoryRender, StoryScore, TagSet,
};
use progscrape_scrapers::{
    ScrapeCollection, ScrapeSource, ScraperHttpResponseInput, ScraperHttpResult, StoryDate,
    TypedScrape,
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
    index: Index<StoryIndex>,
    cron: Arc<Mutex<Cron>>,
    cron_history: Arc<Mutex<CronHistory>>,
}

#[derive(Clone, Serialize, Deserialize)]
struct CurrentUser {
    user: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct CronMarker {}

async fn authorize<B>(
    State(auth): State<Auth>,
    mut req: Request<B>,
    next: Next<B>,
) -> Result<Response, StatusCode> {
    // Allow cron requests to bypass authorization
    if req.extensions().get::<CronMarker>().is_some() {
        req.extensions_mut().insert(CurrentUser {
            user: "cron".into(),
        });
        return Ok(next.run(req).await);
    }

    tracing::info!("Attempting authorization against auth = {:?}", auth);
    let user = match auth {
        Auth::None => None,
        Auth::Fixed(fixed) => Some(fixed),
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
    index: Index<StoryIndex>,
    cron: Arc<Mutex<Cron>>,
    cron_history: Arc<Mutex<CronHistory>>,
    auth: Auth,
) -> Router<S> {
    Router::new()
        .route("/", get(admin))
        .route("/cron/", get(admin_cron))
        .route("/cron/", post(admin_cron_post))
        .route("/cron/refresh", post(admin_cron_refresh))
        .route("/cron/scrape/:service", post(admin_cron_scrape))
        .route("/headers/", get(admin_headers))
        .route("/scrape/", get(admin_scrape))
        .route("/scrape/test", post(admin_scrape_test))
        .route("/index/", get(admin_index_status))
        .route("/index/frontpage/", get(admin_status_frontpage))
        .route(
            "/index/frontpage/scoretuner/",
            get(admin_index_frontpage_scoretuner),
        )
        .route("/index/shard/:shard/", get(admin_status_shard))
        .route("/index/story/:story/", get(admin_status_story))
        .fallback(handle_404)
        .with_state(AdminState {
            resources,
            index,
            cron,
            cron_history,
        })
        .route_layer(middleware::from_fn_with_state(auth, authorize))
}

/// Feed the `Cron` request list into the `Router`.
fn start_cron(
    cron: Arc<Mutex<Cron>>,
    cron_history: Arc<Mutex<CronHistory>>,
    resources: Resources,
    router: Router<()>,
) {
    // Router doesn't require poll_ready
    let mut router = router.into_make_service();
    tokio::spawn(async move {
        let mut router = router.call(()).await.unwrap_infallible();
        loop {
            let ready = cron
                .lock()
                .await
                .tick(&resources.config().cron.jobs, Instant::now());

            // Sleep if no tasks are available
            if ready.is_empty() {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                continue;
            }

            for ready_uri in ready {
                let uri = match ready_uri.parse() {
                    Ok(uri) => uri,
                    Err(e) => {
                        tracing::error!("Failed to parse URI: {} (error was {:?})", ready_uri, e);
                        continue;
                    }
                };
                tracing::info!("Running cron task: POST '{}'...", ready_uri);
                let mut req = Request::<Body>::default();
                *req.method_mut() = Method::POST;
                *req.uri_mut() = uri;
                (*req.extensions_mut()).insert(CronMarker {});
                let response = router.call(req).await.unwrap_infallible();
                let status = response.status();
                tracing::info!("Cron task '{}' ran with status {}", ready_uri, status);

                // TODO: Do we need to read data() multiple times?
                let body = match response.into_body().data().await {
                    Some(Ok(b)) => String::from_utf8_lossy(&b).to_string(),
                    x => {
                        tracing::error!("Could not retrieve body from cron response: {:?}", x);
                        "(empty)".into()
                    }
                };

                cron_history.lock().await.insert(
                    resources.config().cron.history_age,
                    resources.config().cron.history_count,
                    ready_uri,
                    status.as_u16(),
                    body,
                );
            }
        }
    });
}

pub async fn start_server(
    root_path: &std::path::Path,
    address: SocketAddr,
    index: Index<StoryIndex>,
    auth: Auth,
) -> Result<(), WebError> {
    tracing::info!("Root path: {:?}", root_path);
    let resource_path = root_path.join("resource");

    let resources = resource::start_watcher(resource_path).await?;

    let cron = Arc::new(Mutex::new(Cron::new_with_jitter(-20..=20)));
    let cron_history = Arc::new(Mutex::new(CronHistory::default()));

    // build our application with a route
    let app = Router::new()
        .route("/", get(root))
        .with_state((index.clone(), resources.clone()))
        .route("/static/:file", get(serve_static_files_immutable))
        .with_state(resources.clone())
        .nest(
            "/admin",
            admin_routes(
                resources.clone(),
                index.clone(),
                cron.clone(),
                cron_history.clone(),
                auth,
            ),
        )
        .route_layer(middleware::from_fn(ensure_slash))
        .route(
            "/:file",
            get(serve_static_files_well_known).with_state(resources.clone()),
        );
    // run our app with hyper
    // `axum::Server` is a re-export of `hyper::Server`
    tracing::info!("listening on http://{}", address);

    start_cron(
        cron.clone(),
        cron_history.clone(),
        resources.clone(),
        app.clone(),
    );

    axum::Server::bind(&address)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

fn render_stories<'a, S: 'a>(iter: impl Iterator<Item = &'a Story<S>>) -> Vec<StoryRender> {
    iter.enumerate()
        .map(|(n, x)| x.render(n))
        .collect::<Vec<_>>()
}

async fn now(global: &Index<StoryIndex>) -> Result<StoryDate, PersistError> {
    global.storage.read().await.most_recent_story()
}

async fn hot_set(
    now: StoryDate,
    global: &Index<StoryIndex>,
    eval: &StoryEvaluator,
) -> Result<Vec<Story<Shard>>, PersistError> {
    let mut hot_set = global.storage.read().await.query_frontpage_hot_set(500)?;
    eval.scorer.resort_stories(now, &mut hot_set);
    Ok(hot_set)
}

macro_rules! context_assign {
    ($id:ident , ,) => {};
    ($id:ident , , $typ:ty) => {
        let $id: $typ = $id;
    };
    ($id:ident , $expr:expr , $typ:ty) => {
        let $id: $typ = $expr;
    };
    ($id:ident , $expr:expr ,) => {
        let $id = $expr;
    };
}

macro_rules! context {
    ( $($id:ident $(: $typ:ty)? $(= $expr:expr)? ),* $(,)? ) => {
        {
            let mut context = Context::new();

            // Create a local variable for each item of the context, with a type if specified.
            $(
                context_assign!($id , $($expr)? , $($typ)?);
                context.insert(stringify!($id), &$id);
            )*

            context
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
    State((index, resources)): State<(Index<StoryIndex>, Resources)>,
    query: Query<HashMap<String, String>>,
) -> Result<Html<String>, WebError> {
    let now = now(&index).await?;
    let stories = if let Some(search) = query.get("search") {
        index
            .storage
            .read()
            .await
            .query_search(&resources.story_evaluator().tagger, search, 30)?
    } else {
        let mut vec = hot_set(now, &index, &resources.story_evaluator()).await?;
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
    ];
    render(&resources, "index.html", context!(top_tags, stories, now))
}

async fn admin(
    Extension(user): Extension<CurrentUser>,
    State(AdminState { resources, .. }): State<AdminState>,
) -> Result<Html<String>, WebError> {
    render(
        &resources,
        "admin/admin.html",
        context!(user, config = resources.config()),
    )
}

async fn admin_cron(
    Extension(user): Extension<CurrentUser>,
    State(AdminState {
        cron,
        cron_history,
        resources,
        ..
    }): State<AdminState>,
) -> Result<Html<String>, WebError> {
    render(
        &resources,
        "admin/cron.html",
        context!(
            user,
            config = resources.config(),
            cron = cron.lock().await.inspect(),
            history = cron_history.lock().await.entries()
        ),
    )
}

#[derive(Deserialize)]
struct AdminCronRunParams {
    /// Which job do we want to trigger?
    cron: String,
}

async fn admin_cron_post(
    State(AdminState { cron, .. }): State<AdminState>,
    Json(params): Json<AdminCronRunParams>,
) -> Result<Json<bool>, WebError> {
    let success = cron.lock().await.trigger(params.cron);
    Ok(success.into())
}

async fn admin_cron_refresh(
    State(AdminState { resources, .. }): State<AdminState>,
) -> Result<Html<String>, WebError> {
    render(
        &resources,
        "admin/cron_refresh.html",
        context!(config = resources.config()),
    )
}

async fn admin_cron_scrape(
    State(AdminState {
        resources, index, ..
    }): State<AdminState>,
    Path(source): Path<ScrapeSource>,
) -> Result<Html<String>, WebError> {
    let subsources = resources.scrapers().compute_scrape_subsources(source);
    let urls = resources
        .scrapers()
        .compute_scrape_url_demands(source, subsources);
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
            .map(|(k, v)| (k, resources.scrapers().scrape_http_result(source, v))),
    );

    for result in scrapes.values() {
        match result {
            ScraperHttpResult::Ok(_, scrapes) => index
                .storage
                .write()
                .await
                .insert_scrapes(&resources.story_evaluator(), scrapes.iter().cloned())?,
            ScraperHttpResult::Err(..) => {}
        }
    }

    render(
        &resources,
        "admin/cron_scrape_run.html",
        context!(
            source,
            config = resources.config(),
            scrapes: HashMap<String, ScraperHttpResult>,
        ),
    )
}

async fn admin_headers(
    Extension(user): Extension<CurrentUser>,
    State(AdminState { resources, .. }): State<AdminState>,
    Query(query): Query<HashMap<String, String>>,
    raw_headers: HeaderMap,
) -> Result<Html<String>, WebError> {
    let mut headers: HashMap<_, Vec<String>> = HashMap::new();
    for (header, value) in raw_headers {
        let name = header.map(|h| h.to_string()).unwrap_or("(missing)".into());
        headers
            .entry(name)
            .or_default()
            .push(String::from_utf8_lossy(value.as_bytes()).to_string());
    }
    render(
        &resources,
        "admin/headers.html",
        context!(user, query, headers),
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
            user,
            config,
            scrapes = resources.scrapers().compute_scrape_possibilities(),
            endpoint = "/admin/scrape/test"
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
        context!(user, scrapes: HashMap<String, ScraperHttpResult>),
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
            user,
            storage = index.storage.read().await.story_count()?,
            config = resources.config()
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
    let now = now(&index).await?;
    let sort = sort.get("sort").cloned().unwrap_or_default();
    render(
        &resources,
        "admin/frontpage.html",
        context!(
            now,
            user,
            stories = render_stories(
                hot_set(now, &index, &resources.story_evaluator())
                    .await?
                    .iter(),
            ),
            sort,
        ),
    )
}

async fn admin_index_frontpage_scoretuner(
    Extension(user): Extension<CurrentUser>,
    State(AdminState {
        index, resources, ..
    }): State<AdminState>,
) -> Result<Html<String>, WebError> {
    let now = now(&index).await?;

    #[derive(Serialize)]
    struct StoryDetail {
        story: StoryRender,
        score_detail: Vec<(StoryScore, f32)>,
    }

    let stories: Vec<Story<TypedScrape>> = index
        .storage
        .read()
        .await
        .fetch(StoryQuery::FrontPage(), 500)?;
    let mut story_details = vec![];
    let eval = resources.story_evaluator();

    for mut story in stories {
        let scrapes = ScrapeCollection::new_from_iter(story.scrapes.values().cloned());
        let extracted = scrapes.extract(&eval.extractor);
        story.score = eval.scorer.score(&extracted) + eval.scorer.score_age(now - story.date);
        let mut tags = TagSet::from_iter(extracted.tags());
        eval.tagger.tag(extracted.title(), &mut tags);
        story.tags = tags;
        story_details.push(StoryDetail {
            story: story.render(0),
            score_detail: eval.scorer.score_detail(&extracted, now),
        });
    }

    // Quick-and-dirty float sort
    story_details.sort_by_cached_key(|x| (x.story.score * -1000.0) as i32);

    render(
        &resources,
        "admin/scoretuner.html",
        context!(now, user, story_details,),
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
            user,
            shard = shard.clone(),
            stories = render_stories(index.storage.read().await.stories_by_shard(&shard)?.iter(),),
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
    let now = now(&index).await?;
    tracing::info!("Loading story = {:?}", id);
    let story = index
        .storage
        .read()
        .await
        .get_story(&id)?
        .ok_or(WebError::NotFound)?;
    let scrapes = ScrapeCollection::new_from_iter(story.scrapes.clone().into_values());
    let extract = scrapes.extract(&resources.story_evaluator().extractor);
    let score_details = resources
        .story_evaluator()
        .scorer
        .score_detail(&extract, now);
    let tags = Default::default(); // _details = resources.story_evaluator().tagger.tag_detail(&story);

    render(
        &resources,
        "admin/story.html",
        context!(
            now,
            user,
            story = story.render(0),
            scrapes = scrapes.scrapes,
            tags: HashMap<String, Vec<String>>,
            score = score_details
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
