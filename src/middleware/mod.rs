use std::net::SocketAddr;
use std::time::Instant;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::HeaderValue;
use axum::http::header::AUTHORIZATION;
use axum::middleware::Next;
use axum::response::Response;
use metrics::{counter, histogram};
use tracing::{debug, info};
use uuid::Uuid;

use crate::auth;
use crate::config::ProfileConfig;
use crate::db;
use crate::errors::GatewayError;
use crate::state::AppState;

// Inserted into request extensions by `log_request` so handlers can attach
// the same ID to their own structured log lines.
#[derive(Clone)]
pub struct RequestId(pub String);

// Wraps every route (layered outside `require_api_key`, so it also captures
// auth-rejection responses under a request ID): assigns a request ID, times
// the whole request, and logs one structured completion line.
pub async fn log_request(mut req: Request, next: Next) -> Response {
    let request_id = Uuid::new_v4().to_string();
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    req.extensions_mut().insert(RequestId(request_id.clone()));

    let started = Instant::now();
    let mut response = next.run(req).await;
    let elapsed = started.elapsed();
    let latency_ms = elapsed.as_millis();
    let status = response.status();

    info!(
        request_id = %request_id,
        %method,
        %path,
        %status,
        latency_ms,
        "request completed"
    );

    counter!(
        "http_requests_total",
        "method" => method.to_string(),
        "path" => path.clone(),
        "status" => status.as_u16().to_string(),
    )
    .increment(1);
    histogram!(
        "http_request_duration_seconds",
        "method" => method.to_string(),
        "path" => path,
    )
    .record(elapsed.as_secs_f64());

    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert("x-request-id", value);
    }

    response
}

pub async fn require_api_key(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    mut req: Request,
    next: Next,
) -> Result<Response, GatewayError> {
    let token = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(|| GatewayError::Auth("Missing or malformed Authorization header".into()))?;

    let profile = auth::resolve_profile(&state.config.profiles, token)
        .ok_or_else(|| GatewayError::Auth("Invalid API key".into()))?
        .clone();

    if !auth::validate_ip(&profile.allowed_ips, &addr.ip()) {
        return Err(GatewayError::Auth("IP address not allowed".into()));
    }

    debug!(profile = %profile.name, %addr, "request authenticated");
    req.extensions_mut().insert(profile);
    Ok(next.run(req).await)
}

// Runs after `require_api_key` (needs the resolved profile from request
// extensions). Profiles without a `rate_limit` configured are unaffected.
pub async fn rate_limit(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, GatewayError> {
    let profile = req
        .extensions()
        .get::<ProfileConfig>()
        .expect("rate_limit must run after require_api_key")
        .clone();

    if let Some(limit) = &profile.rate_limit
        && !state
            .rate_limiter
            .check(&profile.name, limit.requests_per_minute)
    {
        return Err(GatewayError::RateLimited(format!(
            "Profile '{}' exceeded {} requests/minute",
            profile.name, limit.requests_per_minute
        )));
    }

    Ok(next.run(req).await)
}

// Runs after `require_api_key`, only on the chat-completions route (the one
// route that actually spends money — see main.rs's router split). Unlike
// `rate_limit`, this is DB-backed (a spend cap only exists in `request_log`,
// there's no in-memory equivalent), so it's checked with a query per
// request rather than an in-memory bucket. Profiles without a `cost_limit`
// configured are unaffected.
pub async fn cost_limit(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, GatewayError> {
    let profile = req
        .extensions()
        .get::<ProfileConfig>()
        .expect("cost_limit must run after require_api_key")
        .clone();

    if let Some(limit) = &profile.cost_limit {
        if let Some(cap) = limit.daily_usd {
            let spent = db::spend_today(&state.db, &profile.name)
                .await
                .map_err(|e| GatewayError::Database(e.to_string()))?;
            if spent >= cap {
                return Err(GatewayError::CostLimitExceeded(format!(
                    "Profile '{}' has spent ${spent:.4} today, at or over its ${cap:.2} daily cap",
                    profile.name
                )));
            }
        }

        if let Some(cap) = limit.monthly_usd {
            let spent = db::spend_month_to_date(&state.db, &profile.name)
                .await
                .map_err(|e| GatewayError::Database(e.to_string()))?;
            if spent >= cap {
                return Err(GatewayError::CostLimitExceeded(format!(
                    "Profile '{}' has spent ${spent:.4} this month, at or over its ${cap:.2} monthly cap",
                    profile.name
                )));
            }
        }
    }

    Ok(next.run(req).await)
}
