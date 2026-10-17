use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .components()
        .collect()
}

fn read(relative_path: &str) -> String {
    fs::read_to_string(repo_root().join(relative_path)).unwrap()
}

#[test]
fn android_live_session_contract_is_documented() {
    let readme = read("docs/workflows.md");
    let cli = read("docs/cli-spec.md");
    let protocol = read("docs/runtime-bridge-protocol.md");
    let artifacts = read("docs/artifact-format.md");
    let architecture = read("docs/architecture.md");
    let decisions = read("docs/implementation-decisions.md");
    let security = read("docs/security-model.md");
    let tests = read("docs/test-strategy.md");
    let acceptance = read("docs/acceptance-matrix.md");
    let addon = read("addons/godot_qa/README.md");
    let sample = read("examples/sample_project/README.md");

    assert!(readme.contains("--target android"));
    assert!(cli.contains("--preset <android-debug-preset>"));
    assert!(cli.contains("android_option_unsupported"));
    assert!(cli.contains("deployment_mode: \"debug_apk\""));
    assert!(cli.contains("release/AAB deployment"));
    assert!(protocol.contains("127.0.0.1"));
    assert!(protocol.contains("OS.is_debug_build()"));
    assert!(protocol.contains("user://godot_qa/live/<session-id>"));
    assert!(artifacts.contains("adb exec-out run-as <package> cat"));
    assert!(architecture.contains("target-aware"));
    assert!(decisions.contains("Android live sessions reuse the authenticated bridge"));
    assert!(security.contains("never"));
    assert!(security.contains("process argv"));
    assert!(tests.contains("Fake Godot and"));
    assert!(tests.contains("ADB executables"));
    assert!(acceptance.contains("Android live-session acceptance"));
    assert!(addon.contains("Android debug template builds"));
    assert!(sample.contains("org.godotqa.sample.androidlive"));
}

#[test]
fn android_apk_cache_contract_is_documented() {
    let readme = read("docs/workflows.md");
    let cli = read("docs/cli-spec.md");
    let decisions = read("docs/implementation-decisions.md");
    let artifacts = read("docs/artifact-format.md");
    let acceptance = read("docs/acceptance-matrix.md");
    let tests = read("docs/test-strategy.md");

    assert!(cli.contains("--no-apk-cache"));
    assert!(cli.contains(".qa/cache/android/"));
    assert!(cli.contains("apk_cache"));
    assert!(readme.contains("--no-apk-cache"));
    assert!(!readme.contains("APK caching"));
    assert!(decisions.contains("Android debug-APK cache is conservative and self-healing"));
    assert!(artifacts.contains(".qa/cache/android/"));
    assert!(acceptance.contains("Android APK cache"));
    assert!(tests.contains("APK cache"));
}

#[test]
fn android_one_shot_scenario_contract_is_documented() {
    let readme = read("docs/workflows.md");
    let cli = read("docs/cli-spec.md");
    let protocol = read("docs/runtime-bridge-protocol.md");
    let artifacts = read("docs/artifact-format.md");
    let acceptance = read("docs/acceptance-matrix.md");

    assert!(cli.contains("Android one-shot scenario execution"));
    assert!(cli.contains("android_scenario_unsupported"));
    assert!(cli.contains("android_app_crashed"));
    assert!(cli.contains("android_session_active"));
    assert!(
        protocol.contains("user://godot_qa/live/<session-id>/run")
            || artifacts.contains("user://godot_qa/live/<session-id>/run")
    );
    assert!(acceptance.contains("Android one-shot scenario"));
    assert!(readme.contains("one-shot `scenario run`"));
}

#[test]
fn android_live_edit_contract_is_documented() {
    let readme = read("docs/workflows.md");
    let cli = read("docs/cli-spec.md");
    let acceptance = read("docs/acceptance-matrix.md");
    let tests = read("docs/test-strategy.md");

    assert!(cli.contains("edit-live set/call/diff/export-patch"));
    assert!(cli.contains("clears the in-memory edit ledger"));
    assert!(cli.contains("no device file round-trip"));
    assert!(readme.contains("edit-live"));
    assert!(acceptance.contains("live edit"));
    assert!(tests.contains("edit.set_property"));
}

#[test]
fn readme_and_decisions_document_rust_binary_install_contract() {
    let readme = read("README.md");
    let decisions = read("docs/implementation-decisions.md");
    for required in [
        "cargo install --path crates/godot-qa-cli",
        "Download the published archive for your platform",
        "cargo xtask prepare-sample",
        "Windows",
        "Linux",
        "Godot 4.5.2",
    ] {
        assert!(
            readme.contains(required),
            "README install contract missing: {required}"
        );
    }
    assert!(decisions.contains("published release archives and Cargo source builds"));
    assert!(decisions.contains("release verification exercise the Rust binary"));
}

#[test]
fn s46_scenario_profile_does_not_use_host_perf() {
    let root = repo_root();
    let mut rust_files = vec![
        root.join("crates/godot-qa-cli/src/main.rs"),
        root.join("crates/godot-qa-process/src/lib.rs"),
        root.join("crates/godot-qa-runtime/src/lib.rs"),
        root.join("crates/godot-qa-core/src/lib.rs"),
    ];
    for entry in fs::read_dir(root.join("crates/godot-qa-scenario/src/run")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|value| value.to_str()) == Some("rs") {
            rust_files.push(path);
        }
    }
    for path in rust_files {
        let contents = fs::read_to_string(&path).unwrap();
        let relative_path = path.strip_prefix(&root).unwrap().display();
        assert!(
            !contents.contains("Command::new(\"perf\")"),
            "{relative_path} must not invoke Linux perf"
        );
        assert!(
            !contents.contains("/proc/sys/kernel/perf_event_paranoid"),
            "{relative_path} must not depend on perf_event_paranoid"
        );
    }
}

#[test]
fn sample_project_addon_preparation_is_documented() {
    let root = repo_root();
    assert!(root.join("addons/godot_qa/plugin.cfg").is_file());
    assert!(read("README.md").contains("cargo xtask prepare-sample"));
    assert!(read("examples/sample_project/README.md").contains("cargo xtask prepare-sample"));
    assert!(read("CONTRIBUTING.md").contains("Never edit the generated copy"));
}

#[test]
fn sample_project_ui_is_the_only_checked_in_ui_tree() {
    let root = repo_root();
    assert!(!root.join("ui").exists());
    assert!(
        root.join("examples/sample_project/ui/main_menu.tscn")
            .is_file()
    );
    assert!(
        root.join("examples/sample_project/ui/settings_menu.tscn")
            .is_file()
    );
}

#[test]
fn contributor_docs_describe_supported_workflows() {
    let agents = read("AGENTS.md");
    let architecture = read("docs/architecture.md");
    let test_strategy = read("docs/test-strategy.md");
    let runtime_protocol = read("docs/runtime-bridge-protocol.md");
    let release_checklist = read("docs/release-checklist.md");
    let acceptance_matrix = read("docs/acceptance-matrix.md");
    let mcp_wrapper = read("docs/mcp.md");
    let gitignore = read(".gitignore");

    assert!(agents.contains("The shipped CLI/library is the Rust Cargo workspace"));
    assert!(agents.contains("cargo test"));
    assert!(agents.contains("godot-qa doctor --json"));
    assert!(!agents.contains("python -m godot_qa doctor --json"));
    assert!(!agents.contains("python -m pytest"));

    assert!(architecture.contains("The shipped CLI/library is the Rust Cargo workspace"));
    assert!(test_strategy.contains("The shipped verification baseline is Rust-only."));
    assert!(runtime_protocol.contains("The shipped Rust CLI/runtime client preserves these names"));
    assert!(
        runtime_protocol.contains("the CLI/library turns into a replayable candidate scenario")
    );
    assert!(!runtime_protocol.contains("The Python CLI keeps the names"));
    assert!(
        !runtime_protocol.contains("the Python CLI turns into a replayable candidate scenario")
    );
    assert!(!release_checklist.contains("pre-Rust-cutover"));
    assert!(release_checklist.contains("Publish one SHA256 checksum file"));
    assert!(acceptance_matrix.contains("Portable release packaging"));
    assert!(!acceptance_matrix.contains("Planned next spec wave"));
    assert!(
        !acceptance_matrix.contains("Planned implementation-platform migration wave after `S27`")
    );
    assert!(mcp_wrapper.contains("\"command\": \"godot-qa\""));
    assert!(!mcp_wrapper.contains("\"command\": \"python\""));
    assert!(!release_checklist.contains("python scripts/sync_sample_project_addon.py"));
    assert!(gitignore.contains("build/"));
    assert!(gitignore.contains("dist/"));
    assert!(gitignore.contains("target/"));
}

#[test]
fn bundled_skill_docs_and_contract_are_repo_local() {
    let readme = read("docs/workflows.md");
    let cli_spec = read("docs/cli-spec.md");
    let architecture = read("docs/architecture.md");
    let skill = read("skills/godot-qa/SKILL.md");
    let skill_metadata = read("skills/godot-qa/agents/openai.yaml");

    assert!(readme.contains("godot-qa install-skills --project ."));
    assert!(readme.contains("npx skills add <repo> --skill godot-qa"));
    assert!(readme.contains("does not shell out to Node, npm, or `npx skills add`"));
    assert!(cli_spec.contains("install-skills"));
    assert!(cli_spec.contains(".agents/skills/godot-qa"));
    assert!(cli_spec.contains("source_path: null"));
    assert!(cli_spec.contains("skill_target_conflict"));
    assert!(!cli_spec.contains("shells out to `npx skills add`"));
    assert!(architecture.contains("Bundled AI skill packaging lives under `skills/godot-qa/`"));

    assert!(skill.contains("name: godot-qa"));
    assert!(skill.contains("references/cli-workflows.md"));
    assert!(skill_metadata.contains("display_name: \"Godot QA\""));
}

#[test]
fn temporal_contract_docs_are_present() {
    let scenario_format = read("docs/scenario-format.md");
    let cli_spec = read("docs/cli-spec.md");
    let runtime_protocol = read("docs/runtime-bridge-protocol.md");
    let test_strategy = read("docs/test-strategy.md");
    let artifact_format = read("docs/artifact-format.md");
    let architecture = read("docs/architecture.md");

    assert!(scenario_format.contains("max_delta_position"));
    assert!(scenario_format.contains("follows"));
    assert!(scenario_format.contains("never"));
    assert!(scenario_format.contains("within_limits"));
    assert!(scenario_format.contains("window_frames"));
    assert!(scenario_format.contains("window_seconds"));
    assert!(scenario_format.contains("sample_every_frames"));

    assert!(cli_spec.contains("godot-qa assert max-delta-position"));
    assert!(cli_spec.contains("--window-frames"));
    assert!(cli_spec.contains("--max"));
    assert!(cli_spec.contains("--live"));

    assert!(runtime_protocol.contains("assert.temporal.max_delta_position"));
    assert!(runtime_protocol.contains("assert.temporal.follows"));
    assert!(runtime_protocol.contains("assert.temporal.never"));
    assert!(runtime_protocol.contains("assert.temporal.within_limits"));

    assert!(test_strategy.contains("Temporal assertion validation guidance"));
    assert!(artifact_format.contains("Temporal evidence artifacts"));
    assert!(artifact_format.contains("failure.artifacts.temporal_evidence"));
    assert!(architecture.contains("share this same runtime assertion layer"));
}

#[test]
fn temporal_observe_until_docs_contract() {
    let scenario_format = read("docs/scenario-format.md");
    let cli_spec = read("docs/cli-spec.md");
    let runtime_protocol = read("docs/runtime-bridge-protocol.md");
    let test_strategy = read("docs/test-strategy.md");
    let architecture = read("docs/architecture.md");

    assert!(scenario_format.contains("determinism.temporal_sampling"));
    assert!(scenario_format.contains("temporal_sampling"));
    assert!(scenario_format.contains("fixed_delta"));
    assert!(scenario_format.contains("max_frame_duration"));
    assert!(scenario_format.contains("observe_until"));
    assert!(scenario_format.contains("property"));
    assert!(scenario_format.contains("visible"));
    assert!(scenario_format.contains("matches"));
    assert!(scenario_format.contains("sample_every_frames"));
    assert!(scenario_format.contains("Malformed state paths"));
    assert!(scenario_format.contains("non-JSON-compatible method results"));

    assert!(runtime_protocol.contains("assert.observe_until"));
    assert!(runtime_protocol.contains("first_matching_sample"));
    assert!(runtime_protocol.contains("observed_samples"));
    assert!(runtime_protocol.contains("first_late_sample"));
    assert!(runtime_protocol.contains("why_no_match"));
    assert!(runtime_protocol.contains("temporal_evidence"));
    assert!(runtime_protocol.contains("condition"));
    assert!(runtime_protocol.contains("sample_every_frames"));

    assert!(cli_spec.contains("scenario run <name>"));
    assert!(cli_spec.contains("observe_until"));
    assert!(cli_spec.contains("determinism.temporal_sampling"));
    assert!(cli_spec.contains("assert.observe_until"));
    assert!(cli_spec.contains("S33 CLI aliases and exit-code semantics are unchanged"));
    assert!(cli_spec.contains("observed_samples"));
    assert!(cli_spec.contains("first_late_sample"));
    assert!(cli_spec.contains("artifacts.temporal_evidence"));

    assert!(test_strategy.contains("temporal_observe_until_demo"));
    assert!(test_strategy.contains("temporal_observe_until_late_failure_demo"));
    assert!(test_strategy.contains("temporal_observe_until_failure_contract"));
    assert!(test_strategy.contains("temporal_observe_until_docs_contract"));
    assert!(test_strategy.contains("artifacts.temporal_evidence"));
    assert!(test_strategy.contains("first_matching_sample"));
    assert!(test_strategy.contains("first_late_sample"));
    assert!(test_strategy.contains("why_no_match"));

    assert!(architecture.contains("observe_until"));
    assert!(architecture.contains("assert.observe_until"));
    assert!(architecture.contains("CLI remains the orchestration source of truth"));
}

#[test]
fn repo_contract_time_control_docs() {
    let scenario_format = read("docs/scenario-format.md");
    let cli_spec = read("docs/cli-spec.md");
    let runtime_protocol = read("docs/runtime-bridge-protocol.md");
    let artifact_format = read("docs/artifact-format.md");
    let test_strategy = read("docs/test-strategy.md");
    let architecture = read("docs/architecture.md");

    for doc in [
        &scenario_format,
        &cli_spec,
        &runtime_protocol,
        &artifact_format,
        &test_strategy,
        &architecture,
    ] {
        assert!(doc.contains("time_control"));
        assert!(doc.contains("Engine.time_scale"));
    }

    assert!(scenario_format.contains("<= 64"));
    assert!(scenario_format.contains("does not accelerate audio playback speed"));
    assert!(scenario_format.contains("physics precision"));
    assert!(scenario_format.contains("time_control_physics_pairing_missing"));

    assert!(cli_spec.contains("data.time_control"));
    assert!(cli_spec.contains("data.time_control_warnings"));
    assert!(cli_spec.contains("scenario_validation_failed"));
    assert!(cli_spec.contains("test_manifest_invalid"));

    assert!(runtime_protocol.contains("not a live-session default"));
    assert!(runtime_protocol.contains("restored_matches_original"));
    assert!(runtime_protocol.contains("restoration_status"));
    assert!(runtime_protocol.contains("session.set_time_scale"));
    assert!(runtime_protocol.contains("persistent"));
    assert!(cli_spec.contains("session time-scale"));
    assert!(cli_spec.contains("persistent: true"));

    for field in [
        "original",
        "requested",
        "effective",
        "restored",
        "restored_matches_original",
        "restoration_status",
        "warnings",
    ] {
        assert!(
            artifact_format.contains(field),
            "missing artifact field {field}"
        );
    }
    assert!(artifact_format.contains("profile.json.time_control"));
    assert!(artifact_format.contains("request.json.run.time_control"));
    assert!(artifact_format.contains("result.json"));

    assert!(test_strategy.contains("does not accelerate audio playback speed"));
    assert!(test_strategy.contains("physics precision"));
    assert!(test_strategy.contains("baseline approval remains manual"));

    assert!(architecture.contains("CLI validates requested `time_control` schema"));
    assert!(architecture.contains("Godot addon owns the effective Engine state"));
    assert!(architecture.contains("session time-scale"));
}

#[test]
fn check_tests_selection_docs_contract() {
    let readme = read("docs/workflows.md");
    let cli_spec = read("docs/cli-spec.md");
    let test_strategy = read("docs/test-strategy.md");

    for doc in [&readme, &cli_spec, &test_strategy] {
        assert!(
            doc.contains("--test sample_script_test"),
            "missing sample --test command"
        );
        assert!(doc.contains("--tag smoke"), "missing smoke tag command");
        assert!(
            doc.contains("--profile smoke"),
            "missing smoke profile command"
        );
    }

    for required in [
        "\"tags\"",
        "\"profiles\"",
        "selected_test_count",
        "skipped_test_count",
        "total_test_count",
        "\"selection\"",
        "duplicate test ids",
        "duplicate tags",
        "unknown `--test`, `--tag`, or `--profile`",
        "filters that select no tests",
        "malformed `profiles`",
        "reserved tags",
        "check --tests --test sample_script_test --project examples/sample_project --json",
        "check --tests --tag smoke --project examples/sample_project --json",
        "check --tests --profile smoke --project examples/sample_project --json",
        "check --tests --project examples/sample_project --json",
    ] {
        assert!(
            cli_spec.contains(required),
            "docs/cli-spec.md missing S60 contract text: {required}"
        );
    }

    for required in [
        "S60 check-test selection guardrails",
        "total_test_count",
        "selected_test_count",
        "skipped_test_count",
        "selection",
        "Filtered runs must not mask selected-test failures",
        "Skipped tests are accounting metadata only",
        "check --tests --test sample_script_test --project examples/sample_project --json",
        "check --tests --tag smoke --project examples/sample_project --json",
        "check --tests --profile smoke --project examples/sample_project --json",
        "check --tests --project examples/sample_project --json",
    ] {
        assert!(
            test_strategy.contains(required),
            "docs/test-strategy.md missing S60 contract text: {required}"
        );
    }

    for required in [
        "\"tags\"",
        "\"profiles\"",
        "Manifest tags are optional",
        "check --project /path/to/your/project --tests --test sample_script_test --json",
        "check --project /path/to/your/project --tests --tag smoke --json",
        "check --project /path/to/your/project --tests --profile smoke --json",
        "check --project /path/to/your/project --tests --json",
    ] {
        assert!(
            readme.contains(required),
            "docs/workflows.md missing S60 contract text: {required}"
        );
    }
}

#[test]
fn repo_contract_check_tests_batch_docs() {
    let readme = read("docs/workflows.md");
    let cli_spec = read("docs/cli-spec.md");
    let artifact_format = read("docs/artifact-format.md");
    let test_strategy = read("docs/test-strategy.md");

    for (name, doc) in [
        ("README.md", &readme),
        ("docs/cli-spec.md", &cli_spec),
        ("docs/artifact-format.md", &artifact_format),
        ("docs/test-strategy.md", &test_strategy),
    ] {
        assert!(doc.contains("--batch"), "{name} missing --batch");
        assert!(
            doc.contains("execution_mode"),
            "{name} missing execution_mode"
        );
        assert!(
            doc.contains("batch=false") || doc.contains("\"batch\": false"),
            "{name} missing batch=false opt-out"
        );
    }

    for required in [
        "godot-qa check --project /path/to/your/project --tests --batch --tag smoke --json",
        "Batching is opt-in and never the default",
        "\"batch\": false",
    ] {
        assert!(
            readme.contains(required),
            "docs/workflows.md missing S61 contract text: {required}"
        );
    }

    for required in [
        "check --tests --batch --tag smoke --project examples/sample_project --json",
        "data.execution_mode: \"batch\"",
        "data.execution_mode: \"isolated\"",
        "data.artifacts.batch_summary",
        ".qa/artifacts/check/<run-id>/batch/summary.json",
        "Ordinary selected-test failures fail the aggregate command but do not stop the runner from attempting later selected tests",
        "held input state",
        "unreleased scene instances",
        "unreleased script instances",
        "modified `Engine` settings",
        "runtime errors observed between entries",
    ] {
        assert!(
            cli_spec.contains(required),
            "docs/cli-spec.md missing S61 contract text: {required}"
        );
    }

    for required in [
        ".qa/artifacts/check/<run-id>/batch/",
        "batch/request.json",
        "batch/logs.txt",
        "batch/summary.json",
        ".qa/artifacts/check/<run-id>/tests/<test-id>/",
        "shared batch stream",
        "held input",
        "scene/script instances",
        "`Engine` settings",
        "runtime errors between entries",
    ] {
        assert!(
            artifact_format.contains(required),
            "docs/artifact-format.md missing S61 contract text: {required}"
        );
    }

    for required in [
        "S61 check-test batch guardrails",
        "Batch validation supplements the isolated `check --tests` regression coverage",
        "cargo test repo_contract_check_tests_batch_docs",
        "cargo run --bin godot-qa -- check --tests --batch --tag smoke --project examples/sample_project --json",
        "batch=false",
        "Ordinary selected-test failures should fail the aggregate while preserving later selected results",
        "held input",
        "scene/script instances",
        "`Engine` settings",
        "runtime errors between entries",
    ] {
        assert!(
            test_strategy.contains(required),
            "docs/test-strategy.md missing S61 contract text: {required}"
        );
    }
}

#[test]
fn repo_contract_progress_timeouts_slow_summaries_docs() {
    let cli_spec = read("docs/cli-spec.md");
    let artifact_format = read("docs/artifact-format.md");
    let runtime_protocol = read("docs/runtime-bridge-protocol.md");
    let scenario_format = read("docs/scenario-format.md");
    let test_strategy = read("docs/test-strategy.md");

    for (name, doc) in [
        ("docs/cli-spec.md", &cli_spec),
        ("docs/artifact-format.md", &artifact_format),
        ("docs/runtime-bridge-protocol.md", &runtime_protocol),
        ("docs/scenario-format.md", &scenario_format),
        ("docs/test-strategy.md", &test_strategy),
    ] {
        assert!(
            doc.contains("progress.jsonl"),
            "{name} missing progress.jsonl"
        );
        assert!(
            doc.contains("slow-summary.json"),
            "{name} missing slow-summary.json"
        );
        assert!(
            doc.contains("check_test_timeout"),
            "{name} missing check_test_timeout"
        );
        assert!(
            doc.contains("scenario_run_timeout"),
            "{name} missing scenario_run_timeout"
        );
    }

    for required in [
        "--timeout-seconds",
        "manifest-level and entry-level `timeout_seconds`",
        "CLI `--timeout-seconds <seconds>` takes precedence",
        "zero, negative values, non-numbers, and unsupported duration strings",
        "data.artifacts.progress",
        ".qa/artifacts/check/<run-id>/progress.jsonl",
        ".qa/artifacts/check/<run-id>/slow-summary.json",
        "check --tests --test slow_manifest_test --timeout-seconds 5 --project examples/sample_project --json",
    ] {
        assert!(
            cli_spec.contains(required),
            "docs/cli-spec.md missing S62 contract text: {required}"
        );
    }

    for required in [
        ".qa/artifacts/check/<run-id>/progress.jsonl",
        ".qa/artifacts/check/<run-id>/slow-summary.json",
        ".qa/artifacts/<scenario-name>/<run-id>/progress.jsonl",
        ".qa/artifacts/<scenario-name>/<run-id>/slow-summary.json",
        "slow_manifest_test",
        "slow_step_progress_demo",
        "Partial artifacts are diagnostic evidence only",
        "process-list inspection",
    ] {
        assert!(
            artifact_format.contains(required),
            "docs/artifact-format.md missing S62 contract text: {required}"
        );
    }

    for required in [
        "qa_runner.gd",
        "before and after each runner-owned step or assertion",
        "Batch `check --tests` progress events",
        "Session tokens are never written to progress artifacts",
        "Timeout diagnostics use the most recent progress event",
    ] {
        assert!(
            runtime_protocol.contains(required),
            "docs/runtime-bridge-protocol.md missing S62 contract text: {required}"
        );
    }

    for required in [
        "timeout_seconds",
        "run_timeout_seconds",
        "`ms`, `s`, or `m` suffixes",
        "unsupported duration strings",
        "Progress artifacts redact sensitive context",
        "slow_summary.threshold_ms",
        "slow_step_progress_demo",
        "scenario_frame_budget_exceeded",
        "run.frame_budget",
    ] {
        assert!(
            scenario_format.contains(required),
            "docs/scenario-format.md missing S62 contract text: {required}"
        );
    }

    for required in ["scenario_frame_budget_exceeded", "run.frame_budget"] {
        assert!(
            cli_spec.contains(required),
            "docs/cli-spec.md missing frame-budget contract text: {required}"
        );
    }

    for required in [
        "S62 progress, timeout, and slow-summary guardrails",
        "cargo test repo_contract_progress_timeouts_slow_summaries_docs",
        "cargo run --bin godot-qa -- check --tests --tag smoke --project examples/sample_project --json",
        "cargo run --bin godot-qa -- scenario run sample_setup_steps --project examples/sample_project --json",
        "cargo run --bin godot-qa -- check --tests --test slow_manifest_test --timeout-seconds 5 --project examples/sample_project --json",
        "cargo run --bin godot-qa -- scenario run slow_step_progress_demo --project examples/sample_project --json",
        "process-list inspection",
        "session tokens and secret-looking project args",
    ] {
        assert!(
            test_strategy.contains(required),
            "docs/test-strategy.md missing S62 contract text: {required}"
        );
    }
}

#[test]
fn proximity_contract_docs_are_present() {
    let cli_spec = read("docs/cli-spec.md");
    let scenario_format = read("docs/scenario-format.md");
    let runtime_protocol = read("docs/runtime-bridge-protocol.md");
    let test_strategy = read("docs/test-strategy.md");

    assert!(cli_spec.contains("assert.distance"));
    assert!(scenario_format.contains("distance_changed"));
    assert!(runtime_protocol.contains("unsupported_coordinate_space"));
    assert!(test_strategy.contains("qa:actor"));
    assert!(test_strategy.contains("proximity_assertions_demo"));
}

#[test]
fn touch_drag_wheel_contract_docs_are_present() {
    let cli_spec = read("docs/cli-spec.md");
    let runtime_protocol = read("docs/runtime-bridge-protocol.md");

    assert!(cli_spec.contains("## Touch, drag, and wheel input commands (S78 additive contract)"));
    assert!(cli_spec.contains("input mouse wheel"));
    assert!(cli_spec.contains("input touch down"));
    assert!(cli_spec.contains("input drag"));
    assert!(runtime_protocol.contains("input.mouse_wheel"));
    assert!(runtime_protocol.contains("input.touch_down"));
    assert!(runtime_protocol.contains("input.drag"));
}

#[test]
fn text_entry_contract_docs_are_present() {
    let cli_spec = read("docs/cli-spec.md");
    let runtime_protocol = read("docs/runtime-bridge-protocol.md");

    assert!(cli_spec.contains("input type"));
    assert!(cli_spec.contains("unicode"));
    assert!(runtime_protocol.contains("input.text"));
    assert!(runtime_protocol.contains("unicode"));
}

#[test]
fn visual_assertion_docs_preserve_manual_baseline_approval_contract() {
    let cli_spec = read("docs/cli-spec.md");
    let artifact_format = read("docs/artifact-format.md");
    let runtime_protocol = read("docs/runtime-bridge-protocol.md");
    let scenario_format = read("docs/scenario-format.md");
    let test_strategy = read("docs/test-strategy.md");

    assert!(scenario_format.contains("visible_pixels"));
    assert!(scenario_format.contains("do not replace `visible`, `hidden`, layout checks, or manually reviewed screenshot baselines"));
    assert!(cli_spec.contains("assert visible-pixels"));
    assert!(cli_spec.contains("without approving or updating `qa/baselines`"));
    assert!(cli_spec.contains(
        "Unsupported or ambiguous visual evidence fails with documented limitation payloads"
    ));
    assert!(cli_spec.contains("real rendered pixels"));
    assert!(cli_spec.contains("non-visual state"));
    assert!(runtime_protocol.contains("qa_visual_rect"));
    assert!(runtime_protocol.contains("qa_set_visual_visible"));
    assert!(runtime_protocol.contains("Line2D"));
    assert!(runtime_protocol.contains("unsupported_coordinate_space"));
    assert!(runtime_protocol.contains("screenshot_capture_failed"));
    assert!(scenario_format.contains("non-visual state/probe assertions"));
    assert!(artifact_format.contains("target_hidden"));
    assert!(artifact_format.contains("must not update `qa/baselines`"));
    assert!(test_strategy.contains("Low-information screenshot"));
    assert!(test_strategy.contains("Visual assertion limitations are intentional"));
    assert!(test_strategy.contains("S44 visual evidence validation guidance"));

    // Canvas-vs-screenshot coordinate-space handling for visible_pixels: the
    // additive evidence fields and the loud geometry-mismatch guard must stay
    // documented across the scenario, protocol, and artifact contracts.
    assert!(scenario_format.contains("target_rect_screenshot"));
    assert!(scenario_format.contains("canvas_to_screenshot"));
    assert!(scenario_format.contains("screenshot_geometry_mismatch"));
    assert!(runtime_protocol.contains("target_rect_screenshot"));
    assert!(runtime_protocol.contains("canvas_to_screenshot"));
    assert!(runtime_protocol.contains("screenshot_size"));
    assert!(runtime_protocol.contains("screenshot_geometry_mismatch"));
    assert!(runtime_protocol.contains("Window.get_final_transform()"));
    assert!(artifact_format.contains("target_rect_screenshot"));
    assert!(artifact_format.contains("rect_screenshot"));
    assert!(artifact_format.contains("screenshot_geometry_mismatch"));
}

#[test]
fn scenario_lint_docs_contract_is_present() {
    let cli_spec = read("docs/cli-spec.md");
    let scenario_format = read("docs/scenario-format.md");
    let test_strategy = read("docs/test-strategy.md");
    let readme = read("docs/workflows.md");

    assert!(cli_spec.contains(
        "godot-qa scenario lint --project <project> [--scenario <name>] [--strict] --json"
    ));
    assert!(cli_spec.contains("warning-only lint findings keep exit status `0`"));
    assert!(cli_spec.contains(
        "`--strict` treats warning-level findings as failing for exit status without changing their `warning` severity"
    ));
    assert!(cli_spec.contains("Contract errors"));
    assert!(cli_spec.contains("always exit non-zero in both modes"));
    assert!(cli_spec.contains("data.findings"));
    assert!(cli_spec.contains("data.suppressed_findings"));

    assert!(scenario_format.contains("## Lint metadata (S37)"));
    assert!(scenario_format.contains("GQA-LINT-001"));
    assert!(scenario_format.contains("GQA-LINT-002"));
    assert!(scenario_format.contains("GQA-LINT-003"));
    assert!(scenario_format.contains("GQA-LINT-004"));
    assert!(scenario_format.contains("GQA-LINT-005"));
    assert!(scenario_format.contains("GQA-LINT-008"));
    assert!(scenario_format.contains("scenario_document_normalized"));
    assert!(scenario_format.contains("missing_qa_id_reference"));
    // The obsolete `missing_scenario_reference` rule was replaced by load-time
    // template resolution: a missing base now fails with `scenario_extends_missing`.
    assert!(!scenario_format.contains("missing_scenario_reference"));
    assert!(scenario_format.contains("scenario_extends_missing"));
    assert!(scenario_format.contains("duplicate_scenario_name"));
    assert!(scenario_format.contains("malformed_suppression"));
    assert!(scenario_format.contains("lint.suppressions.<rule_id>.reason"));
    assert!(scenario_format.contains("Contract errors are never suppressible"));

    assert!(test_strategy.contains("Scenario lint validation guidance (S37):"));
    assert!(test_strategy.contains("intentionally bad lint fixtures are valid test assets"));
    assert!(
        test_strategy
            .contains("should not be included in happy-path `scenario run` regression gates")
    );

    assert!(readme.contains("scenario lint"));
}

#[test]
fn repo_contract_scenario_template_docs_contract_is_present() {
    let scenario_format = read("docs/scenario-format.md");
    let cli_spec = read("docs/cli-spec.md");
    let implementation_decisions = read("docs/implementation-decisions.md");

    for required in [
        "## Scenario templates (extends + vars)",
        "### `extends`",
        "### Merge rules",
        "### `vars` and `${}` substitution",
        "### `abstract: true`",
        "### Template error kinds",
        "relative to the current file's directory",
        "Arrays and scalars replace wholesale.",
        "is the author's deletion mechanism",
        "wait_frames: \"${frames}\"",
        "`$${name}` escapes to a literal `${name}`",
        "scenario_abstract",
        "scenario_extends_missing",
        "scenario_extends_cycle",
        "scenario_extends_invalid",
        "scenario_vars_invalid",
        "scenario_var_unresolved",
    ] {
        assert!(
            scenario_format.contains(required),
            "docs/scenario-format.md missing scenario template contract text: {required}"
        );
    }

    assert!(cli_spec.contains(
        "Abstract scenario templates (files with top-level `abstract: true`) are excluded from `scenario list`"
    ));
    assert!(
        implementation_decisions
            .contains("Decision 012: Scenario template vars are file-static; no `--var` CLI flag")
    );
}

#[test]
fn repo_contract_scenario_bundle_docs_contract_is_present() {
    let cli_spec = read("docs/cli-spec.md");
    let artifact_format = read("docs/artifact-format.md");
    let scenario_format = read("docs/scenario-format.md");
    let test_strategy = read("docs/test-strategy.md");
    let readme = read("docs/workflows.md");
    let sample_readme = read("examples/sample_project/README.md");

    assert!(cli_spec.contains("godot-qa scenario bundle list --json"));
    assert!(
        cli_spec.contains(
            "godot-qa scenario bundle run <name> [--jobs <signed-count>] [--strict] [--headless|--windowed] --json"
        )
    );
    assert!(
        cli_spec.contains("\"path\": \"/path/to/project/qa/gates/malformed_bundle_demo.yaml\"")
    );
    assert!(cli_spec.contains("\"artifacts\""));
    assert!(cli_spec.contains("\"meta\""));
    assert!(cli_spec.contains("\"summary\""));
    assert!(cli_spec.contains("\"result\""));
    assert!(cli_spec.contains("required_bundles"));
    assert!(cli_spec.contains("composed_regression_gate"));
    assert!(cli_spec.contains("screenshot_heavy_regression_gate"));
    assert!(cli_spec.contains("source_bundle"));
    assert!(cli_spec.contains("bundle_chain"));
    assert!(cli_spec.contains("godot_args"));
    assert!(cli_spec.contains("project_args"));
    assert!(cli_spec.contains("run_root"));
    assert!(cli_spec.contains("launch_display"));
    assert!(cli_spec.contains("windowed_launch_prefix"));
    assert!(cli_spec.contains("effective_command_prefix"));
    assert!(cli_spec.contains("bundle_cycle"));
    assert!(cli_spec.contains("`launch_args` fields are not part of `artifact-index.json`"));

    // Bundle timing and failure ergonomics: the bundle has its own wall clock, the
    // flat failure list names the child it came from, and one shared cause reads as
    // one cluster instead of N identical failures.
    assert!(cli_spec.contains("`data.timing`"));
    assert!(cli_spec.contains("sum_child_duration_ms"));
    assert!(cli_spec.contains("parallel_efficiency"));
    assert!(cli_spec.contains("queue_wait_ms"));
    assert!(cli_spec.contains("data.failure_clusters"));
    assert!(cli_spec.contains("{kind, key, scenario_count, failure_count, scenarios, hint}"));
    assert!(cli_spec.contains("no failure is suppressed, reordered, or rewritten"));
    assert!(cli_spec.contains("its `wall_ms` / `finished_at_ms` exclude the summary write itself"));
    assert!(
        artifact_format
            .contains("Bundle `summary.json` also carries `timing` and `failure_clusters`")
    );
    assert!(artifact_format.contains("`lane`, and `queue_wait_ms`"));

    // Bundle child selection, dispatch order, and the `latest` convenience link.
    // Selection failures are invocation failures (exit 2, nothing written); an empty
    // selection is a warning, not a failure.
    assert!(
        cli_spec
            .contains("[--only <scenario>]... [--failed-only] [--schedule manifest|longest-first]")
    );
    // Several bundles in one invocation: the usage line takes a list, the envelope reports
    // one verdict, and the argument shapes that cannot mean anything are refused.
    assert!(cli_spec.contains("godot-qa scenario bundle run <name> [<name>...]"));
    assert!(cli_spec.contains("**Running several bundles.**"));
    assert!(cli_spec.contains("`data.bundles[]`"));
    assert!(cli_spec.contains("scenario bundle run completed (N bundles)"));
    assert!(cli_spec.contains("scenario bundle run failed (K of N bundles failed)"));
    assert!(cli_spec.contains("invalid_arguments"));
    assert!(cli_spec.contains("runnable_count_total"));
    assert!(cli_spec.contains(
        "Several bundles named in one `--warm` invocation share ONE pool, and therefore one skip decision."
    ));
    assert!(test_strategy.contains("scenario bundle run smooth_gate lobby_gate"));
    assert!(cli_spec.contains("**Selecting children.**"));
    assert!(cli_spec.contains("**Dispatch order.**"));
    assert!(cli_spec.contains("selection_unknown_scenario"));
    assert!(cli_spec.contains("selection_no_previous_run"));
    assert!(cli_spec.contains("selection_empty"));
    assert!(cli_spec.contains("`data.selection`"));
    assert!(cli_spec.contains("`data.scheduling`"));
    assert!(cli_spec.contains("longest_first"));
    assert!(cli_spec.contains("`scenario.schedule`"));
    assert!(cli_spec.contains("(filtered: k of n)"));
    assert!(cli_spec.contains("`.qa/artifacts/bundles/<bundle>/latest`"));
    assert!(cli_spec.contains("`data.artifacts.latest_path`"));
    assert!(artifact_format.contains("`selected_ids[]`"));
    assert!(artifact_format.contains("`.qa/artifacts/bundles/<bundle>/latest`"));

    // Warm parallelism-skip semantics: the skip still enforces --warm-strict
    // structurally, so the verdict is host-independent (see run/warm.rs skip fix).
    assert!(cli_spec.contains("no_reuse_below_parallelism"));
    // Every run of a skipped-warm bundle says why it ran cold, and the skip reason
    // is never a --warm-strict violation.
    assert!(cli_spec.contains("warm_skipped_below_parallelism"));
    assert!(cli_spec.contains("warm_fallback_forced_cold"));
    assert!(cli_spec.contains("\"strict_structural_check\": true"));
    assert!(cli_spec.contains("same verdict on any host core count"));
    assert!(cli_spec.contains("host-independent verdict"));

    // Warm pool robustness: a bounded start-failure budget, a bounded wait for a
    // free lane, and the pool evidence that replaced a silently empty warnings[].
    assert!(cli_spec.contains("A failed session start is bounded, not terminal."));
    assert!(cli_spec.contains("tolerates two *consecutive* failed starts"));
    assert!(
        cli_spec.contains("no pooled headless session could be acquired before the wait deadline")
    );
    assert!(cli_spec.contains("without* counting against the boot budget"));
    assert!(cli_spec.contains("data.warm_pool"));
    assert!(cli_spec.contains("session_start_failures"));
    assert!(cli_spec.contains("transport_failures"));
    assert!(cli_spec.contains("acquire_wait_ms_budget"));
    assert!(cli_spec.contains("`acquire_wait_ms`"));
    assert!(cli_spec.contains("\"warm_session_start_failed\""));
    assert!(cli_spec.contains("data.warm.start_failure"));

    assert!(scenario_format.contains("## Scenario bundles"));
    assert!(scenario_format.contains("qa/gates/*.json|yaml|yml"));
    assert!(scenario_format.contains("required_bundles"));
    assert!(scenario_format.contains("required_scenarios"));
    assert!(scenario_format.contains("optional_scenarios"));
    assert!(scenario_format.contains("recommended_manual_checks"));
    assert!(scenario_format.contains("declarative data only"));
    assert!(scenario_format.contains("not a CI scripting language"));
    assert!(scenario_format.contains("composed_regression_gate"));
    assert!(scenario_format.contains("screenshot_heavy_regression_gate"));
    assert!(scenario_format.contains("bundle_godot_args_demo"));
    assert!(scenario_format.contains("recursive_bundle_demo"));
    assert!(scenario_format.contains("source_bundle"));
    assert!(scenario_format.contains("bundle_chain"));
    assert!(scenario_format.contains("godot_args"));
    assert!(scenario_format.contains("launch_display"));
    assert!(scenario_format.contains("audio_capture` selects the real audio driver"));
    assert!(scenario_format.contains("does NOT force a windowed launch"));
    assert!(
        scenario_format.contains("`audio_export` needs neither a window nor the real audio driver")
    );
    assert!(!scenario_format.contains("launch_args"));

    assert!(test_strategy.contains("Scenario bundle validation guidance (S38):"));
    assert!(test_strategy.contains("Scenario bundle validation guidance (S50):"));
    assert!(test_strategy.contains("scenario bundle list"));
    assert!(test_strategy.contains("scenario bundle run malformed_bundle_demo"));
    assert!(test_strategy.contains("scenario bundle run composed_regression_gate"));
    assert!(test_strategy.contains("scenario lint"));
    assert!(test_strategy.contains("direct `scenario run` regressions"));
    assert!(test_strategy.contains("required_bundles"));
    assert!(test_strategy.contains("source_bundle"));
    assert!(test_strategy.contains("bundle_chain"));
    assert!(test_strategy.contains("godot_args"));
    assert!(test_strategy.contains("recursive-bundle cycles"));
    assert!(!test_strategy.contains("launch_args"));

    assert!(readme.contains("scenario bundle"));
    assert!(readme.contains("required_bundles"));
    assert!(readme.contains("composed_regression_gate"));
    assert!(readme.contains("screenshot_heavy_regression_gate"));
    assert!(readme.contains("godot_args"));
    assert!(!readme.contains("launch_args"));
    assert!(sample_readme.contains("sample_regression_gate"));
    assert!(sample_readme.contains("composed_regression_gate"));
    assert!(sample_readme.contains("reusable_menu_gate"));
    assert!(sample_readme.contains("screenshot_heavy_regression_gate"));
    assert!(sample_readme.contains("bundle_godot_args_demo"));
    assert!(sample_readme.contains("recursive_bundle_demo"));
    assert!(!sample_readme.contains("launch_args"));
}

#[test]
fn layout_audit_single_viewport_routing_contract_is_documented() {
    // Single-viewport layout_audit runs the normal single-run path (not the
    // matrix), coexists with screenshot_match, and both new failure kinds and
    // the static screenshot_match+matrix rejection are documented.
    let cli_spec = read("docs/cli-spec.md");
    let scenario_format = read("docs/scenario-format.md");
    let layout_rules = read("docs/layout-audit-rules.md");

    // New failure kinds are pinned in the CLI spec + layout rules.
    assert!(cli_spec.contains("layout_audit_missing_snapshot"));
    assert!(cli_spec.contains("viewport_matrix_empty"));
    assert!(layout_rules.contains("layout_audit_missing_snapshot"));
    assert!(layout_rules.contains("viewport_matrix_empty"));

    // The routing/result-shape change is documented loudly.
    assert!(cli_spec.contains("Zero or one `viewports` entry runs the NORMAL single-run path"));
    assert!(layout_rules.contains("## `layout_audit` in `scenario run`"));
    assert!(
        scenario_format
            .contains("single-viewport `layout_audit` produces the ordinary single-run envelope")
    );

    // The `layout_matrix` warm/cold reason no longer appears as a warm-strict
    // structural reason in the CLI spec (multi_viewport subsumes it); it survives
    // only as the Android one-shot rejection reason.
    assert!(!cli_spec.contains("`layout_matrix`, `fixed_fps_mismatch`"));
    assert!(cli_spec.contains("`layout_matrix` (a layout-audit matrix)"));

    // screenshot_match + multi-viewport is rejected statically (validation), and
    // single-viewport coexistence with layout_audit is documented.
    assert!(scenario_format.contains("rejected statically"));
    assert!(scenario_format.contains("coexist with `layout_audit`"));
}

#[test]
fn screenshot_heavy_regression_gate_contract() {
    let root = repo_root();
    let gate_path =
        root.join("examples/sample_project/qa/gates/screenshot_heavy_regression_gate.yaml");
    assert!(gate_path.is_file());

    let gate = fs::read_to_string(gate_path).unwrap();
    for expected in [
        "name: screenshot_heavy_regression_gate",
        "visual_occlusion_demo",
        "saved_snapshot_pixels_demo",
        "visual_metadata_rect_demo",
        "visual_line_indicator_demo",
        "launch_display",
        "run_root",
    ] {
        assert!(gate.contains(expected), "gate missing {expected}");
    }

    let readme = read("docs/workflows.md");
    let sample_readme = read("examples/sample_project/README.md");
    let scenario_format = read("docs/scenario-format.md");

    assert!(readme.contains("screenshot_heavy_regression_gate"));
    assert!(sample_readme.contains("screenshot_heavy_regression_gate"));
    assert!(scenario_format.contains("screenshot_heavy_regression_gate"));
}

#[test]
fn recorder_negative_suggestion_docs_contract_is_present() {
    let cli_spec = read("docs/cli-spec.md");
    let scenario_format = read("docs/scenario-format.md");
    let artifact_format = read("docs/artifact-format.md");
    let test_strategy = read("docs/test-strategy.md");

    assert!(cli_spec.contains("--input-script <path> --duration-frames <n>"));
    assert!(cli_spec.contains("bounded mode"));
    assert!(cli_spec.contains("bounded non-interactive recorder flows"));
    assert!(cli_spec.contains("rejects an existing output file"));
    assert!(
        cli_spec.contains("malformed recorder input scripts return structured validation failures")
    );
    assert!(cli_spec.contains("do not auto-populate `assert`"));
    assert!(cli_spec.contains("do not approve or write baselines"));
    assert!(cli_spec.contains("complement `scenario lint`"));

    assert!(scenario_format.contains("\"kind\": \"negative_visibility_guard\""));
    assert!(scenario_format.contains("\"input_moment\":"));
    assert!(scenario_format.contains("\"target\":"));
    assert!(scenario_format.contains("\"snapshots\":"));
    assert!(scenario_format.contains("\"evidence\":"));
    assert!(scenario_format.contains("\"assert\": []"));
    assert!(
        scenario_format
            .contains("manually accept, reject, or edit each `review.suggested_assertions`")
    );

    assert!(artifact_format.contains("## Recorder artifacts (S39)"));
    assert!(artifact_format.contains(".qa/artifacts/recordings/<name>/<run-id>/"));
    assert!(artifact_format.contains("recording.json"));
    assert!(artifact_format.contains("candidate.json"));
    assert!(artifact_format.contains("deterministic moment snapshots"));
    assert!(artifact_format.contains("must not write to `qa/baselines`"));

    assert!(test_strategy.contains("Recorder negative suggestion validation guidance (S39):"));
    assert!(test_strategy.contains(
        "--input-script qa/recordings/negative_suggestions_input.json --duration-frames 45 --json"
    ));
    assert!(test_strategy.contains("recorder suggestions remain artifacts-only"));
}

#[test]
fn runtime_qa_performance_guidance_docs_contract_is_present() {
    let entrypoint = read("skills/godot-qa/SKILL.md");
    assert!(entrypoint.contains("references/runtime-instrumentation.md"));
    let skill = read("skills/godot-qa/references/runtime-instrumentation.md");
    let qa_ids = read("docs/qa-ids.md");
    let scenario_format = read("docs/scenario-format.md");
    let cli_spec = read("docs/cli-spec.md");
    let test_strategy = read("docs/test-strategy.md");

    assert!(skill.contains("Use `qa_id` metadata for durable anchors"));
    assert!(skill.contains("Avoid per-object QA IDs for every generated cell"));
    assert!(skill.contains("Avoid high-frequency metadata or timed `set_meta()` mirroring"));
    assert!(skill.contains("get_qa_*_state()"));
    assert!(skill.contains("qa_*_probe()"));
    assert!(
        skill.contains("debug-label text when the debug label is not the player-facing contract")
    );
    assert!(skill.contains("Hidden debug labels should not need frequent refreshes"));
    assert!(skill.contains("rather than one runtime node locator per object"));

    assert!(qa_ids.contains("## Durable anchors vs dynamic state"));
    assert!(qa_ids.contains("Use a small set of durable anchors for automation entry points"));
    assert!(qa_ids.contains("Avoid per-object `qa_id` metadata and repeated metadata scans"));
    assert!(qa_ids.contains("Implement `get_qa_*_state()` on a durable anchor"));
    assert!(qa_ids.contains("Implement a named `qa_*_probe()` provider method"));
    assert!(qa_ids.contains("High-cardinality dynamic objects make metadata scans slower"));
    assert!(qa_ids.contains(
        "Scenario assertions can target `qa:match_controller` and call `get_qa_match_state()`"
    ));
    assert!(qa_ids.contains(
        "Scenario assertions can target `qa:surface_provider` and call `qa_surface_probe()`"
    ));
    assert!(qa_ids.contains("logical gameplay identifiers, not `qa:` locators"));

    assert!(scenario_format.contains(
        "Scenarios should target durable `qa:` anchors for important systems and visible controls"
    ));
    assert!(scenario_format.contains("use state/probe assertions for inspectable gameplay facts"));
    assert!(scenario_format.contains(
        "Parse debug labels only when the label text itself is the player-facing contract"
    ));
    assert!(scenario_format.contains(
        "a hidden debug label should not be the primary automation API when a method/probe assertion can inspect the same state"
    ));
    assert!(scenario_format.contains(
        "The scenario format does not require every generated object or transient value to have its own `qa_id`"
    ));
    assert!(scenario_format.contains("get_qa_inventory_state()"));
    assert!(scenario_format.contains("qa_collision_probe()"));
    assert!(scenario_format.contains("qa_surface_probe"));
    assert!(scenario_format.contains(
        "Debug labels remain useful diagnostics, but scenario assertions should read them"
    ));

    assert!(cli_spec.contains("Assertion examples should target durable `qa:` anchors"));
    assert!(cli_spec.contains(
        "prefer project-owned `get_qa_*_state()` methods or named `qa_*_probe()` methods over timed metadata mirroring"
    ));
    assert!(cli_spec.contains(
        "Debug labels are diagnostics unless their text is the actual user-facing contract"
    ));
    assert!(cli_spec.contains(
        "Use `qa:` locators as durable anchors, not as per-object tags for every dynamic gameplay fact"
    ));
    assert!(cli_spec.contains(
        "Dynamic state should come from project-owned `get_qa_*_state()` methods, named `qa_*_probe()` methods"
    ));

    assert!(test_strategy.contains(
        "Use durable `qa:` anchors for stable targets, project-owned `get_qa_*_state()` methods"
    ));
    assert!(test_strategy.contains("named `qa_*_probe()` methods"));
    assert!(test_strategy.contains(
        "Treat debug labels as diagnostics unless their text is the user-facing contract"
    ));
    assert!(test_strategy.contains("optional downstream stress validation and motivation"));
    assert!(
        test_strategy.contains(
            "They are advisory only, not required fixtures and not CI gates for `godot-qa`"
        )
    );
}

#[test]
fn state_matches_docs_contract() {
    let runtime_protocol = read("docs/runtime-bridge-protocol.md");
    let scenario_format = read("docs/scenario-format.md");
    let cli_spec = read("docs/cli-spec.md");
    let qa_ids = read("docs/qa-ids.md");
    let test_strategy = read("docs/test-strategy.md");
    let addon_readme = read("addons/godot_qa/README.md");
    let sample_readme = read("examples/sample_project/README.md");

    for contents in [
        runtime_protocol.as_str(),
        scenario_format.as_str(),
        addon_readme.as_str(),
    ] {
        assert!(contents.contains("qa_*"));
        assert!(contents.contains("get_qa_*"));
        assert!(contents.contains("get_*_qa_state"));
        assert!(contents.contains("zero-argument"));
        assert!(contents.contains("JSON-compatible"));
    }

    assert!(runtime_protocol.contains("read-only by convention"));
    assert!(runtime_protocol.contains("not an arbitrary method-call escape hatch"));
    assert!(runtime_protocol.contains("`name`"));
    assert!(runtime_protocol.contains("`arguments`"));
    assert!(runtime_protocol.contains("`missing_method`"));
    assert!(runtime_protocol.contains("`non_json_return`"));
    assert!(runtime_protocol.contains("`runtime_error`"));

    assert!(scenario_format.contains("get_raft_motion_qa_state"));
    assert!(scenario_format.contains("Aggregate state methods are best"));
    assert!(
        scenario_format.contains("setup `call_method` only for deterministic fixture mutation")
    );
    assert!(scenario_format.contains("debug-label text assertions only when the visible text itself is the player-facing contract"));

    assert!(
        cli_spec.contains("prefer project-owned `get_*_qa_state()` or `get_qa_*_state()` methods")
    );
    assert!(cli_spec.contains("debug-label parsing"));
    assert!(cli_spec.contains(
        "debug labels are diagnostics unless their visible text is the user-facing contract"
    ));

    assert!(qa_ids.contains("`get_qa_*_state()` or `get_*_qa_state()`"));
    assert!(qa_ids.contains("get_raft_motion_qa_state()"));

    assert!(test_strategy.contains("QA state method assertion allowlist guidance (S48)"));
    assert!(test_strategy.contains("cargo test state_matches_docs_contract"));
    assert!(test_strategy.contains("cargo test state_matches_rejected_method_contract"));
    assert!(test_strategy.contains("state_matches_get_named_qa_state_demo"));
    assert!(
        test_strategy.contains(
            "debug-label assertions only when the label text is the user-facing contract"
        )
    );

    assert!(addon_readme.contains("method: \"get_raft_motion_qa_state\""));
    assert!(sample_readme.contains("get_raft_motion_qa_state()"));
    assert!(sample_readme.contains("state_matches_get_named_qa_state_demo"));
}

#[test]
fn repo_contract_qa_state_schema_docs() {
    let readme = read("docs/workflows.md");
    let cli_spec = read("docs/cli-spec.md");
    let runtime_protocol = read("docs/runtime-bridge-protocol.md");
    let scenario_format = read("docs/scenario-format.md");
    let test_strategy = read("docs/test-strategy.md");

    assert!(readme.contains("qa-state validate"));
    assert!(readme.contains("project-owned QA state methods"));
    assert!(readme.contains("saved QA state artifacts"));

    for usage in [
        "godot-qa qa-state validate",
        "--target qa:schema_state_provider",
        "--method get_qa_schema_demo_state",
        "--input examples/sample_project/qa/fixtures/schema_demo_state_valid.json",
        "--trace examples/sample_project/qa/fixtures/schema_demo_trace.json",
        "--state-path samples[*].state",
        "qa_state.validate",
        "CommandResult",
        "qa_state_schema_validation_failed",
    ] {
        assert!(cli_spec.contains(usage), "missing CLI docs text: {usage}");
    }

    for keyword in [
        "type",
        "properties",
        "required",
        "items",
        "enum",
        "const",
        "minimum",
        "maximum",
        "minLength",
        "maxLength",
        "minItems",
        "maxItems",
        "additionalProperties",
    ] {
        assert!(
            cli_spec.contains(keyword),
            "missing supported schema keyword: {keyword}"
        );
    }

    assert!(cli_spec.contains("qa_state_schema_unsupported_keyword"));
    assert!(cli_spec.contains("Unsupported validation keywords"));
    assert!(cli_spec.contains("not silently ignored"));
    assert!(cli_spec.contains("pattern"));
    assert!(cli_spec.contains("oneOf"));
    assert!(cli_spec.contains("anyOf"));

    for allowlist in ["qa_*", "get_qa_*", "get_*_qa_state"] {
        assert!(cli_spec.contains(allowlist));
        assert!(runtime_protocol.contains(allowlist));
    }

    assert!(runtime_protocol.contains("qa_state.read"));
    assert!(runtime_protocol.contains("\"target\": \"qa:race\""));
    assert!(runtime_protocol.contains("\"method\": \"get_qa_race_state\""));
    assert!(runtime_protocol.contains("token-protected live bridge command"));
    assert!(runtime_protocol.contains("Invalid or missing session tokens"));
    for rejection in [
        "name",
        "missing_method",
        "arguments",
        "runtime_error",
        "non_json_return",
    ] {
        assert!(
            runtime_protocol.contains(rejection),
            "missing rejection class: {rejection}"
        );
    }

    assert!(scenario_format.contains("complements `state_matches`"));
    assert!(scenario_format.contains("does not replace scenario assertions"));
    assert!(scenario_format.contains("does not require every project or every QA method"));
    assert!(scenario_format.contains("not a reason to add hidden debug labels"));

    for fixture in [
        "examples/sample_project/ui/fixtures/qa_state_schema_demo.tscn",
        "examples/sample_project/qa/schemas/schema_demo_state.schema.json",
        "examples/sample_project/qa/fixtures/schema_demo_state_valid.json",
        "examples/sample_project/qa/fixtures/schema_demo_state_invalid.json",
        "examples/sample_project/qa/fixtures/schema_demo_trace.json",
    ] {
        assert!(
            cli_spec.contains(fixture) || test_strategy.contains(fixture),
            "missing sample fixture path: {fixture}"
        );
    }

    for validator in [
        "cargo test -p godot-qa-cli repo_contract_qa_state_schema_docs",
        "cargo test -p godot-qa-cli qa_state_validate",
    ] {
        assert!(
            test_strategy.contains(validator),
            "missing validation script or command: {validator}"
        );
    }
}

#[test]
fn qa_locator_fast_path_contract_docs_are_present() {
    let qa_ids = read("docs/qa-ids.md");
    let runtime_protocol = read("docs/runtime-bridge-protocol.md");
    let cli_spec = read("docs/cli-spec.md");
    let addon_readme = read("addons/godot_qa/README.md");

    assert!(qa_ids.contains("QaRuntime.register_qa_id"));
    assert!(qa_ids.contains("qa.id.<id>"));
    assert!(qa_ids.contains("qa.<id>"));
    assert!(qa_ids.contains("metadata-only projects remain supported"));
    assert!(qa_ids.contains("qa_aliases"));
    assert!(qa_ids.contains("deduplicates the same node"));
    assert!(qa_ids.contains("rejected_candidates"));
    assert!(qa_ids.contains("candidate_status"));

    assert!(runtime_protocol.contains("canonical `qa.id.<id>` groups first"));
    assert!(runtime_protocol.contains("compatibility `qa.<id>` groups second"));
    assert!(runtime_protocol.contains("fallback scans for `qa_id` metadata and `qa_aliases`"));
    assert!(runtime_protocol.contains("ambiguous_target"));
    assert!(runtime_protocol.contains("deduplicated `path:/root/...` matches"));
    assert!(runtime_protocol.contains("target_not_found"));
    assert!(runtime_protocol.contains("candidate_status"));
    assert!(runtime_protocol.contains("rejected_candidates"));

    assert!(cli_spec.contains("## Target locators"));
    assert!(cli_spec.contains("canonical `qa.id.<id>` groups as the efficient fast path"));
    assert!(cli_spec.contains("compatibility `qa.<id>` groups"));
    assert!(cli_spec.contains("fallback scans for `qa_id` metadata and `qa_aliases`"));
    assert!(
        cli_spec
            .contains("Metadata-only projects and alias-only migration projects remain supported")
    );
    assert!(cli_spec.contains("Duplicate matches for the same node"));
    assert!(cli_spec.contains("deduplicated matching `path:/root/...` nodes"));
    assert!(cli_spec.contains("candidate_status"));
    assert!(cli_spec.contains("rejected_candidates"));

    assert!(addon_readme.contains("efficient `qa:` resolver"));
    assert!(addon_readme.contains("`register_qa_id` helper"));
    assert!(addon_readme.contains("`qa_id` metadata with canonical `qa.id.<id>` groups"));
}

#[test]
fn s43_cli_docs_document_version_probe_fallback_and_display_failure_contract() {
    let cli_spec = read("docs/cli-spec.md");

    assert!(cli_spec.contains("Version probing has one narrow exception"));
    assert!(cli_spec.contains("retry the selected Godot binary directly as `godot --version`"));
    assert!(cli_spec.contains("does not change normal launch behavior"));
    assert!(cli_spec.contains("`data.godot.version_probe`"));
    assert!(cli_spec.contains(
        "`prefixed_command`, `final_command`, `direct_fallback_used`, and `fallback_reason`"
    ));
    assert!(cli_spec.contains("\"version_probe\""));
    assert!(cli_spec.contains("\"direct_fallback_used\": true"));

    assert!(cli_spec.contains("`display_backend_unavailable`"));
    assert!(cli_spec.contains("distinct from `godot_launch_failed`, `scenario_result_missing`, `scenario_result_invalid`, runtime script errors, and assertion failures"));
    assert!(cli_spec.contains("\"launch_display\""));
    assert!(cli_spec.contains("\"virtual_display\""));
    assert!(cli_spec.contains("\"display_driver\""));
    assert!(cli_spec.contains("\"mode\": \"windowed\""));
}

#[test]
fn s43_artifact_and_scenario_docs_define_real_pixel_display_metadata() {
    let artifact_format = read("docs/artifact-format.md");
    let scenario_format = read("docs/scenario-format.md");

    assert!(artifact_format.contains("`launch_display`"));
    assert!(artifact_format.contains("\"display_available\": false"));
    assert!(artifact_format.contains("\"virtual_display\""));
    assert!(artifact_format.contains("`display_backend_unavailable` failure means screenshot-backed evidence required real rendered pixels"));
    assert!(artifact_format.contains("When artifact preparation already succeeded, `failure.artifacts` carries the same run-level path map as `data.artifacts`"));

    assert!(scenario_format.contains("Screenshot-backed scenarios require real rendered pixels"));
    assert!(scenario_format.contains("safely detected virtual display wrapper"));
    assert!(scenario_format.contains("`type: \"display_backend_unavailable\"`"));
    assert!(scenario_format.contains("Scenarios without media evidence may still run headless"));
}

#[test]
fn s43_decisions_record_limited_launch_prefix_exceptions() {
    let decisions = read("docs/implementation-decisions.md");

    assert!(decisions.contains("S43 keeps that launch contract intact"));
    assert!(decisions.contains(
        "version probes may retry direct `godot --version` after a prefixed probe fails"
    ));
    assert!(decisions.contains("not a general launch fallback"));
    assert!(decisions.contains("does not bypass an explicitly invalid Godot binary path"));
    assert!(
        decisions.contains("## Decision 010: Screenshot evidence requires a real display backend")
    );
    assert!(
        decisions.contains("automatic virtual display use is limited to safely detected wrappers")
    );
    assert!(decisions.contains("non-screenshot scenarios may remain headless"));
}

#[test]
fn s43_test_strategy_requires_self_contained_sample_coverage() {
    let test_strategy = read("docs/test-strategy.md");

    assert!(test_strategy.contains("Display backend and version probe validation guidance (S43)"));
    assert!(test_strategy.contains("Required sample-project validation commands"));
    assert!(
        test_strategy.contains(
            "cargo run --bin godot-qa -- doctor --project examples/sample_project --json"
        )
    );
    assert!(test_strategy.contains("cargo run --bin godot-qa -- scenario run visual_occlusion_demo --project examples/sample_project --json"));
    assert!(test_strategy.contains("cargo run --bin godot-qa -- scenario run sample_main_menu_start --project examples/sample_project --json"));
    assert!(
        test_strategy.contains(
            "cargo run --bin godot-qa -- import --project examples/sample_project --json"
        )
    );
    assert!(test_strategy.contains(
        "cargo run --bin godot-qa -- check --project examples/sample_project --all-scenes --json"
    ));
    assert!(test_strategy.contains("both-probe failure remains `godot_version_probe_failed`"));
    assert!(test_strategy.contains("assert `launch_display` metadata"));
    assert!(test_strategy.contains("fail early as `display_backend_unavailable`"));
    assert!(test_strategy.contains("optional advisory validation only"));
    assert!(test_strategy.contains("must not become a required fixture, required CI input"));

    assert!(!test_strategy.contains("required CI gates for `godot-qa`"));
}

#[test]
fn s45_process_runtime_log_outcome_docs_contract_is_present() {
    let cli_spec = read("docs/cli-spec.md");
    let artifact_format = read("docs/artifact-format.md");
    let scenario_format = read("docs/scenario-format.md");
    let test_strategy = read("docs/test-strategy.md");

    assert!(cli_spec.contains("`data.process`: CLI-owned Godot process summary"));
    assert!(cli_spec.contains("`data.runtime_result`: CLI validation summary"));
    assert!(cli_spec.contains("`data.logs`: CLI-owned log health summary"));
    assert!(cli_spec.contains("`data.outcome`: final scenario decision"));
    assert!(cli_spec.contains("Strict default semantics are unchanged"));
    assert!(cli_spec.contains("A clean runtime result plus a non-zero Godot process exit fails by default with `type: \"godot_run_failed\"`"));
    assert!(cli_spec.contains(
        "The only non-strict process outcome policy is the explicit wrapper-only opt-in"
    ));
    assert!(cli_spec.contains("\"status\": \"pass_with_warning\""));
    assert!(cli_spec.contains("\"severity\": \"warning\""));
    assert!(cli_spec.contains("\"failures\": []"));
    assert!(cli_spec.contains("must not mask crashes, script errors, runtime assertion failures, malformed or missing artifacts, log failures, absent policy, or unlisted exit codes"));

    assert!(artifact_format.contains("runtime `result.json` as Godot-owned runtime truth"));
    assert!(artifact_format.contains("`data.artifacts` is the prepared run-level path map"));
    assert!(
        artifact_format.contains("`data.runtime_result` summarizes whether the runtime payload")
    );
    assert!(
        artifact_format.contains("`data.process` summarizes the Godot process exit independently")
    );
    assert!(artifact_format.contains("`data.logs` summarizes log health independently"));
    assert!(artifact_format.contains("`data.outcome` is the final CLI decision"));
    assert!(artifact_format.contains("`scenario_result_missing`"));
    assert!(artifact_format.contains("`scenario_result_invalid`"));
    assert!(artifact_format.contains(
        "A wrapper-only pass-with-warning still leaves the non-zero process status visible"
    ));

    assert!(scenario_format.contains("`process_outcome_policy.known_wrapper_exit_codes`"));
    assert!(
        scenario_format.contains("non-empty `known_wrapper_exit_codes` list of positive integers")
    );
    assert!(scenario_format.contains("Scenario assertion syntax is unchanged"));
    assert!(scenario_format.contains("Missing or malformed runtime results, runtime failures, log errors or warnings, absent policy, and unlisted exit codes remain strict failures"));

    assert!(test_strategy.contains("Scenario run result mapping validation guidance (S45):"));
    assert!(test_strategy.contains(
        "separate `data.process`, `data.runtime_result`, `data.logs`, and `data.outcome` summaries"
    ));
    assert!(test_strategy.contains("process_exit_strict_failure_demo"));
    assert!(test_strategy.contains("process_exit_warning_policy_demo"));
    assert!(test_strategy.contains("data.runtime_result.status: \"missing\""));
    assert!(test_strategy.contains("data.runtime_result.status: \"invalid\""));
    assert!(test_strategy.contains("data.runtime_result.status: \"failed\""));
    assert!(test_strategy.contains("`godot_log_error` or `godot_log_warning`"));
    assert!(test_strategy.contains("`scenario bundle run` remains strict"));
}

#[test]
fn live_gate_docs_contract_is_present() {
    let cli_spec = read("docs/cli-spec.md");
    let scenario_format = read("docs/scenario-format.md");
    let runtime_protocol = read("docs/runtime-bridge-protocol.md");
    let architecture = read("docs/architecture.md");
    let test_strategy = read("docs/test-strategy.md");

    assert!(cli_spec.contains("godot-qa live-gate list"));
    assert!(cli_spec.contains("godot-qa live-gate run <name>"));
    assert!(cli_spec.contains("qa/live_gates/"));
    assert!(cli_spec.contains(".qa/artifacts/live-gates/"));
    assert!(cli_spec.contains("Optional anchors produce structured warnings"));

    assert!(scenario_format.contains("Live gate manifests"));
    assert!(scenario_format.contains("qa/live_gates"));
    assert!(scenario_format.contains("required"));
    assert!(scenario_format.contains("methods"));
    assert!(scenario_format.contains("probes"));
    assert!(scenario_format.contains("Strict mode promotes optional-anchor warnings"));

    assert!(runtime_protocol.contains("Live gate usage"));
    assert!(runtime_protocol.contains("session.status"));
    assert!(runtime_protocol.contains("node.inspect"));
    assert!(runtime_protocol.contains("assert.state_matches"));
    assert!(runtime_protocol.contains("assert.surface_probe"));
    assert!(runtime_protocol.contains("errors.read"));
    assert!(runtime_protocol.contains("must not infer overlay state"));
    assert!(runtime_protocol.contains("does not add `eval`"));

    assert!(architecture.contains("Live Inspection Gates"));
    assert!(architecture.contains("Godot addon remains the source of truth"));
    assert!(
        test_strategy.contains("Hidden-overlay live inspection gate validation guidance (S49)")
    );
    assert!(test_strategy.contains("cargo test hidden_overlay_missing_anchor_contract"));
}

#[test]
fn s51_bundle_display_isolation_docs_contract() {
    let cli_spec = read("docs/cli-spec.md");
    let artifact_format = read("docs/artifact-format.md");
    let scenario_format = read("docs/scenario-format.md");
    let test_strategy = read("docs/test-strategy.md");
    let architecture = read("docs/architecture.md");

    for failure_kind in [
        "display_startup_exhausted",
        "virtual_display_allocation_failed",
        "stale_process_cleanup_failed",
        "scenario_result_missing",
    ] {
        assert!(cli_spec.contains(failure_kind));
        assert!(artifact_format.contains(failure_kind));
        assert!(test_strategy.contains(failure_kind));
    }

    assert!(cli_spec.contains("[--retry-startup <count>]"));
    assert!(cli_spec.contains("[--retry-startup-backoff-ms <ms>]"));
    assert!(cli_spec.contains("[--jobs <signed-count>]"));
    assert!(cli_spec.contains("`--jobs -2`"));
    assert!(cli_spec.contains("\"parallelism\""));
    assert!(cli_spec.contains("\"complete_queued\": true"));
    assert!(cli_spec.contains("Bundle runs do not retry by default"));
    assert!(cli_spec.contains("Startup retry is opt-in, bounded"));
    assert!(cli_spec.contains("Retry never applies to gameplay/assertion failures"));
    assert!(cli_spec.contains("\"attempts\""));
    assert!(cli_spec.contains("\"cleanup_status\""));
    assert!(cli_spec.contains("\"display_lifecycle\""));
    assert!(cli_spec.contains("\"retry_guidance\""));
    assert!(cli_spec.contains("\"category\": \"infrastructure\""));

    assert!(artifact_format.contains("## Scenario and bundle artifacts"));
    assert!(
        artifact_format
            .contains("Bundle artifacts preserve the existing run-level child artifact paths")
    );
    assert!(artifact_format.contains(
        "bundle summary attempts, `source_bundle`, `bundle_chain`, launch metadata, cleanup metadata, and retry guidance are written"
    ));
    assert!(artifact_format.contains("`attempts` arrays"));
    assert!(artifact_format.contains("`display_lifecycle`"));
    assert!(artifact_format.contains("`cleanup_status`"));
    assert!(artifact_format.contains("`retry_guidance`"));
    assert!(artifact_format.contains("category: \"infrastructure\""));

    assert!(scenario_format.contains("Bundle retry policy is CLI-owned, opt-in, bounded"));
    assert!(scenario_format.contains("Negative retry cases remain strict failures with no retry"));
    assert!(
        scenario_format
            .contains("Child scenario behavior remains sourced from direct `scenario run`")
    );
    assert!(scenario_format.contains("`--jobs -N` reserves `N` available threads"));
    assert!(
        scenario_format
            .contains("`data.runs[]` remains ordered by original bundle expansion index")
    );
    assert!(artifact_format.contains(
        "Bundle command JSON, `meta.json`, and `summary.json` also record `parallelism`"
    ));

    assert!(test_strategy.contains("Scenario bundle validation guidance (S51):"));
    assert!(test_strategy.contains("Parallel execution contract"));
    assert!(
        test_strategy
            .contains("cargo test -p godot-qa-cli s51_bundle_display_isolation_docs_contract")
    );
    assert!(test_strategy.contains("cargo test display_startup_failure_contract"));
    assert!(test_strategy.contains("scenario bundle run screenshot_heavy_regression_gate"));
    assert!(test_strategy.contains("scenario bundle run sample_regression_gate"));
    assert!(test_strategy.contains("Forced display-startup fixture inspection"));

    assert!(architecture.contains("Scenario bundle orchestration remains CLI-owned"));
    assert!(architecture.contains("CLI is the source of truth for bundle expansion"));
    assert!(
        architecture
            .contains("godot-qa-process` owns process launch details, display lifecycle metadata")
    );
}

#[test]
fn s58_bundle_artifact_summary_docs_contract() {
    let cli_spec = read("docs/cli-spec.md");
    let artifact_format = read("docs/artifact-format.md");
    let sample_readme = read("examples/sample_project/README.md");

    for doc in [&cli_spec, &artifact_format] {
        assert!(doc.contains("artifact-index.json"));
        assert!(doc.contains("data.artifact_summary"));
        assert!(doc.contains("data.artifacts.artifact_index_path"));
        assert!(doc.contains("scenario"));
        assert!(doc.contains("status"));
        assert!(doc.contains("run_root"));
        assert!(doc.contains("result_path"));
        assert!(doc.contains("logs_path"));
        assert!(doc.contains("screenshot_paths"));
        assert!(doc.contains("diagnostic_paths"));
        assert!(doc.contains("failure_artifact_paths"));
        assert!(doc.contains("source_bundle"));
        assert!(doc.contains("bundle_chain"));
        assert!(doc.contains("Missing optional scenarios"));
        assert!(doc.contains("warnings"));
        assert!(doc.contains("child `result.json`, logs, screenshots, diagnostics"));
    }

    assert!(cli_spec.contains("\"artifact_summary\""));
    assert!(cli_spec.contains("\"artifact_index_path\""));
    assert!(cli_spec.contains("\"missing_optional_scenario\""));
    assert!(cli_spec.contains("does not add project-specific gate names"));
    assert!(cli_spec.contains("Legacy `child_run_root` or `launch_args` fields are not part"));

    assert!(
        artifact_format
            .contains("The top-level bundle command exposes the same concise summary object")
    );
    // Bundle warnings can carry warm pool session-start evidence.
    assert!(artifact_format.contains("warm_session_start_failed"));
    assert!(artifact_format.contains("never promoted to failures by bundle `--strict`"));
    assert!(artifact_format.contains("must not duplicate large screenshot payloads"));
    assert!(
        artifact_format
            .contains("Nested bundle summaries retain `source_bundle` and `bundle_chain`")
    );

    assert!(sample_readme.contains("artifact-index.json"));
    assert!(sample_readme.contains("`data.artifact_summary`"));
    assert!(sample_readme.contains("in addition to `summary.json`"));
    assert!(sample_readme.contains("`source_bundle` and `bundle_chain` lineage"));
}

#[test]
fn live_profile_retry_docs_contract() {
    let cli_spec = read("docs/cli-spec.md");
    let artifact_format = read("docs/artifact-format.md");
    let runtime_protocol = read("docs/runtime-bridge-protocol.md");
    let test_strategy = read("docs/test-strategy.md");

    for failure_kind in [
        "session_bridge_busy",
        "session_unreachable",
        "session_token_invalid",
        "session_connect_timeout",
        "profile_result_invalid",
        "artifact_write_failed",
    ] {
        assert!(cli_spec.contains(failure_kind));
        assert!(artifact_format.contains(failure_kind));
        assert!(runtime_protocol.contains(failure_kind));
        assert!(test_strategy.contains(failure_kind));
    }

    assert!(cli_spec.contains("`data.retry.attempts`"));
    assert!(cli_spec.contains("`data.retry.backoff_ms`"));
    assert!(cli_spec.contains("`data.retry.final_status`"));
    assert!(cli_spec.contains("Agents may retry `session_bridge_busy`"));
    assert!(cli_spec.contains("Agents should treat `session_unreachable`"));
    assert!(cli_spec.contains("Agents should not retry `profile_result_invalid`"));
    assert!(cli_spec.contains("session list --json"));

    assert!(
        artifact_format.contains("writes the profile artifact only after the runtime returns a valid `session.profile` payload")
    );
    assert!(artifact_format.contains("must not overwrite previous live-session profile artifacts"));
    assert!(artifact_format.contains("Retry metadata such as attempts, bounded backoff timing"));
    assert!(artifact_format.contains("is command JSON evidence"));
    assert!(artifact_format.contains("is not baseline approval"));

    assert!(runtime_protocol.contains("The runtime command name remains `session.profile`"));
    assert!(runtime_protocol.contains("token-protected live bridge envelope"));
    assert!(runtime_protocol.contains("contended resource"));
    assert!(runtime_protocol.contains("`retry_eligible: true`"));
    assert!(runtime_protocol.contains("must not be retried as bridge-busy transport failures"));

    assert!(test_strategy.contains("S54 live session profiling retry validation"));
    assert!(test_strategy.contains("cargo test live_profile_retry_docs_contract"));
    assert!(test_strategy.contains("cargo test live_profile_bridge_busy_retries"));
    assert!(test_strategy.contains(
        "cargo run --bin godot-qa -- session profile --seconds 3 --max-frame-samples 120 --json"
    ));
    assert!(test_strategy.contains("cargo run --bin godot-qa -- session list --json"));
    assert!(test_strategy.contains("Agents may retry `session_bridge_busy`"));
    assert!(test_strategy.contains("Agents should not retry `session_unreachable`"));
}

#[test]
fn doctor_binary_surface_docs() {
    let cli_spec = read("docs/cli-spec.md");
    let test_strategy = read("docs/test-strategy.md");
    let release_checklist = read("docs/release-checklist.md");
    let readme = read("docs/workflows.md");

    assert!(cli_spec.contains("godot-qa doctor --strict-binary-surface"));
    assert!(cli_spec.contains("`data.binary_surface.current_exe`"));
    assert!(cli_spec.contains("`data.binary_surface.first_path_binary`"));
    assert!(cli_spec.contains("`data.binary_surface.expected_commands`"));
    assert!(cli_spec.contains("`data.binary_surface.candidates[]`"));
    assert!(cli_spec.contains("`data.binary_surface.summary`"));
    assert!(cli_spec.contains("`first_path_binary` is the first executable file named `godot-qa` discovered while scanning `PATH`"));
    assert!(
        cli_spec.contains("`data.warnings[]` entry with `type: \"multiple_godot_qa_binaries\"`")
    );
    assert!(cli_spec.contains("`severity: \"warning\"`"));
    assert!(cli_spec.contains("`message`, `current_exe`, `first_path_binary`, `candidate`, `path_order`, `missing_commands`, `extra_commands`, `version`, `probe_errors`, and `remediation`"));
    assert!(cli_spec.contains("timeout/error state rather than hanging doctor"));
    assert!(cli_spec.contains("`binary_surface_mismatch` failure"));
    assert!(cli_spec.contains("does not inspect shell aliases/functions, execute shell snippets, modify `PATH`, delete binaries, or choose a different binary automatically"));
    assert!(cli_spec.contains("stale Cargo source install under `~/.cargo/bin`"));
    assert!(cli_spec.contains("unpacked release archive directory"));
    assert!(cli_spec.contains("repo-local `target/debug` or `target/release` binaries"));

    assert!(test_strategy.contains("warning-only condition by default"));
    assert!(test_strategy.contains("top-level `ok` remains true and `failures` remains empty"));
    assert!(test_strategy.contains("`doctor --strict-binary-surface` is the opt-in CI mode"));
    assert!(test_strategy.contains("`binary_surface_mismatch`"));
    assert!(test_strategy.contains("fake executable `godot-qa` files in temporary directories"));
    assert!(test_strategy.contains("stale `~/.cargo/bin`, unpacked release archive directories, and repo-local `target/debug` or `target/release` binaries"));
    assert!(test_strategy.contains("never edits `PATH`, deletes discovered binaries, executes shell snippets, or relies on aliases/functions"));

    assert!(release_checklist.contains(
        "godot-qa doctor --project examples/sample_project --json --strict-binary-surface"
    ));
    assert!(release_checklist.contains("stale `.cargo/bin`, release archive, or repo-local `target/debug` or `target/release` shadowing warning"));
    assert!(release_checklist.contains("does not modify `PATH` or delete files"));

    assert!(readme.contains("Troubleshooting stale binaries"));
    assert!(readme.contains("`data.binary_surface`"));
    assert!(readme.contains("`data.warnings`"));
    assert!(readme.contains("`godot-qa doctor --json --strict-binary-surface`"));
    assert!(readme.contains("stale `~/.cargo/bin/godot-qa`"));
    assert!(readme.contains("unpacked release archive directory"));
    assert!(readme.contains("repo-local `target/debug` or `target/release` binaries"));
    assert!(readme.contains("never modifies `PATH`, deletes binaries, or runs shell snippets"));
}

#[test]
fn repo_contract_collision_report_docs() {
    let readme = read("docs/workflows.md");
    let cli_spec = read("docs/cli-spec.md");
    let artifact_format = read("docs/artifact-format.md");
    let runtime_protocol = read("docs/runtime-bridge-protocol.md");
    let scenario_format = read("docs/scenario-format.md");
    let test_strategy = read("docs/test-strategy.md");

    for (name, doc) in [
        ("README.md", &readme),
        ("docs/cli-spec.md", &cli_spec),
        ("docs/artifact-format.md", &artifact_format),
        ("docs/runtime-bridge-protocol.md", &runtime_protocol),
        ("docs/scenario-format.md", &scenario_format),
        ("docs/test-strategy.md", &test_strategy),
    ] {
        assert!(
            doc.contains("physics collision-report"),
            "{name} missing collision-report command"
        );
    }

    for required in [
        "godot-qa physics collision-report qa:actor",
        "godot-qa physics collision-report --from-trace",
        "--max-normal-retention <float>",
        "--max-velocity-reversals <int>",
        "--max-same-contact-frames <int>",
        "--max-angular-velocity-spike <float>",
        "--max-overlap-depth <float>",
        "--max-low-progress-contact-frames <int>",
        "--strict-warnings",
        "physics.collision_report",
        "not_evaluated",
        "collision_report.soft_contact_behaves_hard",
        "does not declare high bounce to be a game-specific failure by default",
    ] {
        assert!(
            cli_spec.contains(required),
            "docs/cli-spec.md missing S68 contract text: {required}"
        );
    }

    for provider in [
        "get_qa_collision_state()",
        "get_collision_qa_state()",
        "get_qa_physics_state()",
        "get_physics_qa_state()",
        "qa_collision_probe()",
    ] {
        assert!(
            cli_spec.contains(provider) && runtime_protocol.contains(provider),
            "collision-report docs missing provider name: {provider}"
        );
    }

    for warning_id in [
        "collision_report.pinball_like_rebound",
        "collision_report.repeated_bounce_loop",
        "collision_report.same_contact_pin_risk",
        "collision_report.angular_impulse_spike",
        "collision_report.normal_velocity_retention_high",
        "collision_report.velocity_reversal_high",
        "collision_report.soft_contact_behaves_hard",
        "collision_report.overlap_without_recovery",
        "collision_report.provider_missing",
        "collision_report.provider_invalid",
        "collision_report.insufficient_motion_data",
    ] {
        assert!(
            cli_spec.contains(warning_id),
            "docs/cli-spec.md missing warning ID: {warning_id}"
        );
    }

    for required in [
        ".qa/artifacts/live/<session-id>/collision-reports/<report-id>/collision_report.json",
        ".qa/artifacts/live/<session-id>/traces/<trace-id>/trace.json",
        "normal_velocity_retention_ratio",
        "tangent_velocity_retention_ratio",
        "same_contact_max_consecutive_frames",
        "low_progress_while_contact_frames",
        "not_evaluated",
    ] {
        assert!(
            artifact_format.contains(required),
            "docs/artifact-format.md missing S68 artifact text: {required}"
        );
    }

    assert!(runtime_protocol.contains("CLI-owned trace summarization"));
    assert!(runtime_protocol.contains("no gameplay mutation"));
    assert!(scenario_format.contains("\"surface_type\": \"hard\""));
    assert!(scenario_format.contains("\"surface_type\": \"soft\""));
    assert!(scenario_format.contains("\"surface_type\": \"trigger\""));
    assert!(
        scenario_format
            .contains("Soft-contact warnings require this explicit provider classification")
    );

    for required in [
        "S68 collision-report smoke guidance",
        "cargo test -p godot-qa-cli repo_contract_collision_report_docs",
        "cargo run --bin godot-qa -- physics collision-report qa:slide_actor --project examples/sample_project --frames 180 --sample-every-frames 1 --physics --json",
        "cargo run --bin godot-qa -- physics inspect qa:slide_actor --project examples/sample_project --radius 200 --frames 2 --json",
        "cargo run --bin godot-qa -- scenario run sample_main_menu_start --project examples/sample_project --json",
        "complements human playability review",
    ] {
        assert!(
            test_strategy.contains(required),
            "docs/test-strategy.md missing S68 validation text: {required}"
        );
    }

    assert!(
        readme.contains(
            "Collision-feel reports provide runtime evidence for human playability review"
        )
    );
    assert!(readme.contains(
        "high bounce is not a failure unless the caller opts into thresholds or strict warnings"
    ));
}

#[test]
fn repo_contract_event_trace_docs() {
    let cli_spec = read("docs/cli-spec.md");
    let artifact_format = read("docs/artifact-format.md");
    let runtime_protocol = read("docs/runtime-bridge-protocol.md");
    let scenario_format = read("docs/scenario-format.md");
    let test_strategy = read("docs/test-strategy.md");

    for (name, doc) in [
        ("docs/cli-spec.md", &cli_spec),
        ("docs/scenario-format.md", &scenario_format),
        ("docs/test-strategy.md", &test_strategy),
    ] {
        assert!(
            doc.contains("event trace"),
            "{name} missing event trace command"
        );
        assert!(
            doc.contains("event summarize"),
            "{name} missing event summarize command"
        );
    }

    for provider in [
        "get_qa_event_stream_state",
        "get_event_stream_qa_state",
        "get_qa_events",
        "qa_event_stream",
    ] {
        assert!(
            cli_spec.contains(provider)
                && runtime_protocol.contains(provider)
                && scenario_format.contains(provider)
                && test_strategy.contains(provider),
            "event trace docs missing provider method name: {provider}"
        );
    }

    for field in [
        "total_event_count",
        "event_count_by_category",
        "event_count_by_type",
        "event_rate_per_second_by_type",
        "max_events_in_one_frame",
        "duplicate_event_count",
        "missing_event_id_count",
        "event_spam_warnings",
        "paired_event_warnings",
        "offscreen_event_warnings",
        "provider_error_count",
    ] {
        assert!(
            cli_spec.contains(field)
                && artifact_format.contains(field)
                && runtime_protocol.contains(field),
            "event trace docs missing summary field: {field}"
        );
    }

    for warning_id in [
        "event_trace.provider_missing",
        "event_trace.provider_invalid",
        "event_trace.event_missing_id",
        "event_trace.event_not_json_compatible",
        "event_trace.event_stream_truncated",
        "event_trace.event_spam_risk",
        "event_trace.paired_event_missing",
        "event_trace.offscreen_event_risk",
        "event_trace.provider_error",
        "event_trace.no_events_recorded",
    ] {
        assert!(
            cli_spec.contains(warning_id)
                && artifact_format.contains(warning_id)
                && runtime_protocol.contains(warning_id),
            "event trace docs missing warning ID: {warning_id}"
        );
    }

    for required in [
        ".qa/artifacts/live/<session-id>/events/<event-trace-id>/events.json",
        ".qa/artifacts/live/<session-id>/events/<event-trace-id>/summary.json",
        ".qa/artifacts/live/latest/events/latest/events.json",
        "event_id",
        "frame",
        "time_msec",
        "category",
        "event_type",
        "payload",
        "rule_id",
        "severity",
    ] {
        assert!(
            artifact_format.contains(required),
            "docs/artifact-format.md missing event trace artifact text: {required}"
        );
    }

    for required in [
        "--provider <method>",
        "--pairing-config <path>",
        "--strict-warnings",
        "Event type strings are project-provided",
        "not interpreted or hardcoded by the tool",
        "Warnings become failures only",
    ] {
        assert!(
            cli_spec.contains(required),
            "docs/cli-spec.md missing event trace CLI text: {required}"
        );
    }

    for required in [
        "event.trace",
        "no provider exists",
        "must not add visible debug labels",
        "must not infer gameplay, audio, VFX, UI, or collision events",
    ] {
        assert!(
            runtime_protocol.contains(required),
            "docs/runtime-bridge-protocol.md missing event trace protocol text: {required}"
        );
    }

    for required in [
        "without visible debug labels",
        "Do not add visible debug labels",
        "hidden debug-label parsing",
        "screenshot inference",
        "event trace is the supported generic event stream path",
    ] {
        assert!(
            scenario_format.contains(required),
            "docs/scenario-format.md missing event trace scenario guidance: {required}"
        );
    }

    for required in [
        "S71 event trace validation guidance",
        "cargo test -p godot-qa-cli repo_contract_event_trace_docs",
        "trace record --events-provider` is not implemented",
        "event trace` is the supported event stream path",
        "cargo run --bin godot-qa -- event trace --project examples/sample_project --target qa:event_source --provider get_qa_event_stream_state --frames 120 --sample-every-frames 1 --json",
        "cargo run --bin godot-qa -- event summarize .qa/artifacts/live/latest/events/latest/events.json --json",
        "qa/event_pairing_demo.json",
        "do not make project-specific event names part of the generic contract",
    ] {
        assert!(
            test_strategy.contains(required),
            "docs/test-strategy.md missing S71 validation text: {required}"
        );
    }

    assert!(cli_spec.contains("trace record --events-provider` is not implemented"));
    assert!(runtime_protocol.contains("trace record --events-provider` is not implemented"));
    assert!(scenario_format.contains("trace record --events-provider` is not implemented"));
}

#[test]
fn runtime_bridge_busy_contract_is_present() {
    let bridge = read("addons/godot_qa/qa_bridge.gd");
    let runner = read("addons/godot_qa/qa_session_runner.gd");

    assert!(bridge.contains("session_bridge_busy"));
    assert!(bridge.contains("_bridge_command_in_progress"));
    assert!(bridge.contains("_bridge_command_diagnostics"));
    assert!(bridge.contains("\"invalid_session_token\""));
    assert!(bridge.contains("func is_runtime_qa_active() -> bool:"));
    assert!(bridge.contains("\"runtime_qa_active\": is_runtime_qa_active()"));
    assert!(bridge.contains("func _handle_session_profile"));
    assert!(bridge.contains("\"session.profile\""));
    assert!(bridge.contains("func _handle_session_set_time_scale"));
    assert!(bridge.contains("\"session.set_time_scale\""));
    assert!(bridge.contains("\"sample_window\""));
    assert!(bridge.contains("\"top_script_methods\""));

    let token_check = bridge.find("\"invalid_session_token\"").unwrap();
    let busy_check = bridge.find("session_bridge_busy").unwrap();
    let dispatch = bridge.find("_dispatch_authenticated_command").unwrap();
    assert!(
        token_check < busy_check,
        "invalid tokens must be rejected before bridge-busy classification"
    );
    assert!(
        busy_check < dispatch,
        "bridge-busy must be reported before a second command dispatches"
    );

    assert!(runner.contains("_handle_connection(peer)"));
    assert!(!runner.contains("await _handle_connection(peer)"));
    assert!(runner.contains("_handle_file_request(file_name)"));
    assert!(!runner.contains("await _handle_file_request(file_name)"));
}

#[test]
fn repo_contract_live_audio_docs() {
    let cli_spec = read("docs/cli-spec.md");
    let artifact_format = read("docs/artifact-format.md");
    let runtime_protocol = read("docs/runtime-bridge-protocol.md");
    let test_strategy = read("docs/test-strategy.md");

    for required in [
        "godot-qa audio capture --project <project> --live --seconds 30 --bus Master --name current_race --json",
        "godot-qa audio analyze .qa/artifacts/manual/sfx_capture.wav --json",
        "godot-qa audio export --project <project> --target qa:audio_controller --method qa_export_audio_track",
        "--strict-warnings",
        "requested_seconds",
        "effective_seconds",
        "max_seconds",
        "default maximum is 60 seconds",
        "Bus capture can be unavailable",
        "Warnings are factual measurements or capability reports",
        "does not decide whether a track sounds good",
    ] {
        assert!(
            cli_spec.contains(required),
            "docs/cli-spec.md missing S70 CLI contract: {required}"
        );
    }

    for warning_id in [
        "audio_capture.unavailable",
        "audio_capture.failed",
        "audio_capture.duration_too_long",
        "audio_capture.bus_not_found",
        "audio_analysis.near_silent",
        "audio_analysis.clipping_risk",
        "audio_analysis.high_transient_density",
        "audio_analysis.event_spam_risk",
        "audio_analysis.loop_discontinuity_high",
        "audio_analysis.invalid_wav",
    ] {
        assert!(
            cli_spec.contains(warning_id),
            "docs/cli-spec.md missing S70 warning ID: {warning_id}"
        );
    }

    for analysis_field in [
        "duration_seconds",
        "sample_rate",
        "channels",
        "peak",
        "rms",
        "silence_ratio",
        "near_silence_ratio",
        "clipping_sample_count",
        "clipping_region_count",
        "transient_count",
        "transients_per_second",
        "zero_crossing_rate",
        "spectral_bands",
        "peak_by_window",
        "rms_by_window",
        "transients_by_window",
        "suspected_event_spam_regions",
        "clipping_regions",
        "silence_regions",
        "loop_boundary_delta",
    ] {
        assert!(
            cli_spec.contains(analysis_field) && artifact_format.contains(analysis_field),
            "S70 docs missing analysis field: {analysis_field}"
        );
    }

    for required in [
        ".qa/artifacts/live/<session-id>/audio/",
        "<name>.analysis.json",
        ".qa/artifacts/live/latest",
        "window_seconds = 1.0",
        "hop_seconds = 0.5",
        "must not embed raw sample arrays",
        "Manual output paths",
        "controlled `audio_dir`",
        "intended `wav_path`",
    ] {
        assert!(
            artifact_format.contains(required),
            "docs/artifact-format.md missing S70 artifact contract: {required}"
        );
    }

    for required in [
        "### `audio.capture`",
        "### `audio.export`",
        "\"command\": \"audio.capture\"",
        "\"command\": \"audio.export\"",
        "\"host_microphone_capture\": false",
        "\"audio_dir\"",
        "\"wav_path\"",
        "\"max_bytes\"",
        "method must begin with `qa_`",
        "audio_export_path_rejected",
        "authenticated session operations",
        "token redaction rules",
    ] {
        assert!(
            runtime_protocol.contains(required),
            "docs/runtime-bridge-protocol.md missing S70 protocol contract: {required}"
        );
    }

    for required in [
        "Live audio capture and analysis validation (S70)",
        "cargo test -p godot-qa-cli repo_contract",
        "cargo test -p godot-qa-cli repo_contract_live_audio_docs",
        "cargo run --bin godot-qa -- audio capture --project examples/sample_project --live --seconds 3 --bus Master --name live_audio_demo --analyze --json",
        "cargo run --bin godot-qa -- audio analyze .qa/artifacts/live/latest/audio/live_audio_demo.wav --json",
        "cargo run --bin godot-qa -- audio export --project examples/sample_project --target qa:audio_controller --method qa_export_audio_track --args '[\"deterministic_transients\"]' --name exported_transients --analyze --json",
        "audio_capture.unavailable",
        "audio_capture.bus_not_found",
        "Subjective sound quality remains outside the CLI pass/fail contract",
    ] {
        assert!(
            test_strategy.contains(required),
            "docs/test-strategy.md missing S70 validation contract: {required}"
        );
    }
}

#[test]
fn repo_contract_screenshot_sequence_docs() {
    let readme = read("docs/workflows.md");
    let cli_spec = read("docs/cli-spec.md");
    let artifact_format = read("docs/artifact-format.md");
    let test_strategy = read("docs/test-strategy.md");

    for required in [
        "godot-qa screenshot sequence --project <project> --frames 300 --every 15",
        "godot-qa screenshot sequence --project <project> --seconds 20 --fps 2",
        "godot-qa screenshot sequence --project <project> --scenario screenshot_sequence_demo",
        "godot-qa screenshot contact-sheet .qa/artifacts/manual/race_sequence --columns 5 --thumb-width 320",
        "missing_scenario",
        "scenario_name_ambiguous",
        "scenario_steps_replayed: false",
        "does not fork or replay `scenario run` semantics",
        "low_information_frame_count",
        "failed_capture_count",
        "visual_risk_warning_count",
        "first_failed_frame",
        "`--allow-low-information`",
        "`--visual-risk`",
        "`--visual-risk-strict`",
        "Baseline approval remains manual",
    ] {
        assert!(
            cli_spec.contains(required),
            "docs/cli-spec.md missing screenshot sequence contract: {required}"
        );
    }

    for required in [
        "<sequence-dir>/",
        "manifest.json",
        "frames/",
        "frame_0000.png",
        "contact_sheet.json",
        "\"screenshot.sequence\"",
        "scenario_scene",
        "scenario_steps_replayed: false",
        "frame_count_requested",
        "capture_every_frames",
        "captured_count",
        "engine_frame",
        "time_msec",
        "low_information_reasons",
        "visual_risk_warnings",
        "source_images",
        "grid geometry",
        "artifact margin",
        "baseline approval remains manual",
    ] {
        assert!(
            artifact_format.contains(required),
            "docs/artifact-format.md missing screenshot sequence artifact contract: {required}"
        );
    }

    for required in [
        "cargo test -p godot-qa-cli repo_contract_screenshot_sequence_docs",
        "cargo run --bin godot-qa -- scenario list --project examples/sample_project --json",
        "res://ui/fixtures/screenshot_sequence_demo.tscn",
        "screenshot sequence --frames 120 --every 20",
        "screenshot contact-sheet --columns 3 --thumb-width 320",
        "low-information frame failures remain `screenshot_low_information`",
        "baseline approval remains manual",
    ] {
        assert!(
            test_strategy.contains(required),
            "docs/test-strategy.md missing screenshot sequence validation contract: {required}"
        );
    }

    for required in [
        "screenshot_sequence_demo",
        "screenshot sequence --scenario <name>",
        "contact-sheet companion JSON",
        "never embed screenshot pixels",
        "auto-approve baselines",
    ] {
        assert!(
            readme.contains(required),
            "docs/workflows.md missing screenshot sequence overview: {required}"
        );
    }
}

#[test]
fn repo_contract_review_templates_docs() {
    let cli_spec = read("docs/cli-spec.md");
    let scenario_format = read("docs/scenario-format.md");
    let artifact_format = read("docs/artifact-format.md");
    let test_strategy = read("docs/test-strategy.md");
    let readme = read("docs/workflows.md");
    let sample_readme = read("examples/sample_project/README.md");

    for required in [
        "godot-qa review list-templates --json",
        "godot-qa review init scenario-smoke",
        "godot-qa review init deterministic-runtime-review",
        "godot-qa session run-script .qa/review-scripts/deterministic_review.yaml --project <project> --json",
        "godot-qa scenario run --file .qa/review-scenarios/smoke_review.yaml --project <project> --json",
        "data.templates[]",
        "generated_command_type",
        "required_parameters",
        "optional_parameters",
        "required_tools",
        "\"run_command\"",
        "\"project_args\"",
        "scenario-smoke",
        "live-visual-review",
        "live-physics-review",
        "live-audio-review",
        "live-event-review",
        "visual-physics-audio",
        "deterministic-runtime-review",
        "Generated files are not auto-run",
        "does not include `review run`",
        "session run-script",
        "scenario run --file",
        "forwarded after Godot's `--` separator",
        "does not interpret values such as `--seed`",
        "project-agnostic",
        "do not approve or write `qa/baselines`",
        "final human review",
        "manual screenshot baseline approval",
    ] {
        assert!(
            cli_spec.contains(required),
            "docs/cli-spec.md missing review template contract: {required}"
        );
    }

    for required in [
        "Review Template Guidance",
        "normal scenario files or normal live-session script files",
        "Generated scenario files are run with",
        "Generated live scripts are run with",
        "scenario run --file .qa/review-scenarios/<name>.yaml",
        "session run-script .qa/review-scripts/<name>.yaml",
        "review metadata block",
        "metadata only and is not an assertion surface",
        "session_start",
        "screenshot_sequence",
        "trace_record",
        "physics_collision_report",
        "audio_capture",
        "event_trace",
        "project_args",
        "forwarded exactly as supplied",
        "not interpreted by the CLI",
        "project-agnostic",
        "must not assume racing games, rafts, combat arenas, player controllers",
        "final human review",
        "screenshot baseline approval remain separate manual decisions",
    ] {
        assert!(
            scenario_format.contains(required),
            "docs/scenario-format.md missing review template guidance: {required}"
        );
    }

    for required in [
        "Review-template artifacts are normal scenario or live evidence",
        "`review init` only writes a reusable scenario or live-script file",
        "scenario run --file",
        "session run-script",
        "screenshots",
        "screenshot sequences",
        "contact sheets",
        "traces",
        "physics collision reports",
        "audio captures",
        "logs",
        "errors",
        "never auto-approves baselines",
        "never writes to `qa/baselines`",
        "never replaces final human review",
    ] {
        assert!(
            artifact_format.contains(required),
            "docs/artifact-format.md missing review evidence notes: {required}"
        );
    }

    for required in [
        "Review template validation (S75)",
        "cargo test -p godot-qa-cli repo_contract_review_templates_docs",
        "cargo run --bin godot-qa -- review list-templates --json",
        "scenario-smoke",
        "live-visual-review",
        "live-physics-review",
        "live-audio-review",
        "live-event-review",
        "visual-physics-audio",
        "deterministic-runtime-review",
        "cargo run --bin godot-qa -- review init scenario-smoke --project examples/sample_project",
        "cargo run --bin godot-qa -- scenario run --file .qa/review-scenarios/smoke_review.yaml --project examples/sample_project --json",
        "cargo run --bin godot-qa -- review init deterministic-runtime-review --project examples/sample_project",
        "cargo run --bin godot-qa -- session run-script .qa/review-scripts/deterministic_review.yaml --project examples/sample_project --json",
        "Project arguments are forwarded exactly as strings and are never interpreted by review templates",
        "do not auto-approve baselines",
        "do not write `qa/baselines`",
        "do not replace final human review",
    ] {
        assert!(
            test_strategy.contains(required),
            "docs/test-strategy.md missing S75 validation guidance: {required}"
        );
    }

    for required in [
        "Review template examples",
        "godot-qa review list-templates --json",
        "godot-qa review init scenario-smoke",
        "godot-qa scenario run --file .qa/review-scenarios/smoke_review.yaml",
        "godot-qa review init deterministic-runtime-review",
        "godot-qa session run-script .qa/review-scripts/deterministic_review.yaml",
        "existing commands",
        "do not replace final human review",
        "do not auto-approve baselines",
        "do not write `qa/baselines`",
    ] {
        assert!(
            readme.contains(required),
            "docs/workflows.md missing review template examples: {required}"
        );
    }

    for required in [
        "Review template examples",
        "godot-qa review list-templates --json",
        "godot-qa review init scenario-smoke",
        "godot-qa scenario run --file .qa/review-scenarios/smoke_review.yaml",
        "godot-qa review init deterministic-runtime-review",
        "godot-qa session run-script .qa/review-scripts/deterministic_review.yaml",
        "accelerate exploratory QA",
        "without replacing final human review",
        "manual screenshot baseline approval",
    ] {
        assert!(
            sample_readme.contains(required),
            "examples/sample_project/docs/workflows.md missing review template examples: {required}"
        );
    }
}

#[test]
fn downstream_tool_improvement_hardening_docs_contract() {
    let cli_spec = read("docs/cli-spec.md");
    let runtime_protocol = read("docs/runtime-bridge-protocol.md");
    let test_strategy = read("docs/test-strategy.md");

    for required in [
        "screenshot capture --setup-script",
        "--stop-after-state",
        "audio_capture.stop_condition_timeout",
        "`--frames <n>` accepts `1..=1000`",
        "empty_selection",
        "Direct live commands",
    ] {
        assert!(
            cli_spec.contains(required),
            "docs/cli-spec.md missing S77 contract: {required}"
        );
    }

    for required in [
        "stop_after_state",
        "state_matches",
        "audio_capture.stop_condition_timeout",
        "frame-based requests accept `1..=1000`",
    ] {
        assert!(
            runtime_protocol.contains(required),
            "docs/runtime-bridge-protocol.md missing S77 protocol contract: {required}"
        );
    }

    for required in [
        "Downstream tool-improvement hardening validation guidance (S77)",
        "audio capture --stop-after-state",
        "live bridge retry",
        "cargo run --bin godot-qa -- check --project examples/sample_project --tests --json",
    ] {
        assert!(
            test_strategy.contains(required),
            "docs/test-strategy.md missing S77 validation contract: {required}"
        );
    }
}

#[test]
fn doctor_addon_version_drift_docs_contract() {
    let cli_spec = read("docs/cli-spec.md");
    let release_checklist = read("docs/release-checklist.md");

    for required in [
        "godot-qa doctor --strict-addon-version",
        "addon_version_mismatch",
        "`data.cli.version`",
        "`data.addon.version`",
        "`data.addon.source_version`",
    ] {
        assert!(
            cli_spec.contains(required),
            "docs/cli-spec.md missing addon version drift contract: {required}"
        );
    }

    assert!(
        release_checklist.contains(
            "Bump `[workspace.package]` version and `addons/godot_qa/plugin.cfg` version together"
        ),
        "docs/release-checklist.md missing the version-bump step"
    );
}

#[test]
fn unsupported_config_file_warning_docs_contract() {
    let cli_spec = read("docs/cli-spec.md");

    for required in [
        "unsupported_config_file",
        "Only `qa/config.yaml` and `qa/config.local.yaml` are read",
        "`qa/config.json`",
        "`qa/config.yml`",
        "`type: \"unsupported_config_file\"`",
        "`severity: \"warning\"`",
    ] {
        assert!(
            cli_spec.contains(required),
            "docs/cli-spec.md missing unsupported config-file warning contract: {required}"
        );
    }
}

#[test]
fn unknown_config_key_warning_docs_contract() {
    let cli_spec = read("docs/cli-spec.md");

    for required in [
        "unknown_config_key",
        "`type: \"unknown_config_key\"`",
        "`severity: \"warning\"`",
        "`key` as the dotted config path",
        "Keys that `godot-qa` does not recognise are ignored",
    ] {
        assert!(
            cli_spec.contains(required),
            "docs/cli-spec.md missing unknown config-key warning contract: {required}"
        );
    }
}

#[test]
fn bundle_schedule_and_log_ignore_docs_contract() {
    let cli_spec = read("docs/cli-spec.md");

    for required in [
        "`scenario.schedule` selects the bundle dispatch order (default `longest-first`); `manifest` keeps bundle expansion order.",
        "`logs.ignore` lists substrings that silence engine `ERROR:`/`WARNING:` lines in every log scan (cold, warm, and the warm session-noise scan); it is project-owned and silences real errors, so fix the game first.",
    ] {
        assert!(
            cli_spec.contains(required),
            "docs/cli-spec.md missing project-config contract: {required}"
        );
    }
}

#[test]
fn artifact_retention_docs_contract() {
    let cli_spec = read("docs/cli-spec.md");
    let artifact_format = read("docs/artifact-format.md");

    for required in [
        "artifacts.keep_runs",
        "`artifacts.keep_runs` must be a positive integer",
        "Unset means unlimited retention",
        "`data.retention`",
        "`pruned` (`scenario_runs`, `bundle_runs`, `sessions` counts)",
        "artifact_prune_failed",
        "started more than one hour before the oldest retained bundle run",
    ] {
        assert!(
            cli_spec.contains(required),
            "docs/cli-spec.md missing artifact retention contract: {required}"
        );
    }

    for required in [
        "artifacts.keep_runs",
        "is off by default",
        "`live/latest` symlink is never removed and the session it points at is always kept",
        "`data.retention`",
        "`meta.json` no longer repeats `runs` or `artifact_index`",
        "started more than one hour before the oldest retained bundle run",
        "so `summary.json` never carries it",
    ] {
        assert!(
            artifact_format.contains(required),
            "docs/artifact-format.md missing artifact retention contract: {required}"
        );
    }
}

/// `artifact prune` is the only command that deletes directories no run of its own produced,
/// so the refusal to guess a retention limit and the exact shape of `data.retention` are
/// contract, not implementation detail.
#[test]
fn artifact_prune_docs_contract() {
    let cli_spec = read("docs/cli-spec.md");
    let artifact_format = read("docs/artifact-format.md");

    for required in [
        "godot-qa artifact prune [--keep-runs <n>] [--dry-run] --json",
        "retention_not_configured",
        "`--keep-runs 0` is rejected with `type: \"invalid_arguments\"` and exit code 2",
        "`data.retention` always carries `keep_runs`, `dry_run`, `bytes_reclaimed`",
        "`pruned` and `removed_paths[]` for a real prune, `would_remove` and \
         `would_remove_paths[]` for `--dry-run`",
        "started more than one hour before the oldest bundle run that survives this prune",
    ] {
        assert!(
            cli_spec.contains(required),
            "docs/cli-spec.md missing artifact prune contract: {required}"
        );
    }

    assert!(
        artifact_format
            .contains("`godot-qa artifact prune [--keep-runs <n>] [--dry-run]` applies the same"),
        "docs/artifact-format.md retention section must point at `artifact prune`"
    );
}

#[test]
fn trace_and_screenshot_stop_after_state_docs_contract() {
    let cli_spec = read("docs/cli-spec.md");
    let runtime_protocol = read("docs/runtime-bridge-protocol.md");

    for required in [
        "trace_record.stop_condition_timeout",
        "screenshot_sequence.stop_condition_timeout",
        "`--stop-after-state '<json>'` stops recording early",
        "is polled once per captured frame",
    ] {
        assert!(
            cli_spec.contains(required),
            "docs/cli-spec.md missing trace/screenshot stop-after-state contract: {required}"
        );
    }

    for required in [
        "trace_record.stop_condition_timeout",
        "screenshot_sequence.stop_condition_timeout",
        "polls the predicate once per captured sample",
    ] {
        assert!(
            runtime_protocol.contains(required),
            "docs/runtime-bridge-protocol.md missing trace/screenshot stop-after-state contract: {required}"
        );
    }
}

#[test]
fn repo_contract_editor_profiler_export_docs() {
    let cli_spec = read("docs/cli-spec.md");
    let artifact_format = read("docs/artifact-format.md");
    let architecture = read("docs/architecture.md");
    let runtime_protocol = read("docs/runtime-bridge-protocol.md");
    let test_strategy = read("docs/test-strategy.md");
    let addon_readme = read("addons/godot_qa/README.md");

    for required in [
        "profile editor export",
        "editor_script_functions",
        "editor_debugger_raw_servers_profile",
        "EditorDebuggerPlugin",
        "EditorDebuggerSession.toggle_profiler",
        "servers:function_signature",
        "servers:profile_frame",
        "servers:profile_total",
        "editor_profile_missing",
        "editor_profile_invalid",
    ] {
        assert!(
            cli_spec.contains(required),
            "docs/cli-spec.md missing editor profiler export contract: {required}"
        );
    }

    for required in [
        ".qa/artifacts/editor-profiler/",
        ".qa/artifacts/profile-editor/<run-id>/profile.json",
        "signature",
        "max_total_ms",
        "bounded retained `frames`",
    ] {
        assert!(
            artifact_format.contains(required),
            "docs/artifact-format.md missing editor profiler artifact contract: {required}"
        );
    }

    for required in [
        "qa_editor_profiler.gd",
        ".qa/artifacts/editor-profiler/latest.json",
        "Godot QA Profiler",
        "profile editor export",
    ] {
        assert!(
            addon_readme.contains(required),
            "addons/godot_qa/docs/workflows.md missing editor profiler addon contract: {required}"
        );
    }

    for required in [
        "qa_editor_profiler.gd",
        "runtime bridge",
        "profile editor export",
    ] {
        assert!(
            architecture.contains(required),
            "docs/architecture.md missing editor profiler architecture contract: {required}"
        );
    }

    for required in [
        "out of this runtime protocol",
        "source: \"editor_script_functions\"",
        ".qa/artifacts/editor-profiler/latest.json",
    ] {
        assert!(
            runtime_protocol.contains(required),
            "docs/runtime-bridge-protocol.md missing editor profiler boundary: {required}"
        );
    }

    for required in [
        "Experimental editor profiler export validation",
        "editor_profile_missing",
        "editor_profile_invalid",
        "servers:function_signature",
        "Godot QA Profiler",
    ] {
        assert!(
            test_strategy.contains(required),
            "docs/test-strategy.md missing editor profiler validation contract: {required}"
        );
    }
}

/// Warm engine-log parity (Tranche D): a warm scenario's `logs.txt` is sliced out
/// of the pooled session log between stderr markers, so `data.logs` and the
/// `godot_log_error` / `godot_log_warning` failures are identical to cold. The
/// marker string and the `data.warm.log_slice` shape are a cross-process contract
/// between the CLI and the GDScript addon, so pin them here.
#[test]
fn warm_engine_log_parity_docs_contract_is_present() {
    let cli_spec = read("docs/cli-spec.md");
    let artifact_format = read("docs/artifact-format.md");
    let runtime_protocol = read("docs/runtime-bridge-protocol.md");
    let test_strategy = read("docs/test-strategy.md");

    for required in [
        "scenario-log begin",
        "log_slice",
        "warm_log_markers_missing",
        "`data.warm.session_id`",
        "warm_log_window",
        "markers_missing",
        "warm_session_log_noise",
        "`data.warm.session_log_noise`",
        "outside_scenario_windows",
    ] {
        assert!(
            cli_spec.contains(required),
            "docs/cli-spec.md missing warm engine-log parity contract: {required}"
        );
    }

    for required in ["scenario-log begin", "log_slice"] {
        assert!(
            artifact_format.contains(required),
            "docs/artifact-format.md missing warm engine-log parity contract: {required}"
        );
    }

    for required in [
        "### `scenario.run`",
        "[godot-qa] scenario-log begin <run-id>",
        "[godot-qa] scenario-log end <run-id>",
        "`run_id` is optional",
    ] {
        assert!(
            runtime_protocol.contains(required),
            "docs/runtime-bridge-protocol.md missing scenario.run marker contract: {required}"
        );
    }

    for required in [
        "Warm engine-log parity coverage",
        "warm_log_markers_missing",
    ] {
        assert!(
            test_strategy.contains(required),
            "docs/test-strategy.md missing warm engine-log parity contract: {required}"
        );
    }
}

/// Project-level engine-log ignore list (Tranche F5): `logs.ignore` is consumed
/// by the cold scan, the warm per-scenario scan and the warm session-noise scan
/// from ONE list, and every run publishes what that list silenced. The counters
/// and the parity claim are a consumer-visible contract, so pin them.
#[test]
fn project_log_ignore_list_docs_contract_is_present() {
    let cli_spec = read("docs/cli-spec.md");
    let artifact_format = read("docs/artifact-format.md");
    let test_strategy = read("docs/test-strategy.md");

    for required in [
        "`ignored_count`",
        "`ignore_patterns_applied`",
        "the number of project `logs.ignore` patterns that were in force for the run",
        "The project's `logs.ignore` list is applied on top of the built-in engine-noise ignores by cold runs, warm runs, and the warm session-noise scan alike, so cold and warm never disagree about what counts as noise.",
        "one list configured once in `qa/config.yaml` is applied identically by cold runs, warm runs, and the warm session-noise scan",
        "fix the game first and ignore only what the game cannot fix",
    ] {
        assert!(
            cli_spec.contains(required),
            "docs/cli-spec.md missing project log-ignore contract: {required}"
        );
    }

    for required in [
        "`ignored_count`, and `ignore_patterns_applied`",
        "how many project `logs.ignore` patterns were in force",
    ] {
        assert!(
            artifact_format.contains(required),
            "docs/artifact-format.md missing project log-ignore contract: {required}"
        );
    }

    for required in [
        "Project log-ignore coverage",
        "data.logs.ignored_count",
        "data.logs.ignore_patterns_applied",
    ] {
        assert!(
            test_strategy.contains(required),
            "docs/test-strategy.md missing project log-ignore contract: {required}"
        );
    }
}
