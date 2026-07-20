mod auth;
mod config;
mod db;
mod errors;
mod handlers;
mod health;
mod metrics;
mod middleware;
mod models;
mod providers;
mod ratelimit;
mod routing;
mod state;
mod utils;

use std::net::SocketAddr;

use axum::Router;
use axum::routing::{get, post};
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
    let state = state::AppState::new(config).await?;

    // cost_limit only makes sense on the one route that actually spends
    // money — /v1/models and /v1/usage never do, so they'd pay for a DB
    // query on every call for no reason. Nested inside chat_routes so it
    // still gets rate_limit (cheap, checked first) too.
    let chat_completions_route = Router::new()
        .route("/v1/chat/completions", post(handlers::chat_completions))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            middleware::cost_limit,
        ));

    // /metrics needs require_api_key but not rate_limit — a Prometheus
    // scraper polling every ~15s shouldn't compete with a profile's request
    // budget — so it's built as a separate group merged before the auth
    // layer instead of sharing chat_routes' route_layer stack.
    let chat_routes = chat_completions_route
        .route("/v1/models", get(handlers::list_providers))
        .route("/v1/usage", get(handlers::usage))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            middleware::rate_limit,
        ));

    let metrics_routes = Router::new().route("/metrics", get(handlers::metrics));

    let app = chat_routes
        .merge(metrics_routes)
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            middleware::require_api_key,
        ))
        .route_layer(axum::middleware::from_fn(middleware::log_request))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!(%addr, "Gateway listening");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}
