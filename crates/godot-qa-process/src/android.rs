use godot_qa_core::results::FailureDetail;
use serde_json::json;
use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

pub const ANDROID_BOOTSTRAP_USER_PATH: &str = "user://godot_qa/android_live_bootstrap.json";
pub const ANDROID_BOOTSTRAP_PRIVATE_PATH: &str = "files/godot_qa/android_live_bootstrap.json";
pub const ANDROID_ARTIFACT_USER_ROOT: &str = "user://godot_qa/live";
pub const ANDROID_ARTIFACT_PRIVATE_ROOT: &str = "files/godot_qa/live";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndroidDevice {
    pub serial: String,
    pub state: String,
    pub description: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndroidExportPreset {
    pub name: String,
    pub package: String,
    pub export_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndroidCommandOutput {
    pub command: Vec<String>,
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndroidLaunchResult {
    pub force_stop: AndroidCommandOutput,
    pub launch: AndroidCommandOutput,
    pub used_explicit_fallback: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndroidLogcatSidecar {
    pub process_identity: Option<u64>,
    pub command: Vec<String>,
    pub pid: i64,
    pub logs_path: String,
}

pub fn resolve_adb_binary(override_binary: Option<&str>) -> Result<String, FailureDetail> {
    resolve_adb_binary_with_env(
        override_binary,
        env::var_os("ADB"),
        env::var_os("ANDROID_SDK_ROOT"),
        env::var_os("ANDROID_HOME"),
        env::var_os("PATH"),
    )
}

fn resolve_adb_binary_with_env(
    override_binary: Option<&str>,
    adb: Option<OsString>,
    sdk_root: Option<OsString>,
    android_home: Option<OsString>,
    path: Option<OsString>,
) -> Result<String, FailureDetail> {
    if let Some(binary) = override_binary.filter(|value| !value.trim().is_empty()) {
        return resolve_named_executable(binary, path.as_deref());
    }
    if let Some(binary) = adb.as_deref().and_then(|value| value.to_str())
        && !binary.trim().is_empty()
    {
        return resolve_named_executable(binary, path.as_deref());
    }
    for root in [sdk_root, android_home].into_iter().flatten() {
        let candidate = PathBuf::from(root).join("platform-tools").join(adb_leaf());
        if candidate.is_file() {
            return Ok(candidate.to_string_lossy().into_owned());
        }
    }
    if let Some(found) = find_on_path(adb_leaf(), path.as_deref()) {
        return Ok(found.to_string_lossy().into_owned());
    }
    Err(failure(
        "android_adb_not_found",
        None,
        None,
        "ADB, ANDROID_SDK_ROOT, ANDROID_HOME, or PATH resolves to adb",
        "adb was not found",
    ))
}

fn resolve_named_executable(
    binary: &str,
    path: Option<&std::ffi::OsStr>,
) -> Result<String, FailureDetail> {
    let candidate = Path::new(binary);
    if candidate.components().count() > 1 || candidate.is_absolute() {
        if candidate.is_file() {
            return Ok(candidate.to_string_lossy().into_owned());
        }
    } else if let Some(found) = find_on_path(binary, path) {
        return Ok(found.to_string_lossy().into_owned());
    }
    Err(failure(
        "android_adb_not_found",
        Some(vec![binary.to_string()]),
        Some(binary.to_string()),
        "configured adb executable exists",
        "configured adb executable was not found",
    ))
}

fn adb_leaf() -> &'static str {
    if cfg!(windows) { "adb.exe" } else { "adb" }
}

fn find_on_path(name: &str, path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    env::split_paths(path?)
        .map(|entry| entry.join(name))
        .find(|candidate| candidate.is_file())
}

pub fn parse_adb_devices(output: &str) -> Vec<AndroidDevice> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("List of devices attached"))
        .filter(|line| !line.starts_with('*'))
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let serial = fields.next()?.to_string();
            let state = fields.next()?.to_string();
            let description = fields
                .filter_map(|field| field.split_once(':'))
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect();
            Some(AndroidDevice {
                serial,
                state,
                description,
            })
        })
        .collect()
}

pub fn list_devices(adb: &str) -> Result<Vec<AndroidDevice>, FailureDetail> {
    let output = run_checked(
        "android_adb_devices_failed",
        vec![adb.to_string(), "devices".into(), "-l".into()],
    )?;
    Ok(parse_adb_devices(&output.stdout))
}

pub fn select_device(
    adb: &str,
    requested_serial: Option<&str>,
) -> Result<AndroidDevice, FailureDetail> {
    let devices = list_devices(adb)?;
    select_device_from(&devices, requested_serial)
}

pub fn select_device_from(
    devices: &[AndroidDevice],
    requested_serial: Option<&str>,
) -> Result<AndroidDevice, FailureDetail> {
    if let Some(serial) = requested_serial {
        let Some(device) = devices.iter().find(|device| device.serial == serial) else {
            return Err(device_selection_failure(
                "android_device_not_found",
                devices,
                Some(serial),
                "requested device is connected and authorized",
            ));
        };
        if device.state != "device" {
            return Err(device_selection_failure(
                match device.state.as_str() {
                    "unauthorized" => "android_device_unauthorized",
                    "offline" => "android_device_offline",
                    _ => "android_device_unavailable",
                },
                devices,
                Some(serial),
                "requested device state is device",
            ));
        }
        return Ok(device.clone());
    }

    let authorized = devices
        .iter()
        .filter(|device| device.state == "device")
        .collect::<Vec<_>>();
    match authorized.as_slice() {
        [device] => Ok((*device).clone()),
        [] => Err(device_selection_failure(
            "android_device_not_found",
            devices,
            None,
            "exactly one authorized Android device is connected",
        )),
        _ => Err(device_selection_failure(
            "android_device_ambiguous",
            devices,
            None,
            "exactly one authorized Android device is connected, or --device is provided",
        )),
    }
}

fn device_selection_failure(
    kind: &str,
    devices: &[AndroidDevice],
    requested: Option<&str>,
    expected: &str,
) -> FailureDetail {
    let mut result = failure(
        kind,
        None,
        requested.map(str::to_string),
        expected,
        "device selection failed",
    );
    result.details.insert(
        "devices".into(),
        json!(
            devices
                .iter()
                .map(|device| json!({
                    "serial": device.serial,
                    "state": device.state,
                    "description": device.description,
                }))
                .collect::<Vec<_>>()
        ),
    );
    result
}

pub fn load_export_preset(
    project_root: &Path,
    preset_name: &str,
    package_override: Option<&str>,
) -> Result<AndroidExportPreset, FailureDetail> {
    let presets_path = project_root.join("export_presets.cfg");
    let contents = fs::read_to_string(&presets_path).map_err(|error| FailureDetail {
        kind: "android_export_preset_read_failed".into(),
        path: Some(presets_path.to_string_lossy().into_owned()),
        message: Some(error.to_string()),
        expected: Some(json!("readable export_presets.cfg")),
        actual: Some(json!("unreadable or missing")),
        ..FailureDetail::default()
    })?;
    let sections = parse_config_sections(&contents);
    let Some((preset_section, values)) = sections.iter().find(|(section, values)| {
        section.starts_with("preset.")
            && !section.ends_with(".options")
            && values.get("name").is_some_and(|name| name == preset_name)
    }) else {
        return Err(failure(
            "android_export_preset_not_found",
            None,
            Some(presets_path.to_string_lossy().into_owned()),
            "named export preset exists",
            preset_name,
        ));
    };
    if values.get("platform").map(String::as_str) != Some("Android") {
        return Err(failure(
            "android_export_preset_invalid",
            None,
            Some(presets_path.to_string_lossy().into_owned()),
            "preset platform is Android",
            values
                .get("platform")
                .map(String::as_str)
                .unwrap_or("missing"),
        ));
    }
    let options_name = format!("{preset_section}.options");
    let empty = BTreeMap::new();
    let options = sections.get(&options_name).unwrap_or(&empty);
    if options
        .get("gradle_build/export_format")
        .is_some_and(|value| value != "0")
    {
        return Err(preset_invalid(
            &presets_path,
            "APK export format (0)",
            "AAB export format",
        ));
    }
    if options.get("permissions/internet").map(String::as_str) != Some("true") {
        return Err(preset_invalid(
            &presets_path,
            "permissions/internet=true",
            "Android INTERNET permission is disabled",
        ));
    }
    validate_addon_autoload(project_root, values)?;

    let raw_package = package_override
        .map(str::to_string)
        .or_else(|| options.get("package/unique_name").cloned())
        .unwrap_or_else(|| "com.example.$genname".into());
    let project_name = read_project_name(project_root)?;
    let package = raw_package.replace("$genname", &godot_android_basename(&project_name));
    validate_package_name(&package)?;
    Ok(AndroidExportPreset {
        name: preset_name.to_string(),
        package,
        export_path: values
            .get("export_path")
            .filter(|value| !value.is_empty())
            .cloned(),
    })
}

fn validate_addon_autoload(
    project_root: &Path,
    preset_values: &BTreeMap<String, String>,
) -> Result<(), FailureDetail> {
    let project_path = project_root.join("project.godot");
    let project = fs::read_to_string(&project_path).map_err(|error| FailureDetail {
        kind: "android_export_preset_invalid".into(),
        path: Some(project_path.to_string_lossy().into_owned()),
        message: Some(error.to_string()),
        expected: Some(json!("readable project.godot with GodotQaBridge autoload")),
        actual: Some(json!("unreadable or missing")),
        ..FailureDetail::default()
    })?;
    let sections = parse_config_sections(&project);
    let autoload = sections
        .get("autoload")
        .and_then(|values| values.get("GodotQaBridge"));
    if autoload.map(String::as_str) != Some("*res://addons/godot_qa/qa_bridge.gd") {
        return Err(preset_invalid(
            &project_path,
            "[autoload] GodotQaBridge=\"*res://addons/godot_qa/qa_bridge.gd\"",
            autoload.map(String::as_str).unwrap_or("missing"),
        ));
    }
    if !project_root.join("addons/godot_qa/qa_bridge.gd").is_file() {
        return Err(preset_invalid(
            &project_path,
            "installed addons/godot_qa/qa_bridge.gd",
            "addon bridge file is missing",
        ));
    }
    if preset_values
        .get("exclude_filter")
        .is_some_and(|filter| exclude_filter_removes_addon(filter))
    {
        return Err(preset_invalid(
            &project_path,
            "exclude_filter retains addons/godot_qa",
            "the QA addon is removed by exclude_filter",
        ));
    }
    match preset_values.get("export_filter").map(String::as_str) {
        Some("exclude")
            if preset_values
                .get("export_files")
                .is_some_and(|files| resource_filter_removes_addon(files)) =>
        {
            return Err(preset_invalid(
                &project_path,
                "excluded export resources do not contain addons/godot_qa",
                "the QA addon is explicitly excluded",
            ));
        }
        Some("customized")
            if preset_values
                .get("customized_files")
                .is_some_and(|files| customized_filter_removes_addon(files)) =>
        {
            return Err(preset_invalid(
                &project_path,
                "customized export resources retain addons/godot_qa",
                "the QA addon is removed by the customized export filter",
            ));
        }
        _ => {}
    }
    Ok(())
}

fn resource_filter_removes_addon(filter: &str) -> bool {
    [
        "res://",
        "res://addons",
        "res://addons/godot_qa",
        "res://addons/godot_qa/qa_bridge.gd",
    ]
    .iter()
    .any(|path| filter.contains(&format!("\"{path}\"")))
        || filter.contains("\"res://addons/godot_qa/")
}

fn exclude_filter_removes_addon(filter: &str) -> bool {
    filter.split(',').map(str::trim).any(|pattern| {
        pattern == "*"
            || pattern == "**"
            || pattern == "addons"
            || pattern.starts_with("addons/*")
            || pattern.starts_with("addons/**")
            || pattern.starts_with("addons/godot_qa")
            || pattern.starts_with("res://addons/godot_qa")
    })
}

fn customized_filter_removes_addon(filter: &str) -> bool {
    [
        "res://",
        "res://addons",
        "res://addons/godot_qa",
        "res://addons/godot_qa/qa_bridge.gd",
    ]
    .iter()
    .any(|path| {
        let quoted = format!("\"{path}\"");
        filter.match_indices(&quoted).any(|(index, _)| {
            filter[index + quoted.len()..]
                .trim_start()
                .starts_with(": 3")
        })
    }) || filter
        .match_indices("\"res://addons/godot_qa/")
        .any(|(index, _)| {
            let rest = &filter[index + 1..];
            rest.find('"')
                .is_some_and(|end| rest[end + 1..].trim_start().starts_with(": 3"))
        })
}

fn preset_invalid(path: &Path, expected: &str, actual: &str) -> FailureDetail {
    failure(
        "android_export_preset_invalid",
        None,
        Some(path.to_string_lossy().into_owned()),
        expected,
        actual,
    )
}

fn read_project_name(project_root: &Path) -> Result<String, FailureDetail> {
    let path = project_root.join("project.godot");
    let contents = fs::read_to_string(&path).map_err(|error| FailureDetail {
        kind: "android_project_read_failed".into(),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some(error.to_string()),
        expected: Some(json!("readable project.godot")),
        actual: Some(json!("unreadable or missing")),
        ..FailureDetail::default()
    })?;
    Ok(parse_config_sections(&contents)
        .get("application")
        .and_then(|values| values.get("config/name"))
        .cloned()
        .unwrap_or_else(|| "Godot".into()))
}

fn godot_android_basename(name: &str) -> String {
    let mut result = String::new();
    for character in name.to_ascii_lowercase().chars() {
        if result.is_empty() && character.is_ascii_digit() {
            continue;
        }
        if character == '_' || character.is_ascii_alphanumeric() {
            result.push(character);
        }
    }
    if result.is_empty() {
        "noname".into()
    } else {
        result
    }
}

fn validate_package_name(package: &str) -> Result<(), FailureDetail> {
    let segments = package.split('.').collect::<Vec<_>>();
    let valid = segments.len() >= 2
        && segments.iter().all(|segment| {
            !segment.is_empty()
                && segment
                    .chars()
                    .next()
                    .is_some_and(|character| character.is_ascii_alphabetic())
                && segment
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_')
        });
    if valid {
        Ok(())
    } else {
        Err(failure(
            "android_package_invalid",
            None,
            None,
            "valid Android application ID with at least two segments",
            package,
        ))
    }
}

fn parse_config_sections(contents: &str) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut sections = BTreeMap::<String, BTreeMap<String, String>>::new();
    let mut current = String::new();
    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(section) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            current = section.to_string();
            sections.entry(current.clone()).or_default();
            continue;
        }
        let Some((key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let value = serde_json::from_str::<String>(raw_value.trim())
            .unwrap_or_else(|_| raw_value.trim().to_string());
        sections
            .entry(current.clone())
            .or_default()
            .insert(key.trim().to_string(), value);
    }
    sections
}

pub fn export_debug_apk(
    godot: &str,
    project_root: &Path,
    preset: &str,
    apk_path: &Path,
    logs_path: &Path,
) -> Result<AndroidCommandOutput, FailureDetail> {
    if apk_path.extension().and_then(|value| value.to_str()) != Some("apk") {
        return Err(failure(
            "android_export_failed",
            None,
            Some(apk_path.to_string_lossy().into_owned()),
            "debug export output has an .apk extension",
            "non-APK output path",
        ));
    }
    let command = vec![
        godot.into(),
        "--headless".into(),
        "--path".into(),
        project_root.to_string_lossy().into_owned(),
        "--export-debug".into(),
        preset.into(),
        apk_path.to_string_lossy().into_owned(),
    ];
    let result = run_command(&command, None)
        .map_err(|error| spawn_failure("android_export_failed", &command, error))?;
    if let Some(parent) = logs_path.parent() {
        fs::create_dir_all(parent).map_err(|error| artifact_failure(logs_path, error))?;
    }
    fs::write(logs_path, format!("{}{}", result.stdout, result.stderr))
        .map_err(|error| artifact_failure(logs_path, error))?;
    let result = checked_output("android_export_failed", result)?;
    if !apk_path.is_file() {
        return Err(failure(
            "android_export_failed",
            Some(result.command),
            Some(apk_path.to_string_lossy().into_owned()),
            "Godot creates the debug APK",
            "export command succeeded without creating the APK",
        ));
    }
    Ok(result)
}

pub fn install_apk(
    adb: &str,
    serial: &str,
    apk_path: &Path,
) -> Result<AndroidCommandOutput, FailureDetail> {
    run_checked(
        "android_install_failed",
        adb_command(adb, serial, &["install", "-r", &apk_path.to_string_lossy()]),
    )
}

pub fn reconnect_wireless_device(
    adb: &str,
    serial: &str,
) -> Result<AndroidCommandOutput, FailureDetail> {
    if !is_wireless_adb_serial(serial) {
        return Err(failure(
            "android_device_unavailable",
            None,
            Some(serial.to_string()),
            "wireless ADB serial in host:port or mDNS service form",
            "serial is not a wireless ADB endpoint",
        ));
    }
    let command = vec![adb.to_string(), "connect".to_string(), serial.to_string()];
    let output = run_command(&command, None)
        .map_err(|error| spawn_failure("android_device_unavailable", &command, error))?;
    checked_output("android_device_unavailable", output)
}

pub fn is_wireless_adb_serial(serial: &str) -> bool {
    serial.contains(':')
        || serial.ends_with("._adb-tls-connect._tcp")
        || serial.ends_with("._adb._tcp")
}

/// Labels for a private-file streaming write failure. Each caller keeps its own
/// stable failure kind/message/expected so existing result contracts are unchanged.
struct PrivateWriteLabels {
    kind: &'static str,
    message: &'static str,
    expected: &'static str,
}

/// Shared private-storage streamer: `run-as <pkg> mkdir -p <parent>` then
/// `run-as <pkg> sh -c 'umask 077; cat > <path>'` with the bytes streamed on STDIN
/// (never argv). Stdout/stderr are blanked on success so even a nonstandard ADB
/// wrapper that echoes stdin cannot surface the payload through command results,
/// logs, metadata, or debug formatting.
fn stream_private_file(
    adb: &str,
    serial: &str,
    package: &str,
    parent_dir: &str,
    file_path: &str,
    bytes: &[u8],
    labels: &PrivateWriteLabels,
) -> Result<AndroidCommandOutput, FailureDetail> {
    run_checked(
        labels.kind,
        adb_command(
            adb,
            serial,
            &["shell", "run-as", package, "mkdir", "-p", parent_dir],
        ),
    )?;
    let cat_command = format!("'umask 077; cat > {file_path}'");
    let command = adb_command(
        adb,
        serial,
        &["shell", "run-as", package, "sh", "-c", &cat_command],
    );
    let output = run_command(&command, Some(bytes))
        .map_err(|error| spawn_failure(labels.kind, &command, error))?;
    if output.status != 0 {
        return Err(FailureDetail {
            kind: labels.kind.into(),
            command: Some(crate::format_command(&command)),
            path: command.first().cloned(),
            message: Some(labels.message.to_string()),
            expected: Some(json!(labels.expected)),
            actual: Some(json!(output.status)),
            ..FailureDetail::default()
        });
    }
    Ok(AndroidCommandOutput {
        stdout: String::new(),
        stderr: String::new(),
        ..output
    })
}

pub fn write_private_bootstrap(
    adb: &str,
    serial: &str,
    package: &str,
    bootstrap_json: &[u8],
) -> Result<AndroidCommandOutput, FailureDetail> {
    validate_package_name(package)?;
    // The bootstrap payload is the sole copy containing the session token, so the
    // streamer's stdin/blank-output secrecy pattern is essential here.
    stream_private_file(
        adb,
        serial,
        package,
        "files/godot_qa",
        ANDROID_BOOTSTRAP_PRIVATE_PATH,
        bootstrap_json,
        &PrivateWriteLabels {
            kind: "android_bootstrap_write_failed",
            message: "ADB could not write the private Android live bootstrap",
            expected: "run-as bootstrap write exits with status 0",
        },
    )
}

/// Stream `bytes` into `files/godot_qa/live/<session_id>/<relative_path>` inside the
/// package's private storage, creating the parent directory first. The package,
/// session id, and every relative path segment are charset-validated so the path is
/// safe to embed in the `sh -c` command string; traversal/empty/invalid inputs are
/// rejected. Bytes are streamed on STDIN (never argv) and the returned output is
/// blanked exactly like [`write_private_bootstrap`].
pub fn write_private_file(
    adb: &str,
    serial: &str,
    package: &str,
    session_id: &str,
    relative_path: &str,
    bytes: &[u8],
) -> Result<AndroidCommandOutput, FailureDetail> {
    validate_package_name(package)?;
    validate_session_id(session_id)?;
    validate_private_relative_path(relative_path)?;
    let file_path = format!("{ANDROID_ARTIFACT_PRIVATE_ROOT}/{session_id}/{relative_path}");
    let parent_dir = file_path
        .rsplit_once('/')
        .map(|(head, _)| head.to_string())
        .unwrap_or_else(|| format!("{ANDROID_ARTIFACT_PRIVATE_ROOT}/{session_id}"));
    stream_private_file(
        adb,
        serial,
        package,
        &parent_dir,
        &file_path,
        bytes,
        &PrivateWriteLabels {
            kind: "android_private_write_failed",
            message: "ADB could not write the private Android session file",
            expected: "run-as private file write exits with status 0",
        },
    )
}

/// `run-as <pkg> mkdir -p` every `files/godot_qa/live/<session_id>/<relative>` in a
/// single command. Every relative segment is charset-validated like
/// [`write_private_file`]; an empty list is a no-op.
pub fn make_private_dirs(
    adb: &str,
    serial: &str,
    package: &str,
    session_id: &str,
    relatives: &[&str],
) -> Result<(), FailureDetail> {
    validate_package_name(package)?;
    validate_session_id(session_id)?;
    if relatives.is_empty() {
        return Ok(());
    }
    for relative in relatives {
        validate_private_relative_path(relative)?;
    }
    let session_root = format!("{ANDROID_ARTIFACT_PRIVATE_ROOT}/{session_id}");
    let mut command = adb_command(adb, serial, &["shell", "run-as", package, "mkdir", "-p"]);
    for relative in relatives {
        command.push(format!("{session_root}/{relative}"));
    }
    run_checked("android_private_mkdir_failed", command).map(|_| ())
}

pub fn create_forward(
    adb: &str,
    serial: &str,
    host_port: u16,
    device_port: u16,
) -> Result<AndroidCommandOutput, FailureDetail> {
    run_checked(
        "android_forward_failed",
        adb_command(
            adb,
            serial,
            &[
                "forward",
                "--no-rebind",
                &format!("tcp:{host_port}"),
                &format!("tcp:{device_port}"),
            ],
        ),
    )
}

pub fn remove_forward(
    adb: &str,
    serial: &str,
    host_port: u16,
) -> Result<AndroidCommandOutput, FailureDetail> {
    let command = adb_command(
        adb,
        serial,
        &["forward", "--remove", &format!("tcp:{host_port}")],
    );
    let output = run_command(&command, None)
        .map_err(|error| spawn_failure("android_forward_remove_failed", &command, error))?;
    if output.status == 0 || forward_listener_is_already_absent(&output, host_port) {
        Ok(output)
    } else {
        Err(command_failure(
            "android_forward_remove_failed",
            &output.command,
            output.status,
            &output.stderr,
        ))
    }
}

pub fn remove_exact_forward(
    adb: &str,
    serial: &str,
    host_port: u16,
    device_port: u16,
) -> Result<Option<AndroidCommandOutput>, FailureDetail> {
    let listed = run_checked(
        "android_forward_remove_failed",
        adb_command(adb, serial, &["forward", "--list"]),
    )?;
    let local = format!("tcp:{host_port}");
    let remote = format!("tcp:{device_port}");
    let exact_mapping_exists = listed.stdout.lines().any(|line| {
        let mut fields = line.split_whitespace();
        fields.next() == Some(serial)
            && fields.next() == Some(local.as_str())
            && fields.next() == Some(remote.as_str())
            && fields.next().is_none()
    });
    if !exact_mapping_exists {
        return Ok(None);
    }
    remove_forward(adb, serial, host_port).map(Some)
}

fn forward_listener_is_already_absent(output: &AndroidCommandOutput, host_port: u16) -> bool {
    if !output.stdout.trim().is_empty() {
        return false;
    }
    let listener = format!("listener 'tcp:{host_port}' not found");
    matches!(
        output.stderr.trim(),
        message
            if message == format!("error: {listener}")
                || message == format!("adb: error: {listener}")
    )
}

pub fn launch_app(
    adb: &str,
    serial: &str,
    package: &str,
) -> Result<AndroidLaunchResult, FailureDetail> {
    let force_stop = force_stop(adb, serial, package)?;
    let implicit_command = adb_command(
        adb,
        serial,
        &[
            "shell",
            "am",
            "start",
            "-a",
            "android.intent.action.MAIN",
            "-c",
            "android.intent.category.LAUNCHER",
            "-p",
            package,
        ],
    );
    let implicit = run_command(&implicit_command, None)
        .map_err(|error| spawn_failure("android_launch_failed", &implicit_command, error))?;
    let implicit_failed = android_activity_start_failed(&implicit);
    if !implicit_failed {
        return Ok(AndroidLaunchResult {
            force_stop,
            launch: implicit,
            used_explicit_fallback: false,
        });
    }
    for activity in ["com.godot.game.GodotAppLauncher", "com.godot.game.GodotApp"] {
        let component = format!("{package}/{activity}");
        let explicit_command =
            adb_command(adb, serial, &["shell", "am", "start", "-n", &component]);
        let explicit = run_command(&explicit_command, None)
            .map_err(|error| spawn_failure("android_launch_failed", &explicit_command, error))?;
        if !android_activity_start_failed(&explicit) {
            return Ok(AndroidLaunchResult {
                force_stop,
                launch: explicit,
                used_explicit_fallback: true,
            });
        }
    }
    Err(failure(
        "android_launch_failed",
        Some(implicit_command),
        Some(package.to_string()),
        "implicit launcher or a supported Godot launcher component starts",
        "all launcher attempts failed",
    ))
}

fn android_activity_start_failed(output: &AndroidCommandOutput) -> bool {
    output.status != 0
        || output.stdout.contains("Error: Activity not started")
        || output.stderr.contains("Error: Activity not started")
        || output.stdout.contains("Error type 3")
        || output.stderr.contains("Error type 3")
}

pub fn query_pid(adb: &str, serial: &str, package: &str) -> Result<Option<i64>, FailureDetail> {
    validate_package_name(package)?;
    let command = adb_command(adb, serial, &["shell", "pidof", "-s", package]);
    let output = run_command(&command, None)
        .map_err(|error| spawn_failure("android_pid_query_failed", &command, error))?;
    let value = output.stdout.trim();
    if output.status == 1 && value.is_empty() && output.stderr.trim().is_empty() {
        return Ok(None);
    }
    if output.status != 0 {
        return Err(command_failure(
            "android_pid_query_failed",
            &command,
            output.status,
            &output.stderr,
        ));
    }
    if value.is_empty() {
        Ok(None)
    } else {
        value.parse::<i64>().map(Some).map_err(|_| {
            failure(
                "android_pid_query_failed",
                Some(output.command),
                None,
                "pidof returns a numeric PID",
                value,
            )
        })
    }
}

pub fn start_logcat(
    adb: &str,
    serial: &str,
    device_pid: i64,
    logs_path: &Path,
) -> Result<AndroidLogcatSidecar, FailureDetail> {
    if device_pid <= 0 {
        return Err(failure(
            "android_logcat_start_failed",
            None,
            None,
            "positive device PID",
            &device_pid.to_string(),
        ));
    }
    if let Some(parent) = logs_path.parent() {
        fs::create_dir_all(parent).map_err(|error| artifact_failure(logs_path, error))?;
    }
    let stdout = File::create(logs_path).map_err(|error| artifact_failure(logs_path, error))?;
    let stderr = stdout
        .try_clone()
        .map_err(|error| artifact_failure(logs_path, error))?;
    let command = adb_command(adb, serial, &["logcat", "--pid", &device_pid.to_string()]);
    let mut child = Command::new(&command[0]);
    child
        .args(&command[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    #[cfg(unix)]
    unsafe {
        child.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    crate::configure_background_command(&mut child);
    child.stdin(Stdio::null());
    let child = child
        .spawn()
        .map_err(|error| spawn_failure("android_logcat_start_failed", &command, error))?;
    Ok(AndroidLogcatSidecar {
        process_identity: crate::process_identity(i64::from(child.id())),
        command,
        pid: child.id() as i64,
        logs_path: logs_path.to_string_lossy().into_owned(),
    })
}

pub fn stop_logcat(pid: i64) -> Result<(), FailureDetail> {
    stop_logcat_with_identity(pid, crate::process_identity(pid))
}

pub fn stop_logcat_with_identity(pid: i64, identity: Option<u64>) -> Result<(), FailureDetail> {
    #[cfg(not(windows))]
    let _ = identity;
    if pid <= 0 {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        let command_line_path = PathBuf::from(format!("/proc/{pid}/cmdline"));
        match fs::read(&command_line_path) {
            Ok(command_line)
                if !command_line
                    .windows(b"logcat".len())
                    .any(|window| window == b"logcat") =>
            {
                return Err(FailureDetail {
                    kind: "android_logcat_stop_failed".into(),
                    message: Some("recorded logcat PID is now owned by a different process".into()),
                    expected: Some(json!("owned adb logcat helper process")),
                    actual: Some(json!("PID ownership mismatch")),
                    details: [("pid".into(), json!(pid))].into_iter().collect(),
                    ..FailureDetail::default()
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(FailureDetail {
                    kind: "android_logcat_stop_failed".into(),
                    message: Some(error.to_string()),
                    expected: Some(json!("readable logcat process ownership metadata")),
                    actual: Some(json!(error.kind().to_string())),
                    details: [("pid".into(), json!(pid))].into_iter().collect(),
                    ..FailureDetail::default()
                });
            }
            Ok(_) => {}
        }
    }
    #[cfg(unix)]
    {
        let result = unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGTERM) };
        if result == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        Err(FailureDetail {
            kind: "android_logcat_stop_failed".into(),
            message: Some(error.to_string()),
            expected: Some(json!("owned logcat process group stops")),
            actual: Some(json!("signal failed")),
            details: [("pid".into(), json!(pid))].into_iter().collect(),
            ..FailureDetail::default()
        })
    }
    #[cfg(windows)]
    {
        if !crate::pid_is_running(pid) {
            return Ok(());
        }
        if identity.is_none() {
            return Err(FailureDetail {
                kind: "android_logcat_stop_failed".into(),
                message: Some("saved logcat helper lacks a Windows process identity".into()),
                ..FailureDetail::default()
            });
        }
        crate::terminate_process_tree(pid, true, identity).map_err(|error| FailureDetail {
            kind: "android_logcat_stop_failed".into(),
            message: Some(error),
            expected: Some(json!("owned logcat process tree stops")),
            actual: Some(json!("Windows process termination failed")),
            details: [("pid".into(), json!(pid))].into_iter().collect(),
            ..FailureDetail::default()
        })
    }
}

pub fn force_stop(
    adb: &str,
    serial: &str,
    package: &str,
) -> Result<AndroidCommandOutput, FailureDetail> {
    validate_package_name(package)?;
    run_checked(
        "android_force_stop_failed",
        adb_command(adb, serial, &["shell", "am", "force-stop", package]),
    )
}

pub fn remove_private_session(
    adb: &str,
    serial: &str,
    package: &str,
    session_id: &str,
) -> Result<AndroidCommandOutput, FailureDetail> {
    validate_package_name(package)?;
    validate_session_id(session_id)?;
    let path = format!("{ANDROID_ARTIFACT_PRIVATE_ROOT}/{session_id}");
    run_checked(
        "android_private_cleanup_failed",
        adb_command(
            adb,
            serial,
            &["shell", "run-as", package, "rm", "-rf", &path],
        ),
    )
}

pub fn remove_private_bootstrap(
    adb: &str,
    serial: &str,
    package: &str,
) -> Result<AndroidCommandOutput, FailureDetail> {
    validate_package_name(package)?;
    run_checked(
        "android_private_cleanup_failed",
        adb_command(
            adb,
            serial,
            &[
                "shell",
                "run-as",
                package,
                "rm",
                "-f",
                ANDROID_BOOTSTRAP_PRIVATE_PATH,
            ],
        ),
    )
}

pub fn read_private_artifact(
    adb: &str,
    serial: &str,
    package: &str,
    session_id: &str,
    device_user_path: &str,
) -> Result<Vec<u8>, FailureDetail> {
    validate_package_name(package)?;
    let relative = private_artifact_relative_path(session_id, device_user_path)?;
    let private_path = format!("{ANDROID_ARTIFACT_PRIVATE_ROOT}/{session_id}/{relative}");
    let command = adb_command(
        adb,
        serial,
        &["exec-out", "run-as", package, "cat", &private_path],
    );
    let output = run_command_bytes(&command, None)
        .map_err(|error| spawn_failure("android_artifact_pull_failed", &command, error))?;
    if output.status != 0 {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(command_failure(
            "android_artifact_pull_failed",
            &command,
            output.status,
            &stderr,
        ));
    }
    Ok(output.stdout)
}

pub fn private_artifact_relative_path(
    session_id: &str,
    device_user_path: &str,
) -> Result<String, FailureDetail> {
    validate_session_id(session_id)?;
    let prefix = format!("{ANDROID_ARTIFACT_USER_ROOT}/{session_id}/");
    let Some(relative) = device_user_path.strip_prefix(&prefix) else {
        return Err(failure(
            "android_artifact_path_invalid",
            None,
            Some(device_user_path.to_string()),
            &format!("path beneath {prefix}"),
            "path is outside the Android session artifact root",
        ));
    };
    if !is_valid_private_relative(relative) {
        return Err(failure(
            "android_artifact_path_invalid",
            None,
            Some(device_user_path.to_string()),
            "normalized artifact path using only ASCII letters, digits, '/', '-', '_' and '.'",
            "invalid relative artifact path",
        ));
    }
    Ok(relative.to_string())
}

/// Reject an already-relative private path unsuitable for embedding in a `run-as`
/// command string. Same charset/traversal rules as [`private_artifact_relative_path`].
fn validate_private_relative_path(relative: &str) -> Result<(), FailureDetail> {
    if is_valid_private_relative(relative) {
        Ok(())
    } else {
        Err(failure(
            "android_artifact_path_invalid",
            None,
            Some(relative.to_string()),
            "normalized artifact path using only ASCII letters, digits, '/', '-', '_' and '.'",
            "invalid relative artifact path",
        ))
    }
}

fn is_valid_private_relative(relative: &str) -> bool {
    let path = Path::new(relative);
    !(relative.is_empty()
        || relative.contains('\\')
        || relative.split('/').any(str::is_empty)
        || !relative
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.'))
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_))))
}

fn validate_session_id(session_id: &str) -> Result<(), FailureDetail> {
    let valid = !session_id.is_empty()
        && session_id.len() <= 128
        && session_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    if valid {
        Ok(())
    } else {
        Err(failure(
            "android_session_id_invalid",
            None,
            None,
            "non-empty ASCII session ID containing only letters, digits, '-' and '_'",
            session_id,
        ))
    }
}

fn adb_command(adb: &str, serial: &str, args: &[&str]) -> Vec<String> {
    let mut command = vec![adb.to_string(), "-s".into(), serial.to_string()];
    command.extend(args.iter().map(|value| (*value).to_string()));
    command
}

fn run_checked(kind: &str, command: Vec<String>) -> Result<AndroidCommandOutput, FailureDetail> {
    let output =
        run_command(&command, None).map_err(|error| spawn_failure(kind, &command, error))?;
    checked_output(kind, output)
}

fn checked_output(
    kind: &str,
    output: AndroidCommandOutput,
) -> Result<AndroidCommandOutput, FailureDetail> {
    if output.status == 0 {
        Ok(output)
    } else {
        Err(command_failure(
            kind,
            &output.command,
            output.status,
            &output.stderr,
        ))
    }
}

fn run_command(
    command: &[String],
    input: Option<&[u8]>,
) -> Result<AndroidCommandOutput, std::io::Error> {
    let output = run_command_bytes(command, input)?;
    Ok(AndroidCommandOutput {
        command: output.command,
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

struct BinaryCommandOutput {
    command: Vec<String>,
    status: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_command_bytes(
    command: &[String],
    input: Option<&[u8]>,
) -> Result<BinaryCommandOutput, std::io::Error> {
    let Some(executable) = command.first() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "empty command",
        ));
    };
    let mut spawn_attempt = 0;
    let mut child = loop {
        let mut child = Command::new(executable);
        child
            .args(command.iter().skip(1))
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        match child.spawn() {
            Ok(child) => break child,
            Err(error) if executable_is_temporarily_busy(&error) && spawn_attempt < 4 => {
                spawn_attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(error) => return Err(error),
        }
    };
    if let Some(input) = input {
        child
            .stdin
            .take()
            .expect("piped stdin is available")
            .write_all(input)?;
    }
    let output = child.wait_with_output()?;
    Ok(BinaryCommandOutput {
        command: command.to_vec(),
        status: output.status.code().unwrap_or(-1),
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

fn executable_is_temporarily_busy(error: &std::io::Error) -> bool {
    #[cfg(unix)]
    {
        error.raw_os_error() == Some(libc::ETXTBSY)
    }
    #[cfg(not(unix))]
    {
        let _ = error;
        false
    }
}

fn command_failure(kind: &str, command: &[String], status: i32, stderr: &str) -> FailureDetail {
    FailureDetail {
        kind: kind.into(),
        command: Some(crate::format_command(command)),
        path: command.first().cloned(),
        message: (!stderr.trim().is_empty()).then(|| stderr.trim().to_string()),
        expected: Some(json!("command exits with status 0")),
        actual: Some(json!(status)),
        ..FailureDetail::default()
    }
}

fn spawn_failure(kind: &str, command: &[String], error: std::io::Error) -> FailureDetail {
    FailureDetail {
        kind: kind.into(),
        command: Some(crate::format_command(command)),
        path: command.first().cloned(),
        message: Some(error.to_string()),
        expected: Some(json!("command starts successfully")),
        actual: Some(json!("OSError")),
        ..FailureDetail::default()
    }
}

fn artifact_failure(path: &Path, error: std::io::Error) -> FailureDetail {
    FailureDetail {
        kind: "artifact_write_failed".into(),
        path: Some(path.to_string_lossy().into_owned()),
        message: Some(error.to_string()),
        expected: Some(json!("artifact path is writable")),
        actual: Some(json!("OSError")),
        ..FailureDetail::default()
    }
}

fn failure(
    kind: &str,
    command: Option<Vec<String>>,
    path: Option<String>,
    expected: &str,
    actual: &str,
) -> FailureDetail {
    FailureDetail {
        kind: kind.into(),
        command: command
            .as_ref()
            .map(|command| crate::format_command(command)),
        path,
        expected: Some(json!(expected)),
        actual: Some(json!(actual)),
        ..FailureDetail::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn parses_adb_devices_with_states_and_descriptions() {
        let devices = parse_adb_devices(
            "List of devices attached\n\
             emulator-5554 device product:sdk model:Pixel_8 transport_id:1\n\
             phone offline transport_id:2\n\
             locked unauthorized usb:3-2\n",
        );
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].serial, "emulator-5554");
        assert_eq!(devices[0].state, "device");
        assert_eq!(
            devices[0].description.get("model").map(String::as_str),
            Some("Pixel_8")
        );
        assert_eq!(devices[1].state, "offline");
        assert_eq!(devices[2].state, "unauthorized");
    }

    #[test]
    fn selects_only_authorized_device_or_requires_an_explicit_serial() {
        let devices = vec![device("first", "device"), device("second", "offline")];
        assert_eq!(
            select_device_from(&devices, None)
                .expect("one authorized device")
                .serial,
            "first"
        );
        assert_eq!(
            select_device_from(&devices, Some("second"))
                .expect_err("offline selection fails")
                .kind,
            "android_device_offline"
        );

        let ambiguous = vec![device("first", "device"), device("second", "device")];
        assert_eq!(
            select_device_from(&ambiguous, None)
                .expect_err("multiple authorized devices fail")
                .kind,
            "android_device_ambiguous"
        );
        assert_eq!(
            select_device_from(&[], None)
                .expect_err("no devices fail")
                .kind,
            "android_device_not_found"
        );
    }

    #[test]
    fn adb_resolution_uses_documented_precedence() {
        let temp = unique_temp_dir();
        let path_dir = temp.join("path");
        let sdk_dir = temp.join("sdk");
        fs::create_dir_all(&path_dir).expect("path dir");
        fs::create_dir_all(sdk_dir.join("platform-tools")).expect("sdk dir");
        let explicit = write_executable(&temp.join("explicit-adb"), "#!/bin/sh\n");
        let env_adb = write_executable(&temp.join("env-adb"), "#!/bin/sh\n");
        let sdk_adb = write_executable(
            &sdk_dir.join("platform-tools").join(adb_leaf()),
            "#!/bin/sh\n",
        );
        let path_adb = write_executable(&path_dir.join(adb_leaf()), "#!/bin/sh\n");
        let path = env::join_paths([path_dir]).expect("PATH");

        assert_eq!(
            resolve_adb_binary_with_env(
                explicit.to_str(),
                Some(env_adb.clone().into_os_string()),
                Some(sdk_dir.clone().into_os_string()),
                None,
                Some(path.clone()),
            )
            .expect("explicit"),
            explicit.to_string_lossy()
        );
        assert_eq!(
            resolve_adb_binary_with_env(
                None,
                Some(env_adb.clone().into_os_string()),
                Some(sdk_dir.clone().into_os_string()),
                None,
                Some(path.clone()),
            )
            .expect("ADB"),
            env_adb.to_string_lossy()
        );
        assert_eq!(
            resolve_adb_binary_with_env(
                None,
                None,
                Some(sdk_dir.into_os_string()),
                None,
                Some(path.clone()),
            )
            .expect("SDK"),
            sdk_adb.to_string_lossy()
        );
        assert_eq!(
            resolve_adb_binary_with_env(None, None, None, None, Some(path)).expect("PATH"),
            path_adb.to_string_lossy()
        );
        fs::remove_dir_all(temp).expect("cleanup");
    }

    #[test]
    fn validates_android_preset_and_resolves_genname() {
        let temp = valid_project(
            r#"[preset.0]
name="Android QA"
platform="Android"
export_filter="all_resources"
export_path="build/game.apk"

[preset.0.options]
gradle_build/export_format=0
permissions/internet=true
package/unique_name="org.example.$genname"
"#,
            "42 My-Game_name!",
        );
        let preset = load_export_preset(&temp, "Android QA", None).expect("valid Android preset");
        assert_eq!(preset.name, "Android QA");
        assert_eq!(preset.package, "org.example.mygame_name");
        assert_eq!(preset.export_path.as_deref(), Some("build/game.apk"));

        let overridden = load_export_preset(&temp, "Android QA", Some("dev.qa.sample"))
            .expect("package override");
        assert_eq!(overridden.package, "dev.qa.sample");
        fs::remove_dir_all(temp).expect("cleanup");
    }

    #[test]
    fn preset_validation_rejects_platform_permission_aab_and_missing_autoload() {
        let cases = [
            (
                "platform=\"Linux\"\nexport_filter=\"all_resources\"",
                "permissions/internet=true\npackage/unique_name=\"dev.qa.app\"",
            ),
            (
                "platform=\"Android\"\nexport_filter=\"all_resources\"",
                "permissions/internet=false\npackage/unique_name=\"dev.qa.app\"",
            ),
            (
                "platform=\"Android\"\nexport_filter=\"all_resources\"",
                "gradle_build/export_format=1\npermissions/internet=true\npackage/unique_name=\"dev.qa.app\"",
            ),
        ];
        for (preset, options) in cases {
            let config = format!(
                "[preset.0]\nname=\"Android QA\"\n{preset}\n\n[preset.0.options]\n{options}\n"
            );
            let temp = valid_project(&config, "Sample");
            assert_eq!(
                load_export_preset(&temp, "Android QA", None)
                    .expect_err("invalid preset")
                    .kind,
                "android_export_preset_invalid"
            );
            fs::remove_dir_all(temp).expect("cleanup");
        }

        let temp = valid_project(
            "[preset.0]\nname=\"Android QA\"\nplatform=\"Android\"\nexport_filter=\"all_resources\"\n\n[preset.0.options]\npermissions/internet=true\npackage/unique_name=\"dev.qa.app\"\n",
            "Sample",
        );
        fs::write(
            temp.join("project.godot"),
            "[application]\nconfig/name=\"Sample\"\n",
        )
        .expect("remove autoload");
        assert_eq!(
            load_export_preset(&temp, "Android QA", None)
                .expect_err("missing autoload")
                .kind,
            "android_export_preset_invalid"
        );
        fs::remove_dir_all(temp).expect("cleanup");
    }

    #[test]
    fn preset_validation_understands_godot_resource_filter_names() {
        for filter in ["scenes", "resources"] {
            let config = format!(
                "[preset.0]\nname=\"Android QA\"\nplatform=\"Android\"\nexport_filter=\"{filter}\"\nexport_files=PackedStringArray(\"res://main.tscn\")\n\n[preset.0.options]\npermissions/internet=true\npackage/unique_name=\"dev.qa.app\"\n"
            );
            let temp = valid_project(&config, "Sample");
            load_export_preset(&temp, "Android QA", None)
                .expect("Godot adds autoload dependencies to selected exports");
            fs::remove_dir_all(temp).expect("cleanup");
        }

        for (filter, extra) in [
            (
                "exclude",
                "export_files=PackedStringArray(\"res://addons/godot_qa/qa_bridge.gd\")",
            ),
            (
                "exclude",
                "export_files=PackedStringArray(\"res://addons\")",
            ),
            (
                "exclude",
                "export_files=PackedStringArray(\"res://addons/godot_qa/qa_live_session_host.gd\")",
            ),
            (
                "customized",
                "customized_files={\"res://addons/godot_qa\": 3}",
            ),
            (
                "customized",
                "customized_files={\"res://addons/godot_qa/qa_live_session_host.gd\": 3}",
            ),
        ] {
            let config = format!(
                "[preset.0]\nname=\"Android QA\"\nplatform=\"Android\"\nexport_filter=\"{filter}\"\n{extra}\n\n[preset.0.options]\npermissions/internet=true\npackage/unique_name=\"dev.qa.app\"\n"
            );
            let temp = valid_project(&config, "Sample");
            assert_eq!(
                load_export_preset(&temp, "Android QA", None)
                    .expect_err("filter removes addon")
                    .kind,
                "android_export_preset_invalid"
            );
            fs::remove_dir_all(temp).expect("cleanup");
        }

        let temp = valid_project(
            "[preset.0]\nname=\"Android QA\"\nplatform=\"Android\"\nexport_filter=\"all_resources\"\nexclude_filter=\"addons/godot_qa/*\"\n\n[preset.0.options]\npermissions/internet=true\npackage/unique_name=\"dev.qa.app\"\n",
            "Sample",
        );
        assert_eq!(
            load_export_preset(&temp, "Android QA", None)
                .expect_err("exclude_filter removes addon")
                .kind,
            "android_export_preset_invalid"
        );
        fs::remove_dir_all(temp).expect("cleanup");
    }

    #[test]
    fn private_artifact_paths_are_confined_to_the_exact_session_root() {
        assert_eq!(
            private_artifact_relative_path(
                "session_1",
                "user://godot_qa/live/session_1/screens/a.png"
            )
            .expect("valid path"),
            "screens/a.png"
        );
        for invalid in [
            "user://godot_qa/live/other/a.png",
            "user://godot_qa/live/session_1/../secret",
            "user://godot_qa/live/session_1//a.png",
            "user://godot_qa/live/session_1/a\\b.png",
            "user://godot_qa/live/session_1/",
            "user://godot_qa/live/session_1/a;cat",
            "user://godot_qa/live/session_1/$(id)",
            "user://godot_qa/live/session_1/a'b",
            "user://godot_qa/live/session_1/a\nb",
        ] {
            assert_eq!(
                private_artifact_relative_path("session_1", invalid)
                    .expect_err("invalid path")
                    .kind,
                "android_artifact_path_invalid"
            );
        }
        assert_eq!(
            private_artifact_relative_path("../bad", "anything")
                .expect_err("invalid session")
                .kind,
            "android_session_id_invalid"
        );
    }

    #[cfg(unix)]
    #[test]
    fn bootstrap_is_streamed_on_stdin_and_never_appears_in_command_metadata() {
        let temp = unique_temp_dir();
        fs::create_dir_all(&temp).expect("temp");
        let args_path = temp.join("args");
        let input_path = temp.join("input");
        let adb = write_executable(
            &temp.join("adb"),
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{}'\ncat > '{}'\n",
                args_path.display(),
                input_path.display()
            ),
        );
        let token = "TOP-SECRET-SESSION-TOKEN";
        let payload = format!("{{\"token\":\"{token}\"}}");
        let output = write_private_bootstrap(
            adb.to_str().expect("adb"),
            "serial-1",
            "dev.qa.sample",
            payload.as_bytes(),
        )
        .expect("bootstrap");

        let args = fs::read_to_string(args_path).expect("args");
        let input = fs::read_to_string(input_path).expect("input");
        assert_eq!(input, payload);
        assert!(!args.contains(token));
        assert!(!format!("{output:?}").contains(token));
        assert!(output.command.contains(
            &"'umask 077; cat > files/godot_qa/android_live_bootstrap.json'".to_string()
        ));
        fs::remove_dir_all(temp).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn bootstrap_failure_does_not_echo_token_from_adb_stderr() {
        let temp = unique_temp_dir();
        fs::create_dir_all(&temp).expect("temp");
        let adb = write_executable(
            &temp.join("adb"),
            "#!/bin/sh\ncase \"$*\" in *'cat >'*) payload=$(cat); printf '%s' \"$payload\" >&2; exit 9;; esac\n",
        );
        let token = "TOP-SECRET-FAILURE-TOKEN";
        let payload = format!("{{\"token\":\"{token}\"}}");
        let failure = write_private_bootstrap(
            adb.to_str().expect("adb"),
            "serial-1",
            "dev.qa.sample",
            payload.as_bytes(),
        )
        .expect_err("bootstrap write must fail");

        assert_eq!(failure.kind, "android_bootstrap_write_failed");
        assert!(!serde_json::to_string(&failure).unwrap().contains(token));
        assert!(!format!("{failure:?}").contains(token));
        fs::remove_dir_all(temp).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn private_file_streams_bytes_via_stdin_makes_parent_and_blanks_output() {
        let temp = unique_temp_dir();
        fs::create_dir_all(&temp).expect("temp");
        let args_path = temp.join("args");
        let input_path = temp.join("input");
        let adb = write_executable(
            &temp.join("adb"),
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{}'\ncase \"$*\" in *umask*) cat > '{}';; esac\n",
                args_path.display(),
                input_path.display()
            ),
        );
        let payload = b"private-artifact-bytes";
        let output = write_private_file(
            adb.to_str().expect("adb"),
            "serial-1",
            "dev.qa.sample",
            "session-1",
            "requests/req-1.json",
            payload,
        )
        .expect("private file write");

        assert!(output.stdout.is_empty() && output.stderr.is_empty());
        let args = fs::read_to_string(&args_path).expect("args");
        // The parent directory is created before the file is streamed.
        assert!(args.contains("mkdir"));
        assert!(args.contains("files/godot_qa/live/session-1/requests"));
        // The full private path is embedded in the single-quoted cat command.
        assert!(output.command.contains(
            &"'umask 077; cat > files/godot_qa/live/session-1/requests/req-1.json'".to_string()
        ));
        assert_eq!(fs::read(&input_path).expect("input"), payload);
        fs::remove_dir_all(temp).expect("cleanup");
    }

    #[test]
    fn private_file_and_dirs_reject_unsafe_relative_paths() {
        for invalid in [
            "../escape",
            "/abs/path",
            "",
            "a;b",
            "nested/../x",
            "a\\b",
            "a//b",
        ] {
            assert_eq!(
                write_private_file(
                    "adb",
                    "serial-1",
                    "dev.qa.sample",
                    "session-1",
                    invalid,
                    b"x"
                )
                .expect_err("unsafe relative rejected")
                .kind,
                "android_artifact_path_invalid",
                "{invalid}"
            );
            assert_eq!(
                make_private_dirs("adb", "serial-1", "dev.qa.sample", "session-1", &[invalid])
                    .expect_err("unsafe dir rejected")
                    .kind,
                "android_artifact_path_invalid",
                "{invalid}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn make_private_dirs_issues_one_scoped_mkdir_command() {
        let temp = unique_temp_dir();
        fs::create_dir_all(&temp).expect("temp");
        let calls = temp.join("calls");
        let adb = write_executable(
            &temp.join("adb"),
            &format!("#!/bin/sh\nprintf '%s|' \"$@\" >> '{}'\n", calls.display()),
        );
        make_private_dirs(
            adb.to_str().expect("adb"),
            "device-1",
            "dev.qa.sample",
            "session-1",
            &["requests", "responses", "progress"],
        )
        .expect("make private dirs");
        assert_eq!(
            fs::read_to_string(calls).expect("calls"),
            "-s|device-1|shell|run-as|dev.qa.sample|mkdir|-p|files/godot_qa/live/session-1/requests|files/godot_qa/live/session-1/responses|files/godot_qa/live/session-1/progress|"
        );
        fs::remove_dir_all(temp).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn adb_primitives_use_exact_serial_scoped_commands() {
        let temp = unique_temp_dir();
        fs::create_dir_all(&temp).expect("temp");
        let calls = temp.join("calls");
        let adb = write_executable(
            &temp.join("adb"),
            &format!(
                "#!/bin/sh\nprintf '%s|' \"$@\" >> '{0}'\nprintf '\\n' >> '{0}'\ncase \"$*\" in *'forward --list'*) printf 'device-1 tcp:3210 tcp:4567\\n';; esac\n",
                calls.display(),
            ),
        );
        let apk = temp.join("app.apk");
        fs::write(&apk, b"apk").expect("apk");
        let adb = adb.to_str().expect("adb");

        install_apk(adb, "device-1", &apk).expect("install");
        create_forward(adb, "device-1", 3210, 4567).expect("forward");
        remove_exact_forward(adb, "device-1", 3210, 4567).expect("remove forward");
        force_stop(adb, "device-1", "dev.qa.sample").expect("force stop");
        remove_private_bootstrap(adb, "device-1", "dev.qa.sample").expect("remove bootstrap");
        remove_private_session(adb, "device-1", "dev.qa.sample", "session-1")
            .expect("remove session");

        let calls = fs::read_to_string(calls).expect("calls");
        assert!(calls.contains(&format!("-s|device-1|install|-r|{}|", apk.display())));
        assert!(calls.contains("-s|device-1|forward|--no-rebind|tcp:3210|tcp:4567|"));
        assert!(calls.contains("-s|device-1|forward|--remove|tcp:3210|"));
        assert!(calls.contains("-s|device-1|shell|am|force-stop|dev.qa.sample|"));
        assert!(calls.contains(
            "-s|device-1|shell|run-as|dev.qa.sample|rm|-f|files/godot_qa/android_live_bootstrap.json|"
        ));
        assert!(calls.contains(
            "-s|device-1|shell|run-as|dev.qa.sample|rm|-rf|files/godot_qa/live/session-1|"
        ));
        fs::remove_dir_all(temp).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn wireless_reconnect_uses_adb_connect_without_device_scoping() {
        let temp = unique_temp_dir();
        fs::create_dir_all(&temp).expect("temp");
        let calls = temp.join("calls");
        let adb = write_executable(
            &temp.join("adb"),
            &format!("#!/bin/sh\nprintf '%s|' \"$@\" > '{}'\n", calls.display()),
        );
        reconnect_wireless_device(adb.to_str().expect("adb"), "192.168.0.123:5555")
            .expect("reconnect");
        assert_eq!(
            fs::read_to_string(&calls).expect("calls"),
            "connect|192.168.0.123:5555|"
        );
        reconnect_wireless_device(
            adb.to_str().expect("adb"),
            "adb-DEVICE._adb-tls-connect._tcp",
        )
        .expect("mDNS reconnect");
        assert_eq!(
            fs::read_to_string(&calls).expect("calls"),
            "connect|adb-DEVICE._adb-tls-connect._tcp|"
        );
        assert_eq!(
            reconnect_wireless_device(adb.to_str().expect("adb"), "emulator-5554")
                .expect_err("non-network serial")
                .kind,
            "android_device_unavailable"
        );
        fs::remove_dir_all(temp).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn removing_an_already_missing_exact_forward_is_idempotent() {
        let temp = unique_temp_dir();
        fs::create_dir_all(&temp).expect("temp");
        let adb = write_executable(
            &temp.join("adb"),
            "#!/bin/sh\nprintf \"adb: error: listener 'tcp:3210' not found\\n\" >&2\nexit 1\n",
        );
        let output =
            remove_forward(adb.to_str().expect("adb"), "device-1", 3210).expect("idempotent");
        assert_eq!(output.status, 1);

        let wrong_listener = write_executable(
            &temp.join("wrong-listener-adb"),
            "#!/bin/sh\nprintf \"adb: error: listener 'tcp:9999' not found\\n\" >&2\nexit 1\n",
        );
        assert_eq!(
            remove_forward(wrong_listener.to_str().expect("adb"), "device-1", 3210)
                .expect_err("wrong listener must remain an error")
                .kind,
            "android_forward_remove_failed"
        );
        fs::remove_dir_all(temp).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn exact_forward_cleanup_preserves_a_different_mapping() {
        let temp = unique_temp_dir();
        fs::create_dir_all(&temp).expect("temp");
        let calls = temp.join("calls");
        let adb = write_executable(
            &temp.join("adb"),
            &format!(
                "#!/bin/sh\nprintf '%s|' \"$@\" >> '{0}'\nprintf '\\n' >> '{0}'\nprintf 'device-1 tcp:3210 tcp:9999\\n'\n",
                calls.display()
            ),
        );
        assert!(
            remove_exact_forward(adb.to_str().unwrap(), "device-1", 3210, 4567)
                .unwrap()
                .is_none()
        );
        assert!(!fs::read_to_string(calls).unwrap().contains("--remove|"));
        fs::remove_dir_all(temp).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn activity_launch_uses_implicit_then_godot_component_fallback() {
        let temp = unique_temp_dir();
        fs::create_dir_all(&temp).expect("temp");
        let calls = temp.join("calls");
        let adb = write_executable(
            &temp.join("adb"),
            &format!(
                "#!/bin/sh\nprintf '%s|' \"$@\" >> '{0}'\nprintf '\\n' >> '{0}'\ncase \"$*\" in *android.intent.category.LAUNCHER*) echo 'Error: Activity not started'; exit 1;; esac\n",
                calls.display()
            ),
        );
        let result = launch_app(adb.to_str().expect("adb"), "device-1", "dev.qa.sample")
            .expect("explicit fallback");
        assert!(result.used_explicit_fallback);
        assert!(result.launch.command.ends_with(&[
            "shell".into(),
            "am".into(),
            "start".into(),
            "-n".into(),
            "dev.qa.sample/com.godot.game.GodotAppLauncher".into()
        ]));
        let calls = fs::read_to_string(calls).expect("calls");
        assert_eq!(calls.lines().count(), 3);
        assert!(
            calls
                .lines()
                .nth(1)
                .is_some_and(|line| line.contains("-p|dev.qa.sample|")
                    && line.contains("android.intent.category.LAUNCHER|"))
        );
        fs::remove_dir_all(temp).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn activity_launch_fallback_supports_godot_45_component() {
        let temp = unique_temp_dir();
        fs::create_dir_all(&temp).expect("temp");
        let calls = temp.join("calls");
        let adb = write_executable(
            &temp.join("adb"),
            &format!(
                "#!/bin/sh\nprintf '%s|' \"$@\" >> '{0}'\nprintf '\\n' >> '{0}'\ncase \"$*\" in *android.intent.category.LAUNCHER*|*GodotAppLauncher*) echo 'Error: Activity not started'; exit 1;; esac\n",
                calls.display()
            ),
        );
        let result = launch_app(adb.to_str().expect("adb"), "device-1", "dev.qa.sample")
            .expect("GodotApp fallback");
        assert!(result.used_explicit_fallback);
        assert!(result.launch.command.ends_with(&[
            "shell".into(),
            "am".into(),
            "start".into(),
            "-n".into(),
            "dev.qa.sample/com.godot.game.GodotApp".into()
        ]));
        assert_eq!(fs::read_to_string(calls).unwrap().lines().count(), 4);
        fs::remove_dir_all(temp).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn pid_query_distinguishes_stopped_app_from_unavailable_device() {
        let temp = unique_temp_dir();
        fs::create_dir_all(&temp).expect("temp");
        let adb = write_executable(
            &temp.join("adb"),
            "#!/bin/sh\ncase \"$2\" in stopped) exit 1;; offline) printf 'error: device offline\\n' >&2; exit 1;; missing) printf \"adb: device 'missing' not found\\n\" >&2; exit 1;; running) printf '4321\\n';; esac\n",
        );
        let adb = adb.to_str().expect("adb");
        assert_eq!(query_pid(adb, "stopped", "dev.qa.sample").unwrap(), None);
        assert_eq!(
            query_pid(adb, "running", "dev.qa.sample").unwrap(),
            Some(4321)
        );
        for serial in ["offline", "missing"] {
            assert_eq!(
                query_pid(adb, serial, "dev.qa.sample")
                    .expect_err("unavailable device must not look stopped")
                    .kind,
                "android_pid_query_failed"
            );
        }
        fs::remove_dir_all(temp).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn artifact_pull_preserves_binary_bytes() {
        let temp = unique_temp_dir();
        fs::create_dir_all(&temp).expect("temp");
        let adb = write_executable(
            &temp.join("adb"),
            "#!/bin/sh\nprintf '\\001\\000\\177\\377'\n",
        );
        let bytes = read_private_artifact(
            adb.to_str().expect("adb"),
            "device-1",
            "dev.qa.sample",
            "session-1",
            "user://godot_qa/live/session-1/image.bin",
        )
        .expect("binary artifact");
        assert_eq!(bytes, vec![1, 0, 127, 255]);
        fs::remove_dir_all(temp).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn debug_export_captures_logs_and_requires_created_apk() {
        let temp = unique_temp_dir();
        fs::create_dir_all(&temp).expect("temp");
        let godot = write_executable(
            &temp.join("godot"),
            "#!/bin/sh\nfor last; do :; done\nprintf apk > \"$last\"\necho exported\necho detail >&2\n",
        );
        let apk = temp.join("out/app.apk");
        fs::create_dir_all(apk.parent().expect("parent")).expect("apk parent");
        let logs = temp.join("logs/export.log");
        let result = export_debug_apk(
            godot.to_str().expect("godot"),
            &temp,
            "Android QA",
            &apk,
            &logs,
        )
        .expect("export");
        assert_eq!(result.status, 0);
        assert_eq!(fs::read(&apk).expect("apk"), b"apk");
        assert_eq!(
            fs::read_to_string(&logs).expect("logs"),
            "exported\ndetail\n"
        );
        fs::remove_dir_all(temp).expect("cleanup");
    }

    fn device(serial: &str, state: &str) -> AndroidDevice {
        AndroidDevice {
            serial: serial.into(),
            state: state.into(),
            description: BTreeMap::new(),
        }
    }

    fn valid_project(export_presets: &str, project_name: &str) -> PathBuf {
        let temp = unique_temp_dir();
        fs::create_dir_all(temp.join("addons/godot_qa")).expect("addon dir");
        fs::write(temp.join("addons/godot_qa/qa_bridge.gd"), "extends Node\n").expect("bridge");
        fs::write(temp.join("export_presets.cfg"), export_presets).expect("presets");
        fs::write(
            temp.join("project.godot"),
            format!(
                "[application]\nconfig/name={}\n\n[autoload]\nGodotQaBridge=\"*res://addons/godot_qa/qa_bridge.gd\"\n",
                serde_json::to_string(project_name).expect("project name")
            ),
        )
        .expect("project");
        temp
    }

    fn unique_temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let count = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        env::temp_dir().join(format!(
            "godot-qa-android-process-{}-{nanos}-{count}",
            std::process::id()
        ))
    }

    fn write_executable(path: &Path, contents: &str) -> PathBuf {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("executable parent");
        }
        fs::write(path, contents).expect("write executable");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(path).expect("metadata").permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(path, permissions).expect("executable permissions");
        }
        path.to_path_buf()
    }
}
