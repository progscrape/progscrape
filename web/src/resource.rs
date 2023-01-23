use notify::RecursiveMode;
use notify::Watcher;
use progscrape_scrapers::Scrapers;
use std::borrow::Borrow;
use std::fs::File;
use std::io::BufReader;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tera::Tera;
use tokio::sync::watch;

use progscrape_application::StoryEvaluator;

use crate::config::Config;
use crate::filters::*;
use crate::static_files::StaticFileRegistry;
use crate::web::WebError;

#[derive(Clone)]
struct ResourceHolder {
    templates: Arc<Tera>,
    static_files: Arc<StaticFileRegistry>,
    static_files_root: Arc<StaticFileRegistry>,
    config: Arc<Config>,
    story_evaluator: Arc<StoryEvaluator>,
    scrapers: Arc<Scrapers>,
}

#[derive(Clone)]
pub struct Resources {
    rx: watch::Receiver<ResourceHolder>,
}

impl Resources {
    pub fn templates(&self) -> Arc<Tera> {
        self.rx.borrow().templates.clone()
    }
    pub fn static_files(&self) -> Arc<StaticFileRegistry> {
        self.rx.borrow().static_files.clone()
    }
    pub fn static_files_root(&self) -> Arc<StaticFileRegistry> {
        self.rx.borrow().static_files_root.clone()
    }
    pub fn config(&self) -> Arc<Config> {
        self.rx.borrow().config.clone()
    }
    pub fn story_evaluator(&self) -> Arc<StoryEvaluator> {
        self.rx.borrow().story_evaluator.clone()
    }
    pub fn scrapers(&self) -> Arc<Scrapers> {
        self.rx.borrow().scrapers.clone()
    }
}

fn create_static_files(
    resource_path: &Path,
    css: String,
    admin_css: String,
) -> Result<StaticFileRegistry, WebError> {
    let mut static_files = StaticFileRegistry::default();
    static_files.register_files("resource/static/")?;
    let mut css_vars = ":root {\n".to_owned();
    for key in static_files.keys() {
        let url = static_files.lookup_key(&key).unwrap_or_default();
        css_vars += &format!("--url-{}: url(\"{}\");\n", key.replace('.', "-"), url);
    }
    css_vars += "}\n";
    static_files.register_bytes("style.css", "css", (css_vars.clone() + &css).as_bytes())?;
    static_files.register_bytes("admin.css", "css", (css_vars + &admin_css).as_bytes())?;
    Ok(static_files)
}

fn create_static_files_root(resource_path: &Path) -> Result<StaticFileRegistry, WebError> {
    let mut static_files = StaticFileRegistry::default();
    static_files.register_files(resource_path.join("static/root/"))?;
    Ok(static_files)
}

fn create_templates(
    resource_path: &Path,
    static_files: Arc<StaticFileRegistry>,
) -> Result<Tera, WebError> {
    let mut tera = Tera::new(
        resource_path
            .join("templates/**/*")
            .to_string_lossy()
            .borrow(),
    )?;
    tera.register_filter("comma", CommaFilter::default());
    tera.register_filter("static", StaticFileFilter::new(static_files));
    tera.register_filter("relative_time", RelativeTimeFilter::default());
    tera.register_filter("absolute_time", AbsoluteTimeFilter::default());
    Ok(tera)
}

fn create_css(resource_path: &Path) -> Result<String, WebError> {
    let opts = grass::Options::default()
        .input_syntax(grass::InputSyntax::Scss)
        .style(grass::OutputStyle::Expanded)
        .load_path(resource_path.join("static/css/"));
    let out = grass::from_string("@use 'root'".to_owned(), &opts)?;
    Ok(out)
}

fn create_admin_css(resource_path: &Path) -> Result<String, WebError> {
    let opts = grass::Options::default()
        .input_syntax(grass::InputSyntax::Scss)
        .style(grass::OutputStyle::Expanded)
        .load_path(resource_path.join("static/css/"));
    let out = grass::from_string("@use 'admin'".to_owned(), &opts)?;
    Ok(out)
}

fn create_config(resource_path: &Path) -> Result<Config, WebError> {
    let reader = BufReader::new(File::open(resource_path.join("config/config.json"))?);
    Ok(serde_json::from_reader(reader)?)
}

fn generate<T: AsRef<Path>>(resource_path: T) -> Result<ResourceHolder, WebError> {
    let resource_path = resource_path.as_ref();
    let css = create_css(resource_path)?;
    let admin_css = create_admin_css(resource_path)?;
    let static_files = Arc::new(create_static_files(resource_path, css, admin_css)?);
    let static_files_root = Arc::new(create_static_files_root(resource_path)?);
    let templates = Arc::new(create_templates(resource_path, static_files.clone())?);
    let config = Arc::new(create_config(resource_path)?);
    let story_evaluator = Arc::new(StoryEvaluator::new(
        &config.tagger,
        &config.score,
        &config.scrape,
    ));
    let scrapers = Arc::new(Scrapers::new(&config.scrape));
    Ok(ResourceHolder {
        templates,
        static_files,
        static_files_root,
        config,
        story_evaluator,
        scrapers,
    })
}

/// Starts a process to watch all the templates/static data and regenerates everything if something changes.
pub async fn start_watcher<T: AsRef<Path>>(resource_path: T) -> Result<Resources, WebError> {
    let resource_path = resource_path.as_ref();
    let (tx, rx) = watch::channel(generate(resource_path)?);
    let (tx_dirty, mut rx_dirty) = watch::channel(false);
    let mut watcher = notify::recommended_watcher(move |res| {
        if let Ok(event) = res {
            tracing::debug!("Got FS event: {:?}", event);
            let _ = tx_dirty.send(true);
        }
    })?;
    tracing::info!("Watching path {}...", resource_path.to_string_lossy());
    watcher.watch(resource_path, RecursiveMode::Recursive)?;

    let resource_path = resource_path.to_owned();
    tokio::spawn(async move {
        while let Ok(_) = rx_dirty.changed().await {
            let resource_path = resource_path.clone();
            tracing::info!("Noticed a change in watched paths!");
            while let Ok(_) =
                tokio::time::timeout(Duration::from_millis(100), rx_dirty.changed()).await
            {
                tracing::debug!("Debouncing extra event within timeout period");
            }
            tracing::info!("Regenerating...");
            let res = tokio::task::spawn_blocking(move || generate(resource_path)).await;
            match res {
                Ok(Ok(v)) => drop(tx.send(v)),
                Ok(Err(e)) => tracing::error!("Failed to regenerate data: {:?}", e),
                _ => {}
            };
            tracing::info!("Done!");
        }
        // Keep the watcher alive in this task
        drop(watcher);
    });
    Ok(Resources { rx })
}
