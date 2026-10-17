//! Shared, bounded configuration for scenario and live soak profiling.
#![allow(clippy::result_large_err)] // Keep the existing structured FailureDetail API at call sites.
use crate::results::FailureDetail;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

pub fn load_spec(path: &Path, command: &str) -> Result<Value, FailureDetail> {
    let fail = |message: String| FailureDetail {
        kind: "soak_spec_invalid".to_string(),
        command: Some(command.to_string()),
        path: Some(path.display().to_string()),
        message: Some(message),
        expected: Some(json!("bounded soak specification version 1")),
        ..FailureDetail::default()
    };
    let raw = fs::read_to_string(path).map_err(|e| fail(e.to_string()))?;
    let mut value: Value = if path.extension().is_some_and(|e| e == "json") {
        serde_json::from_str(&raw).map_err(|e| fail(e.to_string()))?
    } else {
        crate::yaml::parse_yaml_document(&raw).map_err(|e| fail(format!("{e:?}")))?
    };
    let object = value
        .as_object_mut()
        .ok_or_else(|| fail("root must be an object".into()))?;
    for key in object.keys() {
        if ![
            "version",
            "warmup_seconds",
            "duration_seconds",
            "window_seconds",
            "state",
            "gauges",
            "budgets",
            "monitor_every_frames",
        ]
        .contains(&key.as_str())
        {
            return Err(fail(format!("unknown field {key}")));
        }
    }
    if object.get("version").and_then(Value::as_u64) != Some(1) {
        return Err(fail("version must be 1".into()));
    }
    object.entry("warmup_seconds").or_insert(json!(0));
    object.entry("duration_seconds").or_insert(json!(600));
    object.entry("window_seconds").or_insert(json!(5));
    object.entry("monitor_every_frames").or_insert(json!(60));
    let number = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite())
            .ok_or_else(|| fail(format!("{key} must be finite numeric")))
    };
    let warmup = number("warmup_seconds")?;
    let duration = number("duration_seconds")?;
    let window = number("window_seconds")?;
    if warmup < 0.0
        || !(0.0 < duration && duration <= 3600.0)
        || warmup + duration > 3600.0
        || !(1.0..=60.0).contains(&window)
        || (duration / window).ceil() > 720.0
        || ((duration / window) - (duration / window).round()).abs() > 0.000001
    {
        return Err(fail(
            "warmup, duration, or window outside limits (duration <=3600s, windows <=720)".into(),
        ));
    }
    if !(1..=3600).contains(
        &object
            .get("monitor_every_frames")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    ) {
        return Err(fail("monitor_every_frames must be 1..=3600".into()));
    }
    if let Some(state) = object.get("state") {
        let state = state
            .as_object()
            .ok_or_else(|| fail("state must be an object".into()))?;
        for key in ["provider", "epoch", "tick", "active"] {
            if state
                .get(key)
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            {
                return Err(fail(format!("state.{key} must be a nonempty string")));
            }
        }
        if state.contains_key("phase")
            && state
                .get("phase")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
        {
            return Err(fail("state.phase must be a nonempty path".into()));
        }
    }
    if let Some(budgets) = object.get("budgets") {
        let budgets = budgets
            .as_object()
            .ok_or_else(|| fail("budgets must be an object".into()))?;
        for (key, value) in budgets {
            if ![
                "frame_p95_ms",
                "frame_max_ms",
                "min_active_ticks_per_second",
            ]
            .contains(&key.as_str())
                || value.as_f64().is_none_or(|n| !n.is_finite() || n <= 0.0)
            {
                return Err(fail(format!("invalid budget {key}")));
            }
        }
        if budgets.contains_key("min_active_ticks_per_second") && !object.contains_key("state") {
            return Err(fail("throughput budget requires state mapping".into()));
        }
    }
    if let Some(gauges) = object.get("gauges") {
        let gauges = gauges
            .as_array()
            .ok_or_else(|| fail("gauges must be an array".into()))?;
        if gauges.len() > 32 || (!gauges.is_empty() && !object.contains_key("state")) {
            return Err(fail(
                "gauges require state mapping and at most 32 entries".into(),
            ));
        }
        let mut names = std::collections::HashSet::new();
        for gauge in gauges {
            let gauge = gauge
                .as_object()
                .ok_or_else(|| fail("gauge must be an object".into()))?;
            for key in ["name", "path"] {
                if gauge
                    .get(key)
                    .and_then(Value::as_str)
                    .is_none_or(str::is_empty)
                {
                    return Err(fail(format!("gauge.{key} must be nonempty")));
                }
            }
            if !names.insert(
                gauge
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            ) {
                return Err(fail("gauge names must be unique".into()));
            }
            for key in ["max", "final_max"] {
                if let Some(value) = gauge.get(key)
                    && value.as_f64().is_none_or(|n| !n.is_finite() || n < 0.0)
                {
                    return Err(fail(format!("gauge.{key} must be nonnegative numeric")));
                }
            }
        }
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::load_spec;
    use std::{env, fs};

    #[test]
    fn accepts_shared_600_second_spec_and_rejects_partial_window() {
        let path = env::temp_dir().join(format!("godot-qa-soak-{}.json", std::process::id()));
        fs::write(&path, r#"{"version":1,"state":{"provider":"qa_soak","epoch":"epoch","tick":"tick","active":"active"},"budgets":{"frame_p95_ms":40,"frame_max_ms":150,"min_active_ticks_per_second":55},"gauges":[{"name":"nodes","path":"counts.nodes","max":500,"final_max":50}]}"#).unwrap();
        let accepted = load_spec(&path, "scenario profile").unwrap();
        assert_eq!(accepted["duration_seconds"], 600);
        assert_eq!(accepted["window_seconds"], 5);
        fs::write(
            &path,
            r#"{"version":1,"duration_seconds":601,"window_seconds":5}"#,
        )
        .unwrap();
        assert!(load_spec(&path, "session profile").is_err());
        fs::remove_file(path).unwrap();
    }
}
