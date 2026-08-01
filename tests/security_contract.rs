use std::{
    fs::File,
    net::SocketAddr,
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use paperless_llm::{
    AppError, RuntimeOptions,
    api::{settings::save_settings, types::AppSettings},
    app::{build_router, initialize},
    auth::AuthService,
    db::Database,
};
use serde_json::Value;
use tempfile::TempDir;
use tokio::time::sleep;
use tower::ServiceExt;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

struct ChildGuard {
    child: Child,
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn process_log(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| format!("<log unavailable: {error}>"))
}

fn runtime(path: &Path) -> RuntimeOptions {
    RuntimeOptions {
        data_dir: path.to_owned(),
        bind: "127.0.0.1".parse().expect("loopback"),
        port: 8765,
        open_browser: false,
    }
}

#[tokio::test]
async fn lan_settings_cannot_be_enabled_before_password_exists() {
    let directory = TempDir::new().expect("temporary directory");
    let state = initialize(runtime(directory.path())).await.expect("state");
    let mut settings = AppSettings::default();
    settings.security.lan_enabled = true;
    settings.security.reverse_proxy_tls = true;
    let error = save_settings(&state, settings)
        .await
        .expect_err("LAN settings must require a password");
    assert!(matches!(error, AppError::Conflict(_)));
    assert!(!state.requires_auth());
}

#[tokio::test]
async fn localhost_shutdown_requires_an_explicit_non_simple_header() {
    let directory = TempDir::new().expect("temporary directory");
    let state = initialize(runtime(directory.path())).await.expect("state");
    let app = build_router(state);

    for header in [None, Some("not-shutdown")] {
        let mut request = Request::builder()
            .method("POST")
            .uri("/api/shutdown")
            .header(header::HOST, "localhost:8765");
        if let Some(value) = header {
            request = request.header("x-paperlessllm-action", value);
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::empty()).expect("request"))
            .await
            .expect("shutdown response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    let accepted = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/shutdown")
                .header(header::HOST, "localhost:8765")
                .header("x-paperlessllm-action", "shutdown")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("shutdown response");
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn localhost_shutdown_accepts_only_the_configured_loopback_browser_origin() {
    let directory = TempDir::new().expect("temporary directory");
    let state = initialize(runtime(directory.path())).await.expect("state");
    let app = build_router(state);

    for origin in [
        "http://localhost:8765",
        "http://127.0.0.1:8765",
        "http://127.25.50.75:8765",
        "http://[::1]:8765",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/shutdown")
                    .header(header::HOST, "localhost:8765")
                    .header("x-paperlessllm-action", "shutdown")
                    .header(header::ORIGIN, origin)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("shutdown response");
        assert_eq!(response.status(), StatusCode::ACCEPTED, "origin {origin}");
    }

    for origin in [
        "null",
        "not a URL",
        "https://localhost:8765",
        "http://localhost",
        "http://localhost:8766",
        "http://localhost.evil:8765",
        "http://192.0.2.10:8765",
        "http://user@localhost:8765",
        "http://localhost:8765/path",
        "http://localhost:8765?query=yes",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/shutdown")
                    .header(header::HOST, "localhost:8765")
                    .header("x-paperlessllm-action", "shutdown")
                    .header(header::ORIGIN, origin)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("shutdown response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "origin {origin}");
    }

    let duplicate_origin = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/shutdown")
                .header(header::HOST, "localhost:8765")
                .header("x-paperlessllm-action", "shutdown")
                .header(header::ORIGIN, "http://localhost:8765")
                .header(header::ORIGIN, "http://localhost:8765")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("shutdown response");
    assert_eq!(duplicate_origin.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn localhost_private_apis_accept_only_the_configured_loopback_authority() {
    let directory = TempDir::new().expect("temporary directory");
    let state = initialize(runtime(directory.path())).await.expect("state");
    let app = build_router(state);

    for host in [
        "localhost:8765",
        "LOCALHOST:8765",
        "127.0.0.1:8765",
        "127.25.50.75:8765",
        "[::1]:8765",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/status")
                    .header(header::HOST, host)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("status response");
        assert_eq!(response.status(), StatusCode::OK, "host {host}");
    }

    let absolute_uri = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("http://localhost:8765/api/status")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("absolute URI response");
    assert_eq!(absolute_uri.status(), StatusCode::OK);

    let missing_host = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/status")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("missing Host response");
    assert_eq!(missing_host.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        missing_host
            .headers()
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("no-store, private")
    );

    for host in [
        "localhost",
        "localhost:8766",
        "localhost.evil:8765",
        "192.0.2.10:8765",
        "user@localhost:8765",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/status")
                    .header(header::HOST, host)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("status response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "host {host}");
    }

    for uri in [
        "https://localhost:8765/api/status",
        "http://localhost.evil:8765/api/status",
        "http://127.0.0.1:8766/api/status",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("absolute URI response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "URI {uri}");
    }

    let duplicate_host = app
        .oneshot(
            Request::builder()
                .uri("/api/status")
                .header(header::HOST, "localhost:8765")
                .header(header::HOST, "localhost:8765")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("duplicate Host response");
    assert_eq!(duplicate_host.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn localhost_unsafe_apis_accept_only_loopback_browser_origins() {
    let directory = TempDir::new().expect("temporary directory");
    let state = initialize(runtime(directory.path())).await.expect("state");
    let database = state.database.clone();
    let app = build_router(state);

    for origin in [
        "null",
        "https://localhost:8765",
        "http://localhost:8766",
        "http://localhost.evil:8765",
        "http://192.0.2.10:8765",
        "http://user@localhost:8765",
        "http://localhost:8765/path",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/worker/pause")
                    .header(header::HOST, "localhost:8765")
                    .header(header::ORIGIN, origin)
                    .header("x-paperlessllm-action", "worker-control")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("worker-control response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "origin {origin}");
    }
    assert_eq!(
        database
            .get_setting::<bool>("worker_paused")
            .await
            .expect("pause setting"),
        None
    );

    for origin in [
        None,
        Some("http://localhost:8765"),
        Some("http://127.0.0.1:8765"),
        Some("http://127.25.50.75:8765"),
        Some("http://[::1]:8765"),
    ] {
        let mut pause = Request::builder()
            .method("POST")
            .uri("/api/worker/pause")
            .header(header::HOST, "localhost:8765")
            .header("x-paperlessllm-action", "worker-control");
        if let Some(origin) = origin {
            pause = pause.header(header::ORIGIN, origin);
        }
        let response = app
            .clone()
            .oneshot(pause.body(Body::empty()).expect("request"))
            .await
            .expect("pause response");
        assert_eq!(response.status(), StatusCode::OK, "origin {origin:?}");

        let mut resume = Request::builder()
            .method("POST")
            .uri("/api/worker/resume")
            .header(header::HOST, "localhost:8765")
            .header("x-paperlessllm-action", "worker-control");
        if let Some(origin) = origin {
            resume = resume.header(header::ORIGIN, origin);
        }
        let response = app
            .clone()
            .oneshot(resume.body(Body::empty()).expect("request"))
            .await
            .expect("resume response");
        assert_eq!(response.status(), StatusCode::OK, "origin {origin:?}");
    }

    let duplicate_origin = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/worker/pause")
                .header(header::HOST, "localhost:8765")
                .header(header::ORIGIN, "http://localhost:8765")
                .header(header::ORIGIN, "http://localhost:8765")
                .header("x-paperlessllm-action", "worker-control")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("duplicate Origin response");
    assert_eq!(duplicate_origin.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn hostile_paperless_error_bodies_never_reach_the_api_or_health_storage() {
    const HOSTILE_OCR: &str = "OCR_PRIVATE_ECHO_6f9f";
    const HOSTILE_TOKEN: &str = "sk-hostile-paperless-token";

    let paperless = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/documents/"))
        .respond_with(ResponseTemplate::new(500).set_body_json(serde_json::json!({
            "error": format!("{HOSTILE_OCR} {HOSTILE_TOKEN}")
        })))
        .expect(1)
        .mount(&paperless)
        .await;

    let directory = TempDir::new().expect("temporary directory");
    let state = initialize(runtime(directory.path())).await.expect("state");
    let database = state.database.clone();
    let response = build_router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/paperless/test")
                .header(header::HOST, "localhost:8765")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "base_url": paperless.uri(),
                        "api_token": "request-only-paperless-token",
                        "custom_ca_pem": null
                    })
                    .to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("Paperless test response");
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("response body")
        .to_bytes();
    let body = String::from_utf8(body.to_vec()).expect("UTF-8 error body");
    assert!(body.contains("Paperless returned HTTP 500"));
    assert!(!body.contains(HOSTILE_OCR));
    assert!(!body.contains(HOSTILE_TOKEN));

    let health = database
        .get_setting::<Value>("paperless_health")
        .await
        .expect("health setting")
        .expect("failed health value");
    let health = health.to_string();
    assert!(health.contains("Paperless returned HTTP 500"));
    assert!(!health.contains(HOSTILE_OCR));
    assert!(!health.contains(HOSTILE_TOKEN));
    paperless.verify().await;
}

#[tokio::test]
async fn hostile_provider_error_bodies_never_reach_api_responses() {
    const HOSTILE_PROMPT: &str = "PROMPT_PRIVATE_ECHO_8d31";
    const HOSTILE_TOKEN: &str = "sk-hostile-provider-token";

    let provider = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": { "message": format!("{HOSTILE_PROMPT} {HOSTILE_TOKEN}") }
        })))
        .expect(1)
        .mount(&provider)
        .await;

    let directory = TempDir::new().expect("temporary directory");
    let state = initialize(runtime(directory.path())).await.expect("state");
    sqlx::query(
        "INSERT INTO provider_profiles(id,name,provider_kind,base_url,model,api_family) \
         VALUES('hostile','Hostile mock','custom',?,'mock-model','responses')",
    )
    .bind(format!("{}/v1", provider.uri()))
    .execute(state.database.pool())
    .await
    .expect("provider fixture");

    let response = build_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/providers/hostile/models")
                .header(header::HOST, "localhost:8765")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("provider models response");
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("response body")
        .to_bytes();
    let body = String::from_utf8(body.to_vec()).expect("UTF-8 error body");
    assert!(body.contains("Provider request failed with HTTP 400"));
    assert!(!body.contains(HOSTILE_PROMPT));
    assert!(!body.contains(HOSTILE_TOKEN));
    provider.verify().await;
}

#[tokio::test]
async fn serve_process_returns_the_shutdown_body_before_exiting_successfully() {
    let directory = TempDir::new().expect("temporary directory");
    let log_path = directory.path().join("serve-test.log");
    let stdout = File::create(&log_path).expect("create process log");
    let stderr = stdout.try_clone().expect("clone process log");
    let child = Command::new(env!("CARGO_BIN_EXE_paperless-llm"))
        .arg("serve")
        .arg("--data-dir")
        .arg(directory.path())
        .arg("--bind")
        .arg("127.0.0.1")
        .arg("--port")
        .arg("0")
        .arg("--no-open")
        .env("RUST_LOG", "paperless_llm=info")
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .expect("spawn PaperlessLLM serve process");
    let mut process = ChildGuard { child };
    let client = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_millis(250))
        .timeout(Duration::from_secs(2))
        .build()
        .expect("HTTP client");
    let port = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(status) = process.child.try_wait().expect("inspect child") {
                panic!(
                    "PaperlessLLM exited before becoming healthy with {status}: {}",
                    process_log(&log_path)
                );
            }
            if let Some(port) = process_log(&log_path).lines().find_map(|line| {
                line.split_whitespace()
                    .find_map(|field| field.strip_prefix("local_address="))
                    .and_then(|address| address.parse::<SocketAddr>().ok())
                    .map(|address| address.port())
            }) {
                break port;
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "PaperlessLLM did not become healthy: {}",
            process_log(&log_path)
        )
    });
    let base_url = format!("http://127.0.0.1:{port}");
    let health = client
        .get(format!("{base_url}/healthz"))
        .send()
        .await
        .expect("reach ephemeral listener");
    assert!(health.status().is_success());
    let health_body = health
        .json::<Value>()
        .await
        .expect("read health response body");
    assert_eq!(
        health_body.get("build_id").and_then(Value::as_str),
        Some(paperless_llm::BUILD_ID)
    );

    let response = client
        .post(format!("{base_url}/api/shutdown"))
        .header("x-paperlessllm-action", "shutdown")
        .header(header::ORIGIN.as_str(), &base_url)
        .send()
        .await
        .expect("receive shutdown response");
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let body = response
        .json::<Value>()
        .await
        .expect("read complete shutdown response body");
    assert_eq!(body, serde_json::json!({ "ok": true }));

    let exit = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(status) = process.child.try_wait().expect("inspect child") {
                break status;
            }
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "PaperlessLLM did not exit after shutdown: {}",
            process_log(&log_path)
        )
    });
    assert!(
        exit.success(),
        "PaperlessLLM exited with {exit}: {}",
        process_log(&log_path)
    );
}

#[tokio::test]
async fn persisted_reverse_proxy_mode_protects_loopback_apis_with_session_and_csrf() {
    let directory = TempDir::new().expect("temporary directory");
    let database = Database::connect(&directory.path().join("paperless-llm.db"))
        .await
        .expect("database");
    AuthService::new(database.clone())
        .set_admin_password("a strong administrator password")
        .await
        .expect("password");
    let mut settings = AppSettings::default();
    settings.security.lan_enabled = true;
    settings.security.reverse_proxy_tls = true;
    database
        .set_setting("dashboard_settings", &settings)
        .await
        .expect("settings");
    database.pool().close().await;
    drop(database);

    let state = initialize(runtime(directory.path())).await.expect("state");
    assert!(state.requires_auth());
    let app = build_router(state);

    let health = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("health response");
    assert_eq!(health.status(), StatusCode::OK);

    let protected = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/bootstrap")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("protected response");
    assert_eq!(protected.status(), StatusCode::UNAUTHORIZED);

    let protected_logs = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/logs")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("protected logs response");
    assert_eq!(protected_logs.status(), StatusCode::UNAUTHORIZED);

    let unauthenticated_shutdown = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/shutdown")
                .header("x-paperlessllm-action", "shutdown")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("protected shutdown response");
    assert_eq!(unauthenticated_shutdown.status(), StatusCode::UNAUTHORIZED);

    let mut login_request = Request::builder()
        .method("POST")
        .uri("/api/auth/login")
        .header(header::HOST, "paperlessllm.example")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            r#"{"password":"a strong administrator password"}"#,
        ))
        .expect("login request");
    login_request.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:45123"
            .parse::<SocketAddr>()
            .expect("peer address"),
    ));
    let login = app
        .clone()
        .oneshot(login_request)
        .await
        .expect("login response");
    assert_eq!(login.status(), StatusCode::OK);
    let set_cookie = login
        .headers()
        .get(header::SET_COOKIE)
        .expect("session cookie")
        .to_str()
        .expect("cookie text")
        .to_owned();
    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("Secure"));
    assert!(set_cookie.contains("SameSite=Strict"));
    let cookie = set_cookie
        .split(';')
        .next()
        .expect("cookie pair")
        .to_owned();
    let body = login
        .into_body()
        .collect()
        .await
        .expect("login body")
        .to_bytes();
    let auth: Value = serde_json::from_slice(&body).expect("auth JSON");
    let csrf = auth["csrf_token"].as_str().expect("CSRF token");

    let bootstrap = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/bootstrap")
                .header(header::HOST, "paperlessllm.example")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("bootstrap response");
    assert_eq!(bootstrap.status(), StatusCode::OK);

    let missing_csrf = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/logout")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("CSRF response");
    assert_eq!(missing_csrf.status(), StatusCode::FORBIDDEN);

    let shutdown_missing_csrf = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/shutdown")
                .header(header::HOST, "paperlessllm.example")
                .header(header::COOKIE, &cookie)
                .header("x-paperlessllm-action", "shutdown")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("CSRF-protected shutdown response");
    assert_eq!(shutdown_missing_csrf.status(), StatusCode::FORBIDDEN);

    let shutdown = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/shutdown")
                .header(header::HOST, "paperlessllm.example")
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", csrf)
                .header("x-paperlessllm-action", "shutdown")
                .header(header::ORIGIN, "https://paperlessllm.example")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("shutdown response");
    assert_eq!(shutdown.status(), StatusCode::ACCEPTED);

    let logout = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/logout")
                .header(header::COOKIE, cookie)
                .header("x-csrf-token", csrf)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("logout response");
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn authenticated_status_polling_is_read_only_and_private_no_store() {
    let directory = TempDir::new().expect("temporary directory");
    let database = Database::connect(&directory.path().join("paperless-llm.db"))
        .await
        .expect("database");
    let auth = AuthService::new(database.clone());
    auth.set_admin_password("a strong administrator password")
        .await
        .expect("password");
    let session = auth
        .login(
            "127.0.0.1".parse().expect("loopback address"),
            "a strong administrator password",
            24,
        )
        .await
        .expect("session");
    sqlx::query("UPDATE sessions SET last_seen_at='2000-01-01T00:00:00Z'")
        .execute(database.pool())
        .await
        .expect("stable last-seen marker");
    let mut settings = AppSettings::default();
    settings.security.lan_enabled = true;
    settings.security.reverse_proxy_tls = true;
    database
        .set_setting("dashboard_settings", &settings)
        .await
        .expect("settings");
    database.pool().close().await;
    drop(auth);
    drop(database);

    let state = initialize(runtime(directory.path())).await.expect("state");
    let database = state.database.clone();
    let app = build_router(state);
    for _ in 0..3 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/status")
                    .header(header::HOST, "localhost:8765")
                    .header(
                        header::COOKIE,
                        format!("paperlessllm_session={}", session.token),
                    )
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("status response");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CACHE_CONTROL)
                .and_then(|value| value.to_str().ok()),
            Some("no-store, private")
        );
    }

    let last_seen: String = sqlx::query_scalar("SELECT last_seen_at FROM sessions")
        .fetch_one(database.pool())
        .await
        .expect("session last seen");
    assert_eq!(last_seen, "2000-01-01T00:00:00Z");
}
