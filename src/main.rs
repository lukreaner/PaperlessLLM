use std::{ffi::OsString, net::IpAddr, path::PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use directories::BaseDirs;
use paperless_llm::RuntimeOptions;
use tracing_subscriber::EnvFilter;

const DESKTOP_LOG_DIRECTIVES: &str = "paperless_llm=info";

#[derive(Debug, Parser)]
#[command(
    name = "paperless-llm",
    version,
    about = "AI metadata sidecar for Paperless-ngx v3"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Start or reuse the background service, open the dashboard, and exit.
    Launch(LaunchArgs),
    /// Run the embedded API, worker, and web dashboard.
    Serve(ServeArgs),
}

#[derive(Debug, Args)]
struct RuntimeArgs {
    /// Directory for the SQLite database, encrypted secrets, and lock file.
    #[arg(long, env = "PAPERLESSLLM_DATA_DIR")]
    data_dir: Option<PathBuf>,
    /// Address to listen on. Non-loopback addresses require LAN authentication.
    #[arg(long, default_value = "127.0.0.1", env = "PAPERLESSLLM_BIND")]
    bind: IpAddr,
    /// TCP port for the dashboard and API.
    #[arg(long, default_value_t = 8765, env = "PAPERLESSLLM_PORT")]
    port: u16,
}

impl RuntimeArgs {
    fn options(self, open_browser: bool) -> Result<RuntimeOptions> {
        Ok(RuntimeOptions {
            data_dir: self.data_dir.unwrap_or(default_data_dir()?),
            bind: self.bind,
            port: self.port,
            open_browser,
        })
    }
}

#[derive(Debug, Args)]
struct ServeArgs {
    #[command(flatten)]
    runtime: RuntimeArgs,
    /// Open the dashboard after startup.
    #[arg(long, conflicts_with = "no_open")]
    open_browser: bool,
    /// Do not open a browser (useful in service files).
    #[arg(long, conflicts_with = "open_browser")]
    no_open: bool,
    /// Write bounded diagnostics for a child started by the desktop launcher.
    #[arg(long, hide = true)]
    desktop_log: bool,
}

#[derive(Debug, Args)]
struct LaunchArgs {
    #[command(flatten)]
    runtime: RuntimeArgs,
    /// Start or reuse the service without opening a browser.
    #[arg(long)]
    no_open: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let command = Cli::parse_from(filtered_process_arguments())
        .command
        .expect("launch subcommand is inserted when none is supplied");
    match command {
        Command::Serve(args) => {
            let desktop_log = args.desktop_log;
            let options = args.runtime.options(args.open_browser && !args.no_open)?;
            if desktop_log {
                init_desktop_tracing(&options.data_dir)?;
            } else {
                init_console_tracing()?;
            }
            let result = paperless_llm::run(options).await;
            if let Err(error) = &result {
                tracing::error!(
                    event = "service.fatal",
                    target = "service",
                    status = "error",
                    error_code = paperless_llm::diagnostics::safe_error_code(error),
                    "service stopped with an error"
                );
            } else {
                tracing::info!(
                    event = "service.stopped",
                    target = "service",
                    status = "completed",
                    "service stopped"
                );
            }
            result.context("PaperlessLLM stopped with an error")?;
        }
        Command::Launch(args) => {
            init_console_tracing()?;
            let show_error = !args.no_open;
            let result =
                paperless_llm::launcher::launch(args.runtime.options(!args.no_open)?).await;
            if let Err(error) = &result
                && show_error
            {
                paperless_llm::launcher::show_launch_error(&error.to_string()).await;
            }
            result.context("PaperlessLLM could not launch")?;
        }
    }
    Ok(())
}

fn console_log_filter() -> EnvFilter {
    EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("paperless_llm=info,tower_http=info"))
}

fn desktop_log_filter() -> EnvFilter {
    // Desktop logs are persisted and may later be inspected through the safe
    // dashboard projection. Never honor process environment directives here:
    // dependency debug traces can contain headers or response bodies before
    // that projection has an opportunity to reject them.
    EnvFilter::new(DESKTOP_LOG_DIRECTIVES)
}

fn init_console_tracing() -> Result<()> {
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_env_filter(console_log_filter())
        .with_target(false)
        .compact()
        .try_init()
        .map_err(|error| anyhow::anyhow!("could not initialize console logging: {error}"))
}

fn init_desktop_tracing(data_dir: &std::path::Path) -> Result<()> {
    let writer = paperless_llm::diagnostics::desktop_log_writer(data_dir)
        .context("could not initialize the private desktop log")?;
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_env_filter(desktop_log_filter())
        .with_target(false)
        .compact()
        .with_writer(writer)
        .try_init()
        .map_err(|error| anyhow::anyhow!("could not initialize desktop logging: {error}"))
}

fn filtered_process_arguments() -> Vec<OsString> {
    prepare_cli_arguments(std::env::args_os())
}

fn prepare_cli_arguments(arguments: impl IntoIterator<Item = OsString>) -> Vec<OsString> {
    let mut arguments = arguments
        .into_iter()
        .enumerate()
        .filter_map(|(index, argument)| {
            let finder_process_serial = index > 0
                && argument
                    .to_str()
                    .is_some_and(|value| value.starts_with("-psn_"));
            (!finder_process_serial).then_some(argument)
        })
        .collect::<Vec<_>>();
    if arguments.len() == 1 {
        arguments.push(OsString::from("launch"));
    }
    arguments
}

fn default_data_dir() -> Result<PathBuf> {
    let base = BaseDirs::new().context("could not determine a user data directory")?;
    #[cfg(target_os = "macos")]
    let path = base.data_dir().join("PaperlessLLM");
    #[cfg(not(target_os = "macos"))]
    let path = base.data_local_dir().join("paperless-llm");
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_arguments_select_desktop_launch_mode() {
        let arguments = prepare_cli_arguments([OsString::from("paperless-llm")]);
        let command = Cli::try_parse_from(arguments).unwrap().command.unwrap();
        let Command::Launch(arguments) = command else {
            panic!("no arguments must select launch mode");
        };
        assert!(!arguments.no_open);
        assert_eq!(arguments.runtime.port, 8765);
    }

    #[test]
    fn finder_process_serial_argument_is_ignored() {
        let arguments = prepare_cli_arguments(
            ["paperless-llm", "-psn_0_12345"]
                .into_iter()
                .map(OsString::from),
        );
        let parsed = Cli::try_parse_from(arguments).unwrap();
        let Command::Launch(arguments) = parsed.command.unwrap() else {
            panic!("Finder launch should select launch mode after filtering");
        };
        assert!(!arguments.no_open);
    }

    #[test]
    fn desktop_log_filter_is_fixed_to_application_info_events() {
        assert_eq!(DESKTOP_LOG_DIRECTIVES, "paperless_llm=info");
        assert_eq!(desktop_log_filter().to_string(), DESKTOP_LOG_DIRECTIVES);
    }
}
