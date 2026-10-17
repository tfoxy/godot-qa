    // Warm session pool tests, plus the warm-skipped-below-parallelism path.
    // Included into `mod warm_tests` in `run/warm.rs`.

    #[test]
    fn warm_pool_max_follows_jobs() {
        // --jobs 1 collapses the pool to a single serial session.
        assert_eq!(warm_pool_max(&opts_with_jobs(1)), 1);
        // An explicit job count is used verbatim as the pool ceiling.
        assert_eq!(warm_pool_max(&opts_with_jobs(3)), 3);
        // jobs = 0 -> available parallelism (whatever the host reports, >= 1).
        assert!(warm_pool_max(&opts_with_jobs(0)) >= 1);
    }

    #[test]
    fn pool_new_seeds_idle_and_records_session_fps() {
        let pool = WarmSessionPool::new(fake_handle("seed", 30), 4, Some(Duration::from_secs(1)));
        assert_eq!(pool.session_fps, 30);
        assert_eq!(pool.max, 4);
        // The eager seed lands in the common audio-off (Dummy) lane.
        let inner = pool.lock_headless(false);
        assert_eq!(inner.idle.len(), 1);
        assert_eq!(inner.started, 1);
        assert_eq!(inner.consecutive_start_failures, 0);
        drop(inner);
        // The rare audio-on (real) lane starts empty.
        let real = pool.lock_headless(true);
        assert!(real.idle.is_empty());
        assert_eq!(real.started, 0);
    }

    #[test]
    fn acquire_wait_budget_tracks_the_seed_boot_within_bounds() {
        // The measured boot is the budget...
        assert_eq!(
            warm_acquire_wait(Some(Duration::from_millis(1200))),
            Duration::from_millis(1200)
        );
        // ...clamped at both ends so a freak measurement cannot busy-wait or stall.
        assert_eq!(
            warm_acquire_wait(Some(Duration::from_millis(5))),
            WARM_ACQUIRE_WAIT_MIN
        );
        assert_eq!(
            warm_acquire_wait(Some(Duration::from_secs(60))),
            WARM_ACQUIRE_WAIT_MAX
        );
        // An unmeasured pool falls back to a flat budget.
        assert_eq!(warm_acquire_wait(None), WARM_ACQUIRE_WAIT_FALLBACK);
    }

    #[test]
    fn acquire_decision_reuses_idle_before_starting() {
        let inner = HeadlessPoolInner {
            idle: vec![fake_handle("a", 60)],
            started: 1,
            consecutive_start_failures: 0,
        };
        assert_eq!(
            headless_acquire_decision(&inner, 4),
            HeadlessAcquire::ReuseIdle
        );
    }

    #[test]
    fn acquire_decision_starts_new_when_below_max_and_none_idle() {
        let inner = HeadlessPoolInner {
            idle: Vec::new(),
            started: 2,
            consecutive_start_failures: 0,
        };
        assert_eq!(
            headless_acquire_decision(&inner, 4),
            HeadlessAcquire::StartNew
        );
    }

    #[test]
    fn acquire_decision_waits_at_capacity_instead_of_going_cold() {
        // At capacity with none idle is TRANSIENT: peers are mid-scenario and will
        // check their sessions back in, which is far cheaper than a cold boot.
        let at_cap = HeadlessPoolInner {
            idle: Vec::new(),
            started: 4,
            consecutive_start_failures: 0,
        };
        assert_eq!(headless_acquire_decision(&at_cap, 4), HeadlessAcquire::Wait);
    }

    #[test]
    fn acquire_decision_waits_while_the_budget_is_spent_but_sessions_live() {
        // Starts are exhausted, but live sessions exist -> wait for one of them.
        let spent = HeadlessPoolInner {
            idle: Vec::new(),
            started: 2,
            consecutive_start_failures: WARM_START_FAILURE_BUDGET,
        };
        assert_eq!(headless_acquire_decision(&spent, 4), HeadlessAcquire::Wait);
    }

    #[test]
    fn acquire_decision_unavailable_only_when_nothing_can_come_free() {
        // Budget spent AND no live session: nothing will ever be handed back, so
        // this is the one state that sends the scenario cold.
        let dead = HeadlessPoolInner {
            idle: Vec::new(),
            started: 0,
            consecutive_start_failures: WARM_START_FAILURE_BUDGET,
        };
        assert_eq!(
            headless_acquire_decision(&dead, 4),
            HeadlessAcquire::Unavailable
        );
        // One failure short of the budget still starts: a single transient failure
        // must not condemn the lane (the old `failed: bool` did exactly that).
        let one_failure = HeadlessPoolInner {
            idle: Vec::new(),
            started: 0,
            consecutive_start_failures: 1,
        };
        assert_eq!(
            headless_acquire_decision(&one_failure, 4),
            HeadlessAcquire::StartNew
        );
    }

    #[test]
    fn acquire_records_each_start_failure_and_gives_up_at_the_budget() {
        let pool = test_pool(4);
        // Drain the seed so the audio-off lane must boot.
        let seed = pool.lock_headless(false).idle.pop().expect("seed idle");
        let attempts = std::sync::atomic::AtomicUsize::new(0);
        let started = Instant::now();
        let acquired = pool.acquire_headless_with(
            false,
            // A tiny deadline: once the budget is spent the call must not linger.
            Instant::now() + Duration::from_millis(50),
            || {
                attempts.fetch_add(1, Ordering::Relaxed);
                Err(start_failure("missing_godot"))
            },
        );
        assert!(acquired.is_none());
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "a spent budget must fall back promptly, took {:?}",
            started.elapsed()
        );
        assert_eq!(
            attempts.load(Ordering::Relaxed),
            WARM_START_FAILURE_BUDGET as usize
        );
        // The reservations were released again, so the lane is not leaked...
        let inner = pool.lock_headless(false);
        assert_eq!(inner.started, 1, "only the checked-out seed remains");
        assert_eq!(inner.consecutive_start_failures, WARM_START_FAILURE_BUDGET);
        drop(inner);
        // ...and every failure is kept as evidence, tagged with lane + attempt.
        let failures = pool.start_failures_snapshot();
        assert_eq!(failures.len(), WARM_START_FAILURE_BUDGET as usize);
        assert_eq!(failures[0]["type"], json!("missing_godot"));
        assert_eq!(failures[0]["lane"], json!("headless_dummy"));
        assert_eq!(failures[0]["attempt"], json!(1));
        assert_eq!(failures[1]["attempt"], json!(2));
        pool.release_headless(seed, false);
    }

    #[test]
    fn a_successful_start_resets_the_failure_budget() {
        let pool = test_pool(4);
        let seed = pool.lock_headless(false).idle.pop().expect("seed idle");
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let acquired = pool.acquire_headless_with(
            false,
            Instant::now() + Duration::from_millis(50),
            || {
                if calls.fetch_add(1, Ordering::Relaxed) == 0 {
                    Err(start_failure("session_probe_failed"))
                } else {
                    Ok(fake_handle("second", 60))
                }
            },
        );
        let (handle, _waited) = acquired.expect("the retry after one failure succeeds");
        assert_eq!(handle.record.session_id, "second");
        let inner = pool.lock_headless(false);
        // Seed + the new session are both live, and the lane is clean again.
        assert_eq!(inner.started, 2);
        assert_eq!(inner.consecutive_start_failures, 0);
        drop(inner);
        assert_eq!(pool.start_failures_snapshot().len(), 1);
        assert_eq!(pool.sessions_started.load(Ordering::Relaxed), 2);
        pool.release_headless(seed, false);
        pool.release_headless(handle, false);
    }

    #[test]
    fn a_waiter_is_woken_by_a_release_instead_of_going_cold() {
        // started = 1 (the seed is checked out), nothing idle, budget spent: the
        // only way to get a session is for the peer holding it to release.
        let pool = Arc::new(test_pool(1));
        let seed = pool.lock_headless(false).idle.pop().expect("seed idle");
        {
            let mut inner = pool.lock_headless(false);
            inner.consecutive_start_failures = WARM_START_FAILURE_BUDGET;
        }
        let releaser_pool = Arc::clone(&pool);
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            releaser_pool.release_headless(seed, false);
        });
        let (handle, waited) = pool
            .acquire_headless_with(false, Instant::now() + Duration::from_secs(5), || {
                panic!("a spent budget must never boot")
            })
            .expect("the released session must reach the waiter");
        releaser.join().expect("releaser thread");
        assert_eq!(handle.record.session_id, "seed");
        assert!(
            waited >= Duration::from_millis(10),
            "the waiter really blocked, waited {waited:?}"
        );
    }

    #[test]
    fn a_waiter_falls_back_to_cold_after_the_deadline() {
        let pool = test_pool(1);
        let seed = pool.lock_headless(false).idle.pop().expect("seed idle");
        {
            let mut inner = pool.lock_headless(false);
            inner.consecutive_start_failures = WARM_START_FAILURE_BUDGET;
        }
        let started = Instant::now();
        let acquired = pool
            .acquire_headless_with(false, Instant::now() + Duration::from_millis(80), || {
                panic!("a spent budget must never boot")
            });
        assert!(acquired.is_none(), "the wait must be BOUNDED, not forever");
        assert!(started.elapsed() >= Duration::from_millis(60));
        assert!(started.elapsed() < Duration::from_secs(3));
        pool.release_headless(seed, false);
    }

    #[test]
    fn discard_wakes_a_waiter_and_leaves_the_boot_budget_alone() {
        // The last live session is discarded while a worker waits: the waiter must
        // be woken (not left until the deadline) and, because a discard is not a
        // failed boot, it must be free to start a replacement.
        let pool = Arc::new(test_pool(1));
        let seed = pool.lock_headless(false).idle.pop().expect("seed idle");
        let discarder_pool = Arc::clone(&pool);
        let discarder = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            // pid 0 -> stop_pooled_session returns without waiting on a process.
            discarder_pool.discard_headless(&discard_test_config(), seed, false);
        });
        let booted = std::sync::atomic::AtomicUsize::new(0);
        let (handle, _waited) = pool
            .acquire_headless_with(false, Instant::now() + Duration::from_secs(5), || {
                booted.fetch_add(1, Ordering::Relaxed);
                Ok(fake_handle("replacement", 60))
            })
            .expect("the freed slot must let the waiter boot a replacement");
        discarder.join().expect("discarder thread");
        assert_eq!(handle.record.session_id, "replacement");
        assert_eq!(booted.load(Ordering::Relaxed), 1);
        assert!(pool.start_failures_snapshot().is_empty());
        pool.release_headless(handle, false);
    }

    #[test]
    fn discard_with_no_replacement_leaves_a_waiter_unavailable() {
        let pool = Arc::new(test_pool(1));
        let seed = pool.lock_headless(false).idle.pop().expect("seed idle");
        {
            let mut inner = pool.lock_headless(false);
            inner.consecutive_start_failures = WARM_START_FAILURE_BUDGET;
        }
        let discarder_pool = Arc::clone(&pool);
        let discarder = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            discarder_pool.discard_headless(&discard_test_config(), seed, false);
        });
        // started reaches 0 with the budget spent -> Unavailable, well before the
        // generous deadline.
        let started = Instant::now();
        let acquired = pool
            .acquire_headless_with(false, Instant::now() + Duration::from_secs(10), || {
                panic!("a spent budget must never boot")
            });
        discarder.join().expect("discarder thread");
        assert!(acquired.is_none());
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the waiter must be woken by the discard, took {:?}",
            started.elapsed()
        );
        assert_eq!(pool.lock_headless(false).started, 0);
    }

    #[test]
    fn shutdown_drains_both_lanes_and_the_windowed_slot_once() {
        let pool = test_pool(4);
        pool.release_headless(fake_handle("dummy_b", 60), false);
        pool.release_headless(fake_handle("real_a", 60), true);
        pool.lock_windowed().session = Some(fake_handle("windowed", 60));
        pool.shutdown(&discard_test_config());
        assert!(pool.lock_headless(false).idle.is_empty());
        assert!(pool.lock_headless(true).idle.is_empty());
        assert!(pool.lock_windowed().session.is_none());
        // The duration is recorded for the bundle summary, and a second call is a
        // no-op (the post-run hook owns shutdown, nothing may stop sessions twice).
        let summary = pool.summary_json();
        assert!(summary["shutdown_ms"].is_number());
        pool.shutdown(&discard_test_config());
        assert!(summary["seed_boot_ms"].is_number());
        assert_eq!(summary["acquire_wait_ms_budget"], json!(400.0));
    }

    #[test]
    fn start_failure_warnings_carry_the_cause_and_never_fail_the_bundle() {
        let pool = test_pool(4);
        let mut detail = start_failure("artifact_write_failed");
        detail
            .details
            .insert("session_artifact_dir".to_string(), json!("/tmp/live/abc"));
        pool.record_start_failure("headless_dummy", 1, &detail);
        let warning = warm_session_start_warning(&pool.start_failures_snapshot()[0]);
        assert_eq!(warning.kind, "warm_session_start_failed");
        assert_eq!(warning.command.as_deref(), Some("scenario bundle run"));
        assert_eq!(warning.path.as_deref(), Some("/tmp/live/abc"));
        assert_eq!(warning.actual, Some(json!("artifact_write_failed")));
        assert_eq!(warning.details["cause"]["lane"], json!("headless_dummy"));
    }

    #[test]
    fn warm_pool_fields_are_copied_onto_the_warm_annotation() {
        // Consumers read data.warm, so the pool's headline numbers must be there
        // and not only under data.warm_pool.
        let mut warm = warm_status_json(true, "warm");
        let warm_pool = json!({
            "sessions_started": 3,
            "session_start_failures": [{"type": "missing_godot"}],
            "transport_failures": [],
            "shutdown_ms": 12.0,
        });
        copy_warm_pool_summary(&mut warm, Some(&warm_pool));
        assert_eq!(warm["sessions_started"], json!(3));
        assert_eq!(warm["session_start_failures"][0]["type"], json!("missing_godot"));
        assert_eq!(warm["transport_failures"], json!([]));
        // Only the three consumer-facing fields are mirrored.
        assert!(warm.get("shutdown_ms").is_none());
    }

    #[test]
    fn release_returns_session_to_idle_free_list() {
        let pool = test_pool(3);
        // Simulate a checkout (idle now empty, still 1 started) then a release,
        // all in the common audio-off lane.
        let handle = { pool.lock_headless(false).idle.pop().expect("seed idle") };
        assert!(pool.lock_headless(false).idle.is_empty());
        pool.release_headless(handle, false);
        assert_eq!(pool.lock_headless(false).idle.len(), 1);
        // Distributing three sessions across the free list keeps them addressable
        // by distinct session-ids (no singleton collision).
        pool.release_headless(fake_handle("b", 60), false);
        pool.release_headless(fake_handle("c", 60), false);
        let ids: Vec<String> = pool
            .lock_headless(false)
            .idle
            .iter()
            .map(|handle| handle.record.session_id.clone())
            .collect();
        assert_eq!(ids, vec!["seed".to_string(), "b".to_string(), "c".to_string()]);
    }

    #[test]
    fn skip_warm_when_lanes_meet_or_exceed_scenario_count() {
        // count == lanes: every scenario gets its own lane, no session reuse -> cold.
        assert!(skip_warm_below_parallelism(4, 4));
        // fewer scenarios than lanes -> still no reuse -> cold.
        assert!(skip_warm_below_parallelism(8, 1));
        // a single-scenario bundle always runs cold.
        assert!(skip_warm_below_parallelism(1, 1));
        // an empty bundle is trivially cold (0 lanes >= 0 count), harmless.
        assert!(skip_warm_below_parallelism(4, 0));
    }

    #[test]
    fn keep_warm_when_scenarios_exceed_lanes() {
        // count > lanes: at least one session runs a second scenario (reuse) -> warm.
        assert!(!skip_warm_below_parallelism(4, 5));
        assert!(!skip_warm_below_parallelism(4, 100));
        // --jobs 1 with more than one scenario: a single session is reused
        // serially across all of them, so warm is NOT skipped.
        assert!(!skip_warm_below_parallelism(1, 3));
    }

    #[test]
    fn multi_bundle_skip_decision_sums_runnable_counts() {
        // Three six-scenario gates on a twelve-lane host: each ALONE would skip warm
        // (6 <= 12), but they share one pool, so eighteen scenarios over twelve lanes
        // do reuse sessions. The decision is the sum, which is the whole point of
        // naming several gates in one invocation.
        assert_eq!(
            multi_bundle_warm_skip_total(12, &[Some(6), Some(6), Some(6)]),
            None
        );
        // Summed count still at or below the lanes -> still no reuse -> still skip,
        // and the total is reported as the evidence for it.
        assert_eq!(
            multi_bundle_warm_skip_total(12, &[Some(4), Some(4), Some(4)]),
            Some(12)
        );
        assert_eq!(
            multi_bundle_warm_skip_total(8, &[Some(2), Some(2)]),
            Some(4)
        );
        // --jobs 1 across two bundles is one session reused serially -> warm.
        assert_eq!(multi_bundle_warm_skip_total(1, &[Some(2), Some(2)]), None);
        // A bundle that cannot be resolved makes the sum unknowable; an unknown sum
        // never justifies skipping, so the normal path runs and reports the real
        // failure there.
        assert_eq!(multi_bundle_warm_skip_total(12, &[Some(4), None]), None);
        // One name behaves exactly like the single-bundle decision.
        assert_eq!(multi_bundle_warm_skip_total(4, &[Some(4)]), Some(4));
        assert_eq!(multi_bundle_warm_skip_total(4, &[Some(5)]), None);
    }

    #[test]
    fn warm_skipped_annotation_marks_cold_run_with_reason() {
        let annotation = warm_skipped_below_parallelism_json(false);
        assert_eq!(annotation["ran_warm"], json!(false));
        assert_eq!(annotation["session_started"], json!(false));
        assert_eq!(annotation["mode"], Value::Null);
        assert_eq!(annotation["reason"], json!("no_reuse_below_parallelism"));
        // Without strict there is no structural check to advertise.
        assert!(annotation.get("strict_structural_check").is_none());
    }

    #[test]
    fn warm_skipped_annotation_records_strict_structural_check() {
        let annotation = warm_skipped_below_parallelism_json(true);
        // The base skip annotation is unchanged...
        assert_eq!(annotation["ran_warm"], json!(false));
        assert_eq!(annotation["session_started"], json!(false));
        assert_eq!(annotation["reason"], json!("no_reuse_below_parallelism"));
        // ...and gains the strict-structural marker so artifacts show strict was
        // enforced despite the parallelism skip.
        assert_eq!(annotation["strict_structural_check"], json!(true));
    }

    #[test]
    fn skip_strict_structural_violation_gets_reason_and_fails_strict() {
        // A structurally-incompatible scenario (godot_args) in a warm-skipped
        // --warm-strict bundle is classified as a cold fallback with a structural
        // reason, which cold_fallback_result -> annotate_warm_result then turns
        // into warm_fallback_forced_cold — exactly the non-skip strict decision.
        let scenario = json!({
            "name": "x", "scene": "res://x.tscn",
            "godot_args": ["--single-window"],
        });
        assert_eq!(
            skip_strict_cold_reason(&scenario, Path::new("."), 60, None, None),
            Some("godot_args")
        );
        let mut result = cmd(true, &[], &[]);
        annotate_warm_result(&mut result, false, None, None, Some("godot_args"), true);
        assert!(!result.ok);
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.failures[0].kind, "warm_fallback_forced_cold");
        assert_eq!(result.data["warm"]["ran_warm"], json!(false));
    }

    #[test]
    fn skip_strict_declared_cold_scenario_stays_exempt() {
        // execution.isolation: cold stays exempt during the skip, exactly as on
        // the non-skip warm path: it is classified with the declared reason, and
        // annotate_warm_result treats that reason as strict-exempt.
        let scenario = json!({
            "name": "x", "scene": "res://x.tscn",
            "execution": {"isolation": "cold"},
        });
        assert_eq!(
            skip_strict_cold_reason(&scenario, Path::new("."), 60, None, None),
            Some(DECLARED_COLD_REASON)
        );
        let mut result = cmd(true, &[], &[]);
        annotate_warm_result(
            &mut result,
            false,
            None,
            None,
            Some(DECLARED_COLD_REASON),
            true,
        );
        assert!(result.ok);
        assert!(result.failures.is_empty());
    }

    #[test]
    fn skip_strict_warm_eligible_scenario_is_not_a_violation() {
        // A plain warm-eligible scenario would have run warm on a smaller host, so
        // a cold run under the skip is not a strict violation: nothing to annotate.
        let scenario = json!({"name": "x", "scene": "res://x.tscn"});
        assert_eq!(
            skip_strict_cold_reason(&scenario, Path::new("."), 60, None, None),
            None
        );
    }
