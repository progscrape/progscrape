use std::{net::SocketAddr, path::Path, sync::Arc};

use axum::{
    extract::{self, State},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use hyper::StatusCode;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use tera::{Context, Tera};
use thiserror::Error;

use crate::persist::StorageSummary;

use self::static_files::StaticFileRegistry;

mod filters;
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
}

impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        let body = format!("Error: {:?}", self);
        (StatusCode::INTERNAL_SERVER_ERROR, body).into_response()
    }
}

lazy_static! {
    pub static ref TEMPLATES: Tera = create_templates();
    pub static ref STATIC_FILES: Arc<StaticFileRegistry> =
        Arc::new(create_static_files().expect("Failed to read static files"));
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
    tera.register_filter(
        "static",
        filters::StaticFileFilter::new(STATIC_FILES.clone()),
    );
    tera
}

fn create_static_files() -> Result<StaticFileRegistry, WebError> {
    let mut static_files = StaticFileRegistry::default();
    let static_root = Path::new("static/");
    for file in std::fs::read_dir(static_root)? {
        let file = file?.file_name();
        let name = Path::new(&file);
        let ext = name
            .extension()
            .expect("Static file did not have an extension")
            .to_string_lossy();
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
        .route("/static/:file", get(serve_static_files::immutable))
        .with_state(STATIC_FILES.clone())
        .route("/admin/status", get(status))
        .with_state(global)
        .route("/admin/templates/reload", get(reload_templates))
        .route(
            "/:file",
            get(serve_static_files::well_known).with_state(STATIC_FILES.clone()),
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

// basic handler that responds with a static string
async fn root() -> &'static str {
    "Hello, World!"
}

async fn status(State(state): State<index::Global>) -> Result<Html<String>, WebError> {
    let context = Context::from_serialize(&Status {
        storage: state.storage.story_count()?,
    })?;
    Ok(TEMPLATES.render("status.html", &context)?.into())
}

async fn reload_templates() -> &'static str {
    // TEMPLATES.full_reload();
    "Reloaded templates!"
}

#[derive(Serialize)]
struct Status {
    storage: StorageSummary,
}
