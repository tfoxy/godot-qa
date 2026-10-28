// Extracted from run.rs; included by mod.rs to preserve the original module namespace.

#[cfg(test)]
mod tests {
    use super::{
        RuntimeExecutionOptions, VisualRiskExecutionOptions, apply_screenshot_quality,
        apply_visual_risk_analysis, duration_millis_u64, execute_runtime_run,
        generated_tree_scenario, latest_active_progress_event, load_runtime_result,
        process_summary_json, profile_result_from_runtime, runtime_log_failures,
        runtime_scenario_payload, scenario_contains_audio_capture,
        scenario_contains_visual_assertion, scenario_godot_args, scenario_project_args,
        scenario_requests_screenshot, scenario_requires_audio_evidence,
        scenario_requires_windowed_media_evidence, scenario_run_timeout_failure,
        scenario_timeout_duration, scenario_with_effective_launch_args,
        selected_audio_for_scenario, selected_headless_for_scenario, slow_scenario_summary_json,
        snapshot_artifacts_for, time_control_warnings, validate_runtime_command_result,
        validate_runtime_failures, variant_scenario, write_runtime_request,
    };
    use crate::artifacts;
    use crate::artifacts::write_json;
    use crate::scenario_doc::validate_scenario_document;
    use godot_qa_core::config::ProjectConfig;
    use godot_qa_core::profile::{ProfilePercentiles, profile_percentiles};
    use godot_qa_core::results::CommandResult;
    use godot_qa_process::GodotRunResult;
    use serde_json::{Value, json};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn assert_path_eq(actual: &str, expected: &Path) {
        let actual = Path::new(actual);
        if actual.exists() && expected.exists() {
            assert_eq!(
                fs::canonicalize(actual).unwrap(),
                fs::canonicalize(expected).unwrap()
            );
        } else {
            assert_eq!(actual, expected);
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("godot-qa-{name}-{unique}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn run_artifact_contract_uses_clean_public_path_keys() {
        let root = temp_dir("run-artifact-contract");
        let scenario_path = root.join("qa/scenarios/artifact_contract.json");
        let scenario = json!({"name":"artifact_contract","scene":"res://main.tscn","steps":[]});
        let artifacts = crate::artifacts::prepare_run_artifacts(
            &root.join(".qa/artifacts"),
            &scenario,
            &root,
            &scenario_path,
            "scenario run artifact_contract",
            None,
            &[],
            &[],
            Some("run-001"),
            Some("0Z"),
        )
        .unwrap();
        write(&artifacts.logs_path, "ERROR: FATAL: Signal 11\n");

        let request: Value =
            serde_json::from_str(&fs::read_to_string(&artifacts.request_path).unwrap()).unwrap();
        let artifact_map = request["artifacts"].as_object().unwrap();
        assert_eq!(artifact_map["run_root"], json!(artifacts.root_dir));
        assert_eq!(artifact_map["result_path"], json!(artifacts.result_path));
        assert_eq!(artifact_map["logs_path"], json!(artifacts.logs_path));
        for legacy_key in ["root", "meta", "request", "result", "logs"] {
            assert!(
                !artifact_map.contains_key(legacy_key),
                "legacy artifact key should be absent: {legacy_key}"
            );
        }

        let failure = load_runtime_result(
            "scenario run",
            &artifacts.result_path,
            &request["artifacts"],
            None,
        )
        .unwrap_err();
        assert_eq!(failure.kind, "scenario_result_missing");
        let failure_artifacts = failure.artifacts.as_ref().unwrap();
        assert_eq!(failure_artifacts["run_root"], json!(artifacts.root_dir));
        assert_eq!(
            failure_artifacts["result_path"],
            json!(artifacts.result_path)
        );
        assert_eq!(failure_artifacts["logs_path"], json!(artifacts.logs_path));
        assert_eq!(
            failure.actual.as_ref().unwrap()["logs_path"],
            json!(artifacts.logs_path)
        );
        assert!(failure_artifacts.get("root").is_none());
        assert!(failure_artifacts.get("result").is_none());
        assert!(failure_artifacts.get("logs").is_none());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn layout_audit_current_requires_scenario_without_live() {
        let config = test_config(Path::new("/nonexistent-layout-audit-args"));
        let result = super::run_layout_audit_current(&config, None, None, None, false, false);
        assert!(!result.ok);
        assert_eq!(result.failures[0].kind, "invalid_layout_args");
        assert!(
            result.failures[0]
                .message
                .as_deref()
                .unwrap()
                .contains("requires --scenario"),
            "unexpected message: {:?}",
            result.failures[0].message
        );
    }

    fn test_config(root: &Path) -> ProjectConfig {
        ProjectConfig {
            project_root: root.to_path_buf(),
            repo_root: root.to_path_buf(),
            verbose: false,
            godot_binary: Some(root.join("missing-godot").to_string_lossy().into_owned()),
            qa_dir: root.join("qa"),
            artifacts_dir: root.join(".qa/artifacts"),
            scenarios_dir: root.join("qa/scenarios"),
            addon_dir: root.join("addons/godot_qa"),
            repo_addon_dir: root.join("addons/godot_qa"),
            addon_identity_cache: Default::default(),
            godot_launch_prefix: Vec::new(),
            godot_windowed_launch_prefix: Vec::new(),
            godot_args: Vec::new(),
            godot_project_args: Vec::new(),
            godot_launch_mode: godot_qa_core::config::GodotLaunchMode::Auto,
            godot_audio_mode: godot_qa_core::config::GodotAudioMode::Auto,
            scenario_warm_default: false,
            scenario_warm_strict_default: false,
            scenario_jobs_default: None,
            project_config_failure: None,
            artifacts_keep_runs: None,
            scenario_schedule_default: None,
            log_ignore_patterns: Vec::new(),
            config_warnings: Vec::new(),
        }
    }

    fn test_process_result(root: &Path, returncode: i32) -> GodotRunResult {
        GodotRunResult {
            command: vec![
                "godot".to_string(),
                "--headless".to_string(),
                "--path".to_string(),
                root.to_string_lossy().into_owned(),
            ],
            returncode,
            termination: None,
            process_timing: None,
            stdout: String::new(),
            stderr: String::new(),
            display: godot_qa_process::resolve_project_display_launch_metadata(root, &[], true),
            pid: None,
            process_group: None,
            cleanup: None,
            infrastructure_failure_kind: None,
            timed_out: false,
            timeout_ms: None,
            output_truncation: None,
        }
    }

    fn display_startup_failure(root: &Path) -> godot_qa_process::DisplayLaunchMetadata {
        let mut display =
            godot_qa_process::resolve_project_display_launch_metadata(root, &[], false);
        display.startup_attempted = true;
        display.startup_failure_kind = Some("display_startup_exhausted".to_string());
        display.startup_failure_retryable = true;
        display.startup_failure_guidance =
            Some("Retry after display resources are available.".to_string());
        display
    }

    fn test_run_artifacts(root: &Path) -> crate::artifacts::RunArtifacts {
        crate::artifacts::RunArtifacts {
            run_id: "run-1".to_string(),
            root_dir: root.to_path_buf(),
            meta_path: root.join("meta.json"),
            request_path: root.join("request.json"),
            result_path: root.join("result.json"),
            logs_path: root.join("logs.txt"),
            progress_path: root.join("progress.jsonl"),
            slow_summary_path: root.join("slow-summary.json"),
            snapshots_dir: root.join("snapshots"),
            temporal_dir: root.join("temporal"),
            audio_dir: root.join("audio"),
            assertion_manifest: crate::assertion_proof::manifest(&json!({})),
        }
    }

    fn strict_summary_result(
        root: &Path,
        returncode: i32,
        runtime_result: &Value,
        logs: &str,
    ) -> godot_qa_core::results::CommandResult {
        summary_result_with_scenario(root, returncode, runtime_result, logs, &json!({}))
    }

    fn summary_result_with_scenario(
        root: &Path,
        returncode: i32,
        runtime_result: &Value,
        logs: &str,
        scenario: &Value,
    ) -> godot_qa_core::results::CommandResult {
        let result_path = root.join("result.json");
        let logs_path = root.join("logs.txt");
        write(&logs_path, logs);
        let artifacts = json!({
            "run_root": root,
            "result_path": result_path,
            "logs_path": logs_path,
        });
        let mut base_data =
            serde_json::Map::from_iter([("artifacts".to_string(), artifacts.clone())]);
        validate_runtime_command_result(
            "scenario run",
            scenario,
            &mut base_data,
            &result_path,
            &artifacts,
            &test_process_result(root, returncode),
            runtime_result,
            &logs_path,
            &[],
        )
    }

    #[test]
    fn truncated_capture_fails_even_when_log_pattern_is_ignored() {
        let root = temp_dir("truncated-capture");
        fs::create_dir_all(&root).unwrap();
        let result_path = root.join("result.json");
        let logs_path = root.join("logs.txt");
        write(&logs_path, "[godot-qa output capture truncated]\n");
        let artifacts =
            json!({"run_root": root, "result_path": result_path, "logs_path": logs_path});
        let mut base_data = serde_json::Map::from_iter([("artifacts".into(), artifacts.clone())]);
        let mut process = test_process_result(&root, 0);
        process.output_truncation = Some(godot_qa_process::OutputTruncation {
            stdout_total_bytes: 9 * 1024 * 1024,
            stderr_total_bytes: 0,
            retained_bytes_per_stream: 8 * 1024 * 1024,
            raw_stdout_path: root.join("logs.stdout.txt").to_string_lossy().into_owned(),
            raw_stderr_path: root.join("logs.stderr.txt").to_string_lossy().into_owned(),
        });
        let result = validate_runtime_command_result(
            "scenario run",
            &json!({}),
            &mut base_data,
            &result_path,
            &artifacts,
            &process,
            &json!({"ok": true, "failures": []}),
            &logs_path,
            &["capture truncated".into()],
        );
        assert!(!result.ok);
        assert_eq!(result.failures[0].kind, "output_capture_truncated");
        assert_eq!(
            result.data["process"]["output_truncation"]["stdout_total_bytes"],
            9 * 1024 * 1024
        );
        let _ = fs::remove_dir_all(root);
    }

    fn runtime_audio_result(
        root: &Path,
        runtime_result: &Value,
    ) -> godot_qa_core::results::CommandResult {
        let result_path = root.join("result.json");
        let logs_path = root.join("logs.txt");
        let audio_dir = root.join("audio");
        fs::create_dir_all(&audio_dir).unwrap();
        write(&logs_path, "");
        let artifacts = json!({
            "run_root": root,
            "result_path": result_path,
            "logs_path": logs_path,
            "audio_dir": audio_dir,
        });
        let mut base_data =
            serde_json::Map::from_iter([("artifacts".to_string(), artifacts.clone())]);
        let result = validate_runtime_command_result(
            "scenario run",
            &json!({}),
            &mut base_data,
            &result_path,
            &artifacts,
            &test_process_result(root, 0),
            runtime_result,
            &logs_path,
            &[],
        );
        super::persist_result(&result_path, result)
    }

    fn write_runtime_audio_wav(root: &Path, name: &str) -> PathBuf {
        let path = root.join("audio").join(format!("{name}.wav"));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(
            &path,
            wav_pcm16(8_000, 1, &[0.0, 0.25, -0.25, 0.5, -0.5, 0.0]),
        )
        .unwrap();
        path
    }

    fn wav_pcm16(sample_rate: u32, channels: u16, mono_samples: &[f64]) -> Vec<u8> {
        let mut sample_bytes = Vec::new();
        for sample in mono_samples {
            let clamped = sample.clamp(-1.0, 1.0);
            let pcm = (clamped * i16::MAX as f64).round() as i16;
            for _ in 0..channels {
                sample_bytes.extend_from_slice(&pcm.to_le_bytes());
            }
        }
        let byte_rate = sample_rate * u32::from(channels) * 2;
        let block_align = channels * 2;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + sample_bytes.len() as u32).to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        bytes.extend_from_slice(&byte_rate.to_le_bytes());
        bytes.extend_from_slice(&block_align.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(sample_bytes.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&sample_bytes);
        bytes
    }

    /// Warm engine-log parity: once the warm runner has materialized this run's
    /// slice of the pooled session log as `<run_root>/logs.txt`, the SHARED
    /// finalizer must reach the same verdict it reaches for cold — even though
    /// the warm process result is synthesized and carries no stdout/stderr of its
    /// own. If this ever regresses, warm silently stops reporting engine errors.
    #[test]
    fn finalize_runtime_run_flags_log_errors_for_a_synthesized_warm_process() {
        let root = temp_dir("warm-log-parity-finalize");
        let scenario_path = root.join("qa/scenarios/warm_log_parity.json");
        let scenario = json!({
            "name": "warm_log_parity",
            "scene": "res://main.tscn",
            "steps": [],
        });
        let artifacts = test_run_artifacts(&root);
        write(
            &artifacts.result_path,
            &json!({"ok": true, "snapshots": [], "failures": []}).to_string(),
        );
        // What materialize_warm_scenario_logs writes: the marker-bounded window,
        // marker lines included.
        let marker = super::WARM_LOG_MARKER_PREFIX;
        write(
            &artifacts.logs_path,
            &format!(
                "{marker} begin run-1\nERROR: Lambda capture at index 0 was freed\n{marker} end \
                 run-1\n"
            ),
        );
        let base_data =
            serde_json::Map::from_iter([("artifacts".to_string(), artifacts.to_json())]);

        let result = super::finalize_runtime_run(
            "scenario run",
            "scenario run failed",
            &test_config(&root),
            &scenario_path,
            &scenario,
            &artifacts,
            false,
            true,
            RuntimeExecutionOptions::default(),
            base_data,
            &super::synth_warm_process_result(
                godot_qa_process::resolve_project_display_launch_metadata(&root, &[], true),
            ),
        );

        let _ = fs::remove_dir_all(&root);

        assert!(!result.ok);
        assert_eq!(result.data["logs"]["status"], json!("error"));
        assert_eq!(result.data["logs"]["error_count"], json!(1));
        assert_eq!(result.data["logs"]["warning_count"], json!(0));
        assert_eq!(result.data["outcome"]["reason"], json!("godot_log_error"));
        assert_eq!(
            result
                .failures
                .iter()
                .filter(|failure| failure.kind == "godot_log_error")
                .count(),
            1,
            "{:?}",
            result.failures
        );
    }

    #[test]
    fn completed_assertion_failure_precedes_process_completion_timeout() {
        let root = temp_dir("result-then-teardown-timeout");
        let mut artifacts = test_run_artifacts(&root);
        artifacts.assertion_manifest =
            crate::assertion_proof::manifest(&json!({"assert":[{"no_errors":true}]}));
        write(
            &artifacts.request_path,
            &json!({"run":{"id":"run-1","nonce":"current","started_at":"0Z"}}).to_string(),
        );
        write(&artifacts.result_path, &json!({
            "ok": false,
            "run_identity": {"run_id":"run-1","nonce":"current","started_at":"0Z"},
            "snapshots": [],
            "assertion_coverage": {"schema_version":1,"run_id":"run-1","nonce":"current","expected":1,"started":1,"finished":1,"passed":0,"failed":1,"complete":true,"cli_assertions_excluded":0,"entries":[{"source_id":"/assert/0","runtime_path":"/assert/0","operation":"no_errors","assertion":{"no_errors":true},"assertion_identity":crate::assertion_proof::definition_identity(&json!({"no_errors":true})),"status":"failed"}]},
            "failures": [{"type":"gameplay_assertion_failed","message":"first failure"}]
        }).to_string());
        let scenario = json!({"name":"teardown_timeout","scene":"res://main.tscn"});
        let mut process = test_process_result(&root, -1);
        process.timed_out = true;
        process.timeout_ms = Some(3000);
        let result = super::finalize_runtime_run(
            "scenario run",
            "scenario run failed",
            &test_config(&root),
            &root.join("scenario.json"),
            &scenario,
            &artifacts,
            false,
            true,
            RuntimeExecutionOptions::default(),
            serde_json::Map::new(),
            &process,
        );
        assert!(!result.ok);
        assert_eq!(result.failures[0].kind, "gameplay_assertion_failed");
        assert_eq!(result.failures[1].kind, "scenario_completion_timeout");
        assert_eq!(result.data["completion"]["assertions_completed"], true);
        assert_eq!(result.data["completion"]["assertions_passed"], false);
        assert_eq!(result.data["completion"]["process_exited_cleanly"], false);
        assert_eq!(result.data["completion"]["result_identity_verified"], true);
    }

    #[test]
    fn stale_result_during_timeout_cannot_claim_completed_assertions() {
        let root = temp_dir("stale-result-timeout");
        let artifacts = test_run_artifacts(&root);
        write(
            &artifacts.request_path,
            &json!({"run":{"id":"run-1","nonce":"current","started_at":"0Z"}}).to_string(),
        );
        write(
            &artifacts.result_path,
            &json!({"ok":true,"failures":[],"run_identity":{"run_id":"run-1","nonce":"previous"}})
                .to_string(),
        );
        let mut process = test_process_result(&root, -1);
        process.timed_out = true;
        process.timeout_ms = Some(3000);
        let result = super::finalize_runtime_run(
            "scenario run",
            "scenario run failed",
            &test_config(&root),
            &root.join("scenario.json"),
            &json!({"name":"stale"}),
            &artifacts,
            false,
            true,
            RuntimeExecutionOptions::default(),
            serde_json::Map::new(),
            &process,
        );
        assert!(!result.ok);
        assert_eq!(result.failures[0].kind, "scenario_run_timeout");
        assert_eq!(result.data["completion"]["result_written"], true);
        assert_eq!(result.data["completion"]["assertions_completed"], false);
        assert_eq!(result.data["completion"]["result_identity_verified"], false);
    }

    fn assert_strict_summaries(result: &godot_qa_core::results::CommandResult) {
        assert!(result.data.get("process").is_some());
        assert!(result.data.get("runtime_result").is_some());
        assert!(result.data.get("logs").is_some());
        assert!(result.data.get("outcome").is_some());
    }

    include!("tests_profile.rs");
    include!("tests_visual.rs");
    include!("tests_runtime_artifacts.rs");
}
