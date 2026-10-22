mod release;

use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{SystemTime, UNIX_EPOCH};

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    // This mode is also used by the copied sample fixture outside the checkout.
    if args.first().map(String::as_str) == Some("process-wrapper") {
        return match process_wrapper(&args[1..]) {
            Ok(code) => ExitCode::from(code),
            Err(error) => {
                eprintln!("process wrapper: {error}");
                ExitCode::FAILURE
            }
        };
    }
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result {
    env::set_current_dir(root())?;
    match args.first().map(String::as_str) {
        Some("prepare-sample") if args.len() == 1 => prepare_sample(),
        Some("check") if args.len() == 1 => checks(),
        Some("release") => release::run(&args[1..]),
        Some("package") if args.len() == 2 => package(&args[1]),
        Some("smoke") if args.len() == 2 || args.len() == 3 => smoke(Path::new(&args[1]), Path::new(args.get(2).map(String::as_str).unwrap_or("examples/sample_project"))),
        Some("verify-version") if args.len() == 2 => release::verify_release(&args[1]),
        Some("check-commits") if args.len() == 3 => release::check_commits(&args[1], &args[2]),
        Some("commit-msg") if args.len() == 2 => release::validate_commit_file(Path::new(&args[1])),
        Some("install-hooks") if args.len() == 1 => command("git", &["config", "core.hooksPath", ".githooks"]),
        _ => Err("usage: cargo xtask <prepare-sample | check | package TARGET | smoke BINARY | verify-version TAG | release [--dry-run | prepare | tag] [--version VERSION] | install-hooks | commit-msg FILE>".into()),
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn command(program: &str, args: &[&str]) -> Result {
    eprintln!("+ {program} {}", args.join(" "));
    let status = Command::new(program).args(args).status()?;
    if !status.success() {
        return Err(format!("{program} exited with {status}").into());
    }
    Ok(())
}

fn tracked_paths(directory: &Path) -> Result<Vec<PathBuf>> {
    let output = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(directory)
        .output()?;
    if !output.status.success() {
        return Err("git ls-files failed while enumerating distribution assets".into());
    }
    Ok(String::from_utf8(output.stdout)?
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .collect())
}

fn copy_tracked_file(source: &Path, destination: &Path) -> Result {
    let metadata = fs::symlink_metadata(source)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!(
            "distribution asset must be a regular tracked file: {}",
            source.display()
        )
        .into());
    }
    fs::create_dir_all(destination.parent().ok_or("asset parent")?)?;
    fs::copy(source, destination)?;
    Ok(())
}

fn copy_tracked_tree(directory: &Path, prefix: &Path, target: &Path) -> Result {
    for file in tracked_paths(directory)? {
        if let Ok(relative) = file.strip_prefix(prefix) {
            let source = directory.join(&file);
            if source.is_file() {
                copy_tracked_file(&source, &target.join(relative))?;
            }
        }
    }
    Ok(())
}

fn copy_distribution(stage: &Path) -> Result {
    for file in tracked_paths(&root())? {
        let selected = [
            "README.md",
            "CONTRIBUTING.md",
            "AGENTS.md",
            "CHANGELOG.md",
            "LICENSE",
        ]
        .iter()
        .any(|name| file == Path::new(name))
            || ["addons/godot_qa", "docs", "skills/godot-qa", "examples"]
                .iter()
                .any(|prefix| file.starts_with(prefix));
        // The previous sample symlink must never become an archive entry. The
        // generated sample addon comes only from canonical, tracked addon files.
        if !selected || file == Path::new("examples/sample_project/addons/godot_qa") {
            continue;
        }
        let source = root().join(&file);
        if !source.exists() {
            continue;
        } // tracked deletion in a local validation checkout
        copy_tracked_file(&source, &stage.join(&file))?;
        if let Ok(relative) = file.strip_prefix("addons/godot_qa") {
            copy_tracked_file(
                &source,
                &stage
                    .join("examples/sample_project/addons/godot_qa")
                    .join(relative),
            )?;
        }
    }
    Ok(())
}

fn prepare_sample() -> Result {
    let sample = root().join("examples/sample_project");
    let _project_lease = godot_qa_process::project_access::LaunchLease::acquire(
        &sample,
        godot_qa_process::project_access::LaunchClass::SourceWriter,
    )
    .map_err(|error| std::io::Error::other(serde_json::to_string(&error).unwrap_or_default()))?;
    let target = sample.join("addons/godot_qa");
    if let Ok(metadata) = fs::symlink_metadata(&target) {
        if metadata.file_type().is_symlink() {
            return Err(
                "sample addon is a symlink; remove that link before preparing the portable sample"
                    .into(),
            );
        }
        fs::remove_dir_all(&target)?;
    }
    copy_tracked_tree(&root(), Path::new("addons/godot_qa"), &target)?;
    let tools = sample.join(".qa/tools");
    fs::create_dir_all(&tools)?;
    let wrapper = tools.join(format!("qa-process-fixture{}", env::consts::EXE_SUFFIX));
    fs::copy(env::current_exe()?, &wrapper)?;
    println!("Prepared {}", sample.display());
    Ok(())
}

fn process_wrapper(args: &[String]) -> Result<u8> {
    let binary = args.first().ok_or("process-wrapper needs an executable")?;
    let status = Command::new(binary).args(&args[1..]).status()?;
    let code = status.code().unwrap_or(1);
    if code != 0 {
        return Ok(u8::try_from(code).unwrap_or(1));
    }
    for arg in &args[1..] {
        if let Some(path) = arg.strip_prefix("--qa-run-request=") {
            let request: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
            if contains_exit_demo(&request) {
                return Ok(7);
            }
        }
    }
    Ok(0)
}

fn contains_exit_demo(value: &serde_json::Value) -> bool {
    let name = value.get("name").and_then(serde_json::Value::as_str);
    matches!(
        name,
        Some("process_exit_strict_failure_demo" | "process_exit_warning_policy_demo")
    ) || value.get("scenario").is_some_and(contains_exit_demo)
}

fn godot() -> String {
    env::var("GODOT_BIN").unwrap_or_else(|_| "godot".into())
}

fn checks() -> Result {
    prepare_sample()?;
    command(
        &godot(),
        &[
            "--headless",
            "--path",
            "examples/sample_project",
            "--import",
        ],
    )?;
    command("cargo", &["test", "--workspace", "--locked"])?;
    command("cargo", &["fmt", "--all", "--check"])?;
    command(
        "cargo",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    command("cargo", &["build", "--locked", "--bin", "godot-qa"])?;
    smoke(
        &root().join(format!("target/debug/godot-qa{}", env::consts::EXE_SUFFIX)),
        Path::new("examples/sample_project"),
    )
}

fn smoke(binary: &Path, project: &Path) -> Result {
    let project = project.to_str().ok_or("project path must be UTF-8")?;
    let binary = fs::canonicalize(binary)?;
    let cli = binary.to_str().ok_or("non-UTF8 binary path")?;
    let recording_path = format!(
        ".qa/recordings/release_recording_{}_{}.yaml",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    );
    command(cli, &["--version"])?;
    command(cli, &["--help"])?;
    for args in [
        vec!["doctor"],
        vec!["scenario", "list"],
        vec!["check", "--scene", "res://ui/main_menu.tscn"],
        vec!["check", "--tests"],
        vec!["scenario", "run", "sample_main_menu_start"],
        vec!["scenario", "run", "sample_settings_layout"],
        vec!["scenario", "run", "audio_export_demo"],
        vec![
            "scenario",
            "record",
            "release_recording",
            "--scene",
            "res://ui/main_menu.tscn",
            "--duration-frames",
            "10",
            "--out",
            recording_path.as_str(),
        ],
        vec![
            "scenario",
            "bundle",
            "run",
            "sample_regression_gate",
            "--jobs",
            "2",
        ],
        vec![
            "scenario",
            "bundle",
            "run",
            "sample_regression_gate",
            "--warm",
            "--warm-strict",
            "--jobs",
            "2",
        ],
    ] {
        let mut args = args;
        args.extend(["--project", project, "--json"]);
        command(cli, &args)?;
    }
    command(
        cli,
        &[
            "session",
            "start",
            "--hot-reload",
            "--project",
            project,
            "--json",
        ],
    )?;
    let live = (|| {
        for mut args in [
            vec!["focus", "graph"],
            vec!["session", "reload", "--script", "res://ui/main_menu.gd"],
            vec!["session", "profile", "--frames", "5"],
            vec![
                "trace",
                "record",
                "--target",
                "qa:start_button",
                "--frames",
                "5",
                "--summary-json",
            ],
        ] {
            args.extend(["--project", project, "--json"]);
            command(cli, &args)?;
        }
        command(
            cli,
            &[
                "input",
                "click",
                "qa:start_button",
                "--project",
                project,
                "--json",
            ],
        )?;
        command(
            cli,
            &[
                "snapshot",
                "--name",
                "release-smoke",
                "--project",
                project,
                "--json",
            ],
        )
    })();
    let stop = command(cli, &["session", "stop", "--project", project, "--json"]);
    live?;
    stop
}

fn package(target: &str) -> Result {
    if !matches!(
        target,
        "x86_64-unknown-linux-gnu" | "x86_64-pc-windows-msvc"
    ) {
        return Err("unsupported release target".into());
    }
    let version = release::current_version()?;
    release::verify_version(&format!("v{version}"))?;
    // Apply remapping to dependencies too, so panic/debug source locations do not
    // expose the builder checkout or Cargo home. No host-specific CPU flags.
    let mut flags = String::new();
    if let Some(home) = env::var_os("CARGO_HOME")
        .or_else(|| env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }))
    {
        flags.push_str(&format!(
            "--remap-path-prefix={}=/build",
            PathBuf::from(home).display()
        ));
    }
    if !flags.is_empty() {
        flags.push('\u{1f}');
    }
    flags.push_str(&format!("--remap-path-prefix={}=.", root().display()));
    if target.contains("windows") {
        flags.push_str("\u{1f}-Ctarget-feature=+crt-static");
    }
    let status = Command::new("cargo")
        .args([
            "build",
            "--release",
            "--locked",
            "--bin",
            "godot-qa",
            "--target",
            target,
        ])
        .env_remove("RUSTFLAGS")
        .env("CARGO_ENCODED_RUSTFLAGS", flags)
        .status()?;
    if !status.success() {
        return Err("optimized release build failed".into());
    }
    let name = format!("godot-qa-v{version}-{target}");
    let stage = root().join("dist").join(&name);
    if stage.exists() {
        fs::remove_dir_all(&stage)?;
    }
    fs::create_dir_all(&stage)?;
    let exe = if target.contains("windows") {
        "godot-qa.exe"
    } else {
        "godot-qa"
    };
    fs::copy(
        root().join("target").join(target).join("release").join(exe),
        stage.join(exe),
    )?;
    copy_distribution(&stage)?;
    let extension = if target.contains("windows") {
        "zip"
    } else {
        "tar.gz"
    };
    let archive = root().join("dist").join(format!("{name}.{extension}"));
    if archive.exists() {
        fs::remove_file(&archive)?;
    }
    if target.contains("windows") {
        let status = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "Compress-Archive -Path $env:QA_STAGE -DestinationPath $env:QA_ARCHIVE",
            ])
            .env("QA_STAGE", &stage)
            .env("QA_ARCHIVE", &archive)
            .status()?;
        if !status.success() {
            return Err("Compress-Archive failed".into());
        }
    } else {
        command(
            "tar",
            &[
                "-czf",
                archive.to_str().ok_or("archive path")?,
                "-C",
                "dist",
                &name,
            ],
        )?;
    }
    let bytes = fs::read(&archive)?;
    let hash = godot_qa_core::hash::sha256_hex(&bytes);
    fs::write(
        PathBuf::from(format!("{}.sha256", archive.display())),
        format!(
            "{hash}  {}\n",
            archive.file_name().unwrap().to_string_lossy()
        ),
    )?;
    // Validate the actual extracted archive, outside the checkout and in a path
    // with spaces. Explicit project marker prevents ancestor-root discovery.
    let scratch = env::temp_dir().join(format!("godot qa release café {}", std::process::id()));
    fs::create_dir_all(&scratch)?;
    if target.contains("windows") {
        let status = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "Expand-Archive -Path $env:QA_ARCHIVE -DestinationPath $env:QA_DEST -Force",
            ])
            .env("QA_ARCHIVE", &archive)
            .env("QA_DEST", &scratch)
            .status()?;
        if !status.success() {
            return Err("release archive extraction failed".into());
        }
    } else {
        command(
            "tar",
            &[
                "-xzf",
                archive.to_str().ok_or("archive path")?,
                "-C",
                scratch.to_str().ok_or("temporary path")?,
            ],
        )?;
    }
    let project = scratch.join("standalone project");
    fs::create_dir_all(&project)?;
    fs::write(project.join("project.godot"), "config_version=5\n")?;
    let extracted = scratch.join(&name).join(exe);
    let status = Command::new(&extracted)
        .args(["install-skills", "--project", ".", "--json"])
        .current_dir(&project)
        .status()?;
    if !status.success() {
        return Err("relocated release binary install-skills smoke failed".into());
    }
    let sample = scratch.join(&name).join("examples/sample_project");
    env::set_current_dir(&scratch)?;
    command(
        &godot(),
        &[
            "--headless",
            "--path",
            sample.to_str().ok_or("sample path")?,
            "--import",
        ],
    )?;
    smoke(&extracted, &sample)?;
    env::set_current_dir(root())?;
    fs::remove_dir_all(&scratch)?;
    println!("Packaged {}", archive.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_copy_uses_only_tracked_addon_files() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            env::temp_dir().join(format!("qa tracked assets {} {unique}", std::process::id()));
        let source = directory.join("source");
        let target = directory.join("target");
        fs::create_dir_all(source.join("addons/godot_qa")).unwrap();
        assert!(
            Command::new("git")
                .arg("init")
                .current_dir(&source)
                .output()
                .unwrap()
                .status
                .success()
        );
        fs::write(source.join("addons/godot_qa/plugin.cfg"), "public addon").unwrap();
        assert!(
            Command::new("git")
                .args(["add", "addons/godot_qa/plugin.cfg"])
                .current_dir(&source)
                .output()
                .unwrap()
                .status
                .success()
        );
        fs::write(
            source.join("addons/godot_qa/signing.local"),
            "private machine settings",
        )
        .unwrap();
        copy_tracked_tree(&source, Path::new("addons/godot_qa"), &target).unwrap();
        assert_eq!(
            fs::read_to_string(target.join("plugin.cfg")).unwrap(),
            "public addon"
        );
        assert!(!target.join("signing.local").exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn exit_fixture_recognizes_only_declared_process_demos() {
        assert!(contains_exit_demo(
            &serde_json::json!({"scenario": {"name": "process_exit_strict_failure_demo"}})
        ));
        assert!(contains_exit_demo(
            &serde_json::json!({"name": "process_exit_warning_policy_demo"})
        ));
        assert!(!contains_exit_demo(
            &serde_json::json!({"name": "sample_main_menu_start"})
        ));
    }
}
