//! Shared Godot diagnostic classification. Callers retain their own verdict policy.
use crate::results::FailureDetail;
use serde_json::json;
use std::{
    fs,
    io::{BufRead, BufReader},
    path::Path,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

pub fn classify(line: &str) -> Option<Severity> {
    let line = line.trim();
    if line.starts_with("ERROR:") || line.starts_with("SCRIPT ERROR:") {
        Some(Severity::Error)
    } else if line.starts_with("WARNING:") || line.starts_with("SCRIPT WARNING:") {
        Some(Severity::Warning)
    } else {
        None
    }
}

pub fn is_ignored(line: &str, severity: Severity, patterns: &[String]) -> bool {
    match severity {
        Severity::Error => is_ignored_godot_log_error(line, patterns),
        Severity::Warning => is_ignored_godot_log_warning(line, patterns),
    }
}

#[derive(Debug, Default)]
pub struct LogScan {
    pub entries: Vec<(Severity, String)>,
    pub ignored_count: usize,
}

impl LogScan {
    pub fn append(&mut self, text: &str, patterns: &[String]) {
        for line in text.lines().map(str::trim) {
            let Some(severity) = classify(line) else {
                continue;
            };
            if is_ignored(line, severity, patterns) {
                self.ignored_count += 1;
            } else {
                self.entries.push((severity, line.to_string()));
            }
        }
    }

    /// Check commands fail on errors and retain warning diagnostics as advisory evidence.
    pub fn diagnostics(
        &self,
        command: &str,
        logs: &Path,
        target: Option<&str>,
    ) -> Vec<FailureDetail> {
        self.entries
            .iter()
            .map(|(severity, line)| FailureDetail {
                kind: match severity {
                    Severity::Error => "godot_log_error",
                    Severity::Warning => "godot_log_warning",
                }
                .to_string(),
                command: Some(command.to_string()),
                path: Some(logs.to_string_lossy().into_owned()),
                target: target.map(str::to_string),
                severity: Some(
                    match severity {
                        Severity::Error => "error",
                        Severity::Warning => "warning",
                    }
                    .to_string(),
                ),
                message: Some("Godot emitted a runtime diagnostic during check".to_string()),
                expected: Some(json!("Godot output without runtime errors")),
                actual: Some(json!(line)),
                ..FailureDetail::default()
            })
            .collect()
    }
}

/// Scan one canonical output copy. Full retained streams take precedence over
/// bounded combined logs; separate fallback streams preserve line boundaries.
pub fn scan_output(logs: &Path, stdout: &str, stderr: &str, patterns: &[String]) -> LogScan {
    let mut result = LogScan::default();
    let mut append_file = |file: fs::File| {
        let mut reader = BufReader::new(file);
        let mut line = Vec::new();
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => result.append(&String::from_utf8_lossy(&line), patterns),
            }
        }
    };
    if let (Ok(out), Ok(err)) = (
        fs::File::open(logs.with_extension("stdout.txt")),
        fs::File::open(logs.with_extension("stderr.txt")),
    ) {
        append_file(out);
        append_file(err);
    } else if let Ok(file) = fs::File::open(logs) {
        append_file(file);
    } else {
        result.append(stdout, patterns);
        result.append(stderr, patterns);
    }
    result
}

fn matches_project_log_ignore(line: &str, ignore_patterns: &[String]) -> bool {
    ignore_patterns
        .iter()
        .any(|pattern| line.contains(pattern.as_str()))
}

fn is_ignored_godot_log_warning(line: &str, ignore_patterns: &[String]) -> bool {
    matches_project_log_ignore(line, ignore_patterns)
        || line.contains("Can't obtain the XDG decoration manager")
        || line.contains("xdg-toplevel-icon protocol not found")
        || line.contains("FIFO protocol not found! Frame pacing will be degraded")
        || line.contains(
            "The requested V-Sync mode Disabled is not available. Falling back to V-Sync mode Enabled",
        )
        || line.contains(
            "Your video card drivers seem not to support the required Vulkan version, switching to OpenGL 3",
        )
        || line.contains(
            "Could not set V-Sync mode, as changing V-Sync mode is not supported by the graphics driver",
        )
        || line.contains("XCreateIC couldn't create wd.xic")
        || line.contains("All audio drivers failed, falling back to the dummy driver")
        || line.contains("ObjectDB instances leaked at exit")
}

fn is_ignored_godot_log_error(line: &str, ignore_patterns: &[String]) -> bool {
    matches_project_log_ignore(line, ignore_patterns)
        || line.contains("Required extension VK_KHR_surface not found")
        || line.contains("Condition \"err != OK\" is true. Returning: err")
        || line.contains("Condition \"status < 0\" is true. Returning: ERR_CANT_OPEN")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn classifies_engine_and_script_diagnostics_without_matching_prose() {
        for (line, expected) in [
            (" ERROR: fault", Some(Severity::Error)),
            ("\tSCRIPT ERROR: fault\r", Some(Severity::Error)),
            ("WARNING: noise", Some(Severity::Warning)),
            (" SCRIPT WARNING: noise", Some(Severity::Warning)),
            ("printed SCRIPT ERROR: text", None),
            ("SCRIPT ERROR count: 1", None),
            ("at: res://main.gd:4", None),
        ] {
            assert_eq!(classify(line), expected, "{line}");
        }
    }

    #[test]
    fn shared_ignores_keep_occurrence_counts_for_both_prefix_families() {
        let mut scan = LogScan::default();
        scan.append("SCRIPT ERROR: ignore-me\nSCRIPT WARNING: ignore-me\nERROR: repeated\nERROR: repeated\nWARNING: ObjectDB instances leaked at exit\n", &["ignore-me".to_string()]);
        assert_eq!(scan.ignored_count, 3);
        assert_eq!(scan.entries.len(), 2);
        assert_eq!(scan.entries[0], scan.entries[1]);
    }

    #[test]
    fn output_uses_one_canonical_copy_and_prefers_complete_retained_streams() {
        let root = std::env::temp_dir().join(format!(
            "qa-log-scan-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let logs = root.join("logs.txt");
        let fallback = scan_output(&logs, "unterminated stdout", "SCRIPT ERROR: stderr\n", &[]);
        assert_eq!(fallback.entries.len(), 1);
        fs::write(&logs, "SCRIPT ERROR: same\n").unwrap();
        assert_eq!(
            scan_output(&logs, "SCRIPT ERROR: same\n", "", &[])
                .entries
                .len(),
            1
        );
        fs::write(
            logs.with_extension("stdout.txt"),
            "SCRIPT ERROR: full-only\nSCRIPT ERROR: full-only\n",
        )
        .unwrap();
        fs::write(
            logs.with_extension("stderr.txt"),
            "SCRIPT WARNING: full-warning\n",
        )
        .unwrap();
        let full = scan_output(&logs, "SCRIPT ERROR: same\n", "", &[]);
        assert_eq!(full.entries.len(), 3);
        assert!(full.entries[0].1.contains("full-only"));
        assert_eq!(
            full.diagnostics("check", &logs, None)[2]
                .severity
                .as_deref(),
            Some("warning")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
