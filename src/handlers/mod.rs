use axum::extract::State;
use axum::Json;

use crate::errors::GatewayError;
use crate::models::{ChatRequest, ChatResponse};
use crate::routing;
use crate::state::AppState;

pub async fn chat_completions(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, GatewayError> {
    let provider = routing::select_provider(&state)?;
    let response = provider.chat(req).await?;
    Ok(Json(response))
}
