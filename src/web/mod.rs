use std::{net::SocketAddr, io::BufReader, path::Path, sync::Arc};

use axum::{Router, routing::{get, post}, response::{IntoResponse, Response, Html}, Json, extract::{State, self}, body::{HttpBody, StreamBody, Bytes}, http::HeaderValue};
use hyper::{StatusCode, HeaderMap, Body, header};
use num_format::ToFormattedString;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tera::{Tera, Context};
use lazy_static::lazy_static;
use thiserror::Error;

use crate::persist::StorageSummary;

use self::static_files::StaticFileRegistry;

mod index;
mod filters;
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
    InvalidHeader(#[from] hyper::header::InvalidHeaderValue)
}

impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        let body = format!("Error: {:?}", self);
        (StatusCode::INTERNAL_SERVER_ERROR, body).into_response()
    }
}

lazy_static! {
    pub static ref TEMPLATES: Tera = create_templates();
    pub static ref STATIC_FILES: Arc<StaticFileRegistry> = Arc::new(create_static_files().expect("Failed to read static files"));
}

fn create_templates() -> Tera {
    let mut tera = match Tera::new("templates/**/*") {
        Ok(t) => t,
        Err(e) => {
            println!("Parsing error(s): {}", e);
            ::std::process::exit(1);
        }
    };
    tera.register_filter("comma", filters::CommaFilter::default());
    tera
}

fn create_static_files() -> Result<StaticFileRegistry, WebError> {
    let mut static_files = StaticFileRegistry::default();
    let static_root = Path::new("static/");
    for file in std::fs::read_dir(static_root)? {
        let file = file?.file_name();
        let name = Path::new(&file);
        let ext = name.extension().expect("Static file did not have an extension").to_string_lossy();
        static_files.register(&file.to_string_lossy(), &ext, static_root.join(name))?;
    }
    Ok(static_files)
}

pub async fn start_server() -> Result<(), WebError> {
    // initialize tracing
    tracing_subscriber::fmt::init();
    TEMPLATES.check_macro_files()?;

    let global = index::initialize_with_testing_data()?;

    // build our application with a route
    let app = Router::new()
        .route("/", get(root))
        .route("/static/:file", get(serve_static_file)).with_state(STATIC_FILES.clone())
        .route("/admin/status", get(status)).with_state(global)
        .route("/admin/templates/reload", get(reload_templates))
        .route("/users", post(create_user));

    // run our app with hyper
    // `axum::Server` is a re-export of `hyper::Server`
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::info!("listening on http://{}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

// basic handler that responds with a static string
async fn root() -> &'static str {
    "Hello, World!"
}

async fn status(State(state): State<index::Global>) -> Result<Html<String>, WebError> {
    let context = Context::from_serialize(&Status { storage: state.storage.story_count()? })?;
    Ok(TEMPLATES.render("status.html", &context)?.into())
}

async fn serve_static_file(headers_in: HeaderMap, extract::Path(key): extract::Path<String>, State(static_files): State<Arc<StaticFileRegistry>>) -> Result<(StatusCode, HeaderMap, impl IntoResponse), WebError> {
    let mut headers = HeaderMap::new();
    headers.append(header::ETAG, key.parse()?);
    headers.append(header::SERVER, "progscrape".parse()?);

    if let Some(etag) = headers_in.get(header::IF_NONE_MATCH) {
        if *etag == key {
            return Ok((StatusCode::NOT_MODIFIED, headers, Response::new(axum::body::Full::new(Default::default()))))
        }
    }

    if let Some(bytes) = static_files.get_bytes_from_key(&key) {
        Ok((StatusCode::OK, headers, Response::new(axum::body::Full::new(bytes))))
    } else {
        Ok((StatusCode::NOT_FOUND, headers, Response::new(axum::body::Full::new(Default::default()))))
    }
}

async fn reload_templates() -> &'static str {
    // TEMPLATES.full_reload();
    "Reloaded templates!"
}

async fn create_user(
    // this argument tells axum to parse the request body
    // as JSON into a `CreateUser` type
    Json(payload): Json<CreateUser>,
) -> impl IntoResponse {
    // insert your application logic here
    let user = User {
        id: 1337,
        username: payload.username,
    };

    // this will be converted into a JSON response
    // with a status code of `201 Created`
    (StatusCode::CREATED, Json(user))
}

// the input to our `create_user` handler
#[derive(Deserialize)]
struct CreateUser {
    username: String,
}

// the output to our `create_user` handler
#[derive(Serialize)]
struct User {
    id: u64,
    username: String,
}

#[derive(Serialize)]
struct Status {
    storage: StorageSummary,
}
