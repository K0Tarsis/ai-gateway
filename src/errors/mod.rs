use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GatewayError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Provider error: {0}")]
    Provider(String),

    #[error("Authentication error: {0}")]
    Auth(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Rate limit exceeded: {0}")]
    RateLimited(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Cost limit exceeded: {0}")]
    CostLimitExceeded(String),
}

impl IntoResponse for GatewayError {
    fn into_response(self) -> Response {
        let status = match &self {
            GatewayError::Config(_) => StatusCode::INTERNAL_SERVER_ERROR,
            GatewayError::Provider(_) => StatusCode::BAD_GATEWAY,
            GatewayError::Auth(_) => StatusCode::UNAUTHORIZED,
            GatewayError::NotFound(_) => StatusCode::NOT_FOUND,
            GatewayError::RateLimited(_) => StatusCode::TOO_MANY_REQUESTS,
            GatewayError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
            // Matches OpenAI's real API (429 on insufficient_quota), not
            // 402 — keeps client SDKs' existing 429 retry/backoff working.
            GatewayError::CostLimitExceeded(_) => StatusCode::TOO_MANY_REQUESTS,
        };

        let body = Json(json!({
            "error": {
                "message": self.to_string(),
            }
        }));

        (status, body).into_response()
    }
}
