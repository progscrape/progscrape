use std::io::BufReader;
use std::fs::File;
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use config::Config;
use progscrape_application::{StoryIndex, PersistLocation, StorageWriter, StoryEvaluator};
use progscrape_scrapers::import_legacy;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::EnvFilter;

mod config;
mod cron;
mod filters;
mod index;
mod resource;
mod serve_static_files;
mod static_files;
mod web;

pub enum Engine {

}

#[derive(Parser, Debug)]
struct Args {
    #[arg(long, value_name = "LOG", help = "Logging filter (overrides SERVER_LOG environment variable)")]
    log: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    Serve {
        #[arg(long, value_name = "DIR", value_hint = clap::ValueHint::DirPath, help = "Persistence path")]
        persist_path: Option<PathBuf>,
    
        #[arg(long, value_name = "DIR", value_hint = clap::ValueHint::DirPath, help = "Root path")]
        root: Option<PathBuf>,
    },
    Initialize {
        #[arg(long, value_name = "DIR", value_hint = clap::ValueHint::DirPath, help = "Persistence path")]
        persist_path: PathBuf,

        #[arg(long, value_name = "DIR", value_hint = clap::ValueHint::DirPath, help = "Root path")]
        root: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // We ask for more detailed tracing in debug mode
    let default_directive = if cfg!(debug_assertions) {
        LevelFilter::DEBUG.into()
    } else {
        LevelFilter::INFO.into()
    };

    // Initialize logging using either the environment variable or --log option
    let env_filter = if let Some(log) = args.log {
        EnvFilter::builder().with_default_directive(default_directive).parse(log)?
    } else {
        EnvFilter::builder().with_default_directive(default_directive).with_env_var("SERVER_LOG").from_env()?
    };

    tracing_subscriber::fmt().with_env_filter(env_filter).init();
    tracing::info!("Logging initialized");

    match args.command {
        Command::Serve { root, .. } => {
            let root_path = root.unwrap_or(".".into()).canonicalize()?;
            web::start_server(&root_path).await?;
        },
        Command::Initialize { root, persist_path } => {
            let resource_path = root.unwrap_or(".".into()).canonicalize()?.join("resource");
            let reader = BufReader::new(File::open(resource_path.join("config/config.json"))?);
            let config: Config = serde_json::from_reader(reader)?;
            let eval = StoryEvaluator::new(&config.tagger, &config.score, &config.scrape);
        
            let mut index = StoryIndex::new(PersistLocation::Path(persist_path))?;
            index.insert_scrapes(&eval, import_legacy(Path::new("."))?.into_iter())?;
        }
    };
    Ok(())
}
