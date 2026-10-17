// Warm engine-log parity: cutting one scenario's window out of a pooled
// session's long-lived `logs.txt` via the addon's marker lines, and scanning
// what falls between those windows for session noise.
//
// Included into `run/warm.rs`, which is itself included into the run module, so
// this file shares the whole `crate::run` scope.

/// One pooled session's engine log, remembered for the post-run gap scan.
///
/// Held by `artifact_dir`-derived path rather than by handle: `shutdown` consumes
/// the live handles and a session discarded after a transport failure has none at
/// all, yet both wrote output this bundle is accountable for.
#[derive(Debug, Clone)]
struct WarmPooledSessionRef {
    session_id: String,
    logs_path: PathBuf,
}

impl WarmPooledSessionRef {
    /// None for a non-desktop session: its engine output lives on the device, so
    /// there is no host-side log to scan (same gate as `warm_session_log_anchor`).
    fn of(record: &godot_qa_runtime::SessionRecord) -> Option<Self> {
        if !record.target.is_desktop() {
            return None;
        }
        Some(WarmPooledSessionRef {
            session_id: record.session_id.clone(),
            logs_path: Path::new(&record.artifact_dir).join("logs.txt"),
        })
    }
}

/// Where a warm scenario's slice of the pooled session log begins.
///
/// The offset is taken immediately before the `scenario.run` dispatch, so the
/// read never has to walk output belonging to earlier scenarios on the same
/// lane. It is only a lower bound, NOT a shared cursor: the session is released
/// before finalize runs, so by the time the slice is read the file may already
/// be growing with the NEXT scenario's output. That is harmless because the
/// window is closed by this run's own `end <run-id>` marker, not by EOF.
#[derive(Debug, Clone)]
struct WarmSessionLogAnchor {
    path: PathBuf,
    start_offset: u64,
}

/// Anchor the session log for a scenario about to be dispatched, or None when
/// there is nothing on the host filesystem to slice. Desktop-only by design:
/// an Android session's engine output lives on the device and never reaches this
/// path anyway (`android.rs` bypasses the warm pool entirely), so gating on the
/// target keeps the "read a local file" assumption explicit rather than implied.
fn warm_session_log_anchor(
    handle: &godot_qa_runtime::PooledSessionHandle,
) -> Option<WarmSessionLogAnchor> {
    if !handle.record.target.is_desktop() {
        return None;
    }
    let path = Path::new(&handle.record.artifact_dir).join("logs.txt");
    let start_offset = fs::metadata(&path).ok()?.len();
    Some(WarmSessionLogAnchor { path, start_offset })
}

/// `data.warm.log_slice.status` values.
const WARM_LOG_SLICE_SLICED: &str = "sliced";
const WARM_LOG_SLICE_MARKERS_MISSING: &str = "markers_missing";
const WARM_LOG_SLICE_UNAVAILABLE: &str = "unavailable";

/// The outcome of slicing one warm scenario out of its session log.
struct WarmLogSlice {
    status: &'static str,
    /// The session log the slice was read from (empty when there was none).
    source: String,
    /// Absolute byte offsets into that session log, present only when sliced.
    begin_offset: Option<u64>,
    end_offset: Option<u64>,
}

impl WarmLogSlice {
    fn unavailable(source: String) -> Self {
        WarmLogSlice {
            status: WARM_LOG_SLICE_UNAVAILABLE,
            source,
            begin_offset: None,
            end_offset: None,
        }
    }

    fn window(&self) -> Option<(u64, u64)> {
        Some((self.begin_offset?, self.end_offset?))
    }

    fn to_json(&self, session_id: &str) -> Value {
        json!({
            "source": if self.source.is_empty() { Value::Null } else { json!(self.source) },
            "session_id": session_id,
            "status": self.status,
            "begin_offset": self.begin_offset,
            "end_offset": self.end_offset,
        })
    }
}

/// Read this scenario's window out of the pooled session log and write it to the
/// run's own `logs.txt`.
///
/// On `markers_missing` NO file is created: `data.logs.status` then stays
/// `"missing"` exactly as it did before warm log parity existed, which is the
/// right answer for an addon that cannot mark its scenario boundaries — better a
/// visibly absent log than a window guessed from offsets that could attribute
/// another scenario's error to this one.
fn materialize_warm_scenario_logs(
    artifacts: &artifacts::RunArtifacts,
    anchor: &Option<WarmSessionLogAnchor>,
) -> WarmLogSlice {
    let Some(anchor) = anchor else {
        return WarmLogSlice::unavailable(String::new());
    };
    let source = anchor.path.to_string_lossy().into_owned();
    // Read-only, from the anchor to EOF. The Godot child owns this file's write
    // end for the whole bundle, so the CLI never truncates, rotates or reopens it.
    let Ok(bytes) = read_from_offset(&anchor.path, anchor.start_offset) else {
        return WarmLogSlice::unavailable(source);
    };
    let Some(range) = slice_scenario_log(&bytes, &artifacts.run_id) else {
        return WarmLogSlice {
            status: WARM_LOG_SLICE_MARKERS_MISSING,
            source,
            begin_offset: None,
            end_offset: None,
        };
    };
    let begin_offset = anchor.start_offset + range.start as u64;
    let end_offset = anchor.start_offset + range.end as u64;
    if fs::write(
        &artifacts.logs_path,
        String::from_utf8_lossy(&bytes[range]).as_bytes(),
    )
    .is_err()
    {
        return WarmLogSlice::unavailable(source);
    }
    WarmLogSlice {
        status: WARM_LOG_SLICE_SLICED,
        source,
        begin_offset: Some(begin_offset),
        end_offset: Some(end_offset),
    }
}

fn read_from_offset(path: &Path, offset: u64) -> io::Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = fs::File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Byte range of one scenario's window inside a session log, marker lines
/// included: from the start of `<prefix> begin <run_id>` through the end of the
/// following `<prefix> end <run_id>` line.
///
/// Whole-line equality (after trimming a trailing `\r`) is deliberate — a
/// substring match would let a game that echoes a marker string reshape the
/// window. Markers for OTHER run ids are ignored, so a concurrently-written log
/// from another lane, or the next scenario's `begin` landing after our `end`,
/// cannot widen or truncate this window. A missing `end` yields None rather than
/// "to EOF": an unterminated window would swallow whatever the next scenario
/// wrote.
fn slice_scenario_log(bytes: &[u8], run_id: &str) -> Option<std::ops::Range<usize>> {
    let begin_marker = format!("{WARM_LOG_MARKER_PREFIX} begin {run_id}");
    let end_marker = format!("{WARM_LOG_MARKER_PREFIX} end {run_id}");
    let mut begin: Option<usize> = None;
    let mut offset = 0usize;
    while offset < bytes.len() {
        let line_end = bytes[offset..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| offset + index)
            .unwrap_or(bytes.len());
        // Past the newline when there is one, so the returned range covers the
        // end marker's own line terminator.
        let next_line = (line_end + 1).min(bytes.len());
        let line = String::from_utf8_lossy(&bytes[offset..line_end]);
        let line = line.trim_end_matches('\r');
        match begin {
            None if line == begin_marker => begin = Some(offset),
            Some(begin) if line == end_marker => return Some(begin..next_line),
            _ => {}
        }
        offset = next_line;
    }
    None
}

/// The `phase` label on `data.warm.session_log_noise`: these lines were emitted
/// by a pooled session while NO scenario window was open — session start, idle
/// time between scenarios, or shutdown.
const WARM_SESSION_LOG_NOISE_PHASE: &str = "outside_scenario_windows";

/// How many characters of a noisy line are kept as its grouping key. Engine
/// errors are one line plus an indented backtrace; 200 chars is enough to tell
/// two distinct faults apart without pasting a whole log into the bundle JSON.
const WARM_SESSION_LOG_NOISE_LINE_MAX: usize = 200;

/// How many session ids a single grouped message lists.
const WARM_SESSION_LOG_NOISE_SESSIONS_MAX: usize = 10;

/// Engine noise a pooled warm session emits as its PROCESS exits, after the last
/// scenario window has closed.
///
/// Deliberately NOT added to the shared cold ignore lists in runtime_result.rs:
/// in a cold run "N resources still in use at exit" is emitted by the very
/// process that ran the scenario, so it is a real leak signal that run owns, and
/// silencing it globally would weaken cold for every consumer. Here the emitter
/// is a long-lived QA session shutting down after many scenarios, where the line
/// says nothing about any one of them. (`ObjectDB instances leaked at exit`, the
/// other half of the same exit pair, is already ignored by the shared list.)
fn is_warm_session_exit_noise(line: &str) -> bool {
    line.contains("resources still in use at exit")
}

/// Engine `ERROR:`/`WARNING:` lines in a pooled session log that fall OUTSIDE
/// every `scenario-log begin … end` window, with the SAME ignore predicates the
/// cold scan applies (`runtime_log_failures`) so the two agree on what counts as
/// noise. That includes the project's own `logs.ignore` patterns, threaded in as
/// `ignore_patterns`: a pattern that silences a line for a cold run must silence
/// the same line here, or a consumer would have to configure the same noise
/// twice. Pure over the bytes so the window arithmetic is unit-testable.
///
/// An unterminated final window (a session killed mid-scenario) swallows the
/// rest of the file: that output is attributable to a scenario, so reporting it
/// as gap noise would be wrong in the one direction that matters.
fn session_log_noise_lines(bytes: &[u8], ignore_patterns: &[String]) -> Vec<String> {
    let begin_prefix = format!("{WARM_LOG_MARKER_PREFIX} begin ");
    let end_prefix = format!("{WARM_LOG_MARKER_PREFIX} end ");
    let mut inside_window = false;
    let mut noise = Vec::new();
    for raw_line in bytes.split(|byte| *byte == b'\n') {
        let line = String::from_utf8_lossy(raw_line);
        let line = line.trim_end_matches('\r');
        if line.starts_with(&begin_prefix) {
            inside_window = true;
            continue;
        }
        if line.starts_with(&end_prefix) {
            inside_window = false;
            continue;
        }
        if inside_window {
            continue;
        }
        let stripped = line.trim();
        let ignored = if stripped.starts_with("ERROR:") {
            is_ignored_godot_log_error(stripped, ignore_patterns)
        } else if stripped.starts_with("WARNING:") {
            is_ignored_godot_log_warning(stripped, ignore_patterns)
        } else {
            continue;
        };
        if ignored || is_warm_session_exit_noise(stripped) {
            continue;
        }
        noise.push(stripped.to_string());
    }
    noise
}

/// The bundle-level tally of gap noise across every session this pool started.
struct WarmSessionLogNoise {
    sessions_scanned: usize,
    line_count: usize,
    /// (line, count, session ids) ordered by count desc, then line.
    by_message: Vec<(String, usize, Vec<String>)>,
}

impl WarmSessionLogNoise {
    fn to_json(&self) -> Value {
        json!({
            "sessions_scanned": self.sessions_scanned,
            "line_count": self.line_count,
            "by_message": self
                .by_message
                .iter()
                .map(|(line, count, sessions)| json!({
                    "line": line,
                    "count": count,
                    "sessions": sessions,
                }))
                .collect::<Vec<_>>(),
            "phase": WARM_SESSION_LOG_NOISE_PHASE,
        })
    }
}

/// Scan every pooled session's completed log for gap noise.
///
/// Runs from the post-run hook AFTER `shutdown`, so each session's process is
/// gone and its `logs.txt` holds everything it will ever hold — including the
/// shutdown-time errors that, by construction, land after the last `end` marker
/// and therefore belong to no run's `logs.txt`. A session whose log cannot be
/// read is simply not counted as scanned.
fn scan_warm_session_log_noise(
    sessions: &[WarmPooledSessionRef],
    ignore_patterns: &[String],
) -> WarmSessionLogNoise {
    let mut sessions_scanned = 0usize;
    let mut line_count = 0usize;
    // Insertion-ordered grouping: index into `groups` keyed by the truncated line.
    let mut order: Vec<String> = Vec::new();
    let mut groups: std::collections::HashMap<String, (usize, Vec<String>)> =
        std::collections::HashMap::new();
    for session in sessions {
        let Ok(bytes) = fs::read(&session.logs_path) else {
            continue;
        };
        sessions_scanned += 1;
        for line in session_log_noise_lines(&bytes, ignore_patterns) {
            let key: String = line.chars().take(WARM_SESSION_LOG_NOISE_LINE_MAX).collect();
            line_count += 1;
            let entry = groups.entry(key.clone()).or_insert_with(|| {
                order.push(key.clone());
                (0, Vec::new())
            });
            entry.0 += 1;
            if !entry.1.contains(&session.session_id)
                && entry.1.len() < WARM_SESSION_LOG_NOISE_SESSIONS_MAX
            {
                entry.1.push(session.session_id.clone());
            }
        }
    }
    let mut by_message: Vec<(String, usize, Vec<String>)> = order
        .into_iter()
        .map(|line| {
            let (count, sessions) = groups.remove(&line).unwrap_or((0, Vec::new()));
            (line, count, sessions)
        })
        .collect();
    by_message.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    WarmSessionLogNoise {
        sessions_scanned,
        line_count,
        by_message,
    }
}

/// The one bundle-level warning for engine output no scenario can be blamed for.
///
/// Advisory ONLY: these lines are real engine faults, but they happened while no
/// scenario window was open, so attributing them to a run would be a lie. Never a
/// failure and never promoted by `--strict` or `--warm-strict` — the same rule
/// the other pool warnings follow.
fn warm_session_log_noise_warning(noise: &WarmSessionLogNoise) -> Option<FailureDetail> {
    if noise.line_count == 0 {
        return None;
    }
    let top = noise.by_message.first().map(|(line, _, _)| line.clone());
    Some(FailureDetail {
        kind: "warm_session_log_noise".to_string(),
        command: Some("scenario bundle run".to_string()),
        message: Some(format!(
            "{} engine log lines were emitted by pooled warm sessions outside any scenario \
             window (session start/idle/shutdown); see data.warm.session_log_noise",
            noise.line_count
        )),
        expected: Some(json!(
            "no engine ERROR:/WARNING: lines outside a scenario window"
        )),
        actual: top.map(Value::String),
        details: Map::from_iter([(
            "by_message".to_string(),
            noise.to_json()["by_message"].clone(),
        )]),
        ..FailureDetail::default()
    })
}

/// Attach the session-log window every `godot_log_error` / `godot_log_warning`
/// was read from. Returns whether anything was annotated (i.e. whether the
/// caller must re-persist result.json).
fn annotate_warm_log_failures(
    result: &mut CommandResult,
    session_id: &str,
    window: Option<&(u64, u64)>,
) -> bool {
    let Some((begin_offset, end_offset)) = window else {
        return false;
    };
    let mut annotated = false;
    for failure in &mut result.failures {
        if failure.kind != "godot_log_error" && failure.kind != "godot_log_warning" {
            continue;
        }
        failure.details.insert(
            "warm_log_window".to_string(),
            json!({
                "session_id": session_id,
                "begin_offset": begin_offset,
                "end_offset": end_offset,
            }),
        );
        annotated = true;
    }
    annotated
}

/// `annotate_warm_result` rebuilds `data.warm` from scratch, which would drop the
/// engine-log fields `warm_finalize_scenario` recorded there (and already
/// persisted to result.json). Lift them out before that rebuild...
fn take_warm_log_annotation(result: &CommandResult) -> Map<String, Value> {
    let Some(warm) = result.data.get("warm").and_then(Value::as_object) else {
        return Map::new();
    };
    [WARM_SESSION_ID_KEY, WARM_LOG_SLICE_KEY]
        .into_iter()
        .filter_map(|key| warm.get(key).map(|value| (key.to_string(), value.clone())))
        .collect()
}

/// ...and put them back after it, so the bundle summary's `data.runs[].warm` and
/// the persisted result.json agree on the log evidence.
fn restore_warm_log_annotation(result: &mut CommandResult, kept: Map<String, Value>) {
    if kept.is_empty() {
        return;
    }
    let Some(warm) = result.data.get_mut("warm").and_then(Value::as_object_mut) else {
        return;
    };
    for (key, value) in kept {
        warm.insert(key, value);
    }
}
