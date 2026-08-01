pub mod providers;
pub mod settings;
pub mod types;

use std::net::SocketAddr;

use axum::{
    Json, Router,
    extract::{ConnectInfo, Path, Query, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use chrono::{TimeDelta, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, QueryBuilder, Sqlite};
use url::Url;
use zeroize::Zeroizing;

use crate::{
    AppError, AppResult,
    api::{
        settings::{cache_catalogs, load_connection, load_settings, save_settings},
        types::{
            AppSettings, AppStatus, ApproveRequest, AuthStatus, BootstrapResponse, CatalogItem,
            Catalogs, ConnectionHealth, LoginRequest, OkResponse, PaperlessTestRequest,
            PaperlessTestResult, PasswordRequest, ProviderProfileDto, WorkerStatusDto,
        },
    },
    app::AppState,
    budget::period_start,
    models::SessionInfo,
    paperless::{CustomFieldDefinitionCreate, PaperlessClient},
    worker::{self, StoredSuggestion},
};

const SESSION_COOKIE: &str = "paperlessllm_session";

pub fn router(state: AppState) -> Router<AppState> {
    let protected = Router::new()
        .route("/api/status", get(status))
        .route("/api/bootstrap", get(bootstrap))
        .route("/api/auth/status", get(auth_status))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/password", put(set_password))
        .route("/api/settings", get(get_settings).put(put_settings))
        .route("/api/paperless/test", post(test_paperless))
        .route("/api/paperless/catalogs", get(paperless_catalogs))
        .route("/api/paperless/custom-fields", post(create_custom_field))
        .route("/api/providers", get(list_providers).post(create_provider))
        .route(
            "/api/providers/{id}",
            put(update_provider).delete(delete_provider),
        )
        .route("/api/providers/{id}/models", get(provider_models))
        .route("/api/providers/{id}/test", post(test_provider))
        .route("/api/scan", post(scan_now))
        .route("/api/jobs", get(list_jobs))
        .route("/api/jobs/{id}/retry", post(retry_job))
        .route("/api/reviews", get(list_reviews))
        .route("/api/reviews/{id}/approve", post(approve_review))
        .route("/api/reviews/{id}/reject", post(reject_review))
        .route("/api/stats/summary", get(stats_summary))
        .route("/api/stats/series", get(stats_series))
        .layer(middleware::from_fn_with_state(state, security_guard));

    Router::new()
        .route("/healthz", get(healthz))
        .merge(protected)
}

async fn healthz() -> Json<Value> {
    Json(json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }))
}

async fn security_guard(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> AppResult<Response> {
    if !state.requires_auth() {
        return Ok(next.run(request).await);
    }
    let path = request.uri().path().to_owned();
    let public = matches!(path.as_str(), "/api/auth/login" | "/api/auth/status");
    let session = if let Some(token) = session_cookie(request.headers()) {
        state.auth.authenticate(&token).await?
    } else {
        None
    };
    if let Some(session) = session.clone() {
        request.extensions_mut().insert(session);
    }
    if public {
        return Ok(next.run(request).await);
    }
    let session = session.ok_or(AppError::Unauthorized)?;
    if !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    ) {
        let csrf = request
            .headers()
            .get("x-csrf-token")
            .and_then(|value| value.to_str().ok());
        if csrf != Some(session.csrf_token.as_str()) {
            return Err(AppError::Forbidden("invalid CSRF token".to_owned()));
        }
    }
    Ok(next.run(request).await)
}

fn session_cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|cookie| {
                let (name, value) = cookie.trim().split_once('=')?;
                (name == SESSION_COOKIE).then(|| value.to_owned())
            })
        })
}

async fn auth_status(
    State(state): State<AppState>,
    session: Option<axum::extract::Extension<SessionInfo>>,
) -> AppResult<Json<AuthStatus>> {
    Ok(Json(
        auth_status_value(&state, session.map(|value| value.0)).await?,
    ))
}

async fn auth_status_value(
    state: &AppState,
    session: Option<SessionInfo>,
) -> AppResult<AuthStatus> {
    let required = state.requires_auth();
    Ok(AuthStatus {
        required,
        authenticated: !required || session.is_some(),
        password_configured: state.auth.has_admin_password().await?,
        csrf_token: session.map(|session| session.csrf_token),
    })
}

async fn login(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(input): Json<LoginRequest>,
) -> AppResult<Response> {
    if !state.requires_auth() {
        return Ok(Json(AuthStatus {
            required: false,
            authenticated: true,
            password_configured: state.auth.has_admin_password().await?,
            csrf_token: None,
        })
        .into_response());
    }
    let session_hours = load_settings(&state).await?.security.session_hours as i64;
    let created = state
        .auth
        .login(peer.ip(), &input.password, session_hours)
        .await?;
    let max_age = (created.expires_at - Utc::now()).num_seconds().max(0);
    let cookie = format!(
        "{SESSION_COOKIE}={}; Path=/; Max-Age={max_age}; HttpOnly; Secure; SameSite=Strict",
        created.token
    );
    let mut response = Json(AuthStatus {
        required: true,
        authenticated: true,
        password_configured: true,
        csrf_token: Some(created.csrf_token),
    })
    .into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie)
            .map_err(|_| AppError::Internal("could not create session cookie".to_owned()))?,
    );
    Ok(response)
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> AppResult<Response> {
    if let Some(token) = session_cookie(&headers) {
        state.auth.logout(&token).await?;
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static(
            "paperlessllm_session=; Path=/; Max-Age=0; HttpOnly; Secure; SameSite=Strict",
        ),
    );
    Ok(response)
}

async fn set_password(
    State(state): State<AppState>,
    Json(input): Json<PasswordRequest>,
) -> AppResult<Json<OkResponse>> {
    // In LAN mode this route is already session- and CSRF-protected. On
    // localhost, OS-level access is the trust boundary by design.
    let _ = input.current_password;
    state.auth.set_admin_password(&input.password).await?;
    Ok(Json(OkResponse { ok: true }))
}

async fn status(
    State(state): State<AppState>,
    session: Option<axum::extract::Extension<SessionInfo>>,
) -> AppResult<Json<AppStatus>> {
    Ok(Json(
        build_status(&state, session.map(|value| value.0)).await?,
    ))
}

async fn build_status(state: &AppState, session: Option<SessionInfo>) -> AppResult<AppStatus> {
    let configured = state.database.is_configured().await?;
    let worker = state.worker.runtime_state().await;
    let paperless = connection_health(
        state,
        "paperless_health",
        load_connection(state).await?.is_some(),
    )
    .await?;
    let settings = load_settings(state).await?;
    let provider = if let Some(id) = settings.active_provider_id.as_deref() {
        connection_health(state, &format!("provider_health:{id}"), true).await?
    } else {
        ConnectionHealth {
            state: "unconfigured".to_owned(),
            message: Some("No active provider".to_owned()),
            checked_at: None,
        }
    };
    Ok(AppStatus {
        version: env!("CARGO_PKG_VERSION"),
        setup_required: !configured,
        worker: WorkerStatusDto {
            state: worker.state,
            last_scan_at: worker.last_scan_at,
            next_scan_at: worker.next_scan_at,
            active_job_id: worker.active_job_id,
        },
        paperless,
        provider,
        queue: worker::queue_counts(state).await?,
        auth: auth_status_value(state, session).await?,
    })
}

async fn connection_health(
    state: &AppState,
    key: &str,
    configured: bool,
) -> AppResult<ConnectionHealth> {
    if !configured {
        return Ok(ConnectionHealth {
            state: "unconfigured".to_owned(),
            message: Some("Not configured".to_owned()),
            checked_at: None,
        });
    }
    let value = state.database.get_setting::<Value>(key).await?;
    Ok(match value {
        Some(value) => ConnectionHealth {
            state: if value.get("ok").and_then(Value::as_bool).unwrap_or(false) {
                "connected"
            } else {
                "disconnected"
            }
            .to_owned(),
            message: value
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_owned),
            checked_at: value
                .get("tested_at")
                .and_then(Value::as_str)
                .map(str::to_owned),
        },
        None => ConnectionHealth {
            state: "degraded".to_owned(),
            message: Some("Configured but not tested in this installation".to_owned()),
            checked_at: None,
        },
    })
}

async fn bootstrap(
    State(state): State<AppState>,
    session: Option<axum::extract::Extension<SessionInfo>>,
) -> AppResult<Json<BootstrapResponse>> {
    let settings = load_settings(&state).await?;
    let catalogs = state
        .database
        .get_setting::<Catalogs>("paperless_catalog_cache")
        .await?;
    Ok(Json(BootstrapResponse {
        status: build_status(&state, session.map(|value| value.0)).await?,
        settings,
        providers: providers::list(&state).await?,
        catalogs,
    }))
}

async fn get_settings(State(state): State<AppState>) -> AppResult<Json<AppSettings>> {
    Ok(Json(load_settings(&state).await?))
}

async fn put_settings(
    State(state): State<AppState>,
    Json(settings): Json<AppSettings>,
) -> AppResult<Json<AppSettings>> {
    Ok(Json(save_settings(&state, settings).await?))
}

async fn test_paperless(
    State(state): State<AppState>,
    Json(input): Json<PaperlessTestRequest>,
) -> AppResult<Json<PaperlessTestResult>> {
    let token = match input.api_token.filter(|token| !token.trim().is_empty()) {
        Some(token) => Zeroizing::new(token),
        None => state
            .secrets
            .get(settings::PAPERLESS_TOKEN_KEY)
            .await?
            .ok_or_else(|| AppError::Validation("enter a Paperless API token".to_owned()))?,
    };
    let client = PaperlessClient::new(&input.base_url, token, input.custom_ca_pem.as_deref())?;
    match client.probe().await {
        Ok(probe) => {
            let result = PaperlessTestResult {
                ok: true,
                version: Some(probe.server_version.clone()),
                api_version: Some(probe.api_version),
                user: None,
                document_count: Some(probe.document_count),
                message: "Connected to a compatible Paperless-ngx v3 / API v10 server".to_owned(),
            };
            state
                .database
                .set_setting(
                    "paperless_health",
                    &json!({
                        "ok": true,
                        "tested_at": Utc::now().to_rfc3339(),
                        "message": result.message,
                        "version": probe.server_version,
                        "api_version": probe.api_version,
                    }),
                )
                .await?;
            Ok(Json(result))
        }
        Err(error) => {
            state
                .database
                .set_setting(
                    "paperless_health",
                    &json!({
                        "ok": false,
                        "tested_at": Utc::now().to_rfc3339(),
                        "message": error.to_string(),
                    }),
                )
                .await?;
            Err(error)
        }
    }
}

async fn configured_paperless(state: &AppState) -> AppResult<PaperlessClient> {
    let connection = load_connection(state)
        .await?
        .ok_or_else(|| AppError::Validation("Paperless is not configured".to_owned()))?;
    let token = state
        .secrets
        .get(&connection.token_secret_key)
        .await?
        .ok_or_else(|| AppError::Validation("Paperless API token is not configured".to_owned()))?;
    PaperlessClient::new(
        &connection.base_url,
        token,
        connection.custom_ca_pem.as_deref(),
    )
}

async fn paperless_catalogs(State(state): State<AppState>) -> AppResult<Json<Catalogs>> {
    let client = configured_paperless(&state).await?;
    let (tags, correspondents, document_types, custom_fields) = tokio::try_join!(
        client.list_tags(),
        client.list_correspondents(),
        client.list_document_types(),
        client.list_custom_fields(),
    )?;
    let catalogs = Catalogs {
        tags: tags
            .into_iter()
            .map(|value| CatalogItem {
                id: value.id,
                name: value.name,
                slug: None,
                color: value.color,
                data_type: None,
                extra_data: None,
            })
            .collect(),
        correspondents: correspondents.into_iter().map(named_catalog).collect(),
        document_types: document_types.into_iter().map(named_catalog).collect(),
        custom_fields: custom_fields
            .into_iter()
            .filter(|value| !matches!(value.data_type.as_str(), "documentlink" | "document_link"))
            .map(|value| CatalogItem {
                id: value.id,
                name: value.name,
                slug: None,
                color: None,
                data_type: Some(frontend_custom_type(&value.data_type)),
                extra_data: Some(value.extra_data),
            })
            .collect(),
        fetched_at: Utc::now().to_rfc3339(),
    };
    cache_catalogs(&state, &catalogs).await?;
    Ok(Json(catalogs))
}

fn named_catalog(value: crate::paperless::PaperlessNamedObject) -> CatalogItem {
    CatalogItem {
        id: value.id,
        name: value.name,
        slug: None,
        color: None,
        data_type: None,
        extra_data: None,
    }
}

fn frontend_custom_type(value: &str) -> String {
    match value {
        "longtext" => "long_text".to_owned(),
        other => other.to_owned(),
    }
}

async fn create_custom_field(
    State(state): State<AppState>,
    Json(mut input): Json<CustomFieldDefinitionCreate>,
) -> AppResult<Json<CatalogItem>> {
    if input.data_type == "long_text" {
        input.data_type = "longtext".to_owned();
    }
    if matches!(input.data_type.as_str(), "documentlink" | "document_link") {
        return Err(AppError::Validation(
            "document-link custom fields are not supported".to_owned(),
        ));
    }
    let client = configured_paperless(&state).await?;
    let created = client.create_custom_field(&input).await?;
    let item = CatalogItem {
        id: created.id,
        name: created.name,
        slug: None,
        color: None,
        data_type: Some(frontend_custom_type(&created.data_type)),
        extra_data: Some(created.extra_data),
    };
    // Refresh the whole cache to keep server-assigned select option IDs and
    // concurrent Paperless changes authoritative.
    let _ = paperless_catalogs(State(state)).await?;
    Ok(Json(item))
}

async fn list_providers(State(state): State<AppState>) -> AppResult<Json<Vec<ProviderProfileDto>>> {
    Ok(Json(providers::list(&state).await?))
}

async fn create_provider(
    State(state): State<AppState>,
    Json(input): Json<ProviderProfileDto>,
) -> AppResult<(StatusCode, Json<ProviderProfileDto>)> {
    Ok((
        StatusCode::CREATED,
        Json(providers::create(&state, input).await?),
    ))
}

async fn update_provider(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<ProviderProfileDto>,
) -> AppResult<Json<ProviderProfileDto>> {
    Ok(Json(providers::update(&state, &id, input).await?))
}

async fn delete_provider(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    providers::delete(&state, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn provider_models(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<types::ModelsResponse>> {
    Ok(Json(providers::models(&state, &id).await?))
}

async fn test_provider(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<types::ProviderTestResult>> {
    Ok(Json(providers::test(&state, &id).await?))
}

async fn scan_now(State(state): State<AppState>) -> AppResult<Json<worker::ScanResult>> {
    Ok(Json(state.worker.scan_now().await?))
}

#[derive(Debug, Deserialize)]
struct JobsQuery {
    state: Option<String>,
    q: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Debug, Serialize, FromRow)]
struct JobDto {
    id: String,
    document_id: i64,
    document_title: String,
    document_url: Option<String>,
    state: String,
    attempt: i64,
    provider_name: Option<String>,
    model: Option<String>,
    functions_json: String,
    created_at: String,
    updated_at: String,
    error: Option<String>,
    truncated: bool,
    token_total: i64,
}

#[derive(Debug, Serialize)]
struct JobResponseItem {
    id: String,
    document_id: i64,
    document_title: String,
    document_url: Option<String>,
    state: String,
    attempt: i64,
    provider_name: Option<String>,
    model: Option<String>,
    functions: Vec<String>,
    created_at: String,
    updated_at: String,
    error: Option<String>,
    truncated: bool,
    token_total: i64,
}

#[derive(Debug, Serialize)]
struct JobsResponse {
    items: Vec<JobResponseItem>,
    total: i64,
    limit: i64,
    offset: i64,
}

async fn list_jobs(
    State(state): State<AppState>,
    Query(query): Query<JobsQuery>,
) -> AppResult<Json<JobsResponse>> {
    if query
        .state
        .as_deref()
        .is_some_and(|state| !worker_state_valid(state))
    {
        return Err(AppError::Validation("unknown job state".to_owned()));
    }
    let limit = query.limit.unwrap_or(50).clamp(1, 250);
    let offset = query.offset.unwrap_or(0).max(0);
    let search = query
        .q
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("%{}%", value.trim()));
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM jobs j WHERE (? IS NULL OR j.status=?) AND (? IS NULL OR j.document_title LIKE ? OR CAST(j.paperless_document_id AS TEXT) LIKE ?)",
    )
    .bind(query.state.as_deref()).bind(query.state.as_deref())
    .bind(search.as_deref()).bind(search.as_deref()).bind(search.as_deref())
    .fetch_one(state.database.pool()).await?;
    let rows: Vec<JobDto> = sqlx::query_as(
        "SELECT j.id,j.paperless_document_id AS document_id,COALESCE(j.document_title,'') AS document_title,NULL AS document_url,j.status AS state,j.attempt_count AS attempt,\
         pp.name AS provider_name,pp.model,\
         json_array(CASE WHEN pr.tags_enabled THEN 'tags' END,CASE WHEN pr.correspondent_enabled THEN 'correspondent' END,CASE WHEN pr.document_type_enabled THEN 'document_type' END,CASE WHEN pr.title_enabled THEN 'title' END,CASE WHEN pr.custom_fields_enabled THEN 'custom_fields' END) AS functions_json,\
         j.queued_at AS created_at,j.updated_at,j.error_message AS error,j.truncated,COALESCE((SELECT SUM(u.total_tokens) FROM usage_events u WHERE u.job_id=j.id),0) AS token_total \
         FROM jobs j JOIN processing_profiles pr ON pr.id=j.processing_profile_id JOIN provider_profiles pp ON pp.id=pr.provider_profile_id \
         WHERE (? IS NULL OR j.status=?) AND (? IS NULL OR j.document_title LIKE ? OR CAST(j.paperless_document_id AS TEXT) LIKE ?) \
         ORDER BY j.updated_at DESC LIMIT ? OFFSET ?",
    )
    .bind(query.state.as_deref()).bind(query.state.as_deref())
    .bind(search.as_deref()).bind(search.as_deref()).bind(search.as_deref())
    .bind(limit).bind(offset)
    .fetch_all(state.database.pool()).await?;
    let paperless_base = load_connection(&state)
        .await?
        .map(|connection| connection.base_url);
    let items = rows
        .into_iter()
        .map(|row| {
            let functions = serde_json::from_str::<Vec<Option<String>>>(&row.functions_json)
                .unwrap_or_default()
                .into_iter()
                .flatten()
                .collect();
            JobResponseItem {
                id: row.id,
                document_id: row.document_id,
                document_title: row.document_title,
                document_url: paperless_base
                    .as_deref()
                    .and_then(|base| paperless_document_url(base, row.document_id)),
                state: row.state,
                attempt: row.attempt,
                provider_name: row.provider_name,
                model: row.model,
                functions,
                created_at: row.created_at,
                updated_at: row.updated_at,
                error: row.error,
                truncated: row.truncated,
                token_total: row.token_total,
            }
        })
        .collect();
    Ok(Json(JobsResponse {
        items,
        total,
        limit,
        offset,
    }))
}

async fn retry_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<OkResponse>> {
    worker::retry_job(&state, &id).await?;
    Ok(Json(OkResponse { ok: true }))
}

fn worker_state_valid(value: &str) -> bool {
    matches!(
        value,
        "queued"
            | "analyzing"
            | "awaiting_review"
            | "applying"
            | "completed"
            | "budget_blocked"
            | "failed"
            | "rejected"
    )
}

#[derive(Debug, Serialize)]
struct ReviewItem {
    id: String,
    job_id: String,
    document_id: i64,
    document_title: String,
    document_url: Option<String>,
    created_at: String,
    current: types::ReviewSuggestion,
    suggested: types::ReviewSuggestion,
    warnings: Vec<String>,
    token_total: i64,
}

#[derive(Debug, Serialize)]
struct ReviewsResponse {
    items: Vec<ReviewItem>,
    total: i64,
}

async fn list_reviews(State(state): State<AppState>) -> AppResult<Json<ReviewsResponse>> {
    let rows: Vec<(String, i64, String, String, String, i64)> = sqlx::query_as(
        "SELECT j.id,j.paperless_document_id,COALESCE(j.document_title,''),j.queued_at,COALESCE(j.suggestion_json,'{}'),COALESCE((SELECT SUM(u.total_tokens) FROM usage_events u WHERE u.job_id=j.id),0) FROM jobs j WHERE j.status='awaiting_review' ORDER BY j.queued_at",
    )
    .fetch_all(state.database.pool()).await?;
    let total = rows.len() as i64;
    let paperless_base = load_connection(&state)
        .await?
        .map(|connection| connection.base_url);
    let mut items = Vec::with_capacity(rows.len());
    for (id, document_id, document_title, created_at, raw, token_total) in rows {
        let stored: StoredSuggestion = serde_json::from_str(&raw)
            .map_err(|_| AppError::Internal("stored review suggestion is invalid".to_owned()))?;
        items.push(ReviewItem {
            id: id.clone(),
            job_id: id,
            document_id,
            document_title,
            document_url: paperless_base
                .as_deref()
                .and_then(|base| paperless_document_url(base, document_id)),
            created_at,
            current: stored.current,
            suggested: stored.suggested,
            warnings: stored.warnings,
            token_total,
        });
    }
    Ok(Json(ReviewsResponse { items, total }))
}

async fn approve_review(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<ApproveRequest>,
) -> AppResult<Json<OkResponse>> {
    worker::approve_review(&state, &id, input.suggestion).await?;
    Ok(Json(OkResponse { ok: true }))
}

async fn reject_review(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<OkResponse>> {
    worker::reject_review(&state, &id).await?;
    Ok(Json(OkResponse { ok: true }))
}

#[derive(Debug, Deserialize)]
struct RangeQuery {
    range: Option<String>,
}

#[derive(Debug, Serialize, FromRow, Default)]
struct UsageTotals {
    input_tokens: i64,
    output_tokens: i64,
    reasoning_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    total_tokens: i64,
    requests: i64,
    failed_requests: i64,
    reported_requests: i64,
    preflight_counted_requests: i64,
    estimated_requests: i64,
    unknown_requests: i64,
}

#[derive(Debug, Serialize)]
struct BudgetStatus {
    daily_used: i64,
    daily_limit: Option<i64>,
    monthly_used: i64,
    monthly_limit: Option<i64>,
    warning_percent: i64,
    timezone: String,
}

#[derive(Debug, Serialize, FromRow)]
struct ProviderBreakdown {
    provider: String,
    model: String,
    total_tokens: i64,
    requests: i64,
}

#[derive(Debug, Serialize, FromRow)]
struct FunctionBreakdown {
    function: String,
    total_tokens: i64,
    documents: i64,
}

#[derive(Debug, Serialize)]
struct StatsSummaryResponse {
    range: String,
    totals: UsageTotals,
    budgets: BudgetStatus,
    by_provider: Vec<ProviderBreakdown>,
    by_function: Vec<FunctionBreakdown>,
}

async fn stats_summary(
    State(state): State<AppState>,
    Query(query): Query<RangeQuery>,
) -> AppResult<Json<StatsSummaryResponse>> {
    let range = normalize_range(query.range.as_deref())?;
    let start = stats_start(range).map(sqlite_time);
    let totals = usage_totals(&state, start.as_deref()).await?;
    let by_provider: Vec<ProviderBreakdown> = sqlx::query_as(
        "SELECT provider_kind AS provider,model,SUM(total_tokens) AS total_tokens,COUNT(*) AS requests FROM usage_events WHERE (? IS NULL OR created_at>=?) GROUP BY provider_kind,model ORDER BY total_tokens DESC",
    )
    .bind(start.as_deref()).bind(start.as_deref()).fetch_all(state.database.pool()).await?;
    // One structured request may perform several functions. Each enabled
    // function receives the request's token total in this attribution view;
    // rows are deliberately not summed into the overall total.
    let by_function: Vec<FunctionBreakdown> = sqlx::query_as(
        "SELECT CAST(f.value AS TEXT) AS function,SUM(u.total_tokens) AS total_tokens,COUNT(DISTINCT u.job_id) AS documents \
         FROM usage_events u JOIN json_each(u.functions_json) f \
         WHERE (? IS NULL OR u.created_at>=?) AND f.value IS NOT NULL \
         GROUP BY f.value ORDER BY total_tokens DESC, function",
    )
    .bind(start.as_deref())
    .bind(start.as_deref())
    .fetch_all(state.database.pool())
    .await?;
    Ok(Json(StatsSummaryResponse {
        range: range.to_owned(),
        totals,
        budgets: budget_status(&state).await?,
        by_provider,
        by_function,
    }))
}

#[derive(Debug, Serialize, FromRow)]
struct StatsPoint {
    period: String,
    input_tokens: i64,
    output_tokens: i64,
    reasoning_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    total_tokens: i64,
    requests: i64,
    failed_requests: i64,
    reported_requests: i64,
    preflight_counted_requests: i64,
    estimated_requests: i64,
    unknown_requests: i64,
}

#[derive(Debug, Serialize)]
struct StatsSeriesResponse {
    range: String,
    points: Vec<StatsPoint>,
}

async fn stats_series(
    State(state): State<AppState>,
    Query(query): Query<RangeQuery>,
) -> AppResult<Json<StatsSeriesResponse>> {
    let range = normalize_range(query.range.as_deref())?;
    let start = stats_start(range).map(sqlite_time);
    let bucket = if matches!(range, "12m" | "lifetime") {
        "%Y-%m"
    } else {
        "%Y-%m-%d"
    };
    let mut builder = QueryBuilder::<Sqlite>::new("SELECT strftime('");
    builder.push(bucket).push("',created_at) AS period,\
        SUM(input_tokens) AS input_tokens,SUM(output_tokens) AS output_tokens,\
        SUM(reasoning_tokens) AS reasoning_tokens,SUM(cache_read_tokens) AS cache_read_tokens,\
        SUM(cache_write_tokens) AS cache_write_tokens,SUM(total_tokens) AS total_tokens,\
        COUNT(*) AS requests,SUM(CASE WHEN status!='succeeded' THEN 1 ELSE 0 END) AS failed_requests,\
        SUM(CASE WHEN usage_source='reported' THEN 1 ELSE 0 END) AS reported_requests,\
        SUM(CASE WHEN usage_source='preflight' THEN 1 ELSE 0 END) AS preflight_counted_requests,\
        SUM(CASE WHEN usage_source='estimated' THEN 1 ELSE 0 END) AS estimated_requests,\
        SUM(CASE WHEN usage_source IN ('unknown','reserved_after_unknown') THEN 1 ELSE 0 END) AS unknown_requests FROM usage_events");
    if let Some(start) = start.as_deref() {
        builder.push(" WHERE created_at>=").push_bind(start);
    }
    builder.push(" GROUP BY period ORDER BY period");
    let points = builder
        .build_query_as::<StatsPoint>()
        .fetch_all(state.database.pool())
        .await?;
    Ok(Json(StatsSeriesResponse {
        range: range.to_owned(),
        points,
    }))
}

async fn usage_totals(state: &AppState, start: Option<&str>) -> AppResult<UsageTotals> {
    Ok(sqlx::query_as(
        "SELECT COALESCE(SUM(input_tokens),0) AS input_tokens,COALESCE(SUM(output_tokens),0) AS output_tokens,COALESCE(SUM(reasoning_tokens),0) AS reasoning_tokens,\
         COALESCE(SUM(cache_read_tokens),0) AS cache_read_tokens,COALESCE(SUM(cache_write_tokens),0) AS cache_write_tokens,COALESCE(SUM(total_tokens),0) AS total_tokens,\
         COUNT(*) AS requests,COALESCE(SUM(CASE WHEN status!='succeeded' THEN 1 ELSE 0 END),0) AS failed_requests,\
         COALESCE(SUM(CASE WHEN usage_source='reported' THEN 1 ELSE 0 END),0) AS reported_requests,\
         COALESCE(SUM(CASE WHEN usage_source='preflight' THEN 1 ELSE 0 END),0) AS preflight_counted_requests,\
         COALESCE(SUM(CASE WHEN usage_source='estimated' THEN 1 ELSE 0 END),0) AS estimated_requests,\
         COALESCE(SUM(CASE WHEN usage_source IN ('unknown','reserved_after_unknown') THEN 1 ELSE 0 END),0) AS unknown_requests \
         FROM usage_events WHERE (? IS NULL OR created_at>=?)",
    ).bind(start).bind(start).fetch_one(state.database.pool()).await?)
}

async fn budget_status(state: &AppState) -> AppResult<BudgetStatus> {
    let settings = load_settings(state).await?;
    let timezone: Tz = settings
        .limits
        .reset_timezone
        .parse()
        .map_err(|_| AppError::Internal("stored timezone is invalid".to_owned()))?;
    let day = sqlite_time(period_start(Utc::now(), timezone, false)?);
    let month = sqlite_time(period_start(Utc::now(), timezone, true)?);
    let daily_used: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_tokens),0) FROM usage_events WHERE created_at>=?",
    )
    .bind(day)
    .fetch_one(state.database.pool())
    .await?;
    let monthly_used: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_tokens),0) FROM usage_events WHERE created_at>=?",
    )
    .bind(month)
    .fetch_one(state.database.pool())
    .await?;
    Ok(BudgetStatus {
        daily_used,
        daily_limit: settings.limits.global_budget.daily,
        monthly_used,
        monthly_limit: settings.limits.global_budget.monthly,
        warning_percent: settings.limits.warning_percent as i64,
        timezone: settings.limits.reset_timezone,
    })
}

fn normalize_range(value: Option<&str>) -> AppResult<&'static str> {
    match value.unwrap_or("month") {
        "day" | "today" => Ok("day"),
        "week" | "7d" => Ok("week"),
        "month" | "30d" => Ok("month"),
        "90d" => Ok("90d"),
        "12m" => Ok("12m"),
        "lifetime" | "all" => Ok("lifetime"),
        _ => Err(AppError::Validation("unknown statistics range".to_owned())),
    }
}

fn stats_start(range: &str) -> Option<chrono::DateTime<Utc>> {
    match range {
        "day" => Some(Utc::now() - TimeDelta::days(1)),
        "week" => Some(Utc::now() - TimeDelta::days(7)),
        "month" => Some(Utc::now() - TimeDelta::days(30)),
        "90d" => Some(Utc::now() - TimeDelta::days(90)),
        "12m" => Some(Utc::now() - TimeDelta::days(365)),
        _ => None,
    }
}

fn sqlite_time(value: chrono::DateTime<Utc>) -> String {
    value.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

fn paperless_document_url(base: &str, id: i64) -> Option<String> {
    let mut base = Url::parse(base).ok()?;
    if !base.path().ends_with('/') {
        base.set_path(&format!("{}/", base.path()));
    }
    base.join(&format!("documents/{id}/details"))
        .ok()
        .map(|value| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_parser_does_not_accept_prefixes() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("xpaperlessllm_session=no; paperlessllm_session=yes"),
        );
        assert_eq!(session_cookie(&headers).as_deref(), Some("yes"));
    }

    #[test]
    fn statistics_ranges_are_bounded() {
        assert_eq!(normalize_range(Some("30d")).unwrap(), "month");
        assert!(normalize_range(Some("forever-ish")).is_err());
    }
}
