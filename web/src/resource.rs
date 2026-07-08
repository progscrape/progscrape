use itertools::Itertools;
use notify::RecursiveMode;
use notify::Watcher;
use progscrape_scrapers::Scrapers;
use progscrape_scrapers::StoryDate;
use progscrape_scrapers::StoryUrl;
use serde::Serialize;
use serde_json::Value;
use std::borrow::Borrow;
use std::fs::File;
use std::io::BufReader;

use keepcalm::{Shared, SharedMut};
use std::path::Path;
use std::time::Duration;
use tera::Tera;
use tokio::sync::watch;

use progscrape_application::StoryEvaluator;

use crate::config::Config;
use crate::filters::*;
use crate::rate_limits::RateLimits;
use crate::static_files::StaticFileRegistry;
use crate::web::WebError;

struct ResourceHolder {
    templates: Tera,
    static_files: StaticFileRegistry,
    static_files_root: StaticFileRegistry,
    blog_posts: Vec<BlogPost>,
    config: Config,
    story_evaluator: StoryEvaluator,
    scrapers: Scrapers,
    rate_limits: RateLimits,
}

#[derive(Clone)]
pub struct Resources {
    pub templates: Shared<Tera>,
    pub static_files: Shared<StaticFileRegistry>,
    pub static_files_root: Shared<StaticFileRegistry>,
    pub blog_posts: Shared<Vec<BlogPost>>,
    pub config: Shared<Config>,
    pub story_evaluator: Shared<StoryEvaluator>,
    pub scrapers: Shared<Scrapers>,
    pub rate_limits: SharedMut<RateLimits>,
}

impl Resources {
    /// Build from freshly-generated values. Each field is its own independent,
    /// `Arc`-backed `Shared` (lock-free reads, cheap clones, never in the
    /// deadlock graph) rather than a projection of one shared lock.
    fn from_holder(h: ResourceHolder) -> Self {
        Resources {
            templates: Shared::new(h.templates),
            static_files: Shared::new(h.static_files),
            static_files_root: Shared::new(h.static_files_root),
            blog_posts: Shared::new(h.blog_posts),
            config: Shared::new(h.config),
            story_evaluator: Shared::new(h.story_evaluator),
            scrapers: Shared::new(h.scrapers),
            rate_limits: SharedMut::new(h.rate_limits),
        }
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

#[derive(Serialize, Clone)]
pub struct BlogPost {
    pub id: String,
    pub title: String,
    pub url: StoryUrl,
    pub slug: String,
    pub date: StoryDate,
    pub html: String,
    pub tags: Vec<String>,
}

fn blog_posts(resource_path: &Path) -> Result<Vec<BlogPost>, WebError> {
    let blog = resource_path.join("blog");
    let mut opts = markdown::Options::gfm();
    opts.parse.constructs.frontmatter = true;
    opts.parse.constructs.html_flow = true;
    opts.parse.constructs.html_text = true;
    let mut posts = vec![];
    let err = || WebError::IOError(std::io::ErrorKind::InvalidData.into());
    for entry in std::fs::read_dir(blog)? {
        let entry = entry?;
        let id = entry
            .file_name()
            .to_string_lossy()
            .trim_end_matches(".md")
            .to_owned();
        let date = StoryDate::parse_from_rfc3339(&format!("{id}T00:00:00Z")).ok_or_else(err)?;
        let contents = std::fs::read_to_string(entry.path().canonicalize()?)?;
        let title = contents
            .split('\n')
            .find(|line| line.starts_with("title:"))
            .ok_or_else(err)?
            .trim_start_matches("title:")
            .trim()
            .to_owned();
        let mut tags = contents
            .split('\n')
            .find(|line| line.starts_with("tags:"))
            .ok_or_else(err)?
            .trim_start_matches("tags:")
            .split(',')
            .map(|s| s.trim().to_owned())
            .collect_vec();
        tags.push("blog".to_owned());
        tags.push("progscrape".to_owned());
        let html =
            markdown::to_html_with_options(&contents, &opts).map_err(WebError::MarkdownError)?;
        let slug = title
            .to_ascii_lowercase()
            .replace(' ', "-")
            .replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '_', "");
        let url =
            StoryUrl::parse(format!("https://progscrape.com/blog/{id}/{slug}")).ok_or_else(err)?;
        posts.push(BlogPost {
            id,
            title,
            url,
            slug,
            date,
            html,
            tags,
        });
    }
    // Sort oldest last
    posts.sort_by(|a, b| b.date.cmp(&a.date));
    Ok(posts)
}

fn create_templates(
    resource_path: &Path,
    static_files: StaticFileRegistry,
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

fn merge_json(base: &mut Value, patch: Value) {
    match (base, patch) {
        (Value::Object(base_map), Value::Object(patch_map)) => {
            for (key, patch_value) in patch_map {
                match base_map.get_mut(&key) {
                    Some(base_value) => merge_json(base_value, patch_value),
                    None => {
                        base_map.insert(key, patch_value);
                    }
                }
            }
        }
        (base, patch) => *base = patch,
    }
}

fn create_config(resource_path: &Path, config: Option<&Path>) -> Result<Config, WebError> {
    let reader = BufReader::new(File::open(resource_path.join("config/config.json"))?);
    let mut value: Value = serde_json::from_reader(reader)?;

    if let Some(config_path) = config {
        if !config_path.is_file() {
            return Err(WebError::ArgumentsInvalid(format!(
                "Config override path is not a file: {} \
                 (if this is a directory, a Docker bind-mount source was probably missing)",
                config_path.to_string_lossy()
            )));
        }
        let reader = BufReader::new(File::open(config_path)?);
        let patch: Value = serde_json::from_reader(reader)?;
        merge_json(&mut value, patch);
    }

    Ok(serde_json::from_value(value)?)
}

fn generate(resource_path: &Path, config: Option<&Path>) -> Result<ResourceHolder, WebError> {
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
    let static_files = create_static_files(resource_path, css, admin_css)?;
    let static_files_root = create_static_files_root(resource_path)?;
    let templates = create_templates(resource_path, static_files.clone())?;
    let config = create_config(resource_path, config)?;
    let story_evaluator = StoryEvaluator::new(&config.tagger, &config.score, &config.scrape);
    let scrapers = Scrapers::new(&config.scrape);
    let blog_posts = blog_posts(resource_path)?;
    let rate_limits = RateLimits::new(&config.rate_limits);
    Ok(ResourceHolder {
        templates,
        static_files,
        static_files_root,
        config,
        story_evaluator,
        scrapers,
        blog_posts,
        rate_limits,
    })
}

impl Resources {
    /// Returns a `Resources` object that doesn't watch a file path.
    pub fn get_resources<T: AsRef<Path>>(resource_path: T) -> Result<Resources, WebError> {
        Ok(Resources::from_holder(generate(resource_path.as_ref(), None)?))
    }

    /// Returns a `Resources` object that doesn't watch a file path.
    pub fn get_resources_override<T: AsRef<Path>, U: AsRef<Path>>(
        resource_path: T,
        config: U,
    ) -> Result<Resources, WebError> {
        Ok(Resources::from_holder(generate(
            resource_path.as_ref(),
            Some(config.as_ref()),
        )?))
    }

    /// Starts a process to watch all the templates/static data and regenerates everything if something changes.
    pub async fn start_watcher<T: AsRef<Path>>(resource_path: T) -> Result<Resources, WebError> {
        let resource_path = resource_path.as_ref();
        let h = generate(resource_path, None)?;
        // Per-field shared-mut handles the watcher hot-swaps on change; handlers
        // see them as independent immutable `Shared` views (`shared_copy`).
        let templates = SharedMut::new(h.templates);
        let static_files = SharedMut::new(h.static_files);
        let static_files_root = SharedMut::new(h.static_files_root);
        let blog_posts = SharedMut::new(h.blog_posts);
        let config = SharedMut::new(h.config);
        let story_evaluator = SharedMut::new(h.story_evaluator);
        let scrapers = SharedMut::new(h.scrapers);
        let resources = Resources {
            templates: templates.shared_copy(),
            static_files: static_files.shared_copy(),
            static_files_root: static_files_root.shared_copy(),
            blog_posts: blog_posts.shared_copy(),
            config: config.shared_copy(),
            story_evaluator: story_evaluator.shared_copy(),
            scrapers: scrapers.shared_copy(),
            rate_limits: SharedMut::new(h.rate_limits),
        };

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
                let res =
                    tokio::task::spawn_blocking(move || generate(resource_path.as_ref(), None))
                        .await;
                match res {
                    Ok(Ok(v)) => {
                        templates.set(v.templates);
                        static_files.set(v.static_files);
                        static_files_root.set(v.static_files_root);
                        blog_posts.set(v.blog_posts);
                        config.set(v.config);
                        story_evaluator.set(v.story_evaluator);
                        scrapers.set(v.scrapers);
                        // rate_limits keeps its runtime state; not reloaded.
                    }
                    Ok(Err(e)) => tracing::error!("Failed to regenerate data: {:?}", e),
                    _ => {}
                };
                tracing::info!("Done!");
            }
            // Keep the watcher alive in this task
            drop(watcher);
        });
        Ok(resources)
    }
}
