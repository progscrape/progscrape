use notify::RecursiveMode;
use notify::Watcher;
use progscrape_scrapers::Scrapers;
use std::borrow::Borrow;
use std::fs::File;
use std::io::BufReader;

use std::path::Path;
use std::time::Duration;
use tera::Tera;
use tokio::sync::watch;

use progscrape_application::StoryEvaluator;

use crate::config::Config;
use crate::filters::*;
use crate::static_files::StaticFileRegistry;
use crate::types::Shared;
use crate::web::WebError;

struct ResourceHolder {
    templates: Tera,
    static_files: Shared<StaticFileRegistry>,
    static_files_root: StaticFileRegistry,
    config: Config,
    story_evaluator: StoryEvaluator,
    scrapers: Scrapers,
}

#[derive(Clone)]
pub struct Resources {
    rx: watch::Receiver<Shared<ResourceHolder>>,
}

impl Resources {
    pub fn templates(&self) -> Shared<Tera> {
        self.rx.borrow().project_fn(|x| &x.templates)
    }
    pub fn static_files(&self) -> Shared<StaticFileRegistry> {
        self.rx.borrow().project_fn(|x| &*x.static_files)
    }
    pub fn static_files_root(&self) -> Shared<StaticFileRegistry> {
        self.rx.borrow().project_fn(|x| &x.static_files_root)
    }
    pub fn config(&self) -> Shared<Config> {
        self.rx.borrow().project_fn(|x| &x.config)
    }
    pub fn story_evaluator(&self) -> Shared<StoryEvaluator> {
        self.rx.borrow().project_fn(|x| &x.story_evaluator)
    }
    pub fn scrapers(&self) -> Shared<Scrapers> {
        self.rx.borrow().project_fn(|x| &x.scrapers)
    }
}

fn create_static_files(
    resource_path: &Path,
    css: String,
    admin_css: String,
) -> Result<StaticFileRegistry, WebError> {
    let mut static_files = StaticFileRegistry::default();
    static_files.register_files(resource_path.join("static/"))?;
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
    static_files: Shared<StaticFileRegistry>,
) -> Result<Tera, WebError> {
    let mut tera = Tera::new(
        resource_path
            .join("templates/**/*")
            .to_string_lossy()
            .borrow(),
    )?;

    tera.register_filter("comma", CommaFilter::default());

    tera.register_filter("rfc_3339", RFC3339Filter::default());
    tera.register_filter("relative_time", RelativeTimeFilter::default());
    tera.register_filter("absolute_time", AbsoluteTimeFilter::default());
    tera.register_filter("approx_time", ApproxTimeFilter::default());

    tera.register_filter("comment_link", CommentLinkFilter::default());

    tera.register_filter("static", StaticFileFilter::new(static_files));

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

fn generate<T: AsRef<Path>>(resource_path: T) -> Result<Shared<ResourceHolder>, WebError> {
    let resource_path = resource_path.as_ref();
    if !resource_path.exists() {
        tracing::error!(
            "Root resources path does not exist: {} (cwd was {})",
            resource_path.to_string_lossy(),
            std::env::current_dir()
                .unwrap_or_default()
                .to_string_lossy()
        );
        return Err(WebError::NotFound);
    }

    let resource_path = &resource_path.canonicalize()?;
    let css = create_css(resource_path)?;
    let admin_css = create_admin_css(resource_path)?;
    let static_files = Shared::new(create_static_files(resource_path, css, admin_css)?);
    let static_files_root = create_static_files_root(resource_path)?;
    let templates = create_templates(resource_path, static_files.clone())?;
    let config = create_config(resource_path)?;
    let story_evaluator = StoryEvaluator::new(&config.tagger, &config.score, &config.scrape);
    let scrapers = Scrapers::new(&config.scrape);
    Ok(Shared::new(ResourceHolder {
        templates,
        static_files,
        static_files_root,
        config,
        story_evaluator,
        scrapers,
    }))
}

impl Resources {
    /// Returns a `Resources` object that doesn't watch a file path.
    #[cfg(test)]
    pub fn get_resources<T: AsRef<Path>>(resource_path: T) -> Result<Resources, WebError> {
        let (_, rx) = watch::channel(generate(resource_path)?);
        Ok(Resources { rx })
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
            while rx_dirty.changed().await.is_ok() {
                let resource_path = resource_path.clone();
                tracing::info!("Noticed a change in watched paths!");
                while tokio::time::timeout(Duration::from_millis(100), rx_dirty.changed())
                    .await
                    .is_ok()
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
}
