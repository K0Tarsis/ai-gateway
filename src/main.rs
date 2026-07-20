mod auth;
mod config;
mod errors;
mod handlers;
mod middleware;
mod models;
mod providers;
mod routing;
mod state;
mod utils;

use axum::routing::post;
use axum::Router;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config = config::load("config.yaml")?;
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let state = state::AppState::new(config);

    let app = Router::new()
        .route("/v1/chat/completions", post(handlers::chat_completions))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!(%addr, "Gateway listening");
    axum::serve(listener, app).await?;

    Ok(())
}
