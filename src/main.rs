mod config;
mod datasci;
mod persist;
mod scrapers;
mod story;
mod web;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    web::start_server().await?;
    Ok(())
}
