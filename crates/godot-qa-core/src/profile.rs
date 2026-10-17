use serde_json::{Map, Value, json};
use std::collections::HashMap;

pub const PROFILE_MAX_FRAME_SAMPLES: usize = 600;

#[derive(Debug, Clone, PartialEq)]
pub struct ProfilePercentiles {
    pub sample_count: usize,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub p50: Option<f64>,
    pub p90: Option<f64>,
    pub p95: Option<f64>,
    pub p99: Option<f64>,
}

pub fn profile_percentiles(samples: &[f64]) -> ProfilePercentiles {
    if samples.is_empty() {
        return ProfilePercentiles {
            sample_count: 0,
            min: None,
            max: None,
            p50: None,
            p90: None,
            p95: None,
            p99: None,
        };
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let pick = |percentile: f64| -> f64 {
        let index = ((sorted.len() - 1) as f64 * percentile).ceil() as usize;
        sorted[index]
    };
    ProfilePercentiles {
        sample_count: sorted.len(),
        min: sorted.first().copied(),
        max: sorted.last().copied(),
        p50: Some(pick(0.50)),
        p90: Some(pick(0.90)),
        p95: Some(pick(0.95)),
        p99: Some(pick(0.99)),
    }
}

pub fn profile_percentiles_json(percentiles: &ProfilePercentiles) -> Value {
    json!({
        "sample_count": percentiles.sample_count,
        "min": percentiles.min,
        "max": percentiles.max,
        "p50": percentiles.p50,
        "p90": percentiles.p90,
        "p95": percentiles.p95,
        "p99": percentiles.p99,
    })
}

pub fn optional_number_array(value: Option<&Value>) -> Result<Vec<f64>, Value> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let Some(values) = value.as_array() else {
        return Err(json!(json_type_name(value)));
    };
    let mut numbers = Vec::new();
    for item in values {
        let Some(number) = item
            .as_f64()
            .filter(|number| number.is_finite() && *number >= 0.0)
        else {
            return Err(json!(json_type_name(item)));
        };
        numbers.push(number);
    }
    Ok(numbers)
}

pub fn optional_object_array(value: Option<&Value>) -> Result<Vec<Value>, Value> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let Some(values) = value.as_array() else {
        return Err(json!(json_type_name(value)));
    };
    if let Some(invalid) = values.iter().find(|item| !item.is_object()) {
        return Err(json!(json_type_name(invalid)));
    }
    Ok(values.clone())
}

pub fn bounded_frame_values(frame_samples: &[f64], max_frame_samples: usize) -> Vec<Value> {
    frame_samples
        .iter()
        .take(max_frame_samples)
        .map(|sample| json!(sample))
        .collect()
}

pub fn monitor_counter_summaries(samples: &[Value]) -> Value {
    let mut counters: Map<String, Value> = Map::new();
    for sample in samples {
        let Some(object) = sample.as_object() else {
            continue;
        };
        let Some(name) = object
            .get("name")
            .or_else(|| object.get("counter"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let Some(value) = object.get("value").and_then(Value::as_f64) else {
            continue;
        };
        let entry = counters.entry(name.to_string()).or_insert_with(|| {
            json!({
                "sample_count": 0,
                "min": value,
                "max": value,
                "latest": value,
            })
        });
        if let Some(entry) = entry.as_object_mut() {
            let sample_count = entry
                .get("sample_count")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                + 1;
            let min = entry
                .get("min")
                .and_then(Value::as_f64)
                .unwrap_or(value)
                .min(value);
            let max = entry
                .get("max")
                .and_then(Value::as_f64)
                .unwrap_or(value)
                .max(value);
            entry.insert("sample_count".to_string(), json!(sample_count));
            entry.insert("min".to_string(), json!(min));
            entry.insert("max".to_string(), json!(max));
            entry.insert("latest".to_string(), json!(value));
        }
    }
    Value::Object(counters)
}

pub fn empty_top_script_methods_json() -> Value {
    json!({
        "status": "available",
        "instrumentation": "opt_in",
        "method_count": 0,
        "sample_count": 0,
        "methods": [],
    })
}

pub fn top_script_methods_or_empty(value: Option<&Value>) -> Value {
    value.cloned().unwrap_or_else(empty_top_script_methods_json)
}

pub fn aggregate_top_script_methods(values: &[Value], top: usize) -> Value {
    let mut summaries: HashMap<String, ScriptMethodSummary> = HashMap::new();
    let mut sample_count = 0_u64;
    for value in values {
        let Some(object) = value.as_object() else {
            continue;
        };
        sample_count += object
            .get("sample_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        for method in object
            .get("methods")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(method_object) = method.as_object() else {
                continue;
            };
            let Some(label) = method_object.get("label").and_then(Value::as_str) else {
                continue;
            };
            if label.is_empty() {
                continue;
            }
            let entry = summaries
                .entry(label.to_string())
                .or_insert_with(|| ScriptMethodSummary {
                    label: label.to_string(),
                    ..ScriptMethodSummary::default()
                });
            entry.call_count += method_object
                .get("call_count")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            entry.total_ms += method_object
                .get("total_ms")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            entry.self_ms += method_object
                .get("self_ms")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            entry.max_ms = entry.max_ms.max(
                method_object
                    .get("max_ms")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0),
            );
            if let Some(last_ms) = method_object.get("last_ms").and_then(Value::as_f64) {
                entry.last_ms = last_ms;
            }
        }
    }
    let method_count = summaries.len();
    let mut methods = summaries.into_values().collect::<Vec<_>>();
    methods.sort_by(|left, right| {
        right
            .total_ms
            .total_cmp(&left.total_ms)
            .then_with(|| right.call_count.cmp(&left.call_count))
            .then_with(|| left.label.cmp(&right.label))
    });
    if top > 0 {
        methods.truncate(top);
    }
    let methods = methods
        .into_iter()
        .map(|method| {
            let avg_ms = if method.call_count > 0 {
                method.total_ms / method.call_count as f64
            } else {
                0.0
            };
            json!({
                "label": method.label,
                "call_count": method.call_count,
                "total_ms": method.total_ms,
                "self_ms": method.self_ms,
                "avg_ms": avg_ms,
                "max_ms": method.max_ms,
                "last_ms": method.last_ms,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "status": "available",
        "instrumentation": "opt_in",
        "method_count": method_count,
        "sample_count": sample_count,
        "methods": methods,
    })
}

#[derive(Debug, Default)]
struct ScriptMethodSummary {
    label: String,
    call_count: u64,
    total_ms: f64,
    self_ms: f64,
    max_ms: f64,
    last_ms: f64,
}

pub fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

#[cfg(test)]
mod tests {
    use super::{aggregate_top_script_methods, monitor_counter_summaries, profile_percentiles};
    use serde_json::json;

    #[test]
    fn profile_percentiles_empty_samples_are_deterministic() {
        let percentiles = profile_percentiles(&[]);
        assert_eq!(percentiles.sample_count, 0);
        assert_eq!(percentiles.min, None);
        assert_eq!(percentiles.p99, None);
    }

    #[test]
    fn profile_percentiles_use_nearest_rank_ceiling() {
        let percentiles = profile_percentiles(&[1.0, 100.0, 2.0, 3.0, 4.0]);
        assert_eq!(percentiles.sample_count, 5);
        assert_eq!(percentiles.min, Some(1.0));
        assert_eq!(percentiles.max, Some(100.0));
        assert_eq!(percentiles.p50, Some(3.0));
        assert_eq!(percentiles.p90, Some(100.0));
    }

    #[test]
    fn monitor_counter_summaries_track_min_max_latest() {
        let samples = vec![
            json!({"name": "time/process", "value": 3.0}),
            json!({"name": "time/process", "value": 1.0}),
            json!({"name": "time/process", "value": 2.0}),
        ];
        let summary = monitor_counter_summaries(&samples);
        assert_eq!(summary["time/process"]["sample_count"], 3);
        assert_eq!(summary["time/process"]["min"], 1.0);
        assert_eq!(summary["time/process"]["max"], 3.0);
        assert_eq!(summary["time/process"]["latest"], 2.0);
    }

    #[test]
    fn aggregate_top_script_methods_merges_labels_by_total_time() {
        let summary = aggregate_top_script_methods(
            &[
                json!({
                    "status": "available",
                    "instrumentation": "opt_in",
                    "sample_count": 2,
                    "methods": [
                        {"label": "A._process", "call_count": 2, "total_ms": 5.0, "self_ms": 3.0, "max_ms": 4.0, "last_ms": 1.0}
                    ]
                }),
                json!({
                    "status": "available",
                    "instrumentation": "opt_in",
                    "sample_count": 1,
                    "methods": [
                        {"label": "A._process", "call_count": 1, "total_ms": 2.0, "self_ms": 1.5, "max_ms": 2.0, "last_ms": 2.0},
                        {"label": "B._ready", "call_count": 1, "total_ms": 9.0, "self_ms": 9.0, "max_ms": 9.0, "last_ms": 9.0}
                    ]
                }),
            ],
            10,
        );
        assert_eq!(summary["sample_count"], 3);
        assert_eq!(summary["method_count"], 2);
        assert_eq!(summary["methods"][0]["label"], "B._ready");
        assert_eq!(summary["methods"][1]["label"], "A._process");
        assert_eq!(summary["methods"][1]["call_count"], 3);
        assert_eq!(summary["methods"][1]["total_ms"], 7.0);
        assert_eq!(summary["methods"][1]["avg_ms"], 7.0 / 3.0);
    }
}
