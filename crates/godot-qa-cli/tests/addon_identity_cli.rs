use godot_qa_core::addon_identity::EXPECTED_FILES;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn doctor(binary: &Path, project: &Path, godot: &Path, strict: bool) -> (bool, Value) {
    let mut command = Command::new(binary);
    command
        .current_dir(project)
        .arg("--project")
        .arg(project)
        .arg("--godot-bin")
        .arg(godot)
        .args(["--json", "doctor"]);
    if strict {
        command.arg("--strict-addon-identity");
    }
    let output = command.output().unwrap();
    (
        output.status.success(),
        serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stdout))),
    )
}

#[test]
fn relocated_binary_uses_compiled_identity_without_git_or_sibling_checkout() {
    let root = std::env::temp_dir().join(format!(
        "godot-qa-identity-cli-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let project = root.join("project");
    let addon = project.join("addons/godot_qa");
    fs::create_dir_all(&addon).unwrap();
    fs::write(project.join("project.godot"), "[application]\n").unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../addons/godot_qa");
    for (relative, _, _) in EXPECTED_FILES {
        let destination = addon.join(relative);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::copy(source.join(relative), destination).unwrap();
    }
    let binary = root.join(if cfg!(windows) {
        "godot-qa.exe"
    } else {
        "godot-qa"
    });
    fs::copy(env!("CARGO_BIN_EXE_godot-qa"), &binary).unwrap();
    let godot =
        godot_qa_test_support::fixture(&root.join("godot"), &[("stdout", "Godot_v4.3-stable\n")]);
    let (ok, baseline) = doctor(&binary, &project, &godot, true);
    assert!(ok, "{baseline:#}");
    assert_eq!(baseline["data"]["addon"]["identity"]["status"], "match");
    let expected = baseline["data"]["addon"]["identity"]["expected"].clone();
    // A mutable sibling distribution, docs, and generated UIDs cannot redefine the
    // expected runtime content carried by a previously built binary.
    fs::create_dir_all(root.join("addons/godot_qa")).unwrap();
    fs::write(
        root.join("addons/godot_qa/plugin.cfg"),
        "[plugin]\nversion=\"0.1.0\"\n",
    )
    .unwrap();
    fs::write(addon.join("README.md"), "local documentation").unwrap();
    fs::write(addon.join("qa_runtime.gd.uid"), "uid://local").unwrap();
    let (ok, ignored) = doctor(&binary, &project, &godot, true);
    assert!(ok, "{ignored:#}");
    assert_eq!(ignored["data"]["addon"]["identity"]["expected"], expected);
    fs::write(addon.join("new_runtime.gd"), "extends Node\n").unwrap();
    let (ok, warning) = doctor(&binary, &project, &godot, false);
    assert!(ok, "{warning:#}");
    assert_eq!(warning["data"]["addon"]["identity"]["status"], "mismatch");
    assert_eq!(
        warning["data"]["addon"]["identity"]["extra"][0],
        "new_runtime.gd"
    );
    assert_eq!(warning["data"]["addon"]["identity"]["expected"], expected);
    let (ok, strict) = doctor(&binary, &project, &godot, true);
    assert!(!ok);
    assert!(
        strict["failures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|failure| failure["type"] == "addon_identity_mismatch")
    );
    fs::remove_dir_all(root).unwrap();
}
