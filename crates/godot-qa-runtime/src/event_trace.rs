use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::hash::{Hash, Hasher};

pub const APPROVED_EVENT_PROVIDER_METHODS: &[&str] = &[
    "get_qa_event_stream_state",
    "get_event_stream_qa_state",
    "get_qa_events",
    "qa_event_stream",
];

pub const WARNING_PROVIDER_MISSING: &str = "event_trace.provider_missing";
pub const WARNING_PROVIDER_INVALID: &str = "event_trace.provider_invalid";
pub const WARNING_EVENT_MISSING_ID: &str = "event_trace.event_missing_id";
pub const WARNING_EVENT_NOT_JSON_COMPATIBLE: &str = "event_trace.event_not_json_compatible";
pub const WARNING_EVENT_STREAM_TRUNCATED: &str = "event_trace.event_stream_truncated";
pub const WARNING_EVENT_SPAM_RISK: &str = "event_trace.event_spam_risk";
pub const WARNING_PAIRED_EVENT_MISSING: &str = "event_trace.paired_event_missing";
pub const WARNING_OFFSCREEN_EVENT_RISK: &str = "event_trace.offscreen_event_risk";
pub const WARNING_PROVIDER_ERROR: &str = "event_trace.provider_error";
pub const WARNING_NO_EVENTS_RECORDED: &str = "event_trace.no_events_recorded";

/// Conservative default until the live command exposes a documented flag.
pub const DEFAULT_MAX_EVENT_PAYLOAD_BYTES: usize = 16 * 1024;
pub const DEFAULT_MAX_EVENTS_PER_STREAM: usize = 10_000;
pub const MAX_PAIRING_WINDOW_MSEC: u64 = 60_000;
pub const MAX_RATE_LIMIT_PER_SECOND: f64 = 100_000.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventTraceRequest {
    pub target: Option<String>,
    pub provider: Option<String>,
    pub frames: Option<u64>,
    pub sample_every_frames: Option<u64>,
    pub pairing_config_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventTraceArtifact {
    pub events: Vec<EventTraceEvent>,
    #[serde(default)]
    pub provider_errors: Vec<EventTraceProviderError>,
    #[serde(default)]
    pub warnings: Vec<EventTraceWarning>,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventTraceProviderError {
    pub message: String,
    pub sample_index: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventTraceEvent {
    pub event_id: Option<String>,
    pub frame: Option<i64>,
    pub time_msec: Option<f64>,
    pub category: String,
    pub event_type: String,
    pub logical_id: Option<String>,
    pub position: Option<EventTracePosition>,
    pub payload: Value,
    #[serde(default)]
    pub warnings: Vec<EventTraceWarning>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EventTracePosition {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventTraceWarning {
    pub id: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame: Option<i64>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub details: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventTraceSummary {
    pub total_event_count: usize,
    pub event_count_by_category: BTreeMap<String, usize>,
    pub event_count_by_type: BTreeMap<String, usize>,
    pub event_rate_per_second_by_type: BTreeMap<String, f64>,
    pub max_events_in_one_frame: usize,
    pub duplicate_event_count: usize,
    pub missing_event_id_count: usize,
    pub event_spam_warnings: Vec<EventTraceWarning>,
    pub paired_event_warnings: Vec<EventTraceWarning>,
    pub offscreen_event_warnings: Vec<EventTraceWarning>,
    pub provider_error_count: usize,
    pub warning_ids: Vec<String>,
    pub warnings: Vec<EventTraceWarning>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct EventTraceConfig {
    #[serde(default)]
    pub pairs: Vec<EventTracePairRule>,
    #[serde(default)]
    pub rate_limits: Vec<EventTraceRateLimit>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventTracePairRule {
    pub source: String,
    pub requires_within_msec: u64,
    pub any_of: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventTraceRateLimit {
    pub event_type: String,
    pub max_per_second: f64,
}

pub fn normalize_event_trace_artifact(
    raw_events: &[Value],
    provider_errors: Vec<EventTraceProviderError>,
) -> EventTraceArtifact {
    normalize_event_trace_artifact_with_limits(
        raw_events,
        provider_errors,
        DEFAULT_MAX_EVENT_PAYLOAD_BYTES,
        DEFAULT_MAX_EVENTS_PER_STREAM,
    )
}

pub fn normalize_event_trace_artifact_with_limits(
    raw_events: &[Value],
    provider_errors: Vec<EventTraceProviderError>,
    max_payload_bytes: usize,
    max_events: usize,
) -> EventTraceArtifact {
    let mut events = Vec::new();
    let mut warnings = Vec::new();
    let truncated = raw_events.len() > max_events;

    if truncated {
        warnings.push(EventTraceWarning::new(
            WARNING_EVENT_STREAM_TRUNCATED,
            "event stream exceeded the configured event limit",
        ));
    }

    for (index, raw_event) in raw_events.iter().take(max_events).enumerate() {
        match normalize_event(raw_event, index, max_payload_bytes) {
            Ok(mut event) => {
                warnings.extend(event.warnings.iter().cloned());
                events.push({
                    event.warnings.sort_by(|a, b| a.id.cmp(&b.id));
                    event
                });
            }
            Err(warning) => warnings.push(warning),
        }
    }

    if !provider_errors.is_empty() {
        warnings.push(EventTraceWarning::new(
            WARNING_PROVIDER_ERROR,
            "event provider returned one or more errors",
        ));
    }

    EventTraceArtifact {
        events,
        provider_errors,
        warnings,
        truncated,
    }
}

pub fn summarize_event_trace(
    events: &[EventTraceEvent],
    config: Option<&EventTraceConfig>,
    provider_error_count: usize,
) -> EventTraceSummary {
    let mut warnings = Vec::new();
    let mut event_spam_warnings = Vec::new();
    let mut paired_event_warnings = Vec::new();
    let offscreen_event_warnings = Vec::new();

    if events.is_empty() {
        warnings.push(EventTraceWarning::new(
            WARNING_NO_EVENTS_RECORDED,
            "event trace recorded no events",
        ));
    }
    if provider_error_count > 0 {
        let mut warning = EventTraceWarning::new(
            WARNING_PROVIDER_ERROR,
            "event provider returned one or more errors",
        );
        warning.details.insert(
            "provider_error_count".to_string(),
            json!(provider_error_count),
        );
        warnings.push(warning);
    }

    let mut seen_event_ids = HashSet::new();
    let mut seen_fallback_keys = HashSet::new();
    let mut duplicate_event_count = 0;
    let mut missing_event_id_count = 0;
    let mut deduped = Vec::new();

    for event in events {
        warnings.extend(event.warnings.iter().cloned());
        let duplicate =
            if let Some(event_id) = event.event_id.as_deref().filter(|id| !id.is_empty()) {
                !seen_event_ids.insert(event_id.to_string())
            } else {
                missing_event_id_count += 1;
                warnings.push(missing_id_warning(event));
                !seen_fallback_keys.insert(fallback_dedupe_key(event))
            };

        if duplicate {
            duplicate_event_count += 1;
        } else {
            deduped.push(event);
        }
    }

    let mut event_count_by_category = BTreeMap::new();
    let mut event_count_by_type = BTreeMap::new();
    let mut frame_counts = BTreeMap::new();
    let mut time_bounds_by_type: BTreeMap<String, (f64, f64)> = BTreeMap::new();

    for event in &deduped {
        *event_count_by_category
            .entry(event.category.clone())
            .or_insert(0) += 1;
        *event_count_by_type
            .entry(event.event_type.clone())
            .or_insert(0) += 1;
        if let Some(frame) = event.frame {
            *frame_counts.entry(frame).or_insert(0) += 1;
        }
        if let Some(time_msec) = event.time_msec.filter(|time| time.is_finite()) {
            time_bounds_by_type
                .entry(event.event_type.clone())
                .and_modify(|bounds| {
                    bounds.0 = bounds.0.min(time_msec);
                    bounds.1 = bounds.1.max(time_msec);
                })
                .or_insert((time_msec, time_msec));
        }
    }

    let mut event_rate_per_second_by_type = BTreeMap::new();
    for (event_type, count) in &event_count_by_type {
        let rate = time_bounds_by_type
            .get(event_type)
            .map(|(min_time, max_time)| {
                let seconds = ((*max_time - *min_time) / 1000.0).max(1.0);
                (*count as f64) / seconds
            })
            .unwrap_or(*count as f64);
        event_rate_per_second_by_type.insert(event_type.clone(), finite_rate(rate));
    }

    if let Some(config) = config {
        event_spam_warnings = spam_warnings(&event_rate_per_second_by_type, &config.rate_limits);
        paired_event_warnings = pairing_warnings(&deduped, &config.pairs);
        warnings.extend(event_spam_warnings.iter().cloned());
        warnings.extend(paired_event_warnings.iter().cloned());
    }

    let warning_ids = warning_ids(&warnings);

    EventTraceSummary {
        total_event_count: deduped.len(),
        event_count_by_category,
        event_count_by_type,
        event_rate_per_second_by_type,
        max_events_in_one_frame: frame_counts.values().copied().max().unwrap_or(0),
        duplicate_event_count,
        missing_event_id_count,
        event_spam_warnings,
        paired_event_warnings,
        offscreen_event_warnings,
        provider_error_count,
        warning_ids,
        warnings,
    }
}

pub fn parse_pairing_config(contents: &str) -> Result<EventTraceConfig, String> {
    let config: EventTraceConfig = serde_json::from_str(contents)
        .map_err(|error| format!("event trace pairing config must be valid JSON: {error}"))?;
    validate_pairing_config(config)
}

fn validate_pairing_config(config: EventTraceConfig) -> Result<EventTraceConfig, String> {
    for (index, pair) in config.pairs.iter().enumerate() {
        validate_event_type(&pair.source, &format!("pairs[{index}].source"))?;
        if pair.requires_within_msec == 0 || pair.requires_within_msec > MAX_PAIRING_WINDOW_MSEC {
            return Err(format!(
                "pairs[{index}].requires_within_msec must be between 1 and {MAX_PAIRING_WINDOW_MSEC}"
            ));
        }
        if pair.any_of.is_empty() {
            return Err(format!(
                "pairs[{index}].any_of must contain at least one event type"
            ));
        }
        for (candidate_index, event_type) in pair.any_of.iter().enumerate() {
            validate_event_type(
                event_type,
                &format!("pairs[{index}].any_of[{candidate_index}]"),
            )?;
        }
    }

    for (index, rate_limit) in config.rate_limits.iter().enumerate() {
        validate_event_type(
            &rate_limit.event_type,
            &format!("rate_limits[{index}].event_type"),
        )?;
        if !rate_limit.max_per_second.is_finite()
            || rate_limit.max_per_second <= 0.0
            || rate_limit.max_per_second > MAX_RATE_LIMIT_PER_SECOND
        {
            return Err(format!(
                "rate_limits[{index}].max_per_second must be finite and between 0 and {MAX_RATE_LIMIT_PER_SECOND}"
            ));
        }
    }

    Ok(config)
}

fn validate_event_type(event_type: &str, field: &str) -> Result<(), String> {
    if event_type.trim().is_empty() {
        return Err(format!("{field} must be a non-empty event type string"));
    }
    Ok(())
}

fn normalize_event(
    raw_event: &Value,
    index: usize,
    max_payload_bytes: usize,
) -> Result<EventTraceEvent, EventTraceWarning> {
    let object = raw_event.as_object().ok_or_else(|| {
        let mut warning = EventTraceWarning::new(
            WARNING_EVENT_NOT_JSON_COMPATIBLE,
            "event entry must be a JSON object",
        );
        warning.details.insert("index".to_string(), json!(index));
        warning
    })?;

    let category = object
        .get("category")
        .and_then(Value::as_str)
        .unwrap_or("custom")
        .trim();
    let event_type = object
        .get("event_type")
        .or_else(|| object.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .trim();

    let mut warnings = Vec::new();
    let payload = object.get("payload").cloned().unwrap_or_else(|| json!({}));
    let bounded_payload = bound_payload(payload, max_payload_bytes, &mut warnings);

    Ok(EventTraceEvent {
        event_id: object
            .get("event_id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_string),
        frame: object.get("frame").and_then(Value::as_i64),
        time_msec: object.get("time_msec").and_then(Value::as_f64),
        category: if category.is_empty() {
            "custom".to_string()
        } else {
            category.to_string()
        },
        event_type: if event_type.is_empty() {
            "unknown".to_string()
        } else {
            event_type.to_string()
        },
        logical_id: object
            .get("logical_id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_string),
        position: normalize_position(object.get("position")),
        payload: bounded_payload,
        warnings,
    })
}

fn normalize_position(value: Option<&Value>) -> Option<EventTracePosition> {
    let object = value?.as_object()?;
    let x = object.get("x")?.as_f64()?;
    let y = object.get("y")?.as_f64()?;
    if x.is_finite() && y.is_finite() {
        Some(EventTracePosition { x, y })
    } else {
        None
    }
}

fn bound_payload(
    payload: Value,
    max_payload_bytes: usize,
    warnings: &mut Vec<EventTraceWarning>,
) -> Value {
    let Ok(encoded) = serde_json::to_vec(&payload) else {
        warnings.push(EventTraceWarning::new(
            WARNING_EVENT_NOT_JSON_COMPATIBLE,
            "event payload could not be serialized as JSON",
        ));
        return json!({"truncated": true, "reason": "not_json_compatible"});
    };

    if encoded.len() <= max_payload_bytes {
        return payload;
    }

    let mut warning = EventTraceWarning::new(
        WARNING_EVENT_STREAM_TRUNCATED,
        "event payload exceeded the configured byte limit",
    );
    warning
        .details
        .insert("max_payload_bytes".to_string(), json!(max_payload_bytes));
    warning
        .details
        .insert("actual_payload_bytes".to_string(), json!(encoded.len()));
    warnings.push(warning);
    json!({
        "truncated": true,
        "original_size_bytes": encoded.len(),
        "max_payload_bytes": max_payload_bytes
    })
}

fn fallback_dedupe_key(event: &EventTraceEvent) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    event.frame.hash(&mut hasher);
    event.category.hash(&mut hasher);
    event.event_type.hash(&mut hasher);
    event.logical_id.hash(&mut hasher);
    serde_json::to_string(&event.payload)
        .unwrap_or_else(|_| "null".to_string())
        .hash(&mut hasher);
    hasher.finish()
}

fn missing_id_warning(event: &EventTraceEvent) -> EventTraceWarning {
    let mut warning = EventTraceWarning::new(
        WARNING_EVENT_MISSING_ID,
        "event is missing event_id; using cautious fallback dedupe key",
    );
    warning.event_type = Some(event.event_type.clone());
    warning.frame = event.frame;
    warning
}

fn spam_warnings(
    rates: &BTreeMap<String, f64>,
    rate_limits: &[EventTraceRateLimit],
) -> Vec<EventTraceWarning> {
    let mut warnings = Vec::new();
    for rate_limit in rate_limits {
        let Some(actual_rate) = rates.get(&rate_limit.event_type).copied() else {
            continue;
        };
        if actual_rate > rate_limit.max_per_second {
            let mut warning = EventTraceWarning::new(
                WARNING_EVENT_SPAM_RISK,
                "event type exceeded configured rate limit",
            );
            warning.event_type = Some(rate_limit.event_type.clone());
            warning
                .details
                .insert("actual_per_second".to_string(), json!(actual_rate));
            warning.details.insert(
                "max_per_second".to_string(),
                json!(rate_limit.max_per_second),
            );
            warnings.push(warning);
        }
    }
    warnings
}

fn pairing_warnings(
    events: &[&EventTraceEvent],
    pair_rules: &[EventTracePairRule],
) -> Vec<EventTraceWarning> {
    let mut warnings = Vec::new();
    for rule in pair_rules {
        for event in events
            .iter()
            .copied()
            .filter(|event| event.event_type == rule.source)
        {
            let Some(source_time) = event.time_msec.filter(|time| time.is_finite()) else {
                continue;
            };
            let has_pair = events.iter().copied().any(|candidate| {
                candidate
                    .time_msec
                    .filter(|time| time.is_finite())
                    .map(|time| {
                        time >= source_time
                            && time - source_time <= rule.requires_within_msec as f64
                            && rule.any_of.contains(&candidate.event_type)
                    })
                    .unwrap_or(false)
            });
            if !has_pair {
                let mut warning = EventTraceWarning::new(
                    WARNING_PAIRED_EVENT_MISSING,
                    "source event did not have a configured companion event within the time window",
                );
                warning.event_type = Some(event.event_type.clone());
                warning.event_id = event.event_id.clone();
                warning.frame = event.frame;
                warning.details.insert(
                    "requires_within_msec".to_string(),
                    json!(rule.requires_within_msec),
                );
                warning
                    .details
                    .insert("any_of".to_string(), json!(rule.any_of));
                warnings.push(warning);
            }
        }
    }
    warnings
}

fn finite_rate(rate: f64) -> f64 {
    if rate.is_finite() { rate } else { 0.0 }
}

fn warning_ids(warnings: &[EventTraceWarning]) -> Vec<String> {
    warnings
        .iter()
        .map(|warning| warning.id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

impl EventTraceWarning {
    fn new(id: &str, message: &str) -> Self {
        Self {
            id: id.to_string(),
            message: message.to_string(),
            event_type: None,
            event_id: None,
            frame: None,
            details: Map::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(id: Option<&str>, frame: i64, time_msec: f64, event_type: &str) -> EventTraceEvent {
        EventTraceEvent {
            event_id: id.map(str::to_string),
            frame: Some(frame),
            time_msec: Some(time_msec),
            category: "custom".to_string(),
            event_type: event_type.to_string(),
            logical_id: None,
            position: None,
            payload: json!({}),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn event_trace_duplicate_ids_are_deduplicated_before_counts() {
        let summary = summarize_event_trace(
            &[
                event(Some("a"), 1, 0.0, "sfx.hit"),
                event(Some("a"), 1, 10.0, "sfx.hit"),
                event(Some("b"), 2, 1000.0, "ui.confirm"),
            ],
            None,
            0,
        );

        assert_eq!(summary.total_event_count, 2);
        assert_eq!(summary.duplicate_event_count, 1);
        assert_eq!(summary.event_count_by_type["sfx.hit"], 1);
        assert_eq!(summary.max_events_in_one_frame, 1);
    }

    #[test]
    fn event_trace_missing_ids_warn_and_use_fallback_dedupe() {
        let mut first = event(None, 4, 25.0, "collision.contact");
        first.logical_id = Some("ball".to_string());
        first.payload = json!({"normal": [1, 0]});
        let duplicate = first.clone();

        let summary = summarize_event_trace(&[first, duplicate], None, 0);

        assert_eq!(summary.total_event_count, 1);
        assert_eq!(summary.missing_event_id_count, 2);
        assert_eq!(summary.duplicate_event_count, 1);
        assert!(
            summary
                .warning_ids
                .contains(&WARNING_EVENT_MISSING_ID.to_string())
        );
    }

    #[test]
    fn event_trace_no_events_records_warning() {
        let summary = summarize_event_trace(&[], None, 0);

        assert_eq!(summary.total_event_count, 0);
        assert!(
            summary
                .warning_ids
                .contains(&WARNING_NO_EVENTS_RECORDED.to_string())
        );
    }

    #[test]
    fn event_trace_provider_errors_are_counted_and_warned() {
        let summary = summarize_event_trace(&[event(Some("a"), 1, 0.0, "gameplay.spawn")], None, 2);

        assert_eq!(summary.provider_error_count, 2);
        assert!(
            summary
                .warning_ids
                .contains(&WARNING_PROVIDER_ERROR.to_string())
        );
    }

    #[test]
    fn event_trace_payload_bounds_truncate_large_payloads() {
        let raw = json!({
            "event_id": "a",
            "frame": 1,
            "time_msec": 0,
            "category": "vfx",
            "event_type": "vfx.burst",
            "payload": {"blob": "x".repeat(64)}
        });

        let artifact = normalize_event_trace_artifact_with_limits(&[raw], Vec::new(), 16, 100);

        assert_eq!(artifact.events.len(), 1);
        assert_eq!(artifact.events[0].payload["truncated"], true);
        assert!(
            artifact
                .warnings
                .iter()
                .any(|warning| warning.id == WARNING_EVENT_STREAM_TRUNCATED)
        );
    }

    #[test]
    fn event_trace_pairing_and_rate_limits_are_config_driven() {
        let config = parse_pairing_config(
            r#"{
                "pairs": [
                    {"source": "collision.contact", "requires_within_msec": 120, "any_of": ["vfx.splash", "sfx.hit"]}
                ],
                "rate_limits": [
                    {"event_type": "sfx.loop", "max_per_second": 1}
                ]
            }"#,
        )
        .unwrap();
        let summary = summarize_event_trace(
            &[
                event(Some("a"), 1, 0.0, "collision.contact"),
                event(Some("b"), 2, 0.0, "sfx.loop"),
                event(Some("c"), 2, 100.0, "sfx.loop"),
                event(Some("d"), 2, 200.0, "sfx.loop"),
            ],
            Some(&config),
            0,
        );

        assert_eq!(summary.paired_event_warnings.len(), 1);
        assert_eq!(summary.event_spam_warnings.len(), 1);
        assert!(
            summary
                .warning_ids
                .contains(&WARNING_PAIRED_EVENT_MISSING.to_string())
        );
        assert!(
            summary
                .warning_ids
                .contains(&WARNING_EVENT_SPAM_RISK.to_string())
        );
    }

    #[test]
    fn event_trace_config_rejects_empty_types_and_invalid_bounds() {
        assert!(
            parse_pairing_config(
                r#"{"pairs":[{"source":"","requires_within_msec":1,"any_of":["a"]}]}"#
            )
            .is_err()
        );
        assert!(
            parse_pairing_config(
                r#"{"pairs":[{"source":"a","requires_within_msec":0,"any_of":["b"]}]}"#
            )
            .is_err()
        );
        assert!(
            parse_pairing_config(r#"{"rate_limits":[{"event_type":"a","max_per_second":0}]}"#)
                .is_err()
        );
    }
}
