use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
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
}

impl IntoResponse for GatewayError {
    fn into_response(self) -> Response {
        let status = match &self {
            GatewayError::Config(_) => StatusCode::INTERNAL_SERVER_ERROR,
            GatewayError::Provider(_) => StatusCode::BAD_GATEWAY,
            GatewayError::Auth(_) => StatusCode::UNAUTHORIZED,
            GatewayError::NotFound(_) => StatusCode::NOT_FOUND,
        };

        let body = Json(json!({
            "error": {
                "message": self.to_string(),
            }
        }));

        (status, body).into_response()
    }
}
