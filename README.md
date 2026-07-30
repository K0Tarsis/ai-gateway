# AI Gateway

AI Gateway is a self-hosted, OpenAI-compatible API gateway written in Rust. Point your existing OpenAI SDK code at it instead of `api.openai.com`, and it transparently routes requests to OpenAI, Anthropic, or Azure OpenAI — with automatic retries, cross-provider failover, per-tenant auth, rate limiting, cost tracking, and Prometheus metrics.

You keep writing code against the OpenAI API shape. The gateway handles which real provider actually answers each request.

```
Your app (OpenAI SDK)
        │  Authorization: Bearer gw_xxx
        ▼
   AI Gateway
        │
        ├─ resolves the key to a profile (tenant)
        ├─ checks that profile's IP allowlist, rate limit, cost cap
        ├─ picks a provider from that profile's fallback chain
        ├─ retries transient errors, fails over to the next provider
        └─ logs the attempt, records cost/metrics
        │
        ▼
 OpenAI / Anthropic / Azure OpenAI
```

## Why

If you call LLM providers directly, every provider outage or rate-limit hit is your application's problem, and every provider's API has its own auth scheme and request/response shape. AI Gateway centralizes that: your application always speaks the OpenAI wire format, and the gateway is the thing that knows how to reach whichever provider is actually configured, retry it, and fail over to another one if it's down.

It's also a multi-tenant front door: different API keys ("profiles") can be handed to different clients, each with its own allowed IPs, its own provider fallback order, its own rate limit, and its own spending cap — useful if you're proxying access for multiple internal teams or external partners through one deployment.

## Features

- **OpenAI-compatible endpoint** — `POST /v1/chat/completions`, including streaming (`"stream": true`, real Server-Sent Events). Any OpenAI SDK works by changing only the `base_url`.
- **Multiple providers, one interface** — OpenAI, Anthropic, and Azure OpenAI are supported today, each behind the same internal `AiProvider` trait. A provider's own wire format is translated to/from a single OpenAI-shaped request/response internally.
- **Cross-provider failover** — if a provider errors after its own retries are exhausted, the gateway automatically tries the next provider in the profile's fallback chain.
- **Same-provider retry** — connection errors, HTTP 429, and 5xx responses are retried with backoff before a candidate is considered failed (configurable attempt count).
- **Reactive health tracking** — a provider that fails repeatedly is deprioritized (tried last, not removed) for a cooldown window, so a known-down provider doesn't eat a full timeout on every request. No separate poller — health is derived from real request outcomes.
- **Multi-tenant profiles** — each gateway API key belongs to a profile with its own IP allowlist, its own provider fallback order (which doubles as its allowlist for pinning a specific provider per-request), its own optional rate limit, and its own optional cost cap.
- **Rate limiting** — optional per-profile requests-per-minute cap (token bucket).
- **Cost tracking & limiting** — per-request cost is estimated from configurable per-model pricing, persisted to SQLite, and queryable via `GET /v1/usage` (today / month-to-date). Profiles can optionally set a daily and/or monthly spend cap that rejects further requests with `429` once exceeded.
- **Request logging** — every attempt gets a UUID (returned as `x-request-id`), structured `tracing` logs, and a persisted row in SQLite (provider, model, latency, tokens, outcome, cost).
- **Prometheus metrics** — `GET /metrics` exposes request counts, latency histograms, token counts, and estimated cost, labeled by profile and provider.
- **Named routes** — pin a provider connection to a specific model (or, for Azure, a specific deployment) under a name of your choosing, reusable in fallback chains or as a per-request override.

### Not yet implemented

The project is under active development. Not (yet) present: an `/v1/embeddings` endpoint, a response cache, additional providers (Gemini, DeepSeek, Groq, OpenRouter, Ollama), and alternative auth schemes (JWT/OAuth2/mTLS) — only bearer-token gateway API keys exist today.

## Getting started

### Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (stable toolchain)
- An API key for at least one provider you want to use (OpenAI, Anthropic, and/or Azure OpenAI)

### Run it

```sh
git clone <this-repo>
cd ai_geteway

cp config.example.yaml config.yaml
# edit config.yaml: enable the provider(s) you want, set your gateway API key(s)

export OPENAI_API_KEY=sk-...   # whichever provider(s) you enabled in config.yaml

cargo run
```

By default the gateway listens on `127.0.0.1:8080` (set in `config.yaml`'s `server:` block). Once it's running:

```sh
curl http://127.0.0.1:8080/v1/chat/completions \
  -H "Authorization: Bearer gw_your_key_here" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "messages": [{"role": "user", "content": "Hello!"}]
  }'
```

Or with the official OpenAI SDK — just point `base_url` at the gateway:

```python
from openai import OpenAI

client = OpenAI(
    base_url="http://127.0.0.1:8080/v1",
    api_key="gw_your_key_here",  # your gateway key, not an OpenAI key
)

response = client.chat.completions.create(
    model="gpt-4o-mini",
    messages=[{"role": "user", "content": "Hello!"}],
)
```

### Useful commands

```sh
cargo build      # compile
cargo run        # run the gateway
cargo test       # run the test suite
cargo clippy     # lint
cargo fmt        # format
```

## Configuration

The gateway reads `config.yaml` (path is hardcoded to `config.yaml` in the working directory) at startup. `config.example.yaml` in this repo is a complete, commented example — copy it to `config.yaml` and adjust. `${VAR_NAME}` anywhere in the file is substituted from the environment before parsing, so secrets stay out of the file itself.

### `server`

```yaml
server:
  host: 127.0.0.1
  port: 8080
```

### `providers`

Each provider you want to use must be `enabled: true` and given credentials. Providers you don't enable are simply unavailable for routing.

```yaml
providers:
  openai:
    enabled: true
    api_key: ${OPENAI_API_KEY}
    # base_url defaults to https://api.openai.com/v1
    # timeout_secs defaults to 30

  anthropic:
    enabled: false
    api_key: ${ANTHROPIC_API_KEY}
    # base_url defaults to https://api.anthropic.com/v1
    # timeout_secs defaults to 30

  azure:
    enabled: false
    api_key: ${AZURE_OPENAI_API_KEY}
    base_url: https://your-resource.openai.azure.com
    # api_version defaults to 2024-06-01
    # timeout_secs defaults to 30
```

Azure OpenAI addresses a **deployment**, not a bare model name — see [`routes`](#routes) below for how to expose an Azure deployment under a name.

### `retry`

Same-provider retry (connection errors, 429, 5xx) attempted before a provider counts as failed and the gateway fails over to the next one in the chain.

```yaml
retry:
  max_attempts: 2   # default
```

### `health`

Circuit breaker: after `failure_threshold` consecutive failures, a provider is tried last (not dropped) for `cooldown_secs`. Current status per provider/route is visible via `"healthy": true/false` on `GET /v1/models`.

```yaml
health:
  failure_threshold: 3    # default
  cooldown_secs: 30       # default
```

### `database`

SQLite file backing persisted request history (used by cost tracking, `GET /v1/usage`, and cost limiting).

```yaml
database:
  path: gateway.db   # default
```

### `pricing`

Optional per-model `$`/1M-token rates, used to estimate cost on each successful request. Key on the exact model string the *provider* returns (visible in a response's `"model"` field) — not necessarily what the client requested, since providers can resolve aliases to a dated snapshot. A model with no entry here simply isn't cost-tracked; everything else still works.

```yaml
pricing:
  gpt-4o-mini-2024-07-18:
    prompt_price_per_million: 0.15
    completion_price_per_million: 0.60
  claude-opus-4-20250514:
    prompt_price_per_million: 15.00
    completion_price_per_million: 75.00
```

### `routes`

A route pins a configured provider connection to one fixed model (or, for Azure, one fixed deployment) under a name of your choosing. Routes live in the same namespace as bare provider names, so they can be used anywhere a provider name is used: in a profile's `routing.fallback` list, or as a per-request `"provider"` override.

```yaml
routes:
  anthropic-opus:
    provider: anthropic
    model: claude-opus-4-20250514
  azure-gpt4o:
    provider: azure
    model: my-gpt4o-deployment   # your deployment name in Azure, not a bare model id
  openai-gpt4o-mini:
    provider: openai
    model: gpt-4o-mini-2024-07-18
```

A `fallback` list may only name a bare provider (e.g. `openai`) when every entry in that list resolves to the same vendor — a bare provider forwards the client's `model` field upstream unchanged, which is only safe if every candidate shares one vendor's model catalog. As soon as a fallback chain spans more than one vendor, **every** entry must be a route, bare providers included; config loading rejects a mixed-vendor chain that still contains a bare provider.

### `profiles`

Each profile is an independent tenant: its own gateway API key(s), its own IP allowlist, and its own provider fallback order. Different clients hold keys for different profiles and get different routing, rate limits, and cost caps.

```yaml
profiles:
  - name: desktop
    api_keys:
      - gw_your_key_here
    allowed_ips:          # omit or leave empty to allow any IP
      - 127.0.0.1
    routing:
      fallback:
        - openai

  - name: partner-a
    api_keys:
      - gw_partner_a_key_here
    routing:
      fallback:
        - anthropic-opus
        - azure-gpt4o
        - openai-gpt4o-mini
    rate_limit:                    # optional; omit for unlimited
      requests_per_minute: 60
    cost_limit:                    # optional; omit for unlimited
      daily_usd: 5.00
      monthly_usd: 100.00
```

Request routing walks `routing.fallback` in order, skipping providers/routes marked unhealthy until they're tried last. A client can pin one entry for a single request instead:

```json
{ "model": "gpt-4o-mini", "provider": "anthropic-opus", "messages": [...] }
```

The pinned entry must already be present in the caller's profile's `fallback` list — a profile can't reach a provider/route it wasn't granted. If the pinned entry fails, the gateway falls through to the rest of that profile's chain, same as when no `provider` is given at all.

## API reference

All endpoints (except none — every route requires auth) expect `Authorization: Bearer <gateway-api-key>`.

| Endpoint | Description |
|---|---|
| `POST /v1/chat/completions` | OpenAI-shaped chat completion. Set `"stream": true` for Server-Sent Events. Optional `"provider"` field pins a specific provider/route for this request. |
| `GET /v1/models` | Lists the provider/route names the caller's profile can reach, each with a `"healthy"` boolean. Does not query upstream vendor catalogs. |
| `GET /v1/usage` | The calling profile's spend, broken down by provider/model, for today and month-to-date. |
| `GET /metrics` | Prometheus exposition format: request counts, latency histograms, token counts, estimated cost. |

### Authentication

Every request needs `Authorization: Bearer gw_xxx`, where `gw_xxx` is one of the API keys listed under a profile in `config.yaml`. The key resolves to that profile, which determines:

- which IPs are allowed to use it (`allowed_ips`; empty/omitted means any IP)
- which providers/routes it can reach, and in what fallback order (`routing.fallback`)
- its rate limit and cost cap, if configured

There is currently one authentication mode: gateway-managed provider credentials from `config.yaml`. There is no per-request "bring your own provider key" header and no JWT/OAuth support yet.

## Project layout

```
src/
├── main.rs        route wiring, middleware stack
├── config/        YAML config loading, ${VAR} substitution
├── auth/          API key → profile resolution, IP allowlist checks
├── handlers/       /v1/chat/completions, /v1/models, /v1/usage, /metrics
├── middleware/     auth, rate limiting, cost limiting, request logging
├── providers/      one file per provider (openai, anthropic, azure) + the shared AiProvider trait
├── routing/        fallback-chain candidate selection
├── health/         reactive circuit breaker
├── ratelimit/      per-profile token bucket
├── db/             SQLite request-log persistence, usage/spend queries
├── metrics/        Prometheus recorder setup
├── models/         shared request/response types
├── state/          AppState — providers, config, db pool, shared across handlers
└── errors/         GatewayError → HTTP response mapping
```

## Roadmap

| Phase | Focus | Status |
|---|---|---|
| 1 | MVP: OpenAI-compatible endpoint, OpenAI provider, config, API keys | Done |
| 1.5 | Anthropic provider (validated the provider abstraction generalizes) | Done |
| 2 | Reliability: retry, timeout, logging, streaming, health checks | Done |
| 3 | Production: SQLite request log, Prometheus metrics, rate limiting, cost tracking & limiting | Done |
| 4 | More providers: Azure OpenAI (done), Gemini | In progress |
| 5 | Observability: Grafana, OpenTelemetry, distributed tracing | Planned |
