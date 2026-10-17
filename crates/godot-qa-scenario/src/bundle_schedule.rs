//! Dispatch order for the scenarios of a bundle run.
//!
//! This is the runner-side counterpart of [`godot_qa_core::config::ScenarioSchedule`]
//! (`scenario.schedule` in `qa/config.yaml`); the config enum is the parsed project
//! setting, this one is the policy the bundle runner actually carries.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::str::FromStr;

use godot_qa_core::config::ScenarioSchedule;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::retention;

/// How the bundle runner orders scenarios before dispatching them.
///
/// `Manifest` preserves bundle expansion order, which keeps a run reproducible and easy to
/// read against the manifest. `LongestFirst` — the default — dispatches the scenarios with
/// the longest known durations first so the slowest one does not land last and stretch the
/// parallel tail.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BundleSchedulePolicy {
    Manifest,
    #[default]
    LongestFirst,
}

impl BundleSchedulePolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            BundleSchedulePolicy::Manifest => "manifest",
            BundleSchedulePolicy::LongestFirst => "longest_first",
        }
    }
}

impl FromStr for BundleSchedulePolicy {
    type Err = String;

    /// Both spellings of the hyphenated CLI form are accepted so `--schedule longest-first`
    /// and `schedule: longest_first` in YAML name the same policy.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "manifest" => Ok(BundleSchedulePolicy::Manifest),
            "longest-first" | "longest_first" => Ok(BundleSchedulePolicy::LongestFirst),
            other => Err(format!(
                "unknown bundle schedule: {other} (expected manifest or longest-first)"
            )),
        }
    }
}

impl From<ScenarioSchedule> for BundleSchedulePolicy {
    fn from(value: ScenarioSchedule) -> Self {
        match value {
            ScenarioSchedule::Manifest => BundleSchedulePolicy::Manifest,
            ScenarioSchedule::LongestFirst => BundleSchedulePolicy::LongestFirst,
        }
    }
}

/// How many of a bundle's most recent runs longest-first dispatch reads durations from.
///
/// Five is enough to survive one anomalous run without turning history into a deep scan:
/// each entry costs one `summary.json` read on the bundle's critical path, and a scenario
/// whose cost changed genuinely wants the newest measurement anyway.
pub const HISTORY_RUNS: usize = 5;

/// The slice of a bundle `summary.json` that scheduling and selection read.
///
/// Deliberately partial and lenient. The file on disk was written by whatever version of
/// the tool ran last, so unknown fields are ignored and every field a reader needs is
/// optional: a history read must never fail because a future release added a key, and a
/// selection baseline must degrade to a clear "no usable previous run" rather than a parse
/// panic.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct BundleSummaryRecord {
    #[serde(default)]
    pub(crate) runs: Vec<BundleSummaryRun>,
    #[serde(default)]
    pub(crate) failures: Vec<BundleSummaryFailure>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BundleSummaryRun {
    #[serde(default)]
    pub(crate) scenario: String,
    /// The child's engine-reported wall clock, or `null` when it produced no result to read
    /// one from (a startup failure, for instance). A null is not a zero: it means unknown,
    /// so scheduling keeps looking further back for that scenario.
    #[serde(default)]
    pub(crate) duration_ms: Option<f64>,
    #[serde(default)]
    pub(crate) status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BundleSummaryFailure {
    #[serde(rename = "type", default)]
    pub(crate) kind: String,
    /// The bundle runner stamps this onto every child failure. Preparation failures such as
    /// `missing_scenario` have no run to stamp and name the scenario in `actual` instead.
    #[serde(default)]
    pub(crate) scenario: Option<String>,
    #[serde(default)]
    pub(crate) actual: Option<Value>,
}

/// Read and parse `<run_root>/summary.json`.
///
/// The error is the human-readable reason, which selection surfaces verbatim as
/// `actual: "unreadable: <err>"`; scheduling just drops the run from its history.
pub(crate) fn read_bundle_summary_record(run_root: &Path) -> Result<BundleSummaryRecord, String> {
    let summary_path = run_root.join("summary.json");
    let raw = fs::read_to_string(&summary_path).map_err(|error| error.to_string())?;
    serde_json::from_str::<BundleSummaryRecord>(&raw).map_err(|error| error.to_string())
}

/// Per-scenario duration predictions distilled from a bundle's recent runs.
///
/// A scenario is absent when no run in the window recorded a duration for it, which is the
/// case that matters: an unknown cost is treated as unbounded by [`order_for_dispatch`].
#[derive(Debug, Clone, Default)]
pub struct DurationHistory {
    by_scenario: HashMap<String, f64>,
    history_runs: usize,
}

impl DurationHistory {
    /// How many `summary.json` files actually contributed. Unreadable ones are skipped and
    /// never counted, so this is an honest denominator for `data.scheduling`.
    pub fn history_runs(&self) -> usize {
        self.history_runs
    }

    /// The predicted engine duration for `scenario` in milliseconds, or `None` when no run
    /// in the window recorded one.
    pub fn predicted_ms(&self, scenario: &str) -> Option<f64> {
        self.by_scenario.get(scenario).copied()
    }

    /// How many distinct scenarios have a prediction.
    pub fn len(&self) -> usize {
        self.by_scenario.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_scenario.is_empty()
    }
}

/// Build a duration history from the newest `max_runs` readable `summary.json` files under
/// `<artifacts_dir>/bundles/<bundle_name>/`.
///
/// Newest wins: the first non-null `duration_ms` seen for a scenario is kept, so a scenario
/// that failed to start in the last run still gets its prediction from the run before. A
/// summary that cannot be read or parsed is skipped and does not consume one of the
/// `max_runs` slots — otherwise a single corrupt file would blind the scheduler for a whole
/// window.
pub fn read_duration_history(
    artifacts_dir: &Path,
    bundle_name: &str,
    max_runs: usize,
) -> DurationHistory {
    let mut history = DurationHistory::default();
    if max_runs == 0 {
        return history;
    }
    for (_, run_root) in retention::bundle_run_roots_newest_first(artifacts_dir, bundle_name) {
        let Ok(record) = read_bundle_summary_record(&run_root) else {
            continue;
        };
        history.history_runs += 1;
        for run in record.runs {
            if run.scenario.is_empty() {
                continue;
            }
            if let Some(duration) = run.duration_ms {
                history.by_scenario.entry(run.scenario).or_insert(duration);
            }
        }
        if history.history_runs >= max_runs {
            break;
        }
    }
    history
}

/// What the scheduler decided for one bundle run, reported as `data.scheduling`.
///
/// `applied` is false whenever the order was left alone — the `manifest` policy, a serial
/// run, or an empty history — and in that case no history was read and `history_runs` is 0.
#[derive(Debug, Clone)]
pub struct SchedulePlan {
    pub policy: BundleSchedulePolicy,
    /// Where the effective policy came from: `"flag"`, `"config"`, or `"default"`.
    pub origin: String,
    pub applied: bool,
    pub history_runs: usize,
    /// How many dispatched scenarios had a predicted duration.
    pub predicted: usize,
    /// How many did not, and were therefore dispatched first.
    pub unknown: usize,
}

impl SchedulePlan {
    pub fn to_json(&self) -> Value {
        json!({
            "policy": self.policy.as_str(),
            "origin": self.origin,
            "applied": self.applied,
            "history_runs": self.history_runs,
            "predicted": self.predicted,
            "unknown": self.unknown,
        })
    }
}

/// Reorder `tasks` longest-predicted-first, returning the new order plus how many tasks had
/// a prediction and how many did not.
///
/// Unknown durations sort FIRST, not last: a scenario nobody has timed is unbounded, and the
/// only way to bound the parallel tail is to start it as early as possible. That also makes
/// the first-ever run of a bundle degenerate to plain expansion order, because every task is
/// unknown and the sort is stable — so a project sees no reordering until it has history.
/// Ties keep their relative order for the same reason: the manifest is the tiebreaker, which
/// keeps a run readable against the file that declared it.
pub fn order_for_dispatch<T>(
    tasks: Vec<T>,
    name_of: impl Fn(&T) -> &str,
    history: &DurationHistory,
) -> (Vec<T>, usize, usize) {
    let mut predicted = 0usize;
    let mut unknown = 0usize;
    let mut keyed: Vec<(f64, T)> = Vec::with_capacity(tasks.len());
    for task in tasks {
        match history.predicted_ms(name_of(&task)) {
            Some(duration) => {
                predicted += 1;
                keyed.push((duration, task));
            }
            None => {
                unknown += 1;
                keyed.push((f64::INFINITY, task));
            }
        }
    }
    // `sort_by` is stable, so equal keys — including every task on a first run, which all
    // key as INFINITY — come out in the order they went in.
    keyed.sort_by(|left, right| {
        right
            .0
            .partial_cmp(&left.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    (
        keyed.into_iter().map(|(_, task)| task).collect(),
        predicted,
        unknown,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "godot-qa-schedule-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("create temp root");
        root
    }

    /// Write a bundle run directory holding a `summary.json` with the given
    /// `(scenario, duration_ms)` runs, matching the shape `prepare_bundle_artifacts` writes.
    fn write_summary(root: &Path, bundle: &str, epoch: u64, runs: &[(&str, Option<f64>)]) {
        let run_root = root.join("bundles").join(bundle).join(epoch.to_string());
        fs::create_dir_all(&run_root).expect("create run root");
        let runs: Vec<Value> = runs
            .iter()
            .map(|(scenario, duration_ms)| {
                json!({"scenario": scenario, "status": "passed", "ok": true, "duration_ms": duration_ms})
            })
            .collect();
        fs::write(
            run_root.join("summary.json"),
            serde_json::to_vec_pretty(&json!({"bundle": bundle, "runs": runs, "failures": []}))
                .expect("serialize summary"),
        )
        .expect("write summary.json");
    }

    fn names(tasks: &[&str]) -> Vec<String> {
        tasks.iter().map(|name| name.to_string()).collect()
    }

    fn dispatch(tasks: &[&str], history: &DurationHistory) -> (Vec<String>, usize, usize) {
        let (ordered, predicted, unknown) =
            order_for_dispatch(names(tasks), |task| task.as_str(), history);
        (ordered, predicted, unknown)
    }

    #[test]
    fn dispatch_puts_the_slowest_first_and_unknowns_ahead_of_everything() {
        let root = temp_root("dispatch-desc");
        write_summary(
            &root,
            "gate",
            1000,
            &[
                ("quick", Some(120.0)),
                ("slow", Some(9000.0)),
                ("middling", Some(2500.0)),
            ],
        );
        let history = read_duration_history(&root, "gate", HISTORY_RUNS);

        let (ordered, predicted, unknown) =
            dispatch(&["quick", "middling", "brand_new", "slow"], &history);

        // `brand_new` has no measurement, so its cost is unbounded and it goes out first;
        // the rest follow longest to shortest.
        assert_eq!(ordered, names(&["brand_new", "slow", "middling", "quick"]));
        assert_eq!((predicted, unknown), (3, 1));
        assert_eq!(history.history_runs(), 1);
        assert_eq!(history.len(), 3);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn dispatch_is_stable_across_ties_and_an_empty_history() {
        let root = temp_root("dispatch-stable");
        write_summary(
            &root,
            "gate",
            1000,
            &[("a", Some(500.0)), ("b", Some(500.0)), ("c", Some(500.0))],
        );
        let history = read_duration_history(&root, "gate", HISTORY_RUNS);

        let (ordered, predicted, unknown) = dispatch(&["c", "a", "b"], &history);
        assert_eq!(ordered, names(&["c", "a", "b"]), "equal costs keep order");
        assert_eq!((predicted, unknown), (3, 0));

        // The first-ever run of a bundle: everything is unknown, so the reorder degenerates
        // to plain expansion order rather than shuffling the manifest.
        let empty = DurationHistory::default();
        assert!(empty.is_empty());
        let (ordered, predicted, unknown) = dispatch(&["c", "a", "b"], &empty);
        assert_eq!(ordered, names(&["c", "a", "b"]));
        assert_eq!((predicted, unknown), (0, 3));

        assert_eq!(dispatch(&[], &history), (Vec::new(), 0, 0));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn history_prefers_the_newest_non_null_duration_per_scenario() {
        let root = temp_root("history-newest");
        write_summary(
            &root,
            "gate",
            1000,
            &[("stable", Some(100.0)), ("flaky", Some(4000.0))],
        );
        // The newer run measured `stable` again but never got a duration out of `flaky`.
        write_summary(
            &root,
            "gate",
            1001,
            &[("stable", Some(250.0)), ("flaky", None)],
        );

        let history = read_duration_history(&root, "gate", HISTORY_RUNS);

        assert_eq!(history.history_runs(), 2);
        assert_eq!(history.predicted_ms("stable"), Some(250.0));
        // A null is unknown, not zero, so the older measurement still stands.
        assert_eq!(history.predicted_ms("flaky"), Some(4000.0));
        assert_eq!(history.predicted_ms("never_seen"), None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn history_reads_at_most_max_runs_summaries() {
        let root = temp_root("history-window");
        // Seven runs, oldest first, each measuring the same scenario differently.
        for (offset, duration) in [10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0]
            .iter()
            .enumerate()
        {
            write_summary(
                &root,
                "gate",
                1000 + offset as u64,
                &[
                    ("shared", Some(*duration)),
                    (&format!("only_in_{offset}"), Some(1.0)),
                ],
            );
        }

        let history = read_duration_history(&root, "gate", HISTORY_RUNS);

        assert_eq!(history.history_runs(), HISTORY_RUNS);
        assert_eq!(history.predicted_ms("shared"), Some(70.0));
        // Runs 6 and 7 (`only_in_5`, `only_in_6`) are the newest five; the two oldest are
        // outside the window and contribute nothing.
        assert_eq!(history.predicted_ms("only_in_6"), Some(1.0));
        assert_eq!(history.predicted_ms("only_in_2"), Some(1.0));
        assert_eq!(history.predicted_ms("only_in_1"), None);
        assert_eq!(history.predicted_ms("only_in_0"), None);

        // A zero window reads nothing at all.
        let none = read_duration_history(&root, "gate", 0);
        assert_eq!(none.history_runs(), 0);
        assert!(none.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn unreadable_summaries_are_skipped_without_consuming_the_window() {
        let root = temp_root("history-unreadable");
        write_summary(&root, "gate", 1000, &[("measured", Some(700.0))]);
        // A run directory with no summary at all.
        fs::create_dir_all(root.join("bundles").join("gate").join("1001")).expect("bare run dir");
        // A run directory whose summary is not JSON.
        let broken = root.join("bundles").join("gate").join("1002");
        fs::create_dir_all(&broken).expect("broken run dir");
        fs::write(broken.join("summary.json"), "{not json").expect("write broken summary");

        let history = read_duration_history(&root, "gate", HISTORY_RUNS);

        // Only the one readable summary counted, and it still contributed its durations
        // even though two newer runs were unusable.
        assert_eq!(history.history_runs(), 1);
        assert_eq!(history.predicted_ms("measured"), Some(700.0));

        // A bundle that has never run has no history and is not an error.
        let missing = read_duration_history(&root, "never_ran", HISTORY_RUNS);
        assert_eq!(missing.history_runs(), 0);
        assert!(missing.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn schedule_plan_reports_the_decision_as_json() {
        let plan = SchedulePlan {
            policy: BundleSchedulePolicy::LongestFirst,
            origin: "config".to_string(),
            applied: true,
            history_runs: 3,
            predicted: 98,
            unknown: 3,
        };

        assert_eq!(
            plan.to_json(),
            json!({
                "policy": "longest_first",
                "origin": "config",
                "applied": true,
                "history_runs": 3,
                "predicted": 98,
                "unknown": 3,
            })
        );
    }

    #[test]
    fn default_policy_is_longest_first() {
        assert_eq!(
            BundleSchedulePolicy::default(),
            BundleSchedulePolicy::LongestFirst
        );
    }

    #[test]
    fn parses_both_spellings_and_round_trips_as_str() {
        assert_eq!(
            "manifest".parse::<BundleSchedulePolicy>(),
            Ok(BundleSchedulePolicy::Manifest)
        );
        assert_eq!(
            "longest-first".parse::<BundleSchedulePolicy>(),
            Ok(BundleSchedulePolicy::LongestFirst)
        );
        assert_eq!(
            "longest_first".parse::<BundleSchedulePolicy>(),
            Ok(BundleSchedulePolicy::LongestFirst)
        );
        assert_eq!(BundleSchedulePolicy::Manifest.as_str(), "manifest");
        assert_eq!(BundleSchedulePolicy::LongestFirst.as_str(), "longest_first");
        // `as_str` output must itself parse back, so the value is safe to round-trip
        // through JSON output and a later flag.
        for policy in [
            BundleSchedulePolicy::Manifest,
            BundleSchedulePolicy::LongestFirst,
        ] {
            assert_eq!(policy.as_str().parse::<BundleSchedulePolicy>(), Ok(policy));
        }
    }

    #[test]
    fn rejects_an_unknown_schedule() {
        assert!("shortest-first".parse::<BundleSchedulePolicy>().is_err());
    }

    #[test]
    fn converts_from_the_project_config_schedule() {
        assert_eq!(
            BundleSchedulePolicy::from(ScenarioSchedule::Manifest),
            BundleSchedulePolicy::Manifest
        );
        assert_eq!(
            BundleSchedulePolicy::from(ScenarioSchedule::LongestFirst),
            BundleSchedulePolicy::LongestFirst
        );
    }
}
