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
}
```

`GET /v1/models` does not query upstream vendor catalogs — it lists the
provider/route names the caller's profile can reach (its own
`routing.fallback`, resolved against configured providers/routes).

### Authentication layers

1. **Gateway auth** — client sends `Authorization: Bearer gw_xxx`; the key resolves to a **profile** (a tenant), which carries its own IP allowlist and its own provider fallback order. Different clients can hold keys for different profiles and get different routing/limits.
2. **Provider auth** — gateway-managed keys from config (`OPENAI_API_KEY`, etc.). No per-request BYOK header yet.

### Planned module layout (`src/`)

```
config/       YAML config loading, env-var substitution
auth/         Gateway API key validation, IP whitelist
handlers/     Axum route handlers (chat, embeddings, models)
middleware/   Tower middleware (auth, rate limit, logging, metrics)
providers/    One file per provider + mod.rs with the trait
routing/      Model-name → provider mapping, failover chain
health/       Reactive circuit breaker (HealthTracker) backing routing's
              healthy-first ordering
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
GET  /v1/models
GET  /v1/usage
GET  /metrics
```

### Configuration shape (YAML)

```yaml
server:
  host: 127.0.0.1
  port: 8080

providers:
  openai:
    enabled: true
    api_key: ${OPENAI_API_KEY}
  anthropic:
    enabled: true
    api_key: ${ANTHROPIC_API_KEY}

# Named shortcuts pinning a connection to a fixed model. Live in the same
# namespace as bare provider names — usable anywhere a provider name is
# (fallback lists, the request-level `provider` field).
#
# A fallback list may name a bare provider (e.g. `openai`) only when every
# entry in that list resolves to the same vendor -- a bare provider forwards
# the client's `model` field upstream unchanged, which is unsafe once the
# chain spans more than one vendor's model catalog. A mixed-vendor chain
# must name only routes, bare providers included; config loading rejects
# a mixed-vendor chain that still contains a bare provider.
routes:
  anthropic-opus:
    provider: anthropic
    model: claude-opus-4-20250514
  openai-gpt4o-mini:
    provider: openai
    model: gpt-4o-mini

# Same-provider retry (connection errors/429/5xx) before a candidate counts
# as failed for cross-provider failover. Both optional, shown at defaults.
retry:
  max_attempts: 2

# Circuit breaker: after `failure_threshold` consecutive failures a provider
# is deprioritized (tried last) for `cooldown_secs`. Both optional, shown at
# defaults. Status surfaced on `GET /v1/models` as `"healthy": true/false`.
health:
  failure_threshold: 3
  cooldown_secs: 30

# Each profile is a tenant: its own keys, its own IP allowlist (omit/empty
# = allow any IP), and its own provider fallback order — which also doubles
# as its allowlist for the request-level `provider` pin. Provider instances
# above are shared/global — a profile picks which of them it can reach.
profiles:
  - name: desktop
    api_keys: [gw_desktop_key]
    allowed_ips: [127.0.0.1]
    routing:
      fallback: [openai]
  - name: partner-a
    api_keys: [gw_partner_a_key]
    routing:
      fallback: [anthropic-opus, openai-gpt4o-mini]
    # Opt-in per-profile cap (token bucket; omit for unlimited).
    rate_limit:
      requests_per_minute: 60
```

## Development Roadmap

| Phase | Focus |
|---|---|
| 1 | MVP: OpenAI-compatible endpoint, OpenAI provider only, config, API keys — done |
| 1.5 | Abstraction proof — done with Anthropic instead of Ollama (hosted API, no local server to run) |
| 2 | Reliability: retry, timeout, logging, streaming, health checks — done |
| 3 | Production: SQLite, metrics, rate limiting, cost tracking, cost limiting — done |
| 4 | Scale: more providers (Claude — done Phase 1.5, Azure — done, Gemini) |
| 5 | Observability: Prometheus, Grafana, OpenTelemetry, distributed tracing |
