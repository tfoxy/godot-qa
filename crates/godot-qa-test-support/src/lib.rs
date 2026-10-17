//! Native subprocess fixtures for tests; requires the Rust toolchain used by Cargo.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// Each option is encoded as a hex string so paths and output can contain newlines.
/// `request_prefix` selects a request argv value. `write:relative-output` writes beside
/// that request; `write_match:component:relative-output` overrides the default when
/// the request's parent path contains that exact component.
/// `remove_file` deletes a fixture before argument dispatch, tolerating absent files.
pub fn fixture(path: &Path, options: &[(&str, &str)]) -> PathBuf {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    let source = BINARY.get_or_init(|| {
        let root =
            std::env::temp_dir().join(format!("godot-qa-native-fixture-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("helper.rs");
        fs::write(&source, include_str!("helper.rs")).unwrap();
        let binary = root.join(format!("helper{}", std::env::consts::EXE_SUFFIX));
        let output = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .arg("--edition=2024")
            .arg(&source)
            .arg("-o")
            .arg(&binary)
            .output()
            .expect("compile native subprocess fixture with rustc");
        assert!(
            output.status.success(),
            "fixture compile failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        binary
    });
    let path = if cfg!(windows) {
        path.with_extension("exe")
    } else {
        path.to_path_buf()
    };
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    // Never overwrite a hard link to the shared executable: another fixture may
    // be running that inode. Replacing a fixture changes only its directory entry.
    if fs::symlink_metadata(&path).is_ok() {
        fs::remove_file(&path).unwrap();
    }
    if fs::hard_link(source, &path).is_err() {
        fs::copy(source, &path).unwrap();
    }
    let config: String = options
        .iter()
        .map(|(key, value)| {
            let hex: String = value
                .as_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            format!("{key}={hex}\n")
        })
        .collect();
    fs::write(path.with_extension("fixture"), config).unwrap();
    path
}
