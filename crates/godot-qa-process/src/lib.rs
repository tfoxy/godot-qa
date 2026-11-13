#![allow(clippy::result_large_err)]

pub mod android;
pub mod apk_cache;
pub mod directory_link;
#[cfg(unix)]
mod owned_group;
mod proc_stat;
pub mod project_access;
#[cfg(windows)]
mod windows;

mod capture;
mod display;
mod launch;
mod lifecycle;
mod probes;

pub use capture::{OutputTruncation, ProcessTermination, ProcessTiming};
pub use display::{
    DisplayLaunchMetadata, resolve_project_display_launch_metadata,
    resolve_project_display_launch_metadata_with_windowed_prefix,
};
pub use launch::{
    GodotOneShotLaunch, GodotRunResult, GodotSessionLaunch, launch_godot_asset_trial_runner,
    launch_godot_check, launch_godot_check_with_timeout,
    launch_godot_check_with_timeout_and_fixed_fps, launch_godot_check_with_timeout_monitor,
    launch_godot_import, launch_godot_import_with_args, launch_godot_one_shot,
    launch_godot_patch_runner, launch_godot_session,
};
pub use lifecycle::{
    ProcessCleanupResult, configure_background_command, pid_is_running, process_identity,
    process_identity_matches, reap_background_child, stop_godot_process,
    stop_godot_process_with_cleanup, stop_godot_process_with_identity, terminate_process_tree,
    wait_for_godot_process_exit,
};
pub use probes::{
    GodotVersionProbeAttempt, GodotVersionProbeMetadata, GodotVersionProbeResult, SafeProbeResult,
    executable_candidates, format_command, probe_godot_version, probe_godot_version_with_metadata,
    resolve_godot_binary, run_safe_probe,
};

pub(crate) use capture::run_owned_child_output_with_timeout_monitor;
#[cfg(unix)]
pub(crate) use lifecycle::is_zombie_process;
#[cfg(all(test, target_os = "linux"))]
use probes::spawn_probe_child;

pub fn migration_marker() -> &'static str {
    "m29"
}

#[cfg(test)]
mod tests;
