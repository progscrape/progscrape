use std::path::Path;

mod config;
mod web;
mod cron;
mod filters;
mod index;
mod resource;
mod serve_static_files;
mod static_files;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root_path = Path::new(".").canonicalize()?;
    web::start_server(&root_path).await?;
    Ok(())
}
