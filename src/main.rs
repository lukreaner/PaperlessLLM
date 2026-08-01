use std::{net::IpAddr, path::PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use directories::BaseDirs;
use paperless_llm::RuntimeOptions;
use tracing_subscriber::EnvFilter;

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
    /// Run the embedded API, worker, and web dashboard.
    Serve(ServeArgs),
}

#[derive(Debug, Args)]
struct ServeArgs {
    /// Directory for the SQLite database, encrypted secrets, and lock file.
    #[arg(long, env = "PAPERLESSLLM_DATA_DIR")]
    data_dir: Option<PathBuf>,
    /// Address to listen on. Non-loopback addresses require LAN authentication.
    #[arg(long, default_value = "127.0.0.1", env = "PAPERLESSLLM_BIND")]
    bind: IpAddr,
    /// TCP port for the dashboard and API.
    #[arg(long, default_value_t = 8765, env = "PAPERLESSLLM_PORT")]
    port: u16,
    /// Open the dashboard after startup.
    #[arg(long, conflicts_with = "no_open")]
    open_browser: bool,
    /// Do not open a browser (useful in service files).
    #[arg(long, conflicts_with = "open_browser")]
    no_open: bool,
}

impl Default for ServeArgs {
    fn default() -> Self {
        Self {
            data_dir: None,
            bind: IpAddr::from([127, 0, 0, 1]),
            port: 8765,
            open_browser: false,
            no_open: false,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("paperless_llm=info,tower_http=info")),
        )
        .with_target(false)
        .compact()
        .init();

    let args = match Cli::parse().command {
        Some(Command::Serve(args)) => args,
        None => ServeArgs::default(),
    };
    let data_dir = args.data_dir.unwrap_or(default_data_dir()?);
    paperless_llm::run(RuntimeOptions {
        data_dir,
        bind: args.bind,
        port: args.port,
        open_browser: args.open_browser && !args.no_open,
    })
    .await
    .context("PaperlessLLM stopped with an error")?;
    Ok(())
}

fn default_data_dir() -> Result<PathBuf> {
    let base = BaseDirs::new().context("could not determine a user data directory")?;
    #[cfg(target_os = "macos")]
    let path = base.data_dir().join("PaperlessLLM");
    #[cfg(not(target_os = "macos"))]
    let path = base.data_local_dir().join("paperless-llm");
    Ok(path)
}
