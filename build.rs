use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use sha2::{Digest, Sha256};

fn main() {
    let inputs = [
        PathBuf::from("build.rs"),
        PathBuf::from("Cargo.toml"),
        PathBuf::from("Cargo.lock"),
        PathBuf::from(".cargo/config.toml"),
        PathBuf::from("src"),
        PathBuf::from("migrations"),
        PathBuf::from("web/dist"),
    ];
    for input in &inputs {
        println!("cargo:rerun-if-changed={}", input.display());
    }

    let mut files = Vec::new();
    for input in &inputs {
        collect_files(input, &mut files);
    }
    files.sort();

    let mut digest = Sha256::new();
    digest.update(b"paperless-llm-build-v1\0");
    for path in files {
        let normalized = path.to_string_lossy().replace('\\', "/");
        let contents = fs::read(&path)
            .unwrap_or_else(|error| panic!("could not read build input {normalized}: {error}"));
        digest.update((normalized.len() as u64).to_le_bytes());
        digest.update(normalized.as_bytes());
        digest.update((contents.len() as u64).to_le_bytes());
        digest.update(contents);
    }

    for key in [
        "TARGET",
        "PROFILE",
        "OPT_LEVEL",
        "DEBUG",
        "CARGO_CFG_TARGET_FEATURE",
        "CARGO_ENCODED_RUSTFLAGS",
        "RUSTFLAGS",
    ] {
        println!("cargo:rerun-if-env-changed={key}");
        if let Ok(value) = env::var(key) {
            digest.update(key.as_bytes());
            digest.update(b"=");
            digest.update(value.as_bytes());
            digest.update(b"\0");
        }
    }

    let mut features = env::vars()
        .filter(|(key, _)| key.starts_with("CARGO_FEATURE_"))
        .collect::<Vec<_>>();
    features.sort();
    for (key, value) in features {
        digest.update(key.as_bytes());
        digest.update(b"=");
        digest.update(value.as_bytes());
        digest.update(b"\0");
    }

    if let Ok(rustc) = env::var("RUSTC") {
        println!("cargo:rerun-if-env-changed=RUSTC");
        let version = Command::new(rustc)
            .arg("-vV")
            .output()
            .expect("could not query rustc build identity");
        assert!(version.status.success(), "rustc -vV failed");
        digest.update(version.stdout);
    }

    println!(
        "cargo:rustc-env=PAPERLESS_LLM_BUILD_ID={}",
        to_hex(digest.finalize().as_slice())
    );
}

fn collect_files(path: &Path, files: &mut Vec<PathBuf>) {
    if path.is_file() {
        files.push(path.to_path_buf());
        return;
    }
    if !path.exists() {
        return;
    }
    let mut entries = fs::read_dir(path)
        .unwrap_or_else(|error| panic!("could not inspect build input {}: {error}", path.display()))
        .map(|entry| entry.expect("could not inspect build input entry").path())
        .collect::<Vec<_>>();
    entries.sort();
    for entry in entries {
        collect_files(&entry, files);
    }
}

fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}
