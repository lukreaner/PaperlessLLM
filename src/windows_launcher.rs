#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
use std::{net::Ipv4Addr, path::PathBuf};

#[cfg(target_os = "windows")]
use paperless_llm::{RuntimeOptions, launcher};

/// Windows Explorer launches this GUI-subsystem executable without creating a
/// console window. The actual Axum service remains in the sibling console
/// binary so command-line and service-manager behavior stays conventional.
#[cfg(target_os = "windows")]
#[tokio::main]
async fn main() {
    if let Err(error) = launch_from_explorer().await {
        launcher::show_launch_error(&error.to_string()).await;
    }
}

#[cfg(target_os = "windows")]
async fn launch_from_explorer() -> paperless_llm::AppResult<()> {
    let gui_executable = std::env::current_exe()?;
    let service_executable = sibling_service_executable(&gui_executable)?;
    if !service_executable.is_file() {
        return Err(paperless_llm::AppError::Validation(
            "paperless-llm.exe is missing next to PaperlessLLM.exe; reinstall the complete Windows archive"
                .to_owned(),
        ));
    }
    launcher::launch_with_service_executable(
        RuntimeOptions {
            data_dir: launcher::default_data_dir()?,
            bind: Ipv4Addr::LOCALHOST.into(),
            port: 8765,
            open_browser: true,
        },
        &service_executable,
    )
    .await
}

#[cfg(target_os = "windows")]
fn sibling_service_executable(
    gui_executable: &std::path::Path,
) -> paperless_llm::AppResult<PathBuf> {
    let directory = gui_executable.parent().ok_or_else(|| {
        paperless_llm::AppError::Internal(
            "could not determine the PaperlessLLM installation directory".to_owned(),
        )
    })?;
    Ok(directory.join("paperless-llm.exe"))
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("PaperlessLLM is the Windows desktop launcher; use paperless-llm on this platform");
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    #[test]
    fn gui_launcher_resolves_the_sibling_console_service() {
        let gui = PathBuf::from(r"C:\Program Files\PaperlessLLM\PaperlessLLM.exe");
        assert_eq!(
            sibling_service_executable(&gui).unwrap(),
            PathBuf::from(r"C:\Program Files\PaperlessLLM\paperless-llm.exe")
        );
    }
}
