use godot_qa_core::config::{find_project_root, load_project_config};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_temp_dir() -> PathBuf {
    std::env::temp_dir().join(format!(
        "godot-qa-project-root-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[test]
fn godot_ancestor_takes_precedence_over_nested_cargo_root() {
    let root = unique_temp_dir();
    let game = root.join("game project");
    let native = game.join("native");
    fs::create_dir_all(native.join("src")).unwrap();
    fs::create_dir_all(game.join("qa")).unwrap();
    fs::write(root.join("Cargo.toml"), "[workspace]\n").unwrap();
    fs::write(game.join("project.godot"), "[application]\n").unwrap();
    fs::write(native.join("Cargo.toml"), "[package]\n").unwrap();
    fs::write(
        game.join("qa/config.yaml"),
        "logs:\n  ignore: [game-config-marker]\n",
    )
    .unwrap();

    let config = load_project_config(Some(&native), None, None, false, None);
    let overridden = load_project_config(Some(&native), None, Some("qa/output"), false, None);
    let from_child = find_project_root(Some(&native.join("src")));
    fs::remove_dir_all(root).unwrap();

    assert_eq!(config.project_root, game);
    assert!(config.project_config_failure.is_none());
    assert_eq!(config.log_ignore_patterns, ["game-config-marker"]);
    assert_eq!(config.qa_dir, game.join("qa"));
    assert_eq!(config.scenarios_dir, game.join("qa/scenarios"));
    assert_eq!(config.artifacts_dir, game.join(".qa/artifacts"));
    assert_eq!(overridden.artifacts_dir, game.join("qa/output"));
    assert_eq!(from_child, game);
}

#[test]
fn nearest_godot_ancestor_wins() {
    let root = unique_temp_dir();
    let game = root.join("game");
    let native = game.join("native");
    fs::create_dir_all(&native).unwrap();
    fs::write(root.join("project.godot"), "[application]\n").unwrap();
    fs::write(game.join("project.godot"), "[application]\n").unwrap();
    fs::write(native.join("Cargo.toml"), "[package]\n").unwrap();

    let discovered = find_project_root(Some(&native));
    fs::remove_dir_all(root).unwrap();

    assert_eq!(discovered, game);
}

#[test]
fn nearest_cargo_ancestor_is_fallback_without_godot_project() {
    let root = unique_temp_dir();
    let native = root.join("native");
    let source = native.join("src");
    fs::create_dir_all(&source).unwrap();
    fs::write(root.join("Cargo.toml"), "[workspace]\n").unwrap();
    fs::write(native.join("Cargo.toml"), "[package]\n").unwrap();

    let discovered = find_project_root(Some(&source));
    fs::remove_dir_all(root).unwrap();

    assert_eq!(discovered, native);
}
