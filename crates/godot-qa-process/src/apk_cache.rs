//! Conservative content-addressed cache for Android debug APK exports.
//!
//! Exporting a debug APK on every `session start --target android` is the slowest step of
//! the flow, yet it only changes when the project (or the toolchain) changes. This module
//! computes a cache key from the Godot binary + version, the export preset/package, and a
//! cheap size+mtime manifest of the project tree, then stores the exported APK under
//! `<root>/.qa/cache/android/<key>/`.
//!
//! The cache is deliberately biased toward *false misses*: any write to a manifested file
//! (size or mtime change) yields a new key and forces a fresh export, so a stale hit is
//! never served. Every operation degrades to "no cache" on error rather than failing a
//! session — a broken cache must never break a start.

use godot_qa_core::hash::sha256_hex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Number of newest cache entries retained by [`prune`].
pub const APK_CACHE_KEEP: usize = 4;
/// File name of the cached debug APK inside each entry directory.
pub const APK_FILE_NAME: &str = "session-debug.apk";
/// Serialization version embedded in the cache key so a format change invalidates old keys.
const KEY_PREFIX: &str = "godot-qa-apk-cache-v1";
/// Guard against runaway symlink graphs while still following the addon symlink.
const MAX_WALK_DEPTH: usize = 32;

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Root directory holding all Android APK cache entries for `project_root`.
pub fn cache_root(project_root: &Path) -> PathBuf {
    project_root.join(".qa").join("cache").join("android")
}

/// Inputs that fully determine a cache key. Everything here participates in the digest.
pub struct ApkCacheKeyInputs<'a> {
    pub godot_binary: &'a str,
    pub godot_version: &'a str,
    pub preset_name: &'a str,
    pub package: &'a str,
    pub project_root: &'a Path,
    /// Directories to exclude from the manifest in addition to the built-in rules (used for
    /// an `artifacts_dir` that resolves inside the project root).
    pub extra_excluded_dirs: &'a [PathBuf],
}

/// Metadata persisted alongside each cached APK. Validated on lookup and used for eviction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApkCacheMeta {
    pub version: u32,
    pub key: String,
    pub created_at_ms: u64,
    pub last_used_at_ms: u64,
    pub godot_binary: String,
    pub godot_version: String,
    pub preset: String,
    pub package: String,
    pub manifest_files: u64,
    pub apk_bytes: u64,
    pub tool_version: String,
}

/// A validated cache entry: the entry directory, the cached APK path, and its metadata.
pub struct ApkCacheEntry {
    pub dir: PathBuf,
    pub apk_path: PathBuf,
    pub meta: ApkCacheMeta,
}

/// Compute the cache key for `inputs`. `Err` carries a human-readable reason (never fatal):
/// the caller treats it as a cache bypass.
pub fn compute_cache_key(inputs: &ApkCacheKeyInputs<'_>) -> Result<String, String> {
    let manifest = collect_manifest(inputs.project_root, inputs.extra_excluded_dirs)?;

    let absolute_binary = std::path::absolute(inputs.godot_binary)
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| inputs.godot_binary.to_string());

    let mut buffer = Vec::new();
    buffer.extend_from_slice(KEY_PREFIX.as_bytes());
    buffer.push(0);
    for field in [
        absolute_binary.as_str(),
        inputs.godot_version,
        inputs.preset_name,
        inputs.package,
    ] {
        buffer.extend_from_slice(field.as_bytes());
        buffer.push(0);
    }
    for (rel_path, (size, mtime_nanos)) in &manifest {
        buffer.extend_from_slice(rel_path.as_bytes());
        buffer.push(0);
        buffer.extend_from_slice(size.to_string().as_bytes());
        buffer.push(0);
        buffer.extend_from_slice(mtime_nanos.to_string().as_bytes());
        buffer.push(b'\n');
    }
    Ok(sha256_hex(&buffer))
}

/// Number of manifested files for a given input set (used to populate [`ApkCacheMeta`]).
pub fn manifest_file_count(inputs: &ApkCacheKeyInputs<'_>) -> Result<u64, String> {
    Ok(collect_manifest(inputs.project_root, inputs.extra_excluded_dirs)?.len() as u64)
}

/// Look up a validated entry for `key`. Returns `None` (and best-effort removes the entry)
/// when the metadata is corrupt, the recorded key mismatches, or the APK is missing/resized.
pub fn lookup(cache_root: &Path, key: &str) -> Option<ApkCacheEntry> {
    let dir = cache_root.join(key);
    if !dir.is_dir() {
        return None;
    }
    let apk_path = dir.join(APK_FILE_NAME);
    let Some(meta) = read_meta(&dir) else {
        remove_entry(&dir);
        return None;
    };
    let dir_name_matches = dir.file_name().and_then(|name| name.to_str()) == Some(key);
    let apk_matches = fs::metadata(&apk_path)
        .map(|metadata| metadata.is_file() && metadata.len() == meta.apk_bytes)
        .unwrap_or(false);
    if meta.key == key && dir_name_matches && apk_matches {
        Some(ApkCacheEntry {
            dir,
            apk_path,
            meta,
        })
    } else {
        remove_entry(&dir);
        None
    }
}

/// Bump `last_used_at_ms` for `entry` (best effort; failures are ignored).
pub fn touch(entry: &ApkCacheEntry) {
    let mut meta = entry.meta.clone();
    meta.last_used_at_ms = now_ms();
    if let Ok(bytes) = serde_json::to_vec_pretty(&meta) {
        let _ = fs::write(entry.dir.join("meta.json"), bytes);
    }
}

/// Store `apk_path` under `key`, replacing any existing entry. The metadata's `key` and
/// `apk_bytes` are refreshed from the stored copy so lookups always round-trip.
pub fn store(
    cache_root: &Path,
    key: &str,
    apk_path: &Path,
    mut meta: ApkCacheMeta,
) -> Result<ApkCacheEntry, String> {
    fs::create_dir_all(cache_root)
        .map_err(|error| format!("create cache root {}: {error}", cache_root.display()))?;
    let staging = cache_root.join(format!(".tmp-{}", tmp_suffix()));
    let result = write_entry(&staging, key, apk_path, &mut meta);
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result?;

    let dest = cache_root.join(key);
    if dest.exists() {
        fs::remove_dir_all(&dest).map_err(|error| {
            let _ = fs::remove_dir_all(&staging);
            format!("replace cache entry {}: {error}", dest.display())
        })?;
    }
    fs::rename(&staging, &dest).map_err(|error| {
        let _ = fs::remove_dir_all(&staging);
        format!("finalize cache entry {}: {error}", dest.display())
    })?;

    Ok(ApkCacheEntry {
        apk_path: dest.join(APK_FILE_NAME),
        dir: dest,
        meta,
    })
}

/// Keep the `keep` newest entries (by `last_used_at_ms`, falling back to `created_at_ms`) and
/// remove the rest plus any stale `.tmp-*` staging directories. Returns the removed names.
pub fn prune(cache_root: &Path, keep: usize) -> Vec<String> {
    let mut removed = Vec::new();
    let Ok(entries) = fs::read_dir(cache_root) else {
        return removed;
    };
    let mut candidates: Vec<(String, PathBuf, u64)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(".tmp-") {
            if fs::remove_dir_all(&path).is_ok() {
                removed.push(name);
            }
            continue;
        }
        if let Some(meta) = read_meta(&path) {
            let recency = if meta.last_used_at_ms != 0 {
                meta.last_used_at_ms
            } else {
                meta.created_at_ms
            };
            candidates.push((name, path, recency));
        }
    }
    // Newest first, then drop everything past `keep`.
    candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.2));
    for (name, path, _) in candidates.into_iter().skip(keep) {
        if fs::remove_dir_all(&path).is_ok() {
            removed.push(name);
        }
    }
    removed
}

fn write_entry(
    staging: &Path,
    key: &str,
    apk_path: &Path,
    meta: &mut ApkCacheMeta,
) -> Result<(), String> {
    fs::create_dir_all(staging)
        .map_err(|error| format!("create staging dir {}: {error}", staging.display()))?;
    let staged_apk = staging.join(APK_FILE_NAME);
    let copied = fs::copy(apk_path, &staged_apk)
        .map_err(|error| format!("copy apk into cache {}: {error}", staged_apk.display()))?;
    meta.key = key.to_string();
    meta.apk_bytes = copied;
    let bytes = serde_json::to_vec_pretty(meta)
        .map_err(|error| format!("serialize cache metadata: {error}"))?;
    fs::write(staging.join("meta.json"), bytes)
        .map_err(|error| format!("write cache metadata: {error}"))?;
    Ok(())
}

fn read_meta(dir: &Path) -> Option<ApkCacheMeta> {
    let bytes = fs::read(dir.join("meta.json")).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn remove_entry(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
}

fn collect_manifest(
    project_root: &Path,
    extra_excluded_dirs: &[PathBuf],
) -> Result<BTreeMap<String, (u64, i128)>, String> {
    let root_canonical = fs::canonicalize(project_root)
        .map_err(|error| format!("resolve project root {}: {error}", project_root.display()))?;
    let excluded: HashSet<PathBuf> = extra_excluded_dirs
        .iter()
        .filter_map(|dir| fs::canonicalize(dir).ok())
        .collect();

    let mut files = BTreeMap::new();
    let mut visited = HashSet::new();
    visited.insert(root_canonical);
    walk_dir(
        project_root,
        project_root,
        &excluded,
        &mut visited,
        &mut files,
        0,
    )?;
    Ok(files)
}

fn walk_dir(
    dir: &Path,
    project_root: &Path,
    excluded: &HashSet<PathBuf>,
    visited: &mut HashSet<PathBuf>,
    files: &mut BTreeMap<String, (u64, i128)>,
    depth: usize,
) -> Result<(), String> {
    if depth > MAX_WALK_DEPTH {
        return Ok(());
    }
    let entries =
        fs::read_dir(dir).map_err(|error| format!("read dir {}: {error}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("read entry in {}: {error}", dir.display()))?;
        let name = entry.file_name();
        // Skip hidden names: covers .git, .qa, .godot, .import, and .gdignore itself.
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let child = dir.join(&name);
        // fs::metadata follows symlinks, so the addon symlink is traversed as a real dir.
        let metadata =
            fs::metadata(&child).map_err(|error| format!("stat {}: {error}", child.display()))?;
        if metadata.is_dir() {
            let canonical = fs::canonicalize(&child)
                .map_err(|error| format!("resolve dir {}: {error}", child.display()))?;
            if excluded.contains(&canonical) {
                continue;
            }
            // Cycle guard for symlink graphs that point back into the tree.
            if !visited.insert(canonical) {
                continue;
            }
            // Mirror the Godot exporter: a directory holding a `.gdignore` is excluded.
            if child.join(".gdignore").exists() {
                continue;
            }
            walk_dir(&child, project_root, excluded, visited, files, depth + 1)?;
        } else if metadata.is_file() {
            let relative = child.strip_prefix(project_root).unwrap_or(&child);
            files.insert(
                relative_to_slash(relative),
                (metadata.len(), mtime_nanos(&metadata)),
            );
        }
    }
    Ok(())
}

fn relative_to_slash(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn mtime_nanos(metadata: &fs::Metadata) -> i128 {
    match metadata.modified() {
        Ok(time) => match time.duration_since(UNIX_EPOCH) {
            Ok(delta) => delta.as_nanos() as i128,
            Err(error) => -(error.duration().as_nanos() as i128),
        },
        Err(_) => 0,
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|delta| delta.as_millis() as u64)
        .unwrap_or(0)
}

fn tmp_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|delta| delta.as_nanos())
        .unwrap_or(0);
    let count = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{nanos}-{count}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let count = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "godot-qa-apk-cache-{label}-{}-{nanos}-{count}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn build_project(root: &Path) {
        fs::write(root.join("project.godot"), b"config").expect("project.godot");
        fs::create_dir_all(root.join("scenes")).expect("scenes dir");
        fs::write(root.join("scenes/main.tscn"), b"[gd_scene]").expect("main.tscn");
    }

    fn key_inputs<'a>(root: &'a Path, excluded: &'a [PathBuf]) -> ApkCacheKeyInputs<'a> {
        ApkCacheKeyInputs {
            godot_binary: "/usr/bin/godot",
            godot_version: "4.3.stable",
            preset_name: "Android QA",
            package: "org.example.qa",
            project_root: root,
            extra_excluded_dirs: excluded,
        }
    }

    fn sample_meta(key: &str, recency: u64) -> ApkCacheMeta {
        ApkCacheMeta {
            version: 1,
            key: key.to_string(),
            created_at_ms: recency,
            last_used_at_ms: recency,
            godot_binary: "/usr/bin/godot".to_string(),
            godot_version: "4.3.stable".to_string(),
            preset: "Android QA".to_string(),
            package: "org.example.qa".to_string(),
            manifest_files: 2,
            apk_bytes: 0,
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    #[test]
    fn key_is_stable_for_an_unchanged_project() {
        let root = unique_temp_dir("stable");
        build_project(&root);
        let first = compute_cache_key(&key_inputs(&root, &[])).expect("key");
        let second = compute_cache_key(&key_inputs(&root, &[])).expect("key");
        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
        assert!(first.chars().all(|ch| ch.is_ascii_hexdigit()));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn key_changes_on_file_modify_add_and_delete() {
        let root = unique_temp_dir("mutate");
        build_project(&root);
        let base = compute_cache_key(&key_inputs(&root, &[])).expect("key");

        // Modify: bump mtime without changing size to exercise the mtime axis.
        let scene = root.join("scenes/main.tscn");
        let handle = fs::File::options().write(true).open(&scene).expect("open");
        handle
            .set_modified(SystemTime::now() + std::time::Duration::from_secs(60))
            .expect("set mtime");
        drop(handle);
        let modified = compute_cache_key(&key_inputs(&root, &[])).expect("key");
        assert_ne!(base, modified, "mtime change must change the key");

        // Add a file.
        fs::write(root.join("scenes/extra.tscn"), b"[gd_scene]").expect("extra");
        let added = compute_cache_key(&key_inputs(&root, &[])).expect("key");
        assert_ne!(modified, added, "adding a file must change the key");

        // Delete it again.
        fs::remove_file(root.join("scenes/extra.tscn")).expect("remove");
        let deleted = compute_cache_key(&key_inputs(&root, &[])).expect("key");
        assert_eq!(
            modified, deleted,
            "removing the added file restores the key"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn key_depends_on_preset_package_and_godot_version() {
        let root = unique_temp_dir("axes");
        build_project(&root);
        let base = compute_cache_key(&key_inputs(&root, &[])).expect("key");

        let mut preset = key_inputs(&root, &[]);
        preset.preset_name = "Other Preset";
        assert_ne!(base, compute_cache_key(&preset).expect("key"));

        let mut package = key_inputs(&root, &[]);
        package.package = "org.example.other";
        assert_ne!(base, compute_cache_key(&package).expect("key"));

        let mut version = key_inputs(&root, &[]);
        version.godot_version = "4.4.stable";
        assert_ne!(base, compute_cache_key(&version).expect("key"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn key_ignores_hidden_gdignored_and_excluded_dirs() {
        let root = unique_temp_dir("excludes");
        build_project(&root);
        let excluded = vec![root.join("build_out")];
        fs::create_dir_all(root.join("build_out")).expect("build_out");
        let base = compute_cache_key(&key_inputs(&root, &excluded)).expect("key");

        // Hidden dir: ignored.
        fs::create_dir_all(root.join(".git")).expect(".git");
        fs::write(root.join(".git/HEAD"), b"ref: refs/heads/main").expect("HEAD");
        // .gdignore'd dir: ignored.
        fs::create_dir_all(root.join("blob")).expect("blob");
        fs::write(root.join("blob/.gdignore"), b"").expect(".gdignore");
        fs::write(root.join("blob/huge.bin"), b"0123456789").expect("huge");
        // Explicitly excluded dir: ignored.
        fs::write(root.join("build_out/out.apk"), b"stale").expect("out");
        let after = compute_cache_key(&key_inputs(&root, &excluded)).expect("key");
        assert_eq!(base, after, "excluded trees must not affect the key");

        // A normal new dir DOES change the key.
        fs::create_dir_all(root.join("assets")).expect("assets");
        fs::write(root.join("assets/logo.png"), b"img").expect("logo");
        let changed = compute_cache_key(&key_inputs(&root, &excluded)).expect("key");
        assert_ne!(base, changed);
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn key_follows_symlinked_dirs_and_terminates_on_cycles() {
        use std::os::unix::fs::symlink;

        let root = unique_temp_dir("symlink");
        build_project(&root);

        // A symlinked addon-style dir whose contents must be walked.
        let real = unique_temp_dir("symlink-target");
        fs::write(real.join("plugin.gd"), b"extends Node").expect("plugin");
        symlink(&real, root.join("linked_addon")).expect("symlink dir");

        // A cycle back into the project root: the walk must terminate.
        symlink(&root, root.join("loop")).expect("symlink loop");

        let base = compute_cache_key(&key_inputs(&root, &[])).expect("key");

        // Changing a file behind the symlink changes the key (proves the link is followed).
        fs::write(real.join("plugin.gd"), b"extends Node2D").expect("plugin update");
        let changed = compute_cache_key(&key_inputs(&root, &[])).expect("key");
        assert_ne!(base, changed);

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&real);
    }

    #[test]
    fn store_then_lookup_round_trips() {
        let cache = unique_temp_dir("roundtrip");
        let apk = cache.join("source.apk");
        fs::write(&apk, b"debug-apk-bytes").expect("apk");
        let entry = store(&cache, "abc123", &apk, sample_meta("abc123", 100)).expect("store");
        assert_eq!(entry.meta.apk_bytes, b"debug-apk-bytes".len() as u64);
        assert!(entry.apk_path.is_file());

        let found = lookup(&cache, "abc123").expect("lookup hit");
        assert_eq!(found.meta.key, "abc123");
        assert_eq!(fs::read(&found.apk_path).expect("apk"), b"debug-apk-bytes");
        assert!(lookup(&cache, "missing").is_none());
        let _ = fs::remove_dir_all(&cache);
    }

    #[test]
    fn lookup_rejects_and_removes_corrupt_entries() {
        let cache = unique_temp_dir("corrupt");
        let apk = cache.join("source.apk");
        fs::write(&apk, b"apk").expect("apk");

        // Corrupt metadata JSON.
        store(&cache, "corrupt", &apk, sample_meta("corrupt", 1)).expect("store");
        fs::write(cache.join("corrupt/meta.json"), b"{ not json").expect("corrupt meta");
        assert!(lookup(&cache, "corrupt").is_none());
        assert!(!cache.join("corrupt").exists(), "corrupt entry removed");

        // Key mismatch between dir name and recorded key.
        store(&cache, "mismatch", &apk, sample_meta("mismatch", 1)).expect("store");
        let mut wrong = read_meta(&cache.join("mismatch")).expect("meta");
        wrong.key = "somethingelse".to_string();
        fs::write(
            cache.join("mismatch/meta.json"),
            serde_json::to_vec_pretty(&wrong).unwrap(),
        )
        .expect("rewrite meta");
        assert!(lookup(&cache, "mismatch").is_none());
        assert!(!cache.join("mismatch").exists(), "mismatched entry removed");

        // APK size mismatch.
        store(&cache, "resized", &apk, sample_meta("resized", 1)).expect("store");
        fs::write(cache.join("resized").join(APK_FILE_NAME), b"different-size").expect("resize");
        assert!(lookup(&cache, "resized").is_none());
        assert!(!cache.join("resized").exists(), "resized entry removed");
        let _ = fs::remove_dir_all(&cache);
    }

    #[test]
    fn prune_keeps_newest_and_clears_tmp_dirs() {
        let cache = unique_temp_dir("prune");
        let apk = cache.join("source.apk");
        fs::write(&apk, b"apk").expect("apk");
        for (index, key) in ["k1", "k2", "k3", "k4", "k5", "k6"].iter().enumerate() {
            let recency = (index as u64 + 1) * 1000;
            store(&cache, key, &apk, sample_meta(key, recency)).expect("store");
        }
        let stale_tmp = cache.join(".tmp-stale");
        fs::create_dir_all(&stale_tmp).expect("stale tmp");

        let removed = prune(&cache, APK_CACHE_KEEP);

        assert!(removed.contains(&".tmp-stale".to_string()));
        assert!(removed.contains(&"k1".to_string()));
        assert!(removed.contains(&"k2".to_string()));
        assert!(!cache.join("k1").exists());
        assert!(!cache.join("k2").exists());
        for key in ["k3", "k4", "k5", "k6"] {
            assert!(cache.join(key).exists(), "{key} should be retained");
        }
        assert!(!stale_tmp.exists(), "stale tmp dir removed");
        let _ = fs::remove_dir_all(&cache);
    }
}
