mod config;
mod web;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    web::start_server().await?;
    Ok(())
}
