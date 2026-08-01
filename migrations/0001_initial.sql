PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS app_settings (
    key TEXT PRIMARY KEY,
    value_json TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS secret_values (
    key TEXT PRIMARY KEY,
    nonce BLOB NOT NULL,
    ciphertext BLOB NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS paperless_connections (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    base_url TEXT NOT NULL,
    token_secret_key TEXT NOT NULL,
    custom_ca_pem TEXT,
    api_version INTEGER,
    server_version TEXT,
    last_tested_at TEXT,
    last_error TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS provider_profiles (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    provider_kind TEXT NOT NULL CHECK(provider_kind IN ('openai','claude','lmstudio','custom')),
    base_url TEXT NOT NULL,
    model TEXT NOT NULL DEFAULT '',
    api_family TEXT NOT NULL CHECK(api_family IN ('responses','chat_completions','messages')),
    chat_output_token_field TEXT NOT NULL DEFAULT 'max_tokens' CHECK(chat_output_token_field IN ('max_tokens','max_completion_tokens')),
    api_key_secret_key TEXT,
    headers_secret_key TEXT,
    capabilities_json TEXT NOT NULL DEFAULT '{}',
    reasoning_effort TEXT,
    temperature REAL,
    max_output_tokens INTEGER NOT NULL DEFAULT 8000,
    request_timeout_seconds INTEGER NOT NULL DEFAULT 300,
    daily_budget_tokens INTEGER,
    monthly_budget_tokens INTEGER,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS processing_profiles (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    paperless_connection_id TEXT NOT NULL REFERENCES paperless_connections(id) ON DELETE CASCADE,
    provider_profile_id TEXT NOT NULL REFERENCES provider_profiles(id) ON DELETE RESTRICT,
    revision INTEGER NOT NULL DEFAULT 1,
    enabled INTEGER NOT NULL DEFAULT 0,
    automatic_mode TEXT NOT NULL DEFAULT 'automatic' CHECK(automatic_mode IN ('automatic','review')),
    poll_interval_seconds INTEGER NOT NULL DEFAULT 300,
    max_documents_per_scan INTEGER NOT NULL DEFAULT 50,
    concurrency INTEGER NOT NULL DEFAULT 1,
    use_existing_data INTEGER NOT NULL DEFAULT 1,
    process_pre_tagged_only INTEGER NOT NULL DEFAULT 0,
    trigger_tag_ids_json TEXT NOT NULL DEFAULT '[]',
    trigger_match_mode TEXT NOT NULL DEFAULT 'any' CHECK(trigger_match_mode IN ('any','all')),
    restrict_tags INTEGER NOT NULL DEFAULT 1,
    restrict_correspondents INTEGER NOT NULL DEFAULT 1,
    restrict_document_types INTEGER NOT NULL DEFAULT 1,
    add_processed_tag INTEGER NOT NULL DEFAULT 1,
    processed_tag_name TEXT NOT NULL DEFAULT 'ai-processed',
    tags_enabled INTEGER NOT NULL DEFAULT 1,
    correspondent_enabled INTEGER NOT NULL DEFAULT 1,
    document_type_enabled INTEGER NOT NULL DEFAULT 1,
    title_enabled INTEGER NOT NULL DEFAULT 1,
    custom_fields_enabled INTEGER NOT NULL DEFAULT 0,
    overwrite_tags INTEGER NOT NULL DEFAULT 0,
    overwrite_correspondent INTEGER NOT NULL DEFAULT 0,
    overwrite_document_type INTEGER NOT NULL DEFAULT 0,
    overwrite_title INTEGER NOT NULL DEFAULT 0,
    prompt_mode TEXT NOT NULL DEFAULT 'additional' CHECK(prompt_mode IN ('additional','expert')),
    custom_prompt TEXT NOT NULL DEFAULT '',
    max_input_tokens INTEGER NOT NULL DEFAULT 32000,
    taxonomy_context_tokens INTEGER NOT NULL DEFAULT 8000,
    include_document_file INTEGER NOT NULL DEFAULT 0,
    file_rendition TEXT NOT NULL DEFAULT 'archive' CHECK(file_rendition IN ('archive','original')),
    max_file_bytes INTEGER NOT NULL DEFAULT 10485760,
    max_file_pages INTEGER NOT NULL DEFAULT 20,
    ocr_excerpt_tokens INTEGER NOT NULL DEFAULT 2000,
    daily_budget_tokens INTEGER,
    monthly_budget_tokens INTEGER,
    budget_timezone TEXT NOT NULL DEFAULT 'Europe/Berlin',
    warning_threshold_percent INTEGER NOT NULL DEFAULT 80,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS custom_field_rules (
    id TEXT PRIMARY KEY,
    processing_profile_id TEXT NOT NULL REFERENCES processing_profiles(id) ON DELETE CASCADE,
    paperless_field_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    data_type TEXT NOT NULL,
    extra_data_json TEXT NOT NULL DEFAULT '{}',
    instructions TEXT NOT NULL DEFAULT '',
    overwrite_existing INTEGER NOT NULL DEFAULT 0,
    enabled INTEGER NOT NULL DEFAULT 1,
    UNIQUE(processing_profile_id, paperless_field_id)
);

CREATE TABLE IF NOT EXISTS jobs (
    id TEXT PRIMARY KEY,
    processing_profile_id TEXT NOT NULL REFERENCES processing_profiles(id) ON DELETE CASCADE,
    profile_revision INTEGER NOT NULL,
    paperless_document_id INTEGER NOT NULL,
    document_checksum TEXT NOT NULL,
    document_title TEXT,
    status TEXT NOT NULL CHECK(status IN ('queued','analyzing','awaiting_review','applying','completed','budget_blocked','failed','rejected')),
    attempt_count INTEGER NOT NULL DEFAULT 0,
    manual_reprocess INTEGER NOT NULL DEFAULT 0,
    truncated INTEGER NOT NULL DEFAULT 0,
    suggestion_json TEXT,
    error_code TEXT,
    error_message TEXT,
    lease_owner TEXT,
    lease_expires_at TEXT,
    queued_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    started_at TEXT,
    completed_at TEXT,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE(processing_profile_id, profile_revision, paperless_document_id, document_checksum, manual_reprocess)
);

CREATE INDEX IF NOT EXISTS idx_jobs_status_queue ON jobs(status, queued_at);
CREATE INDEX IF NOT EXISTS idx_jobs_document ON jobs(processing_profile_id, paperless_document_id);

CREATE TABLE IF NOT EXISTS usage_events (
    id TEXT PRIMARY KEY,
    budget_reservation_id TEXT UNIQUE REFERENCES budget_reservations(id) ON DELETE SET NULL,
    job_id TEXT REFERENCES jobs(id) ON DELETE SET NULL,
    provider_profile_id TEXT NOT NULL REFERENCES provider_profiles(id) ON DELETE RESTRICT,
    provider_kind TEXT NOT NULL,
    model TEXT NOT NULL,
    api_family TEXT NOT NULL,
    operation TEXT NOT NULL DEFAULT 'analysis',
    functions_json TEXT NOT NULL DEFAULT '[]',
    status TEXT NOT NULL,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    cache_write_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    reasoning_tokens INTEGER NOT NULL DEFAULT 0,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    usage_source TEXT NOT NULL CHECK(usage_source IN ('reported','preflight','estimated','unknown','reserved_after_unknown')),
    latency_ms INTEGER NOT NULL DEFAULT 0,
    raw_usage_json TEXT NOT NULL DEFAULT '{}',
    error_code TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_usage_created ON usage_events(created_at);
CREATE INDEX IF NOT EXISTS idx_usage_provider_created ON usage_events(provider_profile_id, created_at);

CREATE TABLE IF NOT EXISTS budget_reservations (
    id TEXT PRIMARY KEY,
    job_id TEXT REFERENCES jobs(id) ON DELETE CASCADE,
    provider_profile_id TEXT NOT NULL REFERENCES provider_profiles(id) ON DELETE CASCADE,
    reserved_tokens INTEGER NOT NULL,
    settled_tokens INTEGER,
    state TEXT NOT NULL CHECK(state IN ('reserved','settled','charged_unknown','released')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    settled_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_budget_active ON budget_reservations(provider_profile_id, state, created_at);

CREATE TABLE IF NOT EXISTS admin_auth (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    password_hash TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS sessions (
    id_hash TEXT PRIMARY KEY,
    csrf_token TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    last_seen_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_sessions_expiry ON sessions(expires_at);
