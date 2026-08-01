use std::{
    fs::{File, OpenOptions},
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use axum::{Router, http::header};
use fs2::FileExt;
use tokio::net::TcpListener;
use tower_http::{
    catch_panic::CatchPanicLayer,
    compression::CompressionLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    set_header::SetResponseHeaderLayer,
    trace::TraceLayer,
};
use tracing::{info, warn};

use crate::{
    AppError, AppResult, api,
    auth::AuthService,
    budget::BudgetService,
    db::Database,
    secrets::SecretStore,
    worker::{self, WorkerHandle},
};

#[derive(Debug, Clone)]
pub struct RuntimeOptions {
    pub data_dir: PathBuf,
    pub bind: IpAddr,
    pub port: u16,
    pub open_browser: bool,
}

impl RuntimeOptions {
    pub fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.bind, self.port)
    }

    pub fn is_lan_mode(&self) -> bool {
        !self.bind.is_loopback()
    }

    pub fn browser_url(&self) -> String {
        let host = if self.bind.is_unspecified() || self.is_lan_mode() {
            "127.0.0.1".to_owned()
        } else if self.bind.is_ipv6() {
            format!("[{}]", self.bind)
        } else {
            self.bind.to_string()
        };
        format!("http://{host}:{}", self.port)
    }
}

#[derive(Clone)]
pub struct AppState {
    pub database: Database,
    pub secrets: SecretStore,
    pub auth: AuthService,
    pub budget: BudgetService,
    pub runtime: Arc<RuntimeOptions>,
    pub worker: WorkerHandle,
    lan_auth_required: Arc<AtomicBool>,
    // Holding this descriptor keeps the single-process data-directory lock alive.
    _data_lock: Arc<File>,
}

impl AppState {
    /// Authentication is required either when the process listens on a
    /// non-loopback address or when LAN/reverse-proxy access was enabled in
    /// Settings. The latter matters when TLS terminates at a local proxy and
    /// PaperlessLLM itself remains bound to 127.0.0.1.
    pub fn requires_auth(&self) -> bool {
        self.lan_auth_required.load(Ordering::Acquire)
    }

    pub fn set_configured_lan(&self, enabled: bool) {
        self.lan_auth_required
            .store(self.runtime.is_lan_mode() || enabled, Ordering::Release);
    }
}

pub async fn initialize(options: RuntimeOptions) -> AppResult<AppState> {
    ensure_private_data_dir(&options.data_dir).await?;
    let data_lock = acquire_data_lock(&options.data_dir)?;
    let database = Database::connect(&options.data_dir.join("paperless-llm.db")).await?;
    database.cleanup_expired_sessions().await?;
    let secrets = SecretStore::new(database.clone(), &options.data_dir).await?;
    let auth = AuthService::new(database.clone());
    let budget = BudgetService::new(database.clone());
    let (worker_handle, receiver) = worker::channel();
    let configured_lan = database
        .get_setting::<serde_json::Value>("dashboard_settings")
        .await?
        .and_then(|settings| {
            settings
                .pointer("/security/lan_enabled")
                .and_then(serde_json::Value::as_bool)
        })
        .unwrap_or(false);
    let lan_auth_required = options.is_lan_mode() || configured_lan;

    let state = AppState {
        database,
        secrets,
        auth,
        budget,
        runtime: Arc::new(options),
        worker: worker_handle,
        lan_auth_required: Arc::new(AtomicBool::new(lan_auth_required)),
        _data_lock: Arc::new(data_lock),
    };

    // Reconcile before the worker or HTTP server can dispatch a new request;
    // every reservation surviving from a previous process is ambiguous.
    let recovered_reservations = state.budget.reconcile_stale_reservations().await?;
    if recovered_reservations > 0 {
        warn!(
            recovered_reservations,
            "charged interrupted AI reservations during startup recovery"
        );
    }

    if state.requires_auth() && !state.auth.has_admin_password().await? {
        return Err(AppError::Validation(
            "LAN access requires an administrator password. Start on localhost, set the password in Settings, then enable LAN access or restart with a non-loopback --bind address".to_owned(),
        ));
    }

    tokio::spawn(worker::run(state.clone(), receiver));
    Ok(state)
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .merge(api::router(state.clone()))
        .fallback(crate::static_files::serve)
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            header::HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::REFERRER_POLICY,
            header::HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            header::HeaderValue::from_static(
                "default-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; connect-src 'self'",
            ),
        ))
        .layer(CompressionLayer::new())
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::new(
            header::HeaderName::from_static("x-request-id"),
            MakeRequestUuid,
        ))
        .layer(TraceLayer::new_for_http())
        .layer(CatchPanicLayer::new())
        .with_state(state)
}

pub async fn run(options: RuntimeOptions) -> AppResult<()> {
    let state = initialize(options).await?;
    let address = state.runtime.socket_addr();
    let listener = TcpListener::bind(address).await?;
    let local_address = listener.local_addr()?;
    let browser_url = if state.runtime.port == 0 {
        let mut options = (*state.runtime).clone();
        options.port = local_address.port();
        options.browser_url()
    } else {
        state.runtime.browser_url()
    };
    info!(%local_address, data_dir = %state.runtime.data_dir.display(), "PaperlessLLM is ready");
    if state.runtime.open_browser {
        open_browser(browser_url.clone());
    }

    axum::serve(
        listener,
        build_router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?;
    Ok(())
}

async fn ensure_private_data_dir(path: &Path) -> AppResult<()> {
    tokio::fs::create_dir_all(path).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).await?;
    }
    Ok(())
}

fn acquire_data_lock(data_dir: &Path) -> AppResult<File> {
    let path = data_dir.join("paperless-llm.lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    file.try_lock_exclusive().map_err(|_| {
        AppError::Conflict(
            "another PaperlessLLM process is already using this data directory".to_owned(),
        )
    })?;
    Ok(file)
}

fn open_browser(url: String) {
    tokio::spawn(async move {
        #[cfg(target_os = "macos")]
        let command = ("open", vec![url.as_str()]);
        #[cfg(target_os = "linux")]
        let command = ("xdg-open", vec![url.as_str()]);
        #[cfg(target_os = "windows")]
        let command = ("cmd", vec!["/C", "start", url.as_str()]);
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        let command = ("", Vec::<&str>::new());

        if command.0.is_empty() {
            return;
        }
        if let Err(error) = tokio::process::Command::new(command.0)
            .args(command.1)
            .status()
            .await
        {
            warn!(%error, "could not open the dashboard in a browser");
        }
    });
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    info!("shutdown requested");
}
