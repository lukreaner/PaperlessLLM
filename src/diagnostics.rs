use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex, MutexGuard},
};

use chrono::{DateTime, Utc};
use fs2::FileExt;
use regex::Regex;
use serde::Serialize;
use tracing_subscriber::fmt::MakeWriter;

use crate::{AppError, AppResult};

const MAX_LOG_BYTES: u64 = 128 * 1024;
const MAX_LOG_LINES: usize = 500;
const MAX_LINE_CHARS: usize = 4_096;
const MAX_DESKTOP_LOG_BYTES: u64 = 4 * 1024 * 1024;
const DESKTOP_LOG_FILE: &str = "paperless-llm.log";
const DESKTOP_LOG_PREVIOUS_FILE: &str = "paperless-llm.log.previous";
const DESKTOP_LOG_TEMP_FILE: &str = ".paperless-llm.log.new";
const DESKTOP_LOG_LOCK_FILE: &str = "paperless-llm.log.lock";
pub(crate) const DESKTOP_LOG_HEADER: &str = "PAPERLESSLLM-DESKTOP-LOG-V1\n";
const OVERSIZED_ENTRY_MARKER: &[u8] = b"OVERSIZED-DIAGNOSTIC-EVENT-OMITTED\n";

#[derive(Clone)]
pub struct DesktopLogWriter {
    state: Arc<Mutex<RotatingLogState>>,
}

struct RotatingLogState {
    file: File,
    current_size: u64,
    max_bytes: u64,
    paths: DesktopLogPaths,
    // Holding this descriptor prevents two desktop children from rotating the
    // same fixed files before the application data-directory lock is acquired.
    _rotation_lock: File,
}

#[derive(Clone)]
struct DesktopLogPaths {
    current: PathBuf,
    previous: PathBuf,
    temporary: PathBuf,
    lock: PathBuf,
}

impl DesktopLogPaths {
    fn new(data_dir: &Path) -> Self {
        Self {
            current: data_dir.join(DESKTOP_LOG_FILE),
            previous: data_dir.join(DESKTOP_LOG_PREVIOUS_FILE),
            temporary: data_dir.join(DESKTOP_LOG_TEMP_FILE),
            lock: data_dir.join(DESKTOP_LOG_LOCK_FILE),
        }
    }
}

/// Build the private bounded writer used only by a desktop-launched child.
/// Foreground services continue to use their normal console subscriber.
pub fn desktop_log_writer(data_dir: &Path) -> AppResult<DesktopLogWriter> {
    desktop_log_writer_with_limit(data_dir, MAX_DESKTOP_LOG_BYTES)
}

/// Validate the fixed log targets in the launcher process so an unsafe path
/// produces a visible launch error even before the background child exists.
pub(crate) fn validate_desktop_log_paths(data_dir: &Path) -> AppResult<()> {
    validate_data_directory(data_dir)?;
    let paths = DesktopLogPaths::new(data_dir);
    for path in [
        &paths.current,
        &paths.previous,
        &paths.temporary,
        &paths.lock,
    ] {
        validate_optional_regular_file(path)?;
    }
    Ok(())
}

fn desktop_log_writer_with_limit(data_dir: &Path, max_bytes: u64) -> AppResult<DesktopLogWriter> {
    if max_bytes < DESKTOP_LOG_HEADER.len() as u64 {
        return Err(AppError::Validation(
            "desktop log size limit is too small".to_owned(),
        ));
    }
    validate_desktop_log_paths(data_dir)?;
    let paths = DesktopLogPaths::new(data_dir);
    let rotation_lock = open_private_file(&paths.lock, false, true)?;
    rotation_lock.try_lock_exclusive().map_err(|_| {
        AppError::Conflict("another desktop service owns the diagnostic log".to_owned())
    })?;
    rotation_lock.set_len(0)?;

    // Recheck after acquiring the inter-process lock. A competing process may
    // have completed a rotation while this process was waiting for the lock.
    for path in [&paths.current, &paths.previous, &paths.temporary] {
        validate_optional_regular_file(path)?;
    }
    // Older launchers redirected arbitrary process output into an unversioned
    // file. It may contain response bodies or other data that is outside the
    // current safe logging contract, so it is deleted rather than retained as
    // a previous generation. The same rule makes stale recovery files safe.
    discard_unversioned_generation(&paths.current)?;
    discard_unversioned_generation(&paths.previous)?;
    discard_unversioned_generation(&paths.temporary)?;
    let recovered = recover_interrupted_rotation(&paths)?;
    bound_existing_generation(&paths.current, max_bytes)?;
    bound_existing_generation(&paths.previous, max_bytes)?;
    let current_exists = regular_file_exists(&paths.current)?;
    let file = if current_exists {
        open_private_file(&paths.current, true, false)?
    } else {
        create_private_file(&paths.current)?
    };
    if !current_exists {
        write_log_header(&file)?;
    }
    let current_size = file.metadata()?.len();
    let mut state = RotatingLogState {
        file,
        current_size,
        max_bytes,
        paths,
        _rotation_lock: rotation_lock,
    };

    // A normal process start preserves the last run as the previous generation
    // and begins a new version-marked current log. When recovering the final
    // half of an interrupted rotation, the recovered empty current file is
    // already the correct destination and must not replace the useful previous
    // generation.
    if current_exists && !recovered {
        state.rotate()?;
    }
    Ok(DesktopLogWriter {
        state: Arc::new(Mutex::new(state)),
    })
}

impl<'a> MakeWriter<'a> for DesktopLogWriter {
    type Writer = DesktopLogGuard<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        DesktopLogGuard {
            state: self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        }
    }
}

pub struct DesktopLogGuard<'a> {
    state: MutexGuard<'a, RotatingLogState>,
}

impl Write for DesktopLogGuard<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.state.write(bytes)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.state.file.flush()
    }
}

impl RotatingLogState {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        if DESKTOP_LOG_HEADER.len() as u64 + bytes.len() as u64 > self.max_bytes {
            if self.current_size > DESKTOP_LOG_HEADER.len() as u64 {
                self.rotate()?;
            }
            if self.current_size + OVERSIZED_ENTRY_MARKER.len() as u64 <= self.max_bytes {
                self.file.write_all(OVERSIZED_ENTRY_MARKER)?;
                self.current_size += OVERSIZED_ENTRY_MARKER.len() as u64;
            }
            // Treat the source bytes as consumed without placing a potentially
            // unbounded or sensitive formatter payload on disk.
            return Ok(bytes.len());
        }
        if self.current_size + bytes.len() as u64 > self.max_bytes {
            self.rotate()?;
        }
        self.file.write_all(bytes)?;
        self.current_size += bytes.len() as u64;
        Ok(bytes.len())
    }

    fn rotate(&mut self) -> std::io::Result<()> {
        self.file.flush()?;
        self.file.sync_data()?;
        verify_open_file_matches_path(&self.file, &self.paths.current)?;

        remove_regular_file_if_present(&self.paths.temporary)?;
        let new_file = create_private_file_io(&self.paths.temporary)?;
        write_log_header_io(&new_file)?;
        new_file.sync_data()?;

        remove_regular_file_if_present(&self.paths.previous)?;
        std::fs::rename(&self.paths.current, &self.paths.previous)?;
        if let Err(error) = std::fs::rename(&self.paths.temporary, &self.paths.current) {
            // Restore the old current name when possible. If the recovery rename
            // also fails, the next process can finish the fixed temp->current
            // transition without following or inventing any alternate path.
            let _ = std::fs::rename(&self.paths.previous, &self.paths.current);
            return Err(error);
        }

        self.file = new_file;
        self.current_size = DESKTOP_LOG_HEADER.len() as u64;
        Ok(())
    }
}

fn validate_data_directory(data_dir: &Path) -> AppResult<()> {
    let metadata = std::fs::symlink_metadata(data_dir).map_err(AppError::from)?;
    if metadata_is_link_or_reparse(&metadata) || !metadata.is_dir() {
        return Err(AppError::Validation(
            "desktop log data directory must be a real directory".to_owned(),
        ));
    }
    Ok(())
}

fn validate_optional_regular_file(path: &Path) -> AppResult<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata_is_link_or_reparse(&metadata) || !metadata.is_file() => Err(
            AppError::Validation("desktop log targets must be regular files".to_owned()),
        ),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn recover_interrupted_rotation(paths: &DesktopLogPaths) -> AppResult<bool> {
    let temporary_exists = regular_file_exists(&paths.temporary)?;
    if !temporary_exists {
        return Ok(false);
    }
    if regular_file_exists(&paths.current)? {
        remove_regular_file_if_present(&paths.temporary)?;
        return Ok(false);
    }
    // The temporary file is create-new, version-marked, and already validated
    // as regular. Completing this same-directory rename is the only recovery
    // action; the API never reads the temporary or previous generations.
    ensure_log_header(&paths.temporary)?;
    std::fs::rename(&paths.temporary, &paths.current)?;
    Ok(true)
}

fn regular_file_exists(path: &Path) -> AppResult<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata_is_link_or_reparse(&metadata) => Ok(true),
        Ok(_) => Err(AppError::Validation(
            "desktop log targets must be regular files".to_owned(),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn discard_unversioned_generation(path: &Path) -> AppResult<()> {
    if regular_file_exists(path)? && !has_log_header(path)? {
        remove_regular_file_if_present(path)?;
    }
    Ok(())
}

fn bound_existing_generation(path: &Path, max_bytes: u64) -> AppResult<()> {
    if !regular_file_exists(path)? {
        return Ok(());
    }
    let file = open_private_file(path, false, false)?;
    if file.metadata()?.len() > max_bytes {
        // Keeping the beginning retains the version marker and complete early
        // entries. The next normal startup rotation then remains bounded even
        // when upgrading from a previously unbounded writer.
        file.set_len(max_bytes)?;
        file.sync_data()?;
    }
    Ok(())
}

fn has_log_header(path: &Path) -> AppResult<bool> {
    let mut file = open_private_file(path, false, false)?;
    let mut header = vec![0_u8; DESKTOP_LOG_HEADER.len()];
    match file.read_exact(&mut header) {
        Ok(()) => Ok(header == DESKTOP_LOG_HEADER.as_bytes()),
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn ensure_log_header(path: &Path) -> AppResult<()> {
    if !has_log_header(path)? {
        return Err(AppError::Validation(
            "desktop log recovery file has an invalid version marker".to_owned(),
        ));
    }
    Ok(())
}

fn open_private_file(path: &Path, append: bool, create: bool) -> AppResult<File> {
    open_private_file_io(path, append, create).map_err(AppError::from)
}

fn open_private_file_io(path: &Path, append: bool, create: bool) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).append(append).create(create);
    crate::private_fs::configure_private_open(&mut options, 0o600);
    let file = options.open(path)?;
    crate::private_fs::harden_private_file(path)?;
    verify_open_file_matches_path(&file, path)?;
    Ok(file)
}

fn create_private_file(path: &Path) -> AppResult<File> {
    create_private_file_io(path).map_err(AppError::from)
}

fn create_private_file_io(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).append(true).create_new(true);
    crate::private_fs::configure_private_open(&mut options, 0o600);
    let file = options.open(path)?;
    crate::private_fs::harden_private_file(path)?;
    verify_open_file_matches_path(&file, path)?;
    Ok(file)
}

fn write_log_header(file: &File) -> AppResult<()> {
    write_log_header_io(file).map_err(AppError::from)
}

fn write_log_header_io(file: &File) -> std::io::Result<()> {
    let mut file = file;
    file.write_all(DESKTOP_LOG_HEADER.as_bytes())?;
    file.flush()
}

fn remove_regular_file_if_present(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata_is_link_or_reparse(&metadata) => {
            std::fs::remove_file(path)
        }
        Ok(_) => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "desktop log rotation target is not a regular file",
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn verify_open_file_matches_path(file: &File, path: &Path) -> std::io::Result<()> {
    let path_metadata = std::fs::symlink_metadata(path)?;
    if metadata_is_link_or_reparse(&path_metadata) || !path_metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "desktop log path changed during rotation",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let file_metadata = file.metadata()?;
        if file_metadata.dev() != path_metadata.dev() || file_metadata.ino() != path_metadata.ino()
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "desktop log path changed during rotation",
            ));
        }
    }
    #[cfg(target_os = "windows")]
    {
        let mut options = OpenOptions::new();
        options.read(true);
        crate::private_fs::configure_private_open(&mut options, 0o600);
        let path_file = options.open(path)?;
        if windows_file_identity(file)? != windows_file_identity(&path_file)? {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "desktop log path changed during rotation",
            ));
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn windows_file_identity(file: &File) -> std::io::Result<(u32, u64)> {
    use std::{mem::MaybeUninit, os::windows::io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, GetFileInformationByHandle,
    };

    let mut information = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // SAFETY: the OS writes the complete POD structure on success and the raw
    // handle remains owned by `file` for the duration of the call.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), information.as_mut_ptr()) }
        == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    let information = unsafe { information.assume_init() };
    if information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "desktop log paths must not be reparse points",
        ));
    }
    let file_index =
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);
    Ok((information.dwVolumeSerialNumber, file_index))
}

fn metadata_is_link_or_reparse(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        return metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    }
    #[cfg(not(target_os = "windows"))]
    false
}

static ANSI_ESCAPE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\x1B(?:\[[0-?]*[ -/]*[@-~]|[@-_])").expect("valid ANSI escape regex")
});
static PEM_BLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)-----BEGIN [^-\r\n]+-----.*?-----END [^-\r\n]+-----")
        .expect("valid PEM block regex")
});
static URL_USER_INFO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(https?://)[^/@\s:]+:[^/@\s]+@").expect("valid URL user-info regex")
});
static URL_QUERY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(https?://[^\s?#]+)[?#][^\s]+").expect("valid URL query-string regex")
});
static UPSTREAM_RESPONSE_DETAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b((?:Paperless returned|provider )[^\r\n:]{1,200}):[^\r\n]*")
        .expect("valid upstream response-detail regex")
});
static CREDENTIAL_REMAINDER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)\b(api[_ -]?(?:key|token)|x[_ -]?api[_ -]?key|access[_ -]?token|refresh[_ -]?token|id[_ -]?token|(?:proxy[_ -]?)?authorization|(?:set[_ -]?)?cookies?|csrf(?:[_ -]?token)?|password|passphrase|secret|session(?:[_ -]?(?:id|token))?|token|master[_ -]?key|custom[_ -]?headers?)\b["']?\s*(?::|=)\s*[^\r\n]*"#,
    )
    .expect("valid credential remainder regex")
});
static SECRET_PATH_FIELD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)\b(data_dir|database_path|secret_store|master_key_file|key_file)\b["']?\s*(?::|=)\s*(?:"[^"\r\n]*"|'[^'\r\n]*'|[^\r\n]+)"#,
    )
    .expect("valid secret path field regex")
});
static BEARER_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bbearer\s+[A-Za-z0-9._~+/=-]{8,}").expect("valid bearer token regex")
});
static PREFIXED_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:sk|pk|key|token)-[A-Za-z0-9._-]{8,}\b")
        .expect("valid prefixed token regex")
});
static OPAQUE_VALUE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[A-Za-z0-9_+/=-]{40,}\b").expect("valid opaque value regex"));
static SENSITIVE_CONTENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)\b(ocr(?:[_ -]?text)?|raw[_ -]?text|system[_ -]?prompt|custom[_ -]?prompt|prompt|document[_ -]?(?:text|content)|instructions)\b["']?\s*(?::|=)\s*.*"#,
    )
    .expect("valid sensitive content regex")
});
static TRACING_ENTRY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(\d{4}-\d{2}-\d{2}T\S+)\s+(TRACE|DEBUG|INFO|WARN|ERROR)\s+(.+)$")
        .expect("valid tracing entry regex")
});

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DiagnosticLogEntry {
    pub timestamp: String,
    pub level: String,
    /// Stable, allow-listed event code. Raw tracing messages are never exposed.
    pub event: String,
    /// Stable subsystem name such as `paperless`, `worker`, or `document`.
    pub target: String,
    /// A small allow-list of validated operational values.
    pub fields: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DiagnosticLogResponse {
    pub available: bool,
    pub entries: Vec<DiagnosticLogEntry>,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl DiagnosticLogResponse {
    fn unavailable(message: impl Into<String>) -> Self {
        Self {
            available: false,
            entries: Vec::new(),
            truncated: false,
            updated_at: None,
            message: Some(message.into()),
        }
    }
}

/// Read only the fixed desktop log name from the private data directory. The
/// browser cannot select a path, and the response is bounded before decoding.
pub async fn read_desktop_log(data_dir: &Path) -> DiagnosticLogResponse {
    let path = data_dir.join("paperless-llm.log");
    match tokio::task::spawn_blocking(move || read_log_tail(path)).await {
        Ok(Ok(response)) => response,
        Ok(Err(LogReadError::Unavailable)) => DiagnosticLogResponse::unavailable(
            "No desktop log is available yet. It is created when PaperlessLLM is started from the app or desktop launcher.",
        ),
        Ok(Err(LogReadError::RestartRequired)) => DiagnosticLogResponse::unavailable(
            "This desktop log was created by an older PaperlessLLM build. Quit and restart PaperlessLLM once to enable the safe Logs view.",
        ),
        Ok(Err(LogReadError::Unreadable)) | Err(_) => DiagnosticLogResponse::unavailable(
            "The desktop log is unavailable. PaperlessLLM did not expose its location or contents.",
        ),
    }
}

#[derive(Debug)]
enum LogReadError {
    Unavailable,
    RestartRequired,
    Unreadable,
}

fn read_log_tail(path: PathBuf) -> Result<DiagnosticLogResponse, LogReadError> {
    let symlink_metadata = std::fs::symlink_metadata(&path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            LogReadError::Unavailable
        } else {
            LogReadError::Unreadable
        }
    })?;
    if symlink_metadata.file_type().is_symlink() || !symlink_metadata.is_file() {
        return Err(LogReadError::Unreadable);
    }

    let mut file = open_read_only_no_follow(&path).map_err(|_| LogReadError::Unreadable)?;
    verify_open_file_matches_path(&file, &path).map_err(|_| LogReadError::Unreadable)?;
    let metadata = file.metadata().map_err(|_| LogReadError::Unreadable)?;
    let file_len = metadata.len();
    let mut header = vec![0_u8; DESKTOP_LOG_HEADER.len()];
    file.read_exact(&mut header)
        .map_err(|_| LogReadError::RestartRequired)?;
    if header != DESKTOP_LOG_HEADER.as_bytes() {
        return Err(LogReadError::RestartRequired);
    }
    let start = file_len.saturating_sub(MAX_LOG_BYTES);
    file.seek(SeekFrom::Start(start))
        .map_err(|_| LogReadError::Unreadable)?;
    let mut bytes = Vec::with_capacity((file_len - start) as usize);
    file.take(MAX_LOG_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|_| LogReadError::Unreadable)?;

    // The first bytes may be the middle of a line. Dropping that partial entry
    // also prevents a multiline payload from becoming detached from its label.
    if start > 0 {
        if let Some(newline) = bytes.iter().position(|byte| *byte == b'\n') {
            bytes.drain(..=newline);
        } else {
            bytes.clear();
        }
    }

    let decoded = String::from_utf8_lossy(&bytes);
    let clean = redact_log_text(&decoded);
    let all_entries = clean
        .lines()
        .filter_map(sanitize_log_entry)
        .collect::<Vec<_>>();
    let line_truncated = all_entries.len() > MAX_LOG_LINES;
    let entries = all_entries
        .into_iter()
        .rev()
        .take(MAX_LOG_LINES)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let updated_at = metadata
        .modified()
        .ok()
        .map(DateTime::<Utc>::from)
        .map(|value| value.to_rfc3339());

    Ok(DiagnosticLogResponse {
        available: true,
        entries,
        truncated: start > 0 || line_truncated,
        updated_at,
        message: None,
    })
}

fn open_read_only_no_follow(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    crate::private_fs::configure_private_open(&mut options, 0o600);
    let file = options.open(path)?;
    verify_open_file_matches_path(&file, path)?;
    Ok(file)
}

/// Produce a single-line, low-detail description suitable for operational
/// logging. Internal errors deliberately lose their details at this boundary.
pub fn safe_error_summary(error: &AppError) -> String {
    let detail = match error {
        AppError::NotFound(_) => "requested item was not found",
        AppError::Forbidden(_) => "operation forbidden",
        AppError::Validation(_) => "request validation failed",
        AppError::Conflict(_) => "operation conflict",
        AppError::Budget(_) => "token budget limit reached",
        AppError::Upstream(message) => {
            return crate::error::safe_upstream_message(message);
        }
        AppError::Unauthorized => "authentication required",
        AppError::Database(_)
        | AppError::Migration(_)
        | AppError::Io(_)
        | AppError::Json(_)
        | AppError::Url(_)
        | AppError::Internal(_) => "internal error",
    };
    redact_log_text(detail)
        .lines()
        .next()
        .unwrap_or("error")
        .chars()
        .take(MAX_LINE_CHARS)
        .collect()
}

/// Return a stable diagnostic category without carrying error text into a
/// tracing field. These codes are deliberately coarse and contain no values
/// derived from documents, configuration, paths, or upstream response bodies.
pub fn safe_error_code(error: &AppError) -> &'static str {
    match error {
        AppError::NotFound(_) => "not_found",
        AppError::Unauthorized => "authentication_required",
        AppError::Forbidden(_) => "forbidden",
        AppError::Validation(_) => "validation_failed",
        AppError::Conflict(message) if message == "worker is paused" => "worker_paused",
        AppError::Conflict(_) => "conflict",
        AppError::Budget(_) => "budget_exhausted",
        AppError::Upstream(message)
            if message == "Paperless pagination attempted to leave the configured origin" =>
        {
            "paperless_pagination_origin"
        }
        AppError::Upstream(message) if message.starts_with("Paperless returned ") => {
            "paperless_http_error"
        }
        AppError::Upstream(message) if message.starts_with("provider ") => "provider_error",
        AppError::Upstream(_) => "upstream_error",
        AppError::Database(_) => "database_error",
        AppError::Migration(_) => "migration_error",
        AppError::Io(_) => "io_error",
        AppError::Json(_) => "serialization_error",
        AppError::Url(_) => "url_error",
        AppError::Internal(_) => "internal_error",
    }
}

/// Strip terminal control codes and aggressively remove values that could be
/// credentials, secret-store locations, prompts, or document/OCR content.
pub fn redact_log_text(input: &str) -> String {
    let mut value = ANSI_ESCAPE.replace_all(input, "").into_owned();
    value = PEM_BLOCK.replace_all(&value, "[REDACTED PEM]").into_owned();
    value = URL_USER_INFO
        .replace_all(&value, "$1[REDACTED]@")
        .into_owned();
    value = URL_QUERY.replace_all(&value, "$1?[REDACTED]").into_owned();
    // Older log files may predate safe operational summaries. Strip the
    // response-derived portion of provider and Paperless failures when those
    // files are viewed through the dashboard.
    value = UPSTREAM_RESPONSE_DETAIL
        .replace_all(&value, "$1")
        .into_owned();
    // Once a credential-bearing field begins, discard its complete value and
    // the rest of that physical line. Header values may contain spaces,
    // authentication schemes, commas, semicolons, or nested JSON, so guessing
    // where they end could leave a secret fragment behind.
    value = CREDENTIAL_REMAINDER
        .replace_all(&value, "$1=[REDACTED]")
        .into_owned();
    value = SECRET_PATH_FIELD
        .replace_all(&value, "$1=[REDACTED]")
        .into_owned();
    value = BEARER_TOKEN
        .replace_all(&value, "Bearer [REDACTED]")
        .into_owned();
    value = PREFIXED_TOKEN
        .replace_all(&value, "[REDACTED]")
        .into_owned();
    value = OPAQUE_VALUE.replace_all(&value, "[REDACTED]").into_owned();
    value = SENSITIVE_CONTENT
        .replace_all(&value, "$1=[REDACTED]")
        .into_owned();
    value
}

fn sanitize_log_entry(line: &str) -> Option<DiagnosticLogEntry> {
    let line = line
        .chars()
        .filter(|character| !character.is_control() || *character == '\t')
        .collect::<String>();
    let line = line.trim();
    // tracing-subscriber emits one timestamped entry per event. Refuse
    // continuation lines so multiline upstream responses, prompts, and OCR can
    // never become standalone entries in the browser.
    if line.is_empty() || !TRACING_ENTRY.is_match(line) {
        return None;
    }
    let bounded = line.chars().take(MAX_LINE_CHARS).collect::<String>();
    let captures = TRACING_ENTRY.captures(&bounded)?;
    let timestamp = captures.get(1)?.as_str().to_owned();
    let level = captures.get(2)?.as_str().to_owned();
    let payload = captures.get(3)?.as_str();
    let (event, target) = classify_event(payload);
    Some(DiagnosticLogEntry {
        timestamp,
        level,
        event: event.to_owned(),
        target: target.to_owned(),
        fields: safe_operational_fields(payload),
    })
}

/// Classify only messages and explicit event markers emitted by PaperlessLLM.
/// An unknown tracing message is deliberately reduced to a generic event so a
/// future log statement cannot accidentally expose document content.
fn classify_event(payload: &str) -> (&'static str, &'static str) {
    if let Some(event) = raw_field_value(payload, "event").and_then(known_event) {
        return (event, event_target(event));
    }
    let lower = payload.to_ascii_lowercase();
    if lower.contains("manual paperless scan failed")
        || lower.contains("scheduled paperless scan failed")
    {
        ("paperless.scan_failed", "paperless")
    } else if lower.contains("paperless scan started") {
        ("paperless.scan_started", "paperless")
    } else if lower.contains("paperless scan completed") {
        ("paperless.scan_completed", "paperless")
    } else if lower.contains("document analysis started") {
        ("document.analysis_started", "document")
    } else if lower.contains("document analysis completed") {
        ("document.analysis_completed", "document")
    } else if lower.contains("document queued") {
        ("document.queued", "document")
    } else if lower.contains("document awaiting review") {
        ("document.awaiting_review", "document")
    } else if lower.contains("document apply started") {
        ("document.apply_started", "document")
    } else if lower.contains("document processing failed") {
        ("document.failed", "document")
    } else if lower.contains("document processing budget blocked") {
        ("document.budget_blocked", "document")
    } else if lower.contains("could not recover expired jobs") {
        ("worker.recovery_failed", "worker")
    } else if lower.contains("processing worker error") {
        ("worker.error", "worker")
    } else if lower.contains("charged interrupted ai reservations") {
        ("budget.interrupted_reservations_charged", "budget")
    } else if lower.contains("paperlessllm is ready") {
        ("service.ready", "service")
    } else if lower.contains("shutdown requested") {
        ("service.shutdown_requested", "service")
    } else if lower.contains("could not open the dashboard") {
        ("dashboard.open_failed", "dashboard")
    } else if lower.contains("request failed") {
        ("api.request_failed", "api")
    } else if lower.contains("finished processing request") {
        ("api.request_completed", "api")
    } else {
        ("service.event", "service")
    }
}

fn known_event(value: &str) -> Option<&'static str> {
    Some(match value {
        "service.ready" => "service.ready",
        "service.shutdown_requested" => "service.shutdown_requested",
        "service.fatal" => "service.fatal",
        "service.stopped" => "service.stopped",
        "dashboard.open_failed" => "dashboard.open_failed",
        "worker.started" => "worker.started",
        "worker.idle" => "worker.idle",
        "worker.paused" => "worker.paused",
        "worker.resumed" => "worker.resumed",
        "worker.error" => "worker.error",
        "worker.recovery_failed" => "worker.recovery_failed",
        "paperless.scan_started" => "paperless.scan_started",
        "paperless.scan_completed" => "paperless.scan_completed",
        "paperless.scan_failed" => "paperless.scan_failed",
        "document.queued" => "document.queued",
        "document.analysis_started" => "document.analysis_started",
        "document.analysis_completed" => "document.analysis_completed",
        "document.awaiting_review" => "document.awaiting_review",
        "document.apply_started" => "document.apply_started",
        "document.completed" => "document.completed",
        "document.failed" => "document.failed",
        "document.budget_blocked" => "document.budget_blocked",
        "document.rejected" => "document.rejected",
        "document.queue_removed" => "document.queue_removed",
        "document.retry_queued" => "document.retry_queued",
        "provider.request_started" => "provider.request_started",
        "provider.request_completed" => "provider.request_completed",
        "provider.request_failed" => "provider.request_failed",
        "budget.interrupted_reservations_charged" => "budget.interrupted_reservations_charged",
        "api.request_failed" => "api.request_failed",
        "api.request_completed" => "api.request_completed",
        _ => return None,
    })
}

fn event_target(event: &str) -> &'static str {
    event
        .split_once('.')
        .map_or("service", |(target, _)| match target {
            "service" => "service",
            "dashboard" => "dashboard",
            "worker" => "worker",
            "paperless" => "paperless",
            "document" => "document",
            "provider" => "provider",
            "budget" => "budget",
            "api" => "api",
            _ => "service",
        })
}

fn safe_operational_fields(payload: &str) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::new();
    for name in [
        "document_id",
        "attempt",
        "attempt_count",
        "discovered",
        "queued",
        "skipped",
        "recovered_jobs",
        "recovered_reservations",
        "input_tokens",
        "output_tokens",
        "total_tokens",
        "latency_ms",
    ] {
        if let Some(value) = raw_field_value(payload, name).filter(|value| is_decimal(value)) {
            fields.insert(name.to_owned(), value.to_owned());
        }
    }

    if let Some(value) = raw_field_value(payload, "job_id").filter(|value| is_uuid(value)) {
        fields.insert("job_id".to_owned(), value.to_ascii_lowercase());
    }
    if let Some(value) = raw_field_value(payload, "method").filter(|value| {
        matches!(
            *value,
            "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
        )
    }) {
        fields.insert("method".to_owned(), value.to_owned());
    }
    if let Some(value) = raw_field_value(payload, "path").and_then(safe_request_path) {
        fields.insert("path".to_owned(), value.to_owned());
    }
    if let Some(value) = raw_field_value(payload, "status").and_then(safe_status) {
        fields.insert("status".to_owned(), value.to_owned());
    }
    if let Some(value) = raw_field_value(payload, "provider").and_then(safe_provider) {
        fields.insert("provider".to_owned(), value.to_owned());
    }
    if let Some(value) = raw_field_value(payload, "operation").and_then(safe_operation) {
        fields.insert("operation".to_owned(), value.to_owned());
    }
    if let Some(value) = raw_field_value(payload, "trigger").and_then(safe_trigger) {
        fields.insert("trigger".to_owned(), value.to_owned());
    }
    if let Some(value) = raw_field_value(payload, "error_code").filter(|value| safe_slug(value)) {
        fields.insert("error_code".to_owned(), value.to_owned());
    }
    if let Some(address) =
        raw_field_value(payload, "local_address").and_then(|value| value.parse::<SocketAddr>().ok())
    {
        fields.insert(
            "listener".to_owned(),
            if address.ip().is_loopback() {
                "loopback"
            } else {
                "network"
            }
            .to_owned(),
        );
        fields.insert("port".to_owned(), address.port().to_string());
    }
    add_safe_error_context(payload, &mut fields);
    fields
}

fn raw_field_value<'a>(payload: &'a str, key: &str) -> Option<&'a str> {
    let marker = format!("{key}=");
    let mut search_from = 0;
    while let Some(relative) = payload.get(search_from..)?.find(&marker) {
        let start = search_from + relative;
        let valid_boundary = start == 0
            || payload[..start]
                .chars()
                .next_back()
                .is_some_and(|character| !character.is_ascii_alphanumeric() && character != '_');
        if !valid_boundary {
            search_from = start + marker.len();
            continue;
        }
        let value = &payload[start + marker.len()..];
        if let Some(quoted) = value.strip_prefix('"') {
            return quoted.split('"').next();
        }
        return Some(
            value
                .split(|character: char| {
                    character.is_ascii_whitespace() || character == ',' || character == '}'
                })
                .next()
                .unwrap_or_default(),
        );
    }
    None
}

fn is_decimal(value: &str) -> bool {
    !value.is_empty() && value.len() <= 20 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

fn safe_slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 48
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn safe_status(value: &str) -> Option<&str> {
    match value {
        "queued" | "analyzing" | "awaiting_review" | "applying" | "completed"
        | "budget_blocked" | "failed" | "rejected" | "running" | "pausing" | "paused"
        | "scanning" | "started" | "succeeded" | "ok" | "error" => Some(value),
        value if value.len() == 3 && value.bytes().all(|byte| byte.is_ascii_digit()) => Some(value),
        _ => None,
    }
}

fn safe_provider(value: &str) -> Option<&str> {
    match value {
        "openai" | "claude" | "lmstudio" | "custom" | "mistral" | "kimi" | "moonshot"
        | "google" | "unknown" => Some(value),
        _ => None,
    }
}

fn safe_operation(value: &str) -> Option<&str> {
    match value {
        "analysis" | "repair" | "probe" | "list_models" | "token_count" => Some(value),
        _ => None,
    }
}

fn safe_trigger(value: &str) -> Option<&str> {
    match value {
        "manual" | "scheduled" | "startup" | "retry" | "review" | "automatic" => Some(value),
        _ => None,
    }
}

pub(crate) fn safe_request_path(value: &str) -> Option<&'static str> {
    Some(match value {
        "/healthz" => "/healthz",
        "/api/logs" => "/api/logs",
        "/api/scan" => "/api/scan",
        "/api/status" => "/api/status",
        "/api/bootstrap" => "/api/bootstrap",
        "/api/settings" => "/api/settings",
        "/api/providers" => "/api/providers",
        "/api/jobs" => "/api/jobs",
        "/api/reviews" => "/api/reviews",
        value if value.starts_with("/api/providers/") => "/api/providers/:id",
        value if value.starts_with("/api/jobs/") => "/api/jobs/:id",
        value if value.starts_with("/api/reviews/") => "/api/reviews/:id",
        value if value.starts_with("/api/stats/") => "/api/stats/:view",
        value if value.starts_with("/api/paperless/") => "/api/paperless/:action",
        value if value.starts_with("/api/auth/") => "/api/auth/:action",
        value if value.starts_with("/api/") => "/api/other",
        _ => return None,
    })
}

fn add_safe_error_context(payload: &str, fields: &mut BTreeMap<String, String>) {
    let lower = payload.to_ascii_lowercase();
    let context = if lower.contains("pagination attempted to leave the configured origin") {
        Some((
            "paperless_pagination_origin",
            "Paperless pagination was rejected because its next-page URL left the configured server origin.",
        ))
    } else if lower.contains("paperless returned ") {
        Some((
            "paperless_http_error",
            "Paperless returned an unsuccessful HTTP response.",
        ))
    } else if lower.contains("provider ") && lower.contains("http ") {
        Some((
            "provider_http_error",
            "The AI provider returned an unsuccessful HTTP response.",
        ))
    } else if lower.contains("budget limit reached") || lower.contains("budget exhausted") {
        Some((
            "budget_exhausted",
            "The configured token budget prevented this operation.",
        ))
    } else if lower.contains("authentication required") {
        Some((
            "authentication_required",
            "Authentication was required for this operation.",
        ))
    } else if lower.contains("processing worker is not running")
        || lower.contains("processing worker stopped")
    {
        Some((
            "worker_unavailable",
            "The processing worker was unavailable.",
        ))
    } else if lower.contains("internal error") {
        Some((
            "internal_error",
            "An internal operation failed; no internal details were exposed.",
        ))
    } else {
        None
    };
    if let Some((code, detail)) = context {
        fields
            .entry("error_code".to_owned())
            .or_insert_with(|| code.to_owned());
        fields.insert("detail".to_owned(), detail.to_owned());
        if matches!(code, "paperless_http_error" | "provider_http_error")
            && let Some(status) = lower
                .split_once("http ")
                .and_then(|(_, tail)| tail.get(..3))
                .filter(|status| {
                    status.bytes().all(|byte| byte.is_ascii_digit())
                        && matches!(status.parse::<u16>(), Ok(100..=599))
                })
        {
            fields.insert("upstream_status".to_owned(), status.to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn desktop_writer_rotates_at_the_limit_and_keeps_private_generations() {
        let directory = TempDir::new().unwrap();
        let max_bytes = DESKTOP_LOG_HEADER.len() as u64 + 64;
        let writer = desktop_log_writer_with_limit(directory.path(), max_bytes).unwrap();
        let first = b"2026-08-01T10:00:00Z INFO first safe event\n";
        let second = b"2026-08-01T10:00:01Z INFO second safe event\n";
        {
            let mut log = writer.make_writer();
            log.write_all(first).unwrap();
            log.write_all(second).unwrap();
            log.flush().unwrap();
        }
        drop(writer);

        let current = std::fs::read(directory.path().join(DESKTOP_LOG_FILE)).unwrap();
        let previous = std::fs::read(directory.path().join(DESKTOP_LOG_PREVIOUS_FILE)).unwrap();
        assert!(current.starts_with(DESKTOP_LOG_HEADER.as_bytes()));
        assert!(current.ends_with(second));
        assert!(!current.windows(first.len()).any(|window| window == first));
        assert!(previous.starts_with(DESKTOP_LOG_HEADER.as_bytes()));
        assert!(previous.ends_with(first));
        assert!(current.len() as u64 <= max_bytes);
        assert!(previous.len() as u64 <= max_bytes);

        #[cfg(unix)]
        for name in [
            DESKTOP_LOG_FILE,
            DESKTOP_LOG_PREVIOUS_FILE,
            DESKTOP_LOG_LOCK_FILE,
        ] {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(directory.path().join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "{name} must remain private");
        }
    }

    #[test]
    fn desktop_writer_preserves_the_prior_run_as_previous() {
        let directory = TempDir::new().unwrap();
        let current_path = directory.path().join(DESKTOP_LOG_FILE);
        let old = format!("{DESKTOP_LOG_HEADER}2026-08-01T10:00:00Z INFO prior run\n");
        std::fs::write(&current_path, &old).unwrap();

        let writer = desktop_log_writer_with_limit(directory.path(), 1_024).unwrap();
        drop(writer);

        assert_eq!(
            std::fs::read_to_string(directory.path().join(DESKTOP_LOG_PREVIOUS_FILE)).unwrap(),
            old
        );
        assert_eq!(
            std::fs::read_to_string(current_path).unwrap(),
            DESKTOP_LOG_HEADER
        );
    }

    #[test]
    fn desktop_writer_discards_unversioned_legacy_logs_instead_of_rotating_them() {
        let directory = TempDir::new().unwrap();
        std::fs::write(
            directory.path().join(DESKTOP_LOG_FILE),
            "legacy upstream response containing private document text\n",
        )
        .unwrap();
        std::fs::write(
            directory.path().join(DESKTOP_LOG_PREVIOUS_FILE),
            "older legacy response containing a private token\n",
        )
        .unwrap();

        let writer = desktop_log_writer_with_limit(directory.path(), 1_024).unwrap();
        drop(writer);

        assert_eq!(
            std::fs::read_to_string(directory.path().join(DESKTOP_LOG_FILE)).unwrap(),
            DESKTOP_LOG_HEADER
        );
        assert!(!directory.path().join(DESKTOP_LOG_PREVIOUS_FILE).exists());
    }

    #[test]
    fn desktop_writer_bounds_an_oversized_versioned_prior_run_on_startup() {
        let directory = TempDir::new().unwrap();
        let max_bytes = DESKTOP_LOG_HEADER.len() as u64 + 64;
        let mut old = DESKTOP_LOG_HEADER.as_bytes().to_vec();
        old.extend(std::iter::repeat_n(b'x', 512));
        std::fs::write(directory.path().join(DESKTOP_LOG_FILE), old).unwrap();

        let writer = desktop_log_writer_with_limit(directory.path(), max_bytes).unwrap();
        drop(writer);

        let previous = std::fs::read(directory.path().join(DESKTOP_LOG_PREVIOUS_FILE)).unwrap();
        assert_eq!(previous.len() as u64, max_bytes);
        assert!(previous.starts_with(DESKTOP_LOG_HEADER.as_bytes()));
        assert_eq!(
            std::fs::read_to_string(directory.path().join(DESKTOP_LOG_FILE)).unwrap(),
            DESKTOP_LOG_HEADER
        );
    }

    #[test]
    fn desktop_writer_completes_an_interrupted_fixed_path_rotation() {
        let directory = TempDir::new().unwrap();
        let previous = format!("{DESKTOP_LOG_HEADER}2026-08-01T10:00:00Z INFO retained\n");
        std::fs::write(directory.path().join(DESKTOP_LOG_PREVIOUS_FILE), &previous).unwrap();
        std::fs::write(
            directory.path().join(DESKTOP_LOG_TEMP_FILE),
            DESKTOP_LOG_HEADER,
        )
        .unwrap();

        let writer = desktop_log_writer_with_limit(directory.path(), 1_024).unwrap();
        drop(writer);

        assert_eq!(
            std::fs::read_to_string(directory.path().join(DESKTOP_LOG_FILE)).unwrap(),
            DESKTOP_LOG_HEADER
        );
        assert_eq!(
            std::fs::read_to_string(directory.path().join(DESKTOP_LOG_PREVIOUS_FILE)).unwrap(),
            previous
        );
        assert!(!directory.path().join(DESKTOP_LOG_TEMP_FILE).exists());
    }

    #[cfg(unix)]
    #[test]
    fn desktop_writer_never_follows_any_fixed_log_symlink() {
        use std::os::unix::fs::symlink;

        for name in [
            DESKTOP_LOG_FILE,
            DESKTOP_LOG_PREVIOUS_FILE,
            DESKTOP_LOG_TEMP_FILE,
            DESKTOP_LOG_LOCK_FILE,
        ] {
            let directory = TempDir::new().unwrap();
            let target = directory.path().join("outside-target");
            std::fs::write(&target, "must remain unchanged").unwrap();
            symlink(&target, directory.path().join(name)).unwrap();

            let error = match desktop_log_writer_with_limit(directory.path(), 1_024) {
                Ok(_) => panic!("{name} symlink must be rejected"),
                Err(error) => error,
            };
            assert!(error.to_string().contains("regular files"));
            assert_eq!(
                std::fs::read_to_string(target).unwrap(),
                "must remain unchanged"
            );
        }
    }

    #[test]
    fn oversized_desktop_entry_is_omitted_without_exceeding_the_limit() {
        let directory = TempDir::new().unwrap();
        let max_bytes = DESKTOP_LOG_HEADER.len() as u64 + OVERSIZED_ENTRY_MARKER.len() as u64;
        let writer = desktop_log_writer_with_limit(directory.path(), max_bytes).unwrap();
        let private_oversized = vec![b's'; max_bytes as usize * 2];
        {
            let mut log = writer.make_writer();
            log.write_all(&private_oversized).unwrap();
            log.flush().unwrap();
        }
        drop(writer);

        let current = std::fs::read(directory.path().join(DESKTOP_LOG_FILE)).unwrap();
        assert_eq!(current.len() as u64, max_bytes);
        assert!(current.ends_with(OVERSIZED_ENTRY_MARKER));
        assert!(!current.windows(40).any(|window| window == vec![b's'; 40]));
    }

    #[test]
    fn redacts_terminal_codes_credentials_paths_and_document_content() {
        let input = concat!(
            "\x1b[32m2026-08-01T10:00:00Z INFO request ",
            "authorization=Bearer very-secret-token ",
            "api_key=sk-example123456789 ",
            "data_dir=/Users/person/Library/Application Support/PaperlessLLM ",
            "url=https://person:password@example.test/api/?token=secret ",
            "prompt=classify this private OCR text\x1b[0m"
        );
        let redacted = redact_log_text(input);
        assert!(!redacted.contains("\x1b"));
        assert!(!redacted.contains("very-secret-token"));
        assert!(!redacted.contains("sk-example"));
        assert!(!redacted.contains("/Users/person"));
        assert!(!redacted.contains("password@example"));
        assert!(!redacted.contains("classify this private OCR text"));
        assert!(redacted.contains("[REDACTED]"));
    }

    #[test]
    fn redacts_the_complete_multi_token_authorization_value() {
        let redacted = redact_log_text(
            "2026-08-01T10:00:00Z WARN request authorization=Bearer very-secret-token",
        );
        assert_eq!(
            redacted,
            "2026-08-01T10:00:00Z WARN request authorization=[REDACTED]"
        );
        assert!(!redacted.contains("very-secret-token"));
    }

    #[test]
    fn redacts_basic_token_cookie_and_json_header_values_to_end_of_line() {
        let cases = [
            (
                "2026-08-01T10:00:00Z WARN request authorization=Basic dXNlcjpwYXNz next=still-sensitive",
                ["dXNlcjpwYXNz", "still-sensitive"].as_slice(),
            ),
            (
                "2026-08-01T10:00:00Z WARN request authorization=Token scheme-secret, secondary-secret",
                ["scheme-secret", "secondary-secret"].as_slice(),
            ),
            (
                r#"2026-08-01T10:00:00Z WARN request custom_headers={"Authorization":"Basic encoded-secret","X-Private":"second-secret"}"#,
                ["encoded-secret", "second-secret"].as_slice(),
            ),
            (
                "2026-08-01T10:00:00Z WARN request set-cookie=session=private-session; csrf=private-csrf",
                ["private-session", "private-csrf"].as_slice(),
            ),
        ];

        for (input, secrets) in cases {
            let redacted = redact_log_text(input);
            assert!(redacted.ends_with("=[REDACTED]"), "{redacted}");
            for secret in secrets {
                assert!(!redacted.contains(secret), "{redacted}");
            }
        }
    }

    #[test]
    fn removes_response_bodies_from_existing_log_entries() {
        let paperless = redact_log_text(
            "2026-08-01T10:00:00Z WARN scan error=Paperless returned 500 Internal Server Error: private OCR response",
        );
        assert_eq!(
            paperless,
            "2026-08-01T10:00:00Z WARN scan error=Paperless returned 500 Internal Server Error"
        );
        let provider = redact_log_text(
            "2026-08-01T10:00:00Z WARN worker error=provider analysis failed with HTTP 400: echoed prompt",
        );
        assert_eq!(
            provider,
            "2026-08-01T10:00:00Z WARN worker error=provider analysis failed with HTTP 400"
        );
    }

    #[tokio::test]
    async fn returns_a_bounded_tail_and_ignores_multiline_payloads() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("paperless-llm.log");
        let mut file = File::create(path).unwrap();
        file.write_all(DESKTOP_LOG_HEADER.as_bytes()).unwrap();
        for index in 0..(MAX_LOG_LINES + 20) {
            writeln!(
                file,
                "2026-08-01T10:00:{:02}Z INFO harmless event index={index}",
                index % 60
            )
            .unwrap();
        }
        writeln!(file, "private OCR continuation that must never be returned").unwrap();
        writeln!(
            file,
            "2026-08-01T11:00:00Z ERROR final event token=top-secret-value"
        )
        .unwrap();
        drop(file);

        let response = read_desktop_log(directory.path()).await;
        assert!(response.available);
        assert!(response.truncated);
        assert_eq!(response.entries.len(), MAX_LOG_LINES);
        assert_eq!(response.entries.last().unwrap().event, "service.event");
        let serialized = serde_json::to_string(&response).unwrap();
        assert!(!serialized.contains("private OCR"));
        assert!(!serialized.contains("top-secret-value"));
    }

    #[tokio::test]
    async fn reports_missing_or_symlinked_logs_without_exposing_a_path() {
        let directory = TempDir::new().unwrap();
        let missing = read_desktop_log(directory.path()).await;
        assert!(!missing.available);
        assert!(missing.entries.is_empty());
        assert!(
            !missing
                .message
                .unwrap()
                .contains(directory.path().to_string_lossy().as_ref())
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let target = directory.path().join("secret-store");
            std::fs::write(&target, "secret").unwrap();
            symlink(&target, directory.path().join("paperless-llm.log")).unwrap();
            let linked = read_desktop_log(directory.path()).await;
            assert!(!linked.available);
            assert!(linked.entries.is_empty());
        }
    }

    #[tokio::test]
    async fn refuses_headerless_legacy_logs_even_when_they_contain_timestamped_lines() {
        let directory = TempDir::new().unwrap();
        std::fs::write(
            directory.path().join("paperless-llm.log"),
            "2026-08-01T10:00:00Z INFO timestamp-looking legacy continuation\n",
        )
        .unwrap();

        let response = read_desktop_log(directory.path()).await;
        assert!(!response.available);
        assert!(response.entries.is_empty());
        assert!(
            response
                .message
                .as_deref()
                .unwrap_or_default()
                .contains("restart PaperlessLLM")
        );
    }

    #[test]
    fn safe_error_summary_drops_arbitrary_upstream_bodies_and_internal_details() {
        let upstream = AppError::Upstream(
            "Paperless returned 500 Internal Server Error: private document contents".to_owned(),
        );
        assert_eq!(safe_error_summary(&upstream), "Paperless returned HTTP 500");
        assert_eq!(
            safe_error_summary(&AppError::Internal("sk-private-value".to_owned())),
            "internal error"
        );
        assert_eq!(
            safe_error_summary(&AppError::Validation(
                "custom prompt contained Private Document Content".to_owned()
            )),
            "request validation failed"
        );
        let provider = AppError::Upstream(
            "provider analysis failed with HTTP 400: response echoed private input".to_owned(),
        );
        assert_eq!(
            safe_error_summary(&provider),
            "Provider request failed with HTTP 400"
        );
        assert_eq!(
            safe_error_summary(&AppError::Upstream(
                "future upstream failure mentioning /private/path".to_owned()
            )),
            "Upstream service request failed"
        );
        assert_eq!(
            safe_error_summary(&AppError::Upstream(
                "Paperless pagination attempted to leave the configured origin".to_owned()
            )),
            "Paperless pagination attempted to leave the configured origin"
        );
    }

    #[test]
    fn safe_error_codes_never_include_source_error_values() {
        assert_eq!(
            safe_error_code(&AppError::Upstream(
                "Paperless pagination attempted to leave the configured origin".to_owned()
            )),
            "paperless_pagination_origin"
        );
        assert_eq!(
            safe_error_code(&AppError::Upstream(
                "provider analysis failed with HTTP 400: private response".to_owned()
            )),
            "provider_error"
        );
        assert_eq!(
            safe_error_code(&AppError::Internal("/private/path sk-secret".to_owned())),
            "internal_error"
        );
    }

    #[test]
    fn returns_structured_allow_listed_context_without_raw_messages() {
        let entry = sanitize_log_entry(concat!(
            "2026-08-01T10:00:00Z WARN manual Paperless scan failed ",
            "document_id=42 discovered=8 queued=3 skipped=5 ",
            "error=Paperless pagination attempted to leave the configured origin ",
            "prompt=private document text"
        ))
        .unwrap();

        assert_eq!(entry.timestamp, "2026-08-01T10:00:00Z");
        assert_eq!(entry.level, "WARN");
        assert_eq!(entry.event, "paperless.scan_failed");
        assert_eq!(entry.target, "paperless");
        assert_eq!(
            entry.fields.get("document_id").map(String::as_str),
            Some("42")
        );
        assert_eq!(entry.fields.get("queued").map(String::as_str), Some("3"));
        assert_eq!(
            entry.fields.get("error_code").map(String::as_str),
            Some("paperless_pagination_origin")
        );
        let serialized = serde_json::to_string(&entry).unwrap();
        assert!(!serialized.contains("private document text"));
        assert!(!serialized.contains("prompt"));
    }

    #[test]
    fn unknown_events_never_return_arbitrary_payloads_or_unapproved_fields() {
        let entry = sanitize_log_entry(
            "2026-08-01T10:00:00Z INFO future message customer_name=PrivateName path=/secret/token",
        )
        .unwrap();

        assert_eq!(entry.event, "service.event");
        assert_eq!(entry.target, "service");
        assert!(entry.fields.is_empty());
        let serialized = serde_json::to_string(&entry).unwrap();
        assert!(!serialized.contains("PrivateName"));
        assert!(!serialized.contains("/secret/token"));
    }

    #[test]
    fn accepts_only_known_explicit_events_and_validated_fields() {
        let entry = sanitize_log_entry(concat!(
            "2026-08-01T10:00:00Z INFO document analysis started ",
            "event=\"document.analysis_started\" target=\"private-document-name\" ",
            "job_id=01234567-89ab-cdef-0123-456789abcdef document_id=17 attempt=2 ",
            "provider=lmstudio model=private-model-name status=analyzing"
        ))
        .unwrap();

        assert_eq!(entry.event, "document.analysis_started");
        assert_eq!(entry.target, "document");
        assert_eq!(
            entry.fields.get("document_id").map(String::as_str),
            Some("17")
        );
        assert_eq!(entry.fields.get("attempt").map(String::as_str), Some("2"));
        assert_eq!(
            entry.fields.get("provider").map(String::as_str),
            Some("lmstudio")
        );
        assert_eq!(
            entry.fields.get("status").map(String::as_str),
            Some("analyzing")
        );
        let serialized = serde_json::to_string(&entry).unwrap();
        assert!(!serialized.contains("private-document-name"));
        assert!(!serialized.contains("private-model-name"));
    }

    #[test]
    fn provider_lifecycle_entries_keep_only_allow_listed_operational_fields() {
        let entry = sanitize_log_entry(concat!(
            "2026-08-01T10:00:00Z INFO provider request completed ",
            "event=\"provider.request_completed\" target=\"provider\" ",
            "job_id=01234567-89ab-cdef-0123-456789abcdef document_id=17 attempt=2 ",
            "provider=lmstudio operation=repair status=succeeded ",
            "input_tokens=120 output_tokens=30 total_tokens=150 latency_ms=42 ",
            "model=private-model title=private-title prompt=private document text"
        ))
        .unwrap();

        assert_eq!(entry.event, "provider.request_completed");
        assert_eq!(entry.target, "provider");
        assert_eq!(entry.fields.get("attempt").map(String::as_str), Some("2"));
        assert_eq!(
            entry.fields.get("operation").map(String::as_str),
            Some("repair")
        );
        assert_eq!(
            entry.fields.get("total_tokens").map(String::as_str),
            Some("150")
        );
        assert_eq!(
            entry.fields.get("latency_ms").map(String::as_str),
            Some("42")
        );
        let serialized = serde_json::to_string(&entry).unwrap();
        assert!(!serialized.contains("private-model"));
        assert!(!serialized.contains("private-title"));
        assert!(!serialized.contains("private document text"));
        assert!(!serialized.contains("model"));
        assert!(!serialized.contains("title"));
        assert!(!serialized.contains("prompt"));
    }

    #[test]
    fn hosted_provider_presets_are_safe_log_dimensions() {
        for provider in ["mistral", "kimi", "moonshot", "google"] {
            let entry = sanitize_log_entry(&format!(
                "2026-08-01T10:00:00Z INFO provider request completed \
                 event=\"provider.request_completed\" target=\"provider\" \
                 provider={provider} operation=analysis status=succeeded"
            ))
            .unwrap();
            assert_eq!(
                entry.fields.get("provider").map(String::as_str),
                Some(provider)
            );
        }
        assert_eq!(safe_provider("attacker-controlled"), None);
    }
}
