use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};

use directories::BaseDirs;
use reqwest::Client;
use serde_json::Value;
use tokio::time::sleep;

use crate::{
    AppError, AppResult, RuntimeOptions,
    app::{ensure_private_data_dir, instance_fingerprint, open_browser_now},
    diagnostics::validate_desktop_log_paths,
};

const STARTUP_ATTEMPTS: usize = 200;
const STARTUP_POLL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceHealth {
    Matching,
    DifferentInstance,
    Unavailable,
}

/// Start the background service used by the desktop launchers, or reuse the
/// service already listening at the configured address. The launcher itself
/// exits after opening the dashboard so a later double-click can reopen it.
pub async fn launch(options: RuntimeOptions) -> AppResult<()> {
    let executable = std::env::current_exe()?;
    launch_with_service_executable(options, &executable).await
}

/// Launch through an explicit service executable. The Windows GUI-subsystem
/// launcher is a separate binary, so it uses this entry point to start the
/// sibling console/service executable without recursively starting itself.
pub async fn launch_with_service_executable(
    mut options: RuntimeOptions,
    service_executable: &Path,
) -> AppResult<()> {
    if options.port == 0 {
        return Err(AppError::Validation(
            "desktop launch mode requires a fixed, non-zero port".to_owned(),
        ));
    }
    ensure_private_data_dir(&options.data_dir).await?;
    options.data_dir = std::fs::canonicalize(&options.data_dir)?;
    let instance_id = instance_fingerprint(&options.data_dir)?;
    let dashboard_url = options.browser_url();
    let client = health_client()?;

    // Give a service that is already starting a brief chance to become ready
    // before creating a competing process.
    for _ in 0..5 {
        match service_health(&client, &dashboard_url, &instance_id).await {
            ServiceHealth::Matching => {
                return finish_launch(&options, &dashboard_url).await;
            }
            ServiceHealth::DifferentInstance => {
                return Err(instance_conflict(&dashboard_url));
            }
            ServiceHealth::Unavailable => {}
        }
        sleep(STARTUP_POLL).await;
    }

    let log_path = options.data_dir.join("paperless-llm.log");
    validate_desktop_log_paths(&options.data_dir)?;
    let mut child = spawn_service(&options, service_executable)?;
    for _ in 0..STARTUP_ATTEMPTS {
        match service_health(&client, &dashboard_url, &instance_id).await {
            ServiceHealth::Matching => {
                return finish_launch(&options, &dashboard_url).await;
            }
            ServiceHealth::DifferentInstance => {
                stop_child(&mut child);
                return Err(instance_conflict(&dashboard_url));
            }
            ServiceHealth::Unavailable => {}
        }
        if let Some(status) = child.try_wait()? {
            // A simultaneous launcher may have won the data-directory lock.
            // Check once more before reporting the losing child as a failure.
            for _ in 0..5 {
                match service_health(&client, &dashboard_url, &instance_id).await {
                    ServiceHealth::Matching => {
                        return finish_launch(&options, &dashboard_url).await;
                    }
                    ServiceHealth::DifferentInstance => {
                        return Err(instance_conflict(&dashboard_url));
                    }
                    ServiceHealth::Unavailable => {}
                }
                sleep(STARTUP_POLL).await;
            }
            return Err(AppError::Internal(format!(
                "the background service exited with {status}; details are in {}",
                log_path.display()
            )));
        }
        sleep(STARTUP_POLL).await;
    }

    match service_health(&client, &dashboard_url, &instance_id).await {
        ServiceHealth::Matching => return finish_launch(&options, &dashboard_url).await,
        ServiceHealth::DifferentInstance => {
            stop_child(&mut child);
            return Err(instance_conflict(&dashboard_url));
        }
        ServiceHealth::Unavailable => stop_child(&mut child),
    }

    Err(AppError::Internal(format!(
        "the background service did not become ready at {dashboard_url}; details are in {}",
        log_path.display()
    )))
}

fn stop_child(child: &mut Child) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    if child.kill().is_ok() {
        let _ = child.wait();
    }
}

fn instance_conflict(dashboard_url: &str) -> AppError {
    AppError::Conflict(format!(
        "another PaperlessLLM data directory is already served at {dashboard_url}; stop that process or choose another --port"
    ))
}

async fn finish_launch(options: &RuntimeOptions, dashboard_url: &str) -> AppResult<()> {
    if options.open_browser {
        open_browser_now(dashboard_url).await?;
    }
    Ok(())
}

fn health_client() -> AppResult<Client> {
    Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_millis(500))
        .timeout(Duration::from_millis(750))
        .build()
        .map_err(|error| AppError::Internal(error.to_string()))
}

async fn service_health(
    client: &Client,
    dashboard_url: &str,
    expected_instance_id: &str,
) -> ServiceHealth {
    let url = format!("{}/healthz", dashboard_url.trim_end_matches('/'));
    let Ok(response) = client.get(url).send().await else {
        return ServiceHealth::Unavailable;
    };
    if !response.status().is_success() {
        return ServiceHealth::Unavailable;
    }
    let Ok(body) = response.json::<Value>().await else {
        return ServiceHealth::Unavailable;
    };
    if body.get("service").and_then(Value::as_str) != Some("paperless-llm")
        || body.get("status").and_then(Value::as_str) != Some("ok")
        || body.get("version").and_then(Value::as_str).is_none()
    {
        return ServiceHealth::Unavailable;
    }
    if body.get("instance_id").and_then(Value::as_str) == Some(expected_instance_id) {
        ServiceHealth::Matching
    } else {
        ServiceHealth::DifferentInstance
    }
}

fn spawn_service(options: &RuntimeOptions, service_executable: &Path) -> AppResult<Child> {
    let mut command = Command::new(service_executable);
    command
        .arg("serve")
        .arg("--data-dir")
        .arg(&options.data_dir)
        .arg("--bind")
        .arg(options.bind.to_string())
        .arg("--port")
        .arg(options.port.to_string())
        .arg("--no-open")
        .arg("--desktop-log")
        .current_dir(&options.data_dir)
        .stdin(Stdio::null())
        // The child owns its bounded diagnostic writer. Inheriting a raw log
        // descriptor here would keep writing to an old inode after rotation.
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW};

        // Keep the long-running console/service executable independent from
        // the short-lived GUI launcher and prevent a console-window flash.
        command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }
    command.spawn().map_err(AppError::from)
}

pub fn default_data_dir() -> AppResult<PathBuf> {
    let base = BaseDirs::new().ok_or_else(|| {
        AppError::Internal("could not determine a user data directory".to_owned())
    })?;
    #[cfg(target_os = "macos")]
    let path = base.data_dir().join("PaperlessLLM");
    #[cfg(not(target_os = "macos"))]
    let path = base.data_local_dir().join("paperless-llm");
    Ok(path)
}

/// Show a visible startup error when launch mode was entered from Finder or a
/// desktop menu where stderr is not visible. Failure to show the notification
/// is deliberately ignored; the same error is still returned to the CLI.
pub async fn show_launch_error(message: &str) {
    let message = message.chars().take(1_000).collect::<String>();
    #[cfg(target_os = "macos")]
    {
        let escaped = message.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!(
            "display alert \"PaperlessLLM could not start\" message \"{escaped}\" as critical"
        );
        let _ = tokio::process::Command::new("/usr/bin/osascript")
            .args(["-e", &script])
            .status()
            .await;
    }
    #[cfg(target_os = "linux")]
    {
        let zenity_succeeded = tokio::process::Command::new("zenity")
            .args([
                "--error",
                "--title=PaperlessLLM could not start",
                &format!("--text={message}"),
            ])
            .status()
            .await
            .is_ok_and(|status| status.success());
        if !zenity_succeeded {
            let _ = tokio::process::Command::new("notify-send")
                .args(["PaperlessLLM could not start", &message])
                .status()
                .await;
        }
    }
    #[cfg(target_os = "windows")]
    {
        use std::{os::windows::ffi::OsStrExt, ptr};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MB_TASKMODAL, MessageBoxW,
        };

        let message = std::ffi::OsStr::new(&message.replace('\0', "�"))
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let caption = std::ffi::OsStr::new("PaperlessLLM could not start")
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        // SAFETY: both buffers are valid, NUL-terminated UTF-16 and remain
        // alive until the modal call returns.
        unsafe {
            MessageBoxW(
                ptr::null_mut(),
                message.as_ptr(),
                caption.as_ptr(),
                MB_OK | MB_ICONERROR | MB_SETFOREGROUND | MB_TASKMODAL,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use axum::{Json, Router, routing::get};
    use serde_json::json;
    use tempfile::TempDir;
    use tokio::net::TcpListener;

    use super::*;

    #[tokio::test]
    async fn launch_reuses_healthy_service_without_spawning_another_process() {
        let directory = TempDir::new().unwrap();
        let instance_id = instance_fingerprint(directory.path()).unwrap();
        let (port, server) = spawn_health_server(instance_id).await;

        launch(RuntimeOptions {
            data_dir: directory.path().to_path_buf(),
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port,
            open_browser: false,
        })
        .await
        .unwrap();

        assert!(!directory.path().join("paperless-llm.log").exists());
        server.abort();
    }

    #[tokio::test]
    async fn launch_rejects_a_different_data_directory_on_the_same_port() {
        let directory = TempDir::new().unwrap();
        let other_directory = TempDir::new().unwrap();
        let other_instance_id = instance_fingerprint(other_directory.path()).unwrap();
        let (port, server) = spawn_health_server(other_instance_id).await;

        let error = launch(RuntimeOptions {
            data_dir: directory.path().to_path_buf(),
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port,
            open_browser: false,
        })
        .await
        .expect_err("another data directory on this port must not be reused");

        assert!(
            error
                .to_string()
                .contains("another PaperlessLLM data directory")
        );
        assert!(!directory.path().join("paperless-llm.log").exists());
        server.abort();
    }

    async fn spawn_health_server(instance_id: String) -> (u16, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/healthz",
                    get(move || {
                        let instance_id = instance_id.clone();
                        async move {
                            Json(json!({
                                "instance_id": instance_id,
                                "service": "paperless-llm",
                                "status": "ok",
                                "version": "test"
                            }))
                        }
                    }),
                ),
            )
            .await
            .unwrap();
        });
        (port, server)
    }

    #[tokio::test]
    async fn launch_rejects_an_ephemeral_port() {
        let directory = TempDir::new().unwrap();
        let error = launch(RuntimeOptions {
            data_dir: directory.path().to_path_buf(),
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 0,
            open_browser: false,
        })
        .await
        .expect_err("port zero must fail");
        assert!(error.to_string().contains("fixed, non-zero port"));
    }
}
