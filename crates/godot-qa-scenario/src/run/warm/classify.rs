// Warm classification: which bucket a scenario falls into (headless-warm,
// windowed-warm, or truly-cold) and the structural/media reasons behind it.
//
// Included into `run/warm.rs`, which is itself included into the run module, so
// this file shares the whole `crate::run` scope.

/// The scheduling bucket for a scenario in a warm bundle.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum WarmBucket {
    /// A warm-eligible scenario, in one of the four media grid cells: `headless`
    /// selects the display flavor (`true` = headless-display session, `false` =
    /// windowed session that renders real pixels) and `audio` selects the audio
    /// flavor (`true` = real audio driver, `false` = `--audio-driver Dummy`).
    Warm { headless: bool, audio: bool },
    /// A structural or declared cold reason — runs cold in a fresh process.
    /// `strict` is whether a `--warm-strict` bundle should treat the cold
    /// fallback as a violation (structural reasons are violations; a declared
    /// `execution.isolation: cold` opt-out is not).
    TrulyCold {
        reason: &'static str,
        strict: bool,
    },
}

/// True when a scenario has per-scenario `godot_args` (process-launch flags such
/// as `--single-window` applied at boot; a warm process launches once, so a
/// scenario that needs its own engine launch flags must run cold. Per-scenario
/// `project_args` ARE delivered warm via the bridge, so they do not force cold).
fn scenario_has_godot_args(scenario: &Value) -> bool {
    !scenario_godot_args(scenario).is_empty()
}

/// True when a scenario declares `execution.isolation: cold`.
fn scenario_declares_cold(scenario: &Value) -> bool {
    scenario
        .get("execution")
        .and_then(Value::as_object)
        .and_then(|execution| execution.get("isolation"))
        .and_then(Value::as_str)
        .map(|isolation| isolation.eq_ignore_ascii_case("cold"))
        .unwrap_or(false)
}

/// A STRUCTURAL reason a scenario cannot be made warm-equivalent in a shared
/// process — process-launch flags, a viewport matrix, or a fixed_fps that cannot
/// change mid-session. These force a fresh cold process and, in strict mode, are
/// violations. Returns None when the scenario has no structural cold reason.
///
/// A single-viewport `layout_audit` is NOT structural: it runs the normal single
/// path and its audit is a CLI post-process, so it is windowed-warm eligible
/// (its media need selects the windowed session). Only a genuine MULTI-viewport
/// matrix (`viewports.len() > 1`, caught below) is structurally cold — and a
/// layout matrix is exactly such a multi-viewport scenario, so `multi_viewport`
/// already covers it.
fn structural_cold_reason(
    scenario: &Value,
    project_root: &Path,
    session_fps: i64,
) -> Option<&'static str> {
    if scenario_has_godot_args(scenario) {
        return Some("godot_args");
    }
    if scenario_viewports(scenario).len() > 1 {
        return Some("multi_viewport");
    }
    // A process-level --fixed-fps cannot change mid-session: a scenario whose
    // explicit fixed_fps differs from the session's must run cold. A frame
    // pacing validation error also falls back to cold so the error surfaces
    // identically.
    match scenario_frame_pacing(scenario, project_root) {
        Ok(Some(pacing)) if pacing.fixed_fps != session_fps => Some("fixed_fps_mismatch"),
        Ok(_) => None,
        Err(_) => Some("frame_pacing_invalid"),
    }
}

/// A MEDIA-ONLY reason a scenario would need a fresh process today: it produces
/// windowed media evidence (a saved screenshot, a visual pixel assertion, a
/// screenshot match, or audio capture). Such a scenario is windowed-warm
/// eligible — it can run in a shared WINDOWED warm session instead. Returns the
/// media reason, or None when the scenario needs no windowed media.
fn media_cold_reason(scenario: &Value) -> Option<&'static str> {
    let has_screenshot_match = !screenshot_match_assertions(scenario).is_empty();
    if has_screenshot_match {
        return Some("screenshot_match");
    }
    if scenario_requires_windowed_media_evidence(scenario, has_screenshot_match) {
        return Some("windowed_media");
    }
    None
}

/// Partition a scenario into its warm scheduling bucket. Structural/declared
/// cold reasons take precedence over media reasons: a scenario that needs BOTH
/// windowed media AND (say) its own `godot_args` is truly-cold (the structural
/// reason wins and, being undeclared, is a strict violation).
///
/// `headless_override` / `audio_override` are the resolved media-axis overrides
/// (the caller ORs in the project-config defaults exactly as the cold path does),
/// so the windowed-vs-headless and dummy-vs-real flavors honor `--headless` /
/// `--windowed` / `--audio` / `--no-audio` and `godot.launch_mode` /
/// `godot.audio_mode` identically to a cold run.
fn classify_warm_bucket(
    scenario: &Value,
    project_root: &Path,
    session_fps: i64,
    headless_override: Option<bool>,
    audio_override: Option<bool>,
) -> WarmBucket {
    if scenario_declares_cold(scenario) {
        return WarmBucket::TrulyCold {
            reason: DECLARED_COLD_REASON,
            strict: false,
        };
    }
    if let Some(reason) = structural_cold_reason(scenario, project_root, session_fps) {
        return WarmBucket::TrulyCold {
            reason,
            strict: true,
        };
    }
    // The display and audio axes are independent: windowed-media evidence selects
    // the display flavor, audio-capture evidence selects the audio flavor.
    let requires_windowed_media = media_cold_reason(scenario).is_some();
    let requires_audio = scenario_requires_audio_evidence(scenario);
    WarmBucket::Warm {
        headless: selected_headless_for_scenario(headless_override, requires_windowed_media),
        audio: selected_audio_for_scenario(audio_override, requires_audio),
    }
}

/// Back-compat shim: True (with the reason code) when a scenario cannot run in
/// the *headless* warm session — i.e. it is truly-cold OR windowed-warm. Kept
/// for the warm unit tests; callers that need the full split use
/// `classify_warm_bucket`.
#[cfg(test)]
fn scenario_needs_cold_isolation(
    scenario: &Value,
    project_root: &Path,
    session_fps: i64,
) -> Option<&'static str> {
    match classify_warm_bucket(scenario, project_root, session_fps, None, None) {
        // A plain warm scenario needs no cold isolation; a windowed-media one has
        // a media reason (it still runs warm, in the windowed session).
        WarmBucket::Warm { .. } => media_cold_reason(scenario),
        WarmBucket::TrulyCold { reason, .. } => Some(reason),
    }
}
