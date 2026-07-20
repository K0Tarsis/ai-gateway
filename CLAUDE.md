# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

AI Gateway is a high-performance, OpenAI-compatible API Gateway written in Rust. It acts as a single entry point for multiple LLM providers (OpenAI, Anthropic, Ollama, Gemini, Azure OpenAI, DeepSeek, Groq, OpenRouter), handling routing, auth, retries, failover, logging, metrics, caching, and request normalization. Clients interact exclusively via the OpenAI API format.

## Commands

```sh
cargo build               # compile
cargo run                 # run the gateway
cargo test                # run all tests
cargo test <test_name>    # run a single test
cargo clippy              # lint
cargo fmt                 # format
cargo bench               # benchmarks (when added)
```

## Planned Tech Stack

| Concern | Crate |
|---|---|
| HTTP server | `axum` + `tower` |
| Async runtime | `tokio` |
| HTTP client | `reqwest` |
| Serialization | `serde` / `serde_json` |
| Config | `serde_yaml` |
| Database | `sqlx` (SQLite → PostgreSQL) |
| Logging | `tracing` + `tracing-subscriber` |
| Metrics | Prometheus (future: OpenTelemetry) |
| Cache | Redis |
| Errors | `thiserror` + `anyhow` |

## Architecture

All requests enter through an OpenAI-compatible HTTP layer and are converted to a unified internal model before being dispatched to a provider.

```
Client (OpenAI SDK)
    ↓
AI Gateway
  ├── Auth middleware (gateway API key / IP whitelist)
  ├── Rate limiting middleware
  ├── Request router (model name → provider)
  ├── Retry / failover logic
  ├── Cache layer (Redis, SHA256 keyed)
  └── Provider abstraction
        ├── OpenAI
        ├── Anthropic
        ├── Ollama
        └── … others
    ↓
Unified response → client
```

### Core trait

Every provider must implement a single trait:

```rust
trait AiProvider {
    async fn chat(&self, req: UnifiedRequest) -> Result<UnifiedResponse>;
    async fn embeddings(&self, req: UnifiedRequest) -> Result<UnifiedResponse>;
    async fn models(&self) -> Result<Vec<Model>>;
}
```

### Authentication layers

1. **Gateway auth** — client sends `Authorization: Bearer gw_xxx`; gateway validates key, rate limits, permissions.
2. **Provider auth** — gateway-managed keys from config (`OPENAI_API_KEY`, etc.) or BYOK via `X-OpenAI-Key` header.

### Planned module layout (`src/`)

```
config/       YAML config loading, env-var substitution
auth/         Gateway API key validation, IP whitelist
handlers/     Axum route handlers (chat, embeddings, models)
middleware/   Tower middleware (auth, rate limit, logging, metrics)
providers/    One file per provider + mod.rs with the trait
routing/      Model-name → provider mapping, failover chain
cache/        Redis cache layer
metrics/      Prometheus counters/histograms
models/       Shared request/response types (UnifiedRequest, etc.)
state/        AppState shared across handlers via Arc<RwLock<>>
errors/       Error types (thiserror)
utils/        Helpers
```

### Exposed endpoints

```
POST /v1/chat/completions
POST /v1/embeddings
GET  /v1/models
```

### Configuration shape (YAML)

```yaml
server:
  host: 127.0.0.1
  port: 8080

security:
  api_keys: [desktop]
  allowed_ips: [127.0.0.1]

providers:
  openai:
    enabled: true
    api_key: ${OPENAI_API_KEY}
  anthropic:
    enabled: false
  ollama:
    enabled: true
    base_url: http://localhost:11434

routing:
  fallback: [openai, anthropic, ollama]
```

## Development Roadmap

| Phase | Focus |
|---|---|
| 1 | MVP: OpenAI-compatible endpoint, OpenAI provider only, config, API keys |
| 1.5 | Abstraction proof: add Ollama provider to validate `AiProvider` trait generalizes before adding more providers |
| 2 | Reliability: retry, timeout, logging, streaming, health checks |
| 3 | Production: SQLite, metrics, dashboard, rate limiting, cost tracking |
| 4 | Scale: Redis cache, more providers (Claude, Gemini, Azure) |
| 5 | Observability: Prometheus, Grafana, OpenTelemetry, distributed tracing |
