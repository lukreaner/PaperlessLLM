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
pub mod provider_factory;
pub mod providers;
pub mod secrets;
pub mod static_files;
pub mod worker;

pub use app::{AppState, RuntimeOptions, build_router, run};
pub use error::{AppError, AppResult};
