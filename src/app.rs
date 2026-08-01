use std::{
    fs::{File, OpenOptions},
    io::Write,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU16, Ordering},
    },
};

use axum::{
    Router,
    body::Body,
    http::{Request, header},
};
use fs2::FileExt;
use sha2::{Digest, Sha256};
use tokio::{net::TcpListener, sync::watch};
use tower_http::{
    catch_panic::CatchPanicLayer,
    compression::CompressionLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    set_header::SetResponseHeaderLayer,
    trace::TraceLayer,
};
use tracing::{info, info_span, warn};
use uuid::Uuid;

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
        let host = if self.bind.is_unspecified() && self.bind.is_ipv4() {
            "127.0.0.1".to_owned()
        } else if self.bind.is_unspecified() {
            "[::1]".to_owned()
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
    pub instance_id: Arc<str>,
    pub(crate) shutdown: ShutdownHandle,
    listening_port: Arc<AtomicU16>,
    lan_auth_required: Arc<AtomicBool>,
    // Holding this descriptor keeps the single-process data-directory lock alive.
    _data_lock: Arc<File>,
}

#[derive(Clone)]
pub(crate) struct ShutdownHandle {
    requested: watch::Sender<bool>,
}

impl ShutdownHandle {
    fn new() -> Self {
        let (requested, _) = watch::channel(false);
        Self { requested }
    }

    pub(crate) fn request(&self) {
        self.requested.send_replace(true);
    }

    fn subscribe(&self) -> watch::Receiver<bool> {
        self.requested.subscribe()
    }
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

    pub(crate) fn listening_port(&self) -> u16 {
        self.listening_port.load(Ordering::Acquire)
    }

    fn set_listening_port(&self, port: u16) {
        self.listening_port.store(port, Ordering::Release);
    }
}

pub async fn initialize(options: RuntimeOptions) -> AppResult<AppState> {
    ensure_private_data_dir(&options.data_dir).await?;
    let data_lock = acquire_data_lock(&options.data_dir)?;
    let instance_id = instance_fingerprint(&options.data_dir)?;
    let database = Database::connect(&options.data_dir.join("paperless-llm.db")).await?;
    database.cleanup_expired_sessions().await?;
    let secrets = SecretStore::new(database.clone(), &options.data_dir).await?;
    let auth = AuthService::new(database.clone());
    let budget = BudgetService::new(database.clone());
    let worker_paused = database
        .get_setting::<bool>(worker::PAUSED_SETTING_KEY)
        .await?
        .unwrap_or(false);
    let (worker_handle, receiver) = worker::channel(worker_paused);
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

    let configured_port = options.port;
    let state = AppState {
        database,
        secrets,
        auth,
        budget,
        runtime: Arc::new(options),
        worker: worker_handle,
        instance_id: Arc::from(instance_id),
        shutdown: ShutdownHandle::new(),
        listening_port: Arc::new(AtomicU16::new(configured_port)),
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
    worker::recover_interrupted_jobs(&state).await?;

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
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &Request<Body>| {
                info_span!(
                    "http_request",
                    method = %request.method(),
                    path = %request_log_path(request)
                )
            }),
        )
        .layer(CatchPanicLayer::new())
        .with_state(state)
}

pub async fn run(options: RuntimeOptions) -> AppResult<()> {
    let state = initialize(options).await?;
    let address = state.runtime.socket_addr();
    let listener = TcpListener::bind(address).await?;
    let local_address = listener.local_addr()?;
    state.set_listening_port(local_address.port());
    let shutdown = state.shutdown.subscribe();
    let browser_url = if state.runtime.port == 0 {
        let mut options = (*state.runtime).clone();
        options.port = local_address.port();
        options.browser_url()
    } else {
        state.runtime.browser_url()
    };
    info!(%local_address, "PaperlessLLM is ready");
    if state.runtime.open_browser {
        tokio::spawn(async move {
            if let Err(error) = open_browser_now(&browser_url).await {
                warn!(
                    error = %crate::diagnostics::safe_error_summary(&error),
                    "could not open the dashboard in a browser"
                );
            }
        });
    }

    axum::serve(
        listener,
        build_router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal(shutdown))
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?;
    Ok(())
}

fn request_log_path<B>(request: &Request<B>) -> &'static str {
    crate::diagnostics::safe_request_path(request.uri().path()).unwrap_or("/dashboard")
}

pub(crate) async fn ensure_private_data_dir(path: &Path) -> AppResult<()> {
    tokio::fs::create_dir_all(path).await?;
    crate::private_fs::harden_private_directory(path)?;
    Ok(())
}

/// Return an opaque identifier tied to both this data directory and a random
/// seed stored inside it. The launcher compares it with `/healthz` so it never
/// mistakes another PaperlessLLM data directory on the same port for its own.
pub(crate) fn instance_fingerprint(data_dir: &Path) -> AppResult<String> {
    let seed = load_or_create_instance_seed(data_dir)?;
    let canonical = std::fs::canonicalize(data_dir)?;
    let mut digest = Sha256::new();
    digest.update(b"paperless-llm-instance-v1\0");
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        digest.update(canonical.as_os_str().as_bytes());
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        for unit in canonical.as_os_str().encode_wide() {
            digest.update(unit.to_le_bytes());
        }
    }
    #[cfg(not(any(unix, target_os = "windows")))]
    digest.update(canonical.to_string_lossy().as_bytes());
    digest.update(b"\0");
    digest.update(seed.as_bytes());
    Ok(hex::encode(digest.finalize()))
}

fn load_or_create_instance_seed(data_dir: &Path) -> AppResult<Uuid> {
    const ATTEMPTS: usize = 20;
    const RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(5);

    let path = data_dir.join("paperless-llm.instance");
    let mut invalid_seed = None;
    for _ in 0..ATTEMPTS {
        match std::fs::read_to_string(&path) {
            Ok(contents) => {
                set_private_file_permissions(&path)?;
                match Uuid::parse_str(contents.trim()) {
                    Ok(seed) => {
                        return Ok(seed);
                    }
                    Err(error) => {
                        // A concurrent launcher may have created the file but not
                        // completed its small write yet.
                        invalid_seed = Some(error.to_string());
                        std::thread::sleep(RETRY_DELAY);
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if let Some(seed) = create_instance_seed_atomically(data_dir, &path)? {
                    return Ok(seed);
                }
                std::thread::sleep(RETRY_DELAY);
            }
            Err(error) => return Err(error.into()),
        }
    }

    Err(AppError::Validation(format!(
        "{} does not contain a valid PaperlessLLM instance identifier{}",
        path.display(),
        invalid_seed
            .map(|error| format!(": {error}"))
            .unwrap_or_default()
    )))
}

fn create_instance_seed_atomically(data_dir: &Path, final_path: &Path) -> AppResult<Option<Uuid>> {
    let seed = Uuid::new_v4();
    let temporary_path = data_dir.join(format!(".paperless-llm.instance.{seed}.tmp"));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    crate::private_fs::configure_private_open(&mut options, 0o600);
    let mut file = options.open(&temporary_path)?;
    writeln!(file, "{seed}")?;
    file.sync_all()?;
    drop(file);

    // A hard link is an atomic create-without-overwrite operation on the same
    // filesystem. Concurrent launchers can race here without ever replacing a
    // complete identifier selected by another process.
    let link_result = std::fs::hard_link(&temporary_path, final_path);
    let _ = std::fs::remove_file(&temporary_path);
    match link_result {
        Ok(()) => {
            set_private_file_permissions(final_path)?;
            Ok(Some(seed))
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn set_private_file_permissions(path: &Path) -> AppResult<()> {
    crate::private_fs::harden_private_file(path)?;
    Ok(())
}

fn acquire_data_lock(data_dir: &Path) -> AppResult<File> {
    let path = data_dir.join("paperless-llm.lock");
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    crate::private_fs::configure_private_open(&mut options, 0o600);
    let file = options.open(&path)?;
    set_private_file_permissions(&path)?;
    file.try_lock_exclusive().map_err(|_| {
        AppError::Conflict(
            "another PaperlessLLM process is already using this data directory".to_owned(),
        )
    })?;
    Ok(file)
}

pub(crate) async fn open_browser_now(url: &str) -> AppResult<()> {
    #[cfg(target_os = "windows")]
    {
        open_browser_windows(url)
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        #[cfg(target_os = "macos")]
        let mut command = {
            let mut command = tokio::process::Command::new("open");
            command.arg(url);
            command
        };
        #[cfg(target_os = "linux")]
        let mut command = {
            let mut command = tokio::process::Command::new("xdg-open");
            command.arg(url);
            command
        };

        let mut child = command.spawn()?;
        let Ok(status) =
            tokio::time::timeout(std::time::Duration::from_secs(2), child.wait()).await
        else {
            // Some Linux browser launchers remain attached to a newly opened
            // browser. The dashboard has been handed off successfully, so do
            // not keep the desktop launcher alive until that browser closes.
            return Ok(());
        };
        let status = status?;
        if !status.success() {
            return Err(AppError::Internal(format!(
                "the browser launcher exited with {status}; open {url} manually"
            )));
        }
        Ok(())
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        Err(AppError::Internal(format!(
            "open this dashboard URL in a browser: {url}"
        )))
    }
}

#[cfg(target_os = "windows")]
fn open_browser_windows(url: &str) -> AppResult<()> {
    use std::{os::windows::ffi::OsStrExt, ptr};
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};

    let operation = std::ffi::OsStr::new("open")
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let url = std::ffi::OsStr::new(url)
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    // SAFETY: the operation and URL are NUL-terminated UTF-16 buffers. No
    // command interpreter is involved; Windows passes the URL directly to the
    // registered HTTP handler.
    let result = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            operation.as_ptr(),
            url.as_ptr(),
            ptr::null(),
            ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    let result_code = result as usize;
    if result_code <= 32 {
        return Err(AppError::Internal(format!(
            "Windows could not open the default browser (ShellExecute error {result_code})"
        )));
    }
    Ok(())
}

async fn shutdown_signal(mut dashboard_shutdown: watch::Receiver<bool>) {
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
    let dashboard = async move {
        if *dashboard_shutdown.borrow() {
            return;
        }
        while dashboard_shutdown.changed().await.is_ok() {
            if *dashboard_shutdown.borrow() {
                return;
            }
        }
    };

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
        () = dashboard => {},
    }
    info!("shutdown requested");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_url_uses_the_specific_bound_interface() {
        let directory = PathBuf::from("test");
        let ipv4 = RuntimeOptions {
            data_dir: directory.clone(),
            bind: "192.0.2.40".parse().unwrap(),
            port: 8765,
            open_browser: false,
        };
        let ipv6 = RuntimeOptions {
            data_dir: directory,
            bind: "2001:db8::40".parse().unwrap(),
            port: 8765,
            open_browser: false,
        };

        assert_eq!(ipv4.browser_url(), "http://192.0.2.40:8765");
        assert_eq!(ipv6.browser_url(), "http://[2001:db8::40]:8765");
    }

    #[test]
    fn request_trace_path_excludes_query_strings() {
        let request = Request::builder()
            .uri("/api/jobs?token=must-not-be-logged&search=private")
            .body(())
            .unwrap();
        assert_eq!(request_log_path(&request), "/api/jobs");
        assert!(!request_log_path(&request).contains("must-not-be-logged"));

        let dynamic = Request::builder()
            .uri("/api/jobs/private-document-title")
            .body(())
            .unwrap();
        assert_eq!(request_log_path(&dynamic), "/api/jobs/:id");

        let unknown = Request::builder()
            .uri("/private-dashboard-path")
            .body(())
            .unwrap();
        assert_eq!(request_log_path(&unknown), "/dashboard");
    }

    #[test]
    fn copied_instance_seed_is_still_bound_to_the_canonical_directory() {
        let first = tempfile::TempDir::new().unwrap();
        let second = tempfile::TempDir::new().unwrap();
        let seed = load_or_create_instance_seed(first.path()).unwrap();
        std::fs::write(
            second.path().join("paperless-llm.instance"),
            format!("{seed}\n"),
        )
        .unwrap();

        assert_ne!(
            instance_fingerprint(first.path()).unwrap(),
            instance_fingerprint(second.path()).unwrap()
        );
    }

    #[test]
    fn concurrent_instance_creation_selects_one_complete_seed() {
        let directory = tempfile::TempDir::new().unwrap();
        let path = Arc::new(directory.path().to_path_buf());
        let workers = (0..16)
            .map(|_| {
                let path = Arc::clone(&path);
                std::thread::spawn(move || instance_fingerprint(&path).unwrap())
            })
            .collect::<Vec<_>>();
        let fingerprints = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();

        assert!(
            fingerprints
                .iter()
                .all(|fingerprint| fingerprint == &fingerprints[0])
        );
        let seed = std::fs::read_to_string(path.join("paperless-llm.instance")).unwrap();
        assert!(Uuid::parse_str(seed.trim()).is_ok());
    }

    #[tokio::test]
    async fn dashboard_request_completes_the_graceful_shutdown_signal() {
        let shutdown = ShutdownHandle::new();
        let receiver = shutdown.subscribe();
        shutdown.request();

        tokio::time::timeout(std::time::Duration::from_secs(1), shutdown_signal(receiver))
            .await
            .expect("dashboard shutdown signal should complete");
    }
}
