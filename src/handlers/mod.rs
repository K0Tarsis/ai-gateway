use std::convert::Infallible;
use std::sync::Arc;
use std::time::Instant;

use axum::Json;
use axum::extract::{Extension, State};
use axum::http::header::CONTENT_TYPE;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures::StreamExt;
use metrics::{counter, gauge, histogram};
use serde::Serialize;
use tracing::{debug, info, warn};

use crate::config::ProfileConfig;
use crate::db::{self, NewRequestLog, UsageBreakdown};
use crate::errors::GatewayError;
use crate::middleware::RequestId;
use crate::models::{ChatRequest, ProviderInfo, ProvidersResponse};
use crate::providers::AiProvider;
use crate::routing;
use crate::state::AppState;

pub async fn chat_completions(
    State(state): State<AppState>,
    Extension(profile): Extension<ProfileConfig>,
    Extension(request_id): Extension<RequestId>,
    Json(req): Json<ChatRequest>,
) -> Result<Response, GatewayError> {
    let candidates = routing::select_providers(
        &state.providers,
        &state.health,
        &profile.routing.fallback,
        req.provider.as_deref(),
    )?;

    if req.stream.unwrap_or(false) {
        return stream_chat_completions(&state, &profile, candidates, req, request_id).await;
    }

    let mut last_err = None;
    for provider in candidates {
        let name = provider.name().to_string();
        let started = Instant::now();

        match provider.chat(req.clone()).await {
            Ok(response) => {
                state.health.record_success(&name);
                let (prompt_tokens, completion_tokens) = response
                    .usage
                    .as_ref()
                    .map(|u| (u.prompt_tokens, u.completion_tokens))
                    .unwrap_or_default();

                info!(
                    request_id = %request_id.0,
                    provider = %name,
                    model = %response.model,
                    latency_ms = started.elapsed().as_millis(),
                    prompt_tokens,
                    completion_tokens,
                    "chat completion succeeded"
                );

                counter!(
                    "gateway_requests_total",
                    "profile" => profile.name.clone(),
                    "provider" => name.clone(),
                    "outcome" => "success",
                )
                .increment(1);
                histogram!(
                    "gateway_request_duration_seconds",
                    "profile" => profile.name.clone(),
                    "provider" => name.clone(),
                )
                .record(started.elapsed().as_secs_f64());
                counter!(
                    "gateway_tokens_total",
                    "profile" => profile.name.clone(),
                    "provider" => name.clone(),
                    "kind" => "prompt",
                )
                .increment(prompt_tokens as u64);
                counter!(
                    "gateway_tokens_total",
                    "profile" => profile.name.clone(),
                    "provider" => name.clone(),
                    "kind" => "completion",
                )
                .increment(completion_tokens as u64);

                let cost_usd =
                    state
                        .config
                        .estimate_cost(&response.model, prompt_tokens, completion_tokens);
                match cost_usd {
                    Some(cost) => {
                        gauge!(
                            "gateway_cost_usd_total",
                            "profile" => profile.name.clone(),
                            "provider" => name.clone(),
                        )
                        .increment(cost);
                    }
                    None => {
                        debug!(model = %response.model, "no pricing configured for model, cost not tracked");
                    }
                }

                if let Err(e) = db::log_request(
                    &state.db,
                    NewRequestLog {
                        request_id: &request_id.0,
                        profile: &profile.name,
                        provider: &name,
                        model: &response.model,
                        outcome: "success",
                        latency_ms: started.elapsed().as_millis() as i64,
                        prompt_tokens: Some(prompt_tokens as i64),
                        completion_tokens: Some(completion_tokens as i64),
                        error: None,
                        cost_usd,
                    },
                )
                .await
                {
                    warn!(request_id = %request_id.0, error = %e, "failed to persist request log");
                }

                return Ok(Json(response).into_response());
            }
            Err(err) => {
                state.health.record_failure(&name);
                warn!(
                    request_id = %request_id.0,
                    provider = %name,
                    latency_ms = started.elapsed().as_millis(),
                    error = %err,
                    "chat completion attempt failed"
                );

                counter!(
                    "gateway_requests_total",
                    "profile" => profile.name.clone(),
                    "provider" => name.clone(),
                    "outcome" => "failure",
                )
                .increment(1);
                histogram!(
                    "gateway_request_duration_seconds",
                    "profile" => profile.name.clone(),
                    "provider" => name.clone(),
                )
                .record(started.elapsed().as_secs_f64());

                if let Err(e) = db::log_request(
                    &state.db,
                    NewRequestLog {
                        request_id: &request_id.0,
                        profile: &profile.name,
                        provider: &name,
                        model: &req.model,
                        outcome: "failure",
                        latency_ms: started.elapsed().as_millis() as i64,
                        prompt_tokens: None,
                        completion_tokens: None,
                        error: Some(&err.to_string()),
                        cost_usd: None,
                    },
                )
                .await
                {
                    warn!(request_id = %request_id.0, error = %e, "failed to persist request log");
                }

                last_err = Some(err);
            }
        }
    }

    Err(last_err.expect("select_providers never returns an empty candidate list"))
}

// Tries each candidate's `chat_stream()` in order until one *establishes*
// successfully (headers + status only) — once bytes are already flowing to
// the client, a mid-stream error just ends the stream rather than failing
// over, since the response has already been committed.
async fn stream_chat_completions(
    state: &AppState,
    profile: &ProfileConfig,
    candidates: Vec<Arc<dyn AiProvider>>,
    req: ChatRequest,
    request_id: RequestId,
) -> Result<Response, GatewayError> {
    let mut last_err = None;

    for provider in candidates {
        let name = provider.name().to_string();
        let started = Instant::now();

        match provider.chat_stream(req.clone()).await {
            Ok(inner) => {
                state.health.record_success(&name);
                info!(request_id = %request_id.0, provider = %name, "chat stream established");

                counter!(
                    "gateway_requests_total",
                    "profile" => profile.name.clone(),
                    "provider" => name.clone(),
                    "outcome" => "success",
                )
                .increment(1);
                histogram!(
                    "gateway_request_duration_seconds",
                    "profile" => profile.name.clone(),
                    "provider" => name.clone(),
                )
                .record(started.elapsed().as_secs_f64());

                if let Err(e) = db::log_request(
                    &state.db,
                    NewRequestLog {
                        request_id: &request_id.0,
                        profile: &profile.name,
                        provider: &name,
                        model: &req.model,
                        outcome: "success",
                        latency_ms: started.elapsed().as_millis() as i64,
                        prompt_tokens: None,
                        completion_tokens: None,
                        error: None,
                        cost_usd: None,
                    },
                )
                .await
                {
                    warn!(request_id = %request_id.0, error = %e, "failed to persist request log");
                }

                let request_id = request_id.0.clone();
                let name = name.clone();
                let sse_stream = inner
                    .filter_map(move |item| {
                        let request_id = request_id.clone();
                        let name = name.clone();
                        async move {
                            match item {
                                Ok(chunk) => serde_json::to_string(&chunk)
                                    .ok()
                                    .map(|data| Ok::<_, Infallible>(Event::default().data(data))),
                                Err(err) => {
                                    warn!(
                                        request_id = %request_id,
                                        provider = %name,
                                        error = %err,
                                        "chat stream ended with an error"
                                    );
                                    None
                                }
                            }
                        }
                    })
                    .chain(futures::stream::once(async {
                        Ok::<_, Infallible>(Event::default().data("[DONE]"))
                    }));

                return Ok(Sse::new(sse_stream)
                    .keep_alive(KeepAlive::default())
                    .into_response());
            }
            Err(err) => {
                state.health.record_failure(&name);
                warn!(
                    request_id = %request_id.0,
                    provider = %name,
                    error = %err,
                    "chat stream establish failed"
                );

                counter!(
                    "gateway_requests_total",
                    "profile" => profile.name.clone(),
                    "provider" => name.clone(),
                    "outcome" => "failure",
                )
                .increment(1);
                histogram!(
                    "gateway_request_duration_seconds",
                    "profile" => profile.name.clone(),
                    "provider" => name.clone(),
                )
                .record(started.elapsed().as_secs_f64());

                if let Err(e) = db::log_request(
                    &state.db,
                    NewRequestLog {
                        request_id: &request_id.0,
                        profile: &profile.name,
                        provider: &name,
                        model: &req.model,
                        outcome: "failure",
                        latency_ms: started.elapsed().as_millis() as i64,
                        prompt_tokens: None,
                        completion_tokens: None,
                        error: Some(&err.to_string()),
                        cost_usd: None,
                    },
                )
                .await
                {
                    warn!(request_id = %request_id.0, error = %e, "failed to persist request log");
                }

                last_err = Some(err);
            }
        }
    }

    Err(last_err.expect("select_providers never returns an empty candidate list"))
}

// Lists the provider/route names this profile can reach — its own fallback
// chain, resolved against configured providers — rather than querying every
// upstream vendor for its full model catalog.
pub async fn list_providers(
    State(state): State<AppState>,
    Extension(profile): Extension<ProfileConfig>,
) -> Json<ProvidersResponse> {
    let data = profile
        .routing
        .fallback
        .iter()
        .filter_map(|name| {
            state.providers.get(name).map(|provider| ProviderInfo {
                id: name.clone(),
                object: "provider".to_string(),
                owned_by: provider.name().to_string(),
                healthy: state.health.is_healthy(provider.name()),
            })
        })
        .collect();

    Json(ProvidersResponse {
        object: "list".to_string(),
        data,
    })
}

// Prometheus scrape endpoint. Renders whatever the process-global recorder
// (installed once in `AppState::new` via `metrics::install()`) has
// accumulated — this handler doesn't record anything itself.
pub async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    (
        [(CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        state.metrics_handle.render(),
    )
}

#[derive(Serialize)]
pub struct UsageResponse {
    pub profile: String,
    pub today: Vec<UsageBreakdown>,
    pub month_to_date: Vec<UsageBreakdown>,
}

// Per-provider/model spend for the calling profile only — today (UTC
// calendar day) and month-to-date (UTC calendar month), aggregated from
// `request_log` rows `db::log_request` already wrote at each successful
// chat completion.
pub async fn usage(
    State(state): State<AppState>,
    Extension(profile): Extension<ProfileConfig>,
) -> Result<Json<UsageResponse>, GatewayError> {
    let today = db::usage_today(&state.db, &profile.name)
        .await
        .map_err(|e| GatewayError::Database(e.to_string()))?;
    let month_to_date = db::usage_month_to_date(&state.db, &profile.name)
        .await
        .map_err(|e| GatewayError::Database(e.to_string()))?;

    Ok(Json(UsageResponse {
        profile: profile.name,
        today,
        month_to_date,
    }))
}
