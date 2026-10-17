    // Shared warm test helpers, plus the classification, strict-annotation and
    // timing tests. Included into `mod warm_tests` in `run/warm.rs`.

    fn cmd(ok: bool, failure_kinds: &[&str], snapshots: &[&str]) -> CommandResult {
        let mut data = Map::new();
        data.insert(
            "snapshots".to_string(),
            json!(snapshots.iter().map(|s| s.to_string()).collect::<Vec<_>>()),
        );
        CommandResult {
            ok,
            command: "scenario run".to_string(),
            message: String::new(),
            data,
            failures: failure_kinds
                .iter()
                .map(|kind| FailureDetail {
                    kind: kind.to_string(),
                    ..FailureDetail::default()
                })
                .collect(),
            exit_code: if ok { 0 } else { 1 },
        }
    }

    fn warm_temp_dir(name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("godot-qa-{name}-{unique}"));
        std::fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    fn warm_test_artifacts(run_root: &Path, run_id: &str) -> artifacts::RunArtifacts {
        artifacts::RunArtifacts {
            run_id: run_id.to_string(),
            root_dir: run_root.to_path_buf(),
            meta_path: run_root.join("meta.json"),
            request_path: run_root.join("request.json"),
            result_path: run_root.join("result.json"),
            logs_path: run_root.join("logs.txt"),
            progress_path: run_root.join("progress.jsonl"),
            slow_summary_path: run_root.join("slow-summary.json"),
            snapshots_dir: run_root.join("snapshots"),
            temporal_dir: run_root.join("temporal"),
            audio_dir: run_root.join("audio"),
        }
    }

    fn fake_handle(id: &str, fps: i64) -> godot_qa_runtime::PooledSessionHandle {
        godot_qa_runtime::PooledSessionHandle {
            record: godot_qa_runtime::SessionRecord {
                process_identity: None,
                protocol_version: "0.1".to_string(),
                session_id: id.to_string(),
                pid: 0,
                host: "127.0.0.1".to_string(),
                port: 0,
                scene: String::new(),
                viewport: vec![1920, 1080],
                artifact_dir: String::new(),
                request_path: String::new(),
                token_file: String::new(),
                started_at: String::new(),
                headless: true,
                project_args: Vec::new(),
                frame_pacing: Some(godot_qa_runtime::SessionFramePacing {
                    fixed_fps: fps,
                    source: "test".to_string(),
                    project_setting: None,
                }),
                target: godot_qa_runtime::SessionTarget::Desktop,
                android: None,
            },
            token: "token".to_string(),
        }
    }

    /// A config for the pool teardown paths. Every handle these tests build has
    /// `pid: 0` / `port: 0`, and `stop_pooled_session` short-circuits on both (the
    /// bridge command fails to connect immediately and
    /// `wait_for_godot_process_exit` / `stop_godot_process` reject a non-positive
    /// pid without waiting), so no test ever blocks on a process.
    fn discard_test_config() -> ProjectConfig {
        ProjectConfig {
            project_root: PathBuf::from("/nonexistent-warm-pool-test"),
            repo_root: PathBuf::from("/nonexistent-warm-pool-test"),
            verbose: false,
            godot_binary: None,
            qa_dir: PathBuf::from("/nonexistent-warm-pool-test/qa"),
            artifacts_dir: PathBuf::from("/nonexistent-warm-pool-test/.qa/artifacts"),
            scenarios_dir: PathBuf::from("/nonexistent-warm-pool-test/qa/scenarios"),
            addon_dir: PathBuf::from("/nonexistent-warm-pool-test/addons/godot_qa"),
            repo_addon_dir: PathBuf::from("/nonexistent-warm-pool-test/addons/godot_qa"),
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
            config_warnings: Vec::new(),
            artifacts_keep_runs: None,
            scenario_schedule_default: None,
            log_ignore_patterns: Vec::new(),
        }
    }

    fn opts_with_jobs(jobs: i64) -> crate::bundle::ScenarioBundleRunOptions {
        crate::bundle::ScenarioBundleRunOptions {
            jobs,
            ..crate::bundle::ScenarioBundleRunOptions::default()
        }
    }

    fn test_pool(max: usize) -> WarmSessionPool {
        WarmSessionPool::new(fake_handle("seed", 60), max, Some(Duration::from_millis(400)))
    }

    fn start_failure(kind: &str) -> FailureDetail {
        FailureDetail {
            kind: kind.to_string(),
            message: Some("session start failed".to_string()),
            ..FailureDetail::default()
        }
    }

    const MARKER_RUN: &str = "20260101T000000Z";
    const OTHER_RUN: &str = "20260101T000005Z";

    fn begin_marker(run_id: &str) -> String {
        format!("{WARM_LOG_MARKER_PREFIX} begin {run_id}")
    }

    fn end_marker(run_id: &str) -> String {
        format!("{WARM_LOG_MARKER_PREFIX} end {run_id}")
    }

    #[test]
    fn cold_isolation_field_forces_cold() {
        let scenario = json!({
            "name": "x",
            "scene": "res://x.tscn",
            "execution": {"isolation": "cold"},
        });
        assert_eq!(
            scenario_needs_cold_isolation(&scenario, Path::new("."), 60),
            Some(DECLARED_COLD_REASON)
        );
        assert_eq!(
            classify_warm_bucket(&scenario, Path::new("."), 60, None, None),
            WarmBucket::TrulyCold {
                reason: DECLARED_COLD_REASON,
                strict: false
            }
        );
    }

    #[test]
    fn scenario_godot_args_force_cold() {
        let scenario = json!({
            "name": "x",
            "scene": "res://x.tscn",
            "godot_args": ["--single-window"],
        });
        assert_eq!(
            scenario_needs_cold_isolation(&scenario, Path::new("."), 60),
            Some("godot_args")
        );
        assert_eq!(
            classify_warm_bucket(&scenario, Path::new("."), 60, None, None),
            WarmBucket::TrulyCold {
                reason: "godot_args",
                strict: true
            }
        );
    }

    #[test]
    fn differing_fixed_fps_forces_cold_but_matching_is_warm() {
        let cold = json!({
            "name": "x", "scene": "res://x.tscn",
            "determinism": {"fixed_fps": 30},
        });
        assert_eq!(
            scenario_needs_cold_isolation(&cold, Path::new("."), 60),
            Some("fixed_fps_mismatch")
        );
        assert_eq!(
            classify_warm_bucket(&cold, Path::new("."), 60, None, None),
            WarmBucket::TrulyCold {
                reason: "fixed_fps_mismatch",
                strict: true
            }
        );
        let warm = json!({
            "name": "x", "scene": "res://x.tscn",
            "determinism": {"fixed_fps": 60},
        });
        assert_eq!(scenario_needs_cold_isolation(&warm, Path::new("."), 60), None);
        assert_eq!(
            classify_warm_bucket(&warm, Path::new("."), 60, None, None),
            WarmBucket::Warm {
                headless: true,
                audio: false
            }
        );
    }

    #[test]
    fn multi_viewport_is_truly_cold_and_strict() {
        let scenario = json!({
            "name": "x", "scene": "res://x.tscn",
            "viewports": [[1920, 1080], [1280, 720]],
        });
        assert_eq!(
            classify_warm_bucket(&scenario, Path::new("."), 60, None, None),
            WarmBucket::TrulyCold {
                reason: "multi_viewport",
                strict: true
            }
        );
    }

    #[test]
    fn single_viewport_layout_audit_is_windowed_warm() {
        // A single-viewport layout_audit is NOT structurally cold: it runs the
        // normal single path and its audit reads UI snapshot + screenshot
        // evidence, so it is WINDOWED-warm (needs a real-pixel session), never a
        // `layout_matrix` truly-cold scenario.
        let scenario = json!({
            "name": "x", "scene": "res://x.tscn",
            "viewports": [[1280, 720]],
            "assert": [{"layout_audit": "strict"}],
        });
        assert_eq!(structural_cold_reason(&scenario, Path::new("."), 60), None);
        assert_eq!(media_cold_reason(&scenario), Some("windowed_media"));
        assert_eq!(
            classify_warm_bucket(&scenario, Path::new("."), 60, None, None),
            WarmBucket::Warm {
                headless: false,
                audio: false
            }
        );
    }

    #[test]
    fn zero_viewport_layout_audit_is_windowed_warm() {
        // No `viewports` block at all: still the normal single path, still
        // windowed-warm (never routed to the matrix, never truly-cold).
        let scenario = json!({
            "name": "x", "scene": "res://x.tscn",
            "assert": [{"layout_audit": "default"}],
        });
        assert_eq!(structural_cold_reason(&scenario, Path::new("."), 60), None);
        assert_eq!(
            classify_warm_bucket(&scenario, Path::new("."), 60, None, None),
            WarmBucket::Warm {
                headless: false,
                audio: false
            }
        );
    }

    #[test]
    fn multi_viewport_layout_audit_is_truly_cold_multi_viewport() {
        // A layout audit across 2+ viewports IS a real matrix -> structurally
        // cold via `multi_viewport` (which now subsumes the old `layout_matrix`).
        let scenario = json!({
            "name": "x", "scene": "res://x.tscn",
            "viewports": [[1280, 720], [1920, 1080]],
            "assert": [{"layout_audit": "strict"}],
        });
        assert_eq!(
            classify_warm_bucket(&scenario, Path::new("."), 60, None, None),
            WarmBucket::TrulyCold {
                reason: "multi_viewport",
                strict: true
            }
        );
    }

    #[test]
    fn plain_scenario_is_headless_warm() {
        let scenario = json!({"name": "x", "scene": "res://x.tscn"});
        assert_eq!(scenario_needs_cold_isolation(&scenario, Path::new("."), 60), None);
        assert_eq!(
            classify_warm_bucket(&scenario, Path::new("."), 60, None, None),
            WarmBucket::Warm {
                headless: true,
                audio: false
            }
        );
    }

    #[test]
    fn save_screenshot_is_windowed_warm_eligible() {
        let scenario = json!({
            "name": "x", "scene": "res://x.tscn",
            "snapshots": {"save_screenshot": true},
        });
        // media-only -> windowed-warm, NOT truly cold.
        assert_eq!(
            classify_warm_bucket(&scenario, Path::new("."), 60, None, None),
            WarmBucket::Warm {
                headless: false,
                audio: false
            }
        );
        assert_eq!(media_cold_reason(&scenario), Some("windowed_media"));
    }

    #[test]
    fn visual_pixel_assertion_is_windowed_warm_eligible() {
        let scenario = json!({
            "name": "x", "scene": "res://x.tscn",
            "assert": [{"not_occluded": {"target": "res://x"}}],
        });
        assert_eq!(
            classify_warm_bucket(&scenario, Path::new("."), 60, None, None),
            WarmBucket::Warm {
                headless: false,
                audio: false
            }
        );
    }

    #[test]
    fn audio_capture_is_headless_warm_with_real_audio() {
        // Audio capture needs the real audio driver but NO window -> the
        // display-off + audio-on grid cell (headless display, real audio).
        let scenario = json!({
            "name": "x", "scene": "res://x.tscn",
            "steps": [{"audio_capture": {"name": "music", "duration_seconds": 0.5}}],
        });
        assert_eq!(
            classify_warm_bucket(&scenario, Path::new("."), 60, None, None),
            WarmBucket::Warm {
                headless: true,
                audio: true
            }
        );
    }

    #[test]
    fn screenshot_plus_audio_capture_is_windowed_warm_with_real_audio() {
        // Needs BOTH a window (screenshot) AND real audio (capture) -> the
        // display-on + audio-on grid cell.
        let scenario = json!({
            "name": "x", "scene": "res://x.tscn",
            "snapshots": {"save_screenshot": true},
            "steps": [{"audio_capture": {"name": "music", "duration_seconds": 0.5}}],
        });
        assert_eq!(
            classify_warm_bucket(&scenario, Path::new("."), 60, None, None),
            WarmBucket::Warm {
                headless: false,
                audio: true
            }
        );
    }

    #[test]
    fn media_axis_overrides_are_honored_in_the_bucket() {
        let scenario = json!({"name": "x", "scene": "res://x.tscn"});
        // --audio / godot.audio_mode: on forces the real audio flavor even with no
        // capture evidence.
        assert_eq!(
            classify_warm_bucket(&scenario, Path::new("."), 60, None, Some(true)),
            WarmBucket::Warm {
                headless: true,
                audio: true
            }
        );
        // --windowed / godot.launch_mode: windowed forces the windowed display
        // flavor even with no media evidence.
        assert_eq!(
            classify_warm_bucket(&scenario, Path::new("."), 60, Some(false), None),
            WarmBucket::Warm {
                headless: false,
                audio: false
            }
        );
    }

    #[test]
    fn media_plus_structural_reason_is_truly_cold_and_strict() {
        // A scenario that needs windowed media AND has its own godot_args is
        // truly-cold: the structural reason wins and is a strict violation.
        let scenario = json!({
            "name": "x", "scene": "res://x.tscn",
            "snapshots": {"save_screenshot": true},
            "godot_args": ["--single-window"],
        });
        assert_eq!(
            classify_warm_bucket(&scenario, Path::new("."), 60, None, None),
            WarmBucket::TrulyCold {
                reason: "godot_args",
                strict: true
            }
        );
    }

    #[test]
    fn windowed_warm_scenario_running_windowed_is_not_a_strict_violation() {
        // When a media-only scenario RUNS in the windowed warm session it is
        // ran_warm=true, mode=windowed, and never a strict violation.
        let mut result = cmd(true, &[], &[]);
        annotate_warm_result(
            &mut result,
            true,
            Some(WARM_MODE_WINDOWED),
            Some(WARM_AUDIO_DUMMY),
            None,
            true,
        );
        assert!(result.ok);
        assert!(result.failures.is_empty());
        assert_eq!(result.data["warm"]["ran_warm"], json!(true));
        assert_eq!(result.data["warm"]["mode"], json!("windowed"));
        assert_eq!(result.data["warm"]["audio"], json!("dummy"));
    }

    #[test]
    fn windowed_session_unavailable_falls_back_cold_without_strict_violation() {
        // If the windowed session can't start, a media-only scenario runs cold
        // and that is NOT a strict violation (infrastructure, not declaration).
        let mut result = cmd(true, &[], &[]);
        annotate_warm_result(
            &mut result,
            false,
            None,
            None,
            Some(WINDOWED_SESSION_UNAVAILABLE_REASON),
            true,
        );
        assert!(result.ok);
        assert!(result.failures.is_empty());
        assert_eq!(result.data["warm"]["ran_warm"], json!(false));
        assert_eq!(
            result.data["warm"]["reason"],
            json!("windowed_session_unavailable")
        );
    }

    #[test]
    fn declared_cold_never_fails_strict() {
        let mut result = cmd(true, &[], &[]);
        annotate_warm_result(&mut result, false, None, None, Some(DECLARED_COLD_REASON), true);
        assert!(result.ok);
        assert!(result.failures.is_empty());
        assert_eq!(result.data["warm"]["declared"], json!(true));
        assert_eq!(result.data["warm"]["ran_warm"], json!(false));
    }

    #[test]
    fn structural_fallback_fails_when_strict() {
        let mut result = cmd(true, &[], &[]);
        annotate_warm_result(&mut result, false, None, None, Some("godot_args"), true);
        assert!(!result.ok);
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.failures.len(), 1);
        assert_eq!(result.failures[0].kind, "warm_fallback_forced_cold");
        assert_eq!(result.failures[0].actual, Some(json!("godot_args")));
    }

    #[test]
    fn transport_failure_fails_when_strict() {
        let mut result = cmd(true, &[], &[]);
        annotate_warm_result(
            &mut result,
            false,
            None,
            None,
            Some("warm_transport_failure"),
            true,
        );
        assert!(!result.ok);
        assert_eq!(result.failures.len(), 1);
        assert_eq!(result.failures[0].kind, "warm_fallback_forced_cold");
    }

    #[test]
    fn non_declared_fallback_does_not_fail_when_not_strict() {
        let mut result = cmd(true, &[], &[]);
        annotate_warm_result(&mut result, false, None, None, Some("godot_args"), false);
        assert!(result.ok);
        assert!(result.failures.is_empty());
    }

    #[test]
    fn ran_warm_never_fails_strict() {
        let mut result = cmd(true, &[], &[]);
        annotate_warm_result(
            &mut result,
            true,
            Some(WARM_MODE_HEADLESS),
            Some(WARM_AUDIO_DUMMY),
            None,
            true,
        );
        assert!(result.ok);
        assert!(result.failures.is_empty());
        assert_eq!(result.data["warm"]["ran_warm"], json!(true));
        assert_eq!(result.data["warm"]["mode"], json!("headless"));
        assert_eq!(result.data["warm"]["audio"], json!("dummy"));
    }

    #[test]
    fn session_start_failure_result_fails_hard_and_keeps_the_cause() {
        let cause = FailureDetail {
            kind: "artifact_write_failed".to_string(),
            message: Some("File exists (os error 17)".to_string()),
            ..FailureDetail::default()
        };
        let result = warm_session_start_failed_result("scenario bundle run", &cause);
        assert!(!result.ok);
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.failures.len(), 1);
        assert_eq!(result.failures[0].kind, "warm_session_start_failed");
        assert_eq!(result.data["warm"]["session_started"], json!(false));
        assert_eq!(result.data["warm"]["mode"], json!("session_start_failed"));
        // The startup failure survives on BOTH surfaces instead of being dropped.
        assert_eq!(
            result.failures[0].details["cause"]["type"],
            json!("artifact_write_failed")
        );
        assert_eq!(
            result.data["warm"]["start_failure"]["type"],
            json!("artifact_write_failed")
        );
    }

    #[test]
    fn warm_timing_nests_under_warm_annotation() {
        let mut result = cmd(true, &[], &[]);
        annotate_warm_result(
            &mut result,
            true,
            Some(WARM_MODE_HEADLESS),
            Some(WARM_AUDIO_DUMMY),
            None,
            false,
        );
        let timing = WarmTiming {
            artifact_prep_ms: 1.5,
            dispatch_ms: 900.0,
            engine_ms: Some(850.0),
            finalize_ms: 40.0,
        };
        insert_warm_timing(&mut result, &timing, 950.0, 12.0);
        assert_eq!(result.data["warm"]["timing"]["artifact_prep_ms"], json!(1.5));
        assert_eq!(result.data["warm"]["timing"]["dispatch_ms"], json!(900.0));
        assert_eq!(result.data["warm"]["timing"]["engine_ms"], json!(850.0));
        assert_eq!(result.data["warm"]["timing"]["finalize_ms"], json!(40.0));
        assert_eq!(result.data["warm"]["timing"]["lane_held_ms"], json!(950.0));
        assert_eq!(result.data["warm"]["timing"]["acquire_wait_ms"], json!(12.0));
    }

    #[test]
    fn warm_timing_without_warm_annotation_is_a_no_op() {
        let mut result = cmd(true, &[], &[]);
        insert_warm_timing(&mut result, &WarmTiming::default(), 0.0, 0.0);
        assert!(result.data.get("warm").is_none());
    }

    #[test]
    fn warm_annotation_round_trips_through_extraction() {
        let mut warm_leg = cmd(true, &[], &[]);
        annotate_warm_result(&mut warm_leg, false, None, None, Some("godot_args"), false);
        let (ran_warm, mode, audio, reason) = warm_annotation_of(&warm_leg);
        assert!(!ran_warm);
        assert_eq!(mode, None);
        assert_eq!(audio, None);
        assert_eq!(reason.as_deref(), Some("godot_args"));

        let mut ran = cmd(true, &[], &[]);
        annotate_warm_result(
            &mut ran,
            true,
            Some(WARM_MODE_WINDOWED),
            Some(WARM_AUDIO_REAL),
            None,
            false,
        );
        let (ran_warm, mode, audio, reason) = warm_annotation_of(&ran);
        assert!(ran_warm);
        assert_eq!(mode.as_deref(), Some("windowed"));
        assert_eq!(audio.as_deref(), Some("real"));
        assert_eq!(reason, None);
    }

    #[test]
    fn repersist_writes_annotation_to_result_path() {
        let dir = std::env::temp_dir().join(format!(
            "godot-qa-warm-repersist-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let result_path = dir.join("result.json");
        std::fs::write(&result_path, "{}").expect("seed result.json");

        let mut result = cmd(true, &[], &[]);
        result.data.insert(
            "artifacts".to_string(),
            json!({"result_path": result_path.to_string_lossy()}),
        );
        annotate_warm_result(&mut result, false, None, None, Some("godot_args"), true);
        repersist_annotated_result(&result);

        let persisted: Value =
            serde_json::from_str(&std::fs::read_to_string(&result_path).expect("read result.json"))
                .expect("parse result.json");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(persisted["data"]["warm"]["reason"], json!("godot_args"));
        assert_eq!(persisted["ok"], json!(false));
        assert_eq!(
            persisted["failures"][0]["type"],
            json!("warm_fallback_forced_cold")
        );
    }

    #[test]
    fn repersist_without_result_path_is_a_no_op() {
        let mut result = cmd(true, &[], &[]);
        annotate_warm_result(
            &mut result,
            true,
            Some(WARM_MODE_HEADLESS),
            Some(WARM_AUDIO_DUMMY),
            None,
            false,
        );
        // Must not panic or error when the result has no artifacts.
        repersist_annotated_result(&result);
    }
