use serde_json::Value;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .components()
        .collect()
}

fn package_files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }

    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{nanos}-{}", std::process::id())
}

fn make_project_root(prefix: &str) -> PathBuf {
    let root = env::temp_dir().join(format!("godot-qa-{prefix}-{}", unique_suffix()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("project.godot"), "[application]\n").unwrap();
    root
}

fn run_install_skills(project_root: &Path, path_dir: Option<&Path>) -> Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_godot-qa"));
    command
        .current_dir(repo_root())
        .arg("--json")
        .arg("install-skills")
        .arg("--project")
        .arg(project_root);
    if let Some(path_dir) = path_dir {
        command.env("PATH", path_dir);
    }

    let output = command.output().unwrap();
    assert!(
        output.status.success() || output.status.code() == Some(1),
        "unexpected exit status: {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn install_skills_copies_bundled_skill_into_project_scope() {
    let project_root = make_project_root("install-skills-success");
    let output = run_install_skills(&project_root, None);

    assert_eq!(output["ok"], true);
    assert_eq!(output["command"], "install-skills");
    assert_eq!(output["data"]["scope"], "project");
    assert_eq!(output["data"]["agent"], "codex");
    assert_eq!(output["data"]["installed"][0]["name"], "godot-qa");
    assert_eq!(output["data"]["installed"][0]["method"], "copy");

    let installed_skill = project_root.join(".agents/skills/godot-qa/SKILL.md");
    assert!(installed_skill.is_file());
    let body = fs::read_to_string(installed_skill).unwrap();
    assert!(body.contains("name: godot-qa"));

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn install_skills_reinstall_replaces_only_the_bundled_skill_directory() {
    let project_root = make_project_root("install-skills-reinstall");
    let sibling_skill = project_root.join(".agents/skills/other-skill");
    fs::create_dir_all(&sibling_skill).unwrap();
    fs::write(sibling_skill.join("SKILL.md"), "placeholder").unwrap();

    let first = run_install_skills(&project_root, None);
    assert_eq!(first["ok"], true);

    let target_root = project_root.join(".agents/skills/godot-qa");
    fs::write(target_root.join("stale.txt"), "remove me").unwrap();

    let second = run_install_skills(&project_root, None);
    assert_eq!(second["ok"], true);
    assert!(!target_root.join("stale.txt").exists());
    assert!(sibling_skill.join("SKILL.md").is_file());

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn install_skills_reports_conflict_when_target_exists_as_file() {
    let project_root = make_project_root("install-skills-conflict");
    let skills_dir = project_root.join(".agents/skills");
    fs::create_dir_all(&skills_dir).unwrap();
    fs::write(skills_dir.join("godot-qa"), "not a directory").unwrap();

    let output = run_install_skills(&project_root, None);
    assert_eq!(output["ok"], false);
    assert_eq!(output["failures"][0]["type"], "skill_target_conflict");

    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn install_skills_does_not_require_node_or_npx_on_path() {
    let project_root = make_project_root("install-skills-no-wrapper");
    let empty_path = env::temp_dir().join(format!("godot-qa-empty-path-{}", unique_suffix()));
    fs::create_dir_all(&empty_path).unwrap();

    let output = run_install_skills(&project_root, Some(&empty_path));
    assert_eq!(output["ok"], true);
    assert!(
        project_root
            .join(".agents/skills/godot-qa/SKILL.md")
            .is_file()
    );

    fs::remove_dir_all(&empty_path).unwrap();
    fs::remove_dir_all(project_root).unwrap();
}

#[test]
fn relocated_binary_installs_embedded_skills_outside_source_checkout() {
    let root = make_project_root("relocated binary ü");
    let distribution = root.join("distribution");
    let project = root.join("unrelated project");
    fs::create_dir_all(&distribution).unwrap();
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join("project.godot"), "[application]\n").unwrap();
    let executable = distribution.join(format!("godot-qa{}", env::consts::EXE_SUFFIX));
    fs::copy(env!("CARGO_BIN_EXE_godot-qa"), &executable).unwrap();
    let output = Command::new(&executable)
        .current_dir(&project)
        .args(["install-skills", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["data"]["installed"][0]["source"], "embedded");
    assert!(result["data"]["installed"][0]["source_path"].is_null());
    let reported_root = PathBuf::from(result["data"]["repo_root"].as_str().unwrap());
    assert_eq!(
        fs::canonicalize(reported_root).unwrap(),
        fs::canonicalize(&distribution).unwrap()
    );
    assert_eq!(
        package_files(&project.join(".agents/skills/godot-qa")),
        package_files(&repo_root().join("skills/godot-qa")),
        "the relocated binary must install every bundled file unchanged",
    );
    fs::remove_dir_all(root).unwrap();
}
