// Per-scenario warm dispatch: the `scenario.run` bridge round trip, the cold
// fallback around it, the `data.warm` annotation (including `--warm-strict`
// enforcement), and the result assembly shared with the cold path.
//
// Included into `run/warm.rs`, which is itself included into the run module, so
// this file shares the whole `crate::run` scope.

fn project_default_fixed_fps(config: &ProjectConfig) -> i64 {
    crate::frame_pacing::project_default_frame_pacing(&config.project_root).fixed_fps
}

/// Run one scenario cold (fresh process) and annotate why it fell back to cold.
/// A cold run is always authoritative, so this is the safe fallback for every
/// path that cannot (or should not) run warm.
#[allow(clippy::too_many_arguments)]
fn cold_fallback_result(
    config: &ProjectConfig,
    scenario_path: &Path,
    scenario: &Value,
    command_line: &str,
    headless_override: Option<bool>,
    audio_override: Option<bool>,
    reason: &str,
    warm_strict: bool,
) -> CommandResult {
    let mut result = run_scenario_once(
        config,
        scenario_path,
        scenario,
        command_line,
        headless_override,
        audio_override,
        VisualRiskExecutionOptions::default(),
    );
    annotate_warm_result(&mut result, false, None, None, Some(reason), warm_strict);
    repersist_annotated_result(&result);
    result
}

/// Drive a headless-warm scenario through a pooled headless session of the given
/// `audio` flavor: check one out, run it, and release it for the next scenario.
/// Falls back to cold if no session is available (infrastructure, strict-exempt)
/// or the round-trip fails (transport failure, a strict violation).
#[allow(clippy::too_many_arguments)]
fn warm_headless_scenario(
    config: &ProjectConfig,
    scenario_path: &Path,
    scenario: &Value,
    command_line: &str,
    headless_override: Option<bool>,
    audio_override: Option<bool>,
    audio: bool,
    pool: &WarmSessionPool,
    warm_strict: bool,
) -> CommandResult {
    let Some((handle, acquire_wait)) = pool.acquire_headless(config, audio) else {
        // No pooled headless session available — run cold. A session-start
        // hiccup / saturated pool is infrastructure, not a scenario declaration,
        // so it is exempt from --warm-strict just like a failed session start.
        return cold_fallback_result(
            config,
            scenario_path,
            scenario,
            command_line,
            headless_override,
            audio_override,
            HEADLESS_SESSION_UNAVAILABLE_REASON,
            warm_strict,
        );
    };
    let lane_held_started = Instant::now();
    match warm_dispatch_scenario(
        config,
        scenario_path,
        scenario,
        command_line,
        &handle,
        true,
        audio,
    ) {
        Ok(dispatch) => {
            // The engine round-trip is done and result.json is on disk — release
            // the session BEFORE the file-only finalize so another worker's next
            // scenario overlaps this thread's post-processing.
            pool.release_headless(handle, audio);
            let lane_held_ms = millis_since(lane_held_started);
            let (mut result, timing) = match dispatch {
                WarmDispatch::EarlyResult(result, timing) => (result, timing),
                WarmDispatch::Dispatched(dispatched) => {
                    warm_finalize_scenario(config, scenario_path, scenario, true, *dispatched, pool)
                }
            };
            // The persisted result.json already carries data.warm (seeded into
            // base_data before finalize), so no annotation re-write is needed;
            // annotate_warm_result just keeps the in-memory strict logic uniform.
            let log_annotation = take_warm_log_annotation(&result);
            annotate_warm_result(
                &mut result,
                true,
                Some(WARM_MODE_HEADLESS),
                Some(warm_audio_flavor(audio)),
                None,
                warm_strict,
            );
            restore_warm_log_annotation(&mut result, log_annotation);
            insert_warm_timing(&mut result, &timing, lane_held_ms, millis_of(acquire_wait));
            result
        }
        Err(WarmDispatchError::Prep) => {
            // CLI-side prep failed before the engine was ever contacted, so the
            // session is healthy: RELEASE it. Killing it here cost the pool a live
            // session for a fault that was never the session's.
            pool.release_headless(handle, audio);
            cold_fallback_result(
                config,
                scenario_path,
                scenario,
                command_line,
                headless_override,
                audio_override,
                WARM_PREP_FAILURE_REASON,
                warm_strict,
            )
        }
        Err(WarmDispatchError::Transport(detail)) => {
            // The warm session round-trip itself failed — retire this (likely
            // dead) session and run the scenario cold instead. This is the
            // clearest literal "Godot needed a restart mid-bundle" signal, and the
            // failure is kept as bundle evidence instead of vanishing.
            pool.record_transport_failure(
                headless_lane_label(audio),
                scenario_name_of(scenario),
                &handle.record.session_id,
                &detail,
            );
            pool.discard_headless(config, handle, audio);
            cold_fallback_result(
                config,
                scenario_path,
                scenario,
                command_line,
                headless_override,
                audio_override,
                "warm_transport_failure",
                warm_strict,
            )
        }
    }
}

/// Drive a windowed-media scenario through the single serialized windowed session,
/// started lazily on first use. Holding the windowed lock across the run serializes
/// windowed scenarios (only one windowed Godot process exists). When the held
/// session is the OTHER audio flavor, it is discarded and restarted with the flavor
/// this scenario needs (rare). Falls back to cold if the windowed session cannot
/// start (strict-exempt) or the round-trip fails.
#[allow(clippy::too_many_arguments)]
fn warm_windowed_scenario(
    config: &ProjectConfig,
    scenario_path: &Path,
    scenario: &Value,
    command_line: &str,
    headless_override: Option<bool>,
    audio_override: Option<bool>,
    audio: bool,
    pool: &WarmSessionPool,
    warm_strict: bool,
) -> CommandResult {
    let acquire_started = Instant::now();
    let mut slot = pool.lock_windowed();
    // The single windowed slot serves one audio flavor at a time. If it currently
    // holds the OTHER flavor, retire that session so we relaunch with the flavor
    // this scenario needs (rare — most bundles never mix windowed audio flavors).
    if slot.session.is_some() && slot.audio != audio {
        if let Some(handle) = slot.session.take() {
            let _ = godot_qa_runtime::stop_pooled_session(config, &handle);
        }
        // A deliberate flavor switch is not a failed start; the new flavor gets a
        // fresh budget.
        slot.consecutive_start_failures = 0;
    }
    slot.audio = audio;
    if slot.session.is_none() {
        // Same failure budget as a headless lane: one transient windowed start
        // failure must not condemn the slot for the whole bundle.
        if slot.consecutive_start_failures >= WARM_START_FAILURE_BUDGET {
            drop(slot);
            return cold_fallback_result(
                config,
                scenario_path,
                scenario,
                command_line,
                headless_override,
                audio_override,
                WINDOWED_SESSION_UNAVAILABLE_REASON,
                warm_strict,
            );
        }
        let attempt = slot.consecutive_start_failures + 1;
        match start_warm_pool_session(config, false, audio) {
            Ok(handle) => {
                pool.note_session_started(&handle.record);
                slot.session = Some(handle);
                slot.consecutive_start_failures = 0;
            }
            Err(detail) => {
                slot.consecutive_start_failures =
                    slot.consecutive_start_failures.saturating_add(1);
                drop(slot);
                pool.record_start_failure(WINDOWED_LANE_LABEL, attempt, &detail);
                return cold_fallback_result(
                    config,
                    scenario_path,
                    scenario,
                    command_line,
                    headless_override,
                    audio_override,
                    WINDOWED_SESSION_UNAVAILABLE_REASON,
                    warm_strict,
                );
            }
        }
    }
    let acquire_wait = acquire_started.elapsed();
    let handle = slot
        .session
        .as_ref()
        .expect("windowed session ensured above")
        .clone();
    let lane_held_started = Instant::now();
    match warm_dispatch_scenario(
        config,
        scenario_path,
        scenario,
        command_line,
        &handle,
        false,
        audio,
    ) {
        Ok(dispatch) => {
            // The engine round-trip is done and result.json is on disk. Drop the
            // slot lock NOW so the next windowed scenario's engine work overlaps
            // this file-only finalize (screenshot compares/pixel analysis) — the
            // windowed PROCESS stays serialized because the lock was held for the
            // entire engine round-trip, which is all that touches the session.
            drop(slot);
            let lane_held_ms = millis_since(lane_held_started);
            let (mut result, timing) = match dispatch {
                WarmDispatch::EarlyResult(result, timing) => (result, timing),
                WarmDispatch::Dispatched(dispatched) => warm_finalize_scenario(
                    config,
                    scenario_path,
                    scenario,
                    false,
                    *dispatched,
                    pool,
                ),
            };
            // result.json already carries data.warm via base_data (see the
            // headless path) — no annotation re-write needed.
            let log_annotation = take_warm_log_annotation(&result);
            annotate_warm_result(
                &mut result,
                true,
                Some(WARM_MODE_WINDOWED),
                Some(warm_audio_flavor(audio)),
                None,
                warm_strict,
            );
            restore_warm_log_annotation(&mut result, log_annotation);
            insert_warm_timing(&mut result, &timing, lane_held_ms, millis_of(acquire_wait));
            result
        }
        Err(WarmDispatchError::Prep) => {
            // CLI-side prep failure: the windowed session never saw a request, so
            // it stays in the slot for the next windowed scenario.
            drop(slot);
            cold_fallback_result(
                config,
                scenario_path,
                scenario,
                command_line,
                headless_override,
                audio_override,
                WARM_PREP_FAILURE_REASON,
                warm_strict,
            )
        }
        Err(WarmDispatchError::Transport(detail)) => {
            // The windowed round-trip failed — retire the (likely dead) windowed
            // session so the next windowed scenario starts a fresh one, then cold.
            pool.record_transport_failure(
                WINDOWED_LANE_LABEL,
                scenario_name_of(scenario),
                &handle.record.session_id,
                &detail,
            );
            if let Some(handle) = slot.session.take() {
                let _ = godot_qa_runtime::stop_pooled_session(config, &handle);
            }
            drop(slot);
            cold_fallback_result(
                config,
                scenario_path,
                scenario,
                command_line,
                headless_override,
                audio_override,
                "warm_transport_failure",
                warm_strict,
            )
        }
    }
}

/// Per-scenario wall-clock spans measured around the warm dispatch path, so the
/// bundle `--json` can attribute where warm time goes (Rust prep vs the engine
/// round-trip vs CLI finalize). In-memory only — surfaced as `data.warm.timing`
/// via the bundle summary, never persisted by an extra artifact write.
#[derive(Debug, Default, Clone, Copy)]
struct WarmTiming {
    /// prepare_run_artifacts + warm_base_data + request writes.
    artifact_prep_ms: f64,
    /// The blocking `scenario.run` round-trip (includes all engine work).
    dispatch_ms: f64,
    /// The engine's own in-scenario wall clock (result.json duration_ms).
    engine_ms: Option<f64>,
    /// finalize_runtime_run: result read-back, validation, compares, persist.
    finalize_ms: f64,
}

/// Nest the measured spans under `data.warm.timing` (annotate_warm_result must
/// have created `data.warm` first).
fn insert_warm_timing(
    result: &mut CommandResult,
    timing: &WarmTiming,
    lane_held_ms: f64,
    acquire_wait_ms: f64,
) {
    let Some(warm) = result.data.get_mut("warm").and_then(Value::as_object_mut) else {
        return;
    };
    warm.insert(
        "timing".to_string(),
        json!({
            "artifact_prep_ms": timing.artifact_prep_ms,
            "dispatch_ms": timing.dispatch_ms,
            "engine_ms": timing.engine_ms,
            "finalize_ms": timing.finalize_ms,
            "lane_held_ms": lane_held_ms,
            "acquire_wait_ms": acquire_wait_ms,
        }),
    );
}

/// The scenario result.json is persisted inside run_scenario_once /
/// finalize_runtime_run, BEFORE the warm annotation (and any strict failure)
/// mutates the in-memory result — re-write it so the on-disk artifact matches
/// what the bundle summary reports. Early-failure results without a
/// result_path have no artifact to update.
fn repersist_annotated_result(result: &CommandResult) {
    let Some(result_path) = result
        .data
        .get("artifacts")
        .and_then(|artifacts| artifacts.get("result_path"))
        .and_then(Value::as_str)
    else {
        return;
    };
    let _ = write_result_artifact(Path::new(result_path), result);
}

/// Read back the `data.warm` annotation a warm-leg result carries, so the
/// equivalence path can re-apply it (with strict enforcement) to the cold gate
/// result. Returns (ran_warm, mode, audio, reason).
fn warm_annotation_of(
    result: &CommandResult,
) -> (bool, Option<String>, Option<String>, Option<String>) {
    let warm = result.data.get("warm");
    let ran_warm = warm
        .and_then(|warm| warm.get("ran_warm"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mode = warm
        .and_then(|warm| warm.get("mode"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let audio = warm
        .and_then(|warm| warm.get("audio"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let reason = warm
        .and_then(|warm| warm.get("reason"))
        .and_then(Value::as_str)
        .map(str::to_string);
    (ran_warm, mode, audio, reason)
}

/// Bundle-level `data.warm` for a run whose SEED warm session could not start: the
/// historical status object plus the failure that caused it, so a bundle that
/// degraded to all-cold names its cause instead of dropping it.
fn warm_session_start_failed_json(detail: &FailureDetail) -> Value {
    let mut status = warm_status_json(false, "session_start_failed");
    if let Some(object) = status.as_object_mut() {
        object.insert("start_failure".to_string(), failure_detail_json(detail));
    }
    status
}

/// Hard failure for --warm-strict when the shared warm session cannot start.
fn warm_session_start_failed_result(command: &str, cause: &FailureDetail) -> CommandResult {
    let mut data = Map::new();
    data.insert("warm".to_string(), warm_session_start_failed_json(cause));
    CommandResult {
        ok: false,
        command: command.to_string(),
        message: "scenario bundle run failed (warm session could not start)".to_string(),
        data,
        failures: vec![FailureDetail {
            kind: "warm_session_start_failed".to_string(),
            command: Some(command.to_string()),
            message: Some(
                "the shared warm Godot session could not start; check the live session logs, \
                 or rerun without --warm-strict to fall back to a cold bundle"
                    .to_string(),
            ),
            expected: Some(json!("warm session starts and hosts every warm-eligible scenario")),
            actual: Some(json!("session_start_failed")),
            // The underlying startup failure, verbatim: without it a strict bundle
            // reports only that "the session could not start" and the operator has
            // to go hunting through the live artifact dirs.
            details: Map::from_iter([("cause".to_string(), failure_detail_json(cause))]),
            ..FailureDetail::default()
        }],
        exit_code: 1,
    }
}

/// A cold-fallback reason that a `--warm-strict` bundle does NOT treat as a
/// violation: a declared `execution.isolation: cold` opt-out, a warm session
/// that could not start (infrastructure, not a scenario declaration), or a
/// bundle that skipped warm below parallelism (a property of the host's core
/// count, not of the scenario). Every other reason — structural cold reasons and
/// `warm_transport_failure` — IS a strict violation.
fn reason_is_strict_exempt(reason: &str) -> bool {
    matches!(
        reason,
        DECLARED_COLD_REASON
            | WINDOWED_SESSION_UNAVAILABLE_REASON
            | HEADLESS_SESSION_UNAVAILABLE_REASON
            | WARM_SKIPPED_BELOW_PARALLELISM_REASON
    )
}

/// The `data.warm` annotation payload shared by annotate_warm_result and the
/// warm dispatch path (which seeds it into base_data BEFORE finalize so the
/// single persisted result.json already carries it — no re-write needed).
/// `audio` records which audio flavor the warm session used
/// (`"dummy"`/`"real"`), or None for a cold fallback (mode is None too).
fn warm_annotation_json(
    ran_warm: bool,
    mode: Option<&str>,
    audio: Option<&str>,
    reason: Option<&str>,
) -> Value {
    json!({
        "ran_warm": ran_warm,
        "declared": reason == Some(DECLARED_COLD_REASON),
        "mode": mode,
        "audio": audio,
        "reason": reason,
    })
}

/// Record whether a scenario actually ran warm, in which mode + audio flavor, and
/// if not, why — and, in strict mode, turn a non-exempt fallback into a hard
/// failure.
#[allow(clippy::too_many_arguments)]
fn annotate_warm_result(
    result: &mut CommandResult,
    ran_warm: bool,
    mode: Option<&str>,
    audio: Option<&str>,
    reason: Option<&str>,
    warm_strict: bool,
) {
    let exempt = reason.map(reason_is_strict_exempt).unwrap_or(false);
    result.data.insert(
        "warm".to_string(),
        warm_annotation_json(ran_warm, mode, audio, reason),
    );
    if warm_strict && !ran_warm && !exempt {
        let reason = reason.unwrap_or("unknown_reason");
        result.ok = false;
        result.exit_code = 1;
        result.failures.push(FailureDetail {
            kind: "warm_fallback_forced_cold".to_string(),
            command: Some("scenario run".to_string()),
            target: result
                .data
                .get("scenario")
                .and_then(Value::as_str)
                .map(str::to_string),
            message: Some(format!(
                "{reason} forced this scenario to run cold in a --warm --warm-strict bundle"
            )),
            expected: Some(json!("scenario runs in the shared warm session")),
            actual: Some(json!(reason)),
            ..FailureDetail::default()
        });
    }
}

/// Why a warm dispatch could not produce a result. The distinction is what keeps a
/// healthy session alive: only a TRANSPORT failure implicates the session itself.
enum WarmDispatchError {
    /// CLI-side prep failed before the engine was contacted (invalid frame pacing).
    /// The session never saw a request, so it is released, not retired.
    Prep,
    /// The `scenario.run` round-trip failed (busy/dead session, connect error).
    /// The session is retired and the failure kept as bundle evidence.
    Transport(Box<FailureDetail>),
}

/// Outcome of the session-holding half of a warm scenario: either the prep
/// failed with a final result before any engine contact, or the engine round
/// trip completed and only file-only finalize work remains.
enum WarmDispatch {
    EarlyResult(CommandResult, WarmTiming),
    Dispatched(Box<WarmDispatched>),
}

/// Everything warm_finalize_scenario needs after the session round-trip; the
/// pooled session itself is deliberately NOT part of this — finalize is
/// file-only, so the session can already host the next scenario.
struct WarmDispatched {
    artifacts: artifacts::RunArtifacts,
    base_data: Map<String, Value>,
    options: RuntimeExecutionOptions,
    use_runtime_request: bool,
    timing: WarmTiming,
    /// The session that hosted this scenario, recorded on `data.warm.session_id`
    /// so a warm run's engine output is traceable to one pooled process.
    session_id: String,
    /// Where in that session's log this scenario's output starts. None when the
    /// session has no readable host-side log (a non-desktop target, or the file
    /// not existing yet), which makes the slice `unavailable`.
    session_log: Option<WarmSessionLogAnchor>,
}

/// Drive one warm scenario via the scenario.run bridge command on a SPECIFIC
/// pooled session (`handle`) — the session-holding half only (prep + the
/// blocking engine round-trip). `headless` selects which session flavor is
/// hosting this scenario (windowed-warm scenarios pass `false` so the
/// synthesized metadata and display match the windowed session that renders the
/// real pixels). Returns Err(..) on a prep or transport failure so the caller can
/// fall back to cold — and, for a transport failure, retire the session.
#[allow(clippy::too_many_arguments)]
fn warm_dispatch_scenario(
    config: &ProjectConfig,
    scenario_path: &Path,
    scenario: &Value,
    command_line: &str,
    handle: &godot_qa_runtime::PooledSessionHandle,
    headless: bool,
    audio: bool,
) -> Result<WarmDispatch, WarmDispatchError> {
    let mut timing = WarmTiming::default();
    let command = "scenario run";
    let result_message = "scenario run failed";
    if let Some(failure) = config.project_config_failure_for_command(command) {
        return Ok(WarmDispatch::EarlyResult(
            CommandResult {
                ok: false,
                command: command.to_string(),
                message: result_message.to_string(),
                data: Map::new(),
                failures: vec![failure],
                exit_code: 1,
            },
            timing,
        ));
    }

    let scenario_timeout = match scenario_timeout_duration(command, scenario_path, scenario) {
        Ok(timeout) => timeout,
        Err(failure) => {
            return Ok(WarmDispatch::EarlyResult(
                CommandResult {
                    ok: false,
                    command: command.to_string(),
                    message: result_message.to_string(),
                    data: Map::new(),
                    failures: vec![failure],
                    exit_code: 1,
                },
                timing,
            ));
        }
    };
    let prep_started = Instant::now();

    // A `screenshot_match` assertion is CLI-side post-processing: the engine
    // renders and saves the snapshot, and the CLI compares it to a baseline in
    // `finalize_runtime_run` (use_runtime_request == true). Everything else
    // (a saved screenshot, a visual pixel assertion, audio capture, or a plain
    // headless-warm scenario) uses the common artifact layout.
    let screenshot_assertions = screenshot_match_assertions(scenario);
    let use_runtime_request = !screenshot_assertions.is_empty();
    let scenario_for_artifacts = if use_runtime_request {
        scenario.clone()
    } else {
        runtime_scenario_payload(scenario)
    };
    let scenario_for_artifacts =
        scenario_with_effective_launch_args(config, &scenario_for_artifacts);
    let artifacts = match artifacts::prepare_run_artifacts(
        &config.artifacts_dir,
        &scenario_for_artifacts,
        &config.project_root,
        scenario_path,
        command_line,
        config.godot_binary.as_deref(),
        &config.godot_launch_prefix,
        &config.godot_windowed_launch_prefix,
        None,
        None,
    ) {
        Ok(artifacts) => artifacts,
        Err(error) => {
            return Ok(WarmDispatch::EarlyResult(
                CommandResult {
                    ok: false,
                    command: command.to_string(),
                    message: result_message.to_string(),
                    data: Map::new(),
                    failures: vec![FailureDetail {
                        kind: "artifact_write_failed".to_string(),
                        command: Some(command.to_string()),
                        path: Some(config.artifacts_dir.to_string_lossy().into_owned()),
                        message: Some(error.to_string()),
                        expected: Some(json!(
                            "artifact metadata written under the artifact directory"
                        )),
                        actual: Some(json!("OSError")),
                        ..FailureDetail::default()
                    }],
                    exit_code: 1,
                },
                timing,
            ));
        }
    };

    let mut base_data =
        match warm_base_data(config, scenario_path, scenario, &artifacts, headless, audio) {
            Ok(base_data) => base_data,
            // A frame-pacing validation error here means the scenario should run
            // cold; signal a CLI-side prep fallback, which leaves the session alone.
            Err(()) => return Err(WarmDispatchError::Prep),
        };
    // Seed the warm annotation into base_data so every finalize_runtime_run exit
    // persists a result.json that already matches the bundle summary — the
    // success path needs no annotation re-write afterwards. Safe because a
    // ran_warm result never gains a strict failure in annotate_warm_result.
    let warm_mode = if headless {
        WARM_MODE_HEADLESS
    } else {
        WARM_MODE_WINDOWED
    };
    base_data.insert(
        "warm".to_string(),
        warm_annotation_json(true, Some(warm_mode), Some(warm_audio_flavor(audio)), None),
    );

    let options = RuntimeExecutionOptions {
        timeout: scenario_timeout,
        ..RuntimeExecutionOptions::default()
    };

    // For a screenshot_match scenario the engine reads a stripped
    // runtime-request.json (the CLI keeps the full scenario for the baseline
    // compare), byte-identical to the cold path. Otherwise it reads the
    // request.json prepare_run_artifacts already wrote.
    let request_path = if use_runtime_request {
        let runtime_request_path = artifacts.root_dir.join("runtime-request.json");
        let scenario_godot_args = scenario_godot_args(scenario);
        let scenario_project_args = scenario_project_args(scenario);
        let godot_args = merged_launch_args(&config.godot_args, &scenario_godot_args);
        let project_args = merged_launch_args(&config.godot_project_args, &scenario_project_args);
        let runtime_scenario = runtime_scenario_payload(&scenario_with_launch_args(
            scenario,
            &godot_args,
            &project_args,
        ));
        if let Err(error) = write_runtime_request(
            &runtime_request_path,
            &runtime_scenario,
            &config.project_root,
            &artifacts,
            command_line,
            options,
        ) {
            return Ok(WarmDispatch::EarlyResult(
                CommandResult {
                    ok: false,
                    command: command.to_string(),
                    message: result_message.to_string(),
                    data: base_data,
                    failures: vec![failure_with_artifacts(
                        FailureDetail {
                            kind: "artifact_write_failed".to_string(),
                            command: Some(command.to_string()),
                            path: Some(runtime_request_path.to_string_lossy().into_owned()),
                            message: Some(error.to_string()),
                            expected: Some(json!("runtime request artifact written to disk")),
                            actual: Some(json!("OSError")),
                            ..FailureDetail::default()
                        },
                        &artifacts.to_json().as_object().cloned().unwrap_or_default(),
                    )],
                    exit_code: 1,
                },
                timing,
            ));
        }
        runtime_request_path
    } else {
        artifacts.request_path.clone()
    };
    timing.artifact_prep_ms = millis_since(prep_started);

    // Send scenario.run with the path to the request the engine should read,
    // plus a timeout budget for the transport.
    let mut params = Map::new();
    params.insert(
        "request_path".to_string(),
        json!(request_path.to_string_lossy().into_owned()),
    );
    // The run id keys this scenario's stderr markers in the shared session log.
    // Old addons ignore the param; new addons run without it (cold) unchanged.
    params.insert("run_id".to_string(), json!(artifacts.run_id));
    if let Some(timeout) = scenario_timeout {
        params.insert("timeout_seconds".to_string(), json!(timeout.as_secs_f64()));
        // With a frame budget the engine enforces the deadline itself; the
        // transport sees timeout_frames and relaxes its wall clock to a hang
        // backstop instead of failing correct runs on loaded hosts.
        if let Some(frames) = scenario_frame_budget_json(scenario, &config.project_root)
            .and_then(|budget| budget.get("frames").and_then(Value::as_u64))
        {
            params.insert("timeout_frames".to_string(), json!(frames));
        }
    }
    // Anchored immediately before the dispatch so the slice read starts past
    // everything earlier scenarios on this lane already wrote.
    let session_log = warm_session_log_anchor(handle);
    let dispatch_started = Instant::now();
    let live = godot_qa_runtime::pooled_session_command(
        handle,
        command,
        godot_qa_runtime::SCENARIO_RUN,
        params,
    );
    timing.dispatch_ms = millis_since(dispatch_started);
    if !live.ok {
        // Session busy / connect failure / request error -> fall back to cold, and
        // KEEP the failure: it is the only evidence of why a warm session had to be
        // retired mid-bundle.
        let message = live.message.clone();
        let detail = live.failures.into_iter().next().unwrap_or(FailureDetail {
            kind: "warm_transport_failure".to_string(),
            command: Some(command.to_string()),
            message: Some(message),
            expected: Some(json!(
                "scenario.run round-trip completes on the pooled warm session"
            )),
            actual: Some(json!("live command failed")),
            ..FailureDetail::default()
        });
        return Err(WarmDispatchError::Transport(Box::new(detail)));
    }

    // The engine has written result.json; everything left (read-back, validation,
    // compares, persist) is file-only, so the session is no longer needed.
    Ok(WarmDispatch::Dispatched(Box::new(WarmDispatched {
        artifacts,
        base_data,
        options,
        use_runtime_request,
        timing,
        session_id: handle.record.session_id.clone(),
        session_log,
    })))
}

/// CLI-side post-processing of a dispatched warm scenario. Needs no session:
/// the engine already wrote result.json, so a headless caller releases its
/// pooled session BEFORE this runs (the next scenario's engine work overlaps
/// this finalize).
fn warm_finalize_scenario(
    config: &ProjectConfig,
    scenario_path: &Path,
    scenario: &Value,
    headless: bool,
    dispatched: WarmDispatched,
    pool: &WarmSessionPool,
) -> (CommandResult, WarmTiming) {
    let WarmDispatched {
        artifacts,
        mut base_data,
        options,
        use_runtime_request,
        mut timing,
        session_id,
        session_log,
    } = dispatched;
    // Cut this scenario's window out of the pooled session's engine log and write
    // it to <run_root>/logs.txt BEFORE finalizing, so runtime_log_failures,
    // log_status_summary_json and missing_result_actual all see exactly what the
    // cold path gives them — no warm special cases downstream.
    let log_slice = materialize_warm_scenario_logs(&artifacts, &session_log);
    if log_slice.status == WARM_LOG_SLICE_MARKERS_MISSING {
        pool.note_log_markers_missing(&session_id, &log_slice.source);
    }
    let window = log_slice.window();
    if let Some(warm) = base_data.get_mut("warm").and_then(Value::as_object_mut) {
        warm.insert(WARM_SESSION_ID_KEY.to_string(), json!(session_id));
        warm.insert(WARM_LOG_SLICE_KEY.to_string(), log_slice.to_json(&session_id));
    }
    // Synthesize a successful process result so the shared finalizer reads the
    // engine's result.json exactly as it does for cold.
    let display = godot_qa_process::resolve_project_display_launch_metadata_with_windowed_prefix(
        &config.project_root,
        &config.godot_launch_prefix,
        &config.godot_windowed_launch_prefix,
        headless,
    );
    let process_result = synth_warm_process_result(display);
    let finalize_started = Instant::now();
    let mut result = finalize_runtime_run(
        "scenario run",
        "scenario run failed",
        config,
        scenario_path,
        scenario,
        &artifacts,
        use_runtime_request,
        headless,
        options,
        base_data,
        &process_result,
    );
    // Point every engine-log failure at the session-log window it was read from,
    // so a warm `godot_log_error` is as traceable as a cold one (which simply owns
    // its whole logs.txt), then re-persist so result.json matches.
    if annotate_warm_log_failures(&mut result, &session_id, window.as_ref()) {
        repersist_annotated_result(&result);
    }
    timing.finalize_ms = millis_since(finalize_started);
    timing.engine_ms = result.data.get("duration_ms").and_then(Value::as_f64);
    (result, timing)
}

/// Build the pre-launch metadata map, mirroring execute_runtime_run so the warm
/// CommandResult carries the same godot_args / project_args / frame_pacing /
/// launch_display fields a cold run records. Returns Err(()) when frame pacing
/// fails to validate (the caller then routes the scenario to cold).
fn warm_base_data(
    config: &ProjectConfig,
    scenario_path: &Path,
    scenario: &Value,
    artifacts: &artifacts::RunArtifacts,
    headless: bool,
    audio: bool,
) -> Result<Map<String, Value>, ()> {
    let scenario_godot_args = scenario_godot_args(scenario);
    let scenario_project_args = scenario_project_args(scenario);
    let godot_args = merged_launch_args(&config.godot_args, &scenario_godot_args);
    let project_args = merged_launch_args(&config.godot_project_args, &scenario_project_args);
    let frame_pacing = match scenario_frame_pacing(scenario, &config.project_root) {
        Ok(frame_pacing) => frame_pacing,
        Err(_) => return Err(()),
    };
    let mut base_data = Map::from_iter([
        (
            "scenario".to_string(),
            json!(scenario.get("name").and_then(Value::as_str).unwrap_or_default()),
        ),
        (
            "scene".to_string(),
            json!(scenario.get("scene").and_then(Value::as_str).unwrap_or_default()),
        ),
        (
            "scenario_path".to_string(),
            json!(scenario_path.to_string_lossy().into_owned()),
        ),
        ("artifacts".to_string(), artifacts.to_json()),
    ]);
    base_data.insert("godot_args".to_string(), json!(godot_args));
    if let Some(frame_pacing) = &frame_pacing {
        base_data.insert("frame_pacing".to_string(), frame_pacing.to_json());
    }
    base_data.insert(
        "project_args".to_string(),
        redact_project_args_for_metadata(&json!(project_args)),
    );
    base_data.insert("headless".to_string(), json!(headless));
    base_data.insert("audio".to_string(), json!(audio));
    let launch_display =
        godot_qa_process::resolve_project_display_launch_metadata_with_windowed_prefix(
            &config.project_root,
            &config.godot_launch_prefix,
            &config.godot_windowed_launch_prefix,
            headless,
        );
    base_data.insert(
        "launch_display".to_string(),
        display_launch_metadata_json(&launch_display),
    );
    Ok(base_data)
}

fn synth_warm_process_result(
    display: godot_qa_process::DisplayLaunchMetadata,
) -> godot_qa_process::GodotRunResult {
    godot_qa_process::GodotRunResult {
        command: Vec::new(),
        returncode: 0,
        termination: None,
        process_timing: None,
        stdout: String::new(),
        stderr: String::new(),
        display,
        pid: None,
        process_group: None,
        cleanup: None,
        infrastructure_failure_kind: None,
        timed_out: false,
        timeout_ms: None,
        output_truncation: None,
    }
}
