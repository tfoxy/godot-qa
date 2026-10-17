//! Which scenarios of a bundle a run is restricted to.
//!
//! The empty default means "run the whole bundle", which is what every caller that does
//! not narrow the run explicitly wants.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use godot_qa_core::results::FailureDetail;
use serde_json::{Value, json};

use crate::bundle_schedule::read_bundle_summary_record;
use crate::retention;

/// The command every selection failure names, so a caller never has to stamp it.
const SELECTION_COMMAND: &str = "scenario bundle run";

/// A narrowing of the scenarios a bundle run executes. `only` names scenarios directly;
/// `failed_only` restricts the run to the scenarios that failed in the previous run of the
/// same bundle. The two are independent filters and an empty/`false` value disables each.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BundleSelection {
    pub only: Vec<String>,
    pub failed_only: bool,
}

/// A [`BundleSelection`] resolved against one bundle's expansion.
#[derive(Debug, Clone)]
pub struct ResolvedBundleSelection {
    /// Indices into the member list, or `None` when the whole bundle runs. `None` and "every
    /// index" are deliberately different: only the former is the unfiltered path, which
    /// leaves messages, warm-skip counts and `--failed-only` baselines exactly as they were.
    pub selected: Option<BTreeSet<usize>>,
    pub selected_count: usize,
    pub member_count: usize,
    /// The run directory `--failed-only` read its baseline from, when it was used.
    pub source_run_root: Option<PathBuf>,
    /// `data.selection`, always emitted so a consumer never has to infer whether a run was
    /// filtered from the absence of a key.
    pub json: Value,
}

/// Resolve a selection against the scenarios a bundle expands to.
///
/// `member_names` is the expansion in order; the returned indices index into it, so the
/// caller keeps its own notion of what a child is and this module never needs to see it.
///
/// `--only` is matched by name, deduplicated, and any unknown name is a hard failure rather
/// than a silent no-op: a typo that quietly ran nothing would report a green bundle.
///
/// `--failed-only` reads the NEWEST run's `summary.json` regardless of what that run itself
/// selected, so repeated `--failed-only` invocations narrow monotonically and an unfiltered
/// run resets the baseline. A scenario counts as failed when its `runs[]` entry is not
/// `passed`, or when it is named by a `failures[]` entry — including a `missing_scenario`
/// preparation failure, which has no run and names the scenario in `actual`. Names the
/// bundle no longer contains are dropped, so an old summary cannot resurrect a scenario that
/// has since been removed from the manifest.
///
/// Both flags together intersect. An empty result is not an error — the previous run may
/// simply have been green — so it returns an empty index set plus a `selection_empty`
/// warning, and the run proceeds to write artifacts that record it ran nothing.
///
/// The returned warnings are JSON values shaped like a serialized [`FailureDetail`]
/// (`type`/`severity`/`message`/…), so a caller collecting `Vec<FailureDetail>` can
/// `serde_json::from_value` them and a caller collecting raw warning values can use them
/// as-is.
pub fn resolve_bundle_selection(
    artifacts_dir: &Path,
    bundle_name: &str,
    member_names: &[String],
    selection: &BundleSelection,
) -> Result<(ResolvedBundleSelection, Vec<Value>), FailureDetail> {
    let member_count = member_names.len();
    let only = dedupe_preserving_order(&selection.only);

    if only.is_empty() && !selection.failed_only {
        return Ok((
            ResolvedBundleSelection {
                selected: None,
                selected_count: member_count,
                member_count,
                source_run_root: None,
                json: json!({
                    "mode": "all",
                    "only": Vec::<String>::new(),
                    "failed_only": false,
                    "source_run_root": Value::Null,
                    "member_count": member_count,
                    "selected_ids": member_names,
                }),
            },
            Vec::new(),
        ));
    }

    let members: HashSet<&str> = member_names.iter().map(String::as_str).collect();
    let mut chosen: Option<HashSet<String>> = None;

    if !only.is_empty() {
        let unknown: Vec<&str> = only
            .iter()
            .map(String::as_str)
            .filter(|name| !members.contains(name))
            .collect();
        if !unknown.is_empty() {
            return Err(unknown_scenario_failure(
                bundle_name,
                member_names,
                &unknown,
            ));
        }
        chosen = Some(only.iter().cloned().collect());
    }

    let mut source_run_root = None;
    if selection.failed_only {
        let Some(run_root) = retention::latest_bundle_run_root(artifacts_dir, bundle_name) else {
            return Err(no_previous_run_failure(bundle_name, artifacts_dir, None));
        };
        let record = read_bundle_summary_record(&run_root)
            .map_err(|error| no_previous_run_failure(bundle_name, artifacts_dir, Some(&error)))?;
        let mut failed: HashSet<String> = HashSet::new();
        for run in &record.runs {
            // Anything that is not an explicit `passed` is worth rerunning, including a
            // status this version of the tool does not recognise.
            if !run.scenario.is_empty() && run.status.as_deref() != Some("passed") {
                failed.insert(run.scenario.clone());
            }
        }
        for failure in &record.failures {
            if let Some(scenario) = failure.scenario.as_deref() {
                failed.insert(scenario.to_string());
            } else if failure.kind == "missing_scenario" {
                if let Some(scenario) = failure.actual.as_ref().and_then(Value::as_str) {
                    failed.insert(scenario.to_string());
                }
            }
        }
        failed.retain(|name| members.contains(name.as_str()));
        source_run_root = Some(run_root);
        chosen = Some(match chosen {
            Some(only) => only.intersection(&failed).cloned().collect(),
            None => failed,
        });
    }

    let chosen = chosen.unwrap_or_default();
    let selected: BTreeSet<usize> = member_names
        .iter()
        .enumerate()
        .filter(|(_, name)| chosen.contains(name.as_str()))
        .map(|(index, _)| index)
        .collect();
    let selected_ids: Vec<&String> = selected
        .iter()
        .filter_map(|index| member_names.get(*index))
        .collect();

    let mut warnings = Vec::new();
    if selected.is_empty() {
        warnings.push(empty_selection_warning(bundle_name, selection));
    }

    let json = json!({
        "mode": "filtered",
        "only": only,
        "failed_only": selection.failed_only,
        "source_run_root": source_run_root
            .as_ref()
            .map(|path: &PathBuf| json!(path))
            .unwrap_or(Value::Null),
        "member_count": member_count,
        "selected_ids": selected_ids,
    });

    Ok((
        ResolvedBundleSelection {
            selected_count: selected.len(),
            selected: Some(selected),
            member_count,
            source_run_root,
            json,
        },
        warnings,
    ))
}

fn dedupe_preserving_order(names: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    names
        .iter()
        .filter(|name| seen.insert(name.as_str()))
        .cloned()
        .collect()
}

fn unknown_scenario_failure(
    bundle_name: &str,
    member_names: &[String],
    unknown: &[&str],
) -> FailureDetail {
    FailureDetail {
        kind: "selection_unknown_scenario".to_string(),
        command: Some(SELECTION_COMMAND.to_string()),
        message: Some(format!(
            "--only named {} that bundle '{bundle_name}' does not contain: {}",
            if unknown.len() == 1 {
                "a scenario"
            } else {
                "scenarios"
            },
            unknown.join(", ")
        )),
        expected: Some(json!(member_names)),
        actual: Some(json!(unknown)),
        severity: Some("error".to_string()),
        details: serde_json::Map::from_iter([
            ("bundle".to_string(), json!(bundle_name)),
            (
                "remediation".to_string(),
                json!(
                    "run `godot-qa scenario bundle list --json` to see the scenarios this bundle expands to, then correct or drop the --only name"
                ),
            ),
        ]),
        ..FailureDetail::default()
    }
}

fn no_previous_run_failure(
    bundle_name: &str,
    artifacts_dir: &Path,
    unreadable: Option<&str>,
) -> FailureDetail {
    let run_parent = artifacts_dir.join("bundles").join(bundle_name);
    FailureDetail {
        kind: "selection_no_previous_run".to_string(),
        command: Some(SELECTION_COMMAND.to_string()),
        path: Some(run_parent.to_string_lossy().into_owned()),
        message: Some(match unreadable {
            Some(error) => format!(
                "--failed-only could not read the previous run of bundle '{bundle_name}': {error}"
            ),
            None => {
                format!("bundle '{bundle_name}' has no previous run to take --failed-only from")
            }
        }),
        expected: Some(json!("a previous bundle run with a readable summary.json")),
        actual: Some(match unreadable {
            Some(error) => json!(format!("unreadable: {error}")),
            None => json!("no previous run"),
        }),
        severity: Some("error".to_string()),
        details: serde_json::Map::from_iter([
            ("bundle".to_string(), json!(bundle_name)),
            (
                "remediation".to_string(),
                json!("run the bundle once without --failed-only to record a baseline"),
            ),
        ]),
        ..FailureDetail::default()
    }
}

fn empty_selection_warning(bundle_name: &str, selection: &BundleSelection) -> Value {
    let reason = if selection.failed_only && !selection.only.is_empty() {
        "no scenario matched both --only and the previous run's failures"
    } else if selection.failed_only {
        "the previous run had no failures"
    } else {
        "the selection matched no scenario"
    };
    json!({
        "type": "selection_empty",
        "severity": "warning",
        "bundle": bundle_name,
        "message": format!("bundle '{bundle_name}' ran no scenarios: {reason}"),
        "remediation": "this is the expected result of --failed-only after a green run; drop the flag to run the whole bundle",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "godot-qa-selection-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("create temp root");
        root
    }

    fn members(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    /// Write a bundle run `summary.json` in the shape `prepare_bundle_artifacts` produces.
    fn write_summary(
        root: &Path,
        bundle: &str,
        epoch: u64,
        runs: &[(&str, &str)],
        failures: Value,
    ) {
        let run_root = root.join("bundles").join(bundle).join(epoch.to_string());
        fs::create_dir_all(&run_root).expect("create run root");
        let runs: Vec<Value> = runs
            .iter()
            .map(|(scenario, status)| {
                json!({"scenario": scenario, "status": status, "ok": *status == "passed", "duration_ms": 10.0})
            })
            .collect();
        fs::write(
            run_root.join("summary.json"),
            serde_json::to_vec_pretty(
                &json!({"bundle": bundle, "runs": runs, "failures": failures}),
            )
            .expect("serialize summary"),
        )
        .expect("write summary.json");
    }

    fn only(names: &[&str]) -> BundleSelection {
        BundleSelection {
            only: members(names),
            failed_only: false,
        }
    }

    #[test]
    fn default_selection_runs_the_whole_bundle() {
        let selection = BundleSelection::default();
        assert!(selection.only.is_empty());
        assert!(!selection.failed_only);
    }

    #[test]
    fn no_flags_leaves_the_run_unfiltered_but_still_reports_the_selection() {
        let root = temp_root("all");
        let member_names = members(&["a", "b", "c"]);

        let (resolved, warnings) =
            resolve_bundle_selection(&root, "gate", &member_names, &BundleSelection::default())
                .expect("resolve");

        // `None` rather than "every index": the unfiltered path must stay byte-identical.
        assert!(resolved.selected.is_none());
        assert_eq!(resolved.selected_count, 3);
        assert_eq!(resolved.member_count, 3);
        assert_eq!(resolved.source_run_root, None);
        assert!(warnings.is_empty());
        assert_eq!(
            resolved.json,
            json!({
                "mode": "all",
                "only": [],
                "failed_only": false,
                "source_run_root": Value::Null,
                "member_count": 3,
                "selected_ids": ["a", "b", "c"],
            })
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn only_selects_by_name_in_member_order_and_collapses_duplicates() {
        let root = temp_root("only");
        let member_names = members(&["a", "b", "c", "d"]);

        let (resolved, warnings) = resolve_bundle_selection(
            &root,
            "gate",
            &member_names,
            // Out of member order and repeated: neither should matter.
            &only(&["c", "a", "c"]),
        )
        .expect("resolve");

        assert_eq!(resolved.selected, Some(BTreeSet::from([0, 2])));
        assert_eq!(resolved.selected_count, 2);
        assert_eq!(resolved.member_count, 4);
        assert!(warnings.is_empty());
        assert_eq!(resolved.json["mode"], "filtered");
        // The flag list keeps first-seen order; the selected ids are in member order.
        assert_eq!(resolved.json["only"], json!(["c", "a"]));
        assert_eq!(resolved.json["selected_ids"], json!(["a", "c"]));
        assert_eq!(resolved.json["source_run_root"], Value::Null);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unknown_only_name_is_a_failure_that_lists_the_members() {
        let root = temp_root("only-unknown");
        let member_names = members(&["a", "b"]);

        let failure =
            resolve_bundle_selection(&root, "gate", &member_names, &only(&["a", "typo", "gone"]))
                .expect_err("unknown name must fail");

        assert_eq!(failure.kind, "selection_unknown_scenario");
        assert_eq!(failure.command.as_deref(), Some("scenario bundle run"));
        assert_eq!(failure.severity.as_deref(), Some("error"));
        assert_eq!(failure.expected, Some(json!(["a", "b"])));
        assert_eq!(failure.actual, Some(json!(["typo", "gone"])));
        assert_eq!(failure.details["bundle"], "gate");
        assert!(
            failure.details["remediation"]
                .as_str()
                .expect("remediation")
                .contains("scenario bundle list")
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn failed_only_reads_the_newest_run_and_ignores_names_no_longer_in_the_bundle() {
        let root = temp_root("failed-only");
        let member_names = members(&["a", "b", "c"]);
        // An older run whose failures must NOT leak into the baseline.
        write_summary(
            &root,
            "gate",
            1000,
            &[("a", "failed"), ("b", "passed"), ("c", "passed")],
            json!([{"type": "assertion_failed", "scenario": "a"}]),
        );
        // The newest run: `b` failed, plus a `missing_scenario` for a name the bundle no
        // longer contains and a failure naming `c`.
        write_summary(
            &root,
            "gate",
            1001,
            &[("a", "passed"), ("b", "failed")],
            json!([
                {"type": "assertion_failed", "scenario": "b"},
                {"type": "missing_scenario", "actual": "c"},
                {"type": "missing_scenario", "actual": "retired_scenario"},
            ]),
        );

        let (resolved, warnings) = resolve_bundle_selection(
            &root,
            "gate",
            &member_names,
            &BundleSelection {
                only: Vec::new(),
                failed_only: true,
            },
        )
        .expect("resolve");

        // `b` from its run status and its failure, `c` from the missing_scenario `actual`;
        // `a` passed in the newest run even though it failed in the older one, and
        // `retired_scenario` is not a member any more.
        assert_eq!(resolved.selected, Some(BTreeSet::from([1, 2])));
        assert_eq!(resolved.json["selected_ids"], json!(["b", "c"]));
        assert_eq!(resolved.json["failed_only"], true);
        assert_eq!(
            resolved.source_run_root,
            Some(root.join("bundles").join("gate").join("1001"))
        );
        assert_eq!(
            resolved.json["source_run_root"],
            json!(root.join("bundles").join("gate").join("1001"))
        );
        assert!(warnings.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn failed_only_after_a_green_run_selects_nothing_and_warns() {
        let root = temp_root("failed-only-green");
        let member_names = members(&["a", "b"]);
        write_summary(
            &root,
            "gate",
            1000,
            &[("a", "passed"), ("b", "passed")],
            json!([]),
        );

        let (resolved, warnings) = resolve_bundle_selection(
            &root,
            "gate",
            &member_names,
            &BundleSelection {
                only: Vec::new(),
                failed_only: true,
            },
        )
        .expect("an empty selection is not an error");

        assert_eq!(resolved.selected, Some(BTreeSet::new()));
        assert_eq!(resolved.selected_count, 0);
        assert_eq!(resolved.json["selected_ids"], json!([]));
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0]["type"], "selection_empty");
        assert_eq!(warnings[0]["severity"], "warning");
        assert!(
            warnings[0]["message"]
                .as_str()
                .expect("message")
                .contains("gate")
        );
        // The warning is shaped like a serialized FailureDetail, so a caller collecting
        // those can take it as-is.
        let as_detail: FailureDetail =
            serde_json::from_value(warnings[0].clone()).expect("warning round-trips");
        assert_eq!(as_detail.kind, "selection_empty");
        assert_eq!(as_detail.severity.as_deref(), Some("warning"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn both_flags_intersect() {
        let root = temp_root("intersect");
        let member_names = members(&["a", "b", "c"]);
        write_summary(
            &root,
            "gate",
            1000,
            &[("a", "failed"), ("b", "failed"), ("c", "passed")],
            json!([]),
        );

        // --only a c  ∩  failed {a, b}  =  {a}
        let (resolved, warnings) = resolve_bundle_selection(
            &root,
            "gate",
            &member_names,
            &BundleSelection {
                only: members(&["a", "c"]),
                failed_only: true,
            },
        )
        .expect("resolve");
        assert_eq!(resolved.selected, Some(BTreeSet::from([0])));
        assert_eq!(resolved.json["only"], json!(["a", "c"]));
        assert!(warnings.is_empty());

        // A disjoint intersection is empty rather than an error.
        let (resolved, warnings) = resolve_bundle_selection(
            &root,
            "gate",
            &member_names,
            &BundleSelection {
                only: members(&["c"]),
                failed_only: true,
            },
        )
        .expect("resolve");
        assert_eq!(resolved.selected, Some(BTreeSet::new()));
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0]["type"], "selection_empty");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn failed_only_needs_a_readable_previous_run() {
        let root = temp_root("failed-only-missing");
        let member_names = members(&["a"]);
        let selection = BundleSelection {
            only: Vec::new(),
            failed_only: true,
        };

        let failure = resolve_bundle_selection(&root, "gate", &member_names, &selection)
            .expect_err("no previous run must fail");
        assert_eq!(failure.kind, "selection_no_previous_run");
        assert_eq!(failure.command.as_deref(), Some("scenario bundle run"));
        assert_eq!(failure.actual, Some(json!("no previous run")));
        assert!(
            failure.details["remediation"]
                .as_str()
                .expect("remediation")
                .contains("without --failed-only")
        );

        // A run directory whose summary cannot be parsed reports the same kind, with the
        // reason in `actual` so the operator can tell "never ran" from "corrupt".
        let run_root = root.join("bundles").join("gate").join("1000");
        fs::create_dir_all(&run_root).expect("create run root");
        fs::write(run_root.join("summary.json"), "{not json").expect("write broken summary");
        let failure = resolve_bundle_selection(&root, "gate", &member_names, &selection)
            .expect_err("unreadable summary must fail");
        assert_eq!(failure.kind, "selection_no_previous_run");
        assert!(
            failure
                .actual
                .as_ref()
                .and_then(Value::as_str)
                .expect("actual")
                .starts_with("unreadable: ")
        );
        let _ = fs::remove_dir_all(&root);
    }
}
