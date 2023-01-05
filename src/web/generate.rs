use std::sync::Arc;
use std::path::Path;
use std::time::Duration;
use notify::RecursiveMode;
use notify::Watcher;
use tera::Tera;
use tokio::sync::watch;

use crate::web::static_files::StaticFileRegistry;
use crate::web::filters::*;
use crate::web::WebError;

#[derive(Clone)]
struct Generated {
    templates: Arc<Tera>,
    static_files: Arc<StaticFileRegistry>,
    static_files_root: Arc<StaticFileRegistry>,
}

#[derive(Clone)]
pub struct GeneratedSource {
    rx: watch::Receiver<Generated>,
}

impl GeneratedSource {
    pub fn templates(&self) -> Arc<Tera> {
        self.rx.borrow().templates.clone()
    }
    pub fn static_files(&self) -> Arc<StaticFileRegistry> {
        self.rx.borrow().static_files.clone()
    }
    pub fn static_files_root(&self) -> Arc<StaticFileRegistry> {
        self.rx.borrow().static_files_root.clone()
    }
}

fn create_static_files(css: String, admin_css: String) -> Result<StaticFileRegistry, WebError> {
    let mut static_files = StaticFileRegistry::default();
    static_files.register_files("static/")?;
    let mut css_vars = ":root {\n".to_owned();
    for key in static_files.keys() {
        let url = static_files.lookup_key(&key).unwrap_or_default();
        css_vars += &format!("--url-{}: url(\"{}\");\n", key.replace(".", "-"), url);
    }
    css_vars += "}\n";
    static_files.register_bytes("style.css", "css", (css_vars.clone() + &css).as_bytes())?;
    static_files.register_bytes("admin.css", "css", (css_vars + &admin_css).as_bytes())?;
    Ok(static_files)
}

fn create_static_files_root() -> Result<StaticFileRegistry, WebError> {
    let mut static_files = StaticFileRegistry::default();
    static_files.register_files("static/root/")?;
    Ok(static_files)
}

fn create_templates(static_files: Arc<StaticFileRegistry>) -> Result<Tera, WebError> {
    let mut tera = Tera::new("templates/**/*")?;
    tera.register_filter("comma", CommaFilter::default());
    tera.register_filter(
        "static",
        StaticFileFilter::new(static_files),
    );
    Ok(tera)
}

fn create_css() -> Result<String, WebError> {
    let opts = grass::Options::default()
        .input_syntax(grass::InputSyntax::Scss)
        .style(grass::OutputStyle::Expanded)
        .load_path("static/css/");
    let out = grass::from_string("@use 'root'".to_owned(), &opts)?;
    Ok(out)
}

fn create_admin_css() -> Result<String, WebError> {
    let opts = grass::Options::default()
        .input_syntax(grass::InputSyntax::Scss)
        .style(grass::OutputStyle::Expanded)
        .load_path("static/css/");
    let out = grass::from_string("@use 'admin'".to_owned(), &opts)?;
    Ok(out)
}

fn generate() -> Result<Generated, WebError> {
    let css = create_css()?;
    let admin_css = create_admin_css()?;
    let static_files = Arc::new(create_static_files(css, admin_css)?);
    let static_files_root = Arc::new(create_static_files_root()?);
    let templates = Arc::new(create_templates(static_files.clone())?);
    Ok(Generated { templates, static_files, static_files_root })
}

/// Starts a process to watch all the templates/static data and regenerates everything if something changes.
pub async fn start_watcher() -> Result<GeneratedSource, WebError> {
    let (tx, rx) = watch::channel(generate()?);
    let (tx_dirty, mut rx_dirty) = watch::channel(false);
    let mut watcher = notify::recommended_watcher(move |res| {
        if let Ok(event) = res {
            tracing::debug!("Got FS event: {:?}", event);
            let _ = tx_dirty.send(true);
        }
    })?;
    for path in ["templates", "static"] {
        tracing::info!("Watching path {}/...", path);
        watcher.watch(Path::new(path), RecursiveMode::Recursive)?;
    }
    tokio::spawn(async move {
        while let Ok(_) = rx_dirty.changed().await {
            tracing::info!("Noticed a change in watched paths!");
            while let Ok(_) = tokio::time::timeout(Duration::from_millis(100), rx_dirty.changed()).await {
                tracing::debug!("Debouncing extra event within timeout period");
            }
            tracing::info!("Regenerating...");
            let res =  tokio::task::spawn_blocking(|| {
                generate()
            }).await ;
            match res {
                Ok(Ok(v)) => drop(tx.send(v)),
                Ok(Err(e)) => tracing::error!("Failed to regenerate data: {:?}", e),
                _ => {},
            };
            tracing::info!("Done!");
        }
        // Keep the watcher alive in this task
        drop(watcher);
    });
    Ok(GeneratedSource { rx })
}