//! Artifact retention: bounded pruning of run directories under the artifacts root.
//!
//! Nothing here runs unless a project explicitly sets `artifacts.keep_runs` in
//! `qa/config.yaml`. The functions are pure filesystem helpers with no `ProjectConfig`
//! dependency beyond the paths they are handed, and deletion errors are always collected
//! into the returned report rather than propagated: retention is housekeeping and must
//! never change a command's exit code.

use std::fs;
use std::path::{Path, PathBuf};

/// What one prune pass did. `removed` lists the directories actually deleted; `errors`
/// collects human-readable deletion failures so a caller can surface them as a warning.
#[derive(Debug, Clone, Default)]
pub struct PruneReport {
    pub removed: Vec<PathBuf>,
    pub errors: Vec<String>,
}

impl PruneReport {
    pub fn merge(&mut self, other: PruneReport) {
        self.removed.extend(other.removed);
        self.errors.extend(other.errors);
    }
}

/// Keep the newest `keep` run directories under `<artifacts_dir>/<scenario_name>/` and
/// remove the rest. Run ids are `<epoch>Z` or `<epoch>Z-NN` (see `artifacts::default_run_id`
/// and `artifacts::allocate_run_root`); ordering is by numeric `(epoch, suffix)` rather than
/// mtime so that second-resolution ids minted by parallel bundle children still sort
/// deterministically. Entries that do not match that shape — files, symlinks, and any other
/// directory name — are ignored and never removed.
pub fn prune_scenario_runs(artifacts_dir: &Path, scenario_name: &str, keep: usize) -> PruneReport {
    prune_run_dirs(
        &artifacts_dir.join(scenario_name),
        keep,
        parse_scenario_run_id,
    )
}

/// Keep the newest `keep` bundle run directories under
/// `<artifacts_dir>/bundles/<bundle_name>/`. Bundle run ids are a bare `<epoch>`.
pub fn prune_bundle_runs(artifacts_dir: &Path, bundle_name: &str, keep: usize) -> PruneReport {
    prune_run_dirs(
        &artifacts_dir.join("bundles").join(bundle_name),
        keep,
        parse_bundle_run_id,
    )
}

/// The epoch-second timestamp of the oldest bundle run still on disk under
/// `<artifacts_dir>/bundles/<bundle_name>/`. Callers use it as the pooled-session cutoff so
/// a session directory is only removed once no retained bundle run could still reference it.
/// `None` when the bundle has no run directories at all.
pub fn oldest_retained_bundle_run_epoch_secs(
    artifacts_dir: &Path,
    bundle_name: &str,
) -> Option<u64> {
    let run_parent = artifacts_dir.join("bundles").join(bundle_name);
    fs::read_dir(run_parent)
        .ok()?
        .flatten()
        .filter(|entry| {
            entry
                .metadata()
                .map(|meta| meta.is_dir() && !godot_qa_process::directory_link::is_link(&meta))
                .unwrap_or(false)
        })
        .filter_map(|entry| parse_bundle_run_id(&entry.file_name().to_string_lossy()))
        .min()
}

/// Every bundle run directory under `<artifacts_dir>/bundles/<bundle_name>/`, newest first,
/// paired with the epoch second parsed out of its name.
///
/// The scan is the same one [`oldest_retained_bundle_run_epoch_secs`] performs: directories
/// only, epoch-shaped names only. `DirEntry::metadata` does not traverse symlinks, so the
/// `latest` convenience link is not a directory here and never appears in the result — the
/// newest entry is always a real run directory.
pub fn bundle_run_roots_newest_first(
    artifacts_dir: &Path,
    bundle_name: &str,
) -> Vec<(u64, PathBuf)> {
    let run_parent = artifacts_dir.join("bundles").join(bundle_name);
    let Ok(entries) = fs::read_dir(run_parent) else {
        return Vec::new();
    };
    let mut roots: Vec<(u64, PathBuf)> = entries
        .flatten()
        .filter(|entry| {
            entry
                .metadata()
                .map(|meta| meta.is_dir() && !godot_qa_process::directory_link::is_link(&meta))
                .unwrap_or(false)
        })
        .filter_map(|entry| {
            parse_bundle_run_id(&entry.file_name().to_string_lossy())
                .map(|epoch| (epoch, entry.path()))
        })
        .collect();
    // Descending by epoch; the path breaks ties so the order is stable across filesystems
    // that hand back directory entries in arbitrary order.
    roots.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    roots
}

/// The newest bundle run directory on disk, or `None` when the bundle has never run.
pub fn latest_bundle_run_root(artifacts_dir: &Path, bundle_name: &str) -> Option<PathBuf> {
    bundle_run_roots_newest_first(artifacts_dir, bundle_name)
        .into_iter()
        .next()
        .map(|(_, path)| path)
}

/// Point `<run_parent>/latest` at `<run_parent>/<run_dir_name>`.
///
/// The link is deliberately RELATIVE (its target is the bare run directory name) so an
/// artifacts tree stays valid when it is copied, archived, or mounted somewhere else: a
/// bundle run root is only ever addressed relative to its own parent.
///
/// On unix the rotation mirrors `godot_qa_runtime`'s `live/latest` handling: create the new
/// symlink under a unique sibling temp name, then `rename` it over `latest`, because
/// `rename(2)` replaces an existing symlink or file in one step. That is what makes a
/// *dangling* `latest` (its previous target already pruned) replaceable at all —
/// `Path::exists` follows the link and would report it missing. A pre-existing real
/// directory is removed first, since `rename` cannot replace a directory with a symlink.
///
/// Retention never removes the link and never leaves it dangling: `prune_run_dirs` only
/// considers entries whose `DirEntry::metadata` reports a directory (symlinks are excluded)
/// and whose name parses as a bare epoch, and the link always names the newest run, which is
/// the last directory a prune would remove.
///
/// Windows uses a directory junction, requiring no symlink privilege. Junction targets
/// are absolute; recreate the convenience link after relocating an artifacts tree.
pub fn update_bundle_latest_link(run_parent: &Path, run_dir_name: &str) -> Result<(), String> {
    #[cfg(unix)]
    {
        fs::create_dir_all(run_parent)
            .map_err(|error| format!("{}: {error}", run_parent.display()))?;
        let latest_path = run_parent.join("latest");
        if let Ok(metadata) = fs::symlink_metadata(&latest_path)
            && metadata.is_dir()
            && !metadata.file_type().is_symlink()
        {
            fs::remove_dir_all(&latest_path)
                .map_err(|error| format!("{}: {error}", latest_path.display()))?;
        }
        // One bundle run rotates this link once, from one thread, so pid + nanosecond is
        // enough to keep the temp name unique; the rename below is what makes the swap
        // atomic for any reader.
        let tmp_path = run_parent.join(format!(
            ".latest.tmp-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::os::unix::fs::symlink(run_dir_name, &tmp_path)
            .map_err(|error| format!("{}: {error}", tmp_path.display()))?;
        if let Err(error) = fs::rename(&tmp_path, &latest_path) {
            let _ = fs::remove_file(&tmp_path);
            return Err(format!("{}: {error}", latest_path.display()));
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        godot_qa_process::directory_link::replace(
            &run_parent.join(run_dir_name),
            &run_parent.join("latest"),
        )
        .map_err(|error| error.to_string())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (run_parent, run_dir_name);
        Ok(())
    }
}

/// Remove abandoned pooled live-session directories under `<artifacts_dir>/live/`.
///
/// A `session-<millis>-<hex>` directory is removed only when both hold:
///
/// 1. it has no `session.json`, or its `session.json` does not name a live process — a
///    session owned by a running process (possibly another checkout's CLI invocation) is
///    always kept; and
/// 2. the millisecond timestamp in its id is older than `older_than_epoch_secs`.
///
/// `live/latest` is a symlink: it is never removed, never traversed as a directory, and the
/// session it points at is kept so documented `.qa/artifacts/live/latest/...` read paths keep
/// resolving. No entry outside the `session-` prefix is touched, and the singleton
/// `.qa/session.json` lives outside this tree and is never reachable from here.
///
/// The rule itself lives in [`select_pool_sessions_for_removal`]; this is that selection
/// followed by the deletions, so a prune and an `artifact prune --dry-run` plan can never
/// disagree about what is in scope.
pub fn prune_pool_sessions(artifacts_dir: &Path, older_than_epoch_secs: u64) -> PruneReport {
    remove_selected(select_pool_sessions_for_removal(
        artifacts_dir,
        older_than_epoch_secs,
    ))
}

/// Where `live/latest` points, as an absolute path. Reading the link is not the same as
/// traversing it: the link is never removed, and resolving it only lets the prune protect
/// the session it names.
fn latest_link_target(live_dir: &Path) -> Option<PathBuf> {
    let target = godot_qa_process::directory_link::read(&live_dir.join("latest")).ok()?;
    Some(if target.is_absolute() {
        target
    } else {
        live_dir.join(target)
    })
}

/// Keep the newest `keep` run directories under `run_parent` and remove the rest.
///
/// The selection rule — which names are run ids, how they order, and which entries are never
/// candidates — lives in [`select_run_dirs_for_removal`] alone; this adds the deletions.
fn prune_run_dirs<K: Ord + Clone>(
    run_parent: &Path,
    keep: usize,
    parse_run_id: impl Fn(&str) -> Option<K>,
) -> PruneReport {
    let (removals, _) = select_run_dirs_for_removal(run_parent, keep, parse_run_id);
    remove_selected(removals)
}

/// Delete every selected directory, collecting failures instead of propagating them.
///
/// Retention is housekeeping: a directory that cannot be removed becomes an entry in
/// `errors` (which callers surface as a warning) and never stops the remaining deletions.
fn remove_selected(paths: Vec<PathBuf>) -> PruneReport {
    let mut report = PruneReport::default();
    for path in paths {
        match fs::remove_dir_all(&path) {
            Ok(()) => report.removed.push(path),
            Err(error) => report.errors.push(format!("{}: {error}", path.display())),
        }
    }
    report
}

/// `<epoch>Z` or `<epoch>Z-NN`. Returns the sort key `(epoch, suffix)`.
fn parse_scenario_run_id(name: &str) -> Option<(u64, u64)> {
    let (epoch, rest) = name.split_once('Z')?;
    let epoch = parse_digits(epoch)?;
    if rest.is_empty() {
        return Some((epoch, 0));
    }
    let suffix = parse_digits(rest.strip_prefix('-')?)?;
    Some((epoch, suffix))
}

/// A bare `<epoch>` (see `bundle::prepare_bundle_artifacts`).
fn parse_bundle_run_id(name: &str) -> Option<u64> {
    parse_digits(name)
}

/// The millisecond timestamp inside `session-<millis>-<hex>`, as whole seconds. Ids that do
/// not carry a parseable timestamp are not candidates for removal.
fn parse_session_id_epoch_secs(name: &str) -> Option<u64> {
    let rest = name.strip_prefix("session-")?;
    let (millis, _suffix) = rest.rsplit_once('-')?;
    Some(parse_digits(millis)? / 1000)
}

fn parse_digits(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse::<u64>().ok()
}

/// True when `session.json` exists and names a process that is still running. A missing,
/// unreadable, or pid-less `session.json` counts as not alive: the session directory is
/// abandoned metadata, and the age cutoff is what actually protects recent sessions.
fn session_process_is_alive(session_json: &Path) -> bool {
    let Ok(raw) = fs::read_to_string(session_json) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    let Some(pid) = value.get("pid").and_then(serde_json::Value::as_i64) else {
        return false;
    };
    godot_qa_process::process_identity_matches(
        pid,
        value
            .get("process_identity")
            .and_then(serde_json::Value::as_u64),
    )
}

// ---------------------------------------------------------------------------
// Whole-root planning for `godot-qa artifact prune`.
//
// The per-command prunes above are driven by what a run just wrote: they are handed one
// scenario name or one bundle name. `artifact prune` instead sweeps the entire artifacts
// root on demand, and it must be able to describe what it would do without doing it, so the
// selection rules are expressed as pure "select, do not delete" helpers.
//
// Those helpers are the single definition of the rules: `prune_run_dirs` and
// `prune_pool_sessions` above are exactly "select, then delete", so a plan and the prune it
// describes cannot drift apart.
// ---------------------------------------------------------------------------

/// How far before the oldest retained bundle run a pooled session must have started before it
/// becomes prunable.
///
/// A bundle run id is the epoch second at which the run *finished*, while its pooled sessions
/// start when the run begins and are shut down before the bundle writes its artifacts, so a
/// bare cutoff would delete a retained run's own session logs. `bundle::apply_bundle_retention`
/// applies the margin per bundle run and `artifact prune` applies it once across every bundle
/// at the same time; both read it from here so the two windows are the same window.
pub const POOL_SESSION_RETENTION_MARGIN_SECS: u64 = 3600;

/// Everything one whole-root prune would remove, grouped by category and computed without
/// deleting anything. `artifact prune --dry-run` reports the plan and a real prune executes
/// it, so the two can never disagree about what is in scope.
#[derive(Debug, Clone, Default)]
pub struct ArtifactPrunePlan {
    /// Run directories outside the newest `keep` under `<artifacts_dir>/<scenario>/`.
    pub scenario_runs: Vec<PathBuf>,
    /// Run directories outside the newest `keep` under `<artifacts_dir>/bundles/<bundle>/`.
    pub bundle_runs: Vec<PathBuf>,
    /// Abandoned pooled live-session directories under `<artifacts_dir>/live/`.
    pub sessions: Vec<PathBuf>,
}

impl ArtifactPrunePlan {
    /// Every planned directory: scenario runs first, then bundle runs, then sessions.
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.scenario_runs
            .iter()
            .chain(&self.bundle_runs)
            .chain(&self.sessions)
            .map(PathBuf::as_path)
    }

    pub fn len(&self) -> usize {
        self.scenario_runs.len() + self.bundle_runs.len() + self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Plan a prune of every scenario directory, every bundle, and the pooled-session tree under
/// `artifacts_dir`, keeping the newest `keep` run directories per scenario and per bundle.
///
/// A scenario directory is any directory directly under the artifacts root that holds
/// epoch-shaped run directories; `bundles/` and `live/` have their own rules and are never
/// treated as scenarios. Pooled sessions use the cutoff `apply_bundle_retention` uses, but
/// taken across *all* bundles at once: the oldest bundle run that survives this plan, less
/// `POOL_SESSION_RETENTION_MARGIN_SECS`. With no bundle runs on disk there is no cutoff that
/// can be justified, so no session is planned for removal.
///
/// `Err` means the artifacts root itself could not be listed — the one condition
/// `artifact prune` reports as a failure. A root that does not exist yet is not an error: a
/// project that has never produced artifacts simply has nothing to prune. Unreadable
/// *subdirectories* are skipped, exactly as the per-command prunes skip them.
pub fn plan_artifact_prune(artifacts_dir: &Path, keep: usize) -> Result<ArtifactPrunePlan, String> {
    let mut plan = ArtifactPrunePlan::default();
    // `keep_runs` is validated as a positive integer by the config parser and by the
    // `--keep-runs` flag; belt-and-braces so no caller can ask this to plan deleting
    // everything on disk.
    if keep == 0 {
        return Ok(plan);
    }
    let entries = match fs::read_dir(artifacts_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(plan),
        Err(error) => return Err(format!("{}: {error}", artifacts_dir.display())),
    };

    let mut scenario_dirs: Vec<PathBuf> = Vec::new();
    let mut bundles_root: Option<PathBuf> = None;
    for entry in entries.flatten() {
        // `DirEntry::metadata` does not follow symlinks, so a convenience link at the root is
        // never descended into and never becomes a candidate.
        if !entry
            .metadata()
            .map(|meta| meta.is_dir() && !godot_qa_process::directory_link::is_link(&meta))
            .unwrap_or(false)
        {
            continue;
        }
        match entry.file_name().to_string_lossy().as_ref() {
            "bundles" => bundles_root = Some(entry.path()),
            "live" => {}
            _ => scenario_dirs.push(entry.path()),
        }
    }
    scenario_dirs.sort();
    for scenario_dir in scenario_dirs {
        let (removals, _) = select_run_dirs_for_removal(&scenario_dir, keep, parse_scenario_run_id);
        plan.scenario_runs.extend(removals);
    }

    let mut oldest_retained_bundle_run: Option<u64> = None;
    if let Some(bundles_root) = bundles_root {
        let mut bundle_dirs: Vec<PathBuf> = fs::read_dir(&bundles_root)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| {
                entry
                    .metadata()
                    .map(|meta| meta.is_dir() && !godot_qa_process::directory_link::is_link(&meta))
                    .unwrap_or(false)
            })
            .map(|entry| entry.path())
            .collect();
        bundle_dirs.sort();
        for bundle_dir in bundle_dirs {
            let (removals, oldest_retained) =
                select_run_dirs_for_removal(&bundle_dir, keep, parse_bundle_run_id);
            plan.bundle_runs.extend(removals);
            if let Some(oldest_retained) = oldest_retained {
                oldest_retained_bundle_run = Some(match oldest_retained_bundle_run {
                    Some(current) => current.min(oldest_retained),
                    None => oldest_retained,
                });
            }
        }
    }
    if let Some(oldest_retained) = oldest_retained_bundle_run {
        plan.sessions = select_pool_sessions_for_removal(
            artifacts_dir,
            oldest_retained.saturating_sub(POOL_SESSION_RETENTION_MARGIN_SECS),
        );
    }
    Ok(plan)
}

/// Best-effort recursive size of `path` in bytes, counting regular files only.
///
/// Symlinks are never followed and contribute nothing, so a `latest` link cannot inflate a
/// directory's size or send the walk outside the tree. Entries that cannot be read contribute
/// zero rather than aborting the walk: the number is a reporting aid for "how much did that
/// reclaim", not an accounting record.
pub fn dir_size_bytes(path: &Path) -> u64 {
    let mut total = 0u64;
    let mut pending = vec![path.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if godot_qa_process::directory_link::is_link(&metadata) {
                continue;
            }
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    total
}

/// The run directories under `run_parent` that fall outside the newest `keep`, plus the oldest
/// run key that survives.
///
/// This is THE retention rule for run directories: ids are ordered numerically by the shape
/// `parse_run_id` recognises, anything unparseable (a file, a symlink, any other directory
/// name) is never a candidate, and `keep == 0` selects nothing at all — `keep_runs` is
/// validated as a positive integer at config load, and this is belt-and-braces so no caller
/// can ask retention to delete everything a run just wrote. Nothing is removed here:
/// `prune_run_dirs` deletes what this returns and `plan_artifact_prune` only reports it.
/// Removals are returned oldest first. The surviving key is what lets a whole-root plan
/// derive a pooled-session cutoff from runs it has only *planned* to keep.
fn select_run_dirs_for_removal<K: Ord + Clone>(
    run_parent: &Path,
    keep: usize,
    parse_run_id: impl Fn(&str) -> Option<K>,
) -> (Vec<PathBuf>, Option<K>) {
    if keep == 0 {
        return (Vec::new(), None);
    }
    let Ok(entries) = fs::read_dir(run_parent) else {
        return (Vec::new(), None);
    };
    let mut candidates: Vec<(K, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        if !entry
            .metadata()
            .map(|meta| meta.is_dir() && !godot_qa_process::directory_link::is_link(&meta))
            .unwrap_or(false)
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(key) = parse_run_id(&name) else {
            continue;
        };
        candidates.push((key, entry.path()));
    }
    candidates.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    let remove_count = candidates.len().saturating_sub(keep);
    let oldest_retained = candidates.get(remove_count).map(|(key, _)| key.clone());
    let removals = candidates
        .into_iter()
        .take(remove_count)
        .map(|(_, path)| path)
        .collect();
    (removals, oldest_retained)
}

/// The pooled live-session directories under `<artifacts_dir>/live/` that a prune with this
/// cutoff would remove, without removing them.
///
/// THE rule for pooled sessions: only `session-<millis>-<hex>` directories are candidates,
/// only when they started before the cutoff, only when no live process owns them, and never
/// `live/latest` or the session it names. Results are sorted so a plan is deterministic.
fn select_pool_sessions_for_removal(
    artifacts_dir: &Path,
    older_than_epoch_secs: u64,
) -> Vec<PathBuf> {
    let live_dir = artifacts_dir.join("live");
    let latest_target = latest_link_target(&live_dir);
    let Ok(entries) = fs::read_dir(&live_dir) else {
        return Vec::new();
    };
    let mut selected: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        // `DirEntry::metadata` does not follow symlinks, so `live/latest` is not a directory
        // here and is skipped before anything else looks at it.
        if !entry
            .metadata()
            .map(|meta| meta.is_dir() && !godot_qa_process::directory_link::is_link(&meta))
            .unwrap_or(false)
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(started_secs) = parse_session_id_epoch_secs(&name) else {
            continue;
        };
        if started_secs >= older_than_epoch_secs {
            continue;
        }
        let path = entry.path();
        if latest_target.as_deref() == Some(path.as_path()) {
            continue;
        }
        if session_process_is_alive(&path.join("session.json")) {
            continue;
        }
        selected.push(path);
    }
    selected.sort();
    selected
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "godot-qa-retention-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("create temp root");
        root
    }

    #[cfg(windows)]
    #[test]
    fn windows_junctions_are_not_walked_and_keep_latest_session_target() {
        let root = temp_root("junction-retention");
        let live = root.join("live");
        let target = live.join("session-1000-aaaa");
        make_dir(&target);
        fs::write(target.join("data"), b"evidence").unwrap();
        godot_qa_process::directory_link::replace(&target, &live.join("latest")).unwrap();
        assert_eq!(
            dir_size_bytes(&live),
            8,
            "junction target must not count twice"
        );
        assert!(select_pool_sessions_for_removal(&root, 10000).is_empty());
        let bundles = root.join("bundles/gate");
        make_dir(&bundles.join("1000"));
        update_bundle_latest_link(&bundles, "1000").unwrap();
        assert_eq!(
            bundle_run_roots_newest_first(&root, "gate"),
            vec![(1000, bundles.join("1000"))]
        );
        fs::remove_dir(live.join("latest")).unwrap();
        fs::remove_dir(bundles.join("latest")).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn live_process_identity_keeps_owned_session_artifacts() {
        let root = temp_root("identity-retention");
        let session = root.join("live/session-1000-aaaa");
        make_dir(&session);
        let pid = i64::from(std::process::id());
        fs::write(session.join("session.json"), serde_json::json!({"pid": pid, "process_identity": godot_qa_process::process_identity(pid)}).to_string()).unwrap();
        assert!(select_pool_sessions_for_removal(&root, 10000).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    fn make_dir(path: &Path) {
        fs::create_dir_all(path).expect("create dir");
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut entries: Vec<String> = fs::read_dir(dir)
            .expect("read dir")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        entries.sort();
        entries
    }

    #[test]
    fn prune_scenario_runs_keeps_the_newest_by_numeric_epoch() {
        let root = temp_root("scenario-newest");
        let scenario_dir = root.join("main_menu");
        // Deliberately out of lexicographic order: "1000Z" sorts before "999Z" as text.
        for run_id in ["999Z", "1000Z", "1001Z", "1002Z"] {
            make_dir(&scenario_dir.join(run_id));
        }

        let report = prune_scenario_runs(&root, "main_menu", 2);

        assert_eq!(report.removed.len(), 2);
        assert!(report.errors.is_empty());
        assert_eq!(names(&scenario_dir), vec!["1001Z", "1002Z"]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn prune_scenario_runs_orders_same_second_suffixes() {
        let root = temp_root("scenario-suffix");
        let scenario_dir = root.join("gate_child");
        // Parallel bundle children collide on the second and disambiguate with `-NN`.
        for run_id in ["1000Z", "1000Z-01", "1000Z-02", "1000Z-10"] {
            make_dir(&scenario_dir.join(run_id));
        }

        let report = prune_scenario_runs(&root, "gate_child", 2);

        assert_eq!(report.removed.len(), 2);
        // "-10" is newer than "-02" numerically even though it sorts earlier as text.
        assert_eq!(names(&scenario_dir), vec!["1000Z-02", "1000Z-10"]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn prune_scenario_runs_ignores_non_run_entries() {
        let root = temp_root("scenario-ignore");
        let scenario_dir = root.join("main_menu");
        for run_id in ["1000Z", "1001Z", "1002Z"] {
            make_dir(&scenario_dir.join(run_id));
        }
        make_dir(&scenario_dir.join("sweep-1000Z"));
        make_dir(&scenario_dir.join("baselines"));
        make_dir(&scenario_dir.join("1000"));
        fs::write(scenario_dir.join("999Z"), "not a directory").expect("write file");

        let report = prune_scenario_runs(&root, "main_menu", 1);

        assert_eq!(report.removed.len(), 2);
        assert_eq!(
            names(&scenario_dir),
            vec!["1000", "1002Z", "999Z", "baselines", "sweep-1000Z"]
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn prune_scenario_runs_is_a_no_op_below_the_limit() {
        let root = temp_root("scenario-under");
        let scenario_dir = root.join("main_menu");
        for run_id in ["1000Z", "1001Z"] {
            make_dir(&scenario_dir.join(run_id));
        }

        let report = prune_scenario_runs(&root, "main_menu", 5);

        assert!(report.removed.is_empty());
        assert_eq!(names(&scenario_dir), vec!["1000Z", "1001Z"]);
        // A scenario that has never run is not an error either.
        let missing = prune_scenario_runs(&root, "never_ran", 1);
        assert!(missing.removed.is_empty());
        assert!(missing.errors.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn prune_bundle_runs_keeps_the_newest_bare_epoch_ids() {
        let root = temp_root("bundle-newest");
        let bundle_dir = root.join("bundles").join("regression_gate");
        for run_id in ["999", "1000", "1001"] {
            make_dir(&bundle_dir.join(run_id));
        }

        let report = prune_bundle_runs(&root, "regression_gate", 1);

        assert_eq!(report.removed.len(), 2);
        assert_eq!(names(&bundle_dir), vec!["1001"]);
        assert_eq!(
            oldest_retained_bundle_run_epoch_secs(&root, "regression_gate"),
            Some(1001)
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn prune_pool_sessions_removes_only_old_ownerless_sessions() {
        let root = temp_root("pool-sessions");
        let live_dir = root.join("live");
        make_dir(&live_dir);

        // Old and abandoned (no session.json): removed.
        make_dir(&live_dir.join("session-1000000-aaaa"));
        // Old, but owned by this very process: kept.
        let live_session = live_dir.join("session-1000000-bbbb");
        make_dir(&live_session);
        fs::write(
            live_session.join("session.json"),
            serde_json::json!({"pid": std::process::id()}).to_string(),
        )
        .expect("write session.json");
        // Old, with a session.json naming a dead pid: removed.
        let dead_session = live_dir.join("session-1000000-cccc");
        make_dir(&dead_session);
        fs::write(
            dead_session.join("session.json"),
            serde_json::json!({"pid": 0}).to_string(),
        )
        .expect("write session.json");
        // Newer than the cutoff: kept even though it is abandoned.
        make_dir(&live_dir.join("session-9000000-dddd"));
        // Not a pooled session directory at all: never a candidate.
        make_dir(&live_dir.join("events"));
        fs::write(live_dir.join("notes.txt"), "keep me").expect("write file");

        let report = prune_pool_sessions(&root, 5000);

        assert_eq!(report.removed.len(), 2);
        assert!(report.errors.is_empty());
        assert_eq!(
            names(&live_dir),
            vec![
                "events",
                "notes.txt",
                "session-1000000-bbbb",
                "session-9000000-dddd",
            ]
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn prune_pool_sessions_never_touches_the_latest_symlink_or_its_target() {
        let root = temp_root("pool-latest");
        let live_dir = root.join("live");
        make_dir(&live_dir);
        let linked = live_dir.join("session-1000000-aaaa");
        let unlinked = live_dir.join("session-1000000-bbbb");
        make_dir(&linked);
        make_dir(&unlinked);
        std::os::unix::fs::symlink(&linked, live_dir.join("latest")).expect("symlink latest");

        let report = prune_pool_sessions(&root, 5000);

        // The symlink is never removed, and neither is what it points at, so documented
        // `live/latest/...` read paths keep resolving.
        assert_eq!(report.removed, vec![unlinked]);
        assert_eq!(
            names(&live_dir),
            vec!["latest", "session-1000000-aaaa"],
            "the latest symlink and its target both survive"
        );
        assert_eq!(
            godot_qa_process::directory_link::read(&live_dir.join("latest")).unwrap(),
            linked
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn prune_run_dirs_never_deletes_everything_when_keep_is_zero() {
        let root = temp_root("keep-zero");
        let scenario_dir = root.join("main_menu");
        make_dir(&scenario_dir.join("1000Z"));

        let report = prune_scenario_runs(&root, "main_menu", 0);

        assert!(report.removed.is_empty());
        assert_eq!(names(&scenario_dir), vec!["1000Z"]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn run_id_parsers_accept_only_their_own_shapes() {
        assert_eq!(parse_scenario_run_id("1788493321Z"), Some((1788493321, 0)));
        assert_eq!(
            parse_scenario_run_id("1788493321Z-07"),
            Some((1788493321, 7))
        );
        assert_eq!(parse_scenario_run_id("1788493321"), None);
        assert_eq!(parse_scenario_run_id("sweep-1788493321Z"), None);
        assert_eq!(parse_scenario_run_id("1788493321Z-"), None);
        assert_eq!(parse_scenario_run_id("Z"), None);

        assert_eq!(parse_bundle_run_id("1788493321"), Some(1788493321));
        assert_eq!(parse_bundle_run_id("1788493321Z"), None);

        assert_eq!(
            parse_session_id_epoch_secs("session-1788493321987-ab12"),
            Some(1788493321)
        );
        assert_eq!(parse_session_id_epoch_secs("session-cleanup"), None);
        assert_eq!(parse_session_id_epoch_secs("events"), None);
    }

    #[test]
    fn bundle_run_roots_are_newest_first_and_skip_junk_entries() {
        let root = temp_root("bundle-roots");
        let bundle_dir = root.join("bundles").join("regression_gate");
        for run_id in ["999", "1000", "1001"] {
            make_dir(&bundle_dir.join(run_id));
        }
        // Everything below is deliberately not a bundle run directory.
        make_dir(&bundle_dir.join("1002Z"));
        make_dir(&bundle_dir.join("baselines"));
        fs::write(bundle_dir.join("1003"), "a file, not a run").expect("write file");

        let roots = bundle_run_roots_newest_first(&root, "regression_gate");

        assert_eq!(
            roots,
            vec![
                (1001, bundle_dir.join("1001")),
                (1000, bundle_dir.join("1000")),
                (999, bundle_dir.join("999")),
            ]
        );
        assert_eq!(
            latest_bundle_run_root(&root, "regression_gate"),
            Some(bundle_dir.join("1001"))
        );
        // A bundle that has never run has no roots and no latest.
        assert!(bundle_run_roots_newest_first(&root, "never_ran").is_empty());
        assert_eq!(latest_bundle_run_root(&root, "never_ran"), None);
        let _ = fs::remove_dir_all(&root);
    }

    /// `.qa/artifacts/` as `artifact prune` finds it: two scenarios, two bundles, and a
    /// pooled-session tree, with the junk entries retention must never touch mixed in.
    fn make_prune_fixture(root: &Path) {
        for run_id in ["1000Z", "1001Z", "1002Z"] {
            make_dir(&root.join("main_menu").join(run_id));
        }
        make_dir(&root.join("main_menu").join("baselines"));
        for run_id in ["2000Z", "2001Z"] {
            make_dir(&root.join("gameplay_flow").join(run_id));
        }
        for run_id in ["9000", "9100", "9200"] {
            make_dir(&root.join("bundles").join("gate_a").join(run_id));
        }
        for run_id in ["8000", "8100"] {
            make_dir(&root.join("bundles").join("gate_b").join(run_id));
        }
        // Well before `8100 - 3600`, so it ages out once `gate_b` keeps only `8100`.
        make_dir(&root.join("live").join("session-1000000-aaaa"));
        // Just inside the window: `4600000 / 1000` is not older than `8100 - 3600`.
        make_dir(&root.join("live").join("session-4600000-bbbb"));
    }

    #[test]
    fn plan_artifact_prune_covers_scenarios_bundles_and_sessions() {
        let root = temp_root("plan-all");
        make_prune_fixture(&root);

        let plan = plan_artifact_prune(&root, 1).expect("plan");

        // Newest one per scenario survives; `baselines` is not a run id and is never listed.
        assert_eq!(
            plan.scenario_runs,
            vec![
                root.join("gameplay_flow/2000Z"),
                root.join("main_menu/1000Z"),
                root.join("main_menu/1001Z"),
            ]
        );
        assert_eq!(
            plan.bundle_runs,
            vec![
                root.join("bundles/gate_a/9000"),
                root.join("bundles/gate_a/9100"),
                root.join("bundles/gate_b/8000"),
            ]
        );
        // The cutoff is the oldest bundle run left across BOTH bundles (8100), less the
        // one-hour margin, so only the far older session is in scope.
        assert_eq!(plan.sessions, vec![root.join("live/session-1000000-aaaa")]);
        assert_eq!(plan.len(), 7);
        assert!(!plan.is_empty());
        assert_eq!(plan.paths().count(), 7);
        // Planning never deletes anything.
        assert_eq!(names(&root.join("main_menu")).len(), 4);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn plan_artifact_prune_keeps_every_run_inside_the_window() {
        let root = temp_root("plan-under");
        make_prune_fixture(&root);

        let plan = plan_artifact_prune(&root, 10).expect("plan");

        assert!(plan.scenario_runs.is_empty(), "{plan:?}");
        assert!(plan.bundle_runs.is_empty(), "{plan:?}");
        // Abandoned sessions age out on their own schedule: with no bundle run pruned the
        // cutoff is the oldest run on disk (8000) less the margin, which still strands the
        // 1000-second session.
        assert_eq!(plan.sessions, vec![root.join("live/session-1000000-aaaa")]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn plan_artifact_prune_leaves_sessions_alone_without_bundle_runs() {
        let root = temp_root("plan-no-bundles");
        for run_id in ["1000Z", "1001Z"] {
            make_dir(&root.join("main_menu").join(run_id));
        }
        make_dir(&root.join("live").join("session-1000-aaaa"));

        let plan = plan_artifact_prune(&root, 1).expect("plan");

        assert_eq!(plan.scenario_runs, vec![root.join("main_menu/1000Z")]);
        // Nothing establishes how old is old enough, so no session is a candidate.
        assert!(plan.sessions.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn latest_link_is_relative_and_excluded_from_the_run_scan() {
        let root = temp_root("latest-relative");
        let bundle_dir = root.join("bundles").join("regression_gate");
        make_dir(&bundle_dir.join("1000"));

        update_bundle_latest_link(&bundle_dir, "1000").expect("link the newest run");

        // Relative on purpose: the link resolves against its own parent, so the artifacts
        // tree survives being copied or mounted elsewhere.
        assert_eq!(
            fs::read_link(bundle_dir.join("latest")).unwrap(),
            PathBuf::from("1000")
        );
        assert!(bundle_dir.join("latest").is_dir());
        // The symlink is not a run directory, so it never shows up as a candidate run and
        // no temp file is left behind.
        assert_eq!(
            bundle_run_roots_newest_first(&root, "regression_gate"),
            vec![(1000, bundle_dir.join("1000"))]
        );
        assert_eq!(names(&bundle_dir), vec!["1000", "latest"]);
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn latest_link_replaces_a_dangling_link_and_a_real_directory() {
        let root = temp_root("latest-replace");
        let bundle_dir = root.join("bundles").join("regression_gate");
        make_dir(&bundle_dir.join("2000"));

        // A dangling link: `Path::exists` follows it and reports missing, so only the
        // rename-over rotation can replace it.
        std::os::unix::fs::symlink("1999", bundle_dir.join("latest")).expect("dangling link");
        assert!(!bundle_dir.join("latest").exists());
        update_bundle_latest_link(&bundle_dir, "2000").expect("replace dangling link");
        assert_eq!(
            fs::read_link(bundle_dir.join("latest")).unwrap(),
            PathBuf::from("2000")
        );

        // A real directory left by an older tool version: removed before the link lands.
        fs::remove_file(bundle_dir.join("latest")).expect("clear link");
        make_dir(&bundle_dir.join("latest").join("stale"));
        make_dir(&bundle_dir.join("2001"));
        update_bundle_latest_link(&bundle_dir, "2001").expect("replace real directory");
        assert!(
            fs::symlink_metadata(bundle_dir.join("latest"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_link(bundle_dir.join("latest")).unwrap(),
            PathBuf::from("2001")
        );
        assert_eq!(names(&bundle_dir), vec!["2000", "2001", "latest"]);
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn prune_bundle_runs_keeps_the_latest_link_and_its_target() {
        let root = temp_root("latest-prune");
        let bundle_dir = root.join("bundles").join("regression_gate");
        for run_id in ["1000", "1001", "1002"] {
            make_dir(&bundle_dir.join(run_id));
        }
        update_bundle_latest_link(&bundle_dir, "1002").expect("link the newest run");

        let report = prune_bundle_runs(&root, "regression_gate", 1);

        // The link is not a directory to the prune scan and its target is the newest run,
        // so the documented `bundles/<name>/latest/...` path keeps resolving.
        assert_eq!(report.removed.len(), 2);
        assert!(report.errors.is_empty());
        assert_eq!(names(&bundle_dir), vec!["1002", "latest"]);
        assert_eq!(
            fs::read_link(bundle_dir.join("latest")).unwrap(),
            PathBuf::from("1002")
        );
        assert!(bundle_dir.join("latest").is_dir());
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn plan_artifact_prune_never_plans_latest_or_a_live_session() {
        let root = temp_root("plan-latest");
        make_dir(&root.join("bundles").join("gate_a").join("9000"));
        let linked = root.join("live/session-1000000-aaaa");
        let owned = root.join("live/session-1000000-bbbb");
        let abandoned = root.join("live/session-1000000-cccc");
        make_dir(&linked);
        make_dir(&owned);
        make_dir(&abandoned);
        fs::write(
            owned.join("session.json"),
            serde_json::json!({"pid": std::process::id()}).to_string(),
        )
        .expect("write session.json");
        std::os::unix::fs::symlink(&linked, root.join("live/latest")).expect("symlink latest");

        let plan = plan_artifact_prune(&root, 1).expect("plan");

        assert_eq!(plan.sessions, vec![abandoned]);
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn plan_artifact_prune_never_plans_a_bundle_latest_link() {
        let root = temp_root("plan-bundle-latest");
        let bundle_dir = root.join("bundles").join("gate_a");
        for run_id in ["9000", "9100"] {
            make_dir(&bundle_dir.join(run_id));
        }
        update_bundle_latest_link(&bundle_dir, "9100").expect("link the newest run");

        let plan = plan_artifact_prune(&root, 1).expect("plan");

        // The link is not a directory to the scan, so it is neither a bundle name to descend
        // into nor a run to remove, and `bundles/<name>/latest/...` keeps resolving.
        assert_eq!(plan.bundle_runs, vec![bundle_dir.join("9000")]);
        assert!(plan.scenario_runs.is_empty());
        assert_eq!(
            fs::read_link(bundle_dir.join("latest")).unwrap(),
            PathBuf::from("9100")
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn plan_artifact_prune_reports_an_unreadable_root_but_not_a_missing_one() {
        let root = temp_root("plan-root");
        // A project that has never produced artifacts has nothing to prune, not an error.
        let missing = root.join("never-created");
        assert!(
            plan_artifact_prune(&missing, 1)
                .expect("missing root")
                .is_empty()
        );

        // A file where the artifacts root should be is actionable: the message names it.
        let not_a_dir = root.join("artifacts");
        fs::write(&not_a_dir, "not a directory").expect("write file");
        let error = plan_artifact_prune(&not_a_dir, 1).expect_err("unreadable root");
        assert!(
            error.starts_with(&format!("{}: ", not_a_dir.display())),
            "{error}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn dir_size_bytes_sums_files_recursively_and_ignores_symlinks() {
        let root = temp_root("dir-size");
        make_dir(&root.join("run/snapshots"));
        fs::write(root.join("run/result.json"), "0123456789").expect("write");
        fs::write(root.join("run/snapshots/tree.json"), "01234").expect("write");
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("run/result.json"), root.join("run/alias.json"))
            .expect("symlink");

        assert_eq!(dir_size_bytes(&root.join("run")), 15);
        // A directory that cannot be read contributes zero rather than panicking.
        assert_eq!(dir_size_bytes(&root.join("missing")), 0);
        let _ = fs::remove_dir_all(&root);
    }
}
