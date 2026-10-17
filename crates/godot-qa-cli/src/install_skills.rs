use crate::Cli;
use godot_qa_core::config::load_project_config;
use godot_qa_core::results::{CommandResult, FailureDetail};
use serde_json::json;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

const COMMAND: &str = "install-skills";
const SKILL_NAME: &str = "godot-qa";
const BUNDLED_FILES: &[(&str, &str)] = &[
    (
        "SKILL.md",
        include_str!("../../../skills/godot-qa/SKILL.md"),
    ),
    (
        "agents/openai.yaml",
        include_str!("../../../skills/godot-qa/agents/openai.yaml"),
    ),
    (
        "references/cli-workflows.md",
        include_str!("../../../skills/godot-qa/references/cli-workflows.md"),
    ),
    (
        "references/scenario-authoring.md",
        include_str!("../../../skills/godot-qa/references/scenario-authoring.md"),
    ),
    (
        "references/bundle-gates.md",
        include_str!("../../../skills/godot-qa/references/bundle-gates.md"),
    ),
    (
        "references/visual-and-artifacts.md",
        include_str!("../../../skills/godot-qa/references/visual-and-artifacts.md"),
    ),
    (
        "references/runtime-instrumentation.md",
        include_str!("../../../skills/godot-qa/references/runtime-instrumentation.md"),
    ),
    (
        "references/diagnostics-and-profiling.md",
        include_str!("../../../skills/godot-qa/references/diagnostics-and-profiling.md"),
    ),
];

pub(crate) fn install_skills_result(cli: &Cli) -> CommandResult {
    let config = load_project_config(
        cli.project.as_deref().map(Path::new),
        cli.godot_bin.as_deref(),
        cli.artifact_dir.as_deref(),
        cli.verbose,
        None,
    );
    let target_path = config
        .project_root
        .join(".agents")
        .join("skills")
        .join(SKILL_NAME);

    if !config.project_root.exists() {
        return failure_result(
            &config.project_root,
            &config.repo_root,
            &target_path,
            missing_project_root_failure(&config.project_root),
        );
    }

    match install_skill_tree(&target_path) {
        Ok(()) => {
            let mut result = CommandResult::success(COMMAND, "bundled skills installed");
            result.data = serde_json::Map::from_iter([
                (
                    "project_root".to_string(),
                    json!(config.project_root.to_string_lossy().into_owned()),
                ),
                (
                    "repo_root".to_string(),
                    json!(config.repo_root.to_string_lossy().into_owned()),
                ),
                ("scope".to_string(), json!("project")),
                ("agent".to_string(), json!("codex")),
                (
                    "installed".to_string(),
                    json!([{
                        "name": SKILL_NAME,
                        "source_path": null,
                        "source": "embedded",
                        "target_path": target_path.to_string_lossy().into_owned(),
                        "method": "copy"
                    }]),
                ),
            ]);
            result
        }
        Err(failure) => failure_result(
            &config.project_root,
            &config.repo_root,
            &target_path,
            failure,
        ),
    }
}

fn install_skill_tree(target_path: &Path) -> Result<(), FailureDetail> {
    let parent = target_path.parent().ok_or_else(|| FailureDetail {
        kind: "skill_install_failed".to_string(),
        message: Some("skill target has no parent directory".to_string()),
        command: Some(COMMAND.to_string()),
        path: Some(target_path.to_string_lossy().into_owned()),
        expected: Some(json!("a project-scoped skill target under .agents/skills")),
        actual: Some(json!("missing parent")),
        ..FailureDetail::default()
    })?;

    fs::create_dir_all(parent)
        .map_err(|error| install_error("failed to create skill parent directory", parent, error))?;

    if target_path.exists() && !target_path.is_dir() {
        return Err(FailureDetail {
            kind: "skill_target_conflict".to_string(),
            message: Some("skill target exists as a file".to_string()),
            command: Some(COMMAND.to_string()),
            path: Some(target_path.to_string_lossy().into_owned()),
            expected: Some(json!("a directory or no existing path")),
            actual: Some(json!("file")),
            ..FailureDetail::default()
        });
    }

    let temp_target = unique_child(parent, ".godot-qa-install");
    let backup_target = unique_child(parent, ".godot-qa-backup");

    if temp_target.exists() {
        let _ = fs::remove_dir_all(&temp_target);
    }
    if backup_target.exists() {
        let _ = fs::remove_dir_all(&backup_target);
    }

    write_bundled_files(&temp_target).map_err(|error| {
        install_error(
            "failed to copy bundled skill into a temporary directory",
            &temp_target,
            error,
        )
    })?;
    if let Err(failure) = validate_skill_package(&temp_target) {
        let _ = fs::remove_dir_all(&temp_target);
        return Err(failure);
    }

    if target_path.exists() {
        fs::rename(target_path, &backup_target).map_err(|error| {
            install_error(
                "failed to rotate existing installed skill out of the way",
                target_path,
                error,
            )
        })?;
    }

    if let Err(error) = fs::rename(&temp_target, target_path) {
        if backup_target.exists() {
            let _ = fs::rename(&backup_target, target_path);
        }
        let _ = fs::remove_dir_all(&temp_target);
        return Err(install_error(
            "failed to finalize installed skill",
            target_path,
            error,
        ));
    }

    if backup_target.exists() {
        fs::remove_dir_all(&backup_target).map_err(|error| {
            install_error(
                "failed to remove previous installed skill copy",
                &backup_target,
                error,
            )
        })?;
    }

    Ok(())
}

fn validate_skill_package(source_path: &Path) -> Result<(), FailureDetail> {
    if !source_path.exists() || !source_path.is_dir() {
        return Err(FailureDetail {
            kind: "missing_skill_source".to_string(),
            message: Some("embedded skill could not be materialized for installation".to_string()),
            command: Some(COMMAND.to_string()),
            path: Some(source_path.to_string_lossy().into_owned()),
            expected: Some(json!("directory skills/godot-qa/ with SKILL.md")),
            actual: Some(json!("missing")),
            ..FailureDetail::default()
        });
    }

    let skill_md = source_path.join("SKILL.md");
    if !skill_md.is_file() {
        return Err(FailureDetail {
            kind: "invalid_skill_package".to_string(),
            message: Some("bundled skill is missing SKILL.md".to_string()),
            command: Some(COMMAND.to_string()),
            path: Some(skill_md.to_string_lossy().into_owned()),
            expected: Some(json!("SKILL.md with YAML frontmatter")),
            actual: Some(json!("missing")),
            ..FailureDetail::default()
        });
    }

    let contents = fs::read_to_string(&skill_md).map_err(|error| FailureDetail {
        kind: "invalid_skill_package".to_string(),
        message: Some(format!("failed to read SKILL.md: {error}")),
        command: Some(COMMAND.to_string()),
        path: Some(skill_md.to_string_lossy().into_owned()),
        expected: Some(json!("readable SKILL.md with YAML frontmatter")),
        actual: Some(json!("unreadable")),
        ..FailureDetail::default()
    })?;

    let found_name = parse_frontmatter_name(&contents).ok_or_else(|| FailureDetail {
        kind: "invalid_skill_package".to_string(),
        message: Some("bundled skill SKILL.md is missing frontmatter name".to_string()),
        command: Some(COMMAND.to_string()),
        path: Some(skill_md.to_string_lossy().into_owned()),
        expected: Some(json!(format!("frontmatter name: {SKILL_NAME}"))),
        actual: Some(json!("missing name field")),
        ..FailureDetail::default()
    })?;

    if found_name != SKILL_NAME {
        return Err(FailureDetail {
            kind: "invalid_skill_package".to_string(),
            message: Some("bundled skill SKILL.md has the wrong frontmatter name".to_string()),
            command: Some(COMMAND.to_string()),
            path: Some(skill_md.to_string_lossy().into_owned()),
            expected: Some(json!(SKILL_NAME)),
            actual: Some(json!(found_name)),
            ..FailureDetail::default()
        });
    }

    Ok(())
}

fn parse_frontmatter_name(contents: &str) -> Option<String> {
    let mut lines = contents.lines();
    if lines.next()? != "---" {
        return None;
    }

    for line in lines {
        if line == "---" {
            break;
        }
        if let Some(rest) = line.strip_prefix("name:") {
            let value = rest.trim().trim_matches('"').trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }

    None
}

fn write_bundled_files(target: &Path) -> io::Result<()> {
    fs::create_dir_all(target)?;
    for (name, contents) in BUNDLED_FILES {
        let path = target.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, contents)?;
    }
    Ok(())
}

fn install_error(message: &str, path: &Path, error: io::Error) -> FailureDetail {
    FailureDetail {
        kind: "skill_install_failed".to_string(),
        message: Some(format!("{message}: {error}")),
        command: Some(COMMAND.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        expected: Some(json!("bundled skill can be copied into the project scope")),
        actual: Some(json!(io_error_kind(&error))),
        ..FailureDetail::default()
    }
}

fn failure_result(
    project_root: &Path,
    repo_root: &Path,
    target_path: &Path,
    failure: FailureDetail,
) -> CommandResult {
    CommandResult {
        ok: false,
        command: COMMAND.to_string(),
        message: "skill installation failed".to_string(),
        data: serde_json::Map::from_iter([
            (
                "project_root".to_string(),
                json!(project_root.to_string_lossy().into_owned()),
            ),
            (
                "repo_root".to_string(),
                json!(repo_root.to_string_lossy().into_owned()),
            ),
            ("scope".to_string(), json!("project")),
            ("agent".to_string(), json!("codex")),
            (
                "target_path".to_string(),
                json!(target_path.to_string_lossy().into_owned()),
            ),
        ]),
        failures: vec![failure],
        exit_code: 1,
    }
}

fn missing_project_root_failure(path: &Path) -> FailureDetail {
    FailureDetail {
        kind: "missing_project_root".to_string(),
        message: Some("project root does not exist".to_string()),
        command: Some(COMMAND.to_string()),
        path: Some(path.to_string_lossy().into_owned()),
        expected: Some(json!("existing project root")),
        actual: Some(json!("missing")),
        ..FailureDetail::default()
    }
}

fn io_error_kind(error: &io::Error) -> &'static str {
    match error.kind() {
        io::ErrorKind::PermissionDenied => "PermissionError",
        io::ErrorKind::AlreadyExists => "AlreadyExists",
        io::ErrorKind::NotFound => "NotFound",
        _ => "OSError",
    }
}

fn unique_child(parent: &Path, prefix: &str) -> PathBuf {
    parent.join(format!("{prefix}-{}", unique_suffix()))
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{nanos}-{}", process::id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn validate_skill_package_rejects_missing_skill_md() {
        let root = make_temp_dir("missing-skill-md");
        let source = root.join("skills").join(SKILL_NAME);
        fs::create_dir_all(&source).unwrap();

        let failure = validate_skill_package(&source).unwrap_err();
        assert_eq!(failure.kind, "invalid_skill_package");
        assert_eq!(failure.command.as_deref(), Some(COMMAND));
        assert!(failure.path.unwrap().ends_with("SKILL.md"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn validate_skill_package_rejects_wrong_frontmatter_name() {
        let root = make_temp_dir("wrong-skill-name");
        let source = root.join("skills").join(SKILL_NAME);
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("SKILL.md"),
            "---\nname: wrong-skill\ndescription: wrong\n---\n",
        )
        .unwrap();

        let failure = validate_skill_package(&source).unwrap_err();
        assert_eq!(failure.kind, "invalid_skill_package");
        assert_eq!(failure.actual, Some(json!("wrong-skill")));

        fs::remove_dir_all(root).unwrap();
    }

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let path = env::temp_dir().join(format!("godot-qa-{prefix}-{}", unique_suffix()));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
