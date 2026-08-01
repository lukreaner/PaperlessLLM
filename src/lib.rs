pub mod api;
pub mod app;
pub mod auth;
pub mod budget;
pub mod db;
pub mod diagnostics;
pub mod error;
pub mod launcher;
pub mod models;
pub mod paperless;
mod private_fs;
pub mod provider_factory;
pub mod providers;
pub mod secrets;
pub mod static_files;
pub mod worker;

pub use app::{AppState, RuntimeOptions, build_router, run};
pub use error::{AppError, AppResult};

/// Opaque fingerprint of the executable inputs, target, profile, features,
/// and Rust toolchain. It is intentionally independent of packaging and code
/// signatures so the CLI and desktop launcher from one build can interoperate.
pub const BUILD_ID: &str = env!("PAPERLESS_LLM_BUILD_ID");
