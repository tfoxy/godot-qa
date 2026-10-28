// Warm (in-process) bundle execution.
//
// Included into the run module (see `include!` at the bottom of mod.rs), so it
// shares all the private cold-path building blocks — `run_scenario_once`,
// `prepare_run_artifacts`, `finalize_runtime_run`, and the scenario metadata
// helpers — and reuses them verbatim. A persistent Godot session drives every
// warm-eligible scenario via the `scenario.run` bridge command; the engine
// writes the SAME result.json the cold path writes, and warm reuses the SAME
// `finalize_runtime_run` post-process, so cold and warm produce identical
// CommandResult/BundleRunSummary shapes.
//
// Scenarios split into three buckets:
//   1. headless-warm  — no windowed-media need; runs in a shared HEADLESS warm
//      session (the historical warm behavior).
//   2. windowed-warm  — the ONLY reason it would need cold is windowed-media
//      evidence (a saved screenshot, a visual pixel assertion, a screenshot
//      match, or audio capture) AND it is single-viewport, its fixed_fps matches
//      the session, it has no per-scenario `godot_args`, and it is not declared
//      `execution.isolation: cold`. It runs in a shared WINDOWED warm session so
//      it captures real pixels between scene reloads instead of relaunching
//      Godot per scenario.
//   3. truly-cold     — declared `execution.isolation: cold`, per-scenario
//      `godot_args`, more than one viewport, a layout matrix, or a differing
//      `determinism.fixed_fps`. These fall back to a fresh cold process each.
//
// The singleton interactive session (`.qa/session.json`) can host only one live
// session, so the warm bundle instead uses a POOL of independently-addressed
// sessions (`WarmSessionPool`, backed by godot_qa_runtime::PooledSessionHandle):
// up to K = the bundle's resolved parallelism concurrent HEADLESS sessions,
// distributed across the parallel bundle workers so warm gets parallelism AND
// per-session boot reuse. Each pooled session is a separate process with its own
// ephemeral port, token, and isolated `.qa/artifacts/live/<session-id>/` dir, so
// the K sessions never collide. Windowed-media scenarios share a single serialized
// windowed session (running many windowed Godot processes at once is display-risky
// and these scenarios are rare), started lazily only if one appears.
//
// Cold stays authoritative: any truly-cold scenario falls back to the exact cold
// path, and if a warm session cannot start or a scenario.run round-trip fails the
// scenario also falls back to cold — so `--warm` never produces a worse outcome
// than the cold bundle.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};

const WARM_SESSION_VIEWPORT: &str = "1920x1080";

/// How many CONSECUTIVE failed session starts one warm lane tolerates before it
/// stops booting. A single transient failure (a lost rename race, a port that was
/// momentarily unavailable) must not condemn the lane for the rest of the bundle:
/// the historical `failed: bool` did exactly that, and one ~1 ms failure among
/// seven concurrently booting lanes sent 10 of 34 scenarios down the ~2.5 s cold
/// path. Any successful start resets the counter, so only a genuinely broken host
/// exhausts the budget — and warm still never becomes worse than cold, because an
/// exhausted lane just falls back to cold.
const WARM_START_FAILURE_BUDGET: u32 = 2;

/// Bounds on how long a worker waits for a busy pooled session to come free before
/// giving up and running the scenario cold. The budget itself is derived from the
/// SEED session's measured boot — waiting longer than one boot can never beat just
/// running cold — and clamped here so a pathologically fast seed cannot degenerate
/// into a busy wait, nor a pathologically slow one into a stall.
const WARM_ACQUIRE_WAIT_MIN: Duration = Duration::from_millis(250);
const WARM_ACQUIRE_WAIT_MAX: Duration = Duration::from_secs(5);
/// Wait budget for a pool whose seed boot was never measured.
const WARM_ACQUIRE_WAIT_FALLBACK: Duration = Duration::from_secs(2);

/// Reason code recorded when a scenario runs cold because it explicitly opted
/// out via `execution.isolation: cold` — an intentional, documented choice,
/// never treated as a `--warm-strict` violation.
const DECLARED_COLD_REASON: &str = "execution_isolation_cold";

/// Reason recorded when a windowed-warm-eligible scenario has to fall back to
/// cold because the shared WINDOWED warm session could not start (e.g. no
/// display backend). A session-start failure is an infrastructure condition,
/// not something the scenario declared wrong, so — like a headless session that
/// fails to start — it is NOT a `--warm-strict` violation. The scenario still
/// runs (cold), so warm is never worse than cold.
const WINDOWED_SESSION_UNAVAILABLE_REASON: &str = "windowed_session_unavailable";

/// Reason recorded when a headless-warm scenario has to fall back to cold
/// because no pooled headless session could be acquired before the wait deadline:
/// session starts exhausted the lane's failure budget, or every live session
/// stayed checked out by another worker for longer than one boot is worth waiting.
/// Same rationale as the windowed case: a session-start hiccup or a saturated pool
/// is infrastructure, not a scenario declaration, so it is strict-exempt.
const HEADLESS_SESSION_UNAVAILABLE_REASON: &str = "warm_session_unavailable";

/// Reason recorded when a warm scenario falls back to cold because CLI-side prep
/// failed before the engine was ever contacted (today: invalid frame pacing). The
/// pooled session is untouched and healthy, so it is released rather than retired;
/// the reason is the same structural one a classification pass would have given.
const WARM_PREP_FAILURE_REASON: &str = "frame_pacing_invalid";

const WARM_MODE_HEADLESS: &str = "headless";
const WARM_MODE_WINDOWED: &str = "windowed";

/// Prefix of the two stderr marker lines the addon's `scenario.run` handler
/// prints around every warm scenario: `<prefix> begin <run-id>` before the
/// executor runs and `<prefix> end <run-id>` after it returns. A pooled warm
/// session writes ONE long-lived `logs.txt` for all its scenarios, so these
/// markers are what lets the CLI cut out the window that belongs to one run and
/// materialize it as that run's `<run_root>/logs.txt` — giving warm the same
/// `data.logs` / `godot_log_error` / `godot_log_warning` reporting as cold.
/// Must stay byte-identical to WARM_LOG_MARKER_PREFIX in
/// `addons/godot_qa/qa_bridge.gd`.
const WARM_LOG_MARKER_PREFIX: &str = "[godot-qa] scenario-log";

/// `data.warm` keys carrying the engine-log evidence for a warm run.
const WARM_SESSION_ID_KEY: &str = "session_id";
const WARM_LOG_SLICE_KEY: &str = "log_slice";

/// The audio-flavor label recorded on the `data.warm` annotation: `dummy` when
/// the session runs `--audio-driver Dummy` (no real device output), `real` when
/// it keeps the real audio driver (audio-capture scenarios).
const WARM_AUDIO_DUMMY: &str = "dummy";
const WARM_AUDIO_REAL: &str = "real";

fn warm_audio_flavor(audio: bool) -> &'static str {
    if audio {
        WARM_AUDIO_REAL
    } else {
        WARM_AUDIO_DUMMY
    }
}

/// Entry point for `scenario bundle run --warm`. Starts the shared warm session
/// (headless), runs the bundle with a runner that prefers warm execution per
/// scenario — lazily switching to a windowed session for windowed-media
/// scenarios — then stops the session. Falls back to the cold bundle if the
/// session cannot start.
pub fn run_scenario_bundle_warm(
    config: &ProjectConfig,
    name: &str,
    options: &crate::bundle::ScenarioBundleRunOptions,
) -> CommandResult {
    let command = "scenario bundle run";
    // Skip warm entirely when the bundle has no more scenarios than parallel
    // lanes. With count <= resolved parallelism the pool would start one fresh
    // session per scenario (one Godot boot each, exactly like cold) and NEVER
    // reuse a session across scenarios — so warm just pays the cold boot cost
    // plus per-scenario bridge overhead and is strictly slower. Run the
    // already-parallel cold path instead.
    //
    // The skip must NOT let --warm-strict off the hook: strict has to give the
    // SAME verdict on any host core count (a 12-core host that skips warm can't
    // quietly pass a bundle a 1-job CI would fail). Every strict-relevant warm
    // violation is STRUCTURAL — computable from the scenario document alone, with
    // no warm session — so under --warm-strict the skip still classifies every
    // scenario and appends the same `warm_fallback_forced_cold` failure a real
    // warm session would for a structural violation (a declared
    // `execution.isolation: cold` opt-out stays exempt), all while the bundle
    // keeps running the parallel cold path — jobs are NOT forced to 1 and nothing
    // hard-errors (see run_scenario_bundle_skipping_warm). Never applied to the
    // equivalence-check entry point, which needs the warm leg to compare.
    let prepared = crate::bundle::prepare_bundle_run(
        config,
        name,
        options,
        &mut crate::bundle::BundlePreparation::new(config),
    );
    if prepared
        .as_ref()
        .is_ok_and(|prepared| skip_warm_below_parallelism(options.jobs, prepared.runnable_count()))
    {
        return run_scenario_bundle_skipping_warm(config, prepared, options);
    }
    // Unresolved preparation cannot justify skipping warm. Keep the existing strict
    // startup failure precedence; successful startup still consumes the prepared error
    // through the runner so its post-run hook tears down the pool.
    // Eagerly start ONE headless pool session first: it both (a) lets --warm-strict
    // fail fast on a dead session and (b) fixes the session fixed_fps used to
    // classify every scenario. The rest of the pool (up to K sessions) starts
    // lazily as concurrent workers demand headless sessions.
    let seed_started = Instant::now();
    let seed = match start_warm_pool_session(config, true, false) {
        Ok(seed) => seed,
        Err(detail) => {
            // Under --warm-strict a dead warm session must fail loud and fast:
            // running the whole bundle cold is exactly what strict mode guards
            // against, so don't pay for it first.
            if options.warm_strict {
                return warm_session_start_failed_result(command, &detail);
            }
            // Warm session could not start — run the whole bundle cold so --warm is
            // never worse than a plain bundle run. The failure rides along instead
            // of being discarded: a bundle that silently paid for an all-cold run
            // must say WHY.
            let mut cold = crate::bundle::run_prepared_scenario_bundle(config, prepared, options);
            cold.data
                .insert("warm".to_string(), warm_session_start_failed_json(&detail));
            return cold;
        }
    };
    // The seed's boot is the pool's own measurement of what a session costs, and
    // therefore of how long waiting for a busy one can possibly be worth it.
    let seed_boot = seed_started.elapsed();

    let warm_strict = options.warm_strict;
    let pool = Arc::new(WarmSessionPool::new(
        seed,
        warm_pool_max(options),
        Some(seed_boot),
    ));
    let runner_pool = Arc::clone(&pool);
    let post_run_pool = Arc::clone(&pool);
    let mut result = crate::bundle::run_prepared_scenario_bundle_with_runner(
        config,
        prepared,
        options,
        move |config, scenario_path, scenario, command_line, headless_override, audio_override| {
            warm_or_cold_scenario(
                config,
                scenario_path,
                scenario,
                command_line,
                headless_override,
                audio_override,
                &runner_pool,
                warm_strict,
            )
        },
        // The pool is torn down INSIDE the bundle runner (see BundlePostRun), so
        // the shutdown cost and every pool failure land in summary.json/meta.json
        // and the CLI JSON instead of happening after the bundle reported.
        move || warm_pool_post_run(config, &post_run_pool),
    );

    let mut warm = warm_status_json(true, "warm");
    copy_warm_pool_summary(&mut warm, result.data.get("warm_pool"));
    result.data.insert("warm".to_string(), warm);
    result
}

/// Entry point for `scenario bundle run <a> <b> [...] --warm`: several bundles over ONE
/// warm pool, under one envelope.
///
/// The whole reason to name several gates in one invocation is that they then share the
/// expensive process-level resource. Running them as N invocations pays N seeds and N
/// shutdowns, and — worse — a small gate (six scenarios on a twelve-lane host) takes the
/// below-parallelism skip and runs cold every time, while the same six scenarios dispatched
/// onto an already-live pool run warm. So every decision that was per bundle becomes per
/// invocation here: one skip decision on the SUMMED runnable count, one seed, one pool, one
/// shutdown, one session prune.
///
/// What stays per bundle is everything a bundle owns: its own children, its own artifacts,
/// its own verdict. Each bundle writes exactly the run directory it writes today, and its
/// `data.warm` says it ran warm — but NOT the pool counters, because it did not start or
/// stop those sessions. The envelope carries them once (see
/// [`crate::bundle_multi::aggregate_bundle_results`]), which is also why every bundle here
/// runs with `prune_pool_sessions: false` and the driver prunes sessions itself afterwards.
pub fn run_scenario_bundles_warm(
    config: &ProjectConfig,
    names: &[String],
    options: &crate::bundle::ScenarioBundleRunOptions,
) -> CommandResult {
    let command = "scenario bundle run";
    let started = Instant::now();
    // Resolve every name before anything runs or boots: a typo in the third gate must not
    // cost a pool seed, and it must be reported alongside a typo in the first.
    let prepared = match crate::bundle_multi::prepare_bundles(config, names, options) {
        Ok(prepared) => prepared,
        Err(result) => return result,
    };
    // Sessions are the invocation's, not any bundle's, so no bundle prunes them.
    let child_options = crate::bundle::ScenarioBundleRunOptions {
        prune_pool_sessions: false,
        ..options.clone()
    };

    // ONE skip decision for the invocation. Deciding per bundle would defeat the point:
    // three six-scenario gates on a twelve-lane host are eighteen scenarios sharing twelve
    // lanes, which reuses sessions, even though each gate on its own would not.
    let runnable_counts: Vec<Option<usize>> = prepared
        .iter()
        .map(|prepared| {
            prepared
                .as_ref()
                .ok()
                .map(|prepared| prepared.runnable_count())
        })
        .collect();
    if let Some(runnable_total) = multi_bundle_warm_skip_total(options.jobs, &runnable_counts) {
        let results: Vec<CommandResult> = prepared
            .into_iter()
            .map(|prepared| run_scenario_bundle_skipping_warm(config, prepared, &child_options))
            .collect();
        let mut warm = warm_skipped_below_parallelism_json(options.warm_strict);
        if let Some(warm) = warm.as_object_mut() {
            // The summed count is the evidence for a decision no single bundle's own
            // `runnable_count` explains.
            warm.insert("runnable_count_total".to_string(), json!(runnable_total));
        }
        return finish_multi_bundle_warm(
            config,
            names,
            results,
            MultiBundleWarmEvidence {
                warm: Some(warm),
                started: Some(started),
                ..MultiBundleWarmEvidence::default()
            },
        );
    }

    let seed_started = Instant::now();
    let seed = match start_warm_pool_session(config, true, false) {
        Ok(seed) => seed,
        Err(detail) => {
            // Same rule as a single bundle, applied once: strict fails fast rather than
            // paying for an all-cold invocation, and non-strict runs every bundle cold with
            // the cause attached ONCE instead of repeated under each bundle.
            if options.warm_strict {
                return warm_session_start_failed_result(command, &detail);
            }
            let results: Vec<CommandResult> = prepared
                .into_iter()
                .map(|prepared| {
                    crate::bundle::run_prepared_scenario_bundle(config, prepared, &child_options)
                })
                .collect();
            return finish_multi_bundle_warm(
                config,
                names,
                results,
                MultiBundleWarmEvidence {
                    warm: Some(warm_session_start_failed_json(&detail)),
                    started: Some(started),
                    ..MultiBundleWarmEvidence::default()
                },
            );
        }
    };
    let seed_boot = seed_started.elapsed();

    let warm_strict = options.warm_strict;
    let pool = Arc::new(WarmSessionPool::new(
        seed,
        warm_pool_max(options),
        Some(seed_boot),
    ));
    let results: Vec<CommandResult> = prepared
        .into_iter()
        .map(|prepared| {
            let runner_pool = Arc::clone(&pool);
            let mut result = crate::bundle::run_prepared_scenario_bundle_with_runner(
                config,
                prepared,
                &child_options,
                move |config,
                      scenario_path,
                      scenario,
                      command_line,
                      headless_override,
                      audio_override| {
                    warm_or_cold_scenario(
                        config,
                        scenario_path,
                        scenario,
                        command_line,
                        headless_override,
                        audio_override,
                        &runner_pool,
                        warm_strict,
                    )
                },
                // The pool outlives this bundle, so it is NOT torn down here: the driver
                // shuts it down once, after the last bundle. A bundle's summary.json
                // therefore reports no pool timing and no pool counters, which is the
                // truth — it neither started nor stopped those sessions.
                crate::bundle::BundlePostRun::default,
            );
            result
                .data
                .insert("warm".to_string(), warm_status_json(true, "warm"));
            result
        })
        .collect();

    // One shutdown, one scan, one set of pool warnings — after every bundle is done.
    let post_run = warm_pool_post_run(config, &pool);
    let warm_pool = post_run.data.get("warm_pool").cloned();
    let mut warm = warm_status_json(true, "warm");
    copy_warm_pool_summary(&mut warm, warm_pool.as_ref());
    finish_multi_bundle_warm(
        config,
        names,
        results,
        MultiBundleWarmEvidence {
            warm: Some(warm),
            warm_pool,
            warnings: post_run.warnings,
            started: Some(started),
            pool_seed_ms: Some(duration_millis_u64(seed_boot)),
            pool_shutdown_ms: pool_shutdown_millis(&post_run.data),
        },
    )
}

/// What a multi-bundle warm invocation learned that no single bundle owns, on its way to the
/// envelope. Separate from [`crate::bundle_multi::MultiBundleEnvelope`] only because the
/// retention pass and the wall clock are finished by [`finish_multi_bundle_warm`] itself.
#[derive(Default)]
struct MultiBundleWarmEvidence {
    warm: Option<Value>,
    warm_pool: Option<Value>,
    warnings: Vec<FailureDetail>,
    started: Option<Instant>,
    pool_seed_ms: Option<u64>,
    pool_shutdown_ms: Option<u64>,
}

/// Run the invocation-level retention pass and fold everything into one envelope.
///
/// Session retention runs here — after the pool is down and every bundle has written its
/// artifacts — because the cutoff needs the oldest retained run across ALL the bundles. A
/// prune error rides along as the same `artifact_prune_failed` warning a single bundle
/// reports, and never changes the verdict.
fn finish_multi_bundle_warm(
    config: &ProjectConfig,
    names: &[String],
    results: Vec<CommandResult>,
    evidence: MultiBundleWarmEvidence,
) -> CommandResult {
    let retention = crate::bundle::apply_multi_bundle_session_retention(config, names);
    let mut warnings = evidence.warnings;
    warnings.extend(crate::bundle::bundle_retention_warning(
        config,
        retention.as_ref(),
    ));
    let envelope = crate::bundle_multi::MultiBundleEnvelope {
        warm: evidence.warm,
        warm_pool: evidence.warm_pool,
        pool_warnings: warnings
            .iter()
            .filter_map(|warning| serde_json::to_value(warning).ok())
            .collect(),
        retention,
        wall_ms: evidence
            .started
            .map(|started| duration_millis_u64(started.elapsed()))
            .unwrap_or_default(),
        pool_seed_ms: evidence.pool_seed_ms,
        pool_shutdown_ms: evidence.pool_shutdown_ms,
    };
    crate::bundle_multi::aggregate_bundle_results(names, results, envelope)
}

/// The runnable-scenario total a multi-bundle invocation should SKIP warm for, or `None`
/// when it should run warm.
///
/// `skip_warm_below_parallelism` asks whether every scenario would get its own fresh session
/// (in which case warm is pure overhead); across several bundles that question is about the
/// sum, because one pool serves all of them. A bundle whose count cannot be resolved makes
/// the sum unknowable, and an unknown sum must never justify skipping: the normal path runs,
/// which is where that bundle's real failure gets reported.
fn multi_bundle_warm_skip_total(jobs: i64, runnable_counts: &[Option<usize>]) -> Option<usize> {
    let total: usize = runnable_counts.iter().copied().sum::<Option<usize>>()?;
    skip_warm_below_parallelism(jobs, total).then_some(total)
}

fn pool_shutdown_millis(post_run_data: &Map<String, Value>) -> Option<u64> {
    let shutdown_ms = post_run_data
        .get("warm_pool")?
        .get("shutdown_ms")?
        .as_f64()?;
    Some(shutdown_ms.max(0.0).round() as u64)
}

/// Entry point for `scenario bundle run --warm --equivalence-check`. Runs each
/// scenario BOTH cold (authoritative) and warm, compares the logical verdict,
/// and returns the cold bundle result annotated with an `equivalence` block.
/// Exits non-zero if any scenario diverges, while the gate pass/fail stays the
/// cold result.
pub fn run_scenario_bundle_warm_equivalence(
    config: &ProjectConfig,
    name: &str,
    options: &crate::bundle::ScenarioBundleRunOptions,
) -> CommandResult {
    let prepared = crate::bundle::prepare_bundle_run(
        config,
        name,
        options,
        &mut crate::bundle::BundlePreparation::new(config),
    );
    let seed_started = Instant::now();
    let seed = start_warm_pool_session(config, true, false);
    let seed_boot = seed_started.elapsed();
    let warm_available = seed.is_ok();
    if let Err(detail) = &seed {
        if options.warm_strict {
            // Same rule as the plain --warm entry point: strict means a dead warm
            // session fails fast instead of silently degrading to cold-only runs
            // (which here would also make the equivalence check vacuous).
            return warm_session_start_failed_result("scenario bundle run", detail);
        }
    }
    let pool = seed.ok().map(|seed| {
        Arc::new(WarmSessionPool::new(
            seed,
            warm_pool_max(options),
            Some(seed_boot),
        ))
    });
    let session_fps = pool
        .as_ref()
        .map(|pool| pool.session_fps)
        .unwrap_or_else(|| project_default_fixed_fps(config));

    let warm_strict = options.warm_strict;
    let divergences: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let collector = Arc::clone(&divergences);
    let runner_pool = pool.clone();
    let post_run_pool = pool.clone();

    let mut result = crate::bundle::run_prepared_scenario_bundle_with_runner(
        config,
        prepared,
        options,
        move |config, scenario_path, scenario, command_line, headless_override, audio_override| {
            // Cold is authoritative for the gate.
            let mut cold = run_scenario_once(
                config,
                scenario_path,
                scenario,
                command_line,
                headless_override,
                audio_override,
                VisualRiskExecutionOptions::default(),
            );
            if let Some(runner_pool) = &runner_pool {
                match classify_warm_bucket(
                    scenario,
                    &config.project_root,
                    session_fps,
                    headless_override.or_else(|| config.launch_mode_headless_override()),
                    audio_override.or_else(|| config.audio_mode_override()),
                ) {
                    // A truly-cold scenario has no warm leg to compare, so skip
                    // the redundant second cold process and annotate the gate
                    // result with the reason directly.
                    WarmBucket::TrulyCold { reason, .. } => {
                        annotate_warm_result(
                            &mut cold,
                            false,
                            None,
                            None,
                            Some(reason),
                            warm_strict,
                        );
                    }
                    WarmBucket::Warm { .. } => {
                        // The warm leg runs with strict off so its failure set
                        // stays clean for the equivalence compare; strict is
                        // applied to the gate result AFTER comparing, else the
                        // injected failure would read as a false divergence.
                        let warm = warm_or_cold_scenario(
                            config,
                            scenario_path,
                            scenario,
                            command_line,
                            headless_override,
                            audio_override,
                            runner_pool,
                            false,
                        );
                        if let Some(divergence) = compare_equivalence(scenario, &cold, &warm) {
                            if let Ok(mut guard) = collector.lock() {
                                guard.push(divergence);
                            }
                        }
                        let (ran_warm, mode, audio_flavor, reason) = warm_annotation_of(&warm);
                        annotate_warm_result(
                            &mut cold,
                            ran_warm,
                            mode.as_deref(),
                            audio_flavor.as_deref(),
                            reason.as_deref(),
                            warm_strict,
                        );
                    }
                }
                repersist_annotated_result(&cold);
            }
            cold
        },
        // The warm legs here run with warm_strict = false, but the pool's own
        // start/transport failures are infrastructure evidence either way and must
        // still be drained into the bundle result.
        move || match &post_run_pool {
            Some(pool) => warm_pool_post_run(config, pool),
            None => crate::bundle::BundlePostRun::default(),
        },
    );

    let divergences = divergences.lock().map(|g| g.clone()).unwrap_or_default();
    let equivalent = warm_available && divergences.is_empty();
    result.data.insert(
        "equivalence".to_string(),
        json!({
            "checked": warm_available,
            "equivalent": equivalent,
            "divergences": divergences,
        }),
    );
    if warm_available && !divergences.is_empty() {
        // Cold gate result is unchanged, but the equivalence CHECK fails.
        result.ok = false;
        result.exit_code = 1;
        result.message = format!("{} (warm/cold equivalence diverged)", result.message);
    }
    result
}

/// Run one scenario warm if it is warm-eligible, else cold. Headless-warm
/// scenarios use the shared headless session; windowed-warm (windowed-media)
/// scenarios use a shared windowed session, started lazily. A warm round-trip
/// that fails for session/transport reasons falls back to cold. Every path
/// annotates the returned result's `data.warm` with whether it ran warm, in
/// which mode, and — if not — why; and, when `warm_strict` is set, turns a
/// non-exempt cold fallback into a hard failure.
#[allow(clippy::too_many_arguments)]
fn warm_or_cold_scenario(
    config: &ProjectConfig,
    scenario_path: &Path,
    scenario: &Value,
    command_line: &str,
    headless_override: Option<bool>,
    audio_override: Option<bool>,
    pool: &WarmSessionPool,
    warm_strict: bool,
) -> CommandResult {
    match classify_warm_bucket(
        scenario,
        &config.project_root,
        pool.session_fps,
        headless_override.or_else(|| config.launch_mode_headless_override()),
        audio_override.or_else(|| config.audio_mode_override()),
    ) {
        WarmBucket::TrulyCold { reason, .. } => cold_fallback_result(
            config,
            scenario_path,
            scenario,
            command_line,
            headless_override,
            audio_override,
            reason,
            warm_strict,
        ),
        // Headless-display flavor -> the pooled headless lane for its audio flavor.
        WarmBucket::Warm {
            headless: true,
            audio,
        } => warm_headless_scenario(
            config,
            scenario_path,
            scenario,
            command_line,
            headless_override,
            audio_override,
            audio,
            pool,
            warm_strict,
        ),
        // Windowed-display flavor -> the single serialized windowed slot.
        WarmBucket::Warm {
            headless: false,
            audio,
        } => warm_windowed_scenario(
            config,
            scenario_path,
            scenario,
            command_line,
            headless_override,
            audio_override,
            audio,
            pool,
            warm_strict,
        ),
    }
}

fn millis_since(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

fn millis_of(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn scenario_name_of(scenario: &Value) -> &str {
    scenario
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn warm_status_json(session_started: bool, mode: &str) -> Value {
    json!({
        "enabled": true,
        "session_started": session_started,
        "mode": mode,
    })
}

/// Decide whether a bundle of `count` runnable scenarios run at `jobs` parallelism
/// should skip warm and take the parallel cold path. True when resolved parallelism
/// is >= the scenario count: every scenario gets its own lane, so no warm session is
/// ever reused across scenarios (warm would be one boot per scenario, same as cold,
/// plus bridge overhead). The threshold is derived purely from parallelism — there
/// is no tunable magic number. `--jobs 1` with count > 1 keeps warm (a single session
/// reused serially), and a single-scenario bundle always runs cold.
fn skip_warm_below_parallelism(jobs: i64, count: usize) -> bool {
    crate::bundle::resolve_bundle_parallelism(jobs, count).resolved_jobs >= count
}

/// The per-run `warm.reason` for a scenario that WOULD have run warm but did not,
/// because the bundle skipped warm below parallelism. It describes the host's core
/// count rather than anything about the scenario, so it is strict-exempt.
const WARM_SKIPPED_BELOW_PARALLELISM_REASON: &str = "warm_skipped_below_parallelism";

/// Run a bundle whose warm was skipped because it has no more scenarios than
/// parallel lanes (see `skip_warm_below_parallelism`). Always runs the
/// already-parallel COLD path; the extra work is the per-run `warm` annotation
/// and, under `--warm-strict`, enforcement.
///
/// Every child is annotated on BOTH paths. A `warm` of `null` was indistinguishable
/// from "warm was never requested", which is exactly the question asked of a gate
/// that silently skipped warm — so a warm-eligible scenario now says
/// `warm_skipped_below_parallelism` and a truly-cold one says its real reason.
///
/// Under `--warm-strict` the skip must not change the verdict versus a host that
/// actually runs warm, so every scenario is classified with the SAME
/// `classify_warm_bucket` the non-skip path uses and every structural cold
/// fallback becomes the SAME `warm_fallback_forced_cold` failure (a declared
/// `execution.isolation: cold` opt-out stays exempt, exactly as on the non-skip
/// path). The bundle still runs on the parallel cold path — jobs are NOT forced
/// to 1 and nothing hard-errors.
fn run_scenario_bundle_skipping_warm(
    config: &ProjectConfig,
    prepared: Result<crate::bundle::PreparedBundleRun, CommandResult>,
    options: &crate::bundle::ScenarioBundleRunOptions,
) -> CommandResult {
    let warm_strict = options.warm_strict;
    // A skipped run starts no warm session, so classify against the fixed_fps a
    // warm session WOULD have booted with (project-default deterministic pacing) —
    // identical to the equivalence path's no-pool fallback — so `fixed_fps_mismatch`
    // is judged the same way as on the non-skip path.
    let session_fps = project_default_fixed_fps(config);
    let mut cold = crate::bundle::run_prepared_scenario_bundle_with_runner(
        config,
        prepared,
        options,
        move |config, scenario_path, scenario, command_line, headless_override, audio_override| {
            skip_cold_scenario(
                config,
                scenario_path,
                scenario,
                command_line,
                headless_override,
                audio_override,
                session_fps,
                warm_strict,
            )
        },
        // No warm pool was ever started on the skip path, so there is nothing
        // to tear down or report.
        crate::bundle::BundlePostRun::default,
    );
    cold.data.insert(
        "warm".to_string(),
        warm_skipped_below_parallelism_json(warm_strict),
    );
    cold
}

/// One scenario of a warm-skipped bundle: run it cold and record WHY it ran cold.
///
/// A structurally or declaredly cold scenario is annotated with its real reason and,
/// under `--warm-strict`, gets the same `warm_fallback_forced_cold` failure the
/// non-skip warm path would. A warm-ELIGIBLE scenario — one that would have run warm
/// on a smaller host — is annotated `warm_skipped_below_parallelism`, which
/// `reason_is_strict_exempt` treats as exempt, so strict still sees no violation.
/// Both cases route through `cold_fallback_result`, so the persisted `result.json`
/// is re-written to match the summary exactly as everywhere else.
#[allow(clippy::too_many_arguments)]
fn skip_cold_scenario(
    config: &ProjectConfig,
    scenario_path: &Path,
    scenario: &Value,
    command_line: &str,
    headless_override: Option<bool>,
    audio_override: Option<bool>,
    session_fps: i64,
    warm_strict: bool,
) -> CommandResult {
    let reason = skip_strict_cold_reason(
        scenario,
        &config.project_root,
        session_fps,
        headless_override.or_else(|| config.launch_mode_headless_override()),
        audio_override.or_else(|| config.audio_mode_override()),
    )
    .unwrap_or(WARM_SKIPPED_BELOW_PARALLELISM_REASON);
    cold_fallback_result(
        config,
        scenario_path,
        scenario,
        command_line,
        headless_override,
        audio_override,
        reason,
        warm_strict,
    )
}

/// The cold-fallback reason a warm-skipped scenario should be annotated with, or
/// None when it is warm-eligible (no annotation). Truly-cold scenarios (declared
/// or structural) return their reason; `cold_fallback_result` /
/// `reason_is_strict_exempt` then decide whether that reason is a strict
/// violation, identical to the non-skip warm path.
fn skip_strict_cold_reason(
    scenario: &Value,
    project_root: &Path,
    session_fps: i64,
    headless_override: Option<bool>,
    audio_override: Option<bool>,
) -> Option<&'static str> {
    match classify_warm_bucket(
        scenario,
        project_root,
        session_fps,
        headless_override,
        audio_override,
    ) {
        WarmBucket::TrulyCold { reason, .. } => Some(reason),
        WarmBucket::Warm { .. } => None,
    }
}

/// Bundle-level `data.warm` when warm was skipped because the bundle has no more
/// scenarios than parallel lanes: with count <= resolved parallelism every
/// scenario gets its own fresh session (one Godot boot each, same as cold) so no
/// session is ever reused across scenarios and warm only adds bridge overhead.
/// The whole bundle ran on the already-parallel cold path instead. Under
/// `--warm-strict` the skip still enforced strict structurally (every scenario
/// classified, structural violations appended as `warm_fallback_forced_cold`), so
/// the annotation gains `strict_structural_check: true` to show the verdict is
/// host-independent; the field is absent otherwise.
fn warm_skipped_below_parallelism_json(warm_strict: bool) -> Value {
    let mut annotation = Map::from_iter([
        ("enabled".to_string(), json!(true)),
        ("session_started".to_string(), json!(false)),
        ("ran_warm".to_string(), json!(false)),
        ("mode".to_string(), Value::Null),
        ("reason".to_string(), json!("no_reuse_below_parallelism")),
    ]);
    if warm_strict {
        annotation.insert("strict_structural_check".to_string(), json!(true));
    }
    Value::Object(annotation)
}

// Implementation chunks, included into this same `crate::run` scope (see the
// module-size rule in AGENTS.md). Paths resolve relative to this file.
include!("warm/pool.rs");
include!("warm/dispatch.rs");
include!("warm/classify.rs");
include!("warm/logs.rs");
include!("warm/equivalence.rs");

#[cfg(test)]
mod warm_tests {
    use super::*;

    include!("warm/tests_classify.rs");
    include!("warm/tests_pool.rs");
    include!("warm/tests_logs.rs");
    include!("warm/tests_equivalence.rs");
}
