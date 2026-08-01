use std::{net::SocketAddr, path::Path};

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
use tower::ServiceExt;

fn runtime(path: &Path) -> RuntimeOptions {
    RuntimeOptions {
        data_dir: path.to_owned(),
        bind: "127.0.0.1".parse().expect("loopback"),
        port: 0,
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

    let mut login_request = Request::builder()
        .method("POST")
        .uri("/api/auth/login")
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
