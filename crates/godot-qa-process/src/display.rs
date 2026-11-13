use godot_qa_core::project_file::read_project_godot_cached;
use serde_json::json;
use std::env;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct DisplayLaunchMetadata {
    pub headless: bool,
    pub selected_mode: String,
    pub platform: String,
    pub linuxbsd_target: bool,
    pub x11_display_available: bool,
    pub wayland_display_available: bool,
    pub selected_display_driver: Option<String>,
    pub project_prefers_wayland: bool,
    pub requested_display_driver: Option<String>,
    pub virtual_display_used: bool,
    pub virtual_display_command: Option<Vec<String>>,
    pub fallback_reason: Option<String>,
    pub launch_prefix_used: bool,
    pub joypad_isolation: bool,
    pub windowed_launch_prefix_used: bool,
    pub effective_command_prefix: Vec<String>,
    pub display_backend_available: bool,
    pub startup_attempted: bool,
    pub virtual_display_exit_code: Option<i32>,
    pub startup_failure_kind: Option<String>,
    pub startup_failure_retryable: bool,
    pub startup_failure_guidance: Option<String>,
}

pub fn resolve_project_display_launch_metadata(
    project_root: &Path,
    launch_prefix: &[String],
    headless: bool,
) -> DisplayLaunchMetadata {
    build_project_display_launch_metadata(project_root, headless, launch_prefix)
}

pub fn resolve_project_display_launch_metadata_with_windowed_prefix(
    project_root: &Path,
    launch_prefix: &[String],
    windowed_launch_prefix: &[String],
    headless: bool,
    joypad_isolation: bool,
) -> DisplayLaunchMetadata {
    let mut display = build_project_display_launch_metadata_with_windowed_prefix(
        project_root,
        headless,
        launch_prefix,
        windowed_launch_prefix,
    );
    display.joypad_isolation = joypad_isolation;
    display
}

pub(super) fn append_project_display_driver_args(
    command: &mut Vec<String>,
    project_root: &Path,
    launch_prefix: &[String],
    windowed_launch_prefix: &[String],
    headless: bool,
) -> DisplayLaunchMetadata {
    append_project_display_driver_args_with_env(
        command,
        project_root,
        launch_prefix,
        windowed_launch_prefix,
        headless,
        env_flag_available("DISPLAY"),
        env_flag_available("WAYLAND_DISPLAY"),
        env::var_os("PATH"),
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn append_project_display_driver_args_with_env(
    command: &mut Vec<String>,
    project_root: &Path,
    launch_prefix: &[String],
    windowed_launch_prefix: &[String],
    headless: bool,
    x11_display_available: bool,
    wayland_display_available: bool,
    path: Option<std::ffi::OsString>,
) -> DisplayLaunchMetadata {
    let display = build_project_display_launch_metadata_with_env(
        project_root,
        headless,
        launch_prefix,
        windowed_launch_prefix,
        x11_display_available,
        wayland_display_available,
        path,
    );
    if let Some(display_driver) = display.requested_display_driver.as_deref() {
        command.push("--display-driver".to_string());
        command.push(display_driver.to_string());
    }
    display
}

fn launch_prefix_uses_virtual_x11_display(launch_prefix: &[String]) -> bool {
    launch_prefix.iter().any(|entry| {
        let entry = entry.trim();
        Path::new(entry)
            .file_stem()
            .is_some_and(|name| name == "xvfb-run")
    })
}

pub(super) fn build_project_display_launch_metadata(
    project_root: &Path,
    headless: bool,
    launch_prefix: &[String],
) -> DisplayLaunchMetadata {
    build_project_display_launch_metadata_with_windowed_prefix(
        project_root,
        headless,
        launch_prefix,
        &[],
    )
}

fn build_project_display_launch_metadata_with_windowed_prefix(
    project_root: &Path,
    headless: bool,
    launch_prefix: &[String],
    windowed_launch_prefix: &[String],
) -> DisplayLaunchMetadata {
    build_project_display_launch_metadata_with_env(
        project_root,
        headless,
        launch_prefix,
        windowed_launch_prefix,
        env_flag_available("DISPLAY"),
        env_flag_available("WAYLAND_DISPLAY"),
        env::var_os("PATH"),
    )
}

fn build_project_display_launch_metadata_with_env(
    project_root: &Path,
    headless: bool,
    launch_prefix: &[String],
    windowed_launch_prefix: &[String],
    x11_display_available: bool,
    wayland_display_available: bool,
    path: Option<std::ffi::OsString>,
) -> DisplayLaunchMetadata {
    let project_prefers_wayland = project_wayland_preference_enabled(project_root);
    let linuxbsd_target = linuxbsd_target();
    let launch_prefix_used = !launch_prefix.is_empty();
    let windowed_launch_prefix_used = !headless && !windowed_launch_prefix.is_empty();
    let configured_virtual_display = launch_prefix_uses_virtual_x11_display(launch_prefix)
        || (!headless && launch_prefix_uses_virtual_x11_display(windowed_launch_prefix));
    let wayland_usable = wayland_display_available && !configured_virtual_display;
    let selected_mode = if headless { "headless" } else { "windowed" }.to_string();
    let auto_virtual_display_command = (!headless
        && linuxbsd_target
        && !x11_display_available
        && !wayland_usable
        && !windowed_launch_prefix_used
        && !configured_virtual_display)
        .then(|| detect_xvfb_run(path.as_deref()))
        .flatten();
    let virtual_display_used = configured_virtual_display || auto_virtual_display_command.is_some();
    let mut effective_command_prefix = Vec::new();
    effective_command_prefix.extend(launch_prefix.iter().cloned());
    if !headless {
        effective_command_prefix.extend(windowed_launch_prefix.iter().cloned());
    }
    if let Some(virtual_display_command) = &auto_virtual_display_command {
        effective_command_prefix.extend(virtual_display_command.iter().cloned());
    }
    let requested_display_driver = if !headless && virtual_display_used && linuxbsd_target {
        Some("x11".to_string())
    } else if !headless && wayland_usable && linuxbsd_target && project_prefers_wayland {
        Some("wayland".to_string())
    } else {
        None
    };
    let selected_display_driver = if headless {
        None
    } else if cfg!(windows) {
        Some("windows".to_string())
    } else if requested_display_driver.as_deref() == Some("x11") {
        Some("x11".to_string())
    } else if requested_display_driver.as_deref() == Some("wayland") {
        Some("wayland".to_string())
    } else if x11_display_available || virtual_display_used {
        Some("x11".to_string())
    } else {
        None
    };
    let display_backend_available =
        headless || !linuxbsd_target || selected_display_driver.is_some() || wayland_usable;
    let fallback_reason = if headless {
        None
    } else if configured_virtual_display {
        Some(
            if !headless && launch_prefix_uses_virtual_x11_display(windowed_launch_prefix) {
                "configured_windowed_launch_prefix"
            } else {
                "configured_launch_prefix"
            }
            .to_string(),
        )
    } else if auto_virtual_display_command.is_some() {
        Some("auto_detected_virtual_display".to_string())
    } else if !display_backend_available {
        Some("display_backend_unavailable".to_string())
    } else if project_prefers_wayland
        && !wayland_usable
        && selected_display_driver.as_deref() == Some("x11")
    {
        Some("wayland_unavailable_using_x11".to_string())
    } else {
        None
    };
    DisplayLaunchMetadata {
        headless,
        selected_mode,
        platform: env::consts::OS.to_string(),
        linuxbsd_target,
        x11_display_available,
        wayland_display_available: wayland_usable,
        selected_display_driver,
        project_prefers_wayland,
        requested_display_driver,
        virtual_display_used,
        virtual_display_command: auto_virtual_display_command,
        fallback_reason,
        launch_prefix_used,
        joypad_isolation: false,
        windowed_launch_prefix_used,
        effective_command_prefix,
        display_backend_available,
        startup_attempted: false,
        virtual_display_exit_code: None,
        startup_failure_kind: None,
        startup_failure_retryable: false,
        startup_failure_guidance: None,
    }
}

#[cfg(test)]
pub(super) fn project_prefers_wayland(
    project_root: &Path,
    headless: bool,
    wayland_display_available: bool,
) -> bool {
    !headless
        && wayland_display_available
        && linuxbsd_target()
        && project_wayland_preference_enabled(project_root)
}

fn project_wayland_preference_enabled(project_root: &Path) -> bool {
    let Some(project) = read_project_godot_cached(&project_root.join("project.godot")) else {
        return false;
    };
    project.lines().any(|line| {
        let trimmed = line.trim();
        let Some(value) = trimmed.strip_prefix("run/platforms/linuxbsd/prefer_wayland=") else {
            return false;
        };
        value.trim().trim_matches('"') == "true"
    })
}

fn env_flag_available(name: &str) -> bool {
    env::var_os(name).is_some_and(|value| !value.is_empty())
}

fn detect_xvfb_run(path: Option<&std::ffi::OsStr>) -> Option<Vec<String>> {
    let path = path?;
    env::split_paths(path)
        .map(|entry| entry.join("xvfb-run"))
        .find(|candidate| candidate.is_file())
        .map(|candidate| vec![candidate.to_string_lossy().into_owned(), "-a".to_string()])
}

#[cfg(all(test, unix))]
pub(super) fn effective_launch_prefix(
    launch_prefix: &[String],
    windowed_launch_prefix: &[String],
    display: &DisplayLaunchMetadata,
) -> Vec<String> {
    let mut prefix = Vec::new();
    prefix.extend(launch_prefix.iter().cloned());
    if !display.headless {
        prefix.extend(windowed_launch_prefix.iter().cloned());
    }
    if let Some(virtual_display_command) = &display.virtual_display_command {
        prefix.extend(virtual_display_command.iter().cloned());
    }
    prefix
}

fn linuxbsd_target() -> bool {
    cfg!(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    ))
}

pub(super) fn classify_launch_spawn_failure(
    display: &DisplayLaunchMetadata,
    command: &[String],
) -> String {
    if let Some(virtual_display_command) = &display.virtual_display_command
        && command.first() == virtual_display_command.first()
    {
        return "virtual_display_allocation_failed".to_string();
    }
    if display.virtual_display_used
        && (display.launch_prefix_used || display.windowed_launch_prefix_used)
    {
        return "virtual_display_allocation_failed".to_string();
    }
    "ordinary_launch_failed".to_string()
}

pub(super) fn classify_display_startup_failure(
    display: &DisplayLaunchMetadata,
    status: Option<i32>,
    stdout: &str,
    stderr: &str,
) -> Option<String> {
    if status == Some(0) || display.headless {
        return None;
    }
    let combined = format!("{stdout}\n{stderr}").to_ascii_lowercase();
    let display_startup_noise = [
        "cannot open display",
        "couldn't connect to display",
        "failed to initialize display",
        "no available display",
        "server is already active for display",
        "xvfb failed",
        "xvfb-run: error",
    ];
    if display_startup_noise
        .iter()
        .any(|needle| combined.contains(needle))
    {
        if display.virtual_display_used {
            return Some("virtual_display_allocation_failed".to_string());
        }
        return Some("display_startup_exhausted".to_string());
    }
    None
}

pub(super) fn display_startup_guidance(display: &DisplayLaunchMetadata) -> String {
    if display.virtual_display_used {
        "Retry after releasing stale virtual displays or configure a different virtual display range."
            .to_string()
    } else {
        "Retry with a healthy DISPLAY/WAYLAND_DISPLAY or configure a virtual display wrapper."
            .to_string()
    }
}

pub(super) fn display_launch_metadata_value(display: &DisplayLaunchMetadata) -> serde_json::Value {
    json!({
        "headless": display.headless,
        "selected_mode": display.selected_mode,
        "platform": display.platform,
        "linuxbsd_target": display.linuxbsd_target,
        "x11_display_available": display.x11_display_available,
        "wayland_display_available": display.wayland_display_available,
        "selected_display_driver": display.selected_display_driver,
        "project_prefers_wayland": display.project_prefers_wayland,
        "requested_display_driver": display.requested_display_driver,
        "virtual_display_used": display.virtual_display_used,
        "virtual_display_command": display.virtual_display_command,
        "fallback_reason": display.fallback_reason,
        "launch_prefix_used": display.launch_prefix_used,
        "joypad_isolation": display.joypad_isolation,
        "windowed_launch_prefix_used": display.windowed_launch_prefix_used,
        "effective_command_prefix": display.effective_command_prefix,
        "display_backend_available": display.display_backend_available,
        "startup_attempted": display.startup_attempted,
        "virtual_display_exit_code": display.virtual_display_exit_code,
        "startup_failure_kind": display.startup_failure_kind,
        "startup_failure_retryable": display.startup_failure_retryable,
        "startup_failure_guidance": display.startup_failure_guidance,
    })
}
