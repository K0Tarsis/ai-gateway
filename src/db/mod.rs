use serde::Serialize;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

use crate::errors::GatewayError;

// WAL mode lets concurrent client requests insert rows without contending
// for SQLite's single writer lock (the default rollback journal mode would
// otherwise surface "database is locked" under concurrent load).
pub async fn connect(path: &str) -> Result<SqlitePool, GatewayError> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal);

    let pool = SqlitePoolOptions::new()
        .connect_with(options)
        .await
        .map_err(|e| GatewayError::Database(format!("failed to connect to '{path}': {e}")))?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .map_err(|e| GatewayError::Database(format!("failed to run migrations: {e}")))?;

    Ok(pool)
}

pub struct NewRequestLog<'a> {
    pub request_id: &'a str,
    pub profile: &'a str,
    pub provider: &'a str,
    pub model: &'a str,
    pub outcome: &'a str,
    pub latency_ms: i64,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub error: Option<&'a str>,
    pub cost_usd: Option<f64>,
}

pub async fn log_request(pool: &SqlitePool, entry: NewRequestLog<'_>) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO request_log \
         (request_id, profile, provider, model, outcome, latency_ms, prompt_tokens, completion_tokens, error, cost_usd) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(entry.request_id)
    .bind(entry.profile)
    .bind(entry.provider)
    .bind(entry.model)
    .bind(entry.outcome)
    .bind(entry.latency_ms)
    .bind(entry.prompt_tokens)
    .bind(entry.completion_tokens)
    .bind(entry.error)
    .bind(entry.cost_usd)
    .execute(pool)
    .await?;

    Ok(())
}

// Both the requesting client's own spend today and their month-to-date
// spend — restricted to `outcome = 'success'` since failed attempts carry
// no tokens/cost (nothing was actually billed). Window boundaries are
// pushed into SQLite's own date functions (UTC by default, matching
// `created_at`'s own `strftime(..., 'now')` default) rather than computed
// in Rust, so no date/time crate dependency is needed.
#[derive(sqlx::FromRow, Serialize)]
pub struct UsageBreakdown {
    pub provider: String,
    pub model: String,
    pub requests: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cost_usd: f64,
}

pub async fn usage_today(
    pool: &SqlitePool,
    profile: &str,
) -> Result<Vec<UsageBreakdown>, sqlx::Error> {
    sqlx::query_as(
        "SELECT provider, model, COUNT(*) AS requests, \
         COALESCE(SUM(prompt_tokens), 0) AS prompt_tokens, \
         COALESCE(SUM(completion_tokens), 0) AS completion_tokens, \
         COALESCE(SUM(cost_usd), 0.0) AS cost_usd \
         FROM request_log \
         WHERE profile = ?1 AND outcome = 'success' AND date(created_at) = date('now') \
         GROUP BY provider, model ORDER BY cost_usd DESC",
    )
    .bind(profile)
    .fetch_all(pool)
    .await
}

pub async fn usage_month_to_date(
    pool: &SqlitePool,
    profile: &str,
) -> Result<Vec<UsageBreakdown>, sqlx::Error> {
    sqlx::query_as(
        "SELECT provider, model, COUNT(*) AS requests, \
         COALESCE(SUM(prompt_tokens), 0) AS prompt_tokens, \
         COALESCE(SUM(completion_tokens), 0) AS completion_tokens, \
         COALESCE(SUM(cost_usd), 0.0) AS cost_usd \
         FROM request_log \
         WHERE profile = ?1 AND outcome = 'success' AND strftime('%Y-%m', created_at) = strftime('%Y-%m', 'now') \
         GROUP BY provider, model ORDER BY cost_usd DESC",
    )
    .bind(profile)
    .fetch_all(pool)
    .await
}

// Total spend only (no per-provider/model breakdown) — used by the
// cost_limit middleware to check a profile's cap before a chat completion
// runs. Same window predicates as usage_today/usage_month_to_date.
pub async fn spend_today(pool: &SqlitePool, profile: &str) -> Result<f64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COALESCE(SUM(cost_usd), 0.0) FROM request_log \
         WHERE profile = ?1 AND outcome = 'success' AND date(created_at) = date('now')",
    )
    .bind(profile)
    .fetch_one(pool)
    .await
}

pub async fn spend_month_to_date(pool: &SqlitePool, profile: &str) -> Result<f64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COALESCE(SUM(cost_usd), 0.0) FROM request_log \
         WHERE profile = ?1 AND outcome = 'success' AND strftime('%Y-%m', created_at) = strftime('%Y-%m', 'now')",
    )
    .bind(profile)
    .fetch_one(pool)
    .await
}
