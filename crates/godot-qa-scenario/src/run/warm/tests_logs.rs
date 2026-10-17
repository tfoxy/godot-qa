    // Warm engine-log slicing and session-noise tests. Included into
    // `mod warm_tests` in `run/warm.rs`.

    #[test]
    fn slice_scenario_log_takes_only_the_window_for_the_run_id() {
        let log = format!(
            "ERROR: before the window\n{}\nWARNING: inside\nERROR: inside too\n{}\nERROR: after \
             the window\n{}\n",
            begin_marker(MARKER_RUN),
            end_marker(MARKER_RUN),
            begin_marker(OTHER_RUN),
        );
        let range = slice_scenario_log(log.as_bytes(), MARKER_RUN).expect("window");
        let sliced = String::from_utf8_lossy(&log.as_bytes()[range]).into_owned();

        assert!(sliced.starts_with(&begin_marker(MARKER_RUN)));
        assert!(sliced.ends_with(&format!("{}\n", end_marker(MARKER_RUN))));
        assert!(sliced.contains("WARNING: inside"));
        assert!(sliced.contains("ERROR: inside too"));
        assert!(!sliced.contains("before the window"));
        assert!(!sliced.contains("after the window"));
    }

    #[test]
    fn slice_scenario_log_ignores_markers_for_other_run_ids() {
        let log = format!(
            "{}\nERROR: not ours\n{}\n{}\nERROR: ours\n{}\n",
            begin_marker(OTHER_RUN),
            end_marker(OTHER_RUN),
            begin_marker(MARKER_RUN),
            end_marker(MARKER_RUN),
        );
        let range = slice_scenario_log(log.as_bytes(), MARKER_RUN).expect("window");
        let sliced = String::from_utf8_lossy(&log.as_bytes()[range]).into_owned();

        assert!(sliced.contains("ERROR: ours"));
        assert!(!sliced.contains("ERROR: not ours"));
    }

    /// An unterminated window must NOT fall back to "slice to EOF" — the session
    /// is released before finalize, so EOF may already hold the next scenario's
    /// output and slicing to it would misattribute that scenario's errors here.
    #[test]
    fn slice_scenario_log_without_an_end_marker_is_none() {
        let log = format!(
            "{}\nERROR: unterminated\n{}\n",
            begin_marker(MARKER_RUN),
            begin_marker(OTHER_RUN),
        );
        assert!(slice_scenario_log(log.as_bytes(), MARKER_RUN).is_none());
    }

    #[test]
    fn slice_scenario_log_without_a_begin_marker_is_none() {
        let log = format!("ERROR: orphan\n{}\n", end_marker(MARKER_RUN));
        assert!(slice_scenario_log(log.as_bytes(), MARKER_RUN).is_none());
    }

    #[test]
    fn slice_scenario_log_matches_crlf_marker_lines() {
        let log = format!(
            "{}\r\nERROR: crlf inside\r\n{}\r\nERROR: outside\r\n",
            begin_marker(MARKER_RUN),
            end_marker(MARKER_RUN),
        );
        let range = slice_scenario_log(log.as_bytes(), MARKER_RUN).expect("window");
        let sliced = String::from_utf8_lossy(&log.as_bytes()[range]).into_owned();

        assert!(sliced.contains("ERROR: crlf inside"));
        assert!(!sliced.contains("ERROR: outside"));
    }

    /// End to end over the pure pieces: a shared session log with fallout on both
    /// sides of the window is materialized into the run's own logs.txt, and the
    /// SHARED cold log scanner then reports exactly the in-window error.
    #[test]
    fn materialize_warm_scenario_logs_then_runtime_log_failures_flags_error() {
        let dir = warm_temp_dir("materialize-warm-logs");
        let run_root = dir.join("run");
        std::fs::create_dir_all(&run_root).expect("create run root");
        let session_logs = dir.join("session-logs.txt");
        // A prefix that predates the anchor, then the window, then fallout the
        // next scenario on this lane will own.
        let prefix = "WARNING: earlier scenario noise\n";
        std::fs::write(
            &session_logs,
            format!(
                "{prefix}{}\nERROR: Lambda capture at index 0 was freed\n{}\nWARNING: after the \
                 window\n",
                begin_marker(MARKER_RUN),
                end_marker(MARKER_RUN),
            ),
        )
        .expect("write session log");

        let artifacts = warm_test_artifacts(&run_root, MARKER_RUN);
        let anchor = Some(WarmSessionLogAnchor {
            path: session_logs.clone(),
            start_offset: prefix.len() as u64,
        });

        let slice = materialize_warm_scenario_logs(&artifacts, &anchor);
        let (failures, ignored) = runtime_log_failures(
            "scenario run",
            &artifacts.logs_path,
            &Map::from_iter([("logs_path".to_string(), json!(artifacts.logs_path))]),
            &[],
        );
        let materialized =
            std::fs::read_to_string(&artifacts.logs_path).expect("materialized logs.txt");
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(slice.status, WARM_LOG_SLICE_SLICED);
        assert_eq!(slice.begin_offset, Some(prefix.len() as u64));
        assert_eq!(slice.end_offset, Some(materialized.len() as u64 + prefix.len() as u64));
        assert!(!materialized.contains("earlier scenario noise"));
        assert!(!materialized.contains("after the window"));
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert_eq!(ignored, 0);
        assert_eq!(failures[0].kind, "godot_log_error");
        assert_eq!(
            failures[0].actual,
            Some(json!("ERROR: Lambda capture at index 0 was freed"))
        );
    }

    /// An addon older than the markers: no logs.txt is created, so `data.logs`
    /// stays exactly as `missing` as it was before warm log parity — and the
    /// bundle raises the warning ONCE no matter how many scenarios hit it.
    #[test]
    fn missing_markers_write_no_logs_and_warn_once_per_bundle() {
        let dir = warm_temp_dir("warm-markers-missing");
        let run_root = dir.join("run");
        std::fs::create_dir_all(&run_root).expect("create run root");
        let session_logs = dir.join("session-logs.txt");
        std::fs::write(&session_logs, "ERROR: unattributed\n").expect("write session log");

        let artifacts = warm_test_artifacts(&run_root, MARKER_RUN);
        let anchor = Some(WarmSessionLogAnchor {
            path: session_logs.clone(),
            start_offset: 0,
        });

        let slice = materialize_warm_scenario_logs(&artifacts, &anchor);
        let logs_exists = artifacts.logs_path.exists();

        let pool = WarmSessionPool::new(fake_handle("session-a", 60), 1, None);
        pool.note_log_markers_missing("session-a", &slice.source);
        pool.note_log_markers_missing("session-a", &slice.source);
        let warnings = pool.log_warnings_snapshot();

        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(slice.status, WARM_LOG_SLICE_MARKERS_MISSING);
        assert!(!logs_exists, "no logs.txt may be written without markers");
        assert_eq!(
            log_status_summary_json(&artifacts.logs_path, &[], 0, 0)["status"],
            json!("missing")
        );
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, "warm_log_markers_missing");
    }

    #[test]
    fn a_session_without_a_readable_log_is_unavailable() {
        let dir = warm_temp_dir("warm-log-unavailable");
        let run_root = dir.join("run");
        std::fs::create_dir_all(&run_root).expect("create run root");
        let artifacts = warm_test_artifacts(&run_root, MARKER_RUN);

        let slice = materialize_warm_scenario_logs(&artifacts, &None);
        let annotation = slice.to_json("session-a");

        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(slice.status, WARM_LOG_SLICE_UNAVAILABLE);
        assert!(slice.window().is_none());
        assert_eq!(annotation["source"], Value::Null);
        assert_eq!(annotation["session_id"], json!("session-a"));
        assert_eq!(annotation["begin_offset"], Value::Null);
    }

    /// An Android session's engine output never reaches a host path, so it must
    /// not be anchored at all (android.rs bypasses the warm pool anyway).
    #[test]
    fn only_desktop_sessions_are_log_anchored() {
        let dir = warm_temp_dir("warm-log-anchor-target");
        std::fs::write(dir.join("logs.txt"), "boot\n").expect("write session log");

        let mut handle = fake_handle("session-a", 60);
        handle.record.artifact_dir = dir.to_string_lossy().into_owned();
        let desktop = warm_session_log_anchor(&handle);
        handle.record.target = godot_qa_runtime::SessionTarget::Android;
        let android = warm_session_log_anchor(&handle);

        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(desktop.expect("desktop anchor").start_offset, 5);
        assert!(android.is_none());
    }

    #[test]
    fn engine_log_failures_carry_the_session_log_window() {
        let mut result = cmd(false, &["godot_log_error", "focused_mismatch"], &[]);
        assert!(annotate_warm_log_failures(
            &mut result,
            "session-a",
            Some(&(10, 240))
        ));

        assert_eq!(
            result.failures[0].details["warm_log_window"],
            json!({"session_id": "session-a", "begin_offset": 10, "end_offset": 240})
        );
        assert!(!result.failures[1].details.contains_key("warm_log_window"));
        // Nothing to annotate when the window is unknown (markers_missing).
        let mut unsliced = cmd(false, &["godot_log_error"], &[]);
        assert!(!annotate_warm_log_failures(&mut unsliced, "session-a", None));
    }

    /// annotate_warm_result rebuilds data.warm wholesale, so the log evidence has
    /// to survive that round trip or the bundle summary would silently disagree
    /// with the result.json on disk.
    #[test]
    fn warm_log_annotation_survives_the_strict_annotation_rebuild() {
        let mut result = cmd(true, &[], &[]);
        result.data.insert(
            "warm".to_string(),
            json!({"ran_warm": true, "session_id": "session-a", "log_slice": {"status": "sliced"}}),
        );

        let kept = take_warm_log_annotation(&result);
        annotate_warm_result(
            &mut result,
            true,
            Some(WARM_MODE_HEADLESS),
            Some(WARM_AUDIO_DUMMY),
            None,
            true,
        );
        restore_warm_log_annotation(&mut result, kept);

        assert_eq!(result.data["warm"]["mode"], json!(WARM_MODE_HEADLESS));
        assert_eq!(result.data["warm"]["session_id"], json!("session-a"));
        assert_eq!(result.data["warm"]["log_slice"]["status"], json!("sliced"));
    }

    /// The consumer finding this exists for: the engine error that fires at
    /// session SHUTDOWN, after the last window closed, plus start-time and
    /// between-scenario output. None of it reaches any run's logs.txt, so the
    /// bundle-level scan is the only thing that can report it.
    #[test]
    fn session_log_noise_lines_reports_gap_lines_before_between_and_after_windows() {
        let log = format!(
            "ERROR: before the first window\n{}\nERROR: inside one\n{}\nWARNING: between \
             windows\n{}\nERROR: inside two\n{}\nERROR: Lambda capture at index 0 was freed. \
             Passed \"null\" instead.\n",
            begin_marker(MARKER_RUN),
            end_marker(MARKER_RUN),
            begin_marker(OTHER_RUN),
            end_marker(OTHER_RUN),
        );

        let noise = session_log_noise_lines(log.as_bytes(), &[]);

        assert_eq!(
            noise,
            vec![
                "ERROR: before the first window".to_string(),
                "WARNING: between windows".to_string(),
                "ERROR: Lambda capture at index 0 was freed. Passed \"null\" instead.".to_string(),
            ]
        );
    }

    /// The other half of the contract: anything inside a window already belongs
    /// to a run's logs.txt, so counting it here would double-report it.
    #[test]
    fn session_log_noise_lines_excludes_everything_inside_a_window() {
        let log = format!(
            "{}\nERROR: attributed to the run\nWARNING: also attributed\n{}\n",
            begin_marker(MARKER_RUN),
            end_marker(MARKER_RUN),
        );

        assert!(session_log_noise_lines(log.as_bytes(), &[]).is_empty());
    }

    /// Gap lines go through the SAME ignore predicates the cold scan uses, so the
    /// two never disagree about what counts as engine noise. `resources still in
    /// use at exit` is filtered locally instead (see is_warm_session_exit_noise).
    #[test]
    fn session_log_noise_lines_applies_the_cold_ignore_list_and_exit_noise() {
        let log = "WARNING: All audio drivers failed, falling back to the dummy driver.\n\
             ERROR: Required extension VK_KHR_surface not found.\n\
             WARNING: ObjectDB instances leaked at exit (run with --verbose for details).\n\
             ERROR: 3 resources still in use at exit (run with --verbose for details).\n\
             ERROR: a genuine fault\n";

        assert_eq!(
            session_log_noise_lines(log.as_bytes(), &[]),
            vec!["ERROR: a genuine fault".to_string()]
        );
    }

    /// Parity with cold: a project `logs.ignore` pattern silences a gap line here
    /// exactly as it silences the same line inside a scenario window, so a
    /// consumer configures the noise once rather than once per scan.
    #[test]
    fn session_log_noise_lines_applies_the_project_ignore_list() {
        let log = "ERROR: Cannot call method 'emit' on a previously freed instance.\n\
             WARNING: Shader compilation took 900ms.\n\
             ERROR: a genuine fault\n";
        let patterns = vec![
            "previously freed instance".to_string(),
            "Shader compilation took".to_string(),
        ];

        assert_eq!(
            session_log_noise_lines(log.as_bytes(), &patterns),
            vec!["ERROR: a genuine fault".to_string()]
        );
        // Without the project list the same log is all noise.
        assert_eq!(session_log_noise_lines(log.as_bytes(), &[]).len(), 3);
    }

    #[test]
    fn session_log_noise_lines_handles_crlf_markers_and_lines() {
        let log = format!(
            "{}\r\nERROR: inside\r\n{}\r\nERROR: outside\r\n",
            begin_marker(MARKER_RUN),
            end_marker(MARKER_RUN),
        );

        assert_eq!(
            session_log_noise_lines(log.as_bytes(), &[]),
            vec!["ERROR: outside".to_string()]
        );
    }

    /// A session killed mid-scenario leaves an unterminated window. That tail is
    /// attributable to the scenario, so it must NOT be reported as gap noise.
    #[test]
    fn session_log_noise_lines_stop_at_an_unterminated_window() {
        let log = format!(
            "ERROR: before\n{}\nERROR: after the abandoned begin\n",
            begin_marker(MARKER_RUN),
        );

        assert_eq!(
            session_log_noise_lines(log.as_bytes(), &[]),
            vec!["ERROR: before".to_string()]
        );
    }

    #[test]
    fn scan_warm_session_log_noise_groups_one_message_across_sessions() {
        let dir = warm_temp_dir("session-log-noise-scan");
        let lambda = "ERROR: Lambda capture at index 0 was freed. Passed \"null\" instead.";
        let sessions: Vec<WarmPooledSessionRef> = ["session-a", "session-b", "session-c"]
            .iter()
            .enumerate()
            .map(|(index, id)| {
                let logs_path = dir.join(format!("{id}.txt"));
                let body = if index == 2 {
                    format!("{}\n{}\n", begin_marker(MARKER_RUN), end_marker(MARKER_RUN))
                } else {
                    format!("{lambda}\nWARNING: ObjectDB instances leaked at exit.\n")
                };
                std::fs::write(&logs_path, body).expect("write session log");
                WarmPooledSessionRef {
                    session_id: (*id).to_string(),
                    logs_path,
                }
            })
            .collect();

        let noise = scan_warm_session_log_noise(&sessions, &[]);
        let warning = warm_session_log_noise_warning(&noise).expect("warning");
        let json = noise.to_json();

        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(noise.sessions_scanned, 3);
        assert_eq!(noise.line_count, 2);
        assert_eq!(json["phase"], json!(WARM_SESSION_LOG_NOISE_PHASE));
        assert_eq!(json["by_message"][0]["line"], json!(lambda));
        assert_eq!(json["by_message"][0]["count"], json!(2));
        assert_eq!(
            json["by_message"][0]["sessions"],
            json!(["session-a", "session-b"])
        );
        assert_eq!(warning.kind, "warm_session_log_noise");
        assert_eq!(warning.actual, Some(json!(lambda)));
        assert!(
            warning
                .message
                .as_deref()
                .unwrap_or_default()
                .starts_with("2 engine log lines were emitted")
        );
        assert_eq!(warning.details["by_message"], json["by_message"]);
    }

    /// The bundle-level path: the post-run hook scans every session the pool
    /// started (including one it discarded, whose handle is long gone) and lands
    /// the tally on `data.warm_pool`, from where copy_warm_pool_summary lifts it
    /// onto `data.warm`. A clean pool must add no warning at all.
    #[test]
    fn warm_pool_post_run_reports_session_log_noise_on_the_bundle() {
        let dir = warm_temp_dir("post-run-session-log-noise");
        let noisy_dir = dir.join("noisy");
        let clean_dir = dir.join("clean");
        std::fs::create_dir_all(&noisy_dir).expect("create noisy dir");
        std::fs::create_dir_all(&clean_dir).expect("create clean dir");
        std::fs::write(
            noisy_dir.join("logs.txt"),
            format!(
                "{}\n{}\nERROR: Lambda capture at index 0 was freed\nWARNING: ObjectDB instances \
                 leaked at exit.\nERROR: 2 resources still in use at exit.\n",
                begin_marker(MARKER_RUN),
                end_marker(MARKER_RUN),
            ),
        )
        .expect("write noisy session log");
        std::fs::write(clean_dir.join("logs.txt"), "boot\n").expect("write clean session log");

        let mut seed = fake_handle("session-seed", 60);
        seed.record.artifact_dir = clean_dir.to_string_lossy().into_owned();
        let pool = WarmSessionPool::new(seed, 2, None);
        // A session that was started and later discarded still has a log to scan.
        let mut discarded = fake_handle("session-discarded", 60);
        discarded.record.artifact_dir = noisy_dir.to_string_lossy().into_owned();
        pool.note_session_started(&discarded.record);

        let post_run = warm_pool_post_run(&discard_test_config(), &pool);
        let mut warm = warm_status_json(true, "warm");
        copy_warm_pool_summary(&mut warm, post_run.data.get("warm_pool"));

        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(warm["session_log_noise"]["sessions_scanned"], json!(2));
        assert_eq!(warm["session_log_noise"]["line_count"], json!(1));
        assert_eq!(
            warm["session_log_noise"]["by_message"][0]["sessions"],
            json!(["session-discarded"])
        );
        let kinds: Vec<&str> = post_run
            .warnings
            .iter()
            .map(|warning| warning.kind.as_str())
            .collect();
        assert_eq!(kinds, vec!["warm_session_log_noise"]);
    }

    /// The bundle-level path honours the project ignore list too: the same
    /// `logs.ignore` patterns that keep a line out of a cold run's failures keep
    /// it out of `data.warm.session_log_noise`, so the noise warning is not the
    /// one place a consumer still has to see the line it deliberately silenced.
    #[test]
    fn warm_pool_post_run_honours_the_project_log_ignore_list() {
        let dir = warm_temp_dir("post-run-session-log-project-ignore");
        std::fs::write(
            dir.join("logs.txt"),
            "ERROR: Cannot call method 'emit' on a previously freed instance.\nERROR: a genuine \
             fault\n",
        )
        .expect("write session log");
        let mut seed = fake_handle("session-seed", 60);
        seed.record.artifact_dir = dir.to_string_lossy().into_owned();
        let pool = WarmSessionPool::new(seed, 1, None);
        let mut config = discard_test_config();
        config.log_ignore_patterns = vec!["previously freed instance".to_string()];

        let post_run = warm_pool_post_run(&config, &pool);
        let noise = post_run.data["warm_pool"]["session_log_noise"].clone();

        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(noise["line_count"], json!(1));
        assert_eq!(
            noise["by_message"][0]["line"],
            json!("ERROR: a genuine fault")
        );
    }

    #[test]
    fn a_pool_with_no_gap_noise_adds_no_warning() {
        let dir = warm_temp_dir("post-run-session-log-clean");
        std::fs::write(dir.join("logs.txt"), "boot\nWARNING: ObjectDB instances leaked at exit.\n")
            .expect("write session log");
        let mut seed = fake_handle("session-seed", 60);
        seed.record.artifact_dir = dir.to_string_lossy().into_owned();
        let pool = WarmSessionPool::new(seed, 1, None);

        let post_run = warm_pool_post_run(&discard_test_config(), &pool);
        let noise = post_run.data["warm_pool"]["session_log_noise"].clone();

        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(noise["line_count"], json!(0));
        assert_eq!(noise["by_message"], json!([]));
        assert!(post_run.warnings.is_empty());
    }
