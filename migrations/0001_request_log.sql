CREATE TABLE IF NOT EXISTS request_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id TEXT NOT NULL,
    profile TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    outcome TEXT NOT NULL,           -- 'success' | 'failure'
    latency_ms INTEGER NOT NULL,
    prompt_tokens INTEGER,           -- NULL on failure / streaming (no usage data there)
    completion_tokens INTEGER,
    error TEXT,                      -- NULL on success
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_request_log_profile_created ON request_log(profile, created_at);
CREATE INDEX IF NOT EXISTS idx_request_log_provider_created ON request_log(provider, created_at);
