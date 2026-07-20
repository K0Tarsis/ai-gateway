# Changelog

Running log of what's been implemented, and why, in date order (newest first).
See `CLAUDE.md` for the static architecture/roadmap reference — this file is
the history of how we got there.

## 2026-07-20

### Added: OpenAI provider + `/v1/chat/completions` handler (Phase 1 MVP)

- `src/providers/openai.rs` — `OpenAiProvider` implementing the `AiProvider`
  trait. Posts `ChatRequest` as JSON to `{base_url}/chat/completions` with
  `Authorization: Bearer <api_key>`, maps non-2xx responses and parse
  failures to `GatewayError::Provider`.
- `src/state/mod.rs` — `AppState` now builds a
  `HashMap<String, Arc<dyn AiProvider>>` at startup from
  `config.providers.*`, keyed by provider name. Only providers with
  `enabled: true` are instantiated.
- `src/routing/mod.rs` — `select_provider()` walks `routing.fallback` in
  order and returns the first provider present in the map. Deliberately not
  model-name-aware yet — there's only one provider, so a per-model mapping
  would be speculative. Revisit when a second provider is added.
- `src/handlers/mod.rs` — `chat_completions` axum handler: extracts
  `ChatRequest` from the request body, resolves a provider via `routing`,
  calls `.chat()`, returns `ChatResponse` as JSON.
- `src/main.rs` — wired an axum `Router` with `POST /v1/chat/completions`,
  bound to `server.host:server.port` from config, served via
  `axum::serve`.
- `src/errors/mod.rs` — `GatewayError` now implements `IntoResponse`, so
  handler errors serialize to `{"error": {"message": ...}}` with an
  appropriate status code (`Config`/internal → 500, `Provider` → 502,
  `Auth` → 401, `NotFound` → 404).

**Verified live**: started the gateway against a real `config.yaml` +
`.env` (OpenAI key), POSTed a minimal chat request, got a correctly-shaped
OpenAI-format response back through the gateway.

**Not yet done** (explicitly deferred, not forgotten):
- Auth middleware (`src/auth`) — gateway API key / IP whitelist checks are
  not enforced yet. Anyone who can reach the port can call the gateway.
- `GET /v1/models`.
- Model-name → provider routing (currently just picks the first available
  provider in `routing.fallback`).
- Anthropic / Ollama providers (config structs exist, no implementation).
- Streaming responses.

### Added: `.env` support

- **Problem**: `config.yaml` uses `${OPENAI_API_KEY}`-style placeholders
  substituted via `std::env::var` (`src/config/mod.rs`), but nothing loaded
  a `.env` file into the process environment — so a key set only in `.env`
  produced `Environment variable 'OPENAI_API_KEY' not set` even though the
  file had it.
- **Fix**: added the `dotenvy` crate and call `dotenvy::dotenv().ok()` as
  the very first line of `main()`, before `tracing_subscriber` init (so
  `RUST_LOG` from `.env` also takes effect) and before `config::load()`
  (so `${VAR}` substitution can see it). `.ok()` — a missing `.env` file
  (e.g. in prod where real env vars are set directly) is not an error.
  Real process env vars still take precedence over `.env`, since
  `dotenvy` only sets a var if it isn't already set.
- Added `.env.example` (`OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `RUST_LOG`)
  and added `.env` to `.gitignore`.
