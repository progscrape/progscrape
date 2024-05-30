use std::{collections::HashMap, net::SocketAddr, time::Instant};

use axum::{
    body::Body,
    extract::{Host, OriginalUri, Path, Query, Request, State},
    http::{HeaderName, HeaderValue},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use hyper::{header, HeaderMap, Method, StatusCode};
use itertools::Itertools;
use keepcalm::SharedMut;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tera::Context;
use thiserror::Error;
use tokio::net::TcpListener;
use tower::Service;
use unwrap_infallible::UnwrapInfallible;

use crate::{
    auth::Auth,
    cron::{Cron, CronHistory},
    index::Index,
    resource::Resources,
    serve_static_files,
    story::FeedStory,
};
use progscrape_application::{
    PersistError, ScrapePersistResultSummarizer, ScrapePersistResultSummary, Shard, Story,
    StoryEvaluator, StoryIdentifier, StoryIndex, StoryQuery, StoryRender, StoryScore, TagSet,
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
    #[error("Authentication failed")]
    AuthError,
    #[error("Invalid command-line arguments")]
    ArgumentsInvalid(String),
}

impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        let body = format!("Error: {:?}", self);
        let code = match self {
            Self::AuthError => StatusCode::UNAUTHORIZED,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::InvalidHeader(_) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (code, body).into_response()
    }
}

#[derive(Clone)]
struct AdminState {
    resources: Resources,
    index: Index<StoryIndex>,
    cron: SharedMut<Cron>,
    cron_history: SharedMut<CronHistory>,
    backup_path: Option<std::path::PathBuf>,
}

#[derive(Clone, Serialize, Deserialize)]
struct CurrentUser {
    user: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct CronMarker {}

async fn authorize(
    State(auth): State<Auth>,
    mut req: Request,
    next: Next,
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

async fn ensure_slash(req: Request, next: Next) -> Result<Response, StatusCode> {
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

async fn handle_404_admin() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, "#progscrape: 404 ▒")
}

pub fn admin_routes<S: Clone + Send + Sync + 'static>(
    resources: Resources,
    index: Index<StoryIndex>,
    cron: SharedMut<Cron>,
    cron_history: SharedMut<CronHistory>,
    backup_path: Option<std::path::PathBuf>,
    auth: Auth,
) -> Router<S> {
    Router::new()
        .route("/", get(admin))
        .route("/cron/", get(admin_cron))
        .route("/cron/", post(admin_cron_post))
        .route("/cron/backup", post(admin_cron_backup))
        .route("/cron/refresh", post(admin_cron_refresh))
        .route("/cron/reindex", post(admin_cron_reindex))
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
        .fallback(handle_404_admin)
        .with_state(AdminState {
            resources,
            index,
            cron,
            cron_history,
            backup_path,
        })
        .route_layer(middleware::from_fn_with_state(auth, authorize))
}

/// Feed the `Cron` request list into the `Router`.
fn start_cron(
    cron: SharedMut<Cron>,
    cron_history: SharedMut<CronHistory>,
    resources: Resources,
    router: Router<()>,
) {
    // Router doesn't require poll_ready
    let mut router = router.into_make_service();
    tokio::spawn(async move {
        let mut router = router.call(()).await.unwrap_infallible();
        loop {
            let ready = cron
                .write()
                .tick(&resources.config.read().cron.jobs, Instant::now());

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
                let body = axum::body::to_bytes(response.into_body(), 1_000_000).await;
                let body = match body {
                    Ok(b) => String::from_utf8_lossy(&b).into_owned(),
                    x => {
                        tracing::error!("Could not retrieve body from cron response: {:?}", x);
                        "(empty)".into()
                    }
                };

                cron_history.write().insert(
                    resources.config.read().cron.history_age,
                    resources.config.read().cron.history_count,
                    ready_uri,
                    status.as_u16(),
                    body,
                );
            }
        }
    });
}

/// Create the router for the root page, the Atom feed, and the JSON API.
pub fn create_feeds<S: Clone + Send + Sync + 'static>(
    index: Index<StoryIndex>,
    resources: Resources,
) -> Router<S> {
    Router::new()
        .route("/", get(root))
        .route("/feed.json", get(root_feed_json))
        .route("/feed.txt", get(root_feed_text))
        .route("/feed", get(root_feed_xml))
        .with_state((index, resources))
}

pub async fn start_server<P2: Into<std::path::PathBuf>>(
    resources: Resources,
    backup_path: Option<P2>,
    address: SocketAddr,
    index: Index<StoryIndex>,
    auth: Auth,
    metrics_auth_bearer_token: Option<String>,
) -> Result<(), WebError> {
    index.refresh_hot_set().await?;

    let cron = SharedMut::new(Cron::new_with_jitter(-20..=20));
    let cron_history = SharedMut::new(CronHistory::default());

    // build our application with a route
    let app = create_feeds(index.clone(), resources.clone())
        .route("/metrics/opentelemetry.txt", get(root_metrics_txt))
        .with_state((index.clone(), resources.clone(), metrics_auth_bearer_token))
        .route("/state", get(state_tracker))
        .nest(
            "/admin/",
            admin_routes(
                resources.clone(),
                index.clone(),
                cron.clone(),
                cron_history.clone(),
                backup_path.map(P2::into),
                auth,
            ),
        )
        .route("/static/:file", get(serve_static_files_immutable))
        .with_state(resources.clone())
        .route_layer(middleware::from_fn(ensure_slash))
        .route(
            "/:file",
            get(serve_static_files_well_known).with_state(resources.clone()),
        )
        .fallback(handle_404);
    // run our app with hyper
    // `axum::Server` is a re-export of `hyper::Server`
    tracing::info!("listening on http://{}", address);

    start_cron(
        cron.clone(),
        cron_history.clone(),
        resources.clone(),
        app.clone(),
    );

    let tcp = TcpListener::bind(&address).await?;
    axum::serve(tcp, app.into_make_service()).await?;

    Ok(())
}

fn render_stories<'a, S: 'a>(
    eval: &StoryEvaluator,
    iter: impl Iterator<Item = &'a Story<S>>,
) -> Vec<StoryRender> {
    iter.enumerate()
        .map(|(n, x)| x.render(eval, n))
        .collect::<Vec<_>>()
}

async fn now(global: &Index<StoryIndex>) -> Result<StoryDate, PersistError> {
    global.most_recent_story().await
}

macro_rules! context_assign {
    ($id:ident , ,) => {};
    ($id:ident , , $typ:ty) => {
        #[allow(clippy::redundant_locals)]
        let $id: $typ = $id;
    };
    ($id:ident , $expr:expr , $typ:ty) => {
        #[allow(clippy::redundant_locals)]
        let $id: $typ = $expr;
    };
    ($id:ident , $expr:expr ,) => {
        #[allow(clippy::redundant_locals)]
        let $id = $expr;
    };
}

macro_rules! context {
    ( $($id:ident $(: $typ:ty)? $(= $expr:expr)? ),* $(,)? ) => {
        {
            #[allow(unused_mut)]
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
    mut context: Context,
) -> Result<Html<String>, WebError> {
    // Add git information to all the templates
    use git_version::git_version;
    const GIT_VERSION: &str = git_version!();
    context.insert("git", GIT_VERSION);

    Ok(resources
        .templates
        .read()
        .render(template_name, &context)?
        .into())
}

/// Render an admin context with a given template name, adding the headers to
/// avoid any caching whatsoever.
fn render_admin(
    user: Option<&CurrentUser>,
    resources: &Resources,
    template_name: &str,
    mut context: Context,
) -> Result<impl IntoResponse, WebError> {
    // If this is an authenticated page, log it with the user
    if let Some(user) = user {
        tracing::debug!("Admin page: template={template_name} user={}", user.user);
    } else {
        tracing::trace!("Admin page (internal):  template={template_name}");
    }
    context.insert("config", &resources.config);
    Ok((
        [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))],
        render(resources, template_name, context),
    ))
}

// basic handler that responds with a static string
async fn root(
    OriginalUri(original_uri): OriginalUri,
    Host(host): Host,
    State((index, resources)): State<(Index<StoryIndex>, Resources)>,
    query: Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, WebError> {
    let now = now(&index).await?;
    let search = query.get("search");
    let offset: usize = query
        .get("offset")
        .map(|x| x.parse().unwrap_or_default())
        .unwrap_or_default();
    let stories = index.stories::<StoryRender>(search, offset, 30).await?;
    let top_tags = index.top_tags(20)?;
    let path = original_uri
        .path_and_query()
        .map(|s| s.as_str())
        .unwrap_or_default();
    Ok(([(
        header::CACHE_CONTROL,
        HeaderValue::from_static(
            "public, max-age=300, s-max-age=300, stale-while-revalidate=60, stale-if-error=86400",
        ),
    )],
    render(&resources, "index.html", context!(top_tags, stories, now, search, offset, host, path))))
}

// basic handler that responds with a static string
async fn root_feed_json(
    State((index, _resources)): State<(Index<StoryIndex>, Resources)>,
    query: Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, WebError> {
    let stories = index
        .stories::<FeedStory>(query.get("search"), 0, 150)
        .await?;
    let top_tags: Vec<_> = index
        .top_tags(usize::MAX)?
        .into_iter()
        .map(|s| s.0)
        .collect();

    Ok((
        [(
            header::CACHE_CONTROL,
            HeaderValue::from_static(
                "public, max-age=300, s-max-age=300, stale-while-revalidate=60, stale-if-error=86400",
            ),
        )],
        Json(json!({
            "v": 1,
            "tags": top_tags,
            "stories": stories
        })),
    ))
}

async fn root_feed_xml(
    Host(host): Host,
    State((index, resources)): State<(Index<StoryIndex>, Resources)>,
    query: Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, WebError> {
    let now = now(&index).await?;
    let stories = index
        .stories::<StoryRender>(query.get("search"), 0, 30)
        .await?;

    let xml = resources
        .templates
        .read()
        .render("feed.xml", &context!(stories, now, host))?;
    Ok((
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/atom+xml"),
        ), (
            header::CACHE_CONTROL,
            HeaderValue::from_static(
                "public, max-age=300, s-max-age=300, stale-while-revalidate=60, stale-if-error=86400",
            ),
        ), (
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static("*"),
        )],
        xml,
    ))
}

async fn root_feed_text(
    Host(host): Host,
    State((index, resources)): State<(Index<StoryIndex>, Resources)>,
    query: Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, WebError> {
    let now = now(&index).await?;
    let stories = index
        .stories::<StoryRender>(query.get("search"), 0, 100)
        .await?;

    let xml = resources
        .templates
        .read()
        .render("feed.txt", &context!(stories, now, host))?;
    Ok((
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        ), (
            header::CACHE_CONTROL,
            HeaderValue::from_static(
                "public, max-age=300, s-max-age=300, stale-while-revalidate=60, stale-if-error=86400",
            ),
        ), (
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static("*"),
        )],
        xml,
    ))
}

async fn state_tracker(
    path: Query<HashMap<String, String>>,
    headers_in: HeaderMap,
) -> Result<impl IntoResponse, WebError> {
    #[derive(Serialize)]
    struct TrackerEntry<'a> {
        p: &'a str,
        r: Option<&'a str>,
        ua: Option<&'a str>,
        ip: Option<&'a str>,
    }

    fn header<'a>(headers_in: &'a HeaderMap, key: HeaderName) -> Option<&'a str> {
        headers_in
            .get(key)
            .map(|s| s.as_bytes())
            .and_then(|s| std::str::from_utf8(s).ok())
    }

    let referrer = path.0.get("r").map(|s| s.as_str()).unwrap_or_default();
    let path = path.0.get("path").map(|s| s.as_str()).unwrap_or_default();
    let entry = TrackerEntry {
        p: path,
        r: Some(referrer),
        ua: header(&headers_in, header::USER_AGENT),
        ip: header(&headers_in, HeaderName::from_static("x-forwarded-for")),
    };

    tracing::info!(
        "pageload data={}",
        serde_json::to_string(&entry).unwrap_or_default()
    );

    Ok((
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/javascript"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, no-store, no-cache, must-revalidate, max-age=0"),
            ),
            (header::PRAGMA, HeaderValue::from_static("no-cache")),
            (
                HeaderName::from_static("surrogate-control"),
                HeaderValue::from_static("no-store, max-age=0"),
            ),
        ],
        "void 0;",
    ))
}

/// Return the current metrics in Prometheus-compatible format.
async fn root_metrics_txt(
    headers_in: HeaderMap,
    State((index, resources, metrics_auth_bearer_token)): State<(
        Index<StoryIndex>,
        Resources,
        Option<String>,
    )>,
) -> Result<impl IntoResponse, WebError> {
    if metrics_auth_bearer_token.is_none() {
        // Temporarily allow the old behaviour if the --metrics-auth-bearer-token flag isn't passed
        if !headers_in.contains_key(header::AUTHORIZATION) {
            return Err(WebError::AuthError);
        }
    } else {
        if !headers_in.contains_key(header::AUTHORIZATION) || metrics_auth_bearer_token.is_none() {
            return Err(WebError::AuthError);
        }
        if headers_in.get(header::AUTHORIZATION).unwrap()
            != format!("Bearer {}", metrics_auth_bearer_token.unwrap()).as_bytes()
        {
            tracing::error!(
                "Invalid bearer token for metrics: {:?}",
                headers_in.get(header::AUTHORIZATION).unwrap()
            );
            return Err(WebError::AuthError);
        }
    }
    let stories = index
        .stories::<StoryRender>(None::<String>, 0, usize::MAX)
        .await?;
    let mut source_count: HashMap<(ScrapeSource, Option<String>), usize> = Default::default();
    for story in stories {
        for source in story.sources {
            if let Some(source) = source {
                *source_count
                    .entry((source.source, source.subsource))
                    .or_default() += 1;
            }
        }
    }
    let source_count: Vec<_> = source_count.into_iter().collect();
    let now = now(&index).await?;
    let top_tags = index.top_tags(usize::MAX)?;
    let storage = index.story_count().await?;
    let metrics = render(
        &resources,
        "metrics.txt",
        context!(source_count, storage, top_tags, now),
    )?;

    Ok((
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("text/plain")),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=300, s-max-age=300"),
            ),
        ],
        metrics,
    ))
}

async fn admin(
    Extension(user): Extension<CurrentUser>,
    State(AdminState { resources, .. }): State<AdminState>,
) -> Result<impl IntoResponse, WebError> {
    render_admin(Some(&user), &resources, "admin/admin.html", context!(user))
}

async fn admin_cron(
    Extension(user): Extension<CurrentUser>,
    State(AdminState {
        cron,
        cron_history,
        resources,
        ..
    }): State<AdminState>,
) -> Result<impl IntoResponse, WebError> {
    render_admin(
        Some(&user),
        &resources,
        "admin/cron.html",
        context!(
            user,
            cron = cron.read().inspect(),
            history = cron_history.read().entries()
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
    let success = cron.write().trigger(params.cron);
    Ok(success.into())
}

async fn admin_cron_backup(
    State(AdminState {
        backup_path, index, ..
    }): State<AdminState>,
) -> Result<Json<impl Serialize>, WebError> {
    let results = if let Some(backup_path) = backup_path {
        index.backup(&backup_path)?
    } else {
        vec![]
    }
    .into_iter()
    .map(|(shard, r)| (shard, r.map_err(|e| e.to_string())))
    .collect_vec();

    Ok(Json(results))
}

async fn admin_cron_refresh(
    State(AdminState {
        resources, index, ..
    }): State<AdminState>,
) -> Result<impl IntoResponse, WebError> {
    let start = Instant::now();
    index.refresh_hot_set().await?;
    let elapsed_ms = start.elapsed().as_millis();
    tracing::info!("Hotset refresh: time={elapsed_ms}ms");
    render_admin(
        None,
        &resources,
        "admin/cron_refresh.html",
        context!(elapsed_ms),
    )
}

async fn admin_cron_reindex(
    State(AdminState {
        resources, index, ..
    }): State<AdminState>,
) -> Result<impl IntoResponse, WebError> {
    let start = Instant::now();
    let results = index.reindex_hot_set().await?;
    let elapsed_ms = start.elapsed().as_millis();
    let summary = results.summary();
    tracing::info!("Hotset reindex: time={elapsed_ms}ms result={summary:?}");
    render_admin(
        None,
        &resources,
        "admin/cron_reindex.html",
        context!(results, elapsed_ms, summary),
    )
}

async fn admin_cron_scrape(
    State(AdminState {
        resources, index, ..
    }): State<AdminState>,
    Path(source): Path<ScrapeSource>,
) -> Result<impl IntoResponse, WebError> {
    let start = Instant::now();
    let subsources = resources.scrapers.read().compute_scrape_subsources(source);
    let urls = resources
        .scrapers
        .read()
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
    let fetch_ms = start.elapsed().as_millis();

    let start = Instant::now();
    let scrapes = HashMap::from_iter(
        map.into_iter()
            .map(|(k, v)| (k, resources.scrapers.read().scrape_http_result(source, v))),
    );
    let process_ms = start.elapsed().as_millis();

    let start = Instant::now();
    let mut summary = ScrapePersistResultSummary::default();
    let mut errors = 0;
    for result in scrapes.values() {
        match result {
            ScraperHttpResult::Ok(_, scrapes) => {
                let res = index.insert_scrapes(scrapes.clone()).await?;
                summary += res.summary();
            }
            ScraperHttpResult::Err(..) => {
                errors += 1;
            }
        }
    }
    let insert_ms = start.elapsed().as_millis();

    tracing::info!("Scrape source={source:?} fetch_time={fetch_ms}ms process_time={process_ms}ms insert_time={insert_ms}ms errors={errors} result={summary:?}");

    render_admin(
        None,
        &resources,
        "admin/cron_scrape_run.html",
        context!(source, scrapes: HashMap<String, ScraperHttpResult>, summary, fetch_ms, process_ms, insert_ms,),
    )
}

async fn admin_headers(
    Extension(user): Extension<CurrentUser>,
    State(AdminState { resources, .. }): State<AdminState>,
    Query(query): Query<HashMap<String, String>>,
    raw_headers: HeaderMap,
) -> Result<impl IntoResponse, WebError> {
    let mut headers: HashMap<_, Vec<String>> = HashMap::new();
    for (header, value) in raw_headers {
        let name = header.map(|h| h.to_string()).unwrap_or("(missing)".into());
        headers
            .entry(name)
            .or_default()
            .push(String::from_utf8_lossy(value.as_bytes()).to_string());
    }
    render_admin(
        Some(&user),
        &resources,
        "admin/headers.html",
        context!(user, query, headers),
    )
}

async fn admin_scrape(
    Extension(user): Extension<CurrentUser>,
    State(AdminState { resources, .. }): State<AdminState>,
) -> Result<impl IntoResponse, WebError> {
    render_admin(
        Some(&user),
        &resources,
        "admin/scrape.html",
        context!(
            user,
            scrapes = resources.scrapers.read().compute_scrape_possibilities(),
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
) -> Result<impl IntoResponse, WebError> {
    let urls = resources
        .scrapers
        .read()
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

    let scrapes = HashMap::from_iter(map.into_iter().map(|(k, v)| {
        (
            k,
            resources
                .scrapers
                .read()
                .scrape_http_result(params.source, v),
        )
    }));

    render_admin(
        Some(&user),
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
) -> Result<impl IntoResponse, WebError> {
    render_admin(
        Some(&user),
        &resources,
        "admin/status.html",
        context!(user, storage = index.story_count().await?,),
    )
}

async fn admin_status_frontpage(
    Extension(user): Extension<CurrentUser>,
    State(AdminState {
        index, resources, ..
    }): State<AdminState>,
    sort: Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, WebError> {
    let now = now(&index).await?;
    let sort = sort.get("sort").cloned().unwrap_or_default();
    let stories = index
        .stories::<StoryRender>(Option::<String>::None, 0, 500)
        .await?;
    render_admin(
        Some(&user),
        &resources,
        "admin/frontpage.html",
        context!(now, user, stories, sort),
    )
}

async fn admin_index_frontpage_scoretuner(
    Extension(user): Extension<CurrentUser>,
    State(AdminState {
        index, resources, ..
    }): State<AdminState>,
) -> Result<impl IntoResponse, WebError> {
    let now = now(&index).await?;

    #[derive(Serialize)]
    struct StoryDetail {
        story: StoryRender,
        score_detail: Vec<(StoryScore, f32)>,
    }

    let stories: Vec<Story<TypedScrape>> = index.fetch(StoryQuery::FrontPage(), 500).await?;
    let mut story_details = vec![];
    let eval = resources.story_evaluator.read();

    for mut story in stories {
        let scrapes = ScrapeCollection::new_from_iter(story.scrapes.values().cloned());
        let extracted = scrapes.extract(&eval.extractor);
        story.score = eval.scorer.score(&extracted) + eval.scorer.score_age(now - story.date);
        let mut tags = TagSet::from_iter(extracted.tags());
        eval.tagger.tag(extracted.title(), &mut tags);
        story.tags = tags;
        story_details.push(StoryDetail {
            story: story.render(&eval, 0),
            score_detail: eval.scorer.score_detail(&extracted, now),
        });
    }

    // Quick-and-dirty float sort
    story_details.sort_by_cached_key(|x| (x.story.score * -1000.0) as i32);

    render_admin(
        Some(&user),
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
) -> Result<impl IntoResponse, WebError> {
    let sort = sort.get("sort").cloned().unwrap_or_default();
    let shard = Shard::from_string(&shard).expect("Failed to parse shard");
    let stories = index
        .fetch::<Shard>(StoryQuery::ByShard(shard), usize::MAX)
        .await?;
    let stories = render_stories(&resources.story_evaluator.read(), stories.iter());
    render_admin(
        Some(&user),
        &resources,
        "admin/shard.html",
        context!(user, shard = shard, stories, sort: String = sort),
    )
}

async fn admin_status_story(
    Extension(user): Extension<CurrentUser>,
    State(AdminState {
        index, resources, ..
    }): State<AdminState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, WebError> {
    let id = StoryIdentifier::from_base64(id).ok_or(WebError::NotFound)?;
    let now = now(&index).await?;
    tracing::info!("Loading story = {:?}", id);
    let story = index
        .fetch_one(StoryQuery::ById(id.clone()))
        .await?
        .ok_or(WebError::NotFound)?;
    let scrapes = ScrapeCollection::new_from_iter(story.scrapes.clone().into_values());

    let eval = resources.story_evaluator.clone();
    let extract = scrapes.extract(&eval.read().extractor);
    let score_details = eval.read().scorer.score_detail(&extract, now);
    let tags = Default::default(); // _details = resources.story_evaluator.tagger.tag_detail(&story);
    let doc = index.fetch_detail_one(id).await?.unwrap_or_default();
    let story = story.render(&eval.read(), 0);

    render_admin(
        Some(&user),
        &resources,
        "admin/story.html",
        context!(
            now,
            user,
            scrapes = scrapes.scrapes,
            tags: HashMap<String, Vec<String>>,
            score = score_details,
            doc,
            story,
        ),
    )
}

pub async fn serve_static_files_immutable(
    headers_in: HeaderMap,
    Path(key): Path<String>,
    State(resources): State<Resources>,
) -> Result<impl IntoResponse, WebError> {
    serve_static_files::immutable(headers_in, key, &resources.static_files.read())
}

pub async fn serve_static_files_well_known(
    headers_in: HeaderMap,
    Path(file): Path<String>,
    State(resources): State<Resources>,
) -> Result<impl IntoResponse, WebError> {
    serve_static_files::well_known(headers_in, file, &resources.static_files_root.read())
}
