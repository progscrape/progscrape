use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufReader;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Instant;

use clap::{Parser, Subcommand};
use config::Config;
use progscrape_application::{
    MemIndex, PersistLocation, Storage, StorageWriter, StoryEvaluator, StoryIndex,
};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::EnvFilter;
use web::WebError;

use crate::auth::Auth;
use crate::index::Index;
use crate::resource::Resources;

mod auth;
mod config;
mod cron;
mod filters;
mod index;
mod resource;
mod serve_static_files;
mod smoketest;
mod static_files;
mod story;
mod web;

pub enum Engine {}

#[derive(Parser, Debug)]
struct Args {
    #[arg(
        long,
        value_name = "LOG",
        help = "Logging filter (overrides SERVER_LOG environment variable)"
    )]
    log: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    Backup {
        #[arg(long, value_name = "DIR", value_hint = clap::ValueHint::DirPath, help = "Persistence path")]
        persist_path: PathBuf,

        #[arg(long, value_name = "DIR", value_hint = clap::ValueHint::DirPath, help = "Backup output path")]
        backup_path: PathBuf,

        #[arg(long, value_name = "DIR", value_hint = clap::ValueHint::DirPath, help = "Root path")]
        root: Option<PathBuf>,
    },
    Serve {
        #[arg(long, value_name = "DIR", value_hint = clap::ValueHint::DirPath, help = "Persistence path")]
        persist_path: Option<PathBuf>,

        #[arg(long, value_name = "DIR", value_hint = clap::ValueHint::DirPath, help = "Backup output path")]
        backup_path: Option<PathBuf>,

        #[arg(long, value_name = "DIR", value_hint = clap::ValueHint::DirPath, help = "Root path")]
        root: Option<PathBuf>,

        #[arg(long, value_name = "ADDRESS", help = "Listen port")]
        listen_port: Option<String>,

        #[arg(
            long,
            value_name = "HEADER",
            help = "Header to extract authorization from"
        )]
        auth_header: Option<String>,

        #[arg(
            long,
            value_name = "HEADER",
            help = "Fixed authorization value for testing purposes"
        )]
        fixed_auth_value: Option<String>,

        #[arg(
            long,
            value_name = "HEADER",
            help = "Metrics authorization bearer token"
        )]
        metrics_auth_bearer_token: Option<String>,
    },
    Initialize {
        #[arg(long, value_name = "DIR", value_hint = clap::ValueHint::DirPath, help = "Persistence path")]
        persist_path: PathBuf,

        #[arg(long, value_name = "DIR", value_hint = clap::ValueHint::DirPath, help = "Root path")]
        root: Option<PathBuf>,

        input: Vec<PathBuf>,
    },
    Load {
        #[arg(long, value_name = "DIR", value_hint = clap::ValueHint::DirPath, help = "Persistence path")]
        persist_path: PathBuf,

        #[arg(long, value_name = "DIR", value_hint = clap::ValueHint::DirPath, help = "Root path")]
        root: Option<PathBuf>,

        #[arg(long, help = "Import only these year(s)")]
        year: Vec<usize>,

        input: Vec<PathBuf>,
    },
}

/// Our entry point.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    go().await?;
    Ok(())
}

async fn go() -> Result<(), WebError> {
    let args = Args::parse();

    // We ask for more detailed tracing in debug mode
    let default_directive = if cfg!(debug_assertions) {
        LevelFilter::DEBUG.into()
    } else {
        LevelFilter::INFO.into()
    };

    // Initialize logging using either the environment variable or --log option
    let env_filter = if let Some(log) = args.log {
        EnvFilter::builder()
            .with_default_directive(default_directive)
            .parse(log)?
    } else {
        EnvFilter::builder()
            .with_default_directive(default_directive)
            .with_env_var("SERVER_LOG")
            .from_env()?
    };

    tracing_subscriber::fmt().with_env_filter(env_filter).init();
    tracing::info!("Logging initialized");

    match args.command {
        Command::Backup {
            persist_path,
            backup_path,
            root,
        } => {
            let root_path = root.unwrap_or(".".into()).canonicalize()?;
            tracing::info!("Root path: {}", root_path.to_string_lossy());
            let resource_path = root_path.join("resource");
            let resources = Resources::get_resources(resource_path)?;
            let index = Index::initialize_with_persistence(
                persist_path,
                resources.story_evaluator.clone(),
            )?;
            index.backup(&backup_path)?;
        }
        Command::Serve {
            root,
            persist_path,
            auth_header,
            fixed_auth_value,
            metrics_auth_bearer_token,
            listen_port,
            backup_path,
        } => {
            let root_path = root.unwrap_or(".".into()).canonicalize()?;
            tracing::info!("Root path: {}", root_path.to_string_lossy());

            let resource_path = root_path.join("resource");

            let resources = Resources::start_watcher(resource_path).await?;

            let persist_path = persist_path
                .unwrap_or("target/index".into())
                .canonicalize()?;
            tracing::info!("Persist path: {}", persist_path.to_string_lossy());
            let index = Index::initialize_with_persistence(
                persist_path,
                resources.story_evaluator.clone(),
            )?;
            let listen_port = listen_port
                .map(|s| s.parse().expect("Failed to parse socket address"))
                .unwrap_or(SocketAddr::from(([127, 0, 0, 1], 3000)));

            let auth = match (auth_header, fixed_auth_value) {
                (Some(auth_header), None) => Auth::FromHeader(auth_header),
                (None, Some(fixed_auth_value)) => Auth::Fixed(fixed_auth_value),
                (None, None) => Auth::None,
                _ => {
                    return Err(WebError::ArgumentsInvalid(
                        "Invalid auth header parameter".into(),
                    ));
                }
            };
            web::start_server(
                resources,
                backup_path,
                listen_port,
                index,
                auth,
                metrics_auth_bearer_token,
            )
            .await?;
        }
        Command::Initialize {
            root,
            persist_path,
            input,
        } => {
            if persist_path.exists() {
                return Err(WebError::ArgumentsInvalid(format!(
                    "Path {} must not exist",
                    persist_path.to_string_lossy()
                )));
            };
            std::fs::create_dir_all(&persist_path)?;
            let resource_path = root.unwrap_or(".".into()).canonicalize()?.join("resource");
            let reader = BufReader::new(File::open(resource_path.join("config/config.json"))?);
            let config: Config = serde_json::from_reader(reader)?;
            let eval = StoryEvaluator::new(&config.tagger, &config.score, &config.scrape);

            let start = Instant::now();

            let import_start = Instant::now();
            let import_time = import_start.elapsed();

            // First, build an in-memory index quickly
            let memindex_start = Instant::now();
            let mut memindex = MemIndex::default();

            for input in input {
                tracing::info!("Importing from {}...", input.to_string_lossy());
                let scrapes = progscrape_scrapers::import_backup(&input)?;
                memindex.insert_scrapes(scrapes)?;
            }
            let memindex_time = memindex_start.elapsed();

            // Now, import those stories
            let story_start = Instant::now();
            let mut index = StoryIndex::new(PersistLocation::Path(persist_path))?;
            index.insert_scrape_collections(&eval, memindex.get_all_stories())?;
            let story_index_time = story_start.elapsed();

            let count = index.story_count()?;
            tracing::info!("Shard   | Count");
            for (shard, count) in &count.by_shard {
                tracing::info!("{} | {}", shard, count.story_count);
            }

            tracing::info!(
                "Completed init in {}s (import={}s, memindex={}s, storyindex={}s)",
                start.elapsed().as_secs(),
                import_time.as_secs(),
                memindex_time.as_secs(),
                story_index_time.as_secs()
            );
        }
        Command::Load {
            persist_path,
            root,
            input,
            year,
        } => {
            let resource_path = root.unwrap_or(".".into()).canonicalize()?.join("resource");
            let reader = BufReader::new(File::open(resource_path.join("config/config.json"))?);
            let config: Config = serde_json::from_reader(reader)?;
            let eval = StoryEvaluator::new(&config.tagger, &config.score, &config.scrape);
            let mut index = StoryIndex::new(PersistLocation::Path(persist_path))?;
            let years: HashSet<usize> = HashSet::from_iter(year);

            for input in input {
                tracing::info!("Importing from {}...", input.to_string_lossy());
                let mut scrapes = progscrape_scrapers::import_backup(&input)?;
                if !years.is_empty() {
                    let size_before = scrapes.len();
                    scrapes.retain(|story| years.contains(&(story.date.year() as usize)));
                    tracing::info!(
                        "Filtered out {} stories not matching the specified years",
                        size_before - scrapes.len()
                    );
                }
                let res = index.insert_scrapes(&eval, scrapes)?;
                let mut result_count = HashMap::<_, usize>::new();
                for res in &res {
                    result_count.entry(res).and_modify(|x| *x += 1).or_default();
                }
                tracing::info!("Results: total={} {:?}", res.len(), result_count);
            }
        }
    };
    Ok(())
}
