#![allow(unused_imports)]

use crate::args::*;
use crate::*;
use godot_qa_core::config::{ProjectConfig, load_project_config};
use godot_qa_core::results::{CommandResult, FailureDetail};
use godot_qa_runtime as runtime;
use godot_qa_scenario::{
    artifact_query, artifact_summary,
    audio::{AudioAnalysisOptions, analyze_wav_file_with_options},
    bundle as scenario_bundle, check as scenario_check, contact_sheet as scenario_contact_sheet,
    edit as scenario_edit, init as scenario_init, lint as scenario_lint, live_gate,
    record as scenario_record, review as scenario_review, run as scenario_run, scenario_doc,
    screenshots as scenario_screenshots, time_control, visual_audit,
};
use notify::{Config as NotifyConfig, PollWatcher, RecursiveMode, Watcher};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub(crate) fn run_session_watch(cli: &Cli, args: &SessionWatchArgs) -> Result<i32, i32> {
    let config = runtime_config(cli);
    let status = runtime::session_status_result(&config);
    if !status.ok {
        print_result(&status, cli.json).map_err(|_| 1)?;
        return Ok(status.exit_code);
    }
    if status.data.get("target").and_then(Value::as_str) == Some("android") {
        let result = watch_setup_result(
            "android_watch_unsupported",
            "Android live sessions do not support host filesystem watch or hot reload",
            json!("desktop session started with --hot-reload"),
            json!("android"),
        );
        print_result(&result, cli.json).map_err(|_| 1)?;
        return Ok(result.exit_code);
    }

    let (tx, rx) = mpsc::channel();
    let native_tx = tx.clone();
    let (_watcher, watched_dir_count, watch_backend, fallback_reason) =
        match notify::recommended_watcher(move |event| {
            // A dropped send means the watch loop's receiver is gone, i.e. the
            // watch session is shutting down. That is expected teardown, so we
            // deliberately ignore it here rather than log per-event noise.
            let _ = native_tx.send(event);
        }) {
            Ok(mut watcher) => match watch_project_dirs(&mut watcher, &config.project_root) {
                Ok(count) => (Box::new(watcher) as Box<dyn Watcher>, count, "native", None),
                Err(native_error) => {
                    match create_polling_watch_backend(tx.clone(), &config.project_root) {
                        Ok((watcher, count)) => (watcher, count, "poll", Some(native_error)),
                        Err(poll_error) => {
                            let result = watch_setup_result(
                                "watch_setup_failed",
                                &poll_error,
                                json!("native filesystem watcher or polling fallback"),
                                json!(config.project_root),
                            );
                            print_result(&result, cli.json).map_err(|_| 1)?;
                            return Ok(result.exit_code);
                        }
                    }
                }
            },
            Err(native_error) => {
                match create_polling_watch_backend(tx.clone(), &config.project_root) {
                    Ok((watcher, count)) => {
                        (watcher, count, "poll", Some(native_error.to_string()))
                    }
                    Err(poll_error) => {
                        let result = watch_setup_result(
                            "watch_setup_failed",
                            &poll_error,
                            json!("native filesystem watcher or polling fallback"),
                            json!("notify error"),
                        );
                        print_result(&result, cli.json).map_err(|_| 1)?;
                        return Ok(result.exit_code);
                    }
                }
            }
        };

    if watched_dir_count == 0 {
        let result = watch_setup_result(
            "watch_setup_failed",
            "no project directories were available to watch",
            json!("at least one non-ignored project directory"),
            json!(config.project_root),
        );
        print_result(&result, cli.json).map_err(|_| 1)?;
        return Ok(result.exit_code);
    }

    let running = Arc::new(AtomicBool::new(true));
    let signal_running = Arc::clone(&running);
    if let Err(error) = ctrlc::set_handler(move || {
        signal_running.store(false, Ordering::SeqCst);
    }) {
        let result = watch_setup_result(
            "watch_setup_failed",
            &error.to_string(),
            json!("Ctrl+C signal handler"),
            json!("handler_unavailable"),
        );
        print_result(&result, cli.json).map_err(|_| 1)?;
        return Ok(result.exit_code);
    }

    let hot_reload_available = runtime::debug_host_available(&config);
    write_watch_event(
        cli.json,
        "watch_started",
        "live session watch started",
        Map::from_iter([
            (
                "project_root".to_string(),
                json!(config.project_root.to_string_lossy()),
            ),
            ("debounce_ms".to_string(), json!(args.debounce_ms)),
            ("watched_dirs".to_string(), json!(watched_dir_count)),
            ("backend".to_string(), json!(watch_backend)),
            ("fallback_reason".to_string(), json!(fallback_reason)),
            ("hot_reload".to_string(), json!(hot_reload_available)),
        ]),
        Vec::new(),
    )
    .map_err(|_| 1)?;

    let debounce = Duration::from_millis(args.debounce_ms);
    let mut pending_paths: BTreeSet<String> = BTreeSet::new();
    let mut reload_deadline: Option<Instant> = None;
    // The resource-reload caveat is emitted at most once per watch session.
    let mut resource_caveat_emitted = false;

    while running.load(Ordering::SeqCst) {
        let timeout = reload_deadline
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
            .unwrap_or_else(|| Duration::from_millis(100));
        match rx.recv_timeout(timeout) {
            Ok(Ok(event)) => {
                let paths = relevant_watch_paths(&config.project_root, &event.paths);
                if !paths.is_empty() {
                    pending_paths.extend(paths);
                    reload_deadline = Some(Instant::now() + debounce);
                }
            }
            Ok(Err(error)) => {
                write_watch_event(
                    cli.json,
                    "reload_failed",
                    "file watcher reported an error",
                    Map::new(),
                    vec![FailureDetail {
                        kind: "watch_event_failed".to_string(),
                        command: Some("session watch".to_string()),
                        message: Some(error.to_string()),
                        expected: Some(json!("native filesystem watch events")),
                        actual: Some(json!("notify error")),
                        ..FailureDetail::default()
                    }],
                )
                .map_err(|_| 1)?;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break;
            }
        }

        if reload_deadline.is_some_and(|deadline| Instant::now() >= deadline)
            && !pending_paths.is_empty()
        {
            let changed_paths: Vec<String> = pending_paths.iter().cloned().collect();
            write_watch_event(
                cli.json,
                "change_detected",
                "project files changed",
                Map::from_iter([("paths".to_string(), json!(changed_paths))]),
                Vec::new(),
            )
            .map_err(|_| 1)?;
            dispatch_watch_reloads(
                &config,
                cli.json,
                hot_reload_available,
                &changed_paths,
                &mut resource_caveat_emitted,
            )
            .map_err(|_| 1)?;
            pending_paths.clear();
            reload_deadline = None;
        }
    }

    write_watch_event(
        cli.json,
        "watch_stopped",
        "live session watch stopped",
        Map::new(),
        Vec::new(),
    )
    .map_err(|_| 1)?;
    Ok(0)
}

/// Classify changed files and apply the most faithful reload for each class.
/// `.gd` -> engine-faithful script soft reload; resource types -> live resource reload;
/// `.tscn` (and anything else) -> full-scene reload. Without a debug host, every change
/// falls back to a full-scene reload (the pre-hot-reload behavior).
fn dispatch_watch_reloads(
    config: &ProjectConfig,
    as_json: bool,
    hot_reload_available: bool,
    changed_paths: &[String],
    resource_caveat_emitted: &mut bool,
) -> io::Result<()> {
    if !hot_reload_available {
        return emit_reload(
            config,
            as_json,
            "full_scene",
            changed_paths,
            runtime::session_reload_result(config),
        );
    }

    let mut scripts = Vec::new();
    let mut resources = Vec::new();
    let mut structure = Vec::new();
    let mut full = Vec::new();
    for path in changed_paths {
        match reload_class_for_path(path) {
            ReloadClass::Script => scripts.push(to_res_path(&config.project_root, path)),
            ReloadClass::Resource => resources.push(to_res_path(&config.project_root, path)),
            ReloadClass::Structure => structure.push(to_res_path(&config.project_root, path)),
            ReloadClass::FullScene => full.push(path.clone()),
        }
    }

    if !scripts.is_empty() {
        let result = runtime::session_reload_scripts_result(config, &scripts);
        emit_reload(config, as_json, "reload_scripts", changed_paths, result)?;
    }
    if !resources.is_empty() {
        if !*resource_caveat_emitted {
            write_watch_event(
                as_json,
                "resource_reload_caveat",
                runtime::RESOURCE_RELOAD_CAVEAT,
                Map::new(),
                Vec::new(),
            )?;
            *resource_caveat_emitted = true;
        }
        let result = runtime::session_reload_resources_result(config, &resources);
        emit_reload(config, as_json, "reload_resources", changed_paths, result)?;
    }
    if !structure.is_empty() {
        let result = runtime::session_reload_structure_result(config, &structure);
        // Scenes whose delta couldn't be expressed as live edits fall back to a
        // full-scene reload (state is not preserved) — reported under `fell_back`.
        let fell_back: Vec<String> = result
            .data
            .get("fell_back")
            .and_then(|value| value.as_array())
            .map(|reasons| {
                reasons
                    .iter()
                    .filter_map(|reason| reason.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        emit_reload(config, as_json, "reload_structure", &structure, result)?;
        if !fell_back.is_empty() {
            write_watch_event(
                as_json,
                "reload_notice",
                "some scene changes could not be applied live; falling back to a full-scene reload (state is not preserved)",
                Map::from_iter([("reasons".to_string(), json!(fell_back))]),
                Vec::new(),
            )?;
            let result = runtime::session_reload_result(config);
            emit_reload(config, as_json, "full_scene", &structure, result)?;
        }
    }
    if !full.is_empty() {
        write_watch_event(
            as_json,
            "reload_notice",
            "scene structure changed; falling back to full-scene reload (state is not preserved)",
            Map::from_iter([("paths".to_string(), json!(full))]),
            Vec::new(),
        )?;
        let result = runtime::session_reload_result(config);
        emit_reload(config, as_json, "full_scene", &full, result)?;
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ReloadClass {
    Script,
    Resource,
    /// A `.tscn` text scene → faithful structural live-edit (with full-scene fallback).
    Structure,
    FullScene,
}

pub(crate) fn reload_class_for_path(path: &str) -> ReloadClass {
    let extension = Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "gd" => ReloadClass::Script,
        "tscn" => ReloadClass::Structure,
        "tres" | "res" | "gdshader" | "shader" | "scn" => ReloadClass::Resource,
        _ => ReloadClass::FullScene,
    }
}

fn emit_reload(
    _config: &ProjectConfig,
    as_json: bool,
    kind: &str,
    paths: &[String],
    mut result: CommandResult,
) -> io::Result<()> {
    if result.ok {
        result.data.insert("reload_kind".to_string(), json!(kind));
        result.data.insert("paths".to_string(), json!(paths));
        write_watch_event(
            as_json,
            "reload_succeeded",
            "hot reload applied",
            result.data,
            Vec::new(),
        )
    } else {
        write_watch_event(
            as_json,
            "reload_failed",
            "hot reload failed",
            Map::from_iter([
                ("reload_kind".to_string(), json!(kind)),
                ("paths".to_string(), json!(paths)),
            ]),
            result.failures,
        )
    }
}

pub(crate) fn watch_setup_result(
    kind: &str,
    message: &str,
    expected: Value,
    actual: Value,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: "session watch".to_string(),
        message: "live session watch failed".to_string(),
        data: Map::new(),
        failures: vec![FailureDetail {
            kind: kind.to_string(),
            command: Some("session watch".to_string()),
            message: Some(message.to_string()),
            expected: Some(expected),
            actual: Some(actual),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}

pub(crate) fn create_polling_watch_backend(
    tx: mpsc::Sender<notify::Result<notify::Event>>,
    project_root: &Path,
) -> Result<(Box<dyn Watcher>, usize), String> {
    let poll_tx = tx.clone();
    let mut watcher = PollWatcher::new(
        move |event| {
            // See the native-watcher note: a dropped send is the watch loop
            // shutting down (receiver gone), so ignoring it is the intended path.
            let _ = poll_tx.send(event);
        },
        NotifyConfig::default().with_poll_interval(Duration::from_millis(250)),
    )
    .map_err(|error| error.to_string())?;
    let count = watch_project_dirs(&mut watcher, project_root)?;
    Ok((Box::new(watcher), count))
}

pub(crate) fn write_watch_event(
    as_json: bool,
    event: &str,
    message: &str,
    data: Map<String, Value>,
    failures: Vec<FailureDetail>,
) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    if as_json {
        serde_json::to_writer(
            &mut stdout,
            &json!({
                "event": event,
                "ok": failures.is_empty(),
                "command": "session watch",
                "message": message,
                "data": data,
                "failures": failures,
            }),
        )?;
        writeln!(stdout)?;
        return Ok(());
    }
    writeln!(stdout, "[watch] {event}: {message}")?;
    if !failures.is_empty() {
        writeln!(stdout, "Failures:")?;
        for failure in failures {
            writeln!(stdout, "- {}", serde_json::to_string(&failure)?)?;
        }
    }
    Ok(())
}

pub(crate) fn relevant_watch_paths(
    project_root: &Path,
    paths: &[std::path::PathBuf],
) -> Vec<String> {
    paths
        .iter()
        .filter(|path| is_relevant_watch_path(project_root, path))
        .map(|path| display_watch_path(project_root, path))
        .collect()
}

pub(crate) fn watch_project_dirs<W: Watcher>(
    watcher: &mut W,
    project_root: &Path,
) -> Result<usize, String> {
    let mut watched_count = 0;
    watch_dir_recursive(watcher, project_root, project_root, &mut watched_count)?;
    Ok(watched_count)
}

pub(crate) fn watch_dir_recursive<W: Watcher>(
    watcher: &mut W,
    project_root: &Path,
    directory: &Path,
    watched_count: &mut usize,
) -> Result<(), String> {
    if is_ignored_watch_path(project_root, directory) {
        return Ok(());
    }
    watcher
        .watch(directory, RecursiveMode::NonRecursive)
        .map_err(|error| error.to_string())?;
    *watched_count += 1;
    let entries = std::fs::read_dir(directory).map_err(|error| error.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            watch_dir_recursive(watcher, project_root, &path, watched_count)?;
        }
    }
    Ok(())
}

/// Normalize a user- or watcher-supplied path to a `res://` path for the reload
/// protocol. Already-`res://` inputs pass through; filesystem paths are made relative
/// to the project root.
pub(crate) fn to_res_path(project_root: &Path, raw: &str) -> String {
    if raw.starts_with("res://") {
        return raw.to_string();
    }
    let path = Path::new(raw);
    let relative = path.strip_prefix(project_root).unwrap_or(path);
    let normalized = relative.to_string_lossy().replace('\\', "/");
    format!("res://{}", normalized.trim_start_matches('/'))
}

pub(crate) fn display_watch_path(project_root: &Path, path: &Path) -> String {
    path.strip_prefix(project_root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

pub(crate) fn is_relevant_watch_path(project_root: &Path, path: &Path) -> bool {
    if is_ignored_watch_path(project_root, path) {
        return false;
    }
    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return false;
    };
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "gd" | "tscn" | "scn" | "tres" | "res" | "shader" | "gdshader"
    )
}

pub(crate) fn is_ignored_watch_path(project_root: &Path, path: &Path) -> bool {
    let relative = path.strip_prefix(project_root).unwrap_or(path);
    let components: Vec<String> = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect();
    if components.iter().any(|component| {
        matches!(
            component.as_str(),
            ".git" | ".godot" | ".qa" | "target" | "build" | "dist"
        )
    }) {
        return true;
    }
    components
        .windows(2)
        .any(|window| window[0] == "qa" && matches!(window[1].as_str(), "artifacts" | "tmp"))
}

#[cfg(test)]
pub(crate) fn coalesced_reload_count(event_times_ms: &[u64], debounce_ms: u64) -> usize {
    let mut count = 0;
    let mut deadline: Option<u64> = None;
    for event_time in event_times_ms {
        if deadline.is_some_and(|deadline| *event_time >= deadline) {
            count += 1;
        }
        deadline = Some(*event_time + debounce_ms);
    }
    if deadline.is_some() {
        count += 1;
    }
    count
}
