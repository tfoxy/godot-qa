// `--equivalence-check`: comparing the logical verdict of a cold and a warm run
// of the same scenario.
//
// Included into `run/warm.rs`, which is itself included into the run module, so
// this file shares the whole `crate::run` scope.

/// Compare the logical verdict (ok + failure-kind multiset + snapshot names) of
/// a cold and a warm run of the same scenario. Returns a divergence record if
/// they differ, else None. Timestamps, ids, paths, profiling and display are
/// deliberately ignored — only the logical outcome is compared.
fn compare_equivalence(scenario: &Value, cold: &CommandResult, warm: &CommandResult) -> Option<Value> {
    let cold_kinds = failure_kind_multiset(cold);
    let warm_kinds = failure_kind_multiset(warm);
    let cold_snaps = snapshot_name_set(cold);
    let warm_snaps = snapshot_name_set(warm);

    let only_in_cold: Vec<String> = difference(&cold_kinds, &warm_kinds);
    let only_in_warm: Vec<String> = difference(&warm_kinds, &cold_kinds);
    let snaps_only_cold: Vec<String> = cold_snaps
        .iter()
        .filter(|name| !warm_snaps.contains(*name))
        .cloned()
        .collect();
    let snaps_only_warm: Vec<String> = warm_snaps
        .iter()
        .filter(|name| !cold_snaps.contains(*name))
        .cloned()
        .collect();

    if cold.ok == warm.ok
        && only_in_cold.is_empty()
        && only_in_warm.is_empty()
        && snaps_only_cold.is_empty()
        && snaps_only_warm.is_empty()
    {
        return None;
    }
    Some(json!({
        "scenario": scenario.get("name").and_then(Value::as_str).unwrap_or_default(),
        "cold_ok": cold.ok,
        "warm_ok": warm.ok,
        "failures_only_in_cold": only_in_cold,
        "failures_only_in_warm": only_in_warm,
        "snapshots_only_in_cold": snaps_only_cold,
        "snapshots_only_in_warm": snaps_only_warm,
    }))
}

fn failure_kind_multiset(result: &CommandResult) -> Vec<String> {
    let mut kinds: Vec<String> = result
        .failures
        .iter()
        .map(|failure| failure.kind.clone())
        .collect();
    kinds.sort();
    kinds
}

fn snapshot_name_set(result: &CommandResult) -> Vec<String> {
    result
        .data
        .get("snapshots")
        .and_then(Value::as_array)
        .map(|snapshots| {
            snapshots
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

// Multiset difference: every element of `a` not matched by an element of `b`.
fn difference(a: &[String], b: &[String]) -> Vec<String> {
    let mut remaining: Vec<String> = b.to_vec();
    let mut out = Vec::new();
    for item in a {
        if let Some(position) = remaining.iter().position(|candidate| candidate == item) {
            remaining.remove(position);
        } else {
            out.push(item.clone());
        }
    }
    out
}
