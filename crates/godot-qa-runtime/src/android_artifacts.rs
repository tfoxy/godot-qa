use godot_qa_core::results::FailureDetail;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Strict mapping between one live session's host artifact directory and its private
/// Android `user://` directory. It intentionally has no ADB dependency: the process
/// layer supplies the binary-safe pull operation.
#[derive(Debug, Clone)]
pub struct AndroidArtifactMapper {
    host_root: PathBuf,
    device_root: String,
}

pub trait AndroidArtifactPuller {
    /// Materialize one controlled device path at `temporary_host_path`.
    ///
    /// The caller atomically renames the temporary path into its final destination.
    fn pull(&mut self, device_path: &str, temporary_host_path: &Path) -> Result<(), FailureDetail>;
}

impl<F> AndroidArtifactPuller for F
where
    F: FnMut(&str, &Path) -> Result<(), FailureDetail>,
{
    fn pull(&mut self, device_path: &str, temporary_host_path: &Path) -> Result<(), FailureDetail> {
        self(device_path, temporary_host_path)
    }
}

impl AndroidArtifactMapper {
    pub fn new(
        host_root: impl Into<PathBuf>,
        device_root: impl Into<String>,
    ) -> Result<Self, FailureDetail> {
        let raw_host_root = host_root.into();
        let host_root = normalize_host_path(&raw_host_root).ok_or_else(|| {
            invalid_mapping_failure(
                "host artifact root must not contain `..`",
                json!("normalized host artifact root"),
                json!(raw_host_root),
            )
        })?;
        let device_root = normalize_device_root(&device_root.into())?;
        Ok(Self {
            host_root,
            device_root,
        })
    }

    pub fn host_root(&self) -> &Path {
        &self.host_root
    }

    pub fn device_root(&self) -> &str {
        &self.device_root
    }

    /// Map a host output path to the equivalent private device path. Paths outside the
    /// exact session root fail closed.
    pub fn host_to_device(&self, host_path: &Path) -> Result<String, FailureDetail> {
        let normalized = normalize_host_path(host_path).ok_or_else(|| {
            artifact_path_failure(
                host_path,
                "host artifact path contains traversal",
                json!(self.host_root),
            )
        })?;
        let relative = normalized.strip_prefix(&self.host_root).map_err(|_| {
            artifact_path_failure(
                host_path,
                "host artifact path is outside the live session artifact root",
                json!(self.host_root),
            )
        })?;
        let suffix = relative_to_slash_path(relative)?;
        Ok(if suffix.is_empty() {
            self.device_root.clone()
        } else {
            format!("{}/{suffix}", self.device_root)
        })
    }

    /// Map a runtime-returned device path to its host destination. Prefix lookalikes,
    /// traversal, and unrelated private paths are rejected.
    pub fn device_to_host(&self, device_path: &str) -> Result<PathBuf, FailureDetail> {
        let relative = device_relative_path(&self.device_root, device_path)?;
        let mut host = self.host_root.clone();
        for segment in relative {
            host.push(segment);
        }
        Ok(host)
    }

    /// Recursively rewrite host artifact strings in outgoing live-command parameters.
    /// Non-path strings are left untouched.
    pub fn map_outgoing_params(&self, value: &mut Value) -> Result<(), FailureDetail> {
        self.walk_outgoing(value)
    }

    /// Recursively rewrite device-root artifact strings to their host destinations
    /// WITHOUT pulling any bytes or creating directories. Applies the same strict
    /// beneath-root validation as [`Self::materialize_response`] (traversal, prefix
    /// lookalikes, and unrelated private paths fail closed) but is byte-free, so it
    /// suits progress heartbeats where the artifacts are not yet materialized.
    pub fn rewrite_paths_only(&self, value: &mut Value) -> Result<(), FailureDetail> {
        match value {
            Value::String(candidate) => {
                if is_device_root_candidate(&self.device_root, candidate) {
                    let destination = self.device_to_host(candidate)?;
                    *candidate = destination.to_string_lossy().into_owned();
                } else if candidate.starts_with("user://godot_qa/live/") {
                    return Err(invalid_mapping_failure(
                        "runtime returned an artifact path outside this Android session root",
                        json!(self.device_root),
                        json!(candidate),
                    ));
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.rewrite_paths_only(item)?;
                }
            }
            Value::Object(object) => {
                for item in object.values_mut() {
                    self.rewrite_paths_only(item)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Pull and recursively rewrite every controlled device artifact path in a live
    /// response. Each unique device path is materialized once.
    pub fn materialize_response(
        &self,
        value: &mut Value,
        puller: &mut impl AndroidArtifactPuller,
    ) -> Result<(), FailureDetail> {
        let mut pulled = BTreeMap::new();
        self.walk_response(value, None, false, false, puller, &mut pulled)
    }

    fn walk_outgoing(&self, value: &mut Value) -> Result<(), FailureDetail> {
        match value {
            Value::String(candidate) => {
                let path = Path::new(candidate);
                match normalize_host_path(path) {
                    Some(normalized) if normalized.starts_with(&self.host_root) => {
                        *candidate = self.host_to_device(path)?;
                    }
                    None => {
                        let prefix: PathBuf = path
                            .components()
                            .take_while(|component| !matches!(component, Component::ParentDir))
                            .collect();
                        if normalize_host_path(&prefix)
                            .is_some_and(|prefix| prefix.starts_with(&self.host_root))
                        {
                            return Err(artifact_path_failure(
                                path,
                                "host artifact path contains traversal",
                                json!(self.host_root),
                            ));
                        }
                    }
                    _ => {}
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.walk_outgoing(item)?;
                }
            }
            Value::Object(object) => {
                for item in object.values_mut() {
                    self.walk_outgoing(item)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn walk_response(
        &self,
        value: &mut Value,
        field_name: Option<&str>,
        inside_error: bool,
        inside_artifacts: bool,
        puller: &mut impl AndroidArtifactPuller,
        pulled: &mut BTreeMap<String, PathBuf>,
    ) -> Result<(), FailureDetail> {
        match value {
            Value::String(candidate) => {
                if is_device_root_candidate(&self.device_root, candidate) {
                    let destination = self.device_to_host(candidate)?;
                    if field_name.is_some_and(is_artifact_directory_field) {
                        fs::create_dir_all(&destination)
                            .map_err(|error| artifact_io_failure(&destination, &error))?;
                    } else if inside_error
                        && !inside_artifacts
                        && field_name.is_some_and(is_best_effort_error_artifact_field)
                        && !pulled.contains_key(candidate)
                    {
                        if materialize_atomically(candidate, &destination, puller).is_ok() {
                            pulled.insert(candidate.clone(), destination.clone());
                        }
                    } else if !(pulled.contains_key(candidate)
                        || inside_error
                            && !inside_artifacts
                            && field_name.is_some_and(is_diagnostic_field))
                    {
                        materialize_atomically(candidate, &destination, puller)?;
                        pulled.insert(candidate.clone(), destination.clone());
                    }
                    *candidate = destination.to_string_lossy().into_owned();
                } else if candidate.starts_with("user://godot_qa/live/") {
                    return Err(invalid_mapping_failure(
                        "runtime returned an artifact path outside this Android session root",
                        json!(self.device_root),
                        json!(candidate),
                    ));
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.walk_response(
                        item,
                        field_name,
                        inside_error,
                        inside_artifacts,
                        puller,
                        pulled,
                    )?;
                }
            }
            Value::Object(object) => {
                for (key, item) in object {
                    self.walk_response(
                        item,
                        Some(key),
                        inside_error || key == "error",
                        inside_artifacts || key == "artifacts",
                        puller,
                        pulled,
                    )?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

fn is_best_effort_error_artifact_field(field: &str) -> bool {
    field == "path"
}

fn is_diagnostic_field(field: &str) -> bool {
    matches!(
        field,
        "path" | "expected" | "actual" | "command" | "message" | "device_root"
    )
}

fn is_artifact_directory_field(field: &str) -> bool {
    matches!(
        field,
        "root"
            | "run_root"
            | "directory"
            | "artifact_dir"
            | "artifacts_dir"
            | "output_dir"
            | "frames_dir"
            | "snapshots_dir"
            | "screenshots_dir"
            | "audio_dir"
            | "temporal_dir"
            | "visual_dir"
    ) || field.ends_with("_dir")
}

fn materialize_atomically(
    device_path: &str,
    destination: &Path,
    puller: &mut impl AndroidArtifactPuller,
) -> Result<(), FailureDetail> {
    let Some(parent) = destination.parent() else {
        return Err(artifact_path_failure(
            destination,
            "host artifact destination has no parent",
            json!("path below live artifact root"),
        ));
    };
    fs::create_dir_all(parent).map_err(|error| artifact_io_failure(destination, &error))?;
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("artifact");
    let temporary = parent.join(format!(".{file_name}.android-pull.tmp"));
    let _ = fs::remove_file(&temporary);
    if let Err(error) = puller.pull(device_path, &temporary) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    if let Err(error) = fs::rename(&temporary, destination) {
        let _ = fs::remove_file(&temporary);
        return Err(artifact_io_failure(destination, &error));
    }
    Ok(())
}

fn normalize_host_path(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => return None,
            Component::CurDir => {}
            #[cfg(windows)]
            Component::Prefix(prefix) => {
                use std::path::Prefix;
                // canonicalize() produces verbatim paths, while Godot emits ordinary
                // drive paths. Both must address the same controlled artifact root.
                match prefix.kind() {
                    Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
                        normalized.push(format!("{}:", char::from(drive.to_ascii_uppercase())))
                    }
                    Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => normalized
                        .push(format!(
                            r"\\{}\{}",
                            server.to_string_lossy(),
                            share.to_string_lossy()
                        )),
                    _ => return None,
                }
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    Some(normalized)
}

fn normalize_device_root(root: &str) -> Result<String, FailureDetail> {
    let normalized = root.trim_end_matches('/');
    if !(normalized.starts_with("user://") || normalized.starts_with('/'))
        || normalized.is_empty()
        || normalized
            .split('/')
            .any(|part| part == "." || part == "..")
    {
        return Err(invalid_mapping_failure(
            "device artifact root must be an absolute or user:// path without traversal",
            json!("controlled absolute device artifact root"),
            json!(root),
        ));
    }
    Ok(normalized.to_string())
}

fn is_device_root_candidate(root: &str, candidate: &str) -> bool {
    candidate == root || candidate.starts_with(&format!("{root}/"))
}

fn device_relative_path(root: &str, candidate: &str) -> Result<Vec<String>, FailureDetail> {
    if !is_device_root_candidate(root, candidate) {
        return Err(invalid_mapping_failure(
            "runtime artifact path is outside the private Android session root",
            json!(root),
            json!(candidate),
        ));
    }
    let suffix = candidate.strip_prefix(root).unwrap_or_default();
    let mut relative = Vec::new();
    for segment in suffix.trim_start_matches('/').split('/') {
        if segment.is_empty() {
            continue;
        }
        if segment == "." || segment == ".." || segment.contains(['\\', ':']) {
            return Err(invalid_mapping_failure(
                "runtime artifact path contains traversal or a non-portable separator",
                json!("normalized path below private Android session root"),
                json!(candidate),
            ));
        }
        #[cfg(windows)]
        if !portable_windows_segment(segment) {
            return Err(invalid_mapping_failure(
                "runtime artifact path contains a reserved Windows filename",
                json!("ordinary filename below private Android session root"),
                json!(candidate),
            ));
        }
        relative.push(segment.to_string());
    }
    Ok(relative)
}

fn relative_to_slash_path(path: &Path) -> Result<String, FailureDetail> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                let Some(value) = value.to_str() else {
                    return Err(invalid_mapping_failure(
                        "host artifact path is not valid UTF-8",
                        json!("UTF-8 artifact path"),
                        json!(path),
                    ));
                };
                parts.push(value.to_string());
            }
            Component::CurDir => {}
            _ => {
                return Err(invalid_mapping_failure(
                    "host artifact path is not a normalized relative path",
                    json!("path below live artifact root"),
                    json!(path),
                ));
            }
        }
    }
    Ok(parts.join("/"))
}

fn invalid_mapping_failure(message: &str, expected: Value, actual: Value) -> FailureDetail {
    FailureDetail {
        kind: "android_artifact_path_invalid".to_string(),
        command: Some("live artifact transport".to_string()),
        message: Some(message.to_string()),
        expected: Some(expected),
        actual: Some(actual),
        ..FailureDetail::default()
    }
}

#[cfg(windows)]
fn portable_windows_segment(segment: &str) -> bool {
    if segment.ends_with(['.', ' '])
        || segment.contains(['<', '>', '"', '|', '?', '*'])
        || segment.chars().any(|c| c.is_control())
    {
        return false;
    }
    let stem = segment.split('.').next().unwrap_or("").to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return false;
    }
    if let Some(suffix) = stem
        .strip_prefix("COM")
        .or_else(|| stem.strip_prefix("LPT"))
        && matches!(
            suffix,
            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
        )
    {
        return false;
    }
    true
}

fn artifact_path_failure(path: &Path, message: &str, expected: Value) -> FailureDetail {
    FailureDetail {
        path: Some(path.to_string_lossy().into_owned()),
        ..invalid_mapping_failure(message, expected, json!(path))
    }
}

fn artifact_io_failure(path: &Path, error: &std::io::Error) -> FailureDetail {
    FailureDetail {
        kind: "android_artifact_materialize_failed".to_string(),
        command: Some("live artifact transport".to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some(error.to_string()),
        expected: Some(json!("binary-safe atomic host artifact materialization")),
        actual: Some(json!(error.kind().to_string())),
        ..FailureDetail::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn assert_json_path_eq(actual: &Value, expected: &Path) {
        assert_eq!(Path::new(actual.as_str().unwrap()), expected);
    }

    fn mapper() -> AndroidArtifactMapper {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        AndroidArtifactMapper::new(
            std::env::temp_dir()
                .join(format!("godot-qa-android-{}-{unique}", std::process::id()))
                .join("session-1"),
            "user://godot_qa/live/session-1",
        )
        .unwrap()
    }

    #[cfg(windows)]
    #[test]
    fn windows_drive_unc_and_verbatim_host_paths_map_consistently() {
        let mapper = AndroidArtifactMapper::new(
            r"C:\QA Space\.qa\live\session-1",
            "user://godot_qa/live/session-1",
        )
        .unwrap();
        for path in [
            r"c:\QA Space\.qa\live\session-1\screenshots\current.png",
            r"\\?\C:\QA Space\.qa\live\session-1\screenshots\current.png",
            "C:/QA Space/.qa/live/session-1/screenshots/current.png",
        ] {
            assert_eq!(
                mapper.host_to_device(Path::new(path)).unwrap(),
                "user://godot_qa/live/session-1/screenshots/current.png"
            );
        }
        let mut escaped = json!(r"\\?\c:\QA Space\.qa\live\session-1\..\escape.png");
        assert!(mapper.map_outgoing_params(&mut escaped).is_err());
        let mapper = AndroidArtifactMapper::new(
            r"\\?\UNC\server\share\live",
            "user://godot_qa/live/session-1",
        )
        .unwrap();
        assert_eq!(
            mapper
                .host_to_device(Path::new(r"\\server\share\live\out.png"))
                .unwrap(),
            "user://godot_qa/live/session-1/out.png"
        );
        assert_eq!(
            mapper
                .device_to_host("user://godot_qa/live/session-1/out.png")
                .unwrap(),
            PathBuf::from(r"\\server\share\live\out.png")
        );
        for segment in ["NUL.png", "COM1", "trailing.", "trailing ", "wild*.png"] {
            assert!(
                mapper
                    .device_to_host(&format!("user://godot_qa/live/session-1/{segment}"))
                    .is_err()
            );
        }
    }

    #[test]
    fn maps_only_exact_controlled_roots() {
        let mapper = mapper();
        assert_eq!(
            mapper
                .host_to_device(&mapper.host_root.join("screenshots/current.png"))
                .unwrap(),
            "user://godot_qa/live/session-1/screenshots/current.png"
        );
        assert_eq!(
            mapper
                .device_to_host("user://godot_qa/live/session-1/audio/capture.wav")
                .unwrap(),
            mapper.host_root.join("audio/capture.wav")
        );
        assert!(
            mapper
                .host_to_device(
                    &mapper
                        .host_root
                        .with_file_name("session-10")
                        .join("leak.png")
                )
                .is_err()
        );
        assert!(
            mapper
                .device_to_host("user://godot_qa/live/session-10/leak.png")
                .is_err()
        );
    }

    #[test]
    fn rejects_host_and_device_traversal() {
        let mapper = mapper();
        assert!(
            mapper
                .host_to_device(&mapper.host_root.join("../../session-2/token"))
                .is_err()
        );
        assert!(
            mapper
                .device_to_host("user://godot_qa/live/session-1/../session-2/token")
                .is_err()
        );
        assert!(
            mapper
                .device_to_host("user://godot_qa/live/session-1\\..\\token")
                .is_err()
        );
    }

    #[test]
    fn device_drive_qualified_segments_cannot_replace_the_host_root() {
        let mapper = mapper();
        for path in [
            "user://godot_qa/live/session-1/C:/outside.txt",
            "user://godot_qa/live/session-1/audio/capture.wav:secret",
        ] {
            assert_eq!(
                mapper.device_to_host(path).unwrap_err().kind,
                "android_artifact_path_invalid"
            );
        }
    }

    #[test]
    fn recursively_maps_outgoing_paths_without_rewriting_normal_strings() {
        let mapper = mapper();
        let mut params = json!({
            "out": json!(mapper.host_root.join("snapshots/state.json")),
            "target": "/root/UI/Button",
            "nested": [json!(mapper.host_root.join("audio/test.wav"))]
        });
        mapper.map_outgoing_params(&mut params).unwrap();
        assert_eq!(
            params["out"],
            "user://godot_qa/live/session-1/snapshots/state.json"
        );
        assert_eq!(params["target"], "/root/UI/Button");
        assert_eq!(
            params["nested"][0],
            "user://godot_qa/live/session-1/audio/test.wav"
        );

        let mut traversal = json!({
            "out": json!(mapper.host_root.join("../../session-2/token"))
        });
        assert!(mapper.map_outgoing_params(&mut traversal).is_err());
    }

    #[test]
    fn response_rejects_artifacts_from_another_session_but_ignores_normal_uris() {
        let mapper = mapper();
        let mut leaked = json!({
            "artifact": "user://godot_qa/live/session-2/screenshots/leak.png"
        });
        let failure = mapper
            .materialize_response(&mut leaked, &mut |_path: &str, _temporary: &Path| {
                unreachable!("out-of-root paths must fail before pulling")
            })
            .unwrap_err();
        assert_eq!(failure.kind, "android_artifact_path_invalid");

        let mut ordinary = json!({
            "scene": "res://main.tscn",
            "settings": "user://settings.json"
        });
        mapper
            .materialize_response(&mut ordinary, &mut |_path: &str, _temporary: &Path| {
                unreachable!("ordinary URIs are not artifacts")
            })
            .unwrap();
    }

    #[test]
    fn failure_diagnostic_paths_do_not_mask_the_runtime_failure() {
        let mapper = mapper();
        let mut response = json!({
            "ok": false,
            "error": {
                "data": {
                    "path": "user://godot_qa/live/session-1/missing.json",
                    "expected": "user://godot_qa/live/session-1/screenshots",
                    "actual": ["user://godot_qa/live/session-1/absent.png"]
                }
            }
        });
        let mut attempts = 0;
        mapper
            .materialize_response(&mut response, &mut |_path: &str, _temporary: &Path| {
                attempts += 1;
                Err(FailureDetail {
                    kind: "android_artifact_pull_failed".to_string(),
                    ..FailureDetail::default()
                })
            })
            .unwrap();
        assert_eq!(attempts, 1, "only error.data.path is a best-effort pull");
        assert_eq!(
            response["error"]["data"]["path"],
            json!(mapper.host_root.join("missing.json"))
        );
        assert_eq!(
            response["error"]["data"]["actual"][0],
            json!(mapper.host_root.join("absent.png"))
        );
        let _ = fs::remove_dir_all(mapper.host_root.parent().unwrap());
    }

    #[test]
    fn error_artifacts_and_success_path_remain_materialized() {
        let mapper = mapper();
        let mut response = json!({
            "ok": false,
            "path": "user://godot_qa/live/session-1/partial.wav",
            "error": {
                "data": {
                    "artifacts": {
                        "screenshot": "user://godot_qa/live/session-1/failure.png"
                    }
                }
            }
        });
        mapper
            .materialize_response(&mut response, &mut |_path: &str, temporary: &Path| {
                fs::write(temporary, b"artifact").unwrap();
                Ok(())
            })
            .unwrap();
        assert!(Path::new(response["path"].as_str().unwrap()).is_file());
        assert!(
            Path::new(
                response["error"]["data"]["artifacts"]["screenshot"]
                    .as_str()
                    .unwrap()
            )
            .is_file()
        );
        let _ = fs::remove_dir_all(mapper.host_root.parent().unwrap());
    }

    #[test]
    fn materializes_binary_data_atomically_and_rewrites_nested_paths() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let host_root = std::env::temp_dir().join(format!("android-artifacts-{unique}"));
        let mapper =
            AndroidArtifactMapper::new(&host_root, "user://godot_qa/live/session-1").unwrap();
        let device_path = "user://godot_qa/live/session-1/screenshots/current.png";
        let mut response = json!({
            "data": {
                "artifacts": {"screenshot": device_path},
                "duplicate": device_path
            }
        });
        let mut calls = 0;
        mapper
            .materialize_response(&mut response, &mut |path: &str, temporary: &Path| {
                calls += 1;
                assert_eq!(path, device_path);
                fs::write(temporary, [0_u8, 255, 1, 128])
                    .map_err(|error| artifact_io_failure(temporary, &error))
            })
            .unwrap();
        let destination = host_root.join("screenshots/current.png");
        assert_eq!(calls, 1);
        assert_eq!(fs::read(&destination).unwrap(), [0_u8, 255, 1, 128]);
        assert_json_path_eq(&response["data"]["artifacts"]["screenshot"], &destination);
        assert_json_path_eq(&response["data"]["duplicate"], &destination);
        assert!(
            !host_root
                .join("screenshots/.current.png.android-pull.tmp")
                .exists()
        );
        let _ = fs::remove_dir_all(host_root);
    }

    #[test]
    fn snapshot_and_audio_directory_artifacts_are_created_without_cat() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let host_root = std::env::temp_dir().join(format!("android-artifact-dirs-{unique}"));
        let mapper =
            AndroidArtifactMapper::new(&host_root, "user://godot_qa/live/session-1").unwrap();
        let mut response = json!({
            "data": {
                "snapshot": {
                    "artifacts": {
                        "root": "user://godot_qa/live/session-1/snapshots/state",
                        "json": "user://godot_qa/live/session-1/snapshots/state/snapshot.json"
                    }
                },
                "audio": {
                    "artifacts": {
                        "audio_dir": "user://godot_qa/live/session-1/audio",
                        "wav": "user://godot_qa/live/session-1/audio/capture.wav"
                    }
                }
            }
        });
        let mut pulled = Vec::new();
        mapper
            .materialize_response(&mut response, &mut |path: &str, temporary: &Path| {
                pulled.push(path.to_string());
                fs::write(temporary, path.as_bytes())
                    .map_err(|error| artifact_io_failure(temporary, &error))
            })
            .unwrap();

        assert_eq!(pulled.len(), 2);
        assert!(
            !pulled
                .iter()
                .any(|path| path.ends_with("/snapshots/state") || path.ends_with("/audio"))
        );
        assert!(host_root.join("snapshots/state").is_dir());
        assert!(host_root.join("audio").is_dir());
        assert!(host_root.join("snapshots/state/snapshot.json").is_file());
        assert!(host_root.join("audio/capture.wav").is_file());
        assert_json_path_eq(
            &response["data"]["snapshot"]["artifacts"]["root"],
            &host_root.join("snapshots/state"),
        );
        assert_json_path_eq(
            &response["data"]["audio"]["artifacts"]["audio_dir"],
            &host_root.join("audio"),
        );
        let _ = fs::remove_dir_all(host_root);
    }

    #[test]
    fn run_root_field_is_created_as_a_directory_without_pulling() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let host_root = std::env::temp_dir().join(format!("android-run-root-{unique}"));
        let mapper =
            AndroidArtifactMapper::new(&host_root, "user://godot_qa/live/session-1").unwrap();
        let mut response = json!({
            "data": {
                "artifacts": {
                    "run_root": "user://godot_qa/live/session-1/scenario/run-1",
                    "result": "user://godot_qa/live/session-1/scenario/run-1/result.json"
                }
            }
        });
        let mut pulled = Vec::new();
        mapper
            .materialize_response(&mut response, &mut |path: &str, temporary: &Path| {
                pulled.push(path.to_string());
                fs::write(temporary, path.as_bytes())
                    .map_err(|error| artifact_io_failure(temporary, &error))
            })
            .unwrap();

        assert_eq!(
            pulled.len(),
            1,
            "run_root is a directory, not a pulled file"
        );
        assert!(!pulled.iter().any(|path| path.ends_with("/run-1")));
        assert!(host_root.join("scenario/run-1").is_dir());
        assert!(host_root.join("scenario/run-1/result.json").is_file());
        assert_json_path_eq(
            &response["data"]["artifacts"]["run_root"],
            &host_root.join("scenario/run-1"),
        );
        let _ = fs::remove_dir_all(host_root);
    }

    #[test]
    fn rewrite_paths_only_rewrites_without_pulling_and_rejects_out_of_root() {
        let mapper = mapper();
        let mut value = json!({
            "progress": {
                "result_path": "user://godot_qa/live/session-1/scenario/run-1/result.json",
                "note": "res://ui/main.tscn"
            },
            "paths": ["user://godot_qa/live/session-1/progress.jsonl"]
        });
        mapper.rewrite_paths_only(&mut value).unwrap();
        assert_json_path_eq(
            &value["progress"]["result_path"],
            &mapper.host_root.join("scenario/run-1/result.json"),
        );
        assert_eq!(value["progress"]["note"], "res://ui/main.tscn");
        assert_eq!(
            value["paths"][0],
            json!(mapper.host_root.join("progress.jsonl"))
        );

        let mut leaked = json!("user://godot_qa/live/session-2/progress.jsonl");
        assert_eq!(
            mapper.rewrite_paths_only(&mut leaked).unwrap_err().kind,
            "android_artifact_path_invalid"
        );
    }

    #[test]
    fn failed_pull_does_not_replace_existing_artifact_or_leave_temporary_file() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let host_root = std::env::temp_dir().join(format!("android-artifacts-failure-{unique}"));
        let destination = host_root.join("trace/events.json");
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(&destination, b"old").unwrap();
        let mapper =
            AndroidArtifactMapper::new(&host_root, "user://godot_qa/live/session-1").unwrap();
        let mut response = json!("user://godot_qa/live/session-1/trace/events.json");
        let failure = mapper
            .materialize_response(&mut response, &mut |_path: &str, temporary: &Path| {
                fs::write(temporary, b"partial").unwrap();
                Err(invalid_mapping_failure(
                    "fake pull failure",
                    json!("success"),
                    json!("failure"),
                ))
            })
            .unwrap_err();
        assert_eq!(failure.kind, "android_artifact_path_invalid");
        assert_eq!(fs::read(&destination).unwrap(), b"old");
        assert!(
            !host_root
                .join("trace/.events.json.android-pull.tmp")
                .exists()
        );
        let _ = fs::remove_dir_all(host_root);
    }
}
