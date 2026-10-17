// Warm session pool: the K independently-addressed pooled sessions a warm bundle
// runs its scenarios on, plus the post-run hook that reports their evidence.
//
// Included into `run/warm.rs`, which is itself included into the run module, so
// this file shares the whole `crate::run` scope.

/// Start ONE independent pooled warm session (its own process/port/token/artifact
/// dir; never the singleton `.qa/session.json`). Sceneless boot + project-default
/// deterministic frame pacing, exactly like the historical single warm session —
/// only now K of them can coexist. Returns the handle, or the startup
/// `FailureDetail` (the caller then runs the scenario cold AND keeps the failure:
/// a discarded start failure is exactly how a warm bundle used to degrade to
/// half-cold while reporting `warnings: []`).
///
/// scene = Some("") -> boot the warm session SCENELESS: the per-scenario
/// QaScenarioExecutor (driven by scenario.run) owns the full scene lifecycle, so
/// the session must NOT also auto-boot the project main scene. A live main scene
/// alongside the scenario scene would react to the same injected input a
/// drive_policy step produces (spawning phantom matches, clobbering QA provider
/// registrations), diverging warm from cold.
fn start_warm_pool_session(
    config: &ProjectConfig,
    headless: bool,
    audio: bool,
) -> Result<godot_qa_runtime::PooledSessionHandle, FailureDetail> {
    godot_qa_runtime::start_pooled_session(
        config,
        Some(""),
        WARM_SESSION_VIEWPORT,
        headless,
        audio,
        &[],
        // frame_pacing_request = Some(None) -> project-default deterministic pacing.
        Some(None),
    )
}

/// The pool ceiling: the bundle's resolved parallelism (the same value the cold
/// bundle runner uses for its worker count). At most this many warm scenarios run
/// concurrently, so at most this many pooled headless sessions are ever needed.
/// `--jobs 1` collapses this to 1 (serial warm on a single reused session).
fn warm_pool_max(options: &crate::bundle::ScenarioBundleRunOptions) -> usize {
    // runnable_count = usize::MAX so resolved_jobs reflects the requested worker
    // count independent of the (not-yet-expanded) scenario count; the bundle
    // runner separately caps its worker count at the real task count, and the pool
    // only starts sessions lazily on demand, so an over-estimate costs nothing.
    crate::bundle::resolve_bundle_parallelism(options.jobs, usize::MAX)
        .resolved_jobs
        .max(1)
}

/// A pool of independent warm sessions, sized to the bundle's resolved
/// parallelism. Headless-warm scenarios are distributed across up to K concurrent
/// headless sessions; a single serialized windowed session hosts windowed-media
/// scenarios (running multiple windowed Godot processes at once is display-risky,
/// and windowed-media scenarios are rare). Every pooled session is a separate
/// process, so there is no cross-session state leak — each scenario still gets its
/// own project_args (seed/fixture/roster) via the fidelity fix.
struct WarmSessionPool {
    /// The fixed_fps shared by every headless pool session (all boot sceneless
    /// with project-default pacing), used to classify each scenario's warm bucket.
    session_fps: i64,
    max: usize,
    /// How long a worker waits for a busy headless lane before running cold.
    acquire_wait: Duration,
    /// The seed session's measured boot, reported as bundle evidence.
    seed_boot_ms: Option<f64>,
    /// Headless-display sessions are keyed by audio flavor: `headless_dummy` is
    /// the common `--audio-driver Dummy` lane (seeded eagerly), `headless_real`
    /// the rare real-audio lane (audio-capture scenarios that need no window).
    headless_dummy: HeadlessLane,
    headless_real: HeadlessLane,
    windowed: Mutex<WindowedSlotInner>,
    /// Sessions this pool successfully started (seed + lazy headless + windowed).
    sessions_started: AtomicUsize,
    /// Where each of those sessions writes its engine log. Kept separately from
    /// the live handles because `shutdown` consumes those, and a session that was
    /// discarded mid-bundle no longer has one at all — yet its log still holds
    /// output this bundle produced. Desktop sessions only (nothing else has a
    /// host-side log to read).
    sessions: Mutex<Vec<WarmPooledSessionRef>>,
    /// Every failed session start, as JSON evidence for the bundle result.
    start_failures: Mutex<Vec<Value>>,
    /// Every scenario round-trip that failed on a pooled session and retired it.
    transport_failures: Mutex<Vec<Value>>,
    /// Bundle-level warnings raised while slicing warm engine logs. Today only
    /// `warm_log_markers_missing` (an addon older than the scenario-log markers),
    /// recorded ONCE per bundle: the condition is a property of the addon, so one
    /// warning per scenario would just be the same sentence 134 times.
    log_warnings: Mutex<Vec<FailureDetail>>,
    /// `shutdown` is idempotent: the post-run hook owns it, and nothing may stop
    /// the same sessions twice.
    shutdown_started: AtomicBool,
    shutdown_ms: Mutex<Option<f64>>,
}

/// One headless pool lane: a mutex-guarded free list plus the condvar that EVERY
/// state change notifies. The condvar is what lets a worker wait briefly for a
/// session to come free instead of immediately paying a ~2.5 s cold boot — a lane
/// at capacity is a transient condition (someone is mid-scenario), not a permanent
/// one, and the old code treated it as permanent.
struct HeadlessLane {
    inner: Mutex<HeadlessPoolInner>,
    changed: Condvar,
}

struct HeadlessPoolInner {
    /// Live, currently-free headless sessions ready to be checked out.
    idle: Vec<godot_qa_runtime::PooledSessionHandle>,
    /// Count of live headless sessions (idle + checked-out). Used to cap at `max`.
    started: usize,
    /// CONSECUTIVE failed starts in this lane, reset to zero by any success. Only
    /// `WARM_START_FAILURE_BUDGET` in a row stop the lane booting. A DISCARDED
    /// (wedged) session deliberately does not count here: retiring a dead session
    /// must never block its replacement.
    consecutive_start_failures: u32,
}

/// What an `acquire_headless` caller should do next, decided purely from the pool
/// state (extracted so it is unit-testable without launching real sessions).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HeadlessAcquire {
    /// Reuse a checked-in idle session (no boot cost).
    ReuseIdle,
    /// Boot a new session (under `max`, failure budget not exhausted).
    StartNew,
    /// Nothing free right now, but the lane holds live sessions that a peer worker
    /// will hand back — block on the condvar until one does or the deadline passes.
    Wait,
    /// The lane holds nothing that could ever come free (no live sessions, and no
    /// budget left to start one) — the caller runs the scenario cold.
    Unavailable,
}

fn headless_acquire_decision(inner: &HeadlessPoolInner, max: usize) -> HeadlessAcquire {
    if !inner.idle.is_empty() {
        HeadlessAcquire::ReuseIdle
    } else if inner.started < max && inner.consecutive_start_failures < WARM_START_FAILURE_BUDGET {
        HeadlessAcquire::StartNew
    } else if inner.started > 0 {
        HeadlessAcquire::Wait
    } else {
        HeadlessAcquire::Unavailable
    }
}

struct WindowedSlotInner {
    session: Option<godot_qa_runtime::PooledSessionHandle>,
    /// The audio flavor of the currently-held windowed session. Windowed
    /// scenarios are serialized through this single slot; when the next one needs
    /// the OTHER audio flavor the held session is discarded and restarted (rare).
    audio: bool,
    /// The same consecutive-failure budget the headless lanes use. No condvar:
    /// windowed scenarios are serialized through this one slot, so there is never
    /// a second live windowed session to wait for.
    consecutive_start_failures: u32,
}

impl HeadlessLane {
    fn with_seed(seed: godot_qa_runtime::PooledSessionHandle) -> Self {
        HeadlessLane {
            inner: Mutex::new(HeadlessPoolInner {
                idle: vec![seed],
                started: 1,
                consecutive_start_failures: 0,
            }),
            changed: Condvar::new(),
        }
    }

    fn empty() -> Self {
        HeadlessLane {
            inner: Mutex::new(HeadlessPoolInner {
                idle: Vec::new(),
                started: 0,
                consecutive_start_failures: 0,
            }),
            changed: Condvar::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HeadlessPoolInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The per-scenario wait budget: a worker waits about as long as ONE session boot
/// for a free lane, because waiting longer than a boot can never beat running cold.
fn warm_acquire_wait(seed_boot: Option<Duration>) -> Duration {
    match seed_boot {
        Some(boot) => boot.clamp(WARM_ACQUIRE_WAIT_MIN, WARM_ACQUIRE_WAIT_MAX),
        None => WARM_ACQUIRE_WAIT_FALLBACK,
    }
}

/// The lane label recorded on pool failure evidence.
fn headless_lane_label(audio: bool) -> &'static str {
    if audio {
        "headless_real"
    } else {
        "headless_dummy"
    }
}

const WINDOWED_LANE_LABEL: &str = "windowed";

fn failure_detail_json(detail: &FailureDetail) -> Value {
    serde_json::to_value(detail).unwrap_or_else(|_| json!({ "type": detail.kind }))
}

impl WarmSessionPool {
    /// Build the pool seeded with the eagerly-started headless session (already
    /// live, checked in as idle). `session_fps` is taken from that session, and
    /// `seed_boot` (when measured) sets the pool's acquire-wait budget.
    fn new(
        seed: godot_qa_runtime::PooledSessionHandle,
        max: usize,
        seed_boot: Option<Duration>,
    ) -> Self {
        // A warm session always boots with project-default pacing (Some(None)), so
        // frame_pacing is set; the 60 fallback is only a last resort.
        let session_fps = seed.fixed_fps_or(60);
        // The seed is already started, so it is counted here rather than through
        // note_session_started (the lane owns the handle from this point on).
        let seed_session = WarmPooledSessionRef::of(&seed.record);
        WarmSessionPool {
            session_fps,
            max: max.max(1),
            acquire_wait: warm_acquire_wait(seed_boot),
            seed_boot_ms: seed_boot.map(|boot| boot.as_secs_f64() * 1000.0),
            // The eager seed is a headless + audio-off (Dummy) session, so it
            // lives in the common `headless_dummy` lane; the rare real-audio lane
            // starts empty and grows lazily only if an audio-capture scenario that
            // needs no window appears.
            headless_dummy: HeadlessLane::with_seed(seed),
            headless_real: HeadlessLane::empty(),
            windowed: Mutex::new(WindowedSlotInner {
                session: None,
                audio: false,
                consecutive_start_failures: 0,
            }),
            sessions_started: AtomicUsize::new(1),
            sessions: Mutex::new(seed_session.into_iter().collect()),
            start_failures: Mutex::new(Vec::new()),
            transport_failures: Mutex::new(Vec::new()),
            log_warnings: Mutex::new(Vec::new()),
            shutdown_started: AtomicBool::new(false),
            shutdown_ms: Mutex::new(None),
        }
    }

    /// The headless lane for the given audio flavor.
    fn headless_lane(&self, audio: bool) -> &HeadlessLane {
        if audio {
            &self.headless_real
        } else {
            &self.headless_dummy
        }
    }

    fn lock_headless(&self, audio: bool) -> std::sync::MutexGuard<'_, HeadlessPoolInner> {
        self.headless_lane(audio).lock()
    }

    fn lock_windowed(&self) -> std::sync::MutexGuard<'_, WindowedSlotInner> {
        self.windowed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn note_session_started(&self, record: &godot_qa_runtime::SessionRecord) {
        self.sessions_started.fetch_add(1, Ordering::Relaxed);
        if let Some(session) = WarmPooledSessionRef::of(record) {
            self.sessions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(session);
        }
    }

    fn sessions_snapshot(&self) -> Vec<WarmPooledSessionRef> {
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Keep a failed session start as bundle evidence. `lane` is which pool lane
    /// tried, `attempt` the 1-based try within that acquire call.
    fn record_start_failure(&self, lane: &str, attempt: u32, detail: &FailureDetail) {
        let mut entry = failure_detail_json(detail);
        if let Some(object) = entry.as_object_mut() {
            object.insert("lane".to_string(), json!(lane));
            object.insert("attempt".to_string(), json!(attempt));
        }
        self.start_failures
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(entry);
    }

    /// Keep a failed scenario round-trip as bundle evidence. Carries the scenario
    /// and session id so a reader can tell "one wedged session" from "the engine
    /// is broken".
    fn record_transport_failure(
        &self,
        lane: &str,
        scenario: &str,
        session_id: &str,
        detail: &FailureDetail,
    ) {
        let mut entry = failure_detail_json(detail);
        if let Some(object) = entry.as_object_mut() {
            object.insert("lane".to_string(), json!(lane));
            object.insert("scenario".to_string(), json!(scenario));
            object.insert("session_id".to_string(), json!(session_id));
        }
        self.transport_failures
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(entry);
    }

    /// Note that a warm scenario's session log carried no `scenario-log` markers,
    /// so its `<run_root>/logs.txt` could not be materialized and `data.logs`
    /// stayed `missing` exactly as it did before this feature. Recorded at most
    /// ONCE per bundle: the cause is the addon version, identical for every
    /// scenario on every lane.
    fn note_log_markers_missing(&self, session_id: &str, source: &str) {
        let mut guard = self
            .log_warnings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !guard.is_empty() {
            return;
        }
        guard.push(warm_log_markers_missing_warning(session_id, source));
    }

    fn log_warnings_snapshot(&self) -> Vec<FailureDetail> {
        self.log_warnings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn start_failures_snapshot(&self) -> Vec<Value> {
        self.start_failures
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn transport_failures_snapshot(&self) -> Vec<Value> {
        self.transport_failures
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// The pool's own report for the bundle JSON: what it started, what failed,
    /// and what its lifecycle cost.
    fn summary_json(&self) -> Value {
        let shutdown_ms = *self
            .shutdown_ms
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        json!({
            "sessions_started": self.sessions_started.load(Ordering::Relaxed),
            "session_start_failures": self.start_failures_snapshot(),
            "transport_failures": self.transport_failures_snapshot(),
            "seed_boot_ms": self.seed_boot_ms,
            "shutdown_ms": shutdown_ms,
            "acquire_wait_ms_budget": self.acquire_wait.as_secs_f64() * 1000.0,
        })
    }

    /// Production checkout of a free headless session: the pool's wait budget, real
    /// pooled sessions. Returns the handle plus how long the acquire took (boot or
    /// wait included), recorded per scenario as `data.warm.timing.acquire_wait_ms`.
    fn acquire_headless(
        &self,
        config: &ProjectConfig,
        audio: bool,
    ) -> Option<(godot_qa_runtime::PooledSessionHandle, Duration)> {
        let deadline = Instant::now() + self.acquire_wait;
        self.acquire_headless_with(audio, deadline, || {
            start_warm_pool_session(config, true, audio)
        })
    }

    /// Check out a free headless session of the given audio flavor.
    ///
    /// Preference order: an idle session (free), a freshly booted one (costs a
    /// boot, capped by `max` and the failure budget), or one a peer worker releases
    /// while we wait (free, and far cheaper than the cold boot the caller pays if
    /// we give up). Returns None only when the deadline passes or the lane holds
    /// nothing that could come free — the caller then runs cold with the
    /// strict-exempt `warm_session_unavailable` reason.
    ///
    /// The lane lock is NEVER held across a boot: the slot is reserved
    /// (`started += 1`), the lock dropped, the session booted, and the lock retaken
    /// to record the outcome. `start` is injected so this whole loop is testable
    /// without launching Godot.
    ///
    /// Deadlock argument: a waiter owns nothing, and every unit of `started` is
    /// owned by a worker that is NOT waiting — it will release, discard, or fail its
    /// start, and all three notify. `shutdown` runs only after every worker exits.
    fn acquire_headless_with<F>(
        &self,
        audio: bool,
        deadline: Instant,
        mut start: F,
    ) -> Option<(godot_qa_runtime::PooledSessionHandle, Duration)>
    where
        F: FnMut() -> Result<godot_qa_runtime::PooledSessionHandle, FailureDetail>,
    {
        let lane = self.headless_lane(audio);
        let acquire_started = Instant::now();
        let mut boot_attempts = 0u32;
        let mut inner = lane.lock();
        loop {
            let decision = match headless_acquire_decision(&inner, self.max) {
                // One acquire call boots at most BUDGET times; past that, let a peer
                // worker's release (or its boot) serve us instead of spinning.
                HeadlessAcquire::StartNew if boot_attempts >= WARM_START_FAILURE_BUDGET => {
                    if inner.started > 0 {
                        HeadlessAcquire::Wait
                    } else {
                        HeadlessAcquire::Unavailable
                    }
                }
                decision => decision,
            };
            match decision {
                HeadlessAcquire::ReuseIdle => {
                    let handle = inner
                        .idle
                        .pop()
                        .expect("idle is non-empty whenever the decision is ReuseIdle");
                    return Some((handle, acquire_started.elapsed()));
                }
                HeadlessAcquire::StartNew => {
                    // Reserve the slot BEFORE the slow boot so concurrent acquirers
                    // pick distinct slots and never over-provision past `max`.
                    inner.started += 1;
                    drop(inner);
                    boot_attempts += 1;
                    match start() {
                        Ok(handle) => {
                            lane.lock().consecutive_start_failures = 0;
                            self.note_session_started(&handle.record);
                            lane.changed.notify_all();
                            return Some((handle, acquire_started.elapsed()));
                        }
                        Err(detail) => {
                            {
                                let mut relocked = lane.lock();
                                relocked.started = relocked.started.saturating_sub(1);
                                relocked.consecutive_start_failures =
                                    relocked.consecutive_start_failures.saturating_add(1);
                            }
                            self.record_start_failure(
                                headless_lane_label(audio),
                                boot_attempts,
                                &detail,
                            );
                            lane.changed.notify_all();
                            inner = lane.lock();
                        }
                    }
                }
                HeadlessAcquire::Wait => {
                    let now = Instant::now();
                    if now >= deadline {
                        return None;
                    }
                    let (waited, _timed_out) = lane
                        .changed
                        .wait_timeout(inner, deadline - now)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    inner = waited;
                }
                HeadlessAcquire::Unavailable => return None,
            }
        }
    }

    /// Return a healthy headless session to its flavor's free list and wake a
    /// waiter. The notify lives INSIDE this method on purpose: a released session
    /// nobody is told about is exactly the missed transition that turns the bounded
    /// wait into a silent stall.
    fn release_headless(&self, handle: godot_qa_runtime::PooledSessionHandle, audio: bool) {
        let lane = self.headless_lane(audio);
        lane.lock().idle.push(handle);
        lane.changed.notify_all();
    }

    /// Retire a headless session that failed its round-trip (likely a dead/wedged
    /// process): free its slot — waking any waiter, which can then start the
    /// replacement — and stop it outside the lock.
    fn discard_headless(
        &self,
        config: &ProjectConfig,
        handle: godot_qa_runtime::PooledSessionHandle,
        audio: bool,
    ) {
        let lane = self.headless_lane(audio);
        {
            let mut inner = lane.lock();
            // Deliberately does NOT touch consecutive_start_failures: a wedged
            // session is not a failed boot, and counting it as one would let a
            // single bad session block every replacement for the rest of the run.
            inner.started = inner.started.saturating_sub(1);
        }
        lane.changed.notify_all();
        let _ = godot_qa_runtime::stop_pooled_session(config, &handle);
    }

    /// Stop every live pool session (idle headless in both audio lanes + the
    /// windowed slot), all at once. Called from the bundle runner's post-run hook
    /// after every worker has finished, so nothing is checked out and no orphaned
    /// Godot processes are left behind. Stopping 12 sessions serially cost ~1.9 s
    /// of pure wall clock at the end of a bundle; in parallel it is one stop.
    /// Idempotent — the hook owns it and must run it exactly once.
    fn shutdown(&self, config: &ProjectConfig) {
        if self.shutdown_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let started = Instant::now();
        let mut handles: Vec<godot_qa_runtime::PooledSessionHandle> = Vec::new();
        for audio in [false, true] {
            let mut inner = self.lock_headless(audio);
            handles.append(&mut inner.idle);
        }
        if let Some(handle) = self.lock_windowed().session.take() {
            handles.push(handle);
        }
        std::thread::scope(|scope| {
            for handle in &handles {
                scope.spawn(move || {
                    let _ = godot_qa_runtime::stop_pooled_session(config, handle);
                });
            }
        });
        *self
            .shutdown_ms
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(millis_since(started));
    }
}

/// The bundle runner's post-run hook for a warm bundle: shut the pool down (so its
/// cost lands inside the measured run and before `summary.json` is written), then
/// hand the bundle the pool's evidence — `data.warm_pool` plus one warning per
/// failed session start.
///
/// Those warnings are INFRASTRUCTURE: bundle `--strict` never promotes warnings to
/// failures, and `--warm-strict` stays exempt from session-start failures exactly as
/// documented. They exist so a bundle that quietly paid cold-boot time for a third
/// of its scenarios says so, instead of reporting `warnings: []`.
fn warm_pool_post_run(
    config: &ProjectConfig,
    pool: &WarmSessionPool,
) -> crate::bundle::BundlePostRun {
    pool.shutdown(config);
    let start_failures = pool.start_failures_snapshot();
    let mut warnings: Vec<FailureDetail> = start_failures
        .iter()
        .map(warm_session_start_warning)
        .collect();
    // Same rationale as the start failures: a bundle whose warm scenarios could
    // not be log-checked at all must say so instead of reporting `warnings: []`.
    warnings.extend(pool.log_warnings_snapshot());
    // Scanned AFTER shutdown: every session process is gone, so each log is
    // complete and includes the shutdown-time errors that fall after the last
    // scenario window and therefore reach no run's logs.txt. Without this they are
    // invisible — attributing them to a scenario would be wrong, but so is
    // dropping them silently.
    // The same project `logs.ignore` list the cold and warm per-scenario scans
    // apply, so one pattern configured once covers every place engine logs are read.
    let noise = scan_warm_session_log_noise(&pool.sessions_snapshot(), &config.log_ignore_patterns);
    warnings.extend(warm_session_log_noise_warning(&noise));
    let mut pool_summary = pool.summary_json();
    if let Some(pool_summary) = pool_summary.as_object_mut() {
        pool_summary.insert("session_log_noise".to_string(), noise.to_json());
    }
    let mut data = Map::new();
    data.insert("warm_pool".to_string(), pool_summary);
    crate::bundle::BundlePostRun { warnings, data }
}

/// The one bundle-level warning for an addon that predates the warm
/// `scenario-log` stderr markers: the CLI could not attribute engine output to
/// individual warm scenarios, so `data.logs` stayed `missing` on every warm run
/// (the pre-parity behavior) and engine errors were reported by nothing.
/// INFRASTRUCTURE, like the session-start warnings: `--strict` never promotes
/// warnings to failures and `--warm-strict` is unaffected.
fn warm_log_markers_missing_warning(session_id: &str, source: &str) -> FailureDetail {
    FailureDetail {
        kind: "warm_log_markers_missing".to_string(),
        command: Some("scenario bundle run".to_string()),
        path: Some(source.to_string()),
        message: Some(format!(
            "the warm session log carried no `{WARM_LOG_MARKER_PREFIX} begin|end <run-id>` \
             markers, so warm scenario logs could not be sliced out of it and `data.logs` \
             stays \"missing\" for warm runs; update the godot_qa addon in the project so \
             warm reports engine errors the same way cold does"
        )),
        expected: Some(json!(format!(
            "`{WARM_LOG_MARKER_PREFIX} begin|end <run-id>` markers in the warm session log"
        ))),
        actual: Some(json!("markers_missing")),
        details: Map::from_iter([("session_id".to_string(), json!(session_id))]),
        ..FailureDetail::default()
    }
}

/// One bundle-level warning per failed warm session start. `actual` keeps the
/// original failure kind and `details.cause` the whole failure, so the warning is
/// both scannable and complete.
fn warm_session_start_warning(failure: &Value) -> FailureDetail {
    let lane = failure
        .get("lane")
        .and_then(Value::as_str)
        .unwrap_or("headless_dummy");
    let cause = failure
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("no message");
    FailureDetail {
        kind: "warm_session_start_failed".to_string(),
        command: Some("scenario bundle run".to_string()),
        path: failure
            .get("session_artifact_dir")
            .or_else(|| failure.get("artifact_dir"))
            .and_then(Value::as_str)
            .map(str::to_string),
        message: Some(format!(
            "a warm pool session failed to start on the {lane} lane, so scenarios that \
             needed it ran cold: {cause}"
        )),
        expected: Some(json!("every warm pool session starts")),
        actual: failure.get("type").cloned(),
        details: Map::from_iter([("cause".to_string(), failure.clone())]),
        ..FailureDetail::default()
    }
}

/// Consumers read `data.warm`; the pool's evidence also lives at `data.warm_pool`.
/// Copy the fields a gate acts on into `data.warm` so an existing reader needs no
/// new lookup path.
fn copy_warm_pool_summary(warm: &mut Value, warm_pool: Option<&Value>) {
    let (Some(warm), Some(warm_pool)) = (warm.as_object_mut(), warm_pool) else {
        return;
    };
    for key in [
        "sessions_started",
        "session_start_failures",
        "transport_failures",
        "session_log_noise",
    ] {
        if let Some(value) = warm_pool.get(key) {
            warm.insert(key.to_string(), value.clone());
        }
    }
}
