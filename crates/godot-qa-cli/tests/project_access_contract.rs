//! Keep new Godot launch paths in the project-reservation review surface.

use std::path::Path;

fn read(path: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path),
    )
    .unwrap()
}

#[test]
fn all_godot_launchers_use_a_project_lease_or_a_guarded_delegate() {
    let source = read("crates/godot-qa-process/src/lib.rs");
    let expected = [
        ("launch_godot_import", "launch_godot_import_with_args("),
        ("launch_godot_import_with_args", "LaunchLease::acquire("),
        ("launch_godot_check", "launch_godot_check_with_timeout("),
        (
            "launch_godot_check_with_timeout",
            "launch_godot_check_with_timeout_and_fixed_fps(",
        ),
        (
            "launch_godot_check_with_timeout_and_fixed_fps",
            "LaunchLease::acquire(",
        ),
        ("launch_godot_one_shot", "LaunchLease::acquire("),
        ("launch_godot_asset_trial_runner", "LaunchLease::acquire("),
        ("launch_godot_patch_runner", "LaunchLease::acquire("),
        ("launch_godot_session", "LaunchLease::acquire("),
    ];
    let found: Vec<_> = source
        .lines()
        .filter_map(|line| {
            line.strip_prefix("pub fn launch_godot_")
                .and_then(|tail| tail.split_once('('))
                .map(|(name, _)| format!("launch_godot_{name}"))
        })
        .collect();
    assert_eq!(
        found.len(),
        expected.len(),
        "review and classify every new Godot launcher"
    );
    for (name, gate) in expected {
        let start = source.find(&format!("pub fn {name}(")).unwrap();
        let body = source[start..].split("\n}").next().unwrap();
        assert!(
            body.contains(gate),
            "{name} must hold a project lease or call its guarded delegate"
        );
    }
    for (path, function) in [
        (
            "crates/godot-qa-process/src/android.rs",
            "pub fn export_debug_apk(",
        ),
        (
            "crates/godot-qa-cli/src/presented_startup.rs",
            "pub(crate) fn assert_startup_presented(",
        ),
        ("xtask/src/main.rs", "fn prepare_sample("),
    ] {
        let text = read(path);
        let body = text[text.find(function).unwrap()..]
            .split("\n}")
            .next()
            .unwrap();
        assert!(
            body.contains("LaunchLease::acquire("),
            "{path}: missing project access guard"
        );
    }
    // Information-only --version/help probes do not launch a project. Runtime
    // checks and diagnostics, including their imports, delegate to these guards.
    for path in [
        "crates/godot-qa-cli/src/check_command.rs",
        "crates/godot-qa-cli/src/diagnostics_command.rs",
        "crates/godot-qa-scenario/src/check.rs",
    ] {
        let text = read(path);
        assert!(
            text.contains("launch_godot_import(")
                || text.contains("launch_godot_import_with_args(")
        );
        assert!(
            !text.contains("Command::new("),
            "{path}: use guarded process launchers"
        );
    }
}
